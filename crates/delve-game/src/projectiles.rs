//! Trap-launcher projectiles: launcher fire, projectile ticking against the
//! world, hit resolution (player/enemy damage, status effects, shared kill
//! path), and billboard rendering — ported from the TS `projectileSystem`,
//! the `gameState.onLauncherFire`/`projectileManager.setHitCallback` wiring
//! in `main.ts`, and `rendering/projectileRenderer.ts`.
//!
//! Heights follow TS's split exactly: every projectile mesh — whatever
//! layer it flies on — renders at the scene-build active layer's height
//! plus `PROJECTILE_HEIGHT` (TS parents all projectile meshes to one group
//! whose `position.y` is set once per scene build,
//! `levelSceneBuilder.ts:565`, captured here as [`ProjectileGroupY`]),
//! while hit visuals (damage numbers, fireball explosions) use the hit
//! layer's own height — TS's hit callback reads `activeLayerIndex` while
//! `tickProjectiles` has it swapped to the ticked projectile's layer.
//!
//! One adaptation from the TS renderer: `FireballExplosions`'
//! `THREE.Points` burst becomes individual small billboard quads sharing one
//! fading material — this engine has no point-sprite/particle-buffer
//! primitive, but every other effect here (damage numbers, item pickups)
//! already uses individual billboard quads, so this keeps the same shape.

use crate::billboard::FacesCamera;
use crate::dungeon::CELL_SIZE;
use crate::enemies::{EnemyBillboards, EnemyDb, KillTarget, handle_kill};
use crate::ground_items::{GroundItemRender, LootTablesRes};
use crate::level_scene::LevelEntity;
use crate::overlay::InputGate;
use crate::session::{DungeonRes, GameRng, Session};
use crate::torch::LUMENS_PER_THREE_UNIT;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::{GameState, WorldEvent, door_key, layer_door_key};
use delve_core::projectiles::{
    HitType, Projectile, ProjectileHitEvent, ProjectileManager, ProjectileUpdateContext,
    SpawnOptions,
};
use delve_core::status_effects::{StatusEffectType, apply_effect};
use delve_core::types::Dungeon;
use std::collections::{HashMap, HashSet};

const PROJECTILE_HEIGHT: f32 = 1.2;

const DART_SIZE: f32 = 0.15;
const ARROW_SIZE: f32 = 0.25;
const FIREBALL_SIZE: f32 = 0.35;
const DEFAULT_PROJECTILE_SIZE: f32 = 0.2;

const DART_COLOR: Color = Color::srgb_u8(0x8B, 0x73, 0x55);
const ARROW_COLOR: Color = Color::srgb_u8(0x55, 0x55, 0x55);
const FIREBALL_COLOR: Color = Color::srgb_u8(0xFF, 0x44, 0x00);
const FIREBALL_EMISSIVE: LinearRgba = LinearRgba::rgb(2.0, 0.533, 0.0);

const FIREBALL_LIGHT_INTENSITY: f32 = 3.0; // THREE.js PointLight units
const FIREBALL_LIGHT_RANGE: f32 = 6.0;

const EXPLOSION_PARTICLE_COUNT: usize = 18;
const EXPLOSION_LIFETIME: f32 = 0.45;
const EXPLOSION_SPEED: f64 = 3.5;
const EXPLOSION_SIZE: f32 = 0.12;
const EXPLOSION_COLOR: Color = Color::srgb_u8(0xFF, 0x44, 0x00);
const EXPLOSION_LIGHT_RANGE: f32 = 8.0;
const EXPLOSION_FLASH_FRACTION: f32 = 0.15;

/// Duration applied to a status effect carried by a projectile hit, matching
/// the TS `applyEffect(..., 6)` calls in `main.ts`'s hit callback.
const PROJECTILE_STATUS_EFFECT_DURATION: f64 = 6.0;

#[derive(Resource, Default)]
pub struct ProjectileManagerRes(pub ProjectileManager);

/// TS's `projectileMeshes.group.position.y = activeLayerIdx * LAYER_HEIGHT`
/// (`levelSceneBuilder.ts:565`): the one Y every projectile mesh renders at,
/// captured from whichever layer is active at scene-build time and untouched
/// by later same-scene layer switches (falling, ramps) — the same
/// build-time-capture rule `LevelZones` follows. Re-inserted by
/// `spawn_level_scene` on every build.
#[derive(Resource, Default)]
pub struct ProjectileGroupY(pub f32);

/// The mesh (and, for fireballs, the accompanying point light) for one
/// active projectile. Kept as independent entities rather than a Bevy
/// parent/child pair — no other renderer in this crate uses hierarchy, so
/// `position_projectile_meshes` just moves both directly each frame.
pub struct ProjectileVisual {
    pub mesh: Entity,
    pub light: Option<Entity>,
}

#[derive(Resource, Default)]
pub struct ProjectileBillboards {
    pub by_id: HashMap<String, ProjectileVisual>,
}

#[derive(Component)]
pub struct ProjectileBillboard;

fn quad_size_for(projectile_type: &str) -> f32 {
    match projectile_type {
        "dart" => DART_SIZE,
        "arrow" => ARROW_SIZE,
        "fireball" => FIREBALL_SIZE,
        _ => DEFAULT_PROJECTILE_SIZE,
    }
}

fn color_for(projectile_type: &str) -> Color {
    match projectile_type {
        "dart" => DART_COLOR,
        "arrow" => ARROW_COLOR,
        "fireball" => FIREBALL_COLOR,
        _ => Color::WHITE,
    }
}

fn spawn_projectile_visual(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    projectile_type: &str,
    translation: Vec3,
) -> ProjectileVisual {
    let size = quad_size_for(projectile_type);
    let color = color_for(projectile_type);
    let is_fireball = projectile_type == "fireball";

    let mesh = if is_fireball {
        meshes.add(Circle::new(size / 2.0))
    } else {
        meshes.add(Rectangle::new(size, size))
    };
    let material = materials.add(StandardMaterial {
        base_color: color,
        emissive: if is_fireball {
            FIREBALL_EMISSIVE
        } else {
            LinearRgba::NONE
        },
        unlit: !is_fireball,
        // Only the lit fireball needs back-face normal flipping (TS's
        // DoubleSide MeshStandardMaterial); darts/arrows are unlit, where
        // the flag changes nothing.
        double_sided: is_fireball,
        cull_mode: None,
        ..default()
    });

    let mesh_entity = commands
        .spawn((
            LevelEntity,
            ProjectileBillboard,
            FacesCamera,
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(translation),
        ))
        .id();

    let light = is_fireball.then(|| {
        commands
            .spawn((
                LevelEntity,
                PointLight {
                    color: FIREBALL_COLOR,
                    intensity: FIREBALL_LIGHT_INTENSITY * LUMENS_PER_THREE_UNIT,
                    range: FIREBALL_LIGHT_RANGE,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_translation(translation),
            ))
            .id()
    });

    ProjectileVisual {
        mesh: mesh_entity,
        light,
    }
}

fn despawn_projectile_visual(commands: &mut Commands, visual: ProjectileVisual) {
    commands.entity(visual.mesh).despawn();
    if let Some(light) = visual.light {
        commands.entity(light).despawn();
    }
}

// --- Fireball explosions ---

#[derive(Component)]
pub(crate) struct ExplosionVelocity(Vec3);

#[derive(Component)]
pub(crate) struct Explosion {
    age: f32,
    material: Handle<StandardMaterial>,
    particles: Vec<Entity>,
}

/// `FireballExplosions.spawn(worldX, worldZ, yOffset)`
/// (`projectileRenderer.ts:56-59`): particles burst from `PROJECTILE_HEIGHT`
/// above the hit layer's base height.
fn spawn_fireball_explosion(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    random: &mut dyn FnMut() -> f64,
    world_x: f32,
    world_z: f32,
    y_offset: f32,
) {
    let base_y = PROJECTILE_HEIGHT + y_offset;
    let particle_mesh = meshes.add(Rectangle::new(EXPLOSION_SIZE, EXPLOSION_SIZE));
    let material = materials.add(StandardMaterial {
        base_color: EXPLOSION_COLOR,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    });

    let mut particles = Vec::with_capacity(EXPLOSION_PARTICLE_COUNT);
    for _ in 0..EXPLOSION_PARTICLE_COUNT {
        let theta = random() * std::f64::consts::TAU;
        let phi = random() * std::f64::consts::PI - std::f64::consts::FRAC_PI_2;
        let speed = EXPLOSION_SPEED * (0.5 + random() * 0.5);
        let velocity = Vec3::new(
            (theta.cos() * phi.cos() * speed) as f32,
            (phi.sin() * speed * 0.6 + 1.0) as f32,
            (theta.sin() * phi.cos() * speed) as f32,
        );
        let particle = commands
            .spawn((
                LevelEntity,
                FacesCamera,
                ExplosionVelocity(velocity),
                Mesh3d(particle_mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(world_x, base_y, world_z),
            ))
            .id();
        particles.push(particle);
    }

    commands.spawn((
        LevelEntity,
        Explosion {
            age: 0.0,
            material,
            particles,
        },
        PointLight {
            color: EXPLOSION_COLOR,
            intensity: 6.0 * LUMENS_PER_THREE_UNIT,
            range: EXPLOSION_LIGHT_RANGE,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(world_x, base_y, world_z),
    ));
}

/// Ages every active explosion: fades its shared material, follows the same
/// flash-then-fade light curve as the TS version, and integrates each
/// particle's velocity (gravity + drag). Despawns the whole group once its
/// lifetime elapses.
pub fn update_fireball_explosions(
    time: Res<Time>,
    mut commands: Commands,
    mut explosions: Query<(Entity, &mut Explosion, &mut PointLight)>,
    mut particles: Query<(&mut Transform, &mut ExplosionVelocity)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    gate: InputGate,
) {
    if gate.paused() {
        return;
    }
    let delta = time.delta_secs();
    for (entity, mut explosion, mut light) in &mut explosions {
        explosion.age += delta;
        if explosion.age >= EXPLOSION_LIFETIME {
            commands.entity(entity).despawn();
            for &particle in &explosion.particles {
                commands.entity(particle).despawn();
            }
            continue;
        }

        let t = explosion.age / EXPLOSION_LIFETIME;
        if let Some(mut material) = materials.get_mut(&explosion.material) {
            material.base_color = EXPLOSION_COLOR.with_alpha(1.0 - t);
        }
        // 0-15%: bright flash peaking at 12 (THREE units); 15-100%: fade from 6 to 0.
        let intensity_three_units = if t < EXPLOSION_FLASH_FRACTION {
            6.0 + 6.0 * (t / EXPLOSION_FLASH_FRACTION * std::f32::consts::PI).sin()
        } else {
            6.0 * (1.0 - (t - EXPLOSION_FLASH_FRACTION) / (1.0 - EXPLOSION_FLASH_FRACTION))
        };
        light.intensity = intensity_three_units * LUMENS_PER_THREE_UNIT;

        for &particle in &explosion.particles {
            let Ok((mut transform, mut velocity)) = particles.get_mut(particle) else {
                continue;
            };
            velocity.0.y -= 4.0 * delta;
            let drag = 1.0 - 2.0 * delta;
            velocity.0.x *= drag;
            velocity.0.z *= drag;
            transform.translation += velocity.0 * delta;
        }
    }
}

// --- Launcher fire + projectile tick + hit resolution ---

/// TS's implicit string-to-enum cast on a behavior's `statusEffect` param —
/// shared by the projectile-hit and enemy melee `onHit` paths.
pub(crate) fn parse_status_effect_type(value: &str) -> Option<StatusEffectType> {
    match value {
        "poison" => Some(StatusEffectType::Poison),
        "slow" => Some(StatusEffectType::Slow),
        "burning" => Some(StatusEffectType::Burning),
        _ => None,
    }
}

/// True when `col`/`row` names a walkable character in `grid`, matching the
/// TS `ctx.ls.walkable.has(layerGrid[row]?.[col])` optional-chain lookup.
fn is_walkable_at(grid: &[String], walkable: &HashSet<char>, col: i64, row: i64) -> bool {
    let (Ok(row_index), Ok(col_index)) = (usize::try_from(row), usize::try_from(col)) else {
        return false;
    };
    grid.get(row_index)
        .and_then(|line| line.chars().nth(col_index))
        .is_some_and(|character| walkable.contains(&character))
}

/// The grid for `layer_index`: the live session grid when it's the player's
/// own layer, otherwise the static grid from the loaded dungeon definition
/// (mirroring the TS `ctx.ls.layerGrids[li] ?? ctx.ls.level.grid` fallback —
/// this engine doesn't cache a live per-layer grid map, so background
/// layers read their grid straight from the dungeon asset instead).
fn layer_grid<'a>(
    dungeon: &'a Dungeon,
    level_id: &str,
    layer_index: usize,
    player_layer_index: usize,
    active_grid: &'a [String],
) -> &'a [String] {
    if layer_index == player_layer_index {
        return active_grid;
    }
    dungeon
        .levels
        .iter()
        .find(|level| level.id.as_deref() == Some(level_id) || level.name == level_id)
        .and_then(|level| level.layers.get(layer_index))
        .map(|layer| layer.grid.as_slice())
        .unwrap_or(active_grid)
}

/// Spawns the projectile for one `LauncherFire` event on `layer_index`.
/// Shared by the non-active-layer loop below and by
/// `session::apply_world_events`, which handles active-layer fire events —
/// `tick_signals` ticks the active layer's launchers, and its events are
/// drained by whichever session system runs first, so the arm there is the
/// only reliable consumer for them.
pub(crate) fn fire_launcher_at(
    game: &GameState,
    manager: &mut ProjectileManager,
    layer_index: usize,
    col: i64,
    row: i64,
) {
    let Some(launcher) = game
        .layer(layer_index)
        .and_then(|layer| layer.trap_launchers.get(&door_key(col, row)))
        .cloned()
    else {
        return;
    };
    let spawned = manager.spawn(SpawnOptions {
        col: launcher.col,
        row: launcher.row,
        direction: launcher.facing,
        projectile_type: &launcher.projectile_type,
        source: None,
        max_range: launcher.max_range,
        layer_index: Some(layer_index),
    });
    if let Err(error) = spawned {
        warn!("trap launcher at ({col},{row}) failed to fire: {error}");
    }
}

/// Fires trap launchers on every layer EXCEPT the active one, whose
/// launchers are already ticked by `tick_signals` (ticking them here too
/// would double-advance `next_fire_at` and drop the shot). Events are
/// drained immediately after each layer's tick — the only point
/// `active_layer_index` unambiguously names the layer that produced them.
fn fire_trap_launchers(game: &mut GameState, manager: &mut ProjectileManager) {
    let saved_layer = game.active_layer_index;
    for layer_index in 0..game.layers.len() {
        if layer_index == saved_layer {
            continue;
        }
        game.active_layer_index = layer_index;
        game.tick_trap_launchers();
        for event in game.take_events() {
            let WorldEvent::LauncherFire { col, row } = event else {
                continue;
            };
            fire_launcher_at(game, manager, layer_index, col, row);
        }
    }
    game.active_layer_index = saved_layer;
}

/// Rendering and reward handles a projectile hit needs — reused for enemy
/// kills via the same [`handle_kill`] path melee attacks use.
#[derive(SystemParam)]
pub struct ProjectileTickEffects<'w, 's> {
    billboards: ResMut<'w, ProjectileBillboards>,
    enemies: ResMut<'w, EnemyBillboards>,
    database: Res<'w, EnemyDb>,
    loot_tables: Res<'w, LootTablesRes>,
    item_render: GroundItemRender<'w, 's>,
    images: ResMut<'w, Assets<Image>>,
    rng: ResMut<'w, GameRng>,
    vitals: ResMut<'w, crate::status_effects::PlayerVitals>,
    feedback: crate::enemy_feedback::CombatFeedback<'w, 's>,
    visibility: Query<'w, 's, &'static mut Visibility>,
    hud: ResMut<'w, crate::hud::HudState>,
    debug_flags: Res<'w, crate::debug::DebugFlags>,
    group_y: Res<'w, ProjectileGroupY>,
}

/// Applies one projectile hit's gameplay effects (damage, status effect,
/// enemy kill) and its visuals (damage number, fireball explosion) — ported
/// from the TS `projectileManager.setHitCallback` body in `main.ts`. Runs
/// with `game.active_layer_index` swapped to the hit projectile's layer,
/// exactly the state TS's callback reads its heights from.
fn apply_projectile_hit(
    game: &mut GameState,
    effects: &mut ProjectileTickEffects,
    event: ProjectileHitEvent,
) {
    match event.hit_type {
        // TS: `if (hitType === 'player' && !debugFullbright)` (`main.ts:976`)
        // — a sibling `if`, not a wrapper, around the fireball-explosion
        // spawn below (`main.ts:1001-1007`), which stays unconditional.
        HitType::Player if !effects.debug_flags.fullbright => {
            game.player.hp = (game.player.hp - event.projectile.damage).max(0.0);
            effects.vitals.flash();
            if let Some(effect_type) = event
                .projectile
                .status_effect
                .as_deref()
                .and_then(parse_status_effect_type)
            {
                apply_effect(
                    &mut game.status_fx.player_status_effects,
                    effect_type,
                    PROJECTILE_STATUS_EFFECT_DURATION,
                );
            }
            info!(
                "A {} hits you for {} — HP {}/{}",
                event.projectile.projectile_type,
                event.projectile.damage,
                game.player.hp,
                game.player.max_hp
            );
            // Death detection is centralized in `save_load::check_player_death`.
        }
        HitType::Player => {}
        HitType::Enemy => {
            let key = door_key(event.col, event.row);
            let render_key = layer_door_key(game.active_layer_index, &key);
            let dying_enemy = {
                let Some(enemy) = game.active_layer_mut().enemies.get_mut(&key) else {
                    return;
                };
                if let Some(effect_type) = event
                    .projectile
                    .status_effect
                    .as_deref()
                    .and_then(parse_status_effect_type)
                {
                    apply_effect(
                        &mut enemy.status_effects,
                        effect_type,
                        PROJECTILE_STATUS_EFFECT_DURATION,
                    );
                }
                (enemy.enemy_type.clone(), enemy.drops.clone())
            };
            if let Some(&entity) = effects.enemies.by_key.get(&render_key) {
                effects.feedback.flash(entity);
                effects.feedback.trigger_hit_shake(entity);
            }
            let killed = game.damage_enemy(event.col, event.row, event.projectile.damage);
            // `main.ts:993`: at the hit layer's own height (activeLayerIndex
            // is the projectile's layer here), on every layer.
            let layer_y_offset = game.active_layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
            crate::damage_numbers::spawn_damage_number(
                &mut effects.item_render.commands,
                &mut effects.item_render.meshes,
                &mut effects.images,
                &mut effects.item_render.materials,
                event.projectile.damage,
                (event.col, event.row),
                layer_y_offset,
            );
            if killed {
                let (enemy_type, drops_override) = dying_enemy;
                let target = KillTarget {
                    col: event.col,
                    row: event.row,
                    enemy_type,
                    drops_override,
                    layer_index: game.active_layer_index,
                };
                let leveled = handle_kill(
                    game,
                    &mut effects.rng.0,
                    &mut effects.enemies,
                    &effects.database.0,
                    &effects.loot_tables.0,
                    &mut effects.item_render,
                    &target,
                );
                effects.feedback.health_bars.remove(&render_key);
                if leveled {
                    effects.hud.trigger_level_up(game.player.level);
                }
            } else if let Some(enemy) = game.get_enemy(event.col, event.row) {
                let (hp, max_hp) = (enemy.hp, enemy.max_hp);
                effects.feedback.update_health_bar(
                    &mut effects.visibility,
                    &mut effects.item_render.materials,
                    &render_key,
                    hp,
                    max_hp,
                );
            }
        }
        HitType::Wall | HitType::Door => {}
    }

    if event.projectile.projectile_type == "fireball" {
        // `main.ts:1001-1007`: unconditional, at the hit layer's height.
        let layer_y_offset = game.active_layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
        let ProjectileTickEffects {
            item_render, rng, ..
        } = effects;
        let mut random = || rng.0.next_f64();
        spawn_fireball_explosion(
            &mut item_render.commands,
            &mut item_render.meshes,
            &mut item_render.materials,
            &mut random,
            event.projectile.col as f32 * CELL_SIZE,
            event.projectile.row as f32 * CELL_SIZE,
            layer_y_offset,
        );
    }
}

/// Adds/removes projectile billboards to match the manager's active set —
/// every projectile on every layer, matching TS's `getAll()` sync
/// (`projectileSystem.ts:46`; a background-layer fireball renders at the
/// scene-build group height like everything else). Positions are synced
/// separately each frame by [`position_projectile_meshes`].
fn sync_projectile_meshes(manager: &ProjectileManager, effects: &mut ProjectileTickEffects) {
    let visible: Vec<&Projectile> = manager.get_all().iter().collect();
    let active_ids: HashSet<&str> = visible
        .iter()
        .map(|projectile| projectile.id.as_str())
        .collect();

    let stale_ids: Vec<String> = effects
        .billboards
        .by_id
        .keys()
        .filter(|id| !active_ids.contains(id.as_str()))
        .cloned()
        .collect();
    for id in stale_ids {
        if let Some(visual) = effects.billboards.by_id.remove(&id) {
            despawn_projectile_visual(&mut effects.item_render.commands, visual);
        }
    }

    for projectile in visible {
        if effects.billboards.by_id.contains_key(&projectile.id) {
            continue;
        }
        let translation = Vec3::new(
            projectile.col as f32 * CELL_SIZE,
            effects.group_y.0 + PROJECTILE_HEIGHT,
            projectile.row as f32 * CELL_SIZE,
        );
        let visual = spawn_projectile_visual(
            &mut effects.item_render.commands,
            &mut effects.item_render.meshes,
            &mut effects.item_render.materials,
            &projectile.projectile_type,
            translation,
        );
        effects
            .billboards
            .by_id
            .insert(projectile.id.clone(), visual);
    }
}

/// Fires launchers, ticks every layer with an active projectile against the
/// world, and resolves hits — the per-frame block ported from TS's
/// `tickTrapLaunchers` + `tickProjectiles(projectileCtx, delta)`. Paused
/// while character creation is open, matching the TS `anyOverlayOpen` guard.
pub fn tick_projectiles(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut manager: ResMut<ProjectileManagerRes>,
    dungeon: Res<DungeonRes>,
    gate: InputGate,
    mut effects: ProjectileTickEffects,
) {
    if gate.paused() {
        return;
    }
    let delta = f64::from(time.delta_secs());

    fire_trap_launchers(&mut session.game, &mut manager.0);

    let player_layer_index = session.game.active_layer_index;
    let mut active_layers: Vec<usize> = manager
        .0
        .get_all()
        .iter()
        .map(|projectile| projectile.layer_index)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    active_layers.sort_unstable();

    for layer_index in active_layers {
        session.game.active_layer_index = layer_index;
        // TS passes the real player cell only for the player's own layer
        // (`projectileSystem.ts:31-32`); background layers get (-1,-1) so a
        // projectile there can't hit the player.
        let (player_col, player_row) = if layer_index == player_layer_index {
            (
                i64::from(session.last_player_pose.0),
                i64::from(session.last_player_pose.1),
            )
        } else {
            (-1, -1)
        };

        let events = {
            let grid = layer_grid(
                &dungeon.0,
                &session.current_level_id,
                layer_index,
                player_layer_index,
                &session.grid,
            );
            let walkable = &session.walkable;
            let game = &session.game;
            let is_walkable = |col: i64, row: i64| is_walkable_at(grid, walkable, col, row);
            let is_door_open = |col: i64, row: i64| game.is_door_open(col, row);
            let is_block_at = |col: i64, row: i64| game.is_block_at(col, row);
            let is_enemy_at = |col: i64, row: i64| game.is_enemy_at(col, row);
            let is_solid_edge_blocked = |from_col: i64, from_row: i64, to_col: i64, to_row: i64| {
                game.is_solid_edge_blocked(from_col, from_row, to_col, to_row)
            };
            let context = ProjectileUpdateContext {
                is_walkable: &is_walkable,
                is_door_open: &is_door_open,
                player_col,
                player_row,
                is_enemy_at: Some(&is_enemy_at),
                is_block_at: Some(&is_block_at),
                is_solid_edge_blocked: Some(&is_solid_edge_blocked),
                layer_filter: Some(layer_index),
            };
            manager.0.update(delta, &context)
        };

        for event in events {
            apply_projectile_hit(&mut session.game, &mut effects, event);
        }
    }
    session.game.active_layer_index = player_layer_index;

    sync_projectile_meshes(&manager.0, &mut effects);
}

/// Moves every active projectile's mesh (and fireball light) to its current
/// world position. Runs every frame regardless of the character-creation
/// gate — a no-op when nothing moved, matching every other billboard-sync
/// system in this crate.
pub fn position_projectile_meshes(
    manager: Res<ProjectileManagerRes>,
    billboards: Res<ProjectileBillboards>,
    group_y: Res<ProjectileGroupY>,
    mut transforms: Query<&mut Transform>,
) {
    for projectile in manager.0.get_all() {
        let Some(visual) = billboards.by_id.get(&projectile.id) else {
            continue;
        };
        let translation = Vec3::new(
            projectile.col as f32 * CELL_SIZE,
            group_y.0 + PROJECTILE_HEIGHT,
            projectile.row as f32 * CELL_SIZE,
        );
        if let Ok(mut transform) = transforms.get_mut(visual.mesh) {
            transform.translation = translation;
        }
        if let Some(light) = visual.light
            && let Ok(mut transform) = transforms.get_mut(light)
        {
            transform.translation = translation;
        }
    }
}

/// Clears all projectile state and billboards. Called on level transitions
/// alongside the other per-transition resets in `transition.rs`, mirroring
/// the TS `projectileManager.clear()` transition hook. Explosion entities
/// are cleaned up implicitly — they're tagged `LevelEntity` like every other
/// transient visual, so the transition's level-entity despawn sweep handles
/// them without needing a separate `FireballExplosions.clear()` call.
pub fn clear_on_transition(
    manager: &mut ProjectileManagerRes,
    billboards: &mut ProjectileBillboards,
) {
    manager.0.clear();
    billboards.by_id.clear();
}

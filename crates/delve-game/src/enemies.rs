//! Enemy billboards and AI ticking: sprites face the camera's view plane,
//! move with the core AI, and melee the player when adjacent.

use crate::barrels::{self, BarrelHandles};
use crate::dungeon::CELL_SIZE;
use crate::ground_items::{self, GroundItemRender, LootTablesRes};
use crate::level_scene::LevelEntity;
use crate::player::Player;
use crate::session::{GameRng, Session};
use crate::wall_entities;
use bevy::prelude::*;
use delve_core::combat::{CombatResultType, enemy_attack_player, player_attack};
use delve_core::enemies::{DEFAULT_SPRITE_SIZE, EnemyDatabase};
use delve_core::enemy_ai::{EnemyActionType, EnemyUpdateContext, update_enemies};
use delve_core::game_state::{DoorState, GameState, door_key};
use delve_core::loot::{DropsOverride, LootTables};
use delve_core::random::Mulberry32;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Shared enemy definitions for the AI context and sprite lookups.
#[derive(Resource)]
pub struct EnemyDb(pub Arc<EnemyDatabase>);

#[derive(Component)]
pub struct EnemyBillboard;

/// Billboard entities by enemy cell key, re-keyed as enemies move.
#[derive(Resource, Default)]
pub struct EnemyBillboards {
    pub by_key: HashMap<String, Entity>,
}

/// Rendering-side handles the enemy tick needs to apply AI actions, plus
/// everything a kill needs (XP, loot, billboard cleanup) so both the melee
/// and status-effect-death paths can reuse [`handle_kill`] without pushing
/// `tick_enemies` over the argument-count lint.
#[derive(bevy::ecs::system::SystemParam)]
pub struct EnemyRenderState<'w, 's> {
    billboards: ResMut<'w, EnemyBillboards>,
    // `Without<HealthBarFill>` makes this provably disjoint from
    // `feedback`'s own `Query<&mut Transform, With<HealthBarFill>>` — see
    // that field's doc comment for why the `With<EnemyBillboard>` half
    // alone isn't enough to satisfy Bevy's conflict check.
    transforms: Query<
        'w,
        's,
        &'static mut Transform,
        (
            With<EnemyBillboard>,
            Without<crate::enemy_feedback::HealthBarFill>,
        ),
    >,
    database: Res<'w, EnemyDb>,
    loot_tables: Res<'w, LootTablesRes>,
    item_render: GroundItemRender<'w, 's>,
    feedback: crate::enemy_feedback::CombatFeedback<'w, 's>,
    visibility: Query<'w, 's, &'static mut Visibility>,
    hud: ResMut<'w, crate::hud::HudState>,
}

fn sprite_asset_path(sprite_path: &str) -> String {
    sprite_path.trim_start_matches('/').to_string()
}

/// Billboard edge length and vertical offset for `def`'s sprite, falling
/// back to `DEFAULT_SPRITE_SIZE`/no offset when a definition (or its size)
/// is missing. Shared by the billboard spawn and `enemy_feedback`'s health
/// bar spawn, which needs the same edge length to place its bar above the
/// sprite.
pub(crate) fn sprite_dimensions(def: Option<&delve_core::enemies::EnemyDef>) -> (f32, f32) {
    let size = def
        .and_then(|def| def.sprite.size)
        .unwrap_or(DEFAULT_SPRITE_SIZE) as f32;
    let y_offset = def.and_then(|def| def.sprite.y_offset).unwrap_or(0.0) as f32;
    (size, y_offset)
}

pub fn spawn_enemy_billboards(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    game: &delve_core::game_state::GameState,
    database: &EnemyDatabase,
) -> EnemyBillboards {
    let mut billboards = EnemyBillboards::default();
    for (key, enemy) in &game.active_layer().enemies {
        let def = database.get_enemy(&enemy.enemy_type);
        let (size, y_offset) = sprite_dimensions(def);
        let texture_path = def
            .map(|def| sprite_asset_path(&def.sprite.path))
            .unwrap_or_else(|| "sprites/skeleton.png".to_string());

        let material = materials.add(StandardMaterial {
            base_color_texture: Some(asset_server.load(texture_path)),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            ..default()
        });
        let center_x = enemy.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = enemy.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let entity = commands
            .spawn((
                LevelEntity,
                EnemyBillboard,
                crate::billboard::FacesCamera,
                crate::enemy_feedback::EnemyDamageFlash::default(),
                crate::enemy_feedback::EnemyHitShake::default(),
                Mesh3d(meshes.add(Rectangle::new(size, size))),
                MeshMaterial3d(material),
                Transform::from_xyz(center_x, size * 0.5 + y_offset, center_z),
            ))
            .id();
        billboards.by_key.insert(key.clone(), entity);
    }
    billboards
}

fn cell_center(col: i64, row: i64) -> (f32, f32) {
    (
        col as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        row as f32 * CELL_SIZE + CELL_SIZE / 2.0,
    )
}

pub fn tick_enemies(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut rng: ResMut<GameRng>,
    gate: crate::overlay::InputGate,
    players: Query<&Player>,
    mut render: EnemyRenderState,
    mut vitals: ResMut<crate::status_effects::PlayerVitals>,
) {
    // TS freezes enemy AI during transitions as well as overlays.
    if gate.blocked() {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let player_state = player.grid_state();
    let (player_col, player_row) = (i64::from(player_state.col), i64::from(player_state.row));

    // Snapshot door state so the AI's callbacks don't borrow the game state.
    let closed_doors: HashSet<String> = session
        .game
        .active_layer()
        .doors
        .values()
        .filter(|door| door.state == DoorState::Closed)
        .map(|door| door_key(door.col, door.row))
        .collect();
    let is_door_open = |col: i64, row: i64| !closed_doors.contains(&door_key(col, row));

    let Session {
        game,
        grid,
        walkable,
        ..
    } = &mut *session;
    let context = EnemyUpdateContext {
        player_col,
        player_row,
        grid,
        walkable,
        is_door_open: &is_door_open,
        is_hole: None,
        is_edge_blocked: None,
        enemies: &render.database.0,
    };
    let rng_ref = &mut rng.0;
    let actions = {
        let mut random = || rng_ref.next_f64();
        update_enemies(game, &context, f64::from(time.delta_secs()), &mut random)
    };

    for action in actions {
        match action.action_type {
            EnemyActionType::Idle => {}
            EnemyActionType::Move => {
                let (Some(to_col), Some(to_row)) = (action.to_col, action.to_row) else {
                    continue;
                };
                let old_key = door_key(action.from_col, action.from_row);
                if let Some(entity) = render.billboards.by_key.remove(&old_key) {
                    let new_key = door_key(to_col, to_row);
                    render.billboards.by_key.insert(new_key.clone(), entity);
                    render.feedback.health_bars.rekey(&old_key, &new_key);
                    if let Ok(mut transform) = render.transforms.get_mut(entity) {
                        let (center_x, center_z) = cell_center(to_col, to_row);
                        transform.translation.x = center_x;
                        transform.translation.z = center_z;
                    }
                }
            }
            EnemyActionType::Attack => {
                let Some(enemy_atk) = game
                    .get_enemy(action.from_col, action.from_row)
                    .map(|enemy| enemy.atk)
                else {
                    continue;
                };
                let mut random = || rng_ref.next_f64();
                let result = enemy_attack_player(game, enemy_atk, &mut random);
                vitals.flash();
                info!(
                    "Enemy hits you for {} — HP {}/{}",
                    result.damage, game.player.hp, game.player.max_hp
                );
                // Death detection is centralized in `save_load::check_player_death`,
                // which runs once per frame after all combat resolves — matching
                // where TS's own single `if (gameState.hp <= 0)` check lives,
                // rather than duplicating it at every place HP can drop to zero.
            }
            EnemyActionType::Regen => {
                let key = door_key(action.from_col, action.from_row);
                if let Some(enemy) = game.get_enemy(action.from_col, action.from_row) {
                    render.feedback.update_health_bar(
                        &mut render.visibility,
                        &mut render.item_render.materials,
                        &key,
                        enemy.hp,
                        enemy.max_hp,
                    );
                }
            }
            EnemyActionType::StatusDamage => {
                let key = door_key(action.from_col, action.from_row);
                if let Some(&entity) = render.billboards.by_key.get(&key) {
                    render.feedback.flash(entity);
                }
                if let Some(enemy) = game.get_enemy(action.from_col, action.from_row) {
                    render.feedback.update_health_bar(
                        &mut render.visibility,
                        &mut render.item_render.materials,
                        &key,
                        enemy.hp,
                        enemy.max_hp,
                    );
                }
            }
            EnemyActionType::StatusKill => {
                let key = door_key(action.from_col, action.from_row);
                if let Some(&entity) = render.billboards.by_key.get(&key) {
                    render.feedback.flash(entity);
                }
                // Unlike `damage_enemy`, the AI tick's direct hp mutation
                // doesn't remove the enemy from the map on death — do that
                // here before handing off to the shared kill effects.
                let Some(enemy) = game.active_layer_mut().enemies.remove(&key) else {
                    continue;
                };
                let target = KillTarget {
                    col: action.from_col,
                    row: action.from_row,
                    enemy_type: enemy.enemy_type,
                    drops_override: enemy.drops,
                };
                let leveled = handle_kill(
                    game,
                    rng_ref,
                    &mut render.billboards,
                    &render.database.0,
                    &render.loot_tables.0,
                    &mut render.item_render,
                    &target,
                );
                render.feedback.health_bars.remove(&key);
                if leveled {
                    render.hud.trigger_level_up(game.player.level);
                }
            }
        }
    }
}

pub fn attack_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    gate: crate::overlay::InputGate,
    mut rng: ResMut<GameRng>,
    mut billboards: ResMut<EnemyBillboards>,
    players: Query<&Player>,
    mut kill_effects: KillEffects,
) {
    if gate.blocked() || !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let results = {
        let rng = &mut rng.0;
        let mut random = || rng.next_f64();
        player_attack(player.grid_state(), &mut session.game, &mut random)
    };

    // TS triggers the swing on anything but an explicit cooldown result —
    // including an empty result list, since JS's `results[0]?.type` is
    // `undefined` (not `'cooldown'`) when there are no results at all.
    if results
        .first()
        .is_none_or(|result| result.result_type != CombatResultType::Cooldown)
    {
        kill_effects.hud.trigger_sword_swing();
    }

    for result in results {
        match result.result_type {
            CombatResultType::Hit => {
                info!(
                    "You hit the {} for {}",
                    result.enemy_type.as_deref().unwrap_or("enemy"),
                    result.damage.unwrap_or(0.0)
                );
                spawn_hit_number(&mut kill_effects, &result);
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                let key = door_key(col, row);
                if let Some(&entity) = billboards.by_key.get(&key) {
                    kill_effects.feedback.flash(entity);
                    kill_effects.feedback.trigger_hit_shake(entity);
                }
                if let Some(enemy) = session.game.get_enemy(col, row) {
                    kill_effects.feedback.update_health_bar(
                        &mut kill_effects.visibility,
                        &mut kill_effects.item_render.materials,
                        &key,
                        enemy.hp,
                        enemy.max_hp,
                    );
                }
            }
            CombatResultType::Kill => {
                info!(
                    "You slay the {}!",
                    result.enemy_type.as_deref().unwrap_or("enemy")
                );
                spawn_hit_number(&mut kill_effects, &result);
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                let key = door_key(col, row);
                if let Some(&entity) = billboards.by_key.get(&key) {
                    kill_effects.feedback.flash(entity);
                    kill_effects.feedback.trigger_hit_shake(entity);
                }
                let target = KillTarget {
                    col,
                    row,
                    enemy_type: result.enemy_type.clone().unwrap_or_default(),
                    drops_override: result.drops_override.clone(),
                };
                let leveled = handle_kill(
                    &mut session.game,
                    &mut rng.0,
                    &mut billboards,
                    &kill_effects.database.0,
                    &kill_effects.loot_tables.0,
                    &mut kill_effects.item_render,
                    &target,
                );
                kill_effects.feedback.health_bars.remove(&key);
                if leveled {
                    kill_effects.hud.trigger_level_up(session.game.player.level);
                }
            }
            CombatResultType::WallHit => {
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                spawn_hit_number(&mut kill_effects, &result);
                // TS wires wall hits through `damageBreakableWall` in
                // inputSystem.ts: hp tracking, grid mutation, and the loot
                // drop comes from the destroy outcome, not the combat
                // result's drops override.
                let Session { game, grid, .. } = &mut *session;
                let outcome =
                    game.damage_breakable_wall(col, row, result.damage.unwrap_or(0.0), grid);
                if outcome.destroyed {
                    info!("The wall crumbles!");
                    wall_entities::reveal_wall_entity(
                        &kill_effects.wall_entities,
                        &mut kill_effects.visibility,
                        &door_key(col, row),
                        false,
                    );
                    let rng = &mut rng.0;
                    let mut random = || rng.next_f64();
                    ground_items::spawn_loot(
                        game,
                        &mut kill_effects.item_render,
                        &kill_effects.loot_tables.0,
                        "",
                        outcome.drops.as_ref(),
                        (col, row),
                        &mut random,
                    );
                } else {
                    info!("You strike the wall for {}", result.damage.unwrap_or(0.0));
                }
            }
            // TS spawns a damage number for both barrel results and has no
            // HUD/log message for either — `inputSystem.ts`'s barrel arm
            // never calls `hud.showMessage`, unlike every other combat
            // result here, so none is added.
            CombatResultType::BarrelHit => {
                spawn_hit_number(&mut kill_effects, &result);
            }
            CombatResultType::BarrelDestroy => {
                spawn_hit_number(&mut kill_effects, &result);
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                barrels::despawn_barrel(
                    &mut kill_effects.barrels,
                    &mut kill_effects.item_render.commands,
                    &door_key(col, row),
                );
                if result.drops_override.is_some() {
                    let rng = &mut rng.0;
                    let mut random = || rng.next_f64();
                    ground_items::spawn_loot(
                        &mut session.game,
                        &mut kill_effects.item_render,
                        &kill_effects.loot_tables.0,
                        "",
                        result.drops_override.as_ref(),
                        (col, row),
                        &mut random,
                    );
                }
            }
            CombatResultType::NoTarget => info!("You swing at nothing."),
            CombatResultType::Cooldown => {}
            other => debug!("attack result: {other:?}"),
        }
    }
}

fn spawn_hit_number(effects: &mut KillEffects, result: &delve_core::combat::CombatResult) {
    let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
        return;
    };
    crate::damage_numbers::spawn_damage_number(
        &mut effects.item_render.commands,
        &mut effects.item_render.meshes,
        &mut effects.images,
        &mut effects.item_render.materials,
        result.damage.unwrap_or(0.0),
        (col, row),
    );
}

/// Loot tables and rendering handles the attack feedback needs — kill
/// rewards, damage numbers, hit flash/shake/health-bar updates, the
/// level-up toast, and (via `wall_entities`/`visibility`) revealing a
/// breakable wall's passage once destroyed. `visibility` is shared between
/// wall reveals and health-bar show/hide rather than each owning a separate
/// `Query<&mut Visibility>` — Bevy's per-system access check would treat
/// two such queries as conflicting even though their target entities never
/// overlap, since an unfiltered query can't be proven disjoint from anything.
#[derive(bevy::ecs::system::SystemParam)]
pub struct KillEffects<'w, 's> {
    database: Res<'w, EnemyDb>,
    loot_tables: Res<'w, LootTablesRes>,
    item_render: GroundItemRender<'w, 's>,
    images: ResMut<'w, Assets<Image>>,
    wall_entities: Res<'w, crate::wall_entities::WallEntityHandles>,
    visibility: Query<'w, 's, &'static mut Visibility>,
    feedback: crate::enemy_feedback::CombatFeedback<'w, 's>,
    hud: ResMut<'w, crate::hud::HudState>,
    barrels: ResMut<'w, BarrelHandles>,
}

/// The enemy a kill applies to — bundled so `handle_kill` stays under the
/// argument-count lint. Owned rather than borrowed: every caller reads this
/// from an `EnemyInstance` immediately before removing it from the map
/// (directly via `damage_enemy`, or indirectly via the melee/status-kill
/// paths), so a live borrow of the enemy can't be threaded through.
pub(crate) struct KillTarget {
    pub col: i64,
    pub row: i64,
    pub enemy_type: String,
    pub drops_override: Option<DropsOverride>,
}

/// XP gain and loot drop on an enemy kill, ported from the TS
/// `handleEnemyKill`. Shared by melee kills, status-effect deaths
/// (`StatusKill`), and projectile kills. Assumes the enemy is already gone
/// from the map — melee and projectile callers get that from `damage_enemy`;
/// the status-kill caller removes it itself first, since the AI tick's
/// direct hp mutation doesn't.
pub(crate) fn handle_kill(
    game: &mut GameState,
    rng: &mut Mulberry32,
    billboards: &mut EnemyBillboards,
    database: &EnemyDatabase,
    loot_tables: &LootTables,
    item_render: &mut GroundItemRender,
    target: &KillTarget,
) -> bool {
    let key = door_key(target.col, target.row);
    if let Some(entity) = billboards.by_key.remove(&key) {
        item_render.commands.entity(entity).despawn();
    }

    let mut leveled = false;
    if let Some(def) = database.get_enemy(&target.enemy_type) {
        leveled = game.add_xp(def.xp as i64);
        if leveled {
            info!("Level up! You are now level {}", game.player.level);
        }
    }

    let mut random = || rng.next_f64();
    ground_items::spawn_loot(
        game,
        item_render,
        loot_tables,
        &target.enemy_type,
        target.drops_override.as_ref(),
        (target.col, target.row),
        &mut random,
    );
    leveled
}

/// Wind down the swing cooldown. Overlay-paused like the other TS
/// per-frame ticks; keeps running through transition fades.
pub fn tick_attack_cooldown(
    time: Res<Time>,
    mut session: ResMut<Session>,
    gate: crate::overlay::InputGate,
) {
    if gate.paused() {
        return;
    }
    if session.game.player.attack_cooldown > 0.0 {
        session.game.player.attack_cooldown =
            (session.game.player.attack_cooldown - f64::from(time.delta_secs())).max(0.0);
    }
}

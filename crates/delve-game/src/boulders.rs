//! Boulder meshes and their tween/fall animator, ported from the TS
//! `boulderRenderer`/`boulderAnimator`, plus the tick shell that drives
//! `delve_core::boulders::{tick_boulders, tick_boulder_spawners}` and applies
//! the returned events. The animator is also this module's handle map (mesh
//! `Entity` lives on each animator entry) rather than a separate resource,
//! since every consumer that needs the entity also needs the tween state.

use crate::chests::{self, ChestHandles};
use crate::dungeon::{CELL_SIZE, LAYER_HEIGHT};
use crate::enemies::{self, EnemyBillboards, EnemyDb, KillTarget};
use crate::enemy_feedback::CombatFeedback;
use crate::ground_items::{GroundItemRender, LootTablesRes};
use crate::hud::HudState;
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use crate::plates::{self, PlateRender};
use crate::session::{DungeonRes, GameRng, Session, find_level_by_id};
use crate::status_effects::PlayerVitals;
use crate::textures::{canvas_to_image, seed_for};
use crate::tripwires::{self, TripwireHandles};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::boulders::{
    BoulderContext, BoulderEvent, BoulderMoved, BoulderTransitionKind, tick_boulder_spawners,
    tick_boulders,
};
use delve_core::game_state::{GameState, door_key, layer_door_key};
use delve_core::grid::Facing;
use std::collections::HashMap;

/// `CELL_SIZE * 0.4`, matching the TS `BOULDER_RADIUS` export.
const BOULDER_RADIUS: f32 = CELL_SIZE * 0.4;
/// Cells/second — TS's `BOULDER_SPEED`.
const BOULDER_SPEED: f32 = 3.0;
/// Seconds per cell of rolling — `1 / BOULDER_SPEED`.
const ROLL_DURATION: f32 = 1.0 / BOULDER_SPEED;
/// `ROLL_DURATION * 1.5` — a ramp descent takes 50% longer than a roll.
const DESCENT_DURATION: f32 = ROLL_DURATION * 1.5;
/// Matches `player.rs`'s own fall constants exactly — TS's boulder and
/// player falls share the same terminal velocity and acceleration distance.
const FALL_TERMINAL_VELOCITY: f32 = 20.0;
const FALL_ACCEL_DISTANCE: f32 = 2.0 * LAYER_HEIGHT;
const FALL_ACCEL: f32 =
    (FALL_TERMINAL_VELOCITY * FALL_TERMINAL_VELOCITY) / (2.0 * FALL_ACCEL_DISTANCE);
/// Rotation rate so the sphere's surface speed matches its translational
/// speed — true rolling, no slipping, matching TS's `ANGULAR_VELOCITY`.
const ANGULAR_VELOCITY: f32 = (CELL_SIZE * BOULDER_SPEED) / BOULDER_RADIUS;

fn rotation_axis(direction: Facing) -> Dir3 {
    match direction {
        Facing::N => Dir3::NEG_X,
        Facing::S => Dir3::X,
        Facing::E => Dir3::NEG_Z,
        Facing::W => Dir3::Z,
    }
}

fn cell_world_pos(col: i64, row: i64, layer_y_offset: f32) -> Vec3 {
    Vec3::new(
        col as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        BOULDER_RADIUS + layer_y_offset,
        row as f32 * CELL_SIZE + CELL_SIZE / 2.0,
    )
}

/// Per-pixel stone noise plus scattered dark blotches, matching TS's
/// `generateBoulderTexture` — the same noise-fill-plus-blotch shape
/// `blocks.rs`'s bevel-highlight texture uses, with blotches instead of
/// bevels.
fn generate_boulder_texture(rng: &mut CanvasRng) -> PixelCanvas {
    const SIZE: i32 = 64;
    let mut canvas = PixelCanvas::new(SIZE as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let v = 70 + rng.below(40);
            canvas.fill_rect(
                x,
                y,
                1,
                1,
                Rgba::opaque(v as u8, (v - 8).max(0) as u8, (v - 12).max(0) as u8),
            );
        }
    }
    for _ in 0..10 {
        let x = rng.below(SIZE) as f32;
        let y = rng.below(SIZE) as f32;
        let radius = 2.0 + rng.random() as f32 * 4.0;
        let alpha = 0.15 + rng.random() as f32 * 0.15;
        canvas.fill_ellipse(x, y, radius, radius, Rgba::translucent(30, 22, 18, alpha));
    }
    canvas
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BoulderAnimMode {
    Rest,
    Rolling,
    Descending,
    Falling,
}

struct BoulderAnimEntry {
    mesh: Entity,
    mode: BoulderAnimMode,
    start_pos: Vec3,
    target_pos: Vec3,
    tween_elapsed: f32,
    tween_duration: f32,
    direction: Facing,
    fall_velocity: f32,
    fall_distance: f32,
    fall_target_y: f32,
}

/// Every boulder's mesh entity and tween/fall state, keyed the same way
/// every other multi-layer handle map is. Doubles as the handle map (no
/// separate `BoulderHandles` resource) since every consumer that needs the
/// entity also needs the tween state right next to it.
#[derive(Resource, Default)]
pub struct BoulderAnimator {
    entries: HashMap<String, BoulderAnimEntry>,
}

impl BoulderAnimator {
    pub(crate) fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
    }

    fn register(&mut self, key: String, mesh: Entity, position: Vec3, direction: Facing) {
        self.entries.insert(
            key,
            BoulderAnimEntry {
                mesh,
                mode: BoulderAnimMode::Rest,
                start_pos: position,
                target_pos: position,
                tween_elapsed: 0.0,
                tween_duration: 0.0,
                direction,
                fall_velocity: 0.0,
                fall_distance: 0.0,
                fall_target_y: position.y,
            },
        );
    }

    /// Matches TS's `getMode`: an unregistered key defaults to resting, so a
    /// boulder the core spawned this same tick (not yet registered here) is
    /// immediately eligible for its first logical move next tick.
    fn is_resting(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .is_none_or(|entry| entry.mode == BoulderAnimMode::Rest)
    }

    fn rekey(&mut self, old_key: &str, new_key: &str) {
        if let Some(entry) = self.entries.remove(old_key) {
            self.entries.insert(new_key.to_string(), entry);
        }
    }

    fn start_roll(&mut self, key: &str, target_pos: Vec3, direction: Facing) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.start_pos = entry.target_pos;
            entry.target_pos = target_pos;
            entry.mode = BoulderAnimMode::Rolling;
            entry.tween_elapsed = 0.0;
            entry.tween_duration = ROLL_DURATION;
            entry.direction = direction;
        }
    }

    fn start_descend(&mut self, key: &str, target_pos: Vec3, direction: Facing) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.start_pos = entry.target_pos;
            entry.target_pos = target_pos;
            entry.mode = BoulderAnimMode::Descending;
            entry.tween_elapsed = 0.0;
            entry.tween_duration = DESCENT_DURATION;
            entry.direction = direction;
        }
    }

    /// The boulder's column/row never change during a fall — only Y, via
    /// kinematic integration in [`animate_boulders`] — so `start_pos`/
    /// `target_pos` (already at the boulder's current XZ) are left alone.
    fn start_fall(&mut self, key: &str, fall_target_y: f32, direction: Facing) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.mode = BoulderAnimMode::Falling;
            entry.fall_velocity = 0.0;
            entry.fall_distance = 0.0;
            entry.fall_target_y = fall_target_y;
            entry.direction = direction;
        }
    }
}

/// Builds a boulder mesh for every boulder on this layer, registered into
/// the animator at rest — ported from TS's `buildBoulderMeshes`.
pub fn spawn_boulders(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &delve_core::game_state::LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
) -> BoulderAnimator {
    let mut animator = BoulderAnimator::default();
    if layer_state.boulders.is_empty() {
        return animator;
    }

    let mut rng = CanvasRng::new(seed_for("boulder"));
    let texture = images.add(canvas_to_image(generate_boulder_texture(&mut rng)));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(texture),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    });
    let mesh = meshes.add(Sphere::new(BOULDER_RADIUS).mesh().uv(16, 12));

    for (key, boulder) in &layer_state.boulders {
        let position = cell_world_pos(boulder.col, boulder.row, layer_spawn.y_offset);
        let entity = commands
            .spawn((
                LevelEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(position),
            ))
            .id();
        animator.register(
            layer_door_key(layer_spawn.index, key),
            entity,
            position,
            boulder.direction,
        );
    }
    animator
}

/// Advances every boulder's roll/descend tween or fall integration one
/// frame, ported from `BoulderAnimator.update`. Ungated like every other
/// pure-visual tween in this crate (door panels, chest lids, levers) — a
/// mid-animation boulder keeps moving through overlays/transitions rather
/// than freezing mid-air.
pub fn animate_boulders(
    time: Res<Time>,
    mut animator: ResMut<BoulderAnimator>,
    mut transforms: Query<&mut Transform>,
) {
    let delta = time.delta_secs();
    for entry in animator.entries.values_mut() {
        let Ok(mut transform) = transforms.get_mut(entry.mesh) else {
            continue;
        };
        match entry.mode {
            BoulderAnimMode::Rest => continue,
            BoulderAnimMode::Rolling | BoulderAnimMode::Descending => {
                entry.tween_elapsed += delta;
                let t = (entry.tween_elapsed / entry.tween_duration).min(1.0);
                transform.translation = entry.start_pos.lerp(entry.target_pos, t);
                transform.rotate_axis(rotation_axis(entry.direction), ANGULAR_VELOCITY * delta);
                if t >= 1.0 {
                    entry.mode = BoulderAnimMode::Rest;
                    entry.start_pos = entry.target_pos;
                }
            }
            BoulderAnimMode::Falling => {
                if entry.fall_distance < FALL_ACCEL_DISTANCE {
                    entry.fall_velocity =
                        (entry.fall_velocity + FALL_ACCEL * delta).min(FALL_TERMINAL_VELOCITY);
                }
                let dy = entry.fall_velocity * delta;
                transform.translation.y -= dy;
                entry.fall_distance += dy;
                transform.rotate_axis(rotation_axis(entry.direction), ANGULAR_VELOCITY * delta);
                if transform.translation.y <= entry.fall_target_y {
                    transform.translation.y = entry.fall_target_y;
                    entry.mode = BoulderAnimMode::Rest;
                    entry.fall_velocity = 0.0;
                    entry.fall_distance = 0.0;
                    entry.start_pos = transform.translation;
                    entry.target_pos = transform.translation;
                }
            }
        }
    }
}

/// Rendering-side handles the boulder tick needs to apply `BoulderEvent`s:
/// mesh/tween updates, tripwire/plate visuals a boulder's own movement can
/// trigger, chest destruction, and the shared enemy-kill/damage-feedback
/// path melee and projectile kills already use.
#[derive(SystemParam)]
pub struct BoulderRenderState<'w, 's> {
    animator: ResMut<'w, BoulderAnimator>,
    enemy_billboards: ResMut<'w, EnemyBillboards>,
    enemy_database: Res<'w, EnemyDb>,
    loot_tables: Res<'w, LootTablesRes>,
    item_render: GroundItemRender<'w, 's>,
    feedback: CombatFeedback<'w, 's>,
    visibility: Query<'w, 's, &'static mut Visibility>,
    hud: ResMut<'w, HudState>,
    chest_handles: ResMut<'w, ChestHandles>,
    tripwire_handles: Res<'w, TripwireHandles>,
    plate_render: PlateRender<'w, 's>,
    rng: ResMut<'w, GameRng>,
    images: ResMut<'w, Assets<Image>>,
}

fn apply_boulder_moved(moved: &BoulderMoved, render: &mut BoulderRenderState) {
    let old_render_key = layer_door_key(moved.old_layer_index, &moved.old_key);
    let new_render_key = layer_door_key(moved.new_layer_index, &moved.new_key);
    render.animator.rekey(&old_render_key, &new_render_key);

    let new_layer_y_offset = moved.new_layer_index as f32 * LAYER_HEIGHT;
    let target_pos = cell_world_pos(moved.col, moved.row, new_layer_y_offset);
    match moved.kind {
        BoulderTransitionKind::Rolled => {
            render
                .animator
                .start_roll(&new_render_key, target_pos, moved.direction);
        }
        BoulderTransitionKind::Descended => {
            render
                .animator
                .start_descend(&new_render_key, target_pos, moved.direction);
        }
        BoulderTransitionKind::Fell => {
            render.animator.start_fall(
                &new_render_key,
                BOULDER_RADIUS + new_layer_y_offset,
                moved.direction,
            );
        }
    }

    if moved.tripwire_activated {
        tripwires::hide_tripwire_mesh(
            &render.tripwire_handles,
            &mut render.item_render.commands,
            &layer_door_key(moved.new_layer_index, &door_key(moved.col, moved.row)),
        );
    }
    if moved.plate_activated {
        plates::press_plate(
            &mut render.plate_render,
            &layer_door_key(moved.new_layer_index, &door_key(moved.col, moved.row)),
            new_layer_y_offset,
        );
    }
}

fn apply_boulder_events(
    events: Vec<BoulderEvent>,
    game: &mut GameState,
    render: &mut BoulderRenderState,
) {
    for event in events {
        match event {
            BoulderEvent::Moved(moved) => apply_boulder_moved(&moved, render),
            BoulderEvent::ChestCrashed {
                col,
                row,
                layer_index,
                drops,
            } => {
                let render_key = layer_door_key(layer_index, &door_key(col, row));
                chests::despawn_chest_mesh(
                    &mut render.chest_handles,
                    &mut render.item_render.commands,
                    &render_key,
                );
                if drops.is_some() {
                    let mut random = || render.rng.0.next_f64();
                    crate::ground_items::spawn_loot(
                        game,
                        &mut render.item_render,
                        &render.loot_tables.0,
                        "",
                        drops.as_ref(),
                        (col, row, layer_index),
                        &mut random,
                    );
                }
            }
            BoulderEvent::EnemyDamaged {
                key,
                col,
                row,
                damage,
                killed,
                enemy,
                layer_index,
            } => {
                let render_key = layer_door_key(layer_index, &key);
                if let Some(&entity) = render.enemy_billboards.by_key.get(&render_key) {
                    render.feedback.flash(entity);
                }
                let layer_y_offset = layer_index as f32 * LAYER_HEIGHT;
                crate::damage_numbers::spawn_damage_number(
                    &mut render.item_render.commands,
                    &mut render.item_render.meshes,
                    &mut render.images,
                    &mut render.item_render.materials,
                    damage,
                    (col, row),
                    layer_y_offset,
                );
                if killed {
                    let target = KillTarget {
                        col,
                        row,
                        enemy_type: enemy.enemy_type,
                        drops_override: enemy.drops,
                        layer_index,
                    };
                    let leveled = enemies::handle_kill(
                        game,
                        &mut render.rng.0,
                        &mut render.enemy_billboards,
                        &render.enemy_database.0,
                        &render.loot_tables.0,
                        &mut render.item_render,
                        &target,
                    );
                    render.feedback.health_bars.remove(&render_key);
                    if leveled {
                        render.hud.trigger_level_up(game.player.level);
                    }
                } else {
                    render.feedback.update_health_bar(
                        &mut render.visibility,
                        &mut render.item_render.materials,
                        &render_key,
                        enemy.hp,
                        enemy.max_hp,
                    );
                }
            }
            BoulderEvent::EnemyInstaKilled {
                key,
                col,
                row,
                enemy,
                layer_index,
            } => {
                let target = KillTarget {
                    col,
                    row,
                    enemy_type: enemy.enemy_type,
                    drops_override: enemy.drops,
                    layer_index,
                };
                let leveled = enemies::handle_kill(
                    game,
                    &mut render.rng.0,
                    &mut render.enemy_billboards,
                    &render.enemy_database.0,
                    &render.loot_tables.0,
                    &mut render.item_render,
                    &target,
                );
                render
                    .feedback
                    .health_bars
                    .remove(&layer_door_key(layer_index, &key));
                if leveled {
                    render.hud.trigger_level_up(game.player.level);
                }
            }
            BoulderEvent::Spawned {
                key,
                layer_index,
                col,
                row,
                direction,
            } => {
                let layer_y_offset = layer_index as f32 * LAYER_HEIGHT;
                let position = cell_world_pos(col, row, layer_y_offset);
                let mut rng = CanvasRng::new(seed_for("boulder"));
                let texture = render
                    .images
                    .add(canvas_to_image(generate_boulder_texture(&mut rng)));
                let material = render.item_render.materials.add(StandardMaterial {
                    base_color_texture: Some(texture),
                    perceptual_roughness: 1.0,
                    metallic: 0.0,
                    reflectance: 0.0,
                    ..default()
                });
                let mesh = render
                    .item_render
                    .meshes
                    .add(Sphere::new(BOULDER_RADIUS).mesh().uv(16, 12));
                let entity = render
                    .item_render
                    .commands
                    .spawn((
                        LevelEntity,
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        Transform::from_translation(position),
                    ))
                    .id();
                render.animator.register(
                    layer_door_key(layer_index, &key),
                    entity,
                    position,
                    direction,
                );
            }
        }
    }
}

/// Ticks every boulder's logical state and applies the resulting events.
/// Gated the same as spawners/enemy AI (`gate.blocked()`) — confirmed via
/// `gameLoop.ts`'s single `tickGameSystems` call site in `main.ts`, shared
/// by `tickBoulders`/`tickBoulderSpawners`/`tickSpawners` alike.
pub fn tick_boulders_system(
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    gate: crate::overlay::InputGate,
    mut vitals: ResMut<PlayerVitals>,
    mut render: BoulderRenderState,
) {
    if gate.blocked() {
        return;
    }
    let level_id = session.current_level_id.clone();
    let Some(level) = find_level_by_id(&dungeon, &level_id) else {
        return;
    };
    let player_layer = session.game.active_layer_index;
    let (player_col, player_row, _) = session.last_player_pose;
    let is_resting = |key: &str| render.animator.is_resting(key);

    // Split into disjoint field borrows so `game` can go mutable into
    // `tick_boulders` while `walkable` stays borrowed inside `context`.
    let Session { game, walkable, .. } = &mut *session;
    let context = BoulderContext {
        layer_defs: &level.layers,
        level_areas: level.areas.as_deref().unwrap_or(&[]),
        char_defs: level.char_defs.as_deref().unwrap_or(&[]),
        walkable,
        player_layer,
        player_col: i64::from(player_col),
        player_row: i64::from(player_row),
        debug_fullbright: false,
        is_resting: &is_resting,
    };
    let events = tick_boulders(game, &context, &mut vitals.0);
    apply_boulder_events(events, game, &mut render);
}

/// Ticks every boulder spawner and spawns a mesh for each freshly rolled
/// boulder, ported from `tickBoulderSpawners`. Same gate as
/// [`tick_boulders_system`].
pub fn tick_boulder_spawners_system(
    time: Res<Time>,
    mut session: ResMut<Session>,
    gate: crate::overlay::InputGate,
    mut render: BoulderRenderState,
) {
    if gate.blocked() {
        return;
    }
    let events = {
        let rng_ref = &mut render.rng.0;
        let mut random = || rng_ref.next_f64();
        tick_boulder_spawners(&mut session.game, f64::from(time.delta_secs()), &mut random)
    };
    apply_boulder_events(events, &mut session.game, &mut render);
}

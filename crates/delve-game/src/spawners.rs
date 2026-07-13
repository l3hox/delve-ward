//! Enemy spawner markers and the spawn-tick shell, ported from the TS
//! `spawnerRenderer`/`spawnerSystem`'s rendering half: a static floor decal
//! per visible spawner, and per-spawn-event enemy billboard + health bar
//! creation for `delve_core::spawners::tick_spawners`'s returned results.

use crate::dungeon::{CELL_SIZE, LAYER_HEIGHT};
use crate::enemies::{self, EnemyBillboards, EnemyDb};
use crate::enemy_feedback::{self, EnemyHealthBars};
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::session::{DungeonRes, GameRng, Session, find_level_by_id};
use crate::textures::canvas_to_image;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, layer_door_key};
use delve_core::spawners::{SpawnerContext, tick_spawners};
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;

const MARKER_RADIUS: f32 = 0.35;
/// Just above the floor, matching the TS decal's `y = 0.01`.
const MARKER_Y: f32 = 0.01;
const MARKER_COLOR: Rgba = Rgba::opaque(0x6b, 0x1a, 0x3a);
const MARKER_ALPHA: f32 = 0.8;
const CANVAS_SIZE: usize = 64;

/// Two ring outlines, a crosshair, and four corner dots — a static decal,
/// not seeded/randomized (unlike every noise-textured surface elsewhere in
/// this module set), matching TS's deterministic canvas drawing exactly.
fn generate_spawner_texture() -> PixelCanvas {
    let mut canvas = PixelCanvas::new(CANVAS_SIZE);
    let (center_x, center_y) = (32.0, 32.0);
    canvas.stroke_ellipse(center_x, center_y, 28.0, 28.0, MARKER_COLOR);
    canvas.stroke_ellipse(center_x, center_y, 18.0, 18.0, MARKER_COLOR);
    canvas.stroke_line(0, 32, 63, 32, MARKER_COLOR);
    canvas.stroke_line(32, 0, 32, 63, MARKER_COLOR);
    for (offset_x, offset_y) in [(-22.0, -22.0), (22.0, -22.0), (-22.0, 22.0), (22.0, 22.0)] {
        canvas.fill_ellipse(
            center_x + offset_x,
            center_y + offset_y,
            2.0,
            2.0,
            MARKER_COLOR,
        );
    }
    canvas
}

/// Spawner marker mesh entities by cell key, layer-prefixed like every
/// other multi-layer handle map.
#[derive(Resource, Default)]
pub struct SpawnerHandles {
    pub by_key: HashMap<String, Entity>,
}

/// Builds a marker decal for every *visible* spawner on this layer — TS
/// skips a mesh entirely for `spawner.visible === false` ones, and there is
/// no per-frame animation to keep a hidden entity around for.
pub fn spawn_spawner_markers(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
) -> SpawnerHandles {
    let mut handles = SpawnerHandles::default();
    if !layer_state.spawners.values().any(|spawner| spawner.visible) {
        return handles;
    }

    let texture = images.add(canvas_to_image(generate_spawner_texture()));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(texture),
        base_color: Color::WHITE.with_alpha(MARKER_ALPHA),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    let mesh = meshes.add(Circle::new(MARKER_RADIUS));

    for (key, spawner) in &layer_state.spawners {
        if !spawner.visible {
            continue;
        }
        let center_x = spawner.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = spawner.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let entity = commands
            .spawn((
                LevelEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(center_x, MARKER_Y + layer_spawn.y_offset, center_z)
                    .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
            ))
            .id();
        handles
            .by_key
            .insert(layer_door_key(layer_spawn.index, key), entity);
    }
    handles
}

/// Rendering-side handles the spawner tick needs to build a freshly spawned
/// enemy's billboard and health bar — the same pair `level_scene`'s per-layer
/// loop builds at scene-build time, created one at a time here instead.
#[derive(SystemParam)]
pub struct SpawnerRenderState<'w, 's> {
    billboards: ResMut<'w, EnemyBillboards>,
    health_bars: ResMut<'w, EnemyHealthBars>,
    database: Res<'w, EnemyDb>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    asset_server: Res<'w, AssetServer>,
    commands: Commands<'w, 's>,
}

/// Ticks every enemy spawner and builds a billboard + health bar for each
/// newly spawned enemy, ported from the TS `tickSpawners`'s spawn-event
/// handling in `spawnerSystem.ts`. Gated the same as the enemy AI tick
/// (`gate.blocked()`, not the overlay-only `paused()`) — confirmed against
/// `gameLoop.ts`'s `tickGameSystems` call site in `main.ts`, which sits
/// inside the same `!transition.isActive && !anyOverlayOpen` guard as the
/// enemy AI update.
pub fn tick_spawners_system(
    time: Res<Time>,
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut rng: ResMut<GameRng>,
    gate: crate::overlay::InputGate,
    mut render: SpawnerRenderState,
) {
    if gate.blocked() {
        return;
    }
    let level_id = session.current_level_id.clone();
    let Some(level) = find_level_by_id(&dungeon, &level_id) else {
        return;
    };
    // Cloned fresh each tick rather than cached across frames — matches how
    // `tick_enemies` already rebuilds its own read-only per-tick context
    // (`closed_doors`) from scratch every call rather than introducing a
    // level-swap-synchronized cache nothing else in this crate uses yet.
    let layer_grids: Vec<Vec<String>> = level.layers.iter().map(|def| def.grid.clone()).collect();
    let player_layer = session.game.active_layer_index;
    let (player_col, player_row, _) = session.last_player_pose;

    // Split into disjoint field borrows so `game` can go mutable into
    // `tick_spawners` while `walkable` stays borrowed inside `context`.
    let Session { game, walkable, .. } = &mut *session;
    let context = SpawnerContext {
        layer_grids: &layer_grids,
        char_defs: level.char_defs.as_deref().unwrap_or(&[]),
        walkable,
        enemies: &render.database.0,
        player_layer,
        player_col: i64::from(player_col),
        player_row: i64::from(player_row),
    };
    let rng_ref = &mut rng.0;
    let results = {
        let mut random = || rng_ref.next_f64();
        tick_spawners(game, &context, f64::from(time.delta_secs()), &mut random)
    };

    for result in results {
        // No `LayerDef` available for a mid-game spawn (only `&GameState` is
        // threaded through the core tick), so this assumes the default
        // `index * LAYER_HEIGHT` placement rather than honoring a custom
        // `LayerDef.yOffset` override — the same disclosed simplification
        // `ground_items::add_single_item_mesh` already makes for
        // gameplay-triggered spawns; no shipped level overrides `yOffset`.
        let layer_y_offset = result.layer_index as f32 * LAYER_HEIGHT;
        let entity = enemies::add_single_enemy_billboard(
            &mut render.commands,
            &mut render.meshes,
            &mut render.materials,
            &render.asset_server,
            &mut render.billboards,
            &render.database.0,
            &enemies::SingleEnemySpawn {
                layer_index: result.layer_index,
                layer_y_offset,
                cell_key: &result.cell_key,
                enemy_type: &result.enemy.enemy_type,
                col: result.enemy.col,
                row: result.enemy.row,
            },
        );
        let def = render.database.0.get_enemy(&result.enemy.enemy_type);
        let (sprite_height, _) = enemies::sprite_dimensions(def);
        let render_key = layer_door_key(result.layer_index, &result.cell_key);
        enemy_feedback::add_single_health_bar(
            &mut render.commands,
            &mut render.meshes,
            &mut render.materials,
            &mut render.health_bars,
            enemy_feedback::SingleHealthBarSpawn {
                parent: entity,
                render_key,
                sprite_height,
                hp: result.enemy.hp,
                max_hp: result.enemy.max_hp,
            },
        );
    }
}

//! Enemy billboards and AI ticking: sprites face the camera's view plane,
//! move with the core AI, and melee the player when adjacent.

use crate::barrels::{self, BarrelHandles};
use crate::dungeon::CELL_SIZE;
use crate::ground_items::{self, GroundItemRender, LootTablesRes};
use crate::level_scene::LevelEntity;
use crate::player::Player;
use crate::session::{self, DungeonRes, GameRng, Session};
use crate::wall_entities;
use bevy::prelude::*;
use delve_core::combat::{CombatResultType, enemy_attack_player, player_attack};
use delve_core::enemies::{DEFAULT_SPRITE_SIZE, EnemyDatabase};
use delve_core::enemy_ai::{EnemyActionType, EnemyUpdateContext, update_enemies};
use delve_core::game_state::{
    DoorState, GameState, LayerState, ThinWallSide, door_key, layer_door_key, thin_wall_key,
};
use delve_core::grid::get_facing_cell;
use delve_core::loot::{DropsOverride, LootTables};
use delve_core::random::Mulberry32;
use delve_core::status_effects::apply_effect;
use delve_core::types::{CharDef, DungeonLevel};
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
    dungeon: Res<'w, DungeonRes>,
    debug_flags: Res<'w, crate::debug::DebugFlags>,
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

/// An enemy billboard's position and sprite lookup key — bundled so
/// [`spawn_one_enemy_billboard`] and [`add_single_enemy_billboard`] stay
/// under the argument-count lint.
struct EnemyBillboardSpawn<'a> {
    enemy_type: &'a str,
    col: i64,
    row: i64,
    layer_y_offset: f32,
}

/// Spawns one enemy billboard and returns its entity, sized and positioned
/// from `def`/`layer_y_offset` the same way every other billboard in this
/// module is. Shared by the bulk per-layer spawn and a single runtime
/// addition (a spawner's enemy).
fn spawn_one_enemy_billboard(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    database: &EnemyDatabase,
    spawn: &EnemyBillboardSpawn,
) -> Entity {
    let def = database.get_enemy(spawn.enemy_type);
    let (size, sprite_y_offset) = sprite_dimensions(def);
    let texture_path = def
        .map(|def| sprite_asset_path(&def.sprite.path))
        .unwrap_or_else(|| "sprites/skeleton.png".to_string());

    let material = materials.add(StandardMaterial {
        base_color_texture: Some(asset_server.load(texture_path)),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Mask(0.5),
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let center_x = spawn.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
    let center_z = spawn.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
    commands
        .spawn((
            LevelEntity,
            EnemyBillboard,
            crate::billboard::FacesCamera,
            crate::enemy_feedback::EnemyDamageFlash::default(),
            crate::enemy_feedback::EnemyHitShake::default(),
            Mesh3d(meshes.add(Rectangle::new(size, size))),
            MeshMaterial3d(material),
            Transform::from_xyz(
                center_x,
                size * 0.5 + sprite_y_offset + spawn.layer_y_offset,
                center_z,
            ),
        ))
        .id()
}

pub fn spawn_enemy_billboards(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
    database: &EnemyDatabase,
) -> EnemyBillboards {
    let mut billboards = EnemyBillboards::default();
    for (key, enemy) in &layer_state.enemies {
        let entity = spawn_one_enemy_billboard(
            commands,
            meshes,
            materials,
            asset_server,
            database,
            &EnemyBillboardSpawn {
                enemy_type: &enemy.enemy_type,
                col: enemy.col,
                row: enemy.row,
                layer_y_offset: layer_spawn.y_offset,
            },
        );
        billboards
            .by_key
            .insert(layer_door_key(layer_spawn.index, key), entity);
    }
    billboards
}

/// A single runtime enemy addition's cell key alongside its billboard spawn
/// position — bundled so [`add_single_enemy_billboard`] stays under the
/// argument-count lint.
pub struct SingleEnemySpawn<'a> {
    pub layer_index: usize,
    pub layer_y_offset: f32,
    pub cell_key: &'a str,
    pub enemy_type: &'a str,
    pub col: i64,
    pub row: i64,
}

/// Adds one enemy billboard for an enemy spawned mid-game (a spawner's
/// enemy), matching TS's `createSingleEnemyMesh` — a standalone factory
/// `spawnerSystem.ts` calls per spawn event rather than reusing the bulk
/// level-load path, since that path has no per-enemy entry point of its own.
/// Returns the spawned entity so the caller can also add a health bar as a
/// child of it, matching TS's immediate `healthBarManager.create(...)` call
/// after `createSingleEnemyMesh`.
pub fn add_single_enemy_billboard(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    billboards: &mut EnemyBillboards,
    database: &EnemyDatabase,
    spawn: &SingleEnemySpawn,
) -> Entity {
    let entity = spawn_one_enemy_billboard(
        commands,
        meshes,
        materials,
        asset_server,
        database,
        &EnemyBillboardSpawn {
            enemy_type: spawn.enemy_type,
            col: spawn.col,
            row: spawn.row,
            layer_y_offset: spawn.layer_y_offset,
        },
    );
    billboards
        .by_key
        .insert(layer_door_key(spawn.layer_index, spawn.cell_key), entity);
    entity
}

fn cell_center(col: i64, row: i64) -> (f32, f32) {
    (
        col as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        row as f32 * CELL_SIZE + CELL_SIZE / 2.0,
    )
}

/// TS: `ch === '#' || (def !== undefined && def.solid && !def.seeThrough)`
/// (`main.ts:1324`) — duplicated rather than reusing `session.rs`'s private
/// `is_solid_floor_char` (same formula, but that one's `fn`-private to its
/// module and this module owns only `enemies.rs` for this change).
fn is_solid_floor_char(character: char, char_defs: &[CharDef]) -> bool {
    character == '#'
        || char_defs
            .iter()
            .find(|def| def.character == character)
            .is_some_and(|def| def.solid && def.see_through != Some(true))
}

/// TS's enemy-AI `isHole` (`main.ts:1320-1325`) — deliberately not
/// `session.rs`'s `is_hole_below` (the player's fall-trigger predicate):
/// that one fails *closed* on out-of-bounds ("not a hole"), matching its own
/// TS source (`main.ts:646-681`), while this one fails *open* ("is a hole"),
/// matching `main.ts:1321`'s explicit `return true` on an out-of-range
/// `col`/`row`. Same-looking predicates, opposite out-of-bounds polarity —
/// verified against both TS sources rather than assumed identical.
fn enemy_is_hole(below_grid: &[String], char_defs: &[CharDef], col: i64, row: i64) -> bool {
    let (Ok(row_usize), Ok(col_usize)) = (usize::try_from(row), usize::try_from(col)) else {
        return true;
    };
    let Some(character) = below_grid
        .get(row_usize)
        .and_then(|line| line.chars().nth(col_usize))
    else {
        return true;
    };
    !is_solid_floor_char(character, char_defs)
}

/// TS's `gameState.isEdgeBlocked` dispatch (`worldEntityState.ts:293-294` /
/// `:283-289`) against a caller-supplied snapshot of blocked-edge keys,
/// rather than `&GameState` directly — the AI callbacks can't borrow `game`
/// while `update_enemies` takes `&mut game`.
fn thin_wall_edge_key(from_col: i64, from_row: i64, to_col: i64, to_row: i64) -> Option<String> {
    match (to_col - from_col, to_row - from_row) {
        (0, 1) => Some(thin_wall_key(from_col, from_row, ThinWallSide::S)),
        (0, -1) => Some(thin_wall_key(to_col, to_row, ThinWallSide::S)),
        (1, 0) => Some(thin_wall_key(from_col, from_row, ThinWallSide::E)),
        (-1, 0) => Some(thin_wall_key(to_col, to_row, ThinWallSide::E)),
        _ => None,
    }
}

/// TS's `ls.layerGrids[li]`: a per-layer grid that starts from the level's
/// pristine definition and stays live-mutated for the whole session as
/// walls are destroyed on *any* layer (`activeGrid()` in `main.ts` returns
/// the very same array `damageBreakableWall`/`openSecretWall` mutate in
/// place, and `ls.layerGrids[li]` is that array — not a fresh snapshot per
/// read). Rust instead records destruction as `LayerState::destroyed_walls`
/// and replays it onto a pristine clone on demand (see
/// `transition.rs::replay_destroyed_walls`'s own doc comment for why); this
/// mirrors that replay for an arbitrary `layer_index` rather than only
/// `active_layer()`, since `transition.rs` isn't this change's file and
/// hardcodes the active layer.
fn live_layer_grid(level: &DungeonLevel, game: &GameState, layer_index: usize) -> Vec<String> {
    let mut grid = level
        .layers
        .get(layer_index)
        .map(|layer| layer.grid.clone())
        .unwrap_or_default();
    let Some(layer_state) = game.layer(layer_index) else {
        return grid;
    };
    for key in &layer_state.destroyed_walls {
        let Some((col_text, row_text)) = key.split_once(',') else {
            continue;
        };
        let (Ok(col), Ok(row)) = (col_text.parse::<usize>(), row_text.parse::<usize>()) else {
            continue;
        };
        let Some(line) = grid.get_mut(row) else {
            continue;
        };
        let mut characters: Vec<char> = line.chars().collect();
        if let Some(cell) = characters.get_mut(col) {
            *cell = '.';
            *line = characters.into_iter().collect();
        }
    }
    grid
}

#[cfg(test)]
mod tick_helper_tests {
    use super::*;

    fn char_def(character: char, solid: bool, see_through: Option<bool>) -> CharDef {
        CharDef {
            character,
            solid,
            see_through,
            textures: delve_core::types::TextureSet::default(),
        }
    }

    #[test]
    fn is_solid_floor_char_treats_hash_as_solid_regardless_of_char_defs() {
        assert!(is_solid_floor_char('#', &[]));
    }

    #[test]
    fn is_solid_floor_char_is_true_for_solid_non_see_through_chars() {
        let defs = [char_def('F', true, None)];
        assert!(is_solid_floor_char('F', &defs));
    }

    #[test]
    fn is_solid_floor_char_is_false_for_solid_see_through_chars() {
        let defs = [char_def('T', true, Some(true))];
        assert!(!is_solid_floor_char('T', &defs));
    }

    #[test]
    fn is_solid_floor_char_is_false_for_undefined_chars() {
        assert!(!is_solid_floor_char('.', &[]));
    }

    #[test]
    fn enemy_is_hole_is_true_out_of_bounds() {
        let grid = vec!["##".to_string(), "##".to_string()];
        assert!(enemy_is_hole(&grid, &[], -1, 0));
        assert!(enemy_is_hole(&grid, &[], 0, -1));
        assert!(enemy_is_hole(&grid, &[], 5, 0));
    }

    #[test]
    fn enemy_is_hole_is_false_over_solid_floor_and_true_over_open_floor() {
        let grid = vec!["#.".to_string()];
        let defs = [char_def('.', false, None)];
        assert!(!enemy_is_hole(&grid, &defs, 0, 0));
        assert!(enemy_is_hole(&grid, &defs, 1, 0));
    }

    #[test]
    fn thin_wall_edge_key_covers_all_four_cardinal_moves() {
        assert_eq!(
            thin_wall_edge_key(1, 1, 1, 2),
            Some(thin_wall_key(1, 1, ThinWallSide::S))
        );
        assert_eq!(
            thin_wall_edge_key(1, 2, 1, 1),
            Some(thin_wall_key(1, 1, ThinWallSide::S))
        );
        assert_eq!(
            thin_wall_edge_key(1, 1, 2, 1),
            Some(thin_wall_key(1, 1, ThinWallSide::E))
        );
        assert_eq!(
            thin_wall_edge_key(2, 1, 1, 1),
            Some(thin_wall_key(1, 1, ThinWallSide::E))
        );
    }

    #[test]
    fn thin_wall_edge_key_is_none_for_a_non_adjacent_move() {
        assert_eq!(thin_wall_edge_key(1, 1, 2, 2), None);
        assert_eq!(thin_wall_edge_key(1, 1, 1, 1), None);
    }
}

/// Mirrors `main.ts:1308-1330`'s enemy-AI tick: every layer updates in
/// real time, not just the player's active one, with a per-layer `isHole`
/// (sourced from the layer below) and `isEdgeBlocked` (thin walls) wired
/// in. `active_layer_index` is saved and restored around the loop exactly
/// like `tick_boulders` (`delve-core/src/boulders.rs`) already does for the
/// same reason: everything `update_enemies` and the action-application
/// loop below it touch — `game.active_layer()`, `layer_door_key`,
/// `KillTarget.layer_index` — reads whichever layer is "active" at the
/// time, so the loop iteration variable has to *be* that layer for the
/// whole time its actions are applied, not just for the `update_enemies`
/// call itself.
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

    // Owned rather than borrowed from `render.dungeon`: `render`'s other
    // fields (billboards, feedback, ...) need `&mut` access further down,
    // and this way that borrow of `render.dungeon` is done and gone before
    // any of that starts, instead of quietly overlapping with it.
    let Some(level) = session::find_level_by_id(&render.dungeon, &session.current_level_id) else {
        return;
    };
    let char_defs: Vec<CharDef> = level.char_defs.clone().unwrap_or_default();
    // Built once per tick, ahead of the mutable `game` split below — every
    // layer's grid, live-mutated per `live_layer_grid`'s doc comment, since
    // TS passes the *same* live grid array whether or not it belongs to
    // the player's current layer.
    let live_grids: Vec<Vec<String>> = (0..session.game.layers.len())
        .map(|layer_index| live_layer_grid(level, &session.game, layer_index))
        .collect();

    let saved_layer = session.game.active_layer_index;
    let Session { game, walkable, .. } = &mut *session;

    for layer_index in 0..live_grids.len() {
        game.active_layer_index = layer_index;

        // Snapshot per-layer door/thin-wall state so the AI's callbacks
        // don't borrow `game` while `update_enemies` takes `&mut game` —
        // the same pattern the pre-existing `closed_doors` snapshot below
        // already used for one layer, now redone fresh each iteration.
        let closed_doors: HashSet<String> = game
            .active_layer()
            .doors
            .values()
            .filter(|door| door.state == DoorState::Closed)
            .map(|door| door_key(door.col, door.row))
            .collect();
        let is_door_open = |col: i64, row: i64| !closed_doors.contains(&door_key(col, row));

        let blocked_edges: HashSet<String> =
            game.active_layer().thin_walls.keys().cloned().collect();
        let is_edge_blocked = |from_col: i64, from_row: i64, to_col: i64, to_row: i64| {
            thin_wall_edge_key(from_col, from_row, to_col, to_row)
                .is_some_and(|key| blocked_edges.contains(&key))
        };

        // `None` below-layer (the ground floor) makes `enemy_is_hole`
        // unreachable, but the closure still needs a body — `is_some_and`
        // on `below_grid` folds "no layer below" into "never a hole",
        // exactly matching TS's `isHole = undefined` for `li === 0`
        // (`context.is_hole.is_some_and(...)` is `false` either way).
        let below_grid = layer_index.checked_sub(1).map(|below| &live_grids[below]);
        let is_hole =
            |col: i64, row: i64| below_grid.is_some_and(|g| enemy_is_hole(g, &char_defs, col, row));

        let context = EnemyUpdateContext {
            player_col,
            player_row,
            grid: &live_grids[layer_index],
            walkable,
            is_door_open: &is_door_open,
            is_hole: Some(&is_hole),
            is_edge_blocked: Some(&is_edge_blocked),
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
                    let old_key = layer_door_key(
                        game.active_layer_index,
                        &door_key(action.from_col, action.from_row),
                    );
                    if let Some(entity) = render.billboards.by_key.remove(&old_key) {
                        let new_key =
                            layer_door_key(game.active_layer_index, &door_key(to_col, to_row));
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
                    // TS: `if (li === savedLayer && !debugFullbright)` —
                    // an enemy can only land a hit on the layer the player
                    // is actually standing on; `player_col`/`player_row`
                    // alone can't tell layers apart, since TS passes the
                    // same player position to every layer's AI tick.
                    // Fullbright is invincibility (`main.ts:976`,
                    // `playerController.ts:22/36` suppress every other
                    // source of player damage the same way), so it blocks
                    // this hit outright regardless of layer.
                    if layer_index != saved_layer || render.debug_flags.fullbright {
                        continue;
                    }
                    let Some((enemy_atk, enemy_type)) = game
                        .get_enemy(action.from_col, action.from_row)
                        .map(|enemy| (enemy.atk, enemy.enemy_type.clone()))
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

                    // TS: `onHitBehavior && Math.random() < (params.chance
                    // as number)` -> `applyEffect(gameState.playerStatusEffects,
                    // params.statusEffect as StatusEffectType, params.duration
                    // as number)` (`main.ts:1345-1348`). Nothing reads
                    // `onHit`'s params yet, so `chance`/`duration` are read
                    // the way `enemy_ai.rs`'s "erratic" behavior already reads
                    // its own `chance` param (`.params.get(...).and_then(|v|
                    // v.as_f64())`) -- the closest established convention,
                    // not a literal `onHit` precedent. A missing `chance`
                    // compares false against any roll, matching TS's
                    // `Math.random() < undefined` always-false NaN comparison.
                    // A missing or unrecognized `statusEffect` skips the
                    // apply outright rather than guessing a type; a missing
                    // `duration` falls back to `0.0`, the least-harmful
                    // stand-in for TS's unchecked `as number` on `undefined`.
                    if let Some(behavior) = render.database.0.get_behavior(&enemy_type, "onHit") {
                        let chance = behavior
                            .params
                            .get("chance")
                            .and_then(|value| value.as_f64())
                            .unwrap_or(0.0);
                        if rng_ref.next_f64() < chance {
                            let effect_type = behavior
                                .params
                                .get("statusEffect")
                                .and_then(|value| value.as_str())
                                .and_then(crate::projectiles::parse_status_effect_type);
                            if let Some(effect_type) = effect_type {
                                let duration = behavior
                                    .params
                                    .get("duration")
                                    .and_then(|value| value.as_f64())
                                    .unwrap_or(0.0);
                                apply_effect(
                                    &mut game.status_fx.player_status_effects,
                                    effect_type,
                                    duration,
                                );
                            }
                        }
                    }
                }
                EnemyActionType::Regen => {
                    let key = door_key(action.from_col, action.from_row);
                    let render_key = layer_door_key(game.active_layer_index, &key);
                    if let Some(enemy) = game.get_enemy(action.from_col, action.from_row) {
                        render.feedback.update_health_bar(
                            &mut render.visibility,
                            &mut render.item_render.materials,
                            &render_key,
                            enemy.hp,
                            enemy.max_hp,
                        );
                    }
                }
                EnemyActionType::StatusDamage => {
                    let key = door_key(action.from_col, action.from_row);
                    let render_key = layer_door_key(game.active_layer_index, &key);
                    if let Some(&entity) = render.billboards.by_key.get(&render_key) {
                        render.feedback.flash(entity);
                    }
                    if let Some(enemy) = game.get_enemy(action.from_col, action.from_row) {
                        render.feedback.update_health_bar(
                            &mut render.visibility,
                            &mut render.item_render.materials,
                            &render_key,
                            enemy.hp,
                            enemy.max_hp,
                        );
                    }
                }
                EnemyActionType::StatusKill => {
                    let key = door_key(action.from_col, action.from_row);
                    let render_key = layer_door_key(game.active_layer_index, &key);
                    if let Some(&entity) = render.billboards.by_key.get(&render_key) {
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
                        layer_index: game.active_layer_index,
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
                    render.feedback.health_bars.remove(&render_key);
                    if leveled {
                        render.hud.trigger_level_up(game.player.level);
                    }
                }
            }
        }
    }

    game.active_layer_index = saved_layer;
}

pub fn attack_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    gate: crate::overlay::InputGate,
    mut rng: ResMut<GameRng>,
    mut billboards: ResMut<EnemyBillboards>,
    mut players: Query<&mut Player>,
    mut kill_effects: KillEffects,
) {
    if gate.blocked() || !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    let Ok(mut player) = players.single_mut() else {
        return;
    };
    // TS: `if (ctx.debugState.debugFullbright) { ... enemy.hp = 1; }`
    // (`main.ts:242-247`) — sets the facing enemy's hp to 1 right before
    // the swing resolves, so any nonzero weapon damage kills it outright.
    if kill_effects.debug_flags.fullbright {
        let (facing_col, facing_row) = get_facing_cell(player.grid_state());
        let key = door_key(i64::from(facing_col), i64::from(facing_row));
        if let Some(enemy) = session.game.active_layer_mut().enemies.get_mut(&key) {
            enemy.hp = 1.0;
        }
    }
    let layer_y_offset = session.game.active_layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
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
                spawn_hit_number(&mut kill_effects, &result, layer_y_offset);
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                let render_key =
                    layer_door_key(session.game.active_layer_index, &door_key(col, row));
                if let Some(&entity) = billboards.by_key.get(&render_key) {
                    kill_effects.feedback.flash(entity);
                    kill_effects.feedback.trigger_hit_shake(entity);
                }
                if let Some(enemy) = session.game.get_enemy(col, row) {
                    kill_effects.feedback.update_health_bar(
                        &mut kill_effects.visibility,
                        &mut kill_effects.item_render.materials,
                        &render_key,
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
                spawn_hit_number(&mut kill_effects, &result, layer_y_offset);
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                let render_key =
                    layer_door_key(session.game.active_layer_index, &door_key(col, row));
                if let Some(&entity) = billboards.by_key.get(&render_key) {
                    kill_effects.feedback.flash(entity);
                    kill_effects.feedback.trigger_hit_shake(entity);
                }
                let target = KillTarget {
                    col,
                    row,
                    enemy_type: result.enemy_type.clone().unwrap_or_default(),
                    drops_override: result.drops_override.clone(),
                    layer_index: session.game.active_layer_index,
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
                kill_effects.feedback.health_bars.remove(&render_key);
                if leveled {
                    kill_effects.hud.trigger_level_up(session.game.player.level);
                }
            }
            CombatResultType::WallHit => {
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                spawn_hit_number(&mut kill_effects, &result, layer_y_offset);
                // TS wires wall hits through `damageBreakableWall` in
                // inputSystem.ts: hp tracking, grid mutation, and the loot
                // drop comes from the destroy outcome, not the combat
                // result's drops override.
                let Session { game, grid, .. } = &mut *session;
                let wall_layer_index = game.active_layer_index;
                let outcome =
                    game.damage_breakable_wall(col, row, result.damage.unwrap_or(0.0), grid);
                if outcome.destroyed {
                    info!("The wall crumbles!");
                    // The passage the crumbled wall leaves behind has to open
                    // in the player's own grid copy too, or the cell stays
                    // unwalkable — see `Player::open_cell`.
                    player.open_cell(col, row);
                    wall_entities::reveal_wall_entity(
                        &kill_effects.wall_entities,
                        &mut kill_effects.visibility,
                        &layer_door_key(wall_layer_index, &door_key(col, row)),
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
                        (col, row, wall_layer_index),
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
                spawn_hit_number(&mut kill_effects, &result, layer_y_offset);
            }
            CombatResultType::BarrelDestroy => {
                spawn_hit_number(&mut kill_effects, &result, layer_y_offset);
                let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
                    continue;
                };
                let barrel_layer_index = session.game.active_layer_index;
                barrels::despawn_barrel(
                    &mut kill_effects.barrels,
                    &mut kill_effects.item_render.commands,
                    &layer_door_key(barrel_layer_index, &door_key(col, row)),
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
                        (col, row, barrel_layer_index),
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

fn spawn_hit_number(
    effects: &mut KillEffects,
    result: &delve_core::combat::CombatResult,
    layer_y_offset: f32,
) {
    let (Some(col), Some(row)) = (result.target_col, result.target_row) else {
        return;
    };
    crate::damage_numbers::spawn_damage_number(
        &mut effects.item_render.commands,
        &mut effects.item_render.meshes,
        &mut effects.images,
        &mut effects.damage_number_materials,
        result.damage.unwrap_or(0.0),
        (col, row),
        layer_y_offset,
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
    damage_number_materials: ResMut<'w, Assets<crate::damage_numbers::DamageNumberMaterial>>,
    wall_entities: Res<'w, crate::wall_entities::WallEntityHandles>,
    visibility: Query<'w, 's, &'static mut Visibility>,
    feedback: crate::enemy_feedback::CombatFeedback<'w, 's>,
    hud: ResMut<'w, crate::hud::HudState>,
    barrels: ResMut<'w, BarrelHandles>,
    debug_flags: Res<'w, crate::debug::DebugFlags>,
}

/// The enemy a kill applies to — bundled so `handle_kill` stays under the
/// argument-count lint. Owned rather than borrowed: every caller reads this
/// from an `EnemyInstance` immediately before removing it from the map
/// (directly via `damage_enemy`, or indirectly via the melee/status-kill
/// paths), so a live borrow of the enemy can't be threaded through.
/// `layer_index` names the layer `target` lived on — usually
/// `game.active_layer_index` (melee/status kills only ever target the
/// player's own layer), but a boulder can insta-kill or finish off an enemy
/// on a layer the player isn't currently on, so callers set it explicitly
/// rather than `handle_kill` assuming the active layer.
pub(crate) struct KillTarget {
    pub col: i64,
    pub row: i64,
    pub enemy_type: String,
    pub drops_override: Option<DropsOverride>,
    pub layer_index: usize,
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
    let render_key = layer_door_key(target.layer_index, &door_key(target.col, target.row));
    if let Some(entity) = billboards.by_key.remove(&render_key) {
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
        (target.col, target.row, target.layer_index),
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

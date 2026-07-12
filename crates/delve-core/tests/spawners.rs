//! `spawnerSystem.ts` has no dedicated vitest suite (it's a `game/`
//! orchestrator, not a `core/` unit under test in TS). This is a
//! from-scratch behavioral spec for `delve_core::spawners`, covering BFS
//! placement, activation/interval gating, and every occupancy/hole rule
//! the TS implementation encodes.

use delve_core::enemies::EnemyDatabase;
use delve_core::game_state::{BlockInstance, GameState, GameStateDeps, SpawnerInstance, door_key};
use delve_core::grid::walkable_cells;
use delve_core::spawners::{SpawnerContext, tick_spawners};
use delve_core::types::{EnemyInstance, LayerDef};

const ENEMIES_JSON: &str = include_str!("fixtures/spawners-enemies-mock.json");

fn enemy_db() -> EnemyDatabase {
    EnemyDatabase::from_json(ENEMIES_JSON).expect("fixture enemies parse")
}

fn rows(rows: &[&str]) -> Vec<String> {
    rows.iter().map(ToString::to_string).collect()
}

/// A fully solid support layer (no holes anywhere above it) and a plain
/// walkable room above it — used for tests unrelated to hole traversal.
fn open_support_grid() -> Vec<String> {
    rows(&["#####", "#####", "#####", "#####", "#####"])
}

fn open_room_grid() -> Vec<String> {
    rows(&["#####", "#...#", "#...#", "#...#", "#####"])
}

/// A single-row corridor where the support layer opens a hole at column 2
/// (row 1); the only path from the spawner at (1,1) to (3,1) runs through
/// that hole, so it also proves whether BFS traversal itself is blocked,
/// not just candidacy at the hole cell.
fn corridor_support_grid() -> Vec<String> {
    rows(&["#####", "##.##", "#####"])
}

fn corridor_room_grid() -> Vec<String> {
    rows(&["#####", "#...#", "#####"])
}

fn layer_def(grid: Vec<String>) -> LayerDef {
    LayerDef {
        id: None,
        y_offset: None,
        grid,
        entities: Vec::new(),
        ceiling: None,
        defaults: None,
        areas: None,
    }
}

/// Layer 0 is the support grid below, layer 1 is the room under test.
/// `GameState::new` resets `active_layer_index` to 0 once every layer's
/// (empty) entities are parsed — point it at layer 1 so the test's own
/// `active_layer_mut()`/`active_layer()` calls reach the room, matching
/// what every test in this file assumes.
fn game_with_layers(grids: [Vec<String>; 2]) -> GameState {
    let [below, above] = grids;
    let layers = [layer_def(below), layer_def(above)];
    let mut game = GameState::new(
        &[],
        None,
        "test",
        Some(&layers),
        GameStateDeps::default(),
        &mut || 0.5,
    );
    game.active_layer_index = 1;
    game
}

fn make_spawner(
    id: Option<&str>,
    col: i64,
    row: i64,
    enemy_type: &str,
    spawn_radius: f64,
) -> SpawnerInstance {
    SpawnerInstance {
        id: id.map(ToString::to_string),
        col,
        row,
        enemy_type: enemy_type.to_string(),
        max_active: 5.0,
        interval: 0.0,
        spawn_radius,
        active: true,
        visible: true,
        gate_mode: None,
        spawn_timer: 0.0,
    }
}

fn make_enemy(col: i64, row: i64, spawner_id: Option<&str>) -> EnemyInstance {
    EnemyInstance {
        col,
        row,
        enemy_type: "rat_test".to_string(),
        hp: 8.0,
        max_hp: 8.0,
        atk: 2.0,
        def: 0.0,
        aggro_range: 3.0,
        move_interval: 0.6,
        blocks_movement: true,
        ai_state: delve_core::types::EnemyAiState::Idle,
        move_timer: 0.0,
        regen_timer: None,
        regen_pause_timer: None,
        drops: None,
        status_effects: Vec::new(),
        spawner_id: spawner_id.map(ToString::to_string),
    }
}

fn context<'a>(
    layer_grids: &'a [Vec<String>],
    walkable: &'a std::collections::HashSet<char>,
    enemies: &'a EnemyDatabase,
    player_layer: usize,
    player_col: i64,
    player_row: i64,
) -> SpawnerContext<'a> {
    SpawnerContext {
        layer_grids,
        char_defs: &[],
        walkable,
        enemies,
        player_layer,
        player_col,
        player_row,
    }
}

#[test]
fn spawns_within_radius_and_tags_the_new_enemy_with_the_spawner_id() {
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    game.active_layer_mut().spawners.insert(
        door_key(2, 1),
        make_spawner(Some("sp1"), 2, 1, "rat_test", 2.0),
    );

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert_eq!(results.len(), 1);
    let spawned = &results[0];
    assert_eq!(spawned.layer_index, 1);
    assert_eq!(spawned.enemy.spawner_id.as_deref(), Some("sp1"));
    assert_eq!(spawned.enemy.enemy_type, "rat_test");
    let in_state = game
        .active_layer()
        .enemies
        .get(&spawned.cell_key)
        .expect("enemy inserted");
    assert_eq!(in_state.spawner_id.as_deref(), Some("sp1"));
}

#[test]
fn does_not_spawn_before_interval_elapses_but_still_accumulates() {
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    let mut spawner = make_spawner(Some("sp1"), 2, 1, "rat_test", 2.0);
    spawner.interval = 5.0;
    game.active_layer_mut()
        .spawners
        .insert(door_key(2, 1), spawner);

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert!(results.is_empty());
    let spawner = game.active_layer().spawners.get(&door_key(2, 1)).unwrap();
    assert_eq!(spawner.spawn_timer, 1.0);
}

#[test]
fn spawn_timer_overshoot_carries_over_instead_of_resetting_to_zero() {
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    let mut spawner = make_spawner(Some("sp1"), 2, 1, "rat_test", 2.0);
    spawner.interval = 2.0;
    game.active_layer_mut()
        .spawners
        .insert(door_key(2, 1), spawner);

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 5.0, &mut || 0.5);

    assert_eq!(results.len(), 1);
    let spawner = game.active_layer().spawners.get(&door_key(2, 1)).unwrap();
    assert_eq!(spawner.spawn_timer, 3.0);
}

#[test]
fn inactive_spawner_never_fires_and_never_accumulates_its_timer() {
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    let mut spawner = make_spawner(Some("sp1"), 2, 1, "rat_test", 2.0);
    spawner.active = false;
    game.active_layer_mut()
        .spawners
        .insert(door_key(2, 1), spawner);

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert!(results.is_empty());
    let spawner = game.active_layer().spawners.get(&door_key(2, 1)).unwrap();
    assert_eq!(spawner.spawn_timer, 0.0);
}

#[test]
fn max_active_gates_spawning_once_the_cap_is_reached() {
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    let mut spawner = make_spawner(Some("sp1"), 2, 1, "rat_test", 2.0);
    spawner.max_active = 1.0;
    game.active_layer_mut()
        .spawners
        .insert(door_key(2, 1), spawner);
    game.active_layer_mut()
        .enemies
        .insert(door_key(1, 1), make_enemy(1, 1, Some("sp1")));

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert!(results.is_empty());
}

#[test]
fn id_less_spawner_counts_every_id_less_enemy_toward_its_own_cap() {
    // Mirrors the TS `enemy.spawnerId === spawner.id` quirk: with both
    // sides `undefined`/`None`, an id-less spawner treats any id-less
    // enemy as if it were its own, even one it never spawned.
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    let mut spawner = make_spawner(None, 2, 1, "rat_test", 2.0);
    spawner.max_active = 1.0;
    game.active_layer_mut()
        .spawners
        .insert(door_key(2, 1), spawner);
    game.active_layer_mut()
        .enemies
        .insert(door_key(1, 1), make_enemy(1, 1, None));

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert!(results.is_empty());
}

#[test]
fn candidate_selection_excludes_enemy_block_and_player_occupied_cells() {
    // Spawner at dead center (2,2) of a 5x5 room, radius 1: the four
    // cardinal neighbors are the only candidates. Three are occupied
    // (enemy, block, player); only the east cell is free, so the spawn
    // must land there regardless of the RNG draw.
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    game.active_layer_mut().spawners.insert(
        door_key(2, 2),
        make_spawner(Some("sp1"), 2, 2, "rat_test", 1.0),
    );
    game.active_layer_mut()
        .enemies
        .insert(door_key(2, 1), make_enemy(2, 1, Some("other")));
    game.active_layer_mut().blocks.insert(
        door_key(2, 3),
        BlockInstance {
            id: None,
            col: 2,
            row: 3,
        },
    );

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, 1, 2); // player occupies the west neighbor

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.99);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].cell_key, door_key(3, 2));
}

#[test]
fn no_candidates_within_radius_skips_the_spawn() {
    // Spawner boxed in on every side by an occupied neighbor.
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    game.active_layer_mut().spawners.insert(
        door_key(2, 2),
        make_spawner(Some("sp1"), 2, 2, "rat_test", 1.0),
    );
    for (col, row) in [(2, 1), (2, 3), (1, 2), (3, 2)] {
        game.active_layer_mut()
            .enemies
            .insert(door_key(col, row), make_enemy(col, row, Some("other")));
    }

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert!(results.is_empty());
    // The spawner still consumed its interval — only the search failed.
    let spawner = game.active_layer().spawners.get(&door_key(2, 2)).unwrap();
    assert_eq!(spawner.spawn_timer, 1.0);
}

#[test]
fn ground_enemy_cannot_traverse_or_spawn_across_a_hole_corridor() {
    let mut game = game_with_layers([corridor_support_grid(), corridor_room_grid()]);
    game.active_layer_mut().spawners.insert(
        door_key(1, 1),
        make_spawner(Some("sp1"), 1, 1, "rat_test", 2.0),
    );

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [corridor_support_grid(), corridor_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert!(results.is_empty());
}

#[test]
fn flying_enemy_traverses_and_spawns_across_a_hole_corridor() {
    let mut game = game_with_layers([corridor_support_grid(), corridor_room_grid()]);
    game.active_layer_mut().spawners.insert(
        door_key(1, 1),
        make_spawner(Some("sp1"), 1, 1, "bat_test", 2.0),
    );

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [corridor_support_grid(), corridor_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert_eq!(results.len(), 1);
    assert!(results[0].cell_key == door_key(2, 1) || results[0].cell_key == door_key(3, 1));
}

#[test]
fn unknown_enemy_type_is_skipped_without_panicking() {
    let mut game = game_with_layers([open_support_grid(), open_room_grid()]);
    game.active_layer_mut().spawners.insert(
        door_key(2, 1),
        make_spawner(Some("sp1"), 2, 1, "no_such_enemy", 2.0),
    );

    let db = enemy_db();
    let walkable = walkable_cells();
    let grids = [open_support_grid(), open_room_grid()];
    let ctx = context(&grids, &walkable, &db, 1, -99, -99);

    let results = tick_spawners(&mut game, &ctx, 1.0, &mut || 0.5);

    assert!(results.is_empty());
}

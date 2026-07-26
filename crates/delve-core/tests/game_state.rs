//! Ported from `src/core/gameState.test.ts` and `gameStateInventory.test.ts`.
//! The TS `vi.mock('./itemDatabase')` stubs become an item database fixture;
//! registrar singletons stay unset exactly like in the TS test module.

use delve_core::boulders::can_boulder_roll_to;
use delve_core::entities::{EquipSlot, ItemLocation};
use delve_core::game_state::{
    BoulderState, ChestState, DoorState, GameState, GameStateDeps, LeverState, PitTrapState,
    ThinWallSide, UsableState, door_key,
};
use delve_core::grid::{Facing, walkable_cells};
use delve_core::inventory_state::AllocatableStat;
use delve_core::items::{ItemDatabase, ItemQuality};
use delve_core::status_effect_state::BuffStat;
use delve_core::types::{Entity, LayerDef};
use serde_json::{Value, json};
use std::sync::Arc;

const ITEMS_JSON: &str = include_str!("fixtures/game-state-items-mock.json");

fn deps() -> GameStateDeps {
    GameStateDeps {
        items: Some(Arc::new(
            ItemDatabase::from_json(ITEMS_JSON).expect("mock items parse"),
        )),
        enemy_registrar: None,
        npc_registrar: None,
    }
}

fn entities(values: Value) -> Vec<Entity> {
    serde_json::from_value(values).expect("test entities parse")
}

fn gs(entity_values: Value) -> GameState {
    GameState::new(
        &entities(entity_values),
        None,
        "default",
        None,
        deps(),
        &mut || 0.5,
    )
}

fn gs_with_grid(entity_values: Value, rows: &[&str]) -> GameState {
    let grid: Vec<String> = rows.iter().map(ToString::to_string).collect();
    GameState::new(
        &entities(entity_values),
        Some(&grid),
        "default",
        None,
        deps(),
        &mut || 0.5,
    )
}

fn gs_level(entity_values: Value, level_id: &str) -> GameState {
    GameState::new(
        &entities(entity_values),
        None,
        level_id,
        None,
        deps(),
        &mut || 0.5,
    )
}

/// Wrap entities into a single-layer LayerDef array for load_new_level.
fn as_layer(entity_values: Value) -> Vec<LayerDef> {
    serde_json::from_value(json!([{
        "id": "0",
        "grid": ["...", "...", "..."],
        "entities": entity_values,
    }]))
    .expect("layer def parses")
}

fn door(col: i64, row: i64, state: &str) -> Value {
    json!({ "col": col, "row": row, "type": "door", "state": state })
}

fn locked_door(col: i64, row: i64, state: &str, key_id: &str) -> Value {
    json!({ "col": col, "row": row, "type": "door", "state": state, "keyId": key_id })
}

// --- Constructor and doors ---

#[test]
fn constructor_extracts_doors_and_defaults_state() {
    let state = gs(json!([
        door(1, 2, "closed"),
        locked_door(3, 4, "closed", "gold_key")
    ]));
    assert_eq!(state.active_layer().doors.len(), 2);
    assert!(state.get_door(1, 2).is_some());
    assert!(state.get_door(3, 4).is_some());

    let defaulted = gs(json!([{ "col": 1, "row": 1, "type": "door" }]));
    assert_eq!(
        defaulted.get_door(1, 1).expect("door present").state,
        DoorState::Closed
    );

    let mixed = gs(json!([
        { "col": 1, "row": 1, "type": "enemy" },
        door(2, 2, "closed"),
        { "col": 3, "row": 3, "type": "key", "keyId": "silver_key" },
    ]));
    assert_eq!(mixed.active_layer().doors.len(), 1);

    assert!(gs(json!([])).player.inventory.is_empty());
}

#[test]
fn get_door_returns_correct_instance() {
    let state = gs(json!([locked_door(5, 6, "closed", "key1")]));
    let door = state.get_door(5, 6).expect("door present");
    assert_eq!(door.col, 5);
    assert_eq!(door.row, 6);
    assert_eq!(door.state, DoorState::Closed);
    assert_eq!(door.key_id.as_deref(), Some("key1"));
    assert!(state.get_door(9, 9).is_none());
}

#[test]
fn is_door_open_semantics() {
    assert!(gs(json!([door(1, 1, "open")])).is_door_open(1, 1));
    assert!(!gs(json!([door(1, 1, "closed")])).is_door_open(1, 1));
    assert!(gs(json!([])).is_door_open(5, 5));
}

#[test]
fn open_door_state_machine() {
    let mut state = gs(json!([door(1, 1, "closed")]));
    assert!(state.open_door(1, 1));
    assert_eq!(state.get_door(1, 1).expect("door").state, DoorState::Open);
    assert!(!state.open_door(1, 1));

    let mut keyed = gs(json!([locked_door(1, 1, "closed", "gold_key")]));
    assert!(!keyed.open_door(1, 1));
    assert_eq!(keyed.get_door(1, 1).expect("door").state, DoorState::Closed);
    keyed.add_key("gold_key");
    assert!(keyed.open_door(1, 1));

    let mut wrong_key = gs(json!([locked_door(1, 1, "closed", "gold_key")]));
    wrong_key.add_key("silver_key");
    assert!(!wrong_key.open_door(1, 1));

    let mut mechanical = gs(json!([
        { "col": 3, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 1, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
    ]));
    assert!(!mechanical.open_door(3, 2));
    assert_eq!(
        mechanical.get_door(3, 2).expect("door").state,
        DoorState::Closed
    );
}

#[test]
fn toggle_and_close_door() {
    let mut state = gs(json!([door(1, 1, "open")]));
    state.toggle_door(1, 1);
    assert_eq!(state.get_door(1, 1).expect("door").state, DoorState::Closed);
    state.toggle_door(1, 1);
    assert_eq!(state.get_door(1, 1).expect("door").state, DoorState::Open);

    assert!(state.close_door(1, 1));
    assert_eq!(state.get_door(1, 1).expect("door").state, DoorState::Closed);
    assert!(!state.close_door(1, 1));
    assert!(!state.close_door(9, 9));

    // toggling a non-existent door is a no-op
    state.toggle_door(99, 99);

    let mut mechanical = gs(json!([
        { "col": 3, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 1, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
    ]));
    mechanical.activate_lever(1, 1);
    assert_eq!(
        mechanical.get_door(3, 2).expect("door").state,
        DoorState::Open
    );
    assert!(!mechanical.close_door(3, 2));
}

#[test]
fn doors_targeted_by_sources_are_mechanical() {
    let lever_target = gs(json!([
        { "col": 5, "row": 3, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
    ]));
    assert!(lever_target.get_door(5, 3).expect("door").mechanical);

    let plate_target = gs(json!([
        { "col": 5, "row": 3, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 2, "row": 2, "type": "pressure_plate", "id": "plate_1", "targets": ["door_1"] },
    ]));
    assert!(plate_target.get_door(5, 3).expect("door").mechanical);

    let plain = gs(json!([door(1, 1, "closed")]));
    assert!(!plain.get_door(1, 1).expect("door").mechanical);
}

// --- entityById index ---

#[test]
fn entity_index_is_populated_and_skips_anonymous_entities() {
    let state = gs(json!([
        { "col": 3, "row": 3, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 1, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
    ]));
    let door_entry = state.entity_by_id.get("door_1").expect("door indexed");
    assert_eq!((door_entry.col, door_entry.row), (3, 3));
    assert_eq!(door_entry.entity_type, "door");
    assert_eq!(door_entry.layer_index, 0);
    let lever_entry = state.entity_by_id.get("lever_1").expect("lever indexed");
    assert_eq!((lever_entry.col, lever_entry.row), (1, 1));

    let anonymous = gs(json!([door(3, 3, "closed")]));
    assert!(anonymous.entity_by_id.is_empty());
}

#[test]
fn resolve_entity_position_lookups() {
    let state = gs(json!([
        { "col": 5, "row": 2, "type": "door", "state": "closed", "id": "door_abc" },
    ]));
    let position = state.resolve_entity_position("door_abc").expect("known id");
    assert_eq!((position.col, position.row), (5, 2));
    assert!(
        gs(json!([]))
            .resolve_entity_position("nonexistent")
            .is_none()
    );
}

#[test]
fn entity_index_rebuilt_after_load_new_level() {
    let mut state = gs(json!([
        { "col": 1, "row": 1, "type": "door", "state": "closed", "id": "door_old" },
    ]));
    assert!(state.entity_by_id.contains_key("door_old"));

    state.load_new_level(
        &as_layer(json!([
            { "col": 3, "row": 3, "type": "door", "state": "closed", "id": "door_new" },
        ])),
        None,
        &mut || 0.5,
    );
    assert!(!state.entity_by_id.contains_key("door_old"));
    let entry = state
        .entity_by_id
        .get("door_new")
        .expect("new door indexed");
    assert_eq!((entry.col, entry.row), (3, 3));
}

#[test]
fn entity_index_rebuilt_after_load_level_state() {
    let state = gs(json!([
        { "col": 2, "row": 2, "type": "door", "state": "closed", "id": "door_snap" },
    ]));
    let snapshot = state.save_level_state();

    let mut restored = gs(json!([]));
    assert!(!restored.entity_by_id.contains_key("door_snap"));
    restored.load_level_state(&snapshot);
    let entry = restored.entity_by_id.get("door_snap").expect("restored id");
    assert_eq!((entry.col, entry.row), (2, 2));
}

// --- Keys ---

#[test]
fn add_key_and_has_key() {
    let mut state = gs(json!([]));
    assert!(!state.has_key("gold_key"));
    state.add_key("gold_key");
    assert!(state.has_key("gold_key"));
}

#[test]
fn key_pickup_flow() {
    let mut state = gs(json!([{ "col": 3, "row": 2, "type": "key", "keyId": "gold_key" }]));
    assert_eq!(state.pickup_key_at(3, 2).as_deref(), Some("gold_key"));
    assert!(state.has_key("gold_key"));
    assert!(state.pickup_key_at(3, 2).is_none());
    assert!(state.pickup_key_at(5, 5).is_none());

    let extracted = gs(json!([
        { "col": 1, "row": 1, "type": "key", "keyId": "silver_key" },
        { "col": 2, "row": 3, "type": "key", "keyId": "gold_key" },
    ]));
    assert_eq!(extracted.active_layer().keys.len(), 2);
    assert_eq!(
        extracted.active_layer().keys[&door_key(1, 1)].key_id,
        "silver_key"
    );
}

// --- Lever activation ---

#[test]
fn lever_activation_toggles_door_and_lever_state() {
    let mut state = gs(json!([
        { "col": 5, "row": 3, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
    ]));
    assert_eq!(state.get_lever(2, 1).expect("lever").state, LeverState::Up);
    let result = state.activate_lever(2, 1);
    assert_eq!(result, Some(vec!["door_1".to_string()]));
    assert_eq!(state.get_door(5, 3).expect("door").state, DoorState::Open);
    assert_eq!(
        state.get_lever(2, 1).expect("lever").state,
        LeverState::Down
    );

    state.activate_lever(2, 1);
    assert_eq!(state.get_door(5, 3).expect("door").state, DoorState::Closed);
    assert_eq!(state.get_lever(2, 1).expect("lever").state, LeverState::Up);
}

#[test]
fn lever_activation_edge_cases() {
    let mut empty = gs(json!([]));
    assert!(empty.activate_lever(9, 9).is_none());

    let mut multi = gs(json!([
        { "col": 1, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 3, "row": 3, "type": "door", "state": "closed", "id": "door_2" },
        { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1", "door_2"] },
    ]));
    multi.activate_lever(2, 1);
    assert_eq!(multi.get_door(1, 1).expect("door").state, DoorState::Open);
    assert_eq!(multi.get_door(3, 3).expect("door").state, DoorState::Open);

    let extracted = gs(json!([
        { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        { "col": 4, "row": 6, "type": "lever", "id": "lever_2", "targets": ["door_2"] },
    ]));
    assert_eq!(extracted.active_layer().levers.len(), 2);
    assert_eq!(
        extracted.active_layer().levers[&door_key(2, 1)].targets,
        vec!["door_1".to_string()]
    );
}

// --- Pressure plates ---

#[test]
fn pressure_plate_activation() {
    let mut state = gs(json!([
        { "col": 5, "row": 3, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 2, "row": 2, "type": "pressure_plate", "id": "plate_1", "targets": ["door_1"] },
    ]));
    let result = state.activate_pressure_plate(2, 2);
    assert_eq!(result, Some(vec!["door_1".to_string()]));
    assert_eq!(state.get_door(5, 3).expect("door").state, DoorState::Open);
    assert!(state.active_layer().plates[&door_key(2, 2)].activated);

    state.activate_pressure_plate(2, 2);
    assert!(!state.active_layer().plates[&door_key(2, 2)].activated);
}

#[test]
fn one_shot_plate_ignores_second_activation() {
    let mut state = gs(json!([
        { "col": 5, "row": 3, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 2, "row": 2, "type": "pressure_plate", "id": "plate_1",
          "targets": ["door_1"], "signalMode": "one_shot" },
    ]));
    assert!(state.activate_pressure_plate(2, 2).is_some());
    assert!(state.activate_pressure_plate(2, 2).is_none());
}

#[test]
fn pressure_plate_edge_cases() {
    let mut empty = gs(json!([]));
    assert!(empty.activate_pressure_plate(9, 9).is_none());

    let mut multi = gs(json!([
        { "col": 1, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 3, "row": 3, "type": "door", "state": "closed", "id": "door_2" },
        { "col": 2, "row": 2, "type": "pressure_plate", "id": "plate_1", "targets": ["door_1", "door_2"] },
    ]));
    multi.activate_pressure_plate(2, 2);
    assert_eq!(multi.get_door(1, 1).expect("door").state, DoorState::Open);
    assert_eq!(multi.get_door(3, 3).expect("door").state, DoorState::Open);

    let extracted = gs(json!([
        { "col": 3, "row": 3, "type": "pressure_plate", "id": "plate_1", "targets": ["door_1"] },
    ]));
    assert_eq!(extracted.active_layer().plates.len(), 1);
    assert_eq!(
        extracted.active_layer().plates[&door_key(3, 3)].targets,
        vec!["door_1".to_string()]
    );

    let mut opens = gs(json!([
        { "col": 4, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 1, "row": 3, "type": "pressure_plate", "id": "plate_1", "targets": ["door_1"] },
    ]));
    assert!(!opens.is_door_open(4, 2));
    opens.activate_pressure_plate(1, 3);
    assert!(opens.is_door_open(4, 2));
}

// --- autoDetectLeverWall ---

#[test]
fn lever_wall_auto_detection() {
    let grid = ["#####", "#...#", "#...#", "#...#", "#####"];
    let lever = |col: i64, row: i64| json!([{ "col": col, "row": row, "type": "lever", "id": "lever_1", "targets": ["door_x"] }]);
    let wall_of = |state: &GameState, col: i64, row: i64| {
        state.active_layer().levers[&door_key(col, row)].wall
    };
    assert_eq!(wall_of(&gs_with_grid(lever(2, 1), &grid), 2, 1), Facing::N);
    assert_eq!(wall_of(&gs_with_grid(lever(2, 3), &grid), 2, 3), Facing::S);
    assert_eq!(wall_of(&gs_with_grid(lever(3, 2), &grid), 3, 2), Facing::E);
    assert_eq!(wall_of(&gs_with_grid(lever(1, 2), &grid), 1, 2), Facing::W);
    assert_eq!(wall_of(&gs_with_grid(lever(2, 2), &grid), 2, 2), Facing::N);
    assert_eq!(wall_of(&gs(lever(2, 2)), 2, 2), Facing::N);
}

// --- Defaults ---

#[test]
fn hp_torch_fuel_and_explored_defaults() {
    let state = gs(json!([]));
    assert_eq!(state.player.hp, 65.0);
    assert_eq!(state.player.max_hp, 65.0);
    assert_eq!(state.status_fx.torch_fuel, 200.0);
    assert_eq!(state.status_fx.max_torch_fuel, 200.0);
    assert!(state.active_layer().explored_cells.is_empty());
}

// --- revealAround ---

#[test]
fn reveal_around_marks_current_adjacent_and_line_of_sight() {
    let grid: Vec<String> = ["#####", "#...#", "#...#", "#...#", "#####"]
        .iter()
        .map(ToString::to_string)
        .collect();

    let mut state = gs(json!([]));
    state.reveal_around(2, 2, Facing::N, &grid);
    let explored = &state.active_layer().explored_cells;
    for key in ["2,2", "2,1", "2,3", "1,2", "3,2"] {
        assert!(explored.contains(key), "missing {key}");
    }

    let mut north = gs(json!([]));
    north.reveal_around(2, 3, Facing::N, &grid);
    for key in ["2,2", "2,1", "2,0"] {
        assert!(north.active_layer().explored_cells.contains(key));
    }

    let mut east = gs(json!([]));
    east.reveal_around(1, 2, Facing::E, &grid);
    for key in ["2,2", "3,2", "4,2"] {
        assert!(east.active_layer().explored_cells.contains(key));
    }

    let mut south = gs(json!([]));
    south.reveal_around(2, 1, Facing::S, &grid);
    for key in ["2,2", "2,3", "2,4"] {
        assert!(south.active_layer().explored_cells.contains(key));
    }

    let mut west = gs(json!([]));
    west.reveal_around(3, 2, Facing::W, &grid);
    for key in ["2,2", "1,2", "0,2"] {
        assert!(west.active_layer().explored_cells.contains(key));
    }
}

#[test]
fn reveal_around_stops_at_walls_and_bounds() {
    let narrow: Vec<String> = ["#######", "#.#...#", "#######"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut state = gs(json!([]));
    state.reveal_around(1, 1, Facing::E, &narrow);
    assert!(state.active_layer().explored_cells.contains("2,1"));
    assert!(!state.active_layer().explored_cells.contains("3,1"));

    let grid: Vec<String> = ["#####", "#...#", "#...#", "#...#", "#####"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut corner = gs(json!([]));
    corner.reveal_around(0, 0, Facing::N, &grid);
    assert!(corner.active_layer().explored_cells.contains("0,0"));
    assert!(!corner.active_layer().explored_cells.contains("0,-1"));

    let mut accumulate = gs(json!([]));
    accumulate.reveal_around(1, 1, Facing::N, &grid);
    let first_count = accumulate.active_layer().explored_cells.len();
    accumulate.reveal_around(3, 3, Facing::S, &grid);
    assert!(accumulate.active_layer().explored_cells.len() > first_count);

    let mut dedupe = gs(json!([]));
    dedupe.reveal_around(2, 2, Facing::N, &grid);
    let count = dedupe.active_layer().explored_cells.len();
    dedupe.reveal_around(2, 2, Facing::N, &grid);
    assert_eq!(dedupe.active_layer().explored_cells.len(), count);
}

// --- Level snapshots ---

#[test]
fn save_level_state_captures_world_maps() {
    let mut state = gs(json!([
        { "col": 1, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 2, "row": 2, "type": "key", "keyId": "gold_key" },
        { "col": 3, "row": 3, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        { "col": 4, "row": 4, "type": "pressure_plate", "id": "plate_1", "targets": ["door_1"] },
    ]));
    state
        .active_layer_mut()
        .explored_cells
        .insert("0,0".to_string());
    state
        .active_layer_mut()
        .explored_cells
        .insert("1,0".to_string());

    let snapshot = state.save_level_state();
    let layer0 = &snapshot.layers[0].layer;
    assert_eq!(layer0.doors.len(), 1);
    assert_eq!(layer0.keys.len(), 1);
    assert_eq!(layer0.levers.len(), 1);
    assert_eq!(layer0.plates.len(), 1);
    assert_eq!(layer0.explored_cells.len(), 2);
    assert!(layer0.explored_cells.contains("0,0"));
}

#[test]
fn snapshot_is_a_deep_copy() {
    let mut state = gs(json!([door(1, 1, "closed")]));
    state
        .active_layer_mut()
        .explored_cells
        .insert("1,1".to_string());
    let snapshot = state.save_level_state();

    state.open_door(1, 1);
    state
        .active_layer_mut()
        .explored_cells
        .insert("2,2".to_string());

    assert_eq!(
        snapshot.layers[0].layer.doors[&door_key(1, 1)].state,
        DoorState::Closed
    );
    assert!(!snapshot.layers[0].layer.explored_cells.contains("2,2"));
}

#[test]
fn load_level_state_restores_and_is_independent_of_snapshot() {
    let mut state = gs(json!([door(5, 5, "open")]));
    state
        .active_layer_mut()
        .explored_cells
        .insert("5,5".to_string());
    let snapshot = state.save_level_state();

    let mut restored = gs(json!([]));
    restored.load_level_state(&snapshot);
    assert_eq!(restored.active_layer().doors.len(), 1);
    assert_eq!(
        restored.get_door(5, 5).expect("door").state,
        DoorState::Open
    );
    assert!(restored.active_layer().explored_cells.contains("5,5"));

    let closed = gs(json!([door(1, 1, "closed")]));
    let mut mutable_snapshot = closed.save_level_state();
    let mut independent = gs(json!([]));
    independent.load_level_state(&mutable_snapshot);
    mutable_snapshot.layers[0]
        .layer
        .doors
        .get_mut(&door_key(1, 1))
        .expect("door in snapshot")
        .state = DoorState::Open;
    mutable_snapshot.layers[0]
        .layer
        .explored_cells
        .insert("9,9".to_string());
    assert_eq!(
        independent.get_door(1, 1).expect("door").state,
        DoorState::Closed
    );
    assert!(!independent.active_layer().explored_cells.contains("9,9"));
}

// --- loadNewLevel ---

#[test]
fn load_new_level_resets_world_but_preserves_player() {
    let mut state = gs(json!([
        door(1, 1, "closed"),
        { "col": 2, "row": 2, "type": "key", "keyId": "gold_key" },
    ]));
    state
        .active_layer_mut()
        .explored_cells
        .insert("1,1".to_string());
    state.player.hp = 15.0;
    state.status_fx.torch_fuel = 50.0;
    state.add_key("iron_key");

    state.load_new_level(&as_layer(json!([door(9, 9, "open")])), None, &mut || 0.5);

    assert!(state.get_door(1, 1).is_none());
    assert_eq!(state.get_door(9, 9).expect("door").state, DoorState::Open);
    assert!(state.active_layer().keys.is_empty());
    assert!(state.active_layer().explored_cells.is_empty());
    assert_eq!(state.player.hp, 15.0);
    assert_eq!(state.status_fx.torch_fuel, 50.0);
    assert!(state.has_key("iron_key"));
}

// --- Stairs ---

#[test]
fn stairs_parse_snapshot_and_reset() {
    let state = gs(json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stairs_2" },
    ]));
    assert_eq!(state.active_layer().stairs.len(), 1);
    let stair = state.get_stair(2, 1).expect("stair present");
    assert_eq!(stair.facing, Facing::S);
    assert!(state.get_stair(9, 9).is_none());

    let snapshot = state.save_level_state();
    assert_eq!(snapshot.layers[0].layer.stairs.len(), 1);

    let mut restored = gs(json!([]));
    restored.load_level_state(&snapshot);
    assert!(restored.get_stair(2, 1).is_some());

    let mut reset = gs(json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stairs_2" },
    ]));
    reset.load_new_level(&as_layer(json!([])), None, &mut || 0.5);
    assert!(reset.active_layer().stairs.is_empty());
}

// --- Torch fuel and hunger ---

#[test]
fn torch_fuel_draining() {
    let mut state = gs(json!([]));
    state.drain_torch_fuel(10.0);
    assert_eq!(state.status_fx.torch_fuel, 190.0);
    state.drain_torch_fuel(200.0);
    assert_eq!(state.status_fx.torch_fuel, 0.0);

    let mut untouched = gs(json!([]));
    untouched.drain_torch_fuel(0.0);
    assert_eq!(untouched.status_fx.torch_fuel, 200.0);
}

#[test]
fn hunger_flows() {
    let mut state = gs(json!([]));
    assert_eq!(state.status_fx.hunger, 100.0);
    assert_eq!(state.status_fx.max_hunger, 100.0);

    state.drain_hunger(10.0);
    assert_eq!(state.status_fx.hunger, 90.0);
    state.drain_hunger(150.0);
    assert_eq!(state.status_fx.hunger, 0.0);

    state.status_fx.hunger = 50.0;
    state.restore_hunger(30.0);
    assert_eq!(state.status_fx.hunger, 80.0);
    state.restore_hunger(30.0);
    assert_eq!(state.status_fx.hunger, 100.0);

    state.status_fx.hunger = 42.0;
    state.load_new_level(&as_layer(json!([])), None, &mut || 0.5);
    assert_eq!(state.status_fx.hunger, 42.0);

    state.status_fx.hunger = 55.0;
    let player_state = state.get_player_state();
    assert_eq!(player_state.hunger, 55.0);
    assert_eq!(player_state.max_hunger, 100.0);

    let mut snapshot = state.get_player_state();
    snapshot.hunger = 30.0;
    snapshot.max_hunger = 100.0;
    state.restore_player_state(&snapshot);
    assert_eq!(state.status_fx.hunger, 30.0);
}

#[test]
fn restore_player_state_defaults_hunger_when_missing() {
    let mut state = gs(json!([]));
    state.status_fx.hunger = 50.0;
    let snapshot = state.get_player_state();
    let mut raw = serde_json::to_value(&snapshot).expect("snapshot serializes");
    raw.as_object_mut().expect("object").remove("hunger");
    raw.as_object_mut().expect("object").remove("maxHunger");
    let old_save: delve_core::game_state::PlayerStateSnapshot =
        serde_json::from_value(raw).expect("old save deserializes with defaults");
    state.restore_player_state(&old_save);
    assert_eq!(state.status_fx.hunger, 100.0);
    assert_eq!(state.status_fx.max_hunger, 100.0);
}

// --- Fountain, bookshelf, altar, barrel ---

#[test]
fn fountain_flows() {
    let mut state = gs(json!([{ "col": 3, "row": 3, "type": "fountain", "healAmount": 20 }]));
    let fountain = state.get_fountain(3, 3).expect("fountain present");
    assert_eq!(fountain.state, UsableState::Active);
    assert_eq!(fountain.heal_amount, 20.0);

    state.player.hp = 30.0;
    let (healed, amount) = state.use_fountain(3, 3);
    assert!(healed);
    assert_eq!(amount, 20.0);
    assert_eq!(state.player.hp, 50.0);
    assert_eq!(
        state.get_fountain(3, 3).expect("fountain").state,
        UsableState::Used
    );
    assert!(!state.use_fountain(3, 3).0);

    let mut clamped = gs(json!([{ "col": 3, "row": 3, "type": "fountain", "healAmount": 50 }]));
    clamped.player.hp = clamped.player.max_hp - 5.0;
    clamped.use_fountain(3, 3);
    assert_eq!(clamped.player.hp, clamped.player.max_hp);
}

#[test]
fn bookshelf_wall_matching() {
    let state = gs(json!([
        { "col": 3, "row": 3, "type": "bookshelf", "wall": "N", "text": "Lore text" },
    ]));
    assert_eq!(
        state
            .get_bookshelf_on_wall(3, 3, Facing::N)
            .expect("bookshelf present")
            .text,
        "Lore text"
    );
    assert!(state.get_bookshelf_on_wall(3, 3, Facing::S).is_none());
}

#[test]
fn altar_flows() {
    let mut state = gs(json!([
        { "col": 3, "row": 3, "type": "altar", "buffType": "atk", "buffAmount": 5, "buffDuration": 60 },
    ]));
    let altar = state.get_altar(3, 3).expect("altar present");
    assert_eq!(altar.state, UsableState::Active);

    let (activated, buff_type, _, _) = state.use_altar(3, 3);
    assert!(activated);
    assert_eq!(buff_type, BuffStat::Atk);
    assert_eq!(
        state.get_altar(3, 3).expect("altar").state,
        UsableState::Used
    );
    assert_eq!(state.status_fx.temp_buffs.len(), 1);
    assert_eq!(state.status_fx.temp_buffs[0].stat, BuffStat::Atk);
    assert!(!state.use_altar(3, 3).0);

    let parsed = gs(json!([
        { "col": 3, "row": 3, "type": "altar", "buffType": "def", "buffAmount": 3, "buffDuration": 30 },
    ]));
    assert_eq!(
        parsed.get_altar(3, 3).expect("altar").buff_type,
        BuffStat::Def
    );
}

#[test]
fn barrel_flows() {
    let mut state = gs(json!([{ "col": 3, "row": 3, "type": "barrel", "hp": 20 }]));
    let barrel = state.get_barrel(3, 3).expect("barrel present");
    assert_eq!(barrel.hp, 20.0);
    assert_eq!(barrel.max_hp, 20.0);

    let outcome = state.damage_barrel(3, 3, 5.0);
    assert!(!outcome.destroyed);
    assert_eq!(state.get_barrel(3, 3).expect("barrel").hp, 15.0);

    let destroyed = state.damage_barrel(3, 3, 20.0);
    assert!(destroyed.destroyed);
    assert!(state.get_barrel(3, 3).is_none());

    let checker = gs(json!([{ "col": 3, "row": 3, "type": "barrel", "hp": 10 }]));
    assert!(checker.is_barrel_at(3, 3));
    assert!(!checker.is_barrel_at(4, 4));
}

// --- Temp buffs ---

#[test]
fn temp_buff_flows() {
    let mut state = gs(json!([]));
    state.add_temp_buff(BuffStat::Atk, 5.0, 60.0);
    assert_eq!(state.status_fx.temp_buffs.len(), 1);
    assert_eq!(state.get_temp_buff_total(BuffStat::Atk), 5.0);

    state.add_temp_buff(BuffStat::Atk, 10.0, 30.0);
    assert_eq!(state.status_fx.temp_buffs.len(), 1);
    assert_eq!(state.get_temp_buff_total(BuffStat::Atk), 10.0);

    let mut expiring = gs(json!([]));
    expiring.add_temp_buff(BuffStat::Atk, 5.0, 2.0);
    expiring.tick_temp_buffs(3.0);
    assert!(expiring.status_fx.temp_buffs.is_empty());
    assert_eq!(expiring.get_temp_buff_total(BuffStat::Atk), 0.0);

    let mut effective = gs(json!([]));
    let base_def = effective.get_effective_stats().def;
    effective.add_temp_buff(BuffStat::Def, 10.0, 60.0);
    assert_eq!(effective.get_effective_stats().def, base_def + 10.0);

    let mut snapshotting = gs(json!([]));
    snapshotting.add_temp_buff(BuffStat::Str, 3.0, 30.0);
    let player_state = snapshotting.get_player_state();
    assert_eq!(player_state.temp_buffs.len(), 1);
    assert_eq!(player_state.temp_buffs[0].stat, BuffStat::Str);

    let mut restored = gs(json!([]));
    restored.restore_player_state(&player_state);
    assert_eq!(restored.status_fx.temp_buffs.len(), 1);
    assert_eq!(restored.get_temp_buff_total(BuffStat::Str), 3.0);
}

// --- Equipment ---

#[test]
fn effective_stats_without_equipment() {
    let state = gs(json!([]));
    assert_eq!(state.get_effective_atk(), 2.0);
    assert_eq!(state.get_effective_def(), 1.0);
}

#[test]
fn pickup_equipment_auto_equips() {
    let mut state = gs(json!([
        { "col": 3, "row": 3, "type": "equipment", "itemId": "sword" },
    ]));
    let (item, denied) = state.pickup_equipment_at(3, 3);
    assert_eq!(item.as_deref(), Some("Sword"));
    assert!(denied.is_none());
    assert!(
        state
            .entity_registry
            .get_equipped(EquipSlot::Weapon)
            .is_some()
    );
    assert!(
        state
            .entity_registry
            .ground_items(&state.current_level_id, 3, 3, None)
            .is_empty()
    );

    let mut empty = gs(json!([]));
    let (no_item, no_denied) = empty.pickup_equipment_at(5, 5);
    assert!(no_item.is_none());
    assert!(no_denied.is_none());
}

#[test]
fn equipment_registry_across_levels() {
    let mut state = gs_level(
        json!([{ "col": 1, "row": 1, "type": "equipment", "itemId": "sword" }]),
        "test_level",
    );
    assert_eq!(
        state
            .entity_registry
            .all_ground_items_for_level("test_level", None)
            .len(),
        1
    );
    assert_eq!(
        state.entity_registry.ground_items("test_level", 1, 1, None)[0].item_id,
        "sword"
    );

    state.pickup_equipment_at(1, 1);
    state.load_new_level(&as_layer(json!([])), None, &mut || 0.5);
    assert!(
        state
            .entity_registry
            .get_equipped(EquipSlot::Weapon)
            .is_some()
    );

    let mut ground_only = gs_level(
        json!([{ "col": 1, "row": 1, "type": "equipment", "itemId": "sword" }]),
        "test_level",
    );
    ground_only.load_new_level(&as_layer(json!([])), None, &mut || 0.5);
    assert!(
        ground_only
            .entity_registry
            .all_ground_items_for_level("test_level", None)
            .is_empty()
    );
}

// --- Consumables ---

#[test]
fn pickup_consumable_flows() {
    let mut state = gs_level(
        json!([{ "col": 2, "row": 2, "type": "consumable", "itemId": "hp1" }]),
        "test_level",
    );
    assert_eq!(state.pickup_consumable_at(2, 2).as_deref(), Some("Potion"));
    assert_eq!(state.entity_registry.backpack_items().len(), 1);
    assert!(
        state
            .entity_registry
            .ground_items("test_level", 2, 2, None)
            .is_empty()
    );

    let mut full = gs_level(
        json!([{ "col": 1, "row": 1, "type": "consumable", "itemId": "hp1" }]),
        "test_level",
    );
    for slot in 0..12 {
        full.entity_registry.create_item(
            "hp1",
            ItemQuality::Common,
            ItemLocation::Backpack { slot },
            Vec::new(),
        );
    }
    assert!(full.pickup_consumable_at(1, 1).is_none());
    assert_eq!(
        full.entity_registry
            .ground_items("test_level", 1, 1, None)
            .len(),
        1
    );

    let mut empty = gs(json!([]));
    assert!(empty.pickup_consumable_at(5, 5).is_none());
}

#[test]
fn use_consumable_effects() {
    let mut potion = gs(json!([]));
    potion.player.hp = 10.0;
    potion.entity_registry.create_item(
        "hp1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(potion.use_consumable(0));
    assert_eq!(potion.player.hp, 15.0);
    assert!(potion.entity_registry.backpack_items().is_empty());

    let mut clamped = gs(json!([]));
    clamped.player.hp = clamped.player.max_hp - 2.0;
    clamped.entity_registry.create_item(
        "hp1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    clamped.use_consumable(0);
    assert_eq!(clamped.player.hp, clamped.player.max_hp);

    let mut oil = gs(json!([]));
    oil.status_fx.torch_fuel = 50.0;
    oil.entity_registry.create_item(
        "oil1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(oil.use_consumable(0));
    assert_eq!(oil.status_fx.torch_fuel, 80.0);

    let mut oil_clamped = gs(json!([]));
    oil_clamped.status_fx.torch_fuel = 190.0;
    oil_clamped.entity_registry.create_item(
        "oil1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    oil_clamped.use_consumable(0);
    assert_eq!(oil_clamped.status_fx.torch_fuel, 200.0);

    let mut food = gs(json!([]));
    food.status_fx.hunger = 50.0;
    food.entity_registry.create_item(
        "food1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(food.use_consumable(0));
    assert_eq!(food.status_fx.hunger, 80.0);

    let mut food_clamped = gs(json!([]));
    food_clamped.status_fx.hunger = 90.0;
    food_clamped.entity_registry.create_item(
        "food1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    food_clamped.use_consumable(0);
    assert_eq!(food_clamped.status_fx.hunger, 100.0);

    let mut registry_food = gs(json!([]));
    registry_food.status_fx.hunger = 40.0;
    let entity = registry_food.entity_registry.create_item(
        "food1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(registry_food.use_consumable_from_registry(&entity.instance_id));
    assert_eq!(registry_food.status_fx.hunger, 70.0);

    let mut invalid = gs(json!([]));
    assert!(!invalid.use_consumable(0));
    assert!(!invalid.use_consumable(10));
}

#[test]
fn registry_persistence_across_levels_and_snapshots() {
    let mut state = gs_level(json!([]), "test_level");
    state.entity_registry.create_item(
        "hp1",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    state.load_new_level(&as_layer(json!([])), None, &mut || 0.5);
    assert_eq!(state.entity_registry.backpack_items().len(), 1);
    assert_eq!(state.entity_registry.backpack_items()[0].item_id, "hp1");

    let mut ground = gs_level(
        json!([{ "col": 1, "row": 1, "type": "consumable", "itemId": "hp1" }]),
        "test_level",
    );
    ground.load_new_level(&as_layer(json!([])), None, &mut || 0.5);
    assert!(
        ground
            .entity_registry
            .all_ground_items_for_level("test_level", None)
            .is_empty()
    );

    let both = gs_level(
        json!([
            { "col": 1, "row": 1, "type": "equipment", "itemId": "sword" },
            { "col": 2, "row": 2, "type": "consumable", "itemId": "hp1" },
        ]),
        "test_level",
    );
    let snapshot = both.save_level_state();
    assert_eq!(snapshot.layers[0].registry_snapshot.len(), 2);

    let mut restored = gs_level(json!([]), "test_level");
    restored.load_level_state(&snapshot);
    assert_eq!(
        restored
            .entity_registry
            .ground_items("test_level", 1, 1, None)[0]
            .item_id,
        "sword"
    );
    assert_eq!(
        restored
            .entity_registry
            .ground_items("test_level", 2, 2, None)[0]
            .item_id,
        "hp1"
    );

    let mut deep_copy = gs_level(
        json!([{ "col": 1, "row": 1, "type": "equipment", "itemId": "sword" }]),
        "test_level",
    );
    let saved = deep_copy.save_level_state();
    let saved_len = saved.layers[0].registry_snapshot.len();
    let instance_id = deep_copy
        .entity_registry
        .all_ground_items_for_level("test_level", None)[0]
        .instance_id
        .clone();
    deep_copy.entity_registry.remove_item(&instance_id);
    assert_eq!(saved.layers[0].registry_snapshot.len(), saved_len);
}

// --- Signal state across save/load ---

#[test]
fn and_gate_door_stays_responsive_after_reload() {
    let mut state = gs(json!([
        { "col": 1, "row": 0, "type": "lever", "id": "lev_1", "targets": ["door_1"], "wall": "N" },
        { "col": 2, "row": 0, "type": "lever", "id": "lev_2", "targets": ["door_1"], "wall": "N" },
        { "col": 3, "row": 0, "type": "lever", "id": "lev_3", "targets": ["door_1"], "wall": "N" },
        { "col": 4, "row": 0, "type": "lever", "id": "lev_4", "targets": ["door_1"], "wall": "N" },
        { "col": 5, "row": 0, "type": "door", "id": "door_1", "state": "closed", "gateMode": "and" },
    ]));
    state.activate_lever(1, 0);
    state.activate_lever(2, 0);
    state.activate_lever(3, 0);
    state.activate_lever(4, 0);
    assert_eq!(state.get_door(5, 0).expect("door").state, DoorState::Open);

    let snapshot = state.save_level_state();
    let mut restored = gs(json!([]));
    restored.load_level_state(&snapshot);
    assert_eq!(
        restored.get_door(5, 0).expect("door").state,
        DoorState::Open
    );

    restored.activate_lever(1, 0);
    assert_eq!(
        restored.get_door(5, 0).expect("door").state,
        DoorState::Closed
    );
}

#[test]
fn standalone_gate_state_preserved_across_reload() {
    let mut state = gs(json!([
        { "col": 0, "row": 0, "type": "lever", "id": "lev_1", "targets": ["gate_1"], "wall": "N" },
        { "col": 1, "row": 0, "type": "gate", "id": "gate_1", "gateType": "and", "targets": ["door_1"] },
        { "col": 2, "row": 0, "type": "door", "id": "door_1", "state": "closed" },
    ]));
    state.activate_lever(0, 0);
    assert_eq!(state.get_door(2, 0).expect("door").state, DoorState::Open);

    let snapshot = state.save_level_state();
    let mut restored = gs(json!([]));
    restored.load_level_state(&snapshot);
    assert_eq!(
        restored.get_door(2, 0).expect("door").state,
        DoorState::Open
    );
    restored.activate_lever(0, 0);
    assert_eq!(
        restored.get_door(2, 0).expect("door").state,
        DoorState::Closed
    );
}

// --- Stats and leveling ---

#[test]
fn xp_for_level_progression() {
    let state = gs(json!([]));
    let expectations = [(1, 100), (2, 300), (3, 600), (4, 1000), (5, 1500)];
    for (level, xp) in expectations {
        assert_eq!(state.xp_for_level(level), xp);
    }
}

#[test]
fn add_xp_levels_and_caps() {
    let mut state = gs(json!([]));
    assert!(!state.add_xp(50));
    assert_eq!(state.player.xp, 50);
    assert_eq!(state.player.level, 1);

    let mut leveller = gs(json!([]));
    assert!(leveller.add_xp(100));
    assert_eq!(leveller.player.level, 2);
    assert_eq!(leveller.player.attribute_points, 3);

    let mut multi = gs(json!([]));
    multi.add_xp(600);
    assert_eq!(multi.player.level, 4);
    assert_eq!(multi.player.attribute_points, 9);

    let mut capped = gs(json!([]));
    capped.add_xp(12000);
    assert_eq!(capped.player.level, 15);
    assert!(!capped.add_xp(99999));
    assert_eq!(capped.player.level, 15);
}

#[test]
fn allocate_point_flows() {
    let mut state = gs(json!([]));
    state.player.attribute_points = 3;
    assert!(state.allocate_point(AllocatableStat::Str));
    assert_eq!(state.player.attribute_points, 2);
    assert_eq!(state.player.str, 6.0);

    let mut no_points = gs(json!([]));
    assert!(!no_points.allocate_point(AllocatableStat::Dex));
    assert_eq!(no_points.player.dex, 5.0);

    let mut vit = gs(json!([]));
    vit.player.attribute_points = 1;
    let previous_max = vit.player.max_hp;
    assert_eq!(vit.player.hp, vit.player.max_hp);
    vit.allocate_point(AllocatableStat::Vit);
    assert_eq!(vit.player.max_hp, previous_max + 5.0);
    assert_eq!(vit.player.hp, vit.player.max_hp);

    let mut wounded = gs(json!([]));
    wounded.player.attribute_points = 1;
    wounded.player.hp = 30.0;
    wounded.allocate_point(AllocatableStat::Vit);
    assert_eq!(wounded.player.hp, 30.0);
}

#[test]
fn effective_stats_formulas() {
    let state = gs(json!([]));
    let stats = state.get_effective_stats();
    assert_eq!(stats.max_hp, 65.0);
    assert_eq!(stats.crit_chance, 6.0);
    assert_eq!(stats.dodge_chance, 0.0);
    assert_eq!(stats.effective_str, 5.0);
    assert_eq!(stats.effective_dex, 5.0);
    assert_eq!(stats.effective_vit, 5.0);
    assert_eq!(stats.effective_wis, 5.0);

    let mut dexterous = gs(json!([]));
    dexterous.player.dex = 9.0;
    assert_eq!(dexterous.get_effective_stats().dodge_chance, 1.0);
    dexterous.player.dex = 200.0;
    assert_eq!(dexterous.get_effective_stats().dodge_chance, 25.0);
}

// --- canEquipItem / getEquippedWeaponDef ---

#[test]
fn can_equip_item_requirement_checks() {
    let items = ItemDatabase::from_json(ITEMS_JSON).expect("fixture parses");

    let state = gs(json!([]));
    let sword = items.get_item("sword").expect("no-requirement item");
    assert!(state.can_equip_item(sword).success);

    let mut weak = gs(json!([]));
    weak.player.str = 1.0;
    let heavy_axe = items.get_item("heavy_axe").expect("str-gated item");
    let denied = weak.can_equip_item(heavy_axe);
    assert!(!denied.success);
    assert!(denied.reason.expect("reason present").contains("STR"));

    let mut strong = gs(json!([]));
    strong.player.str = 25.0;
    assert!(strong.can_equip_item(heavy_axe).success);

    let mut nimble = gs(json!([]));
    nimble.player.str = 10.0;
    let iron_sword = items.get_item("sword_iron").expect("str 3 item");
    assert!(nimble.can_equip_item(iron_sword).success);
}

#[test]
fn equipped_weapon_def_lookup() {
    let mut state = gs(json!([]));
    assert!(state.get_equipped_weapon_def().is_none());
    state.entity_registry.create_item(
        "sword",
        ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Weapon,
        },
        Vec::new(),
    );
    assert_eq!(
        state.get_equipped_weapon_def().expect("weapon def").name,
        "Sword"
    );
}

// --- Phase D environment entities ---

#[test]
fn phase_d_entities_parse() {
    let walls = gs(json!([{ "col": 0, "row": 0, "type": "breakable_wall", "hp": 40 }]));
    let wall = walls.get_breakable_wall(0, 0).expect("wall present");
    assert_eq!(wall.hp, 40.0);
    assert_eq!(wall.max_hp, 40.0);

    let secret = gs(json!([{ "col": 2, "row": 3, "type": "secret_wall" }]));
    assert!(!secret.get_secret_wall(2, 3).expect("secret wall").opened);

    let blocks = gs(json!([{ "col": 1, "row": 2, "type": "block" }]));
    assert!(blocks.get_block(1, 2).is_some());

    let chests = gs(json!([
        { "col": 3, "row": 1, "type": "chest", "state": "locked", "keyId": "gold_key" },
    ]));
    let chest = chests.get_chest(3, 1).expect("chest present");
    assert_eq!(chest.state, ChestState::Locked);
    assert_eq!(chest.key_id.as_deref(), Some("gold_key"));

    let signs = gs_with_grid(
        json!([{ "col": 1, "row": 1, "type": "sign", "wall": "N", "text": "Beware!" }]),
        &["#####", "#...#", "#...#", "#####"],
    );
    let sign = signs.get_sign(1, 1).expect("sign present");
    assert_eq!(sign.wall, Facing::N);
    assert_eq!(sign.text, "Beware!");
}

#[test]
fn breakable_wall_damage_and_destruction() {
    let mut grid: Vec<String> = ["###", "#.#", "###"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut state = gs_with_grid(
        json!([{ "col": 0, "row": 0, "type": "breakable_wall", "hp": 30 }]),
        &["###", "#.#", "###"],
    );
    let outcome = state.damage_breakable_wall(0, 0, 10.0, &mut grid);
    assert!(!outcome.destroyed);
    assert_eq!(state.get_breakable_wall(0, 0).expect("wall").hp, 20.0);

    let mut destroy_grid: Vec<String> = ["###", "#.#", "###"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut destroyer = gs_with_grid(
        json!([{ "col": 0, "row": 0, "type": "breakable_wall", "hp": 10 }]),
        &["###", "#.#", "###"],
    );
    let destroyed = destroyer.damage_breakable_wall(0, 0, 50.0, &mut destroy_grid);
    assert!(destroyed.destroyed);
    assert!(destroyer.get_breakable_wall(0, 0).is_none());
    assert_eq!(destroy_grid[0].chars().next(), Some('.'));
    assert!(destroyer.active_layer().destroyed_walls.contains("0,0"));

    let mut drop_grid: Vec<String> = ["###", "#.#", "###"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut dropper = gs_with_grid(
        json!([{
            "col": 0, "row": 0, "type": "breakable_wall", "hp": 10,
            "drops": { "guaranteed": [{ "itemId": "hp1" }] },
        }]),
        &["###", "#.#", "###"],
    );
    let with_drops = dropper.damage_breakable_wall(0, 0, 50.0, &mut drop_grid);
    assert!(with_drops.destroyed);
    assert!(with_drops.drops.is_some());
}

#[test]
fn secret_wall_opening() {
    let mut grid: Vec<String> = ["###", "#.#", "###"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut state = gs_with_grid(
        json!([{ "col": 0, "row": 0, "type": "secret_wall" }]),
        &["###", "#.#", "###"],
    );
    let (opened, persistent) = state.open_secret_wall(0, 0, &mut grid);
    assert!(opened);
    assert!(!persistent);
    assert_eq!(grid[0].chars().next(), Some('.'));
    assert!(state.active_layer().destroyed_walls.contains("0,0"));
    assert!(state.get_secret_wall(0, 0).expect("secret wall").opened);
    assert!(!state.open_secret_wall(0, 0, &mut grid).0);
}

#[test]
fn block_pushing() {
    let mut state = gs(json!([{ "col": 3, "row": 3, "type": "block" }]));
    assert!(state.push_block(3, 3, 3, 2));
    assert!(state.get_block(3, 3).is_none());
    let block = state.get_block(3, 2).expect("block moved");
    assert_eq!((block.col, block.row), (3, 2));

    let mut empty = gs(json!([]));
    assert!(!empty.push_block(5, 5, 5, 4));

    let checker = gs(json!([{ "col": 2, "row": 2, "type": "block" }]));
    assert!(checker.is_block_at(2, 2));
    assert!(!checker.is_block_at(2, 3));
}

#[test]
fn push_boulder_starts_a_pushable_idle_boulder_rolling_in_the_given_direction() {
    let mut state = gs(json!([{ "col": 3, "row": 3, "type": "boulder", "pushable": true }]));
    assert!(state.push_boulder(3, 3, Facing::E));
    let boulder = state
        .active_layer()
        .boulders
        .get(&door_key(3, 3))
        .expect("boulder still at its cell");
    assert_eq!(boulder.state, BoulderState::Rolling);
    assert_eq!(boulder.direction, Facing::E);
}

#[test]
fn push_boulder_is_a_no_op_for_a_non_pushable_boulder() {
    let mut state =
        gs(json!([{ "col": 3, "row": 3, "type": "boulder", "pushable": false, "direction": "N" }]));
    assert!(!state.push_boulder(3, 3, Facing::E));
    let boulder = state
        .active_layer()
        .boulders
        .get(&door_key(3, 3))
        .expect("boulder unchanged");
    assert_eq!(boulder.state, BoulderState::Idle);
    assert_eq!(boulder.direction, Facing::N);
}

#[test]
fn push_boulder_is_a_no_op_for_a_boulder_that_is_already_rolling() {
    let mut state = gs(json!([{
        "col": 3, "row": 3, "type": "boulder",
        "pushable": true, "state": "rolling", "direction": "N",
    }]));
    assert!(!state.push_boulder(3, 3, Facing::E));
    let boulder = state
        .active_layer()
        .boulders
        .get(&door_key(3, 3))
        .expect("boulder unchanged");
    assert_eq!(boulder.state, BoulderState::Rolling);
    // Direction stays whatever it was already rolling in — a push doesn't
    // redirect a boulder that's already moving, matching TS's `state ===
    // 'idle'` gate on the direct-push branch (main.ts:946).
    assert_eq!(boulder.direction, Facing::N);
}

#[test]
fn push_boulder_returns_false_when_no_boulder_is_at_the_cell() {
    let mut state = gs(json!([]));
    assert!(!state.push_boulder(5, 5, Facing::E));
}

#[test]
fn can_boulder_roll_to_succeeds_into_a_plain_open_cell() {
    let state = gs_with_grid(json!([]), &["....."]);
    assert!(can_boulder_roll_to(
        &state,
        &["....."].map(str::to_string),
        &walkable_cells(),
        2,
        0,
        3,
        0,
    ));
}

#[test]
fn can_boulder_roll_to_fails_out_of_bounds() {
    let state = gs_with_grid(json!([]), &["....."]);
    assert!(!can_boulder_roll_to(
        &state,
        &["....."].map(str::to_string),
        &walkable_cells(),
        2,
        0,
        10,
        0,
    ));
}

#[test]
fn can_boulder_roll_to_fails_into_a_non_walkable_cell() {
    let state = gs_with_grid(json!([]), &["..#.."]);
    assert!(!can_boulder_roll_to(
        &state,
        &["..#.."].map(str::to_string),
        &walkable_cells(),
        1,
        0,
        2,
        0,
    ));
}

#[test]
fn can_boulder_roll_to_fails_into_a_closed_door() {
    let state = gs_with_grid(
        json!([{ "col": 3, "row": 0, "type": "door", "state": "closed" }]),
        &["....."],
    );
    assert!(!can_boulder_roll_to(
        &state,
        &["....."].map(str::to_string),
        &walkable_cells(),
        2,
        0,
        3,
        0,
    ));
}

#[test]
fn can_boulder_roll_to_succeeds_through_an_open_door() {
    let state = gs_with_grid(
        json!([{ "col": 3, "row": 0, "type": "door", "state": "open" }]),
        &["....."],
    );
    assert!(can_boulder_roll_to(
        &state,
        &["....."].map(str::to_string),
        &walkable_cells(),
        2,
        0,
        3,
        0,
    ));
}

#[test]
fn can_boulder_roll_to_fails_into_a_cell_holding_a_block() {
    let state = gs_with_grid(json!([{ "col": 3, "row": 0, "type": "block" }]), &["....."]);
    assert!(!can_boulder_roll_to(
        &state,
        &["....."].map(str::to_string),
        &walkable_cells(),
        2,
        0,
        3,
        0,
    ));
}

#[test]
fn can_boulder_roll_to_fails_into_a_cell_holding_another_boulder() {
    let state = gs_with_grid(
        json!([{ "col": 3, "row": 0, "type": "boulder" }]),
        &["....."],
    );
    assert!(!can_boulder_roll_to(
        &state,
        &["....."].map(str::to_string),
        &walkable_cells(),
        2,
        0,
        3,
        0,
    ));
}

#[test]
fn can_boulder_roll_to_fails_across_a_thin_wall_edge() {
    let state = gs_with_grid(
        json!([{ "col": 2, "row": 0, "type": "thin_wall", "wall": "E" }]),
        &["....."],
    );
    assert!(!can_boulder_roll_to(
        &state,
        &["....."].map(str::to_string),
        &walkable_cells(),
        2,
        0,
        3,
        0,
    ));
}

#[test]
fn chest_opening_flows() {
    let mut state = gs(json!([{ "col": 1, "row": 1, "type": "chest", "state": "closed" }]));
    assert!(state.open_chest(1, 1).opened);
    assert_eq!(
        state.get_chest(1, 1).expect("chest").state,
        ChestState::Open
    );
    assert!(!state.open_chest(1, 1).opened);

    let mut locked = gs(json!([
        { "col": 1, "row": 1, "type": "chest", "state": "locked", "keyId": "gold_key" },
    ]));
    let locked_result = locked.open_chest(1, 1);
    assert!(!locked_result.opened);
    assert!(locked_result.locked);
    assert_eq!(
        locked.get_chest(1, 1).expect("chest").state,
        ChestState::Locked
    );

    locked.add_key("gold_key");
    let unlocked = locked.open_chest(1, 1);
    assert!(unlocked.opened);
    assert_eq!(
        locked.get_chest(1, 1).expect("chest").state,
        ChestState::Open
    );
    assert!(!locked.has_key("gold_key"));
}

#[test]
fn sign_wall_lookup() {
    let state = gs_with_grid(
        json!([{ "col": 1, "row": 1, "type": "sign", "wall": "N", "text": "Hello" }]),
        &["#####", "#...#", "#####"],
    );
    assert_eq!(
        state.get_sign_on_wall(1, 1, Facing::N).expect("sign").text,
        "Hello"
    );
    assert!(state.get_sign_on_wall(1, 1, Facing::S).is_none());
}

#[test]
fn snapshot_roundtrip_includes_phase_d_entities() {
    let mut state = gs_with_grid(
        json!([
            { "col": 0, "row": 0, "type": "breakable_wall", "hp": 20 },
            { "col": 4, "row": 0, "type": "secret_wall" },
            { "col": 1, "row": 1, "type": "block" },
            { "col": 2, "row": 1, "type": "chest", "state": "closed" },
            { "col": 3, "row": 1, "type": "sign", "wall": "N", "text": "Clue" },
        ]),
        &["#####", "#...#", "#...#", "#####"],
    );
    let snapshot = state.save_level_state();

    state.active_layer_mut().breakable_walls.clear();
    state.active_layer_mut().secret_walls.clear();
    state.active_layer_mut().blocks.clear();
    state.active_layer_mut().chests.clear();
    state.active_layer_mut().signs.clear();

    state.load_level_state(&snapshot);
    assert_eq!(state.get_breakable_wall(0, 0).expect("wall").hp, 20.0);
    assert!(!state.get_secret_wall(4, 0).expect("secret wall").opened);
    assert!(state.get_block(1, 1).is_some());
    assert_eq!(
        state.get_chest(2, 1).expect("chest").state,
        ChestState::Closed
    );
    assert_eq!(state.get_sign(3, 1).expect("sign").text, "Clue");
}

// --- Thin walls ---

fn thin_wall_state() -> GameState {
    gs(json!([
        { "col": 3, "row": 2, "type": "thin_wall", "wall": "S", "solid": true, "height": "full", "texture": "stone_thin" },
        { "col": 2, "row": 3, "type": "thin_wall", "wall": "E", "solid": false, "height": "half", "texture": "stone_thin" },
    ]))
}

#[test]
fn thin_wall_between_resolves_canonical_edges() {
    let state = thin_wall_state();
    for (from, to) in [((3, 2), (3, 3)), ((3, 3), (3, 2))] {
        let wall = state
            .get_thin_wall_between(from.0, from.1, to.0, to.1)
            .expect("thin wall on edge");
        assert_eq!((wall.col, wall.row), (3, 2));
        assert_eq!(wall.wall, ThinWallSide::S);
    }
    for (from, to) in [((2, 3), (3, 3)), ((3, 3), (2, 3))] {
        let wall = state
            .get_thin_wall_between(from.0, from.1, to.0, to.1)
            .expect("thin wall on edge");
        assert_eq!((wall.col, wall.row), (2, 3));
        assert_eq!(wall.wall, ThinWallSide::E);
    }
    assert!(state.get_thin_wall_between(1, 1, 1, 2).is_none());
}

#[test]
fn edge_blocking_checks() {
    let solid = gs(json!([
        { "col": 3, "row": 2, "type": "thin_wall", "wall": "S", "solid": true, "height": "full", "texture": "stone_thin" },
    ]));
    assert!(solid.is_edge_blocked(3, 2, 3, 3));
    assert!(solid.is_edge_blocked(3, 3, 3, 2));
    assert!(!solid.is_edge_blocked(1, 1, 1, 2));
    assert!(solid.is_solid_edge_blocked(3, 2, 3, 3));
    assert!(!solid.is_solid_edge_blocked(1, 1, 1, 2));

    let passable = gs(json!([
        { "col": 3, "row": 2, "type": "thin_wall", "wall": "S", "solid": false, "height": "half", "texture": "stone_thin" },
    ]));
    assert!(!passable.is_solid_edge_blocked(3, 2, 3, 3));
}

// --- Inventory management (gameStateInventory.test.ts) ---

#[test]
fn equip_from_backpack_flows() {
    let mut state = gs_level(json!([]), "test_level");
    state.entity_registry.create_item(
        "sword_iron",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    let result = state.equip_from_backpack(0);
    assert!(result.success);
    assert_eq!(
        state
            .entity_registry
            .get_equipped(EquipSlot::Weapon)
            .expect("equipped")
            .item_id,
        "sword_iron"
    );

    let mut out_of_range = gs_level(json!([]), "test_level");
    assert!(!out_of_range.equip_from_backpack(5).success);

    let mut weak = gs_level(json!([]), "test_level");
    weak.player.str = 1.0;
    weak.entity_registry.create_item(
        "heavy_axe",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    let denied = weak.equip_from_backpack(0);
    assert!(!denied.success);
    assert!(denied.reason.expect("reason present").contains("STR"));
}

#[test]
fn equip_from_backpack_swaps_existing_item() {
    let mut state = gs_level(json!([]), "test_level");
    let existing = state.entity_registry.create_item(
        "sword_iron",
        ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Weapon,
        },
        Vec::new(),
    );
    let new_item = state.entity_registry.create_item(
        "heavy_axe",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 3 },
        Vec::new(),
    );
    state.player.str = 25.0;

    let result = state.equip_from_backpack(0);
    assert!(result.success);
    assert_eq!(result.swapped_to_slot, Some(3));
    assert_eq!(
        state
            .entity_registry
            .get_equipped(EquipSlot::Weapon)
            .expect("equipped")
            .instance_id,
        new_item.instance_id
    );
    assert_eq!(
        state
            .entity_registry
            .backpack_item_at(3)
            .expect("displaced item")
            .instance_id,
        existing.instance_id
    );

    let previous_max = state.player.max_hp;
    state.equip_from_backpack(0);
    assert_eq!(state.player.max_hp, previous_max);
}

#[test]
fn unequip_to_backpack_flows() {
    let mut state = gs_level(json!([]), "test_level");
    let entity = state.entity_registry.create_item(
        "sword_iron",
        ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Weapon,
        },
        Vec::new(),
    );
    let result = state.unequip_to_backpack(EquipSlot::Weapon, None);
    assert!(result.success);
    assert!(
        state
            .entity_registry
            .get_equipped(EquipSlot::Weapon)
            .is_none()
    );
    let backpack = state.entity_registry.backpack_items();
    assert_eq!(backpack.len(), 1);
    assert_eq!(backpack[0].instance_id, entity.instance_id);

    let mut empty_slot = gs_level(json!([]), "test_level");
    assert!(
        !empty_slot
            .unequip_to_backpack(EquipSlot::Weapon, None)
            .success
    );

    let mut full = gs_level(json!([]), "test_level");
    full.entity_registry.create_item(
        "sword_iron",
        ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Weapon,
        },
        Vec::new(),
    );
    for slot in 0..12 {
        full.entity_registry.create_item(
            "health_potion",
            ItemQuality::Common,
            ItemLocation::Backpack { slot },
            Vec::new(),
        );
    }
    let full_result = full.unequip_to_backpack(EquipSlot::Weapon, None);
    assert!(!full_result.success);
    assert!(full_result.reason.expect("reason present").contains("full"));
}

#[test]
fn drop_item_flows() {
    let mut state = gs_level(json!([]), "test_level");
    let entity = state.entity_registry.create_item(
        "sword_iron",
        ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Weapon,
        },
        Vec::new(),
    );
    assert!(state.drop_item(&entity.instance_id, 4, 7));
    let dropped = state
        .entity_registry
        .get_item(&entity.instance_id)
        .expect("dropped item");
    match &dropped.location {
        ItemLocation::World {
            level_id, col, row, ..
        } => {
            assert_eq!(level_id, "test_level");
            assert_eq!((*col, *row), (4, 7));
        }
        other => panic!("expected world location, got {other:?}"),
    }

    let mut from_backpack = gs_level(json!([]), "test_level");
    let backpack_item = from_backpack.entity_registry.create_item(
        "health_potion",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 2 },
        Vec::new(),
    );
    assert!(from_backpack.drop_item(&backpack_item.instance_id, 1, 1));
    assert!(matches!(
        from_backpack
            .entity_registry
            .get_item(&backpack_item.instance_id)
            .expect("item present")
            .location,
        ItemLocation::World { .. }
    ));

    let mut missing = gs_level(json!([]), "test_level");
    assert!(!missing.drop_item("item_9999", 1, 1));
}

/// TS's `dropItem` (`gameState.ts` at the pinned commit) recalculates maxHp
/// after the item leaves the player, same as `equipFromBackpack` and
/// `unequipToBackpack`. Using a VIT-bearing ring (rather than TS's own
/// no-bonus sword fixture) makes the recalculation observable: a regression
/// here would leave the ring's VIT bonus applied to max_hp after the drop.
#[test]
fn drop_item_recalculates_max_hp_from_effective_stats() {
    let mut state = gs_level(json!([]), "test_level");
    state.entity_registry.create_item(
        "ring_of_vitality",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    let base_max_hp = state.player.max_hp;

    let equip_result = state.equip_from_backpack(0);
    assert!(equip_result.success);
    let equipped_instance_id = state
        .entity_registry
        .get_equipped(EquipSlot::Ring1)
        .expect("ring equipped")
        .instance_id
        .clone();
    assert_eq!(state.player.max_hp, base_max_hp + 10.0);

    assert!(state.drop_item(&equipped_instance_id, 0, 0));
    assert_eq!(state.player.max_hp, base_max_hp);
}

#[test]
fn use_consumable_from_registry_flows() {
    let mut potion = gs_level(json!([]), "test_level");
    potion.player.hp = 30.0;
    let entity = potion.entity_registry.create_item(
        "health_potion",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(potion.use_consumable_from_registry(&entity.instance_id));
    assert_eq!(potion.player.hp, 50.0);
    assert!(
        potion
            .entity_registry
            .get_item(&entity.instance_id)
            .is_none()
    );

    let mut clamped = gs_level(json!([]), "test_level");
    clamped.player.hp = clamped.player.max_hp - 5.0;
    let clamp_entity = clamped.entity_registry.create_item(
        "health_potion",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    clamped.use_consumable_from_registry(&clamp_entity.instance_id);
    assert_eq!(clamped.player.hp, clamped.player.max_hp);

    let mut oil = gs_level(json!([]), "test_level");
    oil.status_fx.torch_fuel = 50.0;
    let oil_entity = oil.entity_registry.create_item(
        "torch_oil",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(oil.use_consumable_from_registry(&oil_entity.instance_id));
    assert_eq!(oil.status_fx.torch_fuel, 80.0);

    let mut oil_clamped = gs_level(json!([]), "test_level");
    oil_clamped.status_fx.torch_fuel = 190.0;
    let oil_clamp_entity = oil_clamped.entity_registry.create_item(
        "torch_oil",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    oil_clamped.use_consumable_from_registry(&oil_clamp_entity.instance_id);
    assert_eq!(oil_clamped.status_fx.torch_fuel, 200.0);

    let mut missing = gs_level(json!([]), "test_level");
    assert!(!missing.use_consumable_from_registry("item_9999"));

    let mut non_consumable = gs_level(json!([]), "test_level");
    let sword = non_consumable.entity_registry.create_item(
        "sword_iron",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(!non_consumable.use_consumable_from_registry(&sword.instance_id));
}

// --- Pit traps parse (used by later phases, wired through signals) ---

#[test]
fn pit_trap_parses_with_default_state() {
    let state = gs(json!([{ "col": 2, "row": 2, "type": "pit_trap" }]));
    assert_eq!(
        state.active_layer().pit_traps[&door_key(2, 2)].state,
        PitTrapState::Closed
    );
}

// --- layer() peek accessor (phase 5: ramp MoveRules closures only get &GameState) ---

fn two_layer_defs() -> Vec<LayerDef> {
    serde_json::from_value(json!([
        {
            "id": "ground",
            "grid": ["...", "...", "..."],
            "entities": [door(1, 1, "closed")],
        },
        {
            "id": "upper",
            "grid": ["...", "...", "..."],
            "entities": [door(2, 2, "open")],
        },
    ]))
    .expect("layer defs parse")
}

fn multi_layer_state() -> GameState {
    let layers = two_layer_defs();
    GameState::new(&[], None, "default", Some(&layers), deps(), &mut || 0.5)
}

#[test]
fn layer_returns_each_layer_by_index_regardless_of_which_is_active() {
    let state = multi_layer_state();
    assert_eq!(state.active_layer_index, 0);

    let ground = state.layer(0).expect("layer 0 exists");
    assert!(ground.doors.contains_key(&door_key(1, 1)));
    assert!(!ground.doors.contains_key(&door_key(2, 2)));

    let upper = state.layer(1).expect("layer 1 exists");
    assert!(upper.doors.contains_key(&door_key(2, 2)));
    assert!(!upper.doors.contains_key(&door_key(1, 1)));

    // Peeking a non-active layer must not disturb which layer is active.
    assert_eq!(state.active_layer_index, 0);
}

#[test]
fn layer_returns_none_for_an_out_of_range_index() {
    let state = multi_layer_state();
    assert!(state.layer(2).is_none());
    assert!(state.layer(usize::MAX).is_none());
}

#[test]
fn layer_returns_the_only_layer_for_a_single_layer_level() {
    let state = gs(json!([door(1, 1, "closed")]));
    assert!(state.layer(0).is_some());
    assert!(state.layer(1).is_none());
}

// --- layer_mut() companion (phase 5: blocked-door retries that must write
// through a recorded, possibly-non-active layer) ---

#[test]
fn layer_mut_writes_through_to_a_non_active_layer_without_switching_active_index() {
    let mut state = multi_layer_state();
    assert_eq!(state.active_layer_index, 0);
    assert_eq!(
        state.layer(1).unwrap().doors[&door_key(2, 2)].state,
        DoorState::Open
    );

    let upper = state.layer_mut(1).expect("layer 1 exists");
    upper
        .doors
        .get_mut(&door_key(2, 2))
        .expect("door exists")
        .state = DoorState::Closed;

    // The mutation landed on layer 1, not the active layer (0) — layer 0's
    // own door (a different cell, untouched by the write) still resolves
    // through `active_layer()`, proving that accessor still targets layer 0.
    assert_eq!(
        state.layer(1).unwrap().doors[&door_key(2, 2)].state,
        DoorState::Closed
    );
    assert_eq!(
        state.active_layer().doors[&door_key(1, 1)].state,
        DoorState::Closed
    );
    assert!(!state.active_layer().doors.contains_key(&door_key(2, 2)));
    // Peeking/mutating a non-active layer must not disturb which layer is
    // active.
    assert_eq!(state.active_layer_index, 0);
}

#[test]
fn layer_mut_returns_none_for_an_out_of_range_index() {
    let mut state = multi_layer_state();
    assert!(state.layer_mut(2).is_none());
    assert!(state.layer_mut(usize::MAX).is_none());
}

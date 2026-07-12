//! Ported from `src/level/interaction.test.ts`.

use delve_core::game_state::{DoorState, GameState, GameStateDeps, LeverState, door_key};
use delve_core::grid::{Facing, PlayerState};
use delve_core::interaction::{InteractionType, interact};
use delve_core::types::{EnemyAiState, EnemyInstance, Entity};
use serde_json::{Value, json};

const GRID: [&str; 4] = ["#####", "#...#", "#...#", "#####"];
const LEVER_GRID: [&str; 5] = ["#####", "#...#", "#...#", "#...#", "#####"];

fn grid(rows: &[&str]) -> Vec<String> {
    rows.iter().map(ToString::to_string).collect()
}

fn make_state(entity_values: Value, rows: &[&str]) -> GameState {
    let entities: Vec<Entity> = serde_json::from_value(entity_values).expect("entities parse");
    let grid = grid(rows);
    GameState::new(
        &entities,
        Some(&grid),
        "default",
        None,
        GameStateDeps::default(),
        &mut || 0.5,
    )
}

fn player(col: i32, row: i32, facing: Facing) -> PlayerState {
    PlayerState::new(col, row, facing)
}

#[test]
fn opens_a_closed_door() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 1, "type": "door", "state": "closed" }]),
        &GRID,
    );
    let result = interact(&player(2, 2, Facing::N), &grid(&GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::DoorOpened);
    assert_eq!(result.message.as_deref(), Some("Door opened."));
    assert_eq!(state.get_door(2, 1).expect("door").state, DoorState::Open);
}

#[test]
fn closes_an_open_non_mechanical_door() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 1, "type": "door", "state": "open" }]),
        &GRID,
    );
    let result = interact(&player(2, 2, Facing::N), &grid(&GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::DoorClosed);
    assert_eq!(result.message.as_deref(), Some("Door closed."));
    assert_eq!(state.get_door(2, 1).expect("door").state, DoorState::Closed);
}

#[test]
fn cannot_close_a_mechanical_door() {
    let mut state = make_state(
        json!([
            { "col": 2, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 1, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        ]),
        &GRID,
    );
    state.activate_lever(1, 1);
    assert_eq!(state.get_door(2, 1).expect("door").state, DoorState::Open);

    let result = interact(&player(2, 2, Facing::N), &grid(&GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::Nothing);
}

#[test]
fn open_close_open_cycle_works() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 1, "type": "door", "state": "closed" }]),
        &GRID,
    );
    let attacker = player(2, 2, Facing::N);
    assert_eq!(
        interact(&attacker, &grid(&GRID), &mut state).result_type,
        InteractionType::DoorOpened
    );
    assert_eq!(
        interact(&attacker, &grid(&GRID), &mut state).result_type,
        InteractionType::DoorClosed
    );
    assert_eq!(
        interact(&attacker, &grid(&GRID), &mut state).result_type,
        InteractionType::DoorOpened
    );
}

#[test]
fn cannot_open_a_closed_mechanical_door() {
    let mut state = make_state(
        json!([
            { "col": 2, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 1, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        ]),
        &GRID,
    );
    let result = interact(&player(2, 2, Facing::N), &grid(&GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::Nothing);
    assert_eq!(
        result.message.as_deref(),
        Some("This door is operated by a mechanism.")
    );
    assert_eq!(state.get_door(2, 1).expect("door").state, DoorState::Closed);
}

#[test]
fn keyed_door_flows() {
    let mut locked = make_state(
        json!([{ "col": 2, "row": 1, "type": "door", "state": "closed", "keyId": "gold_key" }]),
        &GRID,
    );
    let result = interact(&player(2, 2, Facing::N), &grid(&GRID), &mut locked);
    assert_eq!(result.result_type, InteractionType::DoorLocked);
    assert_eq!(result.message.as_deref(), Some("This door is locked."));

    locked.add_key("gold_key");
    let unlocked = interact(&player(2, 2, Facing::N), &grid(&GRID), &mut locked);
    assert_eq!(unlocked.result_type, InteractionType::DoorOpened);

    let mut wrong_key = make_state(
        json!([{ "col": 2, "row": 1, "type": "door", "state": "closed", "keyId": "gold_key" }]),
        &GRID,
    );
    wrong_key.add_key("silver_key");
    let denied = interact(&player(2, 2, Facing::N), &grid(&GRID), &mut wrong_key);
    assert_eq!(denied.result_type, InteractionType::DoorLocked);
    assert_eq!(
        wrong_key.get_door(2, 1).expect("door").state,
        DoorState::Closed
    );
}

#[test]
fn returns_nothing_for_walls_bounds_and_plain_floor() {
    let mut wall_facing = make_state(json!([]), &GRID);
    assert_eq!(
        interact(&player(1, 1, Facing::N), &grid(&GRID), &mut wall_facing).result_type,
        InteractionType::Nothing
    );

    let mut out_of_bounds = make_state(json!([]), &GRID);
    assert_eq!(
        interact(&player(0, 0, Facing::N), &grid(&GRID), &mut out_of_bounds).result_type,
        InteractionType::Nothing
    );

    let mut floor = make_state(
        json!([{ "col": 2, "row": 1, "type": "door", "state": "closed" }]),
        &GRID,
    );
    assert_eq!(
        interact(&player(1, 2, Facing::N), &grid(&GRID), &mut floor).result_type,
        InteractionType::Nothing
    );
}

#[test]
fn opened_door_becomes_walkable() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 1, "type": "door", "state": "closed" }]),
        &GRID,
    );
    assert!(!state.is_door_open(2, 1));
    interact(&player(2, 2, Facing::N), &grid(&GRID), &mut state);
    assert!(state.is_door_open(2, 1));
}

// --- Lever interaction ---

#[test]
fn lever_activation_from_lever_cell() {
    let mut state = make_state(
        json!([
            { "col": 2, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        ]),
        &LEVER_GRID,
    );
    let result = interact(&player(2, 1, Facing::N), &grid(&LEVER_GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::LeverActivated);
    assert_eq!(result.message.as_deref(), Some("Lever pulled."));
    assert_eq!(result.targets, Some(vec!["door_1".to_string()]));
    assert_eq!(state.get_door(2, 2).expect("door").state, DoorState::Open);
}

#[test]
fn lever_requires_facing_its_wall() {
    let mut state = make_state(
        json!([
            { "col": 2, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        ]),
        &LEVER_GRID,
    );
    let result = interact(&player(2, 1, Facing::S), &grid(&LEVER_GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::Nothing);
    assert_eq!(
        state.get_lever(2, 1).expect("lever present").state,
        LeverState::Up
    );
}

#[test]
fn lever_with_explicit_wall_field() {
    let mut state = make_state(
        json!([
            { "col": 2, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"], "wall": "N" },
        ]),
        &LEVER_GRID,
    );
    let result = interact(&player(2, 1, Facing::N), &grid(&LEVER_GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::LeverActivated);
}

#[test]
fn no_lever_returns_nothing() {
    let mut state = make_state(json!([]), &LEVER_GRID);
    let result = interact(&player(2, 1, Facing::N), &grid(&LEVER_GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::Nothing);
}

#[test]
fn lever_toggles_repeatedly() {
    let mut state = make_state(
        json!([
            { "col": 2, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 2, "row": 1, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        ]),
        &LEVER_GRID,
    );
    let puller = player(2, 1, Facing::N);
    assert_eq!(
        interact(&puller, &grid(&LEVER_GRID), &mut state).result_type,
        InteractionType::LeverActivated
    );
    assert_eq!(state.get_door(2, 2).expect("door").state, DoorState::Open);

    assert_eq!(
        interact(&puller, &grid(&LEVER_GRID), &mut state).result_type,
        InteractionType::LeverActivated
    );
    assert_eq!(state.get_door(2, 2).expect("door").state, DoorState::Closed);
}

// --- Block push ---

#[test]
fn valid_block_push() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 2, "type": "block" }]),
        &LEVER_GRID,
    );
    let result = interact(&player(2, 3, Facing::N), &grid(&LEVER_GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::BlockPushed);
    assert_eq!(result.target_col, Some(2));
    assert_eq!(result.target_row, Some(1));
    assert!(state.get_block(2, 2).is_none());
    assert!(state.get_block(2, 1).is_some());
}

#[test]
fn block_push_blocked_by_wall() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 1, "type": "block" }]),
        &LEVER_GRID,
    );
    let result = interact(&player(2, 2, Facing::N), &grid(&LEVER_GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::Nothing);
    assert!(state.get_block(2, 1).is_some());
}

#[test]
fn block_push_blocked_by_enemy() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 2, "type": "block" }]),
        &LEVER_GRID,
    );
    state.active_layer_mut().enemies.insert(
        door_key(2, 1),
        EnemyInstance {
            col: 2,
            row: 1,
            enemy_type: "rat".to_string(),
            hp: 8.0,
            max_hp: 8.0,
            atk: 2.0,
            def: 0.0,
            aggro_range: 3.0,
            move_interval: 0.6,
            blocks_movement: true,
            ai_state: EnemyAiState::Idle,
            move_timer: 0.0,
            regen_timer: None,
            regen_pause_timer: None,
            drops: None,
            status_effects: Vec::new(),
            spawner_id: None,
        },
    );
    let result = interact(&player(2, 3, Facing::N), &grid(&LEVER_GRID), &mut state);
    assert_eq!(result.result_type, InteractionType::Nothing);
    assert!(state.get_block(2, 2).is_some());
}

// --- Chest interaction ---

#[test]
fn chest_interaction_flows() {
    let mut unlocked = make_state(
        json!([{ "col": 2, "row": 1, "type": "chest", "state": "closed" }]),
        &LEVER_GRID,
    );
    let opened = interact(&player(2, 2, Facing::N), &grid(&LEVER_GRID), &mut unlocked);
    assert_eq!(opened.result_type, InteractionType::ChestOpened);
    assert_eq!(
        unlocked.get_chest(2, 1).expect("chest").state,
        delve_core::game_state::ChestState::Open
    );

    let mut keyed = make_state(
        json!([{ "col": 2, "row": 1, "type": "chest", "state": "locked", "keyId": "gold_key" }]),
        &LEVER_GRID,
    );
    keyed.add_key("gold_key");
    let key_opened = interact(&player(2, 2, Facing::N), &grid(&LEVER_GRID), &mut keyed);
    assert_eq!(key_opened.result_type, InteractionType::ChestOpened);
    assert!(!keyed.has_key("gold_key"));

    let mut locked = make_state(
        json!([{ "col": 2, "row": 1, "type": "chest", "state": "locked", "keyId": "gold_key" }]),
        &LEVER_GRID,
    );
    let denied = interact(&player(2, 2, Facing::N), &grid(&LEVER_GRID), &mut locked);
    assert_eq!(denied.result_type, InteractionType::ChestLocked);
    assert_eq!(
        locked.get_chest(2, 1).expect("chest").state,
        delve_core::game_state::ChestState::Locked
    );
}

// --- Sign interaction ---

#[test]
fn sign_interaction_flows() {
    let mut state = make_state(
        json!([{ "col": 2, "row": 1, "type": "sign", "wall": "N", "text": "You found a secret!" }]),
        &LEVER_GRID,
    );
    let read = interact(&player(2, 1, Facing::N), &grid(&LEVER_GRID), &mut state);
    assert_eq!(read.result_type, InteractionType::SignRead);
    assert_eq!(read.message.as_deref(), Some("You found a secret!"));

    let wrong_wall = interact(&player(2, 1, Facing::S), &grid(&LEVER_GRID), &mut state);
    assert_eq!(wrong_wall.result_type, InteractionType::Nothing);
}

//! Ported from `src/enemies/enemyAI.test.ts` and the GameState enemy-support
//! block, plus `createEnemyInstance` tests from `enemyTypes.test.ts` against
//! the shipped enemies.json.

use delve_core::enemies::EnemyDatabase;
use delve_core::enemy_ai::{EnemyActionType, EnemyUpdateContext, update_enemies};
use delve_core::game_state::{GameState, GameStateDeps, door_key};
use delve_core::grid::walkable_cells;
use delve_core::random::Mulberry32;
use delve_core::types::{EnemyAiState, Entity};
use serde_json::{Value, json};
use std::collections::HashSet;

const ENEMIES_MOCK_JSON: &str = include_str!("fixtures/enemies-ai-mock.json");
const SHIPPED_ENEMIES_JSON: &str = include_str!("../../../assets/data/enemies.json");

const GRID: [&str; 7] = [
    "#######", "#.....#", "#.....#", "#.....#", "#.....#", "#.....#", "#######",
];

fn mock_database() -> EnemyDatabase {
    EnemyDatabase::from_json(ENEMIES_MOCK_JSON).expect("mock enemies parse")
}

fn grid() -> Vec<String> {
    GRID.iter().map(ToString::to_string).collect()
}

fn make_state(entity_values: Value) -> GameState {
    let entities: Vec<Entity> = serde_json::from_value(entity_values).expect("entities parse");
    let grid = grid();
    GameState::new(
        &entities,
        Some(&grid),
        "default",
        None,
        GameStateDeps {
            items: None,
            enemy_registrar: Some(Box::new(mock_database())),
            npc_registrar: None,
        },
        &mut || 0.5,
    )
}

fn enemy_entity(col: i64, row: i64, enemy_type: &str) -> Value {
    json!({ "col": col, "row": row, "type": "enemy", "enemyType": enemy_type })
}

fn tick(
    game_state: &mut GameState,
    player_col: i64,
    player_row: i64,
    delta: f64,
) -> Vec<delve_core::enemy_ai::EnemyAction> {
    let grid = grid();
    let walkable = walkable_cells();
    let database = mock_database();
    let door_open = |_: i64, _: i64| true;
    let context = EnemyUpdateContext {
        player_col,
        player_row,
        grid: &grid,
        walkable: &walkable,
        is_door_open: &door_open,
        is_hole: None,
        is_edge_blocked: None,
        enemies: &database,
    };
    let mut rng = Mulberry32::new(42);
    let mut random = move || rng.next_f64();
    update_enemies(game_state, &context, delta, &mut random)
}

fn positions(game_state: &GameState) -> HashSet<String> {
    game_state
        .active_layer()
        .enemies
        .values()
        .map(|enemy| door_key(enemy.col, enemy.row))
        .collect()
}

// --- GameState enemy support ---

#[test]
fn parses_enemy_entities_into_enemies_map() {
    let state = make_state(json!([
        enemy_entity(1, 1, "rat"),
        enemy_entity(3, 3, "skeleton"),
    ]));
    assert_eq!(state.active_layer().enemies.len(), 2);
    assert!(state.get_enemy(1, 1).is_some());
    assert!(state.get_enemy(3, 3).is_some());
}

#[test]
fn ignores_enemy_entities_with_unknown_type() {
    let state = make_state(json!([enemy_entity(1, 1, "dragon")]));
    assert!(state.active_layer().enemies.is_empty());
}

#[test]
fn is_enemy_at_lookups() {
    let state = make_state(json!([enemy_entity(2, 2, "rat")]));
    assert!(state.is_enemy_at(2, 2));
    assert!(!state.is_enemy_at(3, 3));
}

#[test]
fn move_enemy_rekeys_the_map() {
    let mut state = make_state(json!([enemy_entity(1, 1, "rat")]));
    state.move_enemy(1, 1, 3, 3);
    assert!(!state.is_enemy_at(1, 1));
    let moved = state.get_enemy(3, 3).expect("enemy moved");
    assert_eq!((moved.col, moved.row), (3, 3));
}

#[test]
fn damage_enemy_reduces_hp_and_kills() {
    let mut state = make_state(json!([enemy_entity(1, 1, "orc")]));
    assert!(!state.damage_enemy(1, 1, 5.0));
    assert_eq!(state.get_enemy(1, 1).expect("enemy alive").hp, 35.0);

    let mut lethal = make_state(json!([enemy_entity(1, 1, "rat")]));
    assert!(lethal.damage_enemy(1, 1, 10.0));
    assert!(!lethal.is_enemy_at(1, 1));
}

// --- updateEnemies ---

#[test]
fn idle_enemy_produces_no_action_when_player_far() {
    let mut state = make_state(json!([enemy_entity(1, 1, "rat")]));
    let actions = tick(&mut state, 5, 5, 2.5);
    assert!(actions.is_empty());
}

#[test]
fn enemy_transitions_to_chase_within_aggro_range() {
    let mut state = make_state(json!([enemy_entity(3, 3, "rat")]));
    let actions = tick(&mut state, 3, 1, 2.5);
    assert_eq!(actions[0].action_type, EnemyActionType::Move);
    let enemy = state
        .get_enemy(
            actions[0].to_col.expect("move target"),
            actions[0].to_row.expect("move target"),
        )
        .expect("enemy present");
    assert_eq!(enemy.ai_state, EnemyAiState::Chase);
}

#[test]
fn enemy_moves_toward_player_during_chase() {
    let mut state = make_state(json!([enemy_entity(1, 3, "rat")]));
    let actions = tick(&mut state, 1, 1, 2.5);
    assert_eq!(actions[0].action_type, EnemyActionType::Move);
    assert!(actions[0].to_row.expect("move target") < 3);
}

#[test]
fn enemy_attacks_when_adjacent() {
    let mut state = make_state(json!([enemy_entity(2, 1, "rat")]));
    let actions = tick(&mut state, 1, 1, 2.5);
    assert_eq!(actions[0].action_type, EnemyActionType::Attack);
}

#[test]
fn enemy_returns_to_idle_outside_aggro_plus_buffer() {
    let mut state = make_state(json!([enemy_entity(1, 1, "rat")]));
    tick(&mut state, 1, 3, 2.5);
    assert_eq!(
        state
            .active_layer()
            .enemies
            .values()
            .next()
            .expect("enemy present")
            .ai_state,
        EnemyAiState::Chase
    );

    tick(&mut state, 5, 5, 2.5);
    assert_eq!(
        state
            .active_layer()
            .enemies
            .values()
            .next()
            .expect("enemy present")
            .ai_state,
        EnemyAiState::Idle
    );
}

#[test]
fn move_timer_gates_actions() {
    let mut state = make_state(json!([enemy_entity(3, 3, "skeleton")]));
    let first = tick(&mut state, 3, 1, 0.5);
    assert!(first.is_empty());

    let second = tick(&mut state, 3, 1, 1.1);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].action_type, EnemyActionType::Move);
}

#[test]
fn enemies_do_not_stack_on_the_same_cell() {
    let mut state = make_state(json!([
        enemy_entity(1, 1, "rat"),
        enemy_entity(1, 3, "rat"),
    ]));
    tick(&mut state, 1, 2, 2.5);
    let cells = positions(&state);
    assert_eq!(cells.len(), state.active_layer().enemies.len());
}

#[test]
fn enemy_does_not_move_onto_player_cell() {
    let mut state = make_state(json!([enemy_entity(3, 1, "rat")]));
    let actions = tick(&mut state, 1, 1, 2.5);
    if actions[0].action_type == EnemyActionType::Move {
        let moved_to = door_key(
            actions[0].to_col.expect("move target"),
            actions[0].to_row.expect("move target"),
        );
        assert_ne!(moved_to, door_key(1, 1));
    }
}

#[test]
fn regen_behavior_heals_over_time() {
    let mut state = make_state(json!([enemy_entity(1, 1, "troll_test")]));
    {
        let enemy = state
            .active_layer_mut()
            .enemies
            .get_mut(&door_key(1, 1))
            .expect("troll present");
        enemy.hp = 30.0;
        assert_eq!(enemy.regen_timer, Some(0.0));
        assert_eq!(enemy.regen_pause_timer, Some(0.0));
    }
    let actions = tick(&mut state, 5, 5, 1.0);
    assert!(
        actions
            .iter()
            .any(|action| action.action_type == EnemyActionType::Regen)
    );
    assert_eq!(state.get_enemy(1, 1).expect("troll present").hp, 37.0);
}

// --- createEnemyInstance against the shipped database ---

#[test]
fn create_enemy_instance_from_shipped_database() {
    let database = EnemyDatabase::from_json(SHIPPED_ENEMIES_JSON).expect("shipped enemies parse");

    let rat = database
        .create_enemy_instance(3, 5, "rat")
        .expect("rat exists");
    assert_eq!((rat.col, rat.row), (3, 5));
    assert_eq!(rat.enemy_type, "rat");
    assert_eq!(rat.max_hp, 8.0);
    assert_eq!(rat.atk, 2.0);
    assert_eq!(rat.def, 0.0);
    assert_eq!(rat.aggro_range, 3.0);
    assert_eq!(rat.move_interval, 0.6);
    assert_eq!(rat.hp, rat.max_hp);
    assert_eq!(rat.ai_state, EnemyAiState::Idle);
    assert_eq!(rat.move_timer, 0.0);
    assert!(rat.regen_timer.is_none());
    assert!(rat.regen_pause_timer.is_none());

    let troll = database
        .create_enemy_instance(1, 1, "troll")
        .expect("troll exists");
    assert_eq!(troll.regen_timer, Some(0.0));
    assert_eq!(troll.regen_pause_timer, Some(0.0));

    let error = database
        .create_enemy_instance(1, 1, "dragon")
        .expect_err("unknown type fails");
    assert_eq!(error, "Unknown enemy type: dragon");
}

#[test]
fn shipped_database_behavior_lookups() {
    let database = EnemyDatabase::from_json(SHIPPED_ENEMIES_JSON).expect("shipped enemies parse");
    assert!(database.has_behavior("troll", "regen"));
    assert!(!database.has_behavior("rat", "regen"));
    assert!(database.has_behavior("kobold", "flee"));
    assert!(database.has_behavior("giant_bat", "erratic"));

    let regen = database
        .get_behavior("troll", "regen")
        .expect("troll regen behavior");
    assert_eq!(regen.params["hpPerTick"], json!(7));
    assert_eq!(regen.params["tickInterval"], json!(1));
    assert_eq!(regen.params["pauseOnDamage"], json!(3));

    let flee = database
        .get_behavior("kobold", "flee")
        .expect("kobold flee behavior");
    assert_eq!(flee.params["hpThreshold"], json!(0.3));
    assert_eq!(flee.params["speedMultiplier"], json!(2));

    for def in database.all_enemies() {
        assert!(def.max_hp > 0.0);
        assert!(def.atk > 0.0);
        assert!(!def.sprite.path.is_empty());
    }
}

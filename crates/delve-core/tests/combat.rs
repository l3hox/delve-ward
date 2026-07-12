//! Ported from `src/core/combat.test.ts` in the TS repo. The TS `vi.mock`
//! database stubs become an explicit mock registrar and a small item database
//! fixture; `Math.random` becomes a seeded Mulberry32 stream.

use delve_core::combat::{
    CombatResultType, PLAYER_ATTACK_COOLDOWN, calculate_damage, enemy_attack_player,
    get_weapon_cooldown, player_attack, resolve_weapon_effect, weapon_behavior,
};
use delve_core::game_state::{EnemyRegistrar, GameState, GameStateDeps, NpcRegistrar};
use delve_core::grid::{Facing, PlayerState};
use delve_core::items::{ItemDatabase, ItemSubtype};
use delve_core::random::Mulberry32;
use delve_core::types::{EnemyAiState, EnemyInstance, Entity};
use serde_json::{Value, json};
use std::sync::Arc;

const COMBAT_ITEMS_JSON: &str = include_str!("fixtures/combat-items-mock.json");

struct MockEnemies;

impl EnemyRegistrar for MockEnemies {
    fn has_enemy(&self, enemy_type: &str) -> bool {
        enemy_type == "rat"
    }

    fn create_enemy(&self, col: i64, row: i64, enemy_type: &str) -> EnemyInstance {
        EnemyInstance {
            col,
            row,
            enemy_type: enemy_type.to_string(),
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
        }
    }

    fn regen_pause_duration(&self, _enemy_type: &str) -> Option<f64> {
        None
    }
}

struct MockNpcs;

impl NpcRegistrar for MockNpcs {
    fn has_npc(&self, npc_id: &str) -> bool {
        npc_id == "shopkeeper"
    }
}

fn deps() -> GameStateDeps {
    GameStateDeps {
        items: Some(Arc::new(
            ItemDatabase::from_json(COMBAT_ITEMS_JSON).expect("mock items parse"),
        )),
        enemy_registrar: Some(Box::new(MockEnemies)),
        npc_registrar: Some(Box::new(MockNpcs)),
    }
}

fn entities(values: Value) -> Vec<Entity> {
    serde_json::from_value(values).expect("test entities parse")
}

fn grid(rows: &[&str]) -> Vec<String> {
    rows.iter().map(ToString::to_string).collect()
}

fn game_state(entity_values: Value, rows: &[&str]) -> GameState {
    GameState::new(
        &entities(entity_values),
        Some(&grid(rows)),
        "default",
        None,
        deps(),
        &mut || 0.5,
    )
}

fn game_state_with_enemy() -> GameState {
    game_state(
        json!([{ "col": 3, "row": 1, "type": "enemy", "enemyType": "rat" }]),
        &["#####", "#...#", "#...#", "#####"],
    )
}

fn seeded_random(seed: u32) -> impl FnMut() -> f64 {
    let mut rng = Mulberry32::new(seed);
    move || rng.next_f64()
}

fn player(col: i32, row: i32, facing: Facing) -> PlayerState {
    PlayerState::new(col, row, facing)
}

#[test]
fn calculate_damage_always_deals_at_least_one() {
    let mut random = seeded_random(1);
    for _ in 0..50 {
        assert_eq!(calculate_damage(1.0, 10.0, &mut random), 1.0);
    }
}

#[test]
fn calculate_damage_deals_expected_range_when_atk_exceeds_def() {
    let mut random = seeded_random(2);
    let mut results = std::collections::HashSet::new();
    for _ in 0..100 {
        results.insert(calculate_damage(5.0, 2.0, &mut random) as i64);
    }
    assert!(results.contains(&2));
    assert!(results.contains(&3));
    assert!(results.contains(&4));
    assert!(results.len() <= 3);
}

#[test]
fn player_attack_hits_enemy_in_facing_cell() {
    let mut gs = game_state_with_enemy();
    let attacker = player(2, 1, Facing::E);
    let results = player_attack(&attacker, &mut gs, &mut seeded_random(3));
    let result = &results[0];
    assert!(matches!(
        result.result_type,
        CombatResultType::Hit | CombatResultType::Kill
    ));
    assert!(result.damage.expect("damage present") > 0.0);
    assert_eq!(result.target_col, Some(3));
    assert_eq!(result.target_row, Some(1));
}

#[test]
fn player_attack_returns_no_target_without_enemy() {
    let mut gs = game_state_with_enemy();
    let attacker = player(2, 1, Facing::W);
    let results = player_attack(&attacker, &mut gs, &mut seeded_random(4));
    assert_eq!(results[0].result_type, CombatResultType::NoTarget);
}

#[test]
fn player_attack_sets_cooldown() {
    let mut gs = game_state_with_enemy();
    let attacker = player(2, 1, Facing::E);
    player_attack(&attacker, &mut gs, &mut seeded_random(5));
    assert!(gs.player.attack_cooldown > 0.0);
}

#[test]
fn player_attack_respects_cooldown() {
    let mut gs = game_state_with_enemy();
    gs.player.attack_cooldown = 0.5;
    let attacker = player(2, 1, Facing::E);
    let results = player_attack(&attacker, &mut gs, &mut seeded_random(6));
    assert_eq!(results[0].result_type, CombatResultType::Cooldown);
}

#[test]
fn player_attack_can_kill() {
    let mut gs = game_state_with_enemy();
    gs.player.str = 1000.0;
    let attacker = player(2, 1, Facing::E);
    let results = player_attack(&attacker, &mut gs, &mut seeded_random(7));
    assert_eq!(results[0].result_type, CombatResultType::Kill);
    assert!(gs.get_enemy(3, 1).is_none());
}

#[test]
fn enemy_attack_reduces_player_hp() {
    let mut gs = game_state(json!([]), &["#.#"]);
    let start_hp = gs.player.hp;
    let result = enemy_attack_player(&mut gs, 3.0, &mut seeded_random(8));
    assert!(gs.player.hp < start_hp);
    assert!(result.damage > 0.0);
}

#[test]
fn enemy_attack_does_not_reduce_hp_below_zero() {
    let mut gs = game_state(json!([]), &["#.#"]);
    gs.player.hp = 1.0;
    enemy_attack_player(&mut gs, 100.0, &mut seeded_random(9));
    assert_eq!(gs.player.hp, 0.0);
}

#[test]
fn weapon_cooldown_falls_back_without_weapon() {
    let gs = game_state(json!([]), &["#.#"]);
    assert_eq!(get_weapon_cooldown(&gs), PLAYER_ATTACK_COOLDOWN);
}

#[test]
fn weapon_behavior_table_matches_ts_values() {
    let expectations = [
        (ItemSubtype::Sword, 0.8, 1.0),
        (ItemSubtype::Axe, 1.2, 1.5),
        (ItemSubtype::Dagger, 0.5, 0.7),
        (ItemSubtype::Mace, 1.1, 1.3),
        (ItemSubtype::Spear, 0.9, 1.1),
        (ItemSubtype::Staff, 1.0, 0.8),
    ];
    for (subtype, cooldown, multiplier) in expectations {
        let behavior = weapon_behavior(subtype).expect("weapon subtype has behavior");
        assert_eq!(behavior.cooldown, cooldown);
        assert_eq!(behavior.damage_multiplier, multiplier);
    }
}

#[test]
fn axe_ignores_one_def() {
    let mut random = seeded_random(10);
    let rounds = 200;
    let mut axe_total = 0.0;
    let mut sword_total = 0.0;
    for _ in 0..rounds {
        axe_total += resolve_weapon_effect(Some(ItemSubtype::Axe), 5.0, 2.0, 0.0, &mut random).0;
        sword_total +=
            resolve_weapon_effect(Some(ItemSubtype::Sword), 5.0, 2.0, 0.0, &mut random).0;
    }
    assert!(axe_total / f64::from(rounds) > sword_total / f64::from(rounds));
}

#[test]
fn dagger_overrides_crit_to_ten_percent() {
    let mut random = seeded_random(11);
    let rounds = 500;
    let mut crits = 0;
    for _ in 0..rounds {
        if resolve_weapon_effect(Some(ItemSubtype::Dagger), 5.0, 0.0, 0.0, &mut random).1 {
            crits += 1;
        }
    }
    assert!(crits > rounds * 2 / 100);
    assert!(crits < rounds * 25 / 100);
}

#[test]
fn mace_gains_bonus_damage_against_armor() {
    let mut random = seeded_random(12);
    let rounds = 200;
    let mut armored_total = 0.0;
    let mut unarmored_total = 0.0;
    for _ in 0..rounds {
        armored_total +=
            resolve_weapon_effect(Some(ItemSubtype::Mace), 5.0, 1.0, 0.0, &mut random).0;
        unarmored_total +=
            resolve_weapon_effect(Some(ItemSubtype::Mace), 5.0, 0.0, 0.0, &mut random).0;
    }
    let armored_average = armored_total / f64::from(rounds);
    let unarmored_average = unarmored_total / f64::from(rounds);
    assert!(armored_average > unarmored_average * 0.8);
}

#[test]
fn sword_and_unknown_subtypes_deal_at_least_one_damage() {
    let mut random = seeded_random(13);
    assert!(resolve_weapon_effect(Some(ItemSubtype::Sword), 5.0, 0.0, 0.0, &mut random).0 >= 1.0);
    assert!(resolve_weapon_effect(None, 5.0, 0.0, 0.0, &mut random).0 >= 1.0);
    for _ in 0..50 {
        assert!(
            resolve_weapon_effect(Some(ItemSubtype::Dagger), 1.0, 10.0, 0.0, &mut random).0 >= 1.0
        );
    }
}

#[test]
fn spear_hits_both_cells_when_equipped() {
    let mut gs = game_state(
        json!([
            { "col": 3, "row": 1, "type": "enemy", "enemyType": "rat" },
            { "col": 4, "row": 1, "type": "enemy", "enemyType": "rat" },
        ]),
        &["######", "#....#", "######"],
    );
    // Equip the spear directly through the registry.
    gs.entity_registry.create_item(
        "spear",
        delve_core::items::ItemQuality::Common,
        delve_core::entities::ItemLocation::Equipped {
            slot: delve_core::entities::EquipSlot::Weapon,
        },
        Vec::new(),
    );
    let attacker = player(2, 1, Facing::E);
    let results = player_attack(&attacker, &mut gs, &mut seeded_random(14));
    assert_eq!(results.len(), 2);
    let targets: Vec<i64> = results
        .iter()
        .filter_map(|result| result.target_col)
        .collect();
    assert!(targets.contains(&3));
    assert!(targets.contains(&4));
}

#[test]
fn npc_registrar_gates_npc_parsing() {
    let known = game_state(
        json!([{ "col": 2, "row": 2, "type": "npc", "npcId": "shopkeeper" }]),
        &["#####", "#...#", "#...#", "#...#", "#####"],
    );
    assert_eq!(known.active_layer().npcs.len(), 1);

    let unknown = game_state(
        json!([{ "col": 2, "row": 2, "type": "npc", "npcId": "unknown_npc" }]),
        &["#####", "#...#", "#...#", "#...#", "#####"],
    );
    assert!(unknown.active_layer().npcs.is_empty());
}

#[test]
fn breakable_wall_attack_returns_wall_hit() {
    let mut gs = game_state(
        json!([{ "col": 3, "row": 1, "type": "breakable_wall", "hp": 50 }]),
        &["#####", "#...#", "#...#", "#####"],
    );
    let attacker = player(2, 1, Facing::E);
    let results = player_attack(&attacker, &mut gs, &mut seeded_random(15));
    assert_eq!(results[0].result_type, CombatResultType::WallHit);
    assert!(results[0].damage.expect("damage present") > 0.0);
    assert_eq!(results[0].target_col, Some(3));
    assert_eq!(results[0].target_row, Some(1));
}

#[test]
fn enemy_takes_priority_over_breakable_wall() {
    let mut gs = game_state(
        json!([
            { "col": 3, "row": 1, "type": "enemy", "enemyType": "rat" },
            { "col": 3, "row": 1, "type": "breakable_wall", "hp": 50 },
        ]),
        &["#####", "#...#", "#...#", "#####"],
    );
    let attacker = player(2, 1, Facing::E);
    let results = player_attack(&attacker, &mut gs, &mut seeded_random(16));
    assert!(matches!(
        results[0].result_type,
        CombatResultType::Hit | CombatResultType::Kill
    ));
}

//! Ported from `src/core/questManager.test.ts`. Every `it` block there has
//! a matching `#[test]` here.
//!
//! The TS suite drives a mocked `GameState` (`vi.fn()` stubs) and asserts
//! which methods were called with which arguments. A concrete Rust
//! `GameState` can't be mocked that way, so these tests build a real
//! `GameState` and assert the resulting state instead (xp/gold/backpack
//! contents/flags) — the same behavior, verified by its effect rather than
//! by a call-spy.
//!
//! The "installConditionEvaluator" section drives
//! `QuestManager::evaluate_quest_stage_condition` directly instead of
//! `dialog_manager::evaluate_condition` — see `quest_manager`'s module doc
//! comment for why that wiring doesn't exist yet.

use delve_core::entities::ItemLocation;
use delve_core::game_state::{GameState, GameStateDeps};
use delve_core::items::ItemQuality;
use delve_core::quest_manager::{QuestManager, QuestStatus};
use delve_core::quests::{QuestDef, QuestRewards, QuestStage};
use delve_core::save_system::QuestSaveState;
use std::collections::HashMap;

fn make_def(id: &str, stage_count: usize, last_stage_rewards: QuestRewards) -> QuestDef {
    let stages = (0..stage_count)
        .map(|index| QuestStage {
            description: format!("Stage {index}"),
            rewards: if index == stage_count - 1 {
                Some(last_stage_rewards.clone())
            } else {
                None
            },
        })
        .collect();
    QuestDef {
        id: id.to_string(),
        name: format!("Quest {id}"),
        description: format!("Description for {id}"),
        stages,
    }
}

fn make_game_state() -> GameState {
    GameState::new(
        &[],
        None,
        "default",
        None,
        GameStateDeps {
            items: None,
            enemy_registrar: None,
            npc_registrar: None,
        },
        &mut || 0.5,
    )
}

fn fill_backpack(game_state: &mut GameState) {
    while let Some(slot) = game_state.entity_registry.next_backpack_slot() {
        game_state.entity_registry.create_item(
            "filler_item",
            ItemQuality::Common,
            ItemLocation::Backpack { slot },
            Vec::new(),
        );
    }
}

// ---------------------------------------------------------------------------
// 1. State transitions
// ---------------------------------------------------------------------------

#[test]
fn unknown_quest_returns_undiscovered() {
    let qm = QuestManager::new();
    assert_eq!(qm.get_status("nonexistent"), QuestStatus::Undiscovered);
}

#[test]
fn start_quest_sets_status_to_active_at_stage_index_zero() {
    let mut qm = QuestManager::new();
    qm.start_quest("q1");
    assert_eq!(qm.get_status("q1"), QuestStatus::Active);
    assert_eq!(qm.get_stage_index("q1"), 0);
}

#[test]
fn start_quest_is_a_no_op_if_quest_is_already_started() {
    let mut qm = QuestManager::new();
    qm.start_quest("q1");
    // Advance past stage 0 via advanceQuest to prove the second start is a no-op.
    qm.register_quest_def(make_def("q1", 3, QuestRewards::default()));
    qm.advance_quest("q1", &mut make_game_state());
    qm.start_quest("q1");
    assert_eq!(qm.get_stage_index("q1"), 1);
}

#[test]
fn advance_quest_increments_stage_index() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 3, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    qm.advance_quest("q1", &mut make_game_state());
    assert_eq!(qm.get_stage_index("q1"), 1);
}

#[test]
fn advance_quest_completes_the_quest_when_past_the_last_stage() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 2, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    qm.advance_quest("q1", &mut gs); // advances from stage 0 to 1 — still within bounds
    assert_eq!(qm.get_status("q1"), QuestStatus::Active);
    qm.advance_quest("q1", &mut gs); // advances from stage 1 to 2 — past last, marks complete
    assert_eq!(qm.get_status("q1"), QuestStatus::Complete);
}

#[test]
fn advance_quest_is_a_no_op_on_undiscovered_quest() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 2, QuestRewards::default());
    qm.register_quest_def(def);
    qm.advance_quest("q1", &mut make_game_state());
    assert_eq!(qm.get_status("q1"), QuestStatus::Undiscovered);
}

#[test]
fn advance_quest_is_a_no_op_on_already_completed_quest() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 1, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    qm.advance_quest("q1", &mut gs); // completes
    assert_eq!(qm.get_status("q1"), QuestStatus::Complete);
    qm.advance_quest("q1", &mut gs); // no-op
    assert_eq!(qm.get_status("q1"), QuestStatus::Complete);
    assert_eq!(qm.get_stage_index("q1"), 1);
}

// ---------------------------------------------------------------------------
// 2. Reward application
// ---------------------------------------------------------------------------

#[test]
fn xp_reward_adds_the_correct_amount() {
    let mut qm = QuestManager::new();
    let def = make_def(
        "q1",
        1,
        QuestRewards {
            xp: Some(50),
            ..QuestRewards::default()
        },
    );
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    qm.advance_quest("q1", &mut gs);
    assert_eq!(gs.player.xp, 50);
}

#[test]
fn gold_reward_increments_game_state_gold() {
    let mut qm = QuestManager::new();
    let def = make_def(
        "q1",
        1,
        QuestRewards {
            gold: Some(100),
            ..QuestRewards::default()
        },
    );
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    qm.advance_quest("q1", &mut gs);
    assert_eq!(gs.player.gold, 100);
}

#[test]
fn item_reward_creates_a_backpack_item() {
    let mut qm = QuestManager::new();
    let def = make_def(
        "q1",
        1,
        QuestRewards {
            items: Some(vec!["sword_iron".to_string()]),
            ..QuestRewards::default()
        },
    );
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    qm.advance_quest("q1", &mut gs);
    let created = gs
        .entity_registry
        .snapshot()
        .iter()
        .any(|item| item.item_id == "sword_iron");
    assert!(created);
}

#[test]
fn item_reward_is_skipped_when_backpack_is_full() {
    let mut qm = QuestManager::new();
    let def = make_def(
        "q1",
        1,
        QuestRewards {
            items: Some(vec!["sword_iron".to_string()]),
            ..QuestRewards::default()
        },
    );
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    fill_backpack(&mut gs);
    qm.advance_quest("q1", &mut gs);
    let created = gs
        .entity_registry
        .snapshot()
        .iter()
        .any(|item| item.item_id == "sword_iron");
    assert!(!created);
}

#[test]
fn flag_reward_sets_each_flag() {
    let mut qm = QuestManager::new();
    let def = make_def(
        "q1",
        1,
        QuestRewards {
            flags: Some(vec!["quest_done".to_string(), "gate_open".to_string()]),
            ..QuestRewards::default()
        },
    );
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    qm.advance_quest("q1", &mut gs);
    assert!(gs.player.flags.contains("quest_done"));
    assert!(gs.player.flags.contains("gate_open"));
    assert_eq!(gs.player.flags.len(), 2);
}

#[test]
fn no_rewards_none_of_the_reward_effects_apply() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 1, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    let mut gs = make_game_state();
    qm.advance_quest("q1", &mut gs);
    assert_eq!(gs.player.xp, 0);
    assert_eq!(gs.player.gold, 0);
    assert!(gs.entity_registry.snapshot().is_empty());
    assert!(gs.player.flags.is_empty());
}

// ---------------------------------------------------------------------------
// 3. Serialization roundtrip
// ---------------------------------------------------------------------------

#[test]
fn get_serializable_state_returns_the_expected_shape() {
    let mut qm = QuestManager::new();
    qm.start_quest("q1");
    let out = qm.get_serializable_state();
    assert_eq!(
        out.get("q1"),
        Some(&QuestSaveState {
            status: "active".to_string(),
            stage_index: 0,
        })
    );
}

#[test]
fn restore_state_rebuilds_state_from_a_snapshot() {
    let mut qm = QuestManager::new();
    let mut data = HashMap::new();
    data.insert(
        "q1".to_string(),
        QuestSaveState {
            status: "active".to_string(),
            stage_index: 2,
        },
    );
    data.insert(
        "q2".to_string(),
        QuestSaveState {
            status: "complete".to_string(),
            stage_index: 3,
        },
    );
    qm.restore_state(data).expect("valid statuses restore");
    assert_eq!(qm.get_status("q1"), QuestStatus::Active);
    assert_eq!(qm.get_stage_index("q1"), 2);
    assert_eq!(qm.get_status("q2"), QuestStatus::Complete);
    assert_eq!(qm.get_stage_index("q2"), 3);
}

#[test]
fn serialization_roundtrip_preserves_status_and_stage_index() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 3, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    qm.advance_quest("q1", &mut make_game_state());

    let snapshot = qm.get_serializable_state();

    let mut qm2 = QuestManager::new();
    qm2.restore_state(snapshot).expect("valid statuses restore");

    assert_eq!(qm2.get_status("q1"), QuestStatus::Active);
    assert_eq!(qm2.get_stage_index("q1"), 1);
}

#[test]
fn restore_state_clears_previously_tracked_quests() {
    let mut qm = QuestManager::new();
    qm.start_quest("old");
    let mut data = HashMap::new();
    data.insert(
        "q1".to_string(),
        QuestSaveState {
            status: "active".to_string(),
            stage_index: 0,
        },
    );
    qm.restore_state(data).expect("valid statuses restore");
    assert_eq!(qm.get_status("old"), QuestStatus::Undiscovered);
    assert_eq!(qm.get_status("q1"), QuestStatus::Active);
}

// ---------------------------------------------------------------------------
// 4. get_active_quests / get_completed_quests
// ---------------------------------------------------------------------------

#[test]
fn get_active_quests_returns_ids_of_active_quests() {
    let mut qm = QuestManager::new();
    qm.start_quest("q1");
    qm.start_quest("q2");
    let active = qm.get_active_quests();
    assert!(active.contains(&"q1".to_string()));
    assert!(active.contains(&"q2".to_string()));
    assert_eq!(active.len(), 2);
}

#[test]
fn get_completed_quests_returns_ids_of_completed_quests() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 1, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    qm.advance_quest("q1", &mut make_game_state());
    let completed = qm.get_completed_quests();
    assert!(completed.contains(&"q1".to_string()));
    assert_eq!(completed.len(), 1);
}

#[test]
fn get_active_quests_excludes_completed_quests() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 1, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    qm.advance_quest("q1", &mut make_game_state());
    assert!(!qm.get_active_quests().contains(&"q1".to_string()));
}

#[test]
fn returns_empty_vecs_when_no_quests_match() {
    let qm = QuestManager::new();
    assert!(qm.get_active_quests().is_empty());
    assert!(qm.get_completed_quests().is_empty());
}

// ---------------------------------------------------------------------------
// 5. get_stage_index
// ---------------------------------------------------------------------------

#[test]
fn get_stage_index_returns_negative_one_for_undiscovered_quest() {
    let qm = QuestManager::new();
    assert_eq!(qm.get_stage_index("q1"), -1);
}

#[test]
fn get_stage_index_returns_zero_after_start_quest() {
    let mut qm = QuestManager::new();
    qm.start_quest("q1");
    assert_eq!(qm.get_stage_index("q1"), 0);
}

#[test]
fn get_stage_index_increments_after_advance_quest() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 3, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    qm.advance_quest("q1", &mut make_game_state());
    assert_eq!(qm.get_stage_index("q1"), 1);
    qm.advance_quest("q1", &mut make_game_state());
    assert_eq!(qm.get_stage_index("q1"), 2);
}

// ---------------------------------------------------------------------------
// 6. Condition evaluator integration
// ---------------------------------------------------------------------------

#[test]
fn quest_stage_condition_undiscovered_when_quest_has_not_started() {
    let qm = QuestManager::new();
    assert!(qm.evaluate_quest_stage_condition(Some("q1"), "undiscovered"));
}

#[test]
fn quest_stage_undiscovered_becomes_false_after_start_quest() {
    let mut qm = QuestManager::new();
    qm.start_quest("q1");
    assert!(!qm.evaluate_quest_stage_condition(Some("q1"), "undiscovered"));
}

#[test]
fn quest_stage_active_is_true_while_quest_is_in_progress() {
    let mut qm = QuestManager::new();
    qm.start_quest("q1");
    assert!(qm.evaluate_quest_stage_condition(Some("q1"), "active"));
}

#[test]
fn quest_stage_complete_is_true_after_quest_is_finished() {
    let mut qm = QuestManager::new();
    let def = make_def("q1", 1, QuestRewards::default());
    qm.register_quest_def(def);
    qm.start_quest("q1");
    qm.advance_quest("q1", &mut make_game_state());
    assert!(qm.evaluate_quest_stage_condition(Some("q1"), "complete"));
}

#[test]
fn quest_stage_returns_false_when_quest_id_is_missing() {
    let qm = QuestManager::new();
    assert!(!qm.evaluate_quest_stage_condition(None, "undiscovered"));
}

#[test]
fn quest_stage_returns_false_for_an_unrecognised_stage_value() {
    let qm = QuestManager::new();
    assert!(!qm.evaluate_quest_stage_condition(Some("q1"), "in_progress"));
}

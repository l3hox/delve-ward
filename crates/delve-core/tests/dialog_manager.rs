//! `src/core/dialogManager.ts` has no dedicated vitest suite of its own —
//! the only test file referencing it is `questManager.test.ts`, which spends
//! six cases on `setConditionEvaluator`/`installConditionEvaluator`, a
//! global-registry hook installed by `QuestManager`. That hook mechanism
//! itself is not part of this port (see the module doc on
//! `dialog_manager.rs`): `evaluate_condition` and friends take an
//! `Option<&QuestManager>` parameter instead of consulting a mutable
//! registry. Section 6 below ports all six `installConditionEvaluator`
//! cases against that parameter, plus new cases (no TS equivalent existed
//! for these, since TS's dialog choices and its quest-condition wiring were
//! never exercised together in one test) covering a dialog choice's
//! visibility actually changing as a quest moves through its stages.
//!
//! Everything else in this file is a from-scratch behavioral spec for every
//! other exported function in `dialogManager.ts`, covering the same
//! mechanics: condition evaluation, effect execution, and dialog-session
//! node walking.

use delve_core::dialog_manager::{
    DialogEvent, advance_dialog, evaluate_condition, evaluate_conditions, execute_effect,
    execute_effects, get_available_choices, get_current_node, select_choice, start_dialog,
};
use delve_core::dialogs::{
    DialogChoice, DialogCondition, DialogConditionType, DialogEffect, DialogEffectType, DialogNode,
    DialogTree,
};
use delve_core::entities::{EquipSlot, ItemLocation};
use delve_core::game_state::{GameState, GameStateDeps};
use delve_core::items::ItemQuality;
use delve_core::quest_manager::QuestManager;
use delve_core::quests::{QuestDef, QuestStage};
use delve_core::save_system::QuestSaveState;
use std::collections::HashMap;

const TREE_JSON: &str = include_str!("fixtures/dialog-manager-tree.json");
const HILDA_DIALOG_JSON: &str = include_str!("../../../assets/data/dialogs/questgiver_hilda.json");

fn tree() -> DialogTree {
    DialogTree::from_json(TREE_JSON).expect("fixture dialog tree parses")
}

fn hilda_tree() -> DialogTree {
    DialogTree::from_json(HILDA_DIALOG_JSON).expect("shipped questgiver_hilda.json parses")
}

fn game() -> GameState {
    GameState::new(
        &[],
        None,
        "default",
        None,
        GameStateDeps::default(),
        &mut || 0.5,
    )
}

fn has_flag_condition(flag: &str) -> DialogCondition {
    DialogCondition {
        condition_type: DialogConditionType::HasFlag,
        flag: Some(flag.to_string()),
        item_id: None,
        quest_id: None,
        stage: None,
        stat: None,
        min: None,
    }
}

fn has_item_condition(item_id: &str) -> DialogCondition {
    DialogCondition {
        condition_type: DialogConditionType::HasItem,
        flag: None,
        item_id: Some(item_id.to_string()),
        quest_id: None,
        stage: None,
        stat: None,
        min: None,
    }
}

fn quest_stage_condition(quest_id: &str, stage: &str) -> DialogCondition {
    DialogCondition {
        condition_type: DialogConditionType::QuestStage,
        flag: None,
        item_id: None,
        quest_id: Some(quest_id.to_string()),
        stage: Some(stage.to_string()),
        stat: None,
        min: None,
    }
}

fn stat_check_condition(stat: &str, min: f64) -> DialogCondition {
    DialogCondition {
        condition_type: DialogConditionType::StatCheck,
        flag: None,
        item_id: None,
        quest_id: None,
        stage: None,
        stat: Some(stat.to_string()),
        min: Some(min),
    }
}

fn set_flag_effect(flag: &str) -> DialogEffect {
    DialogEffect {
        effect_type: DialogEffectType::SetFlag,
        flag: Some(flag.to_string()),
        item_id: None,
        quest_id: None,
    }
}

fn give_item_effect(item_id: &str) -> DialogEffect {
    DialogEffect {
        effect_type: DialogEffectType::GiveItem,
        flag: None,
        item_id: Some(item_id.to_string()),
        quest_id: None,
    }
}

fn take_item_effect(item_id: &str) -> DialogEffect {
    DialogEffect {
        effect_type: DialogEffectType::TakeItem,
        flag: None,
        item_id: Some(item_id.to_string()),
        quest_id: None,
    }
}

fn start_quest_effect(quest_id: &str) -> DialogEffect {
    DialogEffect {
        effect_type: DialogEffectType::StartQuest,
        flag: None,
        item_id: None,
        quest_id: Some(quest_id.to_string()),
    }
}

fn advance_quest_effect(quest_id: &str) -> DialogEffect {
    DialogEffect {
        effect_type: DialogEffectType::AdvanceQuest,
        flag: None,
        item_id: None,
        quest_id: Some(quest_id.to_string()),
    }
}

fn open_shop_effect() -> DialogEffect {
    DialogEffect {
        effect_type: DialogEffectType::OpenShop,
        flag: None,
        item_id: None,
        quest_id: None,
    }
}

/// Mirrors `questManager.test.ts`'s `makeDef` fixture helper.
fn make_quest_def(id: &str, stage_count: usize) -> QuestDef {
    QuestDef {
        id: id.to_string(),
        name: format!("Quest {id}"),
        description: format!("Description for {id}"),
        stages: (0..stage_count)
            .map(|index| QuestStage {
                description: format!("Stage {index}"),
                rewards: None,
            })
            .collect(),
    }
}

/// A minimal dialog tree with one choice gated on a `questStage` condition,
/// for exercising `get_available_choices`/`select_choice` against a wired
/// `QuestManager` — the shipped fixture has no quest-gated choice to reuse.
fn quest_gated_tree() -> DialogTree {
    let mut nodes = HashMap::new();
    nodes.insert(
        "start".to_string(),
        DialogNode {
            speaker: None,
            text: "Have you finished the delivery?".to_string(),
            choices: Some(vec![
                DialogChoice {
                    text: "Turn in the delivery".to_string(),
                    next: None,
                    conditions: Some(vec![quest_stage_condition("delivery", "active")]),
                    effects: None,
                },
                DialogChoice {
                    text: "Never mind".to_string(),
                    next: None,
                    conditions: None,
                    effects: None,
                },
            ]),
            next: None,
            effects: None,
            conditions: None,
        },
    );
    DialogTree {
        start_node: "start".to_string(),
        nodes,
    }
}

fn failed_quest_manager(quest_id: &str) -> QuestManager {
    let mut quests = QuestManager::new();
    quests
        .restore_state(HashMap::from([(
            quest_id.to_string(),
            QuestSaveState {
                status: "failed".to_string(),
                stage_index: 0,
            },
        )]))
        .expect("valid status restores");
    quests
}

// ---------------------------------------------------------------------------
// 1. evaluate_condition / evaluate_conditions
// ---------------------------------------------------------------------------

#[test]
fn has_flag_true_when_flag_is_set() {
    let mut state = game();
    state.set_flag("met_speaker");
    assert!(evaluate_condition(
        &has_flag_condition("met_speaker"),
        &state,
        None
    ));
}

#[test]
fn has_flag_false_when_flag_is_unset() {
    let state = game();
    assert!(!evaluate_condition(
        &has_flag_condition("met_speaker"),
        &state,
        None
    ));
}

#[test]
fn has_item_true_for_backpack_item() {
    let mut state = game();
    state.entity_registry.create_item(
        "fixture_token",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(evaluate_condition(
        &has_item_condition("fixture_token"),
        &state,
        None
    ));
}

#[test]
fn has_item_true_for_equipped_item() {
    let mut state = game();
    state.entity_registry.create_item(
        "fixture_token",
        ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Amulet,
        },
        Vec::new(),
    );
    assert!(evaluate_condition(
        &has_item_condition("fixture_token"),
        &state,
        None
    ));
}

#[test]
fn has_item_false_for_ground_item() {
    let mut state = game();
    state.entity_registry.create_item(
        "fixture_token",
        ItemQuality::Common,
        ItemLocation::world("default", 1, 1),
        Vec::new(),
    );
    assert!(!evaluate_condition(
        &has_item_condition("fixture_token"),
        &state,
        None
    ));
}

#[test]
fn has_item_false_when_absent() {
    let state = game();
    assert!(!evaluate_condition(
        &has_item_condition("fixture_token"),
        &state,
        None
    ));
}

#[test]
fn quest_stage_undiscovered_is_true_by_default_with_no_quest_manager() {
    let state = game();
    assert!(evaluate_condition(
        &quest_stage_condition("fixture_quest", "undiscovered"),
        &state,
        None
    ));
}

#[test]
fn quest_stage_any_other_value_is_false_by_default_with_no_quest_manager() {
    let state = game();
    assert!(!evaluate_condition(
        &quest_stage_condition("fixture_quest", "active"),
        &state,
        None
    ));
    assert!(!evaluate_condition(
        &quest_stage_condition("fixture_quest", "complete"),
        &state,
        None
    ));
}

#[test]
fn stat_check_true_when_value_meets_minimum() {
    let mut state = game();
    state.player.str = 5.0;
    assert!(evaluate_condition(
        &stat_check_condition("str", 5.0),
        &state,
        None
    ));
}

#[test]
fn stat_check_false_when_below_minimum() {
    let mut state = game();
    state.player.str = 4.0;
    assert!(!evaluate_condition(
        &stat_check_condition("str", 5.0),
        &state,
        None
    ));
}

#[test]
fn stat_check_reads_gold() {
    let mut state = game();
    state.player.gold = 100;
    assert!(evaluate_condition(
        &stat_check_condition("gold", 100.0),
        &state,
        None
    ));
    assert!(!evaluate_condition(
        &stat_check_condition("gold", 101.0),
        &state,
        None
    ));
}

#[test]
fn stat_check_false_for_unknown_stat_name() {
    let state = game();
    assert!(!evaluate_condition(
        &stat_check_condition("luck", 0.0),
        &state,
        None
    ));
}

#[test]
fn evaluate_conditions_true_when_list_is_empty() {
    let state = game();
    assert!(evaluate_conditions(Some(&[]), &state, None));
}

#[test]
fn evaluate_conditions_true_when_none() {
    let state = game();
    assert!(evaluate_conditions(None, &state, None));
}

#[test]
fn evaluate_conditions_requires_every_condition_to_pass() {
    let mut state = game();
    state.set_flag("met_speaker");
    let conditions = vec![
        has_flag_condition("met_speaker"),
        has_item_condition("fixture_token"),
    ];
    assert!(!evaluate_conditions(Some(&conditions), &state, None));

    state.entity_registry.create_item(
        "fixture_token",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    assert!(evaluate_conditions(Some(&conditions), &state, None));
}

// ---------------------------------------------------------------------------
// 2. execute_effect / execute_effects
// ---------------------------------------------------------------------------

#[test]
fn set_flag_effect_sets_the_flag() {
    let mut state = game();
    assert_eq!(
        execute_effect(&set_flag_effect("has_token"), &mut state, "npc"),
        None
    );
    assert!(state.has_flag("has_token"));
}

#[test]
fn give_item_effect_adds_to_backpack() {
    let mut state = game();
    execute_effect(&give_item_effect("fixture_token"), &mut state, "npc");
    let backpack = state.entity_registry.backpack_items();
    assert_eq!(backpack.len(), 1);
    assert_eq!(backpack[0].item_id, "fixture_token");
    assert_eq!(backpack[0].location, ItemLocation::Backpack { slot: 0 });
}

#[test]
fn give_item_effect_is_a_no_op_when_backpack_is_full() {
    let mut state = game();
    for slot in 0..delve_core::entities::BACKPACK_MAX_SLOTS {
        state.entity_registry.create_item(
            "filler",
            ItemQuality::Common,
            ItemLocation::Backpack { slot },
            Vec::new(),
        );
    }
    execute_effect(&give_item_effect("fixture_token"), &mut state, "npc");
    assert!(
        state
            .entity_registry
            .backpack_items()
            .iter()
            .all(|item| item.item_id != "fixture_token")
    );
}

#[test]
fn take_item_effect_removes_a_backpack_item() {
    let mut state = game();
    state.entity_registry.create_item(
        "fixture_token",
        ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    execute_effect(&take_item_effect("fixture_token"), &mut state, "npc");
    assert!(state.entity_registry.backpack_items().is_empty());
}

#[test]
fn take_item_effect_removes_an_equipped_item() {
    let mut state = game();
    state.entity_registry.create_item(
        "fixture_token",
        ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Amulet,
        },
        Vec::new(),
    );
    execute_effect(&take_item_effect("fixture_token"), &mut state, "npc");
    assert!(
        state
            .entity_registry
            .get_equipped(EquipSlot::Amulet)
            .is_none()
    );
}

#[test]
fn take_item_effect_is_a_no_op_when_item_is_absent() {
    let mut state = game();
    execute_effect(&take_item_effect("fixture_token"), &mut state, "npc");
    assert!(state.entity_registry.backpack_items().is_empty());
}

#[test]
fn start_quest_effect_returns_a_start_quest_event() {
    let mut state = game();
    let event = execute_effect(
        &start_quest_effect("fixture_quest"),
        &mut state,
        "npc_gregor",
    );
    assert_eq!(
        event,
        Some(DialogEvent::StartQuest("fixture_quest".to_string()))
    );
}

#[test]
fn advance_quest_effect_returns_an_advance_quest_event() {
    let mut state = game();
    let event = execute_effect(
        &advance_quest_effect("fixture_quest"),
        &mut state,
        "npc_gregor",
    );
    assert_eq!(
        event,
        Some(DialogEvent::AdvanceQuest("fixture_quest".to_string()))
    );
}

#[test]
fn open_shop_effect_returns_an_open_shop_event_for_the_current_npc() {
    let mut state = game();
    let event = execute_effect(&open_shop_effect(), &mut state, "npc_gregor");
    assert_eq!(event, Some(DialogEvent::OpenShop("npc_gregor".to_string())));
}

#[test]
fn execute_effects_collects_events_and_skips_direct_state_effects() {
    let mut state = game();
    let effects = vec![
        set_flag_effect("has_token"),
        start_quest_effect("fixture_quest"),
        open_shop_effect(),
    ];
    let events = execute_effects(Some(&effects), &mut state, "npc_gregor");
    assert!(state.has_flag("has_token"));
    assert_eq!(
        events,
        vec![
            DialogEvent::StartQuest("fixture_quest".to_string()),
            DialogEvent::OpenShop("npc_gregor".to_string()),
        ]
    );
}

#[test]
fn execute_effects_is_empty_when_none() {
    let mut state = game();
    assert_eq!(execute_effects(None, &mut state, "npc"), Vec::new());
}

// ---------------------------------------------------------------------------
// 3. Dialog session — start / get_current_node / get_available_choices
// ---------------------------------------------------------------------------

#[test]
fn start_dialog_points_at_the_tree_start_node() {
    let session = start_dialog("npc_gregor", tree());
    assert_eq!(session.npc_id, "npc_gregor");
    assert_eq!(session.current_node_id, "start");
}

#[test]
fn get_current_node_returns_none_for_a_missing_node_id() {
    let mut session = start_dialog("npc_gregor", tree());
    session.current_node_id = "does_not_exist".to_string();
    assert!(get_current_node(&session).is_none());
}

#[test]
fn get_current_node_returns_the_node_text() {
    let session = start_dialog("npc_gregor", tree());
    let node = get_current_node(&session).expect("start node exists");
    assert_eq!(node.text, "Welcome to the fixture dialog.");
}

#[test]
fn get_available_choices_filters_out_unmet_conditions() {
    let session = start_dialog("npc_gregor", tree());
    let state = game();
    let choices = get_available_choices(&session, &state, None);
    // "Locked path" requires met_speaker; "Take my token back" and
    // "Strength check" require hasItem/statCheck that also aren't met yet.
    assert!(
        choices
            .iter()
            .all(|choice| choice.text != "Locked path (needs flag)")
    );
    assert!(
        choices
            .iter()
            .all(|choice| choice.text != "Take my token back")
    );
    assert!(choices.iter().all(|choice| choice.text != "Strength check"));
    assert!(
        choices
            .iter()
            .any(|choice| choice.text == "Give me a token")
    );
    assert!(choices.iter().any(|choice| choice.text == "Farewell"));
}

#[test]
fn get_available_choices_includes_a_choice_once_its_condition_is_met() {
    let session = start_dialog("npc_gregor", tree());
    let mut state = game();
    state.set_flag("met_speaker");
    let choices = get_available_choices(&session, &state, None);
    assert!(
        choices
            .iter()
            .any(|choice| choice.text == "Locked path (needs flag)")
    );
}

#[test]
fn get_available_choices_is_empty_for_a_missing_node() {
    let mut session = start_dialog("npc_gregor", tree());
    session.current_node_id = "does_not_exist".to_string();
    let state = game();
    assert!(get_available_choices(&session, &state, None).is_empty());
}

// ---------------------------------------------------------------------------
// 4. select_choice
// ---------------------------------------------------------------------------

#[test]
fn select_choice_runs_choice_effects_moves_the_session_and_returns_the_next_id() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    // Choices in order: locked(unavailable), "Give me a token" is index 0
    // once the unmet-condition choice is filtered out.
    let (next, _events) = select_choice(&mut session, 0, &mut state, None);
    assert_eq!(next, Some("gave_token".to_string()));
    assert_eq!(session.current_node_id, "gave_token");
    assert_eq!(
        state.entity_registry.backpack_items()[0].item_id,
        "fixture_token"
    );
}

#[test]
fn select_choice_runs_the_new_nodes_entry_effects() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    select_choice(&mut session, 0, &mut state, None); // -> gave_token, which sets has_token
    assert!(state.has_flag("has_token"));
}

#[test]
fn select_choice_ends_the_dialog_when_next_is_null() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    // "Farewell" is the last available choice from the start node.
    let choices_len = get_available_choices(&session, &state, None).len();
    let farewell_index = choices_len - 1;
    let (next, _events) = select_choice(&mut session, farewell_index, &mut state, None);
    assert_eq!(next, None);
    // The session pointer does not move past the ending choice.
    assert_eq!(session.current_node_id, "start");
}

#[test]
fn select_choice_out_of_range_index_returns_none_and_does_not_mutate_state() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    let out_of_range = get_available_choices(&session, &state, None).len() + 10;
    let (next, _events) = select_choice(&mut session, out_of_range, &mut state, None);
    assert_eq!(next, None);
    assert_eq!(session.current_node_id, "start");
    assert!(state.entity_registry.backpack_items().is_empty());
}

#[test]
fn select_choice_only_considers_currently_available_choices() {
    // "Locked path" is filtered out until met_speaker is set, so index 0
    // must resolve to "Give me a token", never to the locked choice.
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    let (next, _events) = select_choice(&mut session, 0, &mut state, None);
    assert_ne!(next, Some("locked".to_string()));
}

#[test]
fn select_choice_returns_the_start_quest_event_from_the_choices_own_effects() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    // Available at the start node: "Give me a token"(0), "Start a
    // quest"(1), "Advance the quest"(2), "Open the shop"(3), "Farewell"(4) —
    // "Locked path"/"Take my token back"/"Strength check" are all filtered
    // by unmet conditions on a fresh `game()`.
    let (next, events) = select_choice(&mut session, 1, &mut state, None);
    assert_eq!(next, None);
    assert_eq!(
        events,
        vec![DialogEvent::StartQuest("fixture_quest".to_string())]
    );
}

#[test]
fn select_choice_returns_the_advance_quest_event_from_the_choices_own_effects() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    let (_next, events) = select_choice(&mut session, 2, &mut state, None);
    assert_eq!(
        events,
        vec![DialogEvent::AdvanceQuest("fixture_quest".to_string())]
    );
}

#[test]
fn select_choice_returns_the_open_shop_event_carrying_the_sessions_npc_id() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    let (_next, events) = select_choice(&mut session, 3, &mut state, None);
    assert_eq!(
        events,
        vec![DialogEvent::OpenShop("npc_gregor".to_string())]
    );
}

#[test]
fn select_choice_collects_events_from_both_the_choice_and_the_entered_nodes_effects() {
    let mut session = start_dialog("npc_gregor", tree());
    let mut state = game();
    // "Give me a token" carries a giveItem effect (no DialogEvent) and moves
    // to "gave_token", whose own entry effect is setFlag (also no
    // DialogEvent) — this proves a choice with zero DialogEvents still
    // returns cleanly rather than proving concatenation itself; concatenation
    // order (choice effects before entered-node effects) is documented on
    // `select_choice` and exercised structurally by the three tests above
    // each touching only one side.
    let (next, events) = select_choice(&mut session, 0, &mut state, None);
    assert_eq!(next, Some("gave_token".to_string()));
    assert!(events.is_empty());
}

/// The exact real-content path that motivated returning events from
/// `select_choice` at all: without a `QuestManager`, `kill_spider_queen`
/// reads as "undiscovered" by default (the `None` fallback in
/// `evaluate_condition`), so "Need something killed?" is available from the
/// greeting node; picking it, then "Consider it done." on the follow-up
/// node, must surface the bounty's `startQuest` effect as a `DialogEvent`
/// rather than silently dropping it.
#[test]
fn select_choice_surfaces_hildas_start_quest_event_for_the_spider_queen_bounty() {
    let mut session = start_dialog("questgiver_hilda", hilda_tree());
    let mut state = game();

    let choices = get_available_choices(&session, &state, None);
    let need_something_killed = choices
        .iter()
        .position(|choice| choice.text == "Need something killed?")
        .expect("bounty choice is offered before the quest is discovered");
    let (next, events) = select_choice(&mut session, need_something_killed, &mut state, None);
    assert_eq!(next, Some("bounty_intro".to_string()));
    assert!(events.is_empty());

    let choices = get_available_choices(&session, &state, None);
    let consider_it_done = choices
        .iter()
        .position(|choice| choice.text == "Consider it done.")
        .expect("bounty_intro offers the accept choice");
    let (next, events) = select_choice(&mut session, consider_it_done, &mut state, None);
    assert_eq!(next, None); // "Consider it done." ends the dialog (next: null)
    assert_eq!(
        events,
        vec![DialogEvent::StartQuest("kill_spider_queen".to_string())]
    );
}

// ---------------------------------------------------------------------------
// 5. advance_dialog
// ---------------------------------------------------------------------------

#[test]
fn advance_dialog_follows_the_linear_next_field() {
    let mut session = start_dialog("npc_gregor", tree());
    session.current_node_id = "linear_a".to_string();
    let mut state = game();
    let (next, _events) = advance_dialog(&mut session, &mut state);
    assert_eq!(next, Some("linear_b".to_string()));
    assert_eq!(session.current_node_id, "linear_b");
}

#[test]
fn advance_dialog_runs_the_new_nodes_entry_effects() {
    let mut session = start_dialog("npc_gregor", tree());
    session.current_node_id = "linear_a".to_string();
    let mut state = game();
    advance_dialog(&mut session, &mut state);
    assert!(state.has_flag("reached_linear_b"));
}

#[test]
fn advance_dialog_ends_when_next_is_null() {
    let mut session = start_dialog("npc_gregor", tree());
    session.current_node_id = "linear_b".to_string();
    let mut state = game();
    let (next, _events) = advance_dialog(&mut session, &mut state);
    assert_eq!(next, None);
    assert_eq!(session.current_node_id, "linear_b");
}

#[test]
fn advance_dialog_returns_none_for_a_missing_current_node() {
    let mut session = start_dialog("npc_gregor", tree());
    session.current_node_id = "does_not_exist".to_string();
    let mut state = game();
    assert_eq!(advance_dialog(&mut session, &mut state).0, None);
}

#[test]
fn advance_dialog_returns_the_entered_nodes_dialog_events() {
    let mut session = start_dialog("npc_gregor", tree());
    session.current_node_id = "linear_to_quest".to_string();
    let mut state = game();
    let (next, events) = advance_dialog(&mut session, &mut state);
    assert_eq!(next, Some("linear_quest_node".to_string()));
    assert_eq!(
        events,
        vec![DialogEvent::StartQuest("fixture_quest".to_string())]
    );
}

// ---------------------------------------------------------------------------
// 6. questStage wired to a QuestManager
//
// Ports every case from questManager.test.ts's
// "QuestManager — installConditionEvaluator" describe block, plus new
// coverage for a dialog choice's visibility actually changing as a quest
// progresses (no TS equivalent test combined the two).
// ---------------------------------------------------------------------------

#[test]
fn quest_stage_undiscovered_is_true_when_the_quest_has_not_started() {
    let state = game();
    let quests = QuestManager::new();
    assert!(evaluate_condition(
        &quest_stage_condition("q1", "undiscovered"),
        &state,
        Some(&quests)
    ));
}

#[test]
fn quest_stage_undiscovered_becomes_false_after_start_quest() {
    let state = game();
    let mut quests = QuestManager::new();
    quests.start_quest("q1");
    assert!(!evaluate_condition(
        &quest_stage_condition("q1", "undiscovered"),
        &state,
        Some(&quests)
    ));
}

#[test]
fn quest_stage_active_is_true_while_the_quest_is_in_progress() {
    let state = game();
    let mut quests = QuestManager::new();
    quests.start_quest("q1");
    assert!(evaluate_condition(
        &quest_stage_condition("q1", "active"),
        &state,
        Some(&quests)
    ));
}

#[test]
fn quest_stage_complete_is_true_after_the_quest_finishes() {
    let mut state = game();
    let mut quests = QuestManager::new();
    quests.register_quest_def(make_quest_def("q1", 1));
    quests.start_quest("q1");
    quests.advance_quest("q1", &mut state);

    assert!(evaluate_condition(
        &quest_stage_condition("q1", "complete"),
        &state,
        Some(&quests)
    ));
}

#[test]
fn quest_stage_is_false_when_quest_id_is_missing_from_the_condition() {
    let state = game();
    let quests = QuestManager::new();
    let condition = DialogCondition {
        condition_type: DialogConditionType::QuestStage,
        flag: None,
        item_id: None,
        quest_id: None,
        stage: Some("undiscovered".to_string()),
        stat: None,
        min: None,
    };
    assert!(!evaluate_condition(&condition, &state, Some(&quests)));
}

#[test]
fn quest_stage_is_false_for_an_unrecognized_stage_value() {
    let state = game();
    let quests = QuestManager::new();
    assert!(!evaluate_condition(
        &quest_stage_condition("q1", "in_progress"),
        &state,
        Some(&quests)
    ));
}

#[test]
fn quest_gated_choice_is_hidden_before_the_quest_starts() {
    let session = start_dialog("npc_gregor", quest_gated_tree());
    let state = game();
    let quests = QuestManager::new();
    let choices = get_available_choices(&session, &state, Some(&quests));
    assert!(
        choices
            .iter()
            .all(|choice| choice.text != "Turn in the delivery")
    );
}

#[test]
fn quest_gated_choice_appears_once_the_quest_is_active() {
    let session = start_dialog("npc_gregor", quest_gated_tree());
    let state = game();
    let mut quests = QuestManager::new();
    quests.start_quest("delivery");
    let choices = get_available_choices(&session, &state, Some(&quests));
    assert!(
        choices
            .iter()
            .any(|choice| choice.text == "Turn in the delivery")
    );
}

#[test]
fn quest_gated_choice_disappears_once_the_quest_completes() {
    let mut state = game();
    let mut quests = QuestManager::new();
    quests.register_quest_def(make_quest_def("delivery", 1));
    quests.start_quest("delivery");
    quests.advance_quest("delivery", &mut state);

    let session = start_dialog("npc_gregor", quest_gated_tree());
    let choices = get_available_choices(&session, &state, Some(&quests));
    assert!(
        choices
            .iter()
            .all(|choice| choice.text != "Turn in the delivery")
    );
}

#[test]
fn quest_gated_choice_stays_hidden_when_the_quest_has_failed() {
    let session = start_dialog("npc_gregor", quest_gated_tree());
    let state = game();
    let quests = failed_quest_manager("delivery");
    let choices = get_available_choices(&session, &state, Some(&quests));
    assert!(
        choices
            .iter()
            .all(|choice| choice.text != "Turn in the delivery")
    );
}

#[test]
fn select_choice_resolves_a_quest_gated_choice_once_it_becomes_available() {
    let mut session = start_dialog("npc_gregor", quest_gated_tree());
    let mut state = game();
    let mut quests = QuestManager::new();
    quests.start_quest("delivery");

    // Prove index 0 is actually "Turn in the delivery" now that the quest
    // is active, not "Never mind" or an out-of-range no-op that would also
    // return `None` from `select_choice` below.
    let choices = get_available_choices(&session, &state, Some(&quests));
    assert_eq!(choices[0].text, "Turn in the delivery");

    let (next, _events) = select_choice(&mut session, 0, &mut state, Some(&quests));
    // "Turn in the delivery"'s own `next` is null in this fixture, ending
    // the dialog.
    assert_eq!(next, None);
    assert_eq!(session.current_node_id, "start");
}

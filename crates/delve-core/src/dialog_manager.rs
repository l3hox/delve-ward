//! Dialog runtime, ported from the TS `dialogManager`: evaluates node/choice
//! conditions against `GameState`, applies effects, and walks a
//! `DialogTree` session node by node.
//!
//! The TS module keeps module-level mutable singletons (a dialog fetch
//! cache, `currentNpcId`, and callback hooks for `startQuest` /
//! `advanceQuest` / `openShop`). None of that survives the port: delve-core
//! forbids unsafe code and must stay usable from parallel tests, so global
//! mutable state isn't an option. Loading dialog JSON is an I/O concern that
//! belongs to delve-game (mirroring how `data/npcs.json` etc. are read
//! there), and the three quest/shop hooks become returned `DialogEvent`s —
//! the caller applies them instead of delve-core holding stored callbacks.
//! `currentNpcId` is replaced by `DialogSession::npc_id`, which is already
//! in scope everywhere the TS code reached for the global.

use crate::dialogs::{DialogChoice, DialogCondition, DialogConditionType, DialogEffectType};
use crate::dialogs::{DialogEffect, DialogNode, DialogTree};
use crate::entities::ItemLocation;
use crate::game_state::GameState;
use crate::items::ItemQuality;

/// A side effect that reaches beyond `GameState` — the TS `onStartQuest` /
/// `onAdvanceQuest` / `onOpenShop` hooks. The caller (eventually a quest
/// manager) decides how to act on these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogEvent {
    StartQuest(String),
    AdvanceQuest(String),
    OpenShop(String),
}

// --- Condition evaluation ---

#[must_use]
pub fn evaluate_condition(condition: &DialogCondition, game: &GameState) -> bool {
    match condition.condition_type {
        DialogConditionType::HasFlag => condition
            .flag
            .as_deref()
            .is_some_and(|flag| game.has_flag(flag)),
        DialogConditionType::HasItem => condition
            .item_id
            .as_deref()
            .is_some_and(|item_id| player_has_item(game, item_id)),
        // Quest progress isn't tracked in `GameState` yet, so this mirrors
        // the TS default evaluator's placeholder: quests read as
        // "undiscovered" until a quest manager installs a real evaluator.
        DialogConditionType::QuestStage => condition.stage.as_deref() == Some("undiscovered"),
        DialogConditionType::StatCheck => stat_check(condition, game),
    }
}

fn player_has_item(game: &GameState, item_id: &str) -> bool {
    game.entity_registry.snapshot().iter().any(|item| {
        item.item_id == item_id
            && matches!(
                item.location,
                ItemLocation::Backpack { .. } | ItemLocation::Equipped { .. }
            )
    })
}

/// Reads the same numeric player fields the TS `GameState` getters expose
/// (`gs[c.stat]`), since Rust has no equivalent of indexing an object by an
/// arbitrary string property.
fn stat_check(condition: &DialogCondition, game: &GameState) -> bool {
    let (Some(stat), Some(min)) = (condition.stat.as_deref(), condition.min) else {
        return false;
    };
    let player = &game.player;
    let value = match stat {
        "hp" => player.hp,
        "maxHp" => player.max_hp,
        "atk" => player.atk,
        "def" => player.def,
        "str" => player.str,
        "dex" => player.dex,
        "vit" => player.vit,
        "wis" => player.wis,
        "xp" => player.xp as f64,
        "level" => player.level as f64,
        "gold" => player.gold as f64,
        "attackCooldown" => player.attack_cooldown,
        "attributePoints" => player.attribute_points as f64,
        _ => return false,
    };
    value >= min
}

#[must_use]
pub fn evaluate_conditions(conditions: Option<&[DialogCondition]>, game: &GameState) -> bool {
    match conditions {
        None => true,
        Some(conditions) => conditions
            .iter()
            .all(|condition| evaluate_condition(condition, game)),
    }
}

// --- Effect execution ---

pub fn execute_effect(
    effect: &DialogEffect,
    game: &mut GameState,
    npc_id: &str,
) -> Option<DialogEvent> {
    match effect.effect_type {
        DialogEffectType::SetFlag => {
            if let Some(flag) = &effect.flag {
                game.set_flag(flag);
            }
            None
        }
        DialogEffectType::GiveItem => {
            give_item(effect, game);
            None
        }
        DialogEffectType::TakeItem => {
            take_item(effect, game);
            None
        }
        DialogEffectType::StartQuest => effect.quest_id.clone().map(DialogEvent::StartQuest),
        DialogEffectType::AdvanceQuest => effect.quest_id.clone().map(DialogEvent::AdvanceQuest),
        DialogEffectType::OpenShop => Some(DialogEvent::OpenShop(npc_id.to_string())),
    }
}

fn give_item(effect: &DialogEffect, game: &mut GameState) {
    let Some(item_id) = &effect.item_id else {
        return;
    };
    // Backpack full — item silently not given, matching the TS comment.
    let Some(slot) = game.entity_registry.next_backpack_slot() else {
        return;
    };
    game.entity_registry.create_item(
        item_id,
        ItemQuality::Common,
        ItemLocation::Backpack { slot },
        Vec::new(),
    );
}

fn take_item(effect: &DialogEffect, game: &mut GameState) {
    let Some(item_id) = &effect.item_id else {
        return;
    };
    let found = game.entity_registry.snapshot().into_iter().find(|item| {
        item.item_id == *item_id
            && matches!(
                item.location,
                ItemLocation::Backpack { .. } | ItemLocation::Equipped { .. }
            )
    });
    if let Some(item) = found {
        game.entity_registry.remove_item(&item.instance_id);
    }
}

pub fn execute_effects(
    effects: Option<&[DialogEffect]>,
    game: &mut GameState,
    npc_id: &str,
) -> Vec<DialogEvent> {
    let Some(effects) = effects else {
        return Vec::new();
    };
    effects
        .iter()
        .filter_map(|effect| execute_effect(effect, game, npc_id))
        .collect()
}

// --- Dialog session state ---

#[derive(Debug, Clone)]
pub struct DialogSession {
    pub npc_id: String,
    pub tree: DialogTree,
    pub current_node_id: String,
}

#[must_use]
pub fn start_dialog(npc_id: &str, tree: DialogTree) -> DialogSession {
    let current_node_id = tree.start_node.clone();
    DialogSession {
        npc_id: npc_id.to_string(),
        tree,
        current_node_id,
    }
}

#[must_use]
pub fn get_current_node(session: &DialogSession) -> Option<&DialogNode> {
    session.tree.nodes.get(&session.current_node_id)
}

#[must_use]
pub fn get_available_choices<'session>(
    session: &'session DialogSession,
    game: &GameState,
) -> Vec<&'session DialogChoice> {
    let Some(node) = get_current_node(session) else {
        return Vec::new();
    };
    let Some(choices) = &node.choices else {
        return Vec::new();
    };
    choices
        .iter()
        .filter(|choice| evaluate_conditions(choice.conditions.as_deref(), game))
        .collect()
}

/// Apply the chosen option's effects, move the session to its `next` node,
/// and run that node's entry effects. Returns the new node id, or `None`
/// when the choice ends the dialog (`next` is `null`/absent) or the index
/// is out of range for the currently available choices.
pub fn select_choice(
    session: &mut DialogSession,
    choice_index: usize,
    game: &mut GameState,
) -> Option<String> {
    let (next, effects) = {
        let choices = get_available_choices(session, game);
        let choice = choices.get(choice_index)?;
        (choice.next.clone(), choice.effects.clone())
    };

    execute_effects(effects.as_deref(), game, &session.npc_id);

    let next = next?;
    session.current_node_id = next.clone();
    if let Some(node) = get_current_node(session) {
        execute_effects(node.effects.as_deref(), game, &session.npc_id);
    }
    Some(next)
}

/// Linear advance along the current node's `next` field, running the new
/// node's entry effects. Returns `None` when the current node is missing
/// from the tree, or `next` is `null`/absent (end of dialog).
pub fn advance_dialog(session: &mut DialogSession, game: &mut GameState) -> Option<String> {
    let next = get_current_node(session)?.next.clone()?;

    session.current_node_id = next.clone();
    if let Some(node) = get_current_node(session) {
        execute_effects(node.effects.as_deref(), game, &session.npc_id);
    }
    Some(next)
}

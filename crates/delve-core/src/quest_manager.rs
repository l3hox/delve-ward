//! Quest runtime state, ported from the TS `questManager`: tracks per-quest
//! status and stage progress, and applies stage rewards to `GameState`.
//!
//! `loadQuest`'s `fetch()` call is browser I/O and stays out of delve-core,
//! the same way dialog-tree loading stays out of `dialog_manager`;
//! [`QuestManager::register_quest_def`] replaces it — the caller parses
//! `QuestDef::from_json` and hands the result in directly rather than the
//! manager fetching-and-caching by id.
//!
//! The module-level `questManager` singleton doesn't survive either (no
//! global mutable state, matching every other manager in this crate). Nor
//! does `installConditionEvaluator`'s side effect of registering a closure
//! into `dialogManager`'s mutable evaluator table — `dialog_manager`'s own
//! doc comment rules that pattern out for the same reason.
//! [`QuestManager::evaluate_quest_stage_condition`] exposes the same
//! decision table as a plain method instead. Wiring it into
//! `dialog_manager::evaluate_condition`'s `QuestStage` arm (currently a
//! hardcoded "always undiscovered" placeholder — quest state isn't tracked
//! there yet) needs that module to grow a way to consult quest state, which
//! is out of scope for this port.

use crate::entities::ItemLocation;
use crate::game_state::GameState;
use crate::items::ItemQuality;
use crate::quests::{QuestDef, QuestRewards};
use crate::save_system::QuestSaveState;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestStatus {
    Undiscovered,
    Active,
    Complete,
    Failed,
}

/// Status values a tracked quest can hold. Unlike [`QuestStatus`], there is
/// no `Undiscovered` variant — an untracked quest simply has no entry in
/// [`QuestManager`]'s runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuestRuntimeStatus {
    Active,
    Complete,
    Failed,
}

impl QuestRuntimeStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "complete" => Some(Self::Complete),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl From<QuestRuntimeStatus> for QuestStatus {
    fn from(status: QuestRuntimeStatus) -> Self {
        match status {
            QuestRuntimeStatus::Active => Self::Active,
            QuestRuntimeStatus::Complete => Self::Complete,
            QuestRuntimeStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QuestRuntimeState {
    status: QuestRuntimeStatus,
    stage_index: i64,
}

#[derive(Debug, Default)]
pub struct QuestManager {
    defs: HashMap<String, QuestDef>,
    state: HashMap<String, QuestRuntimeState>,
}

impl QuestManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a parsed quest definition, replacing any previous
    /// definition sharing its id. Stands in for the TS `loadQuest`'s
    /// fetch-and-cache; the fetch itself belongs to delve-game.
    pub fn register_quest_def(&mut self, def: QuestDef) {
        self.defs.insert(def.id.clone(), def);
    }

    #[must_use]
    pub fn get_quest_def(&self, quest_id: &str) -> Option<&QuestDef> {
        self.defs.get(quest_id)
    }

    #[must_use]
    pub fn get_status(&self, quest_id: &str) -> QuestStatus {
        self.state
            .get(quest_id)
            .map_or(QuestStatus::Undiscovered, |runtime| runtime.status.into())
    }

    #[must_use]
    pub fn get_stage_index(&self, quest_id: &str) -> i64 {
        self.state
            .get(quest_id)
            .map_or(-1, |runtime| runtime.stage_index)
    }

    /// Starts tracking `quest_id` at stage 0. A no-op if the quest is
    /// already tracked (started, completed, or failed).
    pub fn start_quest(&mut self, quest_id: &str) {
        if self.state.contains_key(quest_id) {
            return;
        }
        self.state.insert(
            quest_id.to_string(),
            QuestRuntimeState {
                status: QuestRuntimeStatus::Active,
                stage_index: 0,
            },
        );
    }

    /// Applies the current stage's rewards, then advances to the next
    /// stage, marking the quest complete once past the last stage. A no-op
    /// when the quest isn't active or its definition hasn't been
    /// registered.
    pub fn advance_quest(&mut self, quest_id: &str, game_state: &mut GameState) {
        let Some(runtime) = self.state.get(quest_id) else {
            return;
        };
        if runtime.status != QuestRuntimeStatus::Active {
            return;
        }
        let stage_index = runtime.stage_index;

        let Some(def) = self.defs.get(quest_id) else {
            return;
        };

        let current_rewards = def
            .stages
            .get(stage_index as usize)
            .and_then(|stage| stage.rewards.as_ref());
        if let Some(rewards) = current_rewards {
            apply_rewards(rewards, game_state);
        }
        let stage_count = def.stages.len();

        // Both branches above already confirmed `quest_id` is tracked.
        if let Some(runtime) = self.state.get_mut(quest_id) {
            runtime.stage_index += 1;
            if runtime.stage_index as usize >= stage_count {
                runtime.status = QuestRuntimeStatus::Complete;
            }
        }
    }

    #[must_use]
    pub fn get_active_quests(&self) -> Vec<String> {
        self.state
            .iter()
            .filter(|(_, runtime)| runtime.status == QuestRuntimeStatus::Active)
            .map(|(id, _)| id.clone())
            .collect()
    }

    #[must_use]
    pub fn get_completed_quests(&self) -> Vec<String> {
        self.state
            .iter()
            .filter(|(_, runtime)| runtime.status == QuestRuntimeStatus::Complete)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Snapshot of all tracked quests, in the exact shape `save_system`
    /// expects for `SaveData.quests`.
    #[must_use]
    pub fn get_serializable_state(&self) -> HashMap<String, QuestSaveState> {
        self.state
            .iter()
            .map(|(id, runtime)| {
                (
                    id.clone(),
                    QuestSaveState {
                        status: runtime.status.as_str().to_string(),
                        stage_index: runtime.stage_index,
                    },
                )
            })
            .collect()
    }

    /// Replaces all tracked quest state from a `save_system::QuestSaveState`
    /// snapshot. Errors without mutating any state if an entry's `status`
    /// isn't one of `"active"`, `"complete"`, or `"failed"`.
    pub fn restore_state(&mut self, data: HashMap<String, QuestSaveState>) -> Result<(), String> {
        let mut restored = HashMap::with_capacity(data.len());
        for (quest_id, entry) in data {
            let status = QuestRuntimeStatus::parse(&entry.status)
                .ok_or_else(|| format!("Unknown quest status: '{}'", entry.status))?;
            restored.insert(
                quest_id,
                QuestRuntimeState {
                    status,
                    stage_index: entry.stage_index,
                },
            );
        }
        self.state = restored;
        Ok(())
    }

    /// The TS `installConditionEvaluator` closure's decision table, exposed
    /// directly rather than installed into a mutable registry (see the
    /// module doc comment).
    #[must_use]
    pub fn evaluate_quest_stage_condition(&self, quest_id: Option<&str>, stage: &str) -> bool {
        let Some(quest_id) = quest_id.filter(|id| !id.is_empty()) else {
            return false;
        };
        let status = self.get_status(quest_id);
        match stage {
            "undiscovered" => status == QuestStatus::Undiscovered,
            "active" => status == QuestStatus::Active,
            "complete" => status == QuestStatus::Complete,
            "failed" => status == QuestStatus::Failed,
            _ => false,
        }
    }
}

fn apply_rewards(rewards: &QuestRewards, game_state: &mut GameState) {
    if let Some(xp) = rewards.xp {
        game_state.add_xp(xp);
    }

    if let Some(gold) = rewards.gold {
        game_state.player.gold += gold;
    }

    if let Some(items) = &rewards.items {
        for item_id in items {
            let Some(slot) = game_state.entity_registry.next_backpack_slot() else {
                continue; // backpack full
            };
            game_state.entity_registry.create_item(
                item_id,
                ItemQuality::Common,
                ItemLocation::Backpack { slot },
                Vec::new(),
            );
        }
    }

    if let Some(flags) = &rewards.flags {
        for flag in flags {
            game_state.set_flag(flag);
        }
    }
}

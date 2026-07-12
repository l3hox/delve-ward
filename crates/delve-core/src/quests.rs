//! Quest definitions — typed model of `data/quests/*.json`.
//! Runtime quest state (status, stage advancement, rewards) arrives in phase 4.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QuestRewards {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestStage {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewards: Option<QuestRewards>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub stages: Vec<QuestStage>,
}

impl QuestDef {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| format!("Failed to load quest: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FETCH_AMULET_JSON: &str = include_str!("../../../assets/data/quests/fetch_amulet.json");

    #[test]
    fn fetch_amulet_quest_parses_with_stage_rewards() {
        let quest = QuestDef::from_json(FETCH_AMULET_JSON).expect("shipped quest parses");
        assert_eq!(quest.id, "fetch_amulet");
        assert_eq!(quest.name, "The Lost Amulet");
        assert!(!quest.stages.is_empty());
        let rewards = quest.stages[0]
            .rewards
            .as_ref()
            .expect("stage 0 has rewards");
        assert_eq!(rewards.xp, Some(50));
        assert_eq!(rewards.gold, Some(75));
    }

    #[test]
    fn load_failure_reports_error() {
        let error = QuestDef::from_json("[]").expect_err("wrong shape fails");
        assert!(error.contains("Failed to load quest"));
    }
}

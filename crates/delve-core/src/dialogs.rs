//! Dialog tree data — typed model of `data/dialogs/*.json`.
//! Condition evaluation, effects, and session state arrive in phase 4.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogConditionType {
    HasFlag,
    HasItem,
    QuestStage,
    StatCheck,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogCondition {
    #[serde(rename = "type")]
    pub condition_type: DialogConditionType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogEffectType {
    SetFlag,
    GiveItem,
    TakeItem,
    StartQuest,
    AdvanceQuest,
    OpenShop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogEffect {
    #[serde(rename = "type")]
    pub effect_type: DialogEffectType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DialogChoice {
    pub text: String,
    /// Next node ID; `None` (JSON `null`) ends the dialog.
    pub next: Option<String>,
    /// All must be true for the choice to appear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<DialogCondition>>,
    /// Executed when the choice is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<DialogEffect>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DialogNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    /// If present, shown as selectable options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<DialogChoice>>,
    /// Linear advance (used when no choices); `None` ends the dialog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// Effects applied when the node is displayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<DialogEffect>>,
    /// If present, all must be true to show this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<DialogCondition>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogTree {
    pub start_node: String,
    pub nodes: HashMap<String, DialogNode>,
}

impl DialogTree {
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| format!("Failed to load dialog: {error}"))
    }
}

/// Editor-only node positions stored in `*.layout.json` companions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DialogNodePosition {
    pub x: f64,
    pub y: f64,
}

pub type DialogLayout = HashMap<String, DialogNodePosition>;

pub fn dialog_layout_from_json(json: &str) -> Result<DialogLayout, String> {
    serde_json::from_str(json).map_err(|error| format!("Failed to load dialog layout: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREGOR_JSON: &str = include_str!("../../../assets/data/dialogs/merchant_gregor.json");
    const GREGOR_LAYOUT_JSON: &str =
        include_str!("../../../assets/data/dialogs/merchant_gregor.layout.json");

    #[test]
    fn merchant_gregor_dialog_parses_and_start_node_exists() {
        let tree = DialogTree::from_json(GREGOR_JSON).expect("shipped dialog parses");
        assert!(
            tree.nodes.contains_key(&tree.start_node),
            "startNode {:?} missing from nodes",
            tree.start_node
        );
        let start = &tree.nodes[&tree.start_node];
        assert!(!start.text.is_empty());
    }

    #[test]
    fn merchant_gregor_layout_parses() {
        let layout = dialog_layout_from_json(GREGOR_LAYOUT_JSON).expect("shipped layout parses");
        let greeting = layout.get("greeting").expect("greeting position present");
        assert_eq!(greeting.x, 100.0);
        assert_eq!(greeting.y, 100.0);
    }

    #[test]
    fn load_failure_reports_error() {
        let error = DialogTree::from_json("42").expect_err("wrong shape fails");
        assert!(error.contains("Failed to load dialog"));
    }
}

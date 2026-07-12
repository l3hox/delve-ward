//! NPC database — typed model of `data/npcs.json` and query methods.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_NPC_SPRITE_SIZE: f64 = 2.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcSpriteData {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_offset: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NpcDef {
    pub id: String,
    pub name: String,
    pub sprite: NpcSpriteData,
    /// Dialog file id (loads from `data/dialogs/{dialog}.json`).
    pub dialog: String,
    /// Item IDs for merchants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock: Option<Vec<String>>,
    /// Buy price multiplier (default 1.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markup: Option<f64>,
    /// Default facing direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing: Option<String>,
}

#[derive(Deserialize)]
struct NpcsJsonPayload {
    #[allow(dead_code)]
    version: String,
    npcs: Vec<NpcDef>,
}

#[derive(Debug)]
pub struct NpcDatabase {
    npcs: Vec<NpcDef>,
    index: HashMap<String, usize>,
}

impl NpcDatabase {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let payload: NpcsJsonPayload = serde_json::from_str(json)
            .map_err(|error| format!("Failed to load NPC database: {error}"))?;
        let mut npcs: Vec<NpcDef> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for npc in payload.npcs {
            if let Some(&position) = index.get(&npc.id) {
                npcs[position] = npc;
            } else {
                index.insert(npc.id.clone(), npcs.len());
                npcs.push(npc);
            }
        }
        Ok(Self { npcs, index })
    }

    #[must_use]
    pub fn get_npc(&self, id: &str) -> Option<&NpcDef> {
        self.index.get(id).map(|&position| &self.npcs[position])
    }

    #[must_use]
    pub fn all_npcs(&self) -> &[NpcDef] {
        &self.npcs
    }

    #[must_use]
    pub fn all_npc_ids(&self) -> HashSet<String> {
        self.npcs.iter().map(|npc| npc.id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPCS_JSON: &str = include_str!("../../../assets/data/npcs.json");

    fn database() -> NpcDatabase {
        NpcDatabase::from_json(NPCS_JSON).expect("shipped npcs.json parses")
    }

    #[test]
    fn merchant_gregor_has_expected_fields() {
        let db = database();
        let gregor = db
            .get_npc("merchant_gregor")
            .expect("merchant_gregor exists");
        assert_eq!(gregor.name, "Gregor the Merchant");
        assert_eq!(gregor.dialog, "merchant_gregor");
        assert_eq!(gregor.markup, Some(1.5));
        assert!(
            gregor
                .stock
                .as_ref()
                .is_some_and(|stock| stock.contains(&"health_potion_small".to_string()))
        );
    }

    #[test]
    fn unknown_npc_returns_none() {
        assert!(database().get_npc("nobody").is_none());
    }

    #[test]
    fn load_failure_reports_error() {
        let error = NpcDatabase::from_json("{}").expect_err("wrong shape fails");
        assert!(error.contains("Failed to load NPC database"));
    }
}

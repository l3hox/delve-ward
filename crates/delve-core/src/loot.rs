//! Loot table data — typed model of `data/loot-tables.json` and lookups.
//! Rolling (quality draws, gold, drops) arrives with the phase 2 loot game.

use crate::items::ItemQuality;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const ENCHANTED_MODIFIERS: [&str; 8] = [
    "fire_damage",
    "life_steal",
    "bonus_str",
    "bonus_dex",
    "hp_regen",
    "crit_bonus",
    "def_boost",
    "torch_range",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LootTableDrop {
    pub item_id: String,
    pub chance: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<ItemQuality>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LootTableEntry {
    pub xp: i64,
    /// Inclusive `[min, max]` gold range.
    pub gold: [i64; 2],
    pub drops: Vec<LootTableDrop>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuaranteedDrop {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<ItemQuality>,
}

/// Per-enemy-instance override of the base loot table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropsOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guaranteed: Option<Vec<GuaranteedDrop>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Vec<LootTableDrop>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppress_table: Option<bool>,
}

/// Quality tier weights in draw order (poor first, enchanted last).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityWeights {
    pub poor: f64,
    pub common: f64,
    pub fine: f64,
    pub masterwork: f64,
    pub enchanted: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LootTablesJsonPayload {
    #[allow(dead_code)]
    version: String,
    quality_weights: QualityWeights,
    enemies: HashMap<String, LootTableEntry>,
}

#[derive(Debug)]
pub struct LootTables {
    pub quality_weights: QualityWeights,
    enemies: HashMap<String, LootTableEntry>,
}

impl LootTables {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let payload: LootTablesJsonPayload = serde_json::from_str(json)
            .map_err(|error| format!("Failed to load loot tables: {error}"))?;
        Ok(Self {
            quality_weights: payload.quality_weights,
            enemies: payload.enemies,
        })
    }

    /// The loot table entry for a given enemy type.
    #[must_use]
    pub fn get(&self, enemy_type: &str) -> Option<&LootTableEntry> {
        self.enemies.get(enemy_type)
    }

    /// The XP value for a given enemy type, or 0 if not found.
    #[must_use]
    pub fn xp(&self, enemy_type: &str) -> i64 {
        self.enemies.get(enemy_type).map_or(0, |entry| entry.xp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOT_TABLES_JSON: &str = include_str!("../../../assets/data/loot-tables.json");

    fn tables() -> LootTables {
        LootTables::from_json(LOOT_TABLES_JSON).expect("shipped loot-tables.json parses")
    }

    #[test]
    fn quality_weights_match_shipped_data() {
        let tables = tables();
        assert_eq!(tables.quality_weights.poor, 0.10);
        assert_eq!(tables.quality_weights.common, 0.50);
        assert_eq!(tables.quality_weights.fine, 0.25);
        assert_eq!(tables.quality_weights.masterwork, 0.12);
        assert_eq!(tables.quality_weights.enchanted, 0.03);
    }

    #[test]
    fn goblin_entry_has_expected_gold_range_and_drops() {
        let tables = tables();
        let goblin = tables.get("goblin").expect("goblin table exists");
        assert_eq!(goblin.xp, 12);
        assert_eq!(goblin.gold, [2, 5]);
        assert!(goblin.drops.iter().any(|drop| drop.item_id == "bone"));
        let forced_quality = goblin
            .drops
            .iter()
            .find(|drop| drop.item_id == "dagger_iron")
            .and_then(|drop| drop.quality);
        assert_eq!(forced_quality, Some(ItemQuality::Poor));
    }

    #[test]
    fn unknown_enemy_type_has_no_table_and_zero_xp() {
        let tables = tables();
        assert!(tables.get("unknown_enemy").is_none());
        assert_eq!(tables.xp("unknown_enemy"), 0);
    }

    #[test]
    fn load_failure_reports_error() {
        let error = LootTables::from_json("null").expect_err("wrong shape fails");
        assert!(error.contains("Failed to load loot tables"));
    }
}

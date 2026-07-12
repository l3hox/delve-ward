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

impl QualityWeights {
    /// Tiers in draw order (matches the JSON key order the TS roll iterates).
    #[must_use]
    pub fn tiers(&self) -> [(ItemQuality, f64); 5] {
        [
            (ItemQuality::Poor, self.poor),
            (ItemQuality::Common, self.common),
            (ItemQuality::Fine, self.fine),
            (ItemQuality::Masterwork, self.masterwork),
            (ItemQuality::Enchanted, self.enchanted),
        ]
    }
}

/// Weighted random quality draw. Iterates through tiers accumulating weight
/// until the random threshold is crossed. `random` yields floats in [0, 1).
pub fn roll_quality(weights: &QualityWeights, random: &mut dyn FnMut() -> f64) -> ItemQuality {
    let tiers = weights.tiers();
    let total: f64 = tiers.iter().map(|(_, weight)| weight).sum();
    let mut threshold = random() * total;
    for (tier, weight) in tiers {
        threshold -= weight;
        if threshold < 0.0 {
            return tier;
        }
    }
    // Floating-point edge where the threshold lands exactly on the total.
    tiers[tiers.len() - 1].0
}

/// Random integer in `[min, max]` inclusive.
pub fn roll_gold(min: i64, max: i64, random: &mut dyn FnMut() -> f64) -> i64 {
    (random() * (max - min + 1) as f64).floor() as i64 + min
}

fn pick_enchanted_modifier(random: &mut dyn FnMut() -> f64) -> String {
    let index = (random() * ENCHANTED_MODIFIERS.len() as f64).floor() as usize;
    ENCHANTED_MODIFIERS[index.min(ENCHANTED_MODIFIERS.len() - 1)].to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub struct LootRollItem {
    pub item_id: String,
    pub quality: ItemQuality,
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LootRollResult {
    pub gold: i64,
    pub items: Vec<LootRollItem>,
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

    /// Full loot roll for a given enemy type. Returns gold 0 and no items for
    /// an unknown enemy type (guaranteed/extra overrides still apply).
    pub fn roll_loot(
        &self,
        enemy_type: &str,
        drops_override: Option<&DropsOverride>,
        random: &mut dyn FnMut() -> f64,
    ) -> LootRollResult {
        let entry = self.enemies.get(enemy_type);
        let gold = entry.map_or(0, |entry| roll_gold(entry.gold[0], entry.gold[1], random));
        let mut items = Vec::new();

        let mut push_item = |item_id: &str, quality: Option<ItemQuality>, random: &mut dyn FnMut() -> f64| {
            let quality = quality.unwrap_or_else(|| roll_quality(&self.quality_weights, random));
            let modifiers = if quality == ItemQuality::Enchanted {
                vec![pick_enchanted_modifier(random)]
            } else {
                Vec::new()
            };
            LootRollItem {
                item_id: item_id.to_string(),
                quality,
                modifiers,
            }
        };

        // 1. Guaranteed items from the override — always added.
        if let Some(guaranteed) = drops_override.and_then(|o| o.guaranteed.as_ref()) {
            for drop in guaranteed {
                items.push(push_item(&drop.item_id, drop.quality, random));
            }
        }

        // 2. Base table drops — skipped when suppressTable is set or no table exists.
        let suppress_table =
            drops_override.is_some_and(|o| o.suppress_table.unwrap_or(false));
        if let Some(entry) = entry {
            if !suppress_table {
                for drop in &entry.drops {
                    if random() < drop.chance {
                        items.push(push_item(&drop.item_id, drop.quality, random));
                    }
                }
            }
        }

        // 3. Extra drops from the override — rolled independently like base drops.
        if let Some(extra) = drops_override.and_then(|o| o.extra.as_ref()) {
            for drop in extra {
                if random() < drop.chance {
                    items.push(push_item(&drop.item_id, drop.quality, random));
                }
            }
        }

        LootRollResult { gold, items }
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

    // --- roll tests, ported from lootTable.test.ts against the mock tables ---

    use crate::random::Mulberry32;

    const MOCK_TABLES_JSON: &str = include_str!("../tests/fixtures/loot-tables-mock.json");

    fn mock_tables() -> LootTables {
        LootTables::from_json(MOCK_TABLES_JSON).expect("mock tables parse")
    }

    fn seeded_random(seed: u32) -> impl FnMut() -> f64 {
        let mut rng = Mulberry32::new(seed);
        move || rng.next_f64()
    }

    #[test]
    fn roll_quality_only_returns_valid_tiers_and_matches_distribution() {
        let tables = mock_tables();
        let mut random = seeded_random(1);
        let mut counts = std::collections::HashMap::new();
        let rolls = 10_000;
        for _ in 0..rolls {
            let tier = roll_quality(&tables.quality_weights, &mut random);
            *counts.entry(tier).or_insert(0u32) += 1;
        }
        for (tier, weight) in tables.quality_weights.tiers() {
            let frequency =
                f64::from(counts.get(&tier).copied().unwrap_or(0)) / f64::from(rolls);
            assert!(
                (frequency - weight).abs() < 0.03,
                "{tier:?} frequency {frequency} too far from weight {weight}"
            );
        }
    }

    #[test]
    fn roll_quality_boundary_values() {
        let tables = mock_tables();
        assert_eq!(
            roll_quality(&tables.quality_weights, &mut || 0.0),
            ItemQuality::Poor
        );
        assert_eq!(
            roll_quality(&tables.quality_weights, &mut || 0.999_999),
            ItemQuality::Enchanted
        );
    }

    #[test]
    fn roll_gold_stays_in_range_and_covers_all_values() {
        let mut random = seeded_random(2);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let gold = roll_gold(1, 3, &mut random);
            assert!((1..=3).contains(&gold));
            seen.insert(gold);
        }
        assert_eq!(seen, [1, 2, 3].into());
    }

    #[test]
    fn roll_gold_boundary_values() {
        assert_eq!(roll_gold(5, 5, &mut || 0.5), 5);
        assert_eq!(roll_gold(1, 3, &mut || 0.0), 1);
        assert_eq!(roll_gold(1, 3, &mut || 0.999_999), 3);
    }

    #[test]
    fn roll_loot_unknown_enemy_returns_nothing() {
        let tables = mock_tables();
        let result = tables.roll_loot("dragon_king", None, &mut seeded_random(3));
        assert_eq!(
            result,
            LootRollResult {
                gold: 0,
                items: Vec::new()
            }
        );
    }

    #[test]
    fn roll_loot_rat_gold_stays_in_range() {
        let tables = mock_tables();
        let mut random = seeded_random(4);
        for _ in 0..100 {
            let result = tables.roll_loot("rat", None, &mut random);
            assert!((1..=3).contains(&result.gold));
        }
    }

    #[test]
    fn roll_loot_items_come_from_the_table() {
        let tables = mock_tables();
        let result = tables.roll_loot("rat", None, &mut || 0.0);
        assert!(!result.items.is_empty());
        let known = ["bone", "health_potion_small", "torch_oil"];
        for item in &result.items {
            assert!(known.contains(&item.item_id.as_str()));
        }
    }

    #[test]
    fn roll_loot_high_random_suppresses_all_drops() {
        let tables = mock_tables();
        let result = tables.roll_loot("rat", None, &mut || 0.99);
        assert!(result.items.is_empty());
    }

    #[test]
    fn guaranteed_items_always_appear() {
        let tables = mock_tables();
        let over = DropsOverride {
            guaranteed: Some(vec![GuaranteedDrop {
                item_id: "sword_iron".to_string(),
                quality: None,
            }]),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.99);
        assert!(result.items.iter().any(|item| item.item_id == "sword_iron"));
    }

    #[test]
    fn guaranteed_item_uses_forced_quality() {
        let tables = mock_tables();
        let over = DropsOverride {
            guaranteed: Some(vec![GuaranteedDrop {
                item_id: "sword_iron".to_string(),
                quality: Some(ItemQuality::Masterwork),
            }]),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.99);
        let item = result
            .items
            .iter()
            .find(|item| item.item_id == "sword_iron")
            .expect("guaranteed item present");
        assert_eq!(item.quality, ItemQuality::Masterwork);
    }

    #[test]
    fn guaranteed_item_rolls_quality_when_not_forced() {
        let tables = mock_tables();
        let over = DropsOverride {
            guaranteed: Some(vec![GuaranteedDrop {
                item_id: "sword_iron".to_string(),
                quality: None,
            }]),
            ..DropsOverride::default()
        };
        // 0.61 lands in the fine band (poor+common = 0.60 .. 0.85).
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.61);
        let item = result
            .items
            .iter()
            .find(|item| item.item_id == "sword_iron")
            .expect("guaranteed item present");
        assert_eq!(item.quality, ItemQuality::Fine);
    }

    #[test]
    fn multiple_guaranteed_items_all_appear() {
        let tables = mock_tables();
        let over = DropsOverride {
            guaranteed: Some(vec![
                GuaranteedDrop {
                    item_id: "sword_iron".to_string(),
                    quality: Some(ItemQuality::Common),
                },
                GuaranteedDrop {
                    item_id: "bone".to_string(),
                    quality: Some(ItemQuality::Poor),
                },
            ]),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.99);
        assert!(result.items.len() >= 2);
        assert!(result.items.iter().any(|item| item.item_id == "sword_iron"));
        assert!(result.items.iter().any(|item| item.item_id == "bone"));
    }

    #[test]
    fn suppress_table_keeps_only_guaranteed() {
        let tables = mock_tables();
        let over = DropsOverride {
            guaranteed: Some(vec![GuaranteedDrop {
                item_id: "sword_iron".to_string(),
                quality: Some(ItemQuality::Common),
            }]),
            suppress_table: Some(true),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.0);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].item_id, "sword_iron");
    }

    #[test]
    fn suppress_table_with_no_guaranteed_yields_empty() {
        let tables = mock_tables();
        let over = DropsOverride {
            suppress_table: Some(true),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn extra_drops_roll_alongside_base_table() {
        let tables = mock_tables();
        let over = DropsOverride {
            extra: Some(vec![LootTableDrop {
                item_id: "ring_of_power".to_string(),
                chance: 0.99,
                quality: None,
            }]),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.0);
        assert!(result.items.iter().any(|item| item.item_id == "ring_of_power"));
        assert!(result.items.iter().any(|item| item.item_id == "bone"));
    }

    #[test]
    fn extra_drops_suppressed_when_chance_roll_fails() {
        let tables = mock_tables();
        let over = DropsOverride {
            extra: Some(vec![LootTableDrop {
                item_id: "ring_of_power".to_string(),
                chance: 0.01,
                quality: None,
            }]),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.99);
        assert!(!result.items.iter().any(|item| item.item_id == "ring_of_power"));
    }

    #[test]
    fn extra_drops_appear_with_suppress_table() {
        let tables = mock_tables();
        let over = DropsOverride {
            suppress_table: Some(true),
            extra: Some(vec![LootTableDrop {
                item_id: "amulet_of_fortitude".to_string(),
                chance: 0.50,
                quality: None,
            }]),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.0);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].item_id, "amulet_of_fortitude");
    }

    #[test]
    fn enchanted_item_receives_exactly_one_known_modifier() {
        let tables = mock_tables();
        let over = DropsOverride {
            guaranteed: Some(vec![GuaranteedDrop {
                item_id: "sword_iron".to_string(),
                quality: None,
            }]),
            suppress_table: Some(true),
            ..DropsOverride::default()
        };
        // 0.98 lands past the cumulative 0.97 → enchanted.
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.98);
        let item = result
            .items
            .iter()
            .find(|item| item.item_id == "sword_iron")
            .expect("guaranteed item present");
        assert_eq!(item.quality, ItemQuality::Enchanted);
        assert_eq!(item.modifiers.len(), 1);
        assert!(ENCHANTED_MODIFIERS.contains(&item.modifiers[0].as_str()));
    }

    #[test]
    fn non_enchanted_items_have_no_modifiers() {
        let tables = mock_tables();
        let over = DropsOverride {
            guaranteed: Some(vec![GuaranteedDrop {
                item_id: "sword_iron".to_string(),
                quality: Some(ItemQuality::Common),
            }]),
            suppress_table: Some(true),
            ..DropsOverride::default()
        };
        let result = tables.roll_loot("rat", Some(&over), &mut || 0.99);
        assert_eq!(result.items[0].quality, ItemQuality::Common);
        assert!(result.items[0].modifiers.is_empty());
    }

    #[test]
    fn base_table_drop_with_forced_quality_has_no_modifier() {
        let tables = mock_tables();
        let result = tables.roll_loot("goblin", None, &mut || 0.0);
        let dagger = result
            .items
            .iter()
            .find(|item| item.item_id == "dagger_iron")
            .expect("forced-quality drop present");
        assert_eq!(dagger.quality, ItemQuality::Poor);
        assert!(dagger.modifiers.is_empty());
    }

    #[test]
    fn mock_table_xp_lookups() {
        let tables = mock_tables();
        assert_eq!(tables.xp("rat"), 10);
        assert_eq!(tables.xp("goblin"), 12);
        assert_eq!(tables.xp("beholder"), 0);
    }

    #[test]
    fn enchanted_modifiers_contains_expected_entries() {
        assert!(ENCHANTED_MODIFIERS.contains(&"fire_damage"));
        assert!(ENCHANTED_MODIFIERS.contains(&"life_steal"));
        assert!(ENCHANTED_MODIFIERS.contains(&"torch_range"));
    }
}

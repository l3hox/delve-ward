//! Item database — typed model of `data/items.json` and query methods.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemQuality {
    Poor,
    Common,
    Fine,
    Masterwork,
    Enchanted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Weapon,
    Armor,
    Accessory,
    Consumable,
    /// Out-of-union value in shipped data (e.g. `armor-steel`). The TS runtime
    /// loads such items but they match no type filter; this mirrors that.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemSubtype {
    // Weapons
    Sword,
    Axe,
    Dagger,
    Mace,
    Spear,
    Staff,
    // Armor
    Head,
    Chest,
    Legs,
    Hands,
    Feet,
    Shield,
    // Accessories
    Ring,
    Amulet,
    // Consumables
    HealthPotion,
    ManaPotion,
    TorchOil,
    Antidote,
    Food,
    Junk,
    /// Out-of-union value in shipped data; mirrors the TS runtime tolerance.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atk: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub def: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub str: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dex: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wis: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crit_chance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dodge_chance: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemModifier {
    pub id: String,
    pub name: String,
    pub effect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ItemStats>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub str: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dex: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wis: Option<f64>,
}

/// Consumable-only effect payload — torch fuel, cure, or hunger restore.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemEffect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torch_fuel: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cure_poison: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_hunger: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDef {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub subtype: ItemSubtype,
    pub quality: ItemQuality,
    pub icon: String,
    pub weight: f64,
    pub value: f64,
    pub description: String,
    pub stats: ItemStats,
    pub modifiers: Vec<ItemModifier>,
    pub requirements: ItemRequirements,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stackable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<ItemEffect>,
}

#[derive(Deserialize)]
struct ItemsJsonPayload {
    #[allow(dead_code)]
    version: String,
    items: Vec<ItemDef>,
}

#[derive(Debug)]
pub struct ItemDatabase {
    items: Vec<ItemDef>,
    index: HashMap<String, usize>,
}

impl ItemDatabase {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let payload: ItemsJsonPayload = serde_json::from_str(json)
            .map_err(|error| format!("Failed to load item database: {error}"))?;
        let mut items: Vec<ItemDef> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for item in payload.items {
            if let Some(&position) = index.get(&item.id) {
                items[position] = item;
            } else {
                index.insert(item.id.clone(), items.len());
                items.push(item);
            }
        }
        Ok(Self { items, index })
    }

    #[must_use]
    pub fn get_item(&self, id: &str) -> Option<&ItemDef> {
        self.index.get(id).map(|&position| &self.items[position])
    }

    #[must_use]
    pub fn items_by_type(&self, item_type: ItemType) -> Vec<&ItemDef> {
        self.items
            .iter()
            .filter(|item| item.item_type == item_type)
            .collect()
    }

    #[must_use]
    pub fn all_items(&self) -> &[ItemDef] {
        &self.items
    }

    pub fn all_item_ids(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|item| item.id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS_JSON: &str = include_str!("../../../assets/data/items.json");

    fn database() -> ItemDatabase {
        ItemDatabase::from_json(ITEMS_JSON).expect("shipped items.json parses")
    }

    #[test]
    fn returns_the_correct_item_after_load() {
        let db = database();
        let item = db.get_item("sword_iron").expect("sword_iron exists");
        assert_eq!(item.id, "sword_iron");
        assert_eq!(item.name, "Iron Sword");
    }

    #[test]
    fn returns_none_for_a_nonexistent_id() {
        assert!(database().get_item("nonexistent").is_none());
    }

    #[test]
    fn items_by_type_returns_only_matching_items() {
        let db = database();
        for item_type in [
            ItemType::Weapon,
            ItemType::Armor,
            ItemType::Consumable,
            ItemType::Accessory,
        ] {
            let matching = db.items_by_type(item_type);
            assert!(!matching.is_empty());
            assert!(matching.iter().all(|item| item.item_type == item_type));
        }
    }

    #[test]
    fn items_by_type_returns_empty_for_empty_database() {
        let db = ItemDatabase::from_json(r#"{ "version": "1.0", "note": "", "items": [] }"#)
            .expect("empty payload parses");
        assert!(db.items_by_type(ItemType::Weapon).is_empty());
    }

    #[test]
    fn sword_rusty_has_correct_type_subtype_quality_and_stats() {
        let db = database();
        let item = db.get_item("sword_rusty").expect("sword_rusty exists");
        assert_eq!(item.item_type, ItemType::Weapon);
        assert_eq!(item.subtype, ItemSubtype::Sword);
        assert_eq!(item.quality, ItemQuality::Poor);
        assert_eq!(item.stats.atk, Some(3.0));
    }

    #[test]
    fn sword_flamebrand_has_enchanted_quality_and_a_modifier() {
        let db = database();
        let item = db
            .get_item("sword_flamebrand")
            .expect("sword_flamebrand exists");
        assert_eq!(item.quality, ItemQuality::Enchanted);
        assert_eq!(item.modifiers.len(), 1);
        assert_eq!(item.modifiers[0].id, "fire_damage");
    }

    #[test]
    fn torch_oil_has_consumable_effect_fields() {
        let db = database();
        let item = db.get_item("torch_oil").expect("torch_oil exists");
        assert_eq!(item.stackable, Some(true));
        assert_eq!(item.stack_max, Some(5));
        assert_eq!(
            item.effect.as_ref().and_then(|effect| effect.torch_fuel),
            Some(100.0)
        );
    }

    #[test]
    fn antidote_has_cure_poison_effect() {
        let db = database();
        let item = db.get_item("antidote").expect("antidote exists");
        assert_eq!(
            item.effect.as_ref().and_then(|effect| effect.cure_poison),
            Some(true)
        );
    }

    #[test]
    fn armor_leather_cap_has_correct_subtype_and_stats() {
        let db = database();
        let item = db
            .get_item("armor_leather_cap")
            .expect("armor_leather_cap exists");
        assert_eq!(item.item_type, ItemType::Armor);
        assert_eq!(item.subtype, ItemSubtype::Head);
        assert_eq!(item.stats.def, Some(1.0));
    }

    #[test]
    fn ring_of_power_has_correct_accessory_fields() {
        let db = database();
        let item = db.get_item("ring_of_power").expect("ring_of_power exists");
        assert_eq!(item.item_type, ItemType::Accessory);
        assert_eq!(item.subtype, ItemSubtype::Ring);
        assert_eq!(item.stats.str, Some(2.0));
    }

    #[test]
    fn dagger_vipers_fang_modifier_includes_crit_chance_stat() {
        let db = database();
        let item = db
            .get_item("dagger_vipers_fang")
            .expect("dagger_vipers_fang exists");
        assert_eq!(
            item.modifiers[0]
                .stats
                .as_ref()
                .and_then(|stats| stats.crit_chance),
            Some(15.0)
        );
    }

    #[test]
    fn sword_iron_has_str_requirement() {
        let db = database();
        let item = db.get_item("sword_iron").expect("sword_iron exists");
        assert_eq!(item.requirements.str, Some(3.0));
    }

    #[test]
    fn load_failure_reports_error() {
        let error = ItemDatabase::from_json("not json").expect_err("invalid JSON fails");
        assert!(error.contains("Failed to load item database"));
    }
}

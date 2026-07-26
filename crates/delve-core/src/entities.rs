//! Entity registry and item location model.
//! Owns all item instances (ground, backpack, equipped) as a single source of truth.

use crate::items::ItemQuality;
use serde::{Deserialize, Serialize};

pub const BACKPACK_MAX_SLOTS: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EquipSlot {
    Weapon,
    Head,
    Chest,
    Legs,
    Hands,
    Feet,
    Shield,
    Ring1,
    Ring2,
    Amulet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ItemLocation {
    #[serde(rename_all = "camelCase")]
    World {
        level_id: String,
        col: i32,
        row: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer_index: Option<i32>,
    },
    Backpack {
        slot: u32,
    },
    Equipped {
        slot: EquipSlot,
    },
}

impl ItemLocation {
    #[must_use]
    pub fn world(level_id: &str, col: i32, row: i32) -> Self {
        ItemLocation::World {
            level_id: level_id.to_string(),
            col,
            row,
            layer_index: None,
        }
    }

    fn backpack_slot(&self) -> Option<u32> {
        match self {
            ItemLocation::Backpack { slot } => Some(*slot),
            _ => None,
        }
    }
}

/// A concrete instance of an item definition. `item_id` references
/// `ItemDef.id`; `modifiers` holds `ItemModifier.id` values on enchanted items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemEntity {
    pub instance_id: String,
    pub item_id: String,
    pub quality: ItemQuality,
    pub modifiers: Vec<String>,
    pub location: ItemLocation,
}

#[derive(Debug, Default)]
pub struct EntityRegistry {
    items: Vec<ItemEntity>,
    next_id: u64,
}

impl EntityRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new item instance, register it, and return a clone of it.
    pub fn create_item(
        &mut self,
        item_id: &str,
        quality: ItemQuality,
        location: ItemLocation,
        modifiers: Vec<String>,
    ) -> ItemEntity {
        let instance_id = format!("item_{}", self.next_id);
        self.next_id += 1;
        let entity = ItemEntity {
            instance_id,
            item_id: item_id.to_string(),
            quality,
            modifiers,
            location,
        };
        self.items.push(entity.clone());
        entity
    }

    /// Register an existing item (e.g. restored from a snapshot), replacing
    /// any item with the same instance id.
    pub fn add_item(&mut self, entity: ItemEntity) {
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|item| item.instance_id == entity.instance_id)
        {
            *existing = entity;
        } else {
            self.items.push(entity);
        }
    }

    pub fn remove_item(&mut self, instance_id: &str) {
        self.items.retain(|item| item.instance_id != instance_id);
    }

    #[must_use]
    pub fn get_item(&self, instance_id: &str) -> Option<&ItemEntity> {
        self.items
            .iter()
            .find(|item| item.instance_id == instance_id)
    }

    pub fn move_item(&mut self, instance_id: &str, location: ItemLocation) {
        if let Some(entity) = self
            .items
            .iter_mut()
            .find(|item| item.instance_id == instance_id)
        {
            entity.location = location;
        }
    }

    /// Items sitting on the ground at a specific cell, optionally filtered by layer.
    #[must_use]
    pub fn ground_items(
        &self,
        level_id: &str,
        col: i32,
        row: i32,
        layer_index: Option<i32>,
    ) -> Vec<&ItemEntity> {
        self.items
            .iter()
            .filter(|item| match &item.location {
                ItemLocation::World {
                    level_id: id,
                    col: c,
                    row: r,
                    layer_index: item_layer,
                } => {
                    id == level_id
                        && *c == col
                        && *r == row
                        && layer_index.is_none_or(|wanted| item_layer.unwrap_or(0) == wanted)
                }
                _ => false,
            })
            .collect()
    }

    /// All ground items in a level, optionally filtered by layer.
    #[must_use]
    pub fn all_ground_items_for_level(
        &self,
        level_id: &str,
        layer_index: Option<i32>,
    ) -> Vec<&ItemEntity> {
        self.items
            .iter()
            .filter(|item| match &item.location {
                ItemLocation::World {
                    level_id: id,
                    layer_index: item_layer,
                    ..
                } => {
                    id == level_id
                        && layer_index.is_none_or(|wanted| item_layer.unwrap_or(0) == wanted)
                }
                _ => false,
            })
            .collect()
    }

    /// Backpack items sorted by slot index (ascending).
    #[must_use]
    pub fn backpack_items(&self) -> Vec<&ItemEntity> {
        let mut result: Vec<&ItemEntity> = self
            .items
            .iter()
            .filter(|item| item.location.backpack_slot().is_some())
            .collect();
        result.sort_by_key(|item| item.location.backpack_slot());
        result
    }

    #[must_use]
    pub fn backpack_item_at(&self, slot: u32) -> Option<&ItemEntity> {
        self.items
            .iter()
            .find(|item| item.location.backpack_slot() == Some(slot))
    }

    /// Swap or move backpack items by slot index. If both slots are occupied:
    /// swap. If only the source is occupied: move it to the target slot.
    pub fn swap_backpack_slots(&mut self, slot_a: u32, slot_b: u32) {
        let index_a = self
            .items
            .iter()
            .position(|item| item.location.backpack_slot() == Some(slot_a));
        let index_b = self
            .items
            .iter()
            .position(|item| item.location.backpack_slot() == Some(slot_b));
        match (index_a, index_b) {
            (Some(a), Some(b)) => {
                self.items[a].location = ItemLocation::Backpack { slot: slot_b };
                self.items[b].location = ItemLocation::Backpack { slot: slot_a };
            }
            (Some(a), None) => {
                self.items[a].location = ItemLocation::Backpack { slot: slot_b };
            }
            _ => {}
        }
    }

    /// First free backpack slot, or `None` when the backpack is full.
    #[must_use]
    pub fn next_backpack_slot(&self) -> Option<u32> {
        (0..BACKPACK_MAX_SLOTS).find(|&slot| self.backpack_item_at(slot).is_none())
    }

    #[must_use]
    pub fn get_equipped(&self, slot: EquipSlot) -> Option<&ItemEntity> {
        self.items
            .iter()
            .find(|item| matches!(item.location, ItemLocation::Equipped { slot: s } if s == slot))
    }

    /// All equipped items with their slots.
    #[must_use]
    pub fn all_equipped(&self) -> Vec<(EquipSlot, &ItemEntity)> {
        self.items
            .iter()
            .filter_map(|item| match item.location {
                ItemLocation::Equipped { slot } => Some((slot, item)),
                _ => None,
            })
            .collect()
    }

    /// Drop all ground items belonging to a level (called on level transition).
    pub fn clear_level(&mut self, level_id: &str) {
        self.items.retain(|item| {
            !matches!(&item.location, ItemLocation::World { level_id: id, .. } if id == level_id)
        });
    }

    /// Remove all items — full game reset.
    pub fn clear(&mut self) {
        self.items.clear();
        self.next_id = 0;
    }

    /// Deep copy of every item for save/load.
    #[must_use]
    pub fn snapshot(&self) -> Vec<ItemEntity> {
        self.items.clone()
    }

    /// Restore from a snapshot — replaces current state entirely. The next
    /// issued id is set above the highest restored `item_N` id.
    pub fn restore(&mut self, items: Vec<ItemEntity>) {
        let mut max_id: i64 = -1;
        for entity in &items {
            if let Some(numeric) = entity
                .instance_id
                .strip_prefix("item_")
                .and_then(|suffix| suffix.parse::<i64>().ok())
            {
                max_id = max_id.max(numeric);
            }
        }
        self.items = items;
        self.next_id = u64::try_from(max_id + 1).unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ItemQuality::{Common, Enchanted, Fine, Masterwork};

    fn world(level_id: &str, col: i32, row: i32) -> ItemLocation {
        ItemLocation::world(level_id, col, row)
    }

    fn backpack(slot: u32) -> ItemLocation {
        ItemLocation::Backpack { slot }
    }

    fn equipped(slot: EquipSlot) -> ItemLocation {
        ItemLocation::Equipped { slot }
    }

    fn instance_number(entity: &ItemEntity) -> u64 {
        entity
            .instance_id
            .strip_prefix("item_")
            .expect("item_N id")
            .parse()
            .expect("numeric suffix")
    }

    #[test]
    fn create_item_returns_item_n_instance_ids() {
        let mut registry = EntityRegistry::new();
        let entity = registry.create_item("sword_iron", Common, world("l1", 1, 1), Vec::new());
        assert!(entity.instance_id.starts_with("item_"));
        instance_number(&entity);
    }

    #[test]
    fn create_item_increments_instance_id() {
        let mut registry = EntityRegistry::new();
        let first = registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        let second = registry.create_item("sword_iron", Common, world("l1", 1, 1), Vec::new());
        assert_eq!(instance_number(&second), instance_number(&first) + 1);
    }

    #[test]
    fn create_item_stores_fields() {
        let mut registry = EntityRegistry::new();
        let location = world("dungeon1", 3, 5);
        let entity = registry.create_item("axe_hand", Masterwork, location.clone(), Vec::new());
        assert_eq!(entity.item_id, "axe_hand");
        assert_eq!(entity.quality, Masterwork);
        assert_eq!(entity.location, location);
        assert!(entity.modifiers.is_empty());
    }

    #[test]
    fn create_item_stores_modifiers() {
        let mut registry = EntityRegistry::new();
        let entity = registry.create_item(
            "sword_flamebrand",
            Enchanted,
            world("l1", 0, 0),
            vec!["fire_damage".to_string()],
        );
        assert_eq!(entity.modifiers, vec!["fire_damage".to_string()]);
    }

    #[test]
    fn add_item_then_get_item_returns_it() {
        let mut registry = EntityRegistry::new();
        let entity = ItemEntity {
            instance_id: "item_99".to_string(),
            item_id: "sword_iron".to_string(),
            quality: Common,
            modifiers: Vec::new(),
            location: world("l1", 1, 1),
        };
        registry.add_item(entity.clone());
        assert_eq!(registry.get_item("item_99"), Some(&entity));
    }

    #[test]
    fn get_item_returns_none_for_unknown_id() {
        let registry = EntityRegistry::new();
        assert!(registry.get_item("item_9999").is_none());
    }

    #[test]
    fn remove_item_removes_it_and_tolerates_unknown_ids() {
        let mut registry = EntityRegistry::new();
        let entity = registry.create_item("dagger_iron", Common, world("l1", 0, 0), Vec::new());
        registry.remove_item(&entity.instance_id);
        assert!(registry.get_item(&entity.instance_id).is_none());
        registry.remove_item("item_999");
    }

    #[test]
    fn move_item_updates_location_and_tolerates_unknown_ids() {
        let mut registry = EntityRegistry::new();
        let entity = registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        registry.move_item(&entity.instance_id, backpack(3));
        assert_eq!(
            registry.get_item(&entity.instance_id).map(|e| &e.location),
            Some(&backpack(3))
        );
        registry.move_item("item_999", backpack(0));
    }

    #[test]
    fn ground_items_returns_only_matching_cell() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 2, 3), Vec::new());
        registry.create_item("dagger_iron", Common, world("l1", 5, 7), Vec::new());
        let items = registry.ground_items("l1", 2, 3, None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_id, "sword_iron");
    }

    #[test]
    fn ground_items_returns_multiple_at_same_cell() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 1, 1), Vec::new());
        registry.create_item("dagger_iron", Common, world("l1", 1, 1), Vec::new());
        assert_eq!(registry.ground_items("l1", 1, 1, None).len(), 2);
    }

    #[test]
    fn ground_items_excludes_other_cells_levels_and_locations() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        assert!(registry.ground_items("l1", 9, 9, None).is_empty());

        registry.clear();
        registry.create_item("sword_iron", Common, world("l2", 1, 1), Vec::new());
        assert!(registry.ground_items("l1", 1, 1, None).is_empty());

        registry.clear();
        registry.create_item("sword_iron", Common, backpack(0), Vec::new());
        registry.create_item(
            "dagger_iron",
            Common,
            equipped(EquipSlot::Weapon),
            Vec::new(),
        );
        assert!(registry.ground_items("l1", 0, 0, None).is_empty());
    }

    /// A shipped level never stacks ground items across layers at the same
    /// cell, so this overlap has to be constructed by hand: without layer
    /// filtering, `ground_items` would return both the requested layer's
    /// item and its neighbour's, letting a walk-over pickup on one layer
    /// grab an item that is really sitting on the layer below it.
    #[test]
    fn ground_items_filters_by_layer_when_two_layers_share_a_cell() {
        let mut registry = EntityRegistry::new();
        registry.create_item(
            "sword_iron",
            Common,
            ItemLocation::World {
                level_id: "l1".to_string(),
                col: 4,
                row: 4,
                layer_index: Some(0),
            },
            Vec::new(),
        );
        registry.create_item(
            "dagger_iron",
            Common,
            ItemLocation::World {
                level_id: "l1".to_string(),
                col: 4,
                row: 4,
                layer_index: Some(1),
            },
            Vec::new(),
        );

        let ground_floor = registry.ground_items("l1", 4, 4, Some(0));
        assert_eq!(ground_floor.len(), 1);
        assert_eq!(ground_floor[0].item_id, "sword_iron");

        let upper_floor = registry.ground_items("l1", 4, 4, Some(1));
        assert_eq!(upper_floor.len(), 1);
        assert_eq!(upper_floor[0].item_id, "dagger_iron");

        assert_eq!(registry.ground_items("l1", 4, 4, None).len(), 2);
    }

    #[test]
    fn all_ground_items_for_level_filters_by_level_and_location() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        registry.create_item("dagger_iron", Common, world("l1", 1, 1), Vec::new());
        registry.create_item("axe_hand", Common, world("l2", 0, 0), Vec::new());
        registry.create_item("mace_iron", Common, backpack(0), Vec::new());
        registry.create_item("bone", Common, equipped(EquipSlot::Weapon), Vec::new());
        assert_eq!(registry.all_ground_items_for_level("l1", None).len(), 2);
        assert_eq!(registry.all_ground_items_for_level("l3", None).len(), 0);
    }

    #[test]
    fn backpack_items_sorted_by_slot_ascending() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, backpack(5), Vec::new());
        registry.create_item("dagger_iron", Common, backpack(2), Vec::new());
        registry.create_item("axe_hand", Common, backpack(0), Vec::new());
        let slots: Vec<u32> = registry
            .backpack_items()
            .iter()
            .filter_map(|item| item.location.backpack_slot())
            .collect();
        assert_eq!(slots, vec![0, 2, 5]);
    }

    #[test]
    fn backpack_items_excludes_world_and_equipped() {
        let mut registry = EntityRegistry::new();
        registry.create_item(
            "sword_iron",
            Common,
            equipped(EquipSlot::Weapon),
            Vec::new(),
        );
        registry.create_item("dagger_iron", Common, world("l1", 0, 0), Vec::new());
        assert!(registry.backpack_items().is_empty());
    }

    #[test]
    fn backpack_item_at_finds_slot_occupant() {
        let mut registry = EntityRegistry::new();
        let entity = registry.create_item("sword_iron", Common, backpack(4), Vec::new());
        assert_eq!(
            registry.backpack_item_at(4).map(|e| e.instance_id.clone()),
            Some(entity.instance_id)
        );
        assert!(registry.backpack_item_at(7).is_none());
    }

    #[test]
    fn swap_backpack_slots_swaps_and_moves() {
        let mut registry = EntityRegistry::new();
        let first = registry.create_item("sword_iron", Common, backpack(0), Vec::new());
        let second = registry.create_item("dagger_iron", Common, backpack(1), Vec::new());
        registry.swap_backpack_slots(0, 1);
        assert_eq!(
            registry.get_item(&first.instance_id).map(|e| &e.location),
            Some(&backpack(1))
        );
        assert_eq!(
            registry.get_item(&second.instance_id).map(|e| &e.location),
            Some(&backpack(0))
        );

        registry.swap_backpack_slots(1, 5);
        assert_eq!(
            registry.get_item(&first.instance_id).map(|e| &e.location),
            Some(&backpack(5))
        );
    }

    #[test]
    fn next_backpack_slot_finds_first_gap() {
        let mut registry = EntityRegistry::new();
        assert_eq!(registry.next_backpack_slot(), Some(0));

        registry.create_item("sword_iron", Common, backpack(0), Vec::new());
        registry.create_item("dagger_iron", Common, backpack(1), Vec::new());
        assert_eq!(registry.next_backpack_slot(), Some(2));

        registry.clear();
        registry.create_item("sword_iron", Common, backpack(0), Vec::new());
        registry.create_item("dagger_iron", Common, backpack(2), Vec::new());
        assert_eq!(registry.next_backpack_slot(), Some(1));
    }

    #[test]
    fn next_backpack_slot_returns_none_when_full() {
        let mut registry = EntityRegistry::new();
        for slot in 0..BACKPACK_MAX_SLOTS {
            registry.create_item("sword_iron", Common, backpack(slot), Vec::new());
        }
        assert_eq!(registry.next_backpack_slot(), None);
    }

    #[test]
    fn next_backpack_slot_ignores_world_and_equipped() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        registry.create_item(
            "dagger_iron",
            Common,
            equipped(EquipSlot::Weapon),
            Vec::new(),
        );
        assert_eq!(registry.next_backpack_slot(), Some(0));
    }

    #[test]
    fn get_equipped_finds_slot_occupant() {
        let mut registry = EntityRegistry::new();
        let entity = registry.create_item(
            "sword_iron",
            Common,
            equipped(EquipSlot::Weapon),
            Vec::new(),
        );
        assert_eq!(
            registry
                .get_equipped(EquipSlot::Weapon)
                .map(|e| e.instance_id.clone()),
            Some(entity.instance_id)
        );
        assert!(registry.get_equipped(EquipSlot::Head).is_none());
    }

    #[test]
    fn all_equipped_returns_slot_item_pairs() {
        let mut registry = EntityRegistry::new();
        registry.create_item(
            "sword_iron",
            Common,
            equipped(EquipSlot::Weapon),
            Vec::new(),
        );
        registry.create_item(
            "armor_leather_cap",
            Common,
            equipped(EquipSlot::Head),
            Vec::new(),
        );
        let equipped_items = registry.all_equipped();
        assert_eq!(equipped_items.len(), 2);
        assert!(
            equipped_items
                .iter()
                .any(|(slot, _)| *slot == EquipSlot::Weapon)
        );
        assert!(
            equipped_items
                .iter()
                .any(|(slot, _)| *slot == EquipSlot::Head)
        );

        registry.clear();
        registry.create_item("sword_iron", Common, backpack(0), Vec::new());
        assert!(registry.all_equipped().is_empty());
    }

    #[test]
    fn clear_level_removes_only_that_level_ground_items() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        registry.create_item("dagger_iron", Common, world("l1", 1, 1), Vec::new());
        registry.create_item("axe_hand", Common, world("l2", 0, 0), Vec::new());
        let kept_backpack = registry.create_item("mace_iron", Common, backpack(0), Vec::new());
        let kept_equipped =
            registry.create_item("bone", Common, equipped(EquipSlot::Weapon), Vec::new());

        registry.clear_level("l1");
        assert!(registry.all_ground_items_for_level("l1", None).is_empty());
        assert_eq!(registry.all_ground_items_for_level("l2", None).len(), 1);
        assert!(registry.get_item(&kept_backpack.instance_id).is_some());
        assert!(registry.get_item(&kept_equipped.instance_id).is_some());
    }

    #[test]
    fn clear_removes_everything_and_resets_ids() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        registry.create_item("dagger_iron", Common, backpack(0), Vec::new());
        registry.create_item("axe_hand", Common, equipped(EquipSlot::Weapon), Vec::new());
        registry.clear();
        assert!(registry.all_ground_items_for_level("l1", None).is_empty());
        assert!(registry.backpack_items().is_empty());
        assert!(registry.all_equipped().is_empty());

        let entity = registry.create_item("dagger_iron", Common, world("l1", 0, 0), Vec::new());
        assert_eq!(entity.instance_id, "item_0");
    }

    #[test]
    fn snapshot_is_a_deep_copy() {
        let mut registry = EntityRegistry::new();
        let entity = registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        registry.create_item("dagger_iron", Fine, backpack(2), Vec::new());
        let mut snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
        snapshot[0].item_id = "tampered".to_string();
        assert_eq!(
            registry
                .get_item(&entity.instance_id)
                .map(|e| e.item_id.clone()),
            Some("sword_iron".to_string())
        );
    }

    #[test]
    fn restore_produces_identical_state() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 1, 2), Vec::new());
        registry.create_item(
            "dagger_iron",
            Fine,
            backpack(0),
            vec!["crit_bonus".to_string()],
        );
        registry.create_item(
            "axe_hand",
            Masterwork,
            equipped(EquipSlot::Weapon),
            Vec::new(),
        );
        let snapshot = registry.snapshot();

        let mut restored = EntityRegistry::new();
        restored.restore(snapshot.clone());
        let mut restored_snapshot = restored.snapshot();

        let mut expected = snapshot;
        expected.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        restored_snapshot.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        assert_eq!(restored_snapshot, expected);
    }

    #[test]
    fn restore_sets_next_id_above_highest_restored() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        registry.create_item("dagger_iron", Common, world("l1", 1, 0), Vec::new());
        registry.create_item("axe_hand", Common, world("l1", 2, 0), Vec::new());
        let snapshot = registry.snapshot();

        let mut restored = EntityRegistry::new();
        restored.restore(snapshot);
        let new_entity = restored.create_item("mace_iron", Common, world("l1", 0, 0), Vec::new());
        assert!(instance_number(&new_entity) > 2);
    }

    #[test]
    fn restore_replaces_current_state_entirely() {
        let mut registry = EntityRegistry::new();
        registry.create_item("sword_iron", Common, world("l1", 0, 0), Vec::new());
        let snapshot = registry.snapshot();

        registry.create_item("dagger_iron", Common, world("l1", 1, 1), Vec::new());
        assert_eq!(registry.all_ground_items_for_level("l1", None).len(), 2);

        registry.restore(snapshot);
        assert_eq!(registry.all_ground_items_for_level("l1", None).len(), 1);
    }
}

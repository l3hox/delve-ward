//! Canonical equipment-slot layout (two rows of five) shared by the mini
//! inventory panel, the full-screen inventory overlay, and the item
//! tooltip's slot-comparison lookup — ported from TS's `EQUIP_SLOTS`
//! (identical, independently defined, in both `inventoryOverlay.ts` and
//! `inventoryPanel.ts`) and `subtypeToEquipSlot`. Centralized per
//! `PHASE4-PLAN.md`'s risk #1: two independently maintained copies of this
//! order previously risked silently drifting apart.

use delve_core::entities::EquipSlot;
use delve_core::game_state::GameState;
use delve_core::items::ItemSubtype;

pub const EQUIP_SLOTS: [EquipSlot; 10] = [
    EquipSlot::Weapon,
    EquipSlot::Head,
    EquipSlot::Chest,
    EquipSlot::Legs,
    EquipSlot::Hands,
    EquipSlot::Shield,
    EquipSlot::Feet,
    EquipSlot::Ring1,
    EquipSlot::Ring2,
    EquipSlot::Amulet,
];

/// Index of `slot` within [`EQUIP_SLOTS`].
#[must_use]
pub fn equip_slot_index(slot: EquipSlot) -> Option<usize> {
    EQUIP_SLOTS.iter().position(|&candidate| candidate == slot)
}

/// Which equip slot an item subtype targets, ported from TS's
/// `subtypeToEquipSlot`: rings alternate to ring2 once ring1 is occupied.
/// Falls back to `Weapon` for subtypes with no equip slot (consumables'
/// subtypes are never passed here in practice, matching TS's own
/// never-called-for-consumables invariant, but the fallback still matches
/// TS's unconditional default case).
#[must_use]
pub fn subtype_to_equip_slot(subtype: ItemSubtype, game: &GameState) -> EquipSlot {
    match subtype {
        ItemSubtype::Sword
        | ItemSubtype::Axe
        | ItemSubtype::Dagger
        | ItemSubtype::Mace
        | ItemSubtype::Spear
        | ItemSubtype::Staff => EquipSlot::Weapon,
        ItemSubtype::Head => EquipSlot::Head,
        ItemSubtype::Chest => EquipSlot::Chest,
        ItemSubtype::Legs => EquipSlot::Legs,
        ItemSubtype::Hands => EquipSlot::Hands,
        ItemSubtype::Feet => EquipSlot::Feet,
        ItemSubtype::Shield => EquipSlot::Shield,
        ItemSubtype::Ring => {
            if game
                .entity_registry
                .get_equipped(EquipSlot::Ring1)
                .is_some()
            {
                EquipSlot::Ring2
            } else {
                EquipSlot::Ring1
            }
        }
        ItemSubtype::Amulet => EquipSlot::Amulet,
        _ => EquipSlot::Weapon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use delve_core::entities::ItemLocation;
    use delve_core::items::ItemQuality;

    fn game() -> GameState {
        GameState::new(
            &[],
            None,
            "test_level",
            None,
            delve_core::game_state::GameStateDeps::default(),
            &mut || 0.0,
        )
    }

    #[test]
    fn weapon_subtypes_map_to_weapon() {
        let game = game();
        for subtype in [
            ItemSubtype::Sword,
            ItemSubtype::Axe,
            ItemSubtype::Dagger,
            ItemSubtype::Mace,
            ItemSubtype::Spear,
            ItemSubtype::Staff,
        ] {
            assert_eq!(subtype_to_equip_slot(subtype, &game), EquipSlot::Weapon);
        }
    }

    #[test]
    fn armor_subtypes_map_to_matching_slots() {
        let game = game();
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Head, &game),
            EquipSlot::Head
        );
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Chest, &game),
            EquipSlot::Chest
        );
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Legs, &game),
            EquipSlot::Legs
        );
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Hands, &game),
            EquipSlot::Hands
        );
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Feet, &game),
            EquipSlot::Feet
        );
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Shield, &game),
            EquipSlot::Shield
        );
    }

    #[test]
    fn ring_maps_to_ring1_when_empty() {
        let game = game();
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Ring, &game),
            EquipSlot::Ring1
        );
    }

    #[test]
    fn ring_maps_to_ring2_when_ring1_occupied() {
        let mut game = game();
        game.entity_registry.create_item(
            "ring_gold",
            ItemQuality::Common,
            ItemLocation::Equipped {
                slot: EquipSlot::Ring1,
            },
            Vec::new(),
        );
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Ring, &game),
            EquipSlot::Ring2
        );
    }

    #[test]
    fn amulet_maps_to_amulet_slot() {
        let game = game();
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Amulet, &game),
            EquipSlot::Amulet
        );
    }

    #[test]
    fn unknown_subtype_falls_back_to_weapon() {
        let game = game();
        assert_eq!(
            subtype_to_equip_slot(ItemSubtype::Junk, &game),
            EquipSlot::Weapon
        );
    }

    #[test]
    fn equip_slot_index_matches_declared_order() {
        assert_eq!(equip_slot_index(EquipSlot::Weapon), Some(0));
        assert_eq!(equip_slot_index(EquipSlot::Amulet), Some(9));
    }
}

//! Ported from `src/core/playerController.test.ts`.

use delve_core::entities::{EquipSlot, ItemLocation};
use delve_core::game_state::{GameState, GameStateDeps};
use delve_core::items::ItemDatabase;
use delve_core::player_controller::{
    HUNGER_DRAIN_INTERVAL, InventoryAction, PLAYER_DAMAGE_FLASH_DURATION, PlayerTickState,
    STARVATION_INTERVAL, process_inventory_action, should_drain_torch, tick_player_controller,
};
use delve_core::status_effects::{StatusEffectType, apply_effect};
use delve_core::types::Environment;
use std::sync::Arc;

const ITEMS_JSON: &str = include_str!("fixtures/player-controller-items-mock.json");

fn deps() -> GameStateDeps {
    GameStateDeps {
        items: Some(Arc::new(
            ItemDatabase::from_json(ITEMS_JSON).expect("mock items parse"),
        )),
        enemy_registrar: None,
        npc_registrar: None,
    }
}

fn gs() -> GameState {
    GameState::new(&[], None, "default", None, deps(), &mut || 0.5)
}

// ---------------------------------------------------------------------------
// tick_player_controller
// ---------------------------------------------------------------------------

#[test]
fn hunger_accumulator_drains_after_hunger_drain_interval_seconds() {
    let mut game = gs();
    let mut tick_state = PlayerTickState {
        hunger_drain_accumulator: HUNGER_DRAIN_INTERVAL - 0.5,
        ..PlayerTickState::default()
    };
    let hunger_before = game.status_fx.hunger;

    tick_player_controller(&mut game, &mut tick_state, 1.0, false);

    assert_eq!(game.status_fx.hunger, hunger_before - 1.0);
    assert_eq!(tick_state.hunger_drain_accumulator, 0.5);
}

#[test]
fn starvation_deals_one_damage_after_starvation_interval_when_hungry() {
    let mut game = gs();
    game.status_fx.hunger = 0.0;
    let mut tick_state = PlayerTickState {
        starvation_accumulator: STARVATION_INTERVAL - 0.1,
        ..PlayerTickState::default()
    };
    let hp_before = game.player.hp;

    tick_player_controller(&mut game, &mut tick_state, 0.2, false);

    assert_eq!(game.player.hp, hp_before - 1.0);
    assert_eq!(
        tick_state.player_damage_flash_timer,
        PLAYER_DAMAGE_FLASH_DURATION
    );
    assert!(tick_state.starvation_accumulator < STARVATION_INTERVAL);
}

#[test]
fn starvation_resets_accumulator_when_not_hungry() {
    let mut game = gs();
    game.status_fx.hunger = 50.0;
    let mut tick_state = PlayerTickState {
        starvation_accumulator: 1.5,
        ..PlayerTickState::default()
    };

    tick_player_controller(&mut game, &mut tick_state, 0.1, false);

    assert_eq!(tick_state.starvation_accumulator, 0.0);
}

#[test]
fn status_effect_damage_applied_and_flash_timer_set() {
    let mut game = gs();
    let mut tick_state = PlayerTickState::default();
    apply_effect(
        &mut game.status_fx.player_status_effects,
        StatusEffectType::Poison,
        5.0,
    );
    let hp_before = game.player.hp;

    tick_player_controller(&mut game, &mut tick_state, 1.1, false);

    // Poison ticks 2 damage per elapsed 1s interval; one interval elapses.
    assert_eq!(game.player.hp, hp_before - 2.0);
    assert_eq!(
        tick_state.player_damage_flash_timer,
        PLAYER_DAMAGE_FLASH_DURATION
    );
    assert_eq!(game.status_fx.player_status_effects.len(), 1);
    assert!(game.status_fx.player_status_effects[0].remaining > 3.8);
}

#[test]
fn debug_fullbright_suppresses_status_effect_damage_and_starvation() {
    let mut game = gs();
    let mut tick_state = PlayerTickState {
        starvation_accumulator: STARVATION_INTERVAL - 0.1,
        ..PlayerTickState::default()
    };
    apply_effect(
        &mut game.status_fx.player_status_effects,
        StatusEffectType::Poison,
        5.0,
    );
    game.status_fx.hunger = 0.0;
    let hp_before = game.player.hp;

    tick_player_controller(&mut game, &mut tick_state, 0.5, true);

    assert_eq!(game.player.hp, hp_before);
    assert_eq!(tick_state.player_damage_flash_timer, 0.0);
    // The starvation branch is gated on `!debug_fullbright` too, so the
    // accumulator still resets via the else arm even while hungry.
    assert_eq!(tick_state.starvation_accumulator, 0.0);
}

// ---------------------------------------------------------------------------
// should_drain_torch
// ---------------------------------------------------------------------------

#[test]
fn should_drain_torch_true_for_dungeon() {
    assert!(should_drain_torch(Environment::Dungeon));
}

#[test]
fn should_drain_torch_false_for_outdoor() {
    assert!(!should_drain_torch(Environment::Outdoor));
}

#[test]
fn should_drain_torch_false_for_mist() {
    assert!(!should_drain_torch(Environment::Mist));
}

// ---------------------------------------------------------------------------
// process_inventory_action
// ---------------------------------------------------------------------------

fn noop_on_drop(_instance_id: &str, _col: i64, _row: i64) {}

#[test]
fn equip_delegates_to_equip_from_backpack() {
    let mut game = gs();
    game.entity_registry.create_item(
        "sword_iron",
        delve_core::items::ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    let mut on_drop = noop_on_drop;

    process_inventory_action(
        &InventoryAction::Equip {
            backpack_slot: 0,
            equip_slot: EquipSlot::Weapon,
        },
        &mut game,
        &mut on_drop,
    );

    let equipped = game
        .entity_registry
        .get_equipped(EquipSlot::Weapon)
        .expect("sword equipped");
    assert_eq!(equipped.item_id, "sword_iron");
    assert!(game.entity_registry.backpack_item_at(0).is_none());
}

#[test]
fn unequip_delegates_to_unequip_to_backpack() {
    let mut game = gs();
    game.entity_registry.create_item(
        "sword_iron",
        delve_core::items::ItemQuality::Common,
        ItemLocation::Equipped {
            slot: EquipSlot::Weapon,
        },
        Vec::new(),
    );
    let mut on_drop = noop_on_drop;

    process_inventory_action(
        &InventoryAction::Unequip {
            equip_slot: EquipSlot::Weapon,
            backpack_slot: 0,
        },
        &mut game,
        &mut on_drop,
    );

    assert!(
        game.entity_registry
            .get_equipped(EquipSlot::Weapon)
            .is_none()
    );
    let backpack = game
        .entity_registry
        .backpack_item_at(0)
        .expect("sword unequipped into slot 0");
    assert_eq!(backpack.item_id, "sword_iron");
}

#[test]
fn swap_delegates_to_swap_backpack_slots() {
    let mut game = gs();
    game.entity_registry.create_item(
        "sword_iron",
        delve_core::items::ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    game.entity_registry.create_item(
        "health_potion",
        delve_core::items::ItemQuality::Common,
        ItemLocation::Backpack { slot: 1 },
        Vec::new(),
    );
    let mut on_drop = noop_on_drop;

    process_inventory_action(
        &InventoryAction::Swap {
            index_a: 0,
            index_b: 1,
        },
        &mut game,
        &mut on_drop,
    );

    assert_eq!(
        game.entity_registry.backpack_item_at(0).unwrap().item_id,
        "health_potion"
    );
    assert_eq!(
        game.entity_registry.backpack_item_at(1).unwrap().item_id,
        "sword_iron"
    );
}

#[test]
fn drop_triggers_on_drop_callback() {
    let mut game = gs();
    let item = game.entity_registry.create_item(
        "health_potion",
        delve_core::items::ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    let instance_id = item.instance_id.clone();

    let mut recorded: Vec<(String, i64, i64)> = Vec::new();
    {
        let mut on_drop = |dropped_id: &str, col: i64, row: i64| {
            recorded.push((dropped_id.to_string(), col, row));
        };

        process_inventory_action(
            &InventoryAction::Drop {
                instance_id: instance_id.clone(),
                col: 2,
                row: 3,
            },
            &mut game,
            &mut on_drop,
        );
    }
    assert_eq!(recorded, vec![(instance_id.clone(), 2, 3)]);

    let moved = game
        .entity_registry
        .get_item(&instance_id)
        .expect("item still tracked");
    assert!(matches!(
        moved.location,
        ItemLocation::World { col: 2, row: 3, .. }
    ));
}

#[test]
fn use_invokes_use_consumable_from_registry() {
    let mut game = gs();
    game.player.hp = 10.0;
    game.entity_registry.create_item(
        "health_potion",
        delve_core::items::ItemQuality::Common,
        ItemLocation::Backpack { slot: 0 },
        Vec::new(),
    );
    let mut on_drop = noop_on_drop;

    process_inventory_action(
        &InventoryAction::Use { backpack_slot: 0 },
        &mut game,
        &mut on_drop,
    );

    assert_eq!(game.player.hp, 30.0);
    assert!(game.entity_registry.backpack_item_at(0).is_none());
}

// ---------------------------------------------------------------------------
// Exported constants — values match the TS legacy constants.
// ---------------------------------------------------------------------------

#[test]
fn hunger_drain_interval_is_ten() {
    assert_eq!(HUNGER_DRAIN_INTERVAL, 10.0);
}

#[test]
fn starvation_interval_is_three() {
    assert_eq!(STARVATION_INTERVAL, 3.0);
}

#[test]
fn player_damage_flash_duration_is_point_one_five() {
    assert_eq!(PLAYER_DAMAGE_FLASH_DURATION, 0.15);
}

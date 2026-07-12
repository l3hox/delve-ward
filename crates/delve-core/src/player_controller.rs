//! Player-side per-frame ticking and inventory-action dispatch, ported from
//! the TS `playerController`. Status effects, hunger, and torch fuel already
//! tick correctly for enemies and via direct `GameState` calls; this module
//! is what drives them for the player each frame, plus the HUD-driven
//! equip/unequip/use/drop/swap flow.
//!
//! The TS module imports `InventoryAction` from `../hud/inventoryOverlay`
//! (a Bevy-shell concern); since delve-core owns the game logic that acts on
//! it, the type is defined here instead and the shell constructs values of
//! it once the inventory overlay lands.

use crate::entities::EquipSlot;
use crate::game_state::GameState;
use crate::status_effects::tick_effects;
use crate::types::Environment;

pub const HUNGER_DRAIN_INTERVAL: f64 = 10.0;
pub const STARVATION_INTERVAL: f64 = 3.0;
pub const PLAYER_DAMAGE_FLASH_DURATION: f64 = 0.15;

/// Mutable per-frame accumulators the caller owns and passes back in each
/// tick, mirroring the TS `PlayerTickState`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerTickState {
    pub player_damage_flash_timer: f64,
    pub hunger_drain_accumulator: f64,
    pub starvation_accumulator: f64,
}

/// Applies player status-effect damage, temp-buff decay, hunger drain, and
/// starvation damage for one frame. `debug_fullbright` suppresses both
/// status-effect and starvation damage, matching the TS debug flag.
pub fn tick_player_controller(
    game: &mut GameState,
    tick_state: &mut PlayerTickState,
    delta: f64,
    debug_fullbright: bool,
) {
    let effect_result = tick_effects(&mut game.status_fx.player_status_effects, delta);
    if effect_result.damage > 0.0 && !debug_fullbright {
        game.player.hp = (game.player.hp - effect_result.damage).max(0.0);
        tick_state.player_damage_flash_timer = PLAYER_DAMAGE_FLASH_DURATION;
    }
    game.status_fx
        .player_status_effects
        .retain(|effect| effect.remaining > 0.0);

    game.tick_temp_buffs(delta);

    tick_state.hunger_drain_accumulator += delta;
    while tick_state.hunger_drain_accumulator >= HUNGER_DRAIN_INTERVAL {
        tick_state.hunger_drain_accumulator -= HUNGER_DRAIN_INTERVAL;
        game.drain_hunger(1.0);
    }

    if game.status_fx.hunger <= 0.0 && !debug_fullbright {
        tick_state.starvation_accumulator += delta;
        while tick_state.starvation_accumulator >= STARVATION_INTERVAL {
            tick_state.starvation_accumulator -= STARVATION_INTERVAL;
            game.player.hp = (game.player.hp - 1.0).max(0.0);
            tick_state.player_damage_flash_timer = PLAYER_DAMAGE_FLASH_DURATION;
        }
    } else {
        tick_state.starvation_accumulator = 0.0;
    }
}

/// The torch drains everywhere except environments with their own light
/// source (open sky, luminous mist).
#[must_use]
pub fn should_drain_torch(environment: Environment) -> bool {
    !matches!(environment, Environment::Outdoor | Environment::Mist)
}

/// HUD-driven inventory action, mirrored from the TS `InventoryAction`
/// union. `Message` carries HUD text and is handled by the shell before
/// reaching `process_inventory_action` (same as in TS, where it has no
/// matching switch case here).
#[derive(Debug, Clone, PartialEq)]
pub enum InventoryAction {
    Equip {
        backpack_slot: u32,
        equip_slot: EquipSlot,
    },
    Unequip {
        equip_slot: EquipSlot,
        backpack_slot: u32,
    },
    Use {
        backpack_slot: u32,
    },
    Drop {
        instance_id: String,
        col: i64,
        row: i64,
    },
    Swap {
        index_a: u32,
        index_b: u32,
    },
    Message {
        text: String,
    },
}

/// Dispatches an inventory action to the matching `GameState` mutation.
/// `on_drop` mirrors the TS callback the shell uses to despawn/respawn the
/// dropped item's world mesh; it only fires when the item actually existed,
/// matching the TS existence check before `dropItem`.
pub fn process_inventory_action(
    action: &InventoryAction,
    game: &mut GameState,
    on_drop: &mut dyn FnMut(&str, i64, i64),
) {
    match action {
        InventoryAction::Equip { backpack_slot, .. } => {
            game.equip_from_backpack(*backpack_slot as usize);
        }
        InventoryAction::Unequip {
            equip_slot,
            backpack_slot,
        } => {
            game.unequip_to_backpack(*equip_slot, Some(*backpack_slot));
        }
        InventoryAction::Use { backpack_slot } => {
            let instance_id = {
                let backpack_items = game.entity_registry.backpack_items();
                backpack_items
                    .get(*backpack_slot as usize)
                    .map(|entity| entity.instance_id.clone())
            };
            if let Some(instance_id) = instance_id {
                game.use_consumable_from_registry(&instance_id);
            }
        }
        InventoryAction::Drop {
            instance_id,
            col,
            row,
        } => {
            if game.entity_registry.get_item(instance_id).is_some() {
                game.drop_item(instance_id, *col, *row);
                on_drop(instance_id, *col, *row);
            }
        }
        InventoryAction::Swap { index_a, index_b } => {
            game.entity_registry.swap_backpack_slots(*index_a, *index_b);
        }
        InventoryAction::Message { .. } => {}
    }
}

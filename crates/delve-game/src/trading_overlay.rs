//! Trading panel, ported from `hud/tradingOverlay.ts`. Opened only via
//! `DialogEvent::OpenShop` (see `dialog_overlay.rs`) — TS has no keyboard
//! binding that opens it either (confirmed via `game/inputSystem.ts`: no
//! `case` for trading in its key switch at all). Mouse-driven like the
//! inventory overlay (buy/sell rows are click targets, hover highlights the
//! row under the cursor); Escape is its only keyboard binding, matching
//! TS's own `_keyHandler`, which ignores every other key.
//!
//! TS builds one DOM panel and mutates it in place — there is no canvas
//! draw function to port 1:1, so the two-column layout here (shop stock
//! left, backpack right) is a fresh HUD-canvas design carrying the same
//! visual intent, the same pattern `dialog_overlay.rs`/`inventory_overlay.rs`
//! already established for DOM-only TS overlays. TS's mouse-clickable
//! "Close" button is not ported — every overlay in this port closes via
//! Escape only, and trading is not the first place to add a mouse-clickable
//! close target; the ESC hint line documents the binding instead.
//!
//! Stock is never decremented by buying — confirmed in `tradingOverlay.ts`:
//! `_rebuildContent` reads `npcDef.stock` fresh every time, and nothing in
//! `_handleBuy` mutates it. Buying is effectively unlimited-supply, ported
//! faithfully as such.

use crate::hud::{HUD_HEIGHT, HUD_WIDTH, HudState, IconCache, draw_item_icon};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::mouse::MouseState;
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::session::Session;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::entities::{BACKPACK_MAX_SLOTS, ItemLocation};
use delve_core::game_state::GameState;
use delve_core::items::ItemDatabase;
use delve_core::npcs::NpcDatabase;

/// TS: `this.npcDef.markup ?? 1.5`.
const DEFAULT_MARKUP: f64 = 1.5;

/// Which NPC's shop is open — resolved fresh against `NpcDb` every frame
/// (draw and input both re-read `npc_def.stock`/`.markup` live, matching
/// TS's own no-snapshot design) rather than caching a stock list here.
#[derive(Resource, Default)]
pub struct TradingOverlayState {
    pub npc_id: String,
}

// ---------------------------------------------------------------------------
// Pricing — ported 1:1 from `tradingOverlay.ts`'s `buyPrice`/`sellPrice`.
// ---------------------------------------------------------------------------

/// `Math.ceil(item.value * markup)`.
#[must_use]
pub fn buy_price(value: f64, markup: f64) -> i64 {
    (value * markup).ceil() as i64
}

/// `Math.floor(item.value * 0.5)`.
#[must_use]
pub fn sell_price(value: f64) -> i64 {
    (value * 0.5).floor() as i64
}

// ---------------------------------------------------------------------------
// Buy/sell — ported from `_handleBuy`/`_handleSell`, dispatched through
// `EntityRegistry::create_item`/`remove_item` and `player.gold` exactly as
// TS mutates `registry`/`gs.gold` directly (no new delve-core API needed —
// both are already public).
// ---------------------------------------------------------------------------

pub enum BuyOutcome {
    Bought { name: String, price: i64 },
    BackpackFull,
    NotEnoughGold,
    UnknownItem,
}

/// Backpack-full is checked before affordability, matching TS's exact
/// guard order in `_handleBuy` (`nextBackpackSlot` first, `gold < price`
/// second). The created item carries the item def's own `quality` and
/// `modifiers` (mapped to their ids) — TS: `registry.createItem(itemId,
/// def.quality, {kind:'backpack', slot}, def.modifiers.map(m => m.id))`.
pub fn buy_item(
    game: &mut GameState,
    items: &ItemDatabase,
    item_id: &str,
    markup: f64,
) -> BuyOutcome {
    let Some(def) = items.get_item(item_id) else {
        return BuyOutcome::UnknownItem;
    };
    let price = buy_price(def.value, markup);
    let Some(slot) = game.entity_registry.next_backpack_slot() else {
        return BuyOutcome::BackpackFull;
    };
    if game.player.gold < price {
        return BuyOutcome::NotEnoughGold;
    }
    let modifiers = def.modifiers.iter().map(|m| m.id.clone()).collect();
    game.entity_registry.create_item(
        item_id,
        def.quality,
        ItemLocation::Backpack { slot },
        modifiers,
    );
    game.player.gold -= price;
    BuyOutcome::Bought {
        name: def.name.clone(),
        price,
    }
}

pub enum SellOutcome {
    Sold { name: String, price: i64 },
    NotSellable,
    UnknownItem,
}

/// TS: `gs.gold += price; gs.entityRegistry.removeItem(instanceId);` — a
/// zero-value item (`price <= 0`) has no Sell button in TS at all
/// (`canSell = price > 0`); this mirrors that as a no-op guard rather than
/// crediting zero gold for a removed item.
pub fn sell_item(game: &mut GameState, items: &ItemDatabase, instance_id: &str) -> SellOutcome {
    let Some(entity) = game.entity_registry.get_item(instance_id) else {
        return SellOutcome::UnknownItem;
    };
    let Some(def) = items.get_item(&entity.item_id) else {
        return SellOutcome::UnknownItem;
    };
    let price = sell_price(def.value);
    if price <= 0 {
        return SellOutcome::NotSellable;
    }
    let name = def.name.clone();
    game.entity_registry.remove_item(instance_id);
    game.player.gold += price;
    SellOutcome::Sold { name, price }
}

/// Backpack slots holding an item, in slot order — the backpack column
/// lists occupied slots only (nothing to sell from an empty one).
fn occupied_backpack_slots(game: &GameState) -> Vec<u32> {
    (0..BACKPACK_MAX_SLOTS)
        .filter(|&slot| game.entity_registry.backpack_item_at(slot).is_some())
        .collect()
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

const PANEL_W: i32 = 580;
const PANEL_H: i32 = 320;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const PANEL_Y: i32 = (HUD_HEIGHT as i32 - PANEL_H) / 2;
const COLUMN_GAP: i32 = 8;
const COLUMN_W: i32 = (PANEL_W - COLUMN_GAP * 3) / 2;
const LEFT_COLUMN_X: i32 = PANEL_X + COLUMN_GAP;
const RIGHT_COLUMN_X: i32 = LEFT_COLUMN_X + COLUMN_W + COLUMN_GAP;
const HEADER_Y: i32 = PANEL_Y + 44;
const ROWS_START_Y: i32 = HEADER_Y + 16;
const ROW_H: i32 = 20;
const ROW_GAP: i32 = 2;
/// The row's own drawn height (`ROW_H - ROW_GAP`) doubles as the icon's slot
/// edge length — `draw_item_icon` fits the sprite to whatever size it's
/// given, so the icon fills the row top-to-bottom with no separate vertical
/// centering needed.
const ROW_ICON_SIZE: i32 = ROW_H - ROW_GAP;
const ROW_ICON_TEXT_GAP: i32 = 4;
const BOTTOM_BAR_Y: i32 = PANEL_Y + PANEL_H - 22;

const BACKDROP: Rgba = Rgba::translucent(0, 0, 0, 0.85);
const PANEL_BG: Rgba = Rgba::translucent(10, 8, 12, 0.75);
const PANEL_BORDER: Rgba = Rgba::opaque(0x2a, 0x22, 0x30);
const TITLE_TEXT: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const HEADER_TEXT: Rgba = Rgba::opaque(0x88, 0x88, 0x88);
const ROW_TEXT: Rgba = Rgba::opaque(0xcc, 0xcc, 0xcc);
const ROW_TEXT_DIM: Rgba = Rgba::opaque(0x55, 0x55, 0x55);
const ROW_HOVER_FILL: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.1);
const GOLD_COIN: Rgba = Rgba::opaque(0xda, 0xa5, 0x20);
const WARNING_TEXT: Rgba = Rgba::opaque(0xcc, 0x44, 0x44);
const HINT_TEXT: Rgba = Rgba::opaque(0x66, 0x66, 0x66);
const ICON_FALLBACK: Rgba = Rgba::opaque(0x88, 0x88, 0x88);

#[derive(Clone, Copy, PartialEq, Eq)]
enum TradeSide {
    Shop,
    Backpack,
}

fn row_rect(side: TradeSide, index: usize) -> (i32, i32, i32, i32) {
    let x = match side {
        TradeSide::Shop => LEFT_COLUMN_X,
        TradeSide::Backpack => RIGHT_COLUMN_X,
    };
    let y = ROWS_START_Y + index as i32 * ROW_H;
    (x, y, COLUMN_W, ROW_H - ROW_GAP)
}

fn in_rect(hud_x: f32, hud_y: f32, x: i32, y: i32, w: i32, h: i32) -> bool {
    hud_x >= x as f32 && hud_x < (x + w) as f32 && hud_y >= y as f32 && hud_y < (y + h) as f32
}

fn hit_test(
    hud_x: f32,
    hud_y: f32,
    shop_count: usize,
    backpack_count: usize,
) -> Option<(TradeSide, usize)> {
    for index in 0..shop_count {
        let (x, y, w, h) = row_rect(TradeSide::Shop, index);
        if in_rect(hud_x, hud_y, x, y, w, h) {
            return Some((TradeSide::Shop, index));
        }
    }
    for index in 0..backpack_count {
        let (x, y, w, h) = row_rect(TradeSide::Backpack, index);
        if in_rect(hud_x, hud_y, x, y, w, h) {
            return Some((TradeSide::Backpack, index));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[derive(SystemParam)]
pub struct TradingInputEffects<'w> {
    state: Res<'w, TradingOverlayState>,
    session: ResMut<'w, Session>,
    items: Res<'w, crate::ground_items::ItemDb>,
    npc_db: Res<'w, crate::npcs::NpcDb>,
    hud: ResMut<'w, HudState>,
}

/// Never opened by a key — only `dialog_overlay::apply_dialog_events`'s
/// `OpenShop` arm sets `ActiveOverlay::Trading`. Escape closes; a left
/// click on a shop row buys, on a backpack row sells.
pub fn trading_overlay_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<MouseState>,
    mut overlay: ResMut<ActiveOverlay>,
    mut effects: TradingInputEffects,
) {
    if *overlay != ActiveOverlay::Trading {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        *overlay = ActiveOverlay::None;
        return;
    }
    if !mouse.left_just_pressed || !mouse.in_window {
        return;
    }

    let Some(npc_def) = effects.npc_db.0.get_npc(&effects.state.npc_id).cloned() else {
        return;
    };
    let stock = npc_def.stock.clone().unwrap_or_default();
    let markup = npc_def.markup.unwrap_or(DEFAULT_MARKUP);
    let occupied = occupied_backpack_slots(&effects.session.game);

    let Some((side, index)) = hit_test(mouse.hud_x, mouse.hud_y, stock.len(), occupied.len())
    else {
        return;
    };

    match side {
        TradeSide::Shop => {
            let Some(item_id) = stock.get(index) else {
                return;
            };
            match buy_item(&mut effects.session.game, &effects.items.0, item_id, markup) {
                BuyOutcome::Bought { name, price } => {
                    effects
                        .hud
                        .show_message(&format!("Bought {name} for {price}g"));
                }
                BuyOutcome::BackpackFull => effects.hud.show_message("Backpack is full!"),
                BuyOutcome::NotEnoughGold => effects.hud.show_message("Not enough gold!"),
                BuyOutcome::UnknownItem => {
                    warn!(
                        "shop stock for '{}' references unknown item '{item_id}'",
                        effects.state.npc_id
                    );
                }
            }
        }
        TradeSide::Backpack => {
            let Some(&slot) = occupied.get(index) else {
                return;
            };
            let Some(instance_id) = effects
                .session
                .game
                .entity_registry
                .backpack_item_at(slot)
                .map(|entity| entity.instance_id.clone())
            else {
                return;
            };
            match sell_item(&mut effects.session.game, &effects.items.0, &instance_id) {
                SellOutcome::Sold { name, price } => {
                    effects
                        .hud
                        .show_message(&format!("Sold {name} for {price}g"));
                }
                SellOutcome::NotSellable => {}
                SellOutcome::UnknownItem => {
                    warn!("sell target '{instance_id}' missing from entity registry");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

fn draw_row(canvas: &mut PixelCanvas, x: i32, y: i32, w: i32, h: i32, hovered: bool) {
    if hovered {
        canvas.fill_rect(x, y, w, h, ROW_HOVER_FILL);
    }
}

pub fn draw_trading_overlay(
    canvas: &mut PixelCanvas,
    trading_state: &TradingOverlayState,
    npc_db: &NpcDatabase,
    game: &GameState,
    items: &ItemDatabase,
    mouse: &MouseState,
    icons: &mut IconCache,
) {
    let Some(npc_def) = npc_db.get_npc(&trading_state.npc_id) else {
        return;
    };
    let stock = npc_def.stock.as_deref().unwrap_or(&[]);
    let markup = npc_def.markup.unwrap_or(DEFAULT_MARKUP);
    let occupied = occupied_backpack_slots(game);
    let backpack_full = game.entity_registry.next_backpack_slot().is_none();

    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, BACKDROP);
    canvas.fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BG);
    canvas.stroke_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BORDER);

    let title = npc_def.name.to_uppercase();
    let title_w = measure_pixel_text(&title, 2);
    draw_pixel_text(
        canvas,
        &title,
        PANEL_X + (PANEL_W - title_w) / 2,
        PANEL_Y + 14,
        TITLE_TEXT,
        2,
    );
    canvas.stroke_line(
        PANEL_X + 8,
        PANEL_Y + 32,
        PANEL_X + PANEL_W - 8,
        PANEL_Y + 32,
        PANEL_BORDER,
    );

    draw_pixel_text(
        canvas,
        "SHOP STOCK",
        LEFT_COLUMN_X,
        HEADER_Y,
        HEADER_TEXT,
        1,
    );
    draw_pixel_text(canvas, "BACKPACK", RIGHT_COLUMN_X, HEADER_Y, HEADER_TEXT, 1);

    for (index, item_id) in stock.iter().enumerate() {
        let Some(def) = items.get_item(item_id) else {
            continue;
        };
        let price = buy_price(def.value, markup);
        let can_buy = game.player.gold >= price && !backpack_full;
        let (x, y, w, h) = row_rect(TradeSide::Shop, index);
        draw_row(
            canvas,
            x,
            y,
            w,
            h,
            in_rect(mouse.hud_x, mouse.hud_y, x, y, w, h),
        );
        draw_item_icon(
            canvas,
            icons,
            items,
            item_id,
            (x, y, ROW_ICON_SIZE),
            ICON_FALLBACK,
        );
        let text_color = if can_buy { ROW_TEXT } else { ROW_TEXT_DIM };
        draw_pixel_text(
            canvas,
            &def.name.to_uppercase(),
            x + ROW_ICON_SIZE + ROW_ICON_TEXT_GAP,
            y + 4,
            text_color,
            1,
        );
        let price_text = format!("{price}G");
        let price_w = measure_pixel_text(&price_text, 1);
        draw_pixel_text(
            canvas,
            &price_text,
            x + w - price_w - 4,
            y + 4,
            if can_buy { GOLD_COIN } else { ROW_TEXT_DIM },
            1,
        );
    }

    for (index, &slot) in occupied.iter().enumerate() {
        let Some(entity) = game.entity_registry.backpack_item_at(slot) else {
            continue;
        };
        let Some(def) = items.get_item(&entity.item_id) else {
            continue;
        };
        let price = sell_price(def.value);
        let can_sell = price > 0;
        let (x, y, w, h) = row_rect(TradeSide::Backpack, index);
        draw_row(
            canvas,
            x,
            y,
            w,
            h,
            in_rect(mouse.hud_x, mouse.hud_y, x, y, w, h),
        );
        draw_item_icon(
            canvas,
            icons,
            items,
            &entity.item_id,
            (x, y, ROW_ICON_SIZE),
            ICON_FALLBACK,
        );
        let text_color = if can_sell { ROW_TEXT } else { ROW_TEXT_DIM };
        draw_pixel_text(
            canvas,
            &def.name.to_uppercase(),
            x + ROW_ICON_SIZE + ROW_ICON_TEXT_GAP,
            y + 4,
            text_color,
            1,
        );
        if can_sell {
            let price_text = format!("{price}G");
            let price_w = measure_pixel_text(&price_text, 1);
            draw_pixel_text(
                canvas,
                &price_text,
                x + w - price_w - 4,
                y + 4,
                GOLD_COIN,
                1,
            );
        }
    }

    draw_pixel_text(
        canvas,
        &format!("GOLD: {}", game.player.gold),
        PANEL_X + 8,
        BOTTOM_BAR_Y,
        GOLD_COIN,
        1,
    );
    if backpack_full {
        let message = "BACKPACK IS FULL";
        let message_w = measure_pixel_text(message, 1);
        draw_pixel_text(
            canvas,
            message,
            PANEL_X + (PANEL_W - message_w) / 2,
            BOTTOM_BAR_Y,
            WARNING_TEXT,
            1,
        );
    }
    let hint = "CLICK TO BUY/SELL   ESC: CLOSE";
    let hint_w = measure_pixel_text(hint, 1);
    draw_pixel_text(
        canvas,
        hint,
        PANEL_X + PANEL_W - hint_w - 8,
        BOTTOM_BAR_Y,
        HINT_TEXT,
        1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use delve_core::game_state::GameStateDeps;
    use delve_core::items::ItemQuality;

    const ITEMS_JSON: &str = include_str!("../../../assets/data/items.json");

    fn items() -> ItemDatabase {
        ItemDatabase::from_json(ITEMS_JSON).expect("shipped items.json parses")
    }

    fn game() -> GameState {
        GameState::new(
            &[],
            None,
            "test_level",
            None,
            GameStateDeps::default(),
            &mut || 0.0,
        )
    }

    // -- buy_price, ported from tradingOverlay.test.ts's `describe('buyPrice', ...)` --

    #[test]
    fn buy_price_applies_markup_and_rounds_up() {
        assert_eq!(buy_price(10.0, 1.5), 15);
    }

    #[test]
    fn buy_price_rounds_up_fractional_prices() {
        assert_eq!(buy_price(7.0, 1.5), 11);
    }

    #[test]
    fn buy_price_works_with_markup_of_one() {
        assert_eq!(buy_price(10.0, 1.0), 10);
    }

    #[test]
    fn buy_price_handles_zero_value_items() {
        assert_eq!(buy_price(0.0, 1.5), 0);
    }

    #[test]
    fn buy_price_handles_high_markup() {
        assert_eq!(buy_price(10.0, 3.0), 30);
    }

    // -- sell_price, ported from tradingOverlay.test.ts's `describe('sellPrice', ...)` --

    #[test]
    fn sell_price_halves_value_and_rounds_down() {
        assert_eq!(sell_price(10.0), 5);
    }

    #[test]
    fn sell_price_rounds_down_fractional_prices() {
        assert_eq!(sell_price(7.0), 3);
    }

    #[test]
    fn sell_price_returns_zero_for_zero_value_items() {
        assert_eq!(sell_price(0.0), 0);
    }

    #[test]
    fn sell_price_returns_zero_for_value_one_items() {
        assert_eq!(sell_price(1.0), 0);
    }

    // -- buy_item / sell_item transition table --

    #[test]
    fn buy_item_deducts_gold_and_creates_a_backpack_item() {
        let mut game = game();
        game.player.gold = 100;
        let outcome = buy_item(&mut game, &items(), "health_potion_small", 1.5);
        let BuyOutcome::Bought { price, .. } = outcome else {
            panic!("expected Bought");
        };
        assert_eq!(price, 23); // ceil(15 * 1.5)
        assert_eq!(game.player.gold, 77);
        assert_eq!(game.entity_registry.next_backpack_slot(), Some(1));
    }

    #[test]
    fn buy_item_carries_the_item_defs_quality_and_modifiers() {
        let mut game = game();
        game.player.gold = 1000;
        buy_item(&mut game, &items(), "sword_flamebrand", 1.5);
        let created = game
            .entity_registry
            .backpack_item_at(0)
            .expect("item landed in slot 0");
        assert_eq!(created.quality, ItemQuality::Enchanted);
        assert_eq!(created.modifiers, vec!["fire_damage".to_string()]);
    }

    #[test]
    fn buy_item_fails_when_gold_is_insufficient() {
        let mut game = game();
        game.player.gold = 0;
        let outcome = buy_item(&mut game, &items(), "health_potion_small", 1.5);
        assert!(matches!(outcome, BuyOutcome::NotEnoughGold));
        assert_eq!(game.entity_registry.next_backpack_slot(), Some(0));
    }

    #[test]
    fn buy_item_checks_backpack_full_before_gold() {
        let mut game = game();
        game.player.gold = 10_000;
        for slot in 0..BACKPACK_MAX_SLOTS {
            game.entity_registry.create_item(
                "health_potion_small",
                ItemQuality::Common,
                ItemLocation::Backpack { slot },
                Vec::new(),
            );
        }
        let outcome = buy_item(&mut game, &items(), "health_potion_small", 1.5);
        assert!(matches!(outcome, BuyOutcome::BackpackFull));
    }

    #[test]
    fn sell_item_adds_gold_and_removes_the_item() {
        let mut game = game();
        let entity = game.entity_registry.create_item(
            "health_potion_small",
            ItemQuality::Common,
            ItemLocation::Backpack { slot: 0 },
            Vec::new(),
        );
        game.player.gold = 0;
        let outcome = sell_item(&mut game, &items(), &entity.instance_id);
        let SellOutcome::Sold { price, .. } = outcome else {
            panic!("expected Sold");
        };
        assert_eq!(price, 7); // floor(15 * 0.5)
        assert_eq!(game.player.gold, 7);
        assert!(game.entity_registry.get_item(&entity.instance_id).is_none());
    }

    #[test]
    fn occupied_backpack_slots_lists_only_filled_slots_in_order() {
        let mut game = game();
        game.entity_registry.create_item(
            "health_potion_small",
            ItemQuality::Common,
            ItemLocation::Backpack { slot: 3 },
            Vec::new(),
        );
        game.entity_registry.create_item(
            "health_potion_small",
            ItemQuality::Common,
            ItemLocation::Backpack { slot: 0 },
            Vec::new(),
        );
        assert_eq!(occupied_backpack_slots(&game), vec![0, 3]);
    }
}

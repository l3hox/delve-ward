//! Full-screen interactive inventory overlay (`KeyI`), distinct from the
//! always-visible mini panel (`hud.rs::draw_inventory_panel`) — ported from
//! `hud/inventoryOverlay.ts` and its mouse wiring in `hud/hudCanvas.ts`.
//!
//! Hit-test geometry and the drag-and-drop transition table have zero TS
//! test coverage (`PHASE4-PLAN.md` risk #3) — every branch below is backed
//! by its own unit test rather than relying on a ported fixture, since
//! there is no upstream spec to check against.

use crate::equip_layout::{EQUIP_SLOTS, equip_slot_index, subtype_to_equip_slot};
use crate::ground_items::{GroundItemRender, ItemDb};
use crate::hud::{HUD_HEIGHT, HUD_WIDTH, HudState, IconCache, IconSampling, draw_item_icon};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::item_tooltip::draw_item_tooltip;
use crate::mouse::MouseState;
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::player::Player;
use crate::session::Session;
use crate::transition::Transition;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::entities::EquipSlot;
use delve_core::game_state::GameState;
use delve_core::items::{ItemDatabase, ItemSubtype, ItemType};
use delve_core::player_controller::InventoryAction;
use std::collections::HashSet;

const SLOT_SIZE: i32 = 28;
const SLOT_GAP: i32 = 4;
const EQUIP_COLS: i32 = 5;
const BACKPACK_COLS: i32 = 4;
const BACKPACK_SLOTS: usize = 12;
const PANEL_W: i32 = 460;
const PANEL_H: i32 = 310;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const PANEL_Y: i32 = (HUD_HEIGHT as i32 - PANEL_H) / 2;

const BACKDROP: Rgba = Rgba::translucent(0, 0, 0, 0.85);
const PANEL_BG: Rgba = Rgba::translucent(10, 8, 12, 0.75);
const PANEL_BORDER: Rgba = Rgba::opaque(0x2a, 0x22, 0x30);
const SLOT_BG: Rgba = Rgba::opaque(0x1a, 0x16, 0x20);
const SLOT_BORDER: Rgba = Rgba::opaque(0x3a, 0x30, 0x40);
const TITLE_TEXT: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const GOLD_COIN: Rgba = Rgba::opaque(0xda, 0xa5, 0x20);
const CURSOR_FILL: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.3);
const CURSOR_BORDER: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const VALID_DROP_FILL: Rgba = Rgba::translucent(0x44, 0xc8, 0x44, 0.25);
const VALID_DROP_BORDER: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const BACKPACK_DROP_FILL: Rgba = Rgba::translucent(0x44, 0xc8, 0x44, 0.15);
const BACKPACK_DROP_BORDER: Rgba = Rgba::translucent(0x44, 0xcc, 0x44, 0.4);
const HINT_TEXT: Rgba = Rgba::opaque(0x66, 0x66, 0x66);
const ICON_FALLBACK: Rgba = Rgba::opaque(0x88, 0x88, 0x88);

/// Empty equipment slots draw a translucent paperdoll ghost icon —
/// `inventoryOverlay.ts`'s own `pad`/`globalAlpha` (its mini-panel sibling,
/// `hud.rs::draw_inventory_panel`, uses a tighter 3px pad for the same
/// alpha, ported from `inventoryPanel.ts`'s separate constant).
const PAPERDOLL_PAD: i32 = 4;
const PAPERDOLL_ALPHA: f32 = 0.3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Equipment,
    Backpack,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CursorPos {
    pub section: Section,
    pub index: usize,
}

struct DragState {
    source: CursorPos,
    item_id: String,
    hud_x: f32,
    hud_y: f32,
    valid_equip_slots: HashSet<usize>,
}

#[derive(Resource)]
pub struct InventoryOverlayState {
    cursor: CursorPos,
    drag: Option<DragState>,
}

impl Default for InventoryOverlayState {
    fn default() -> Self {
        Self {
            cursor: CursorPos {
                section: Section::Equipment,
                index: 0,
            },
            drag: None,
        }
    }
}

impl InventoryOverlayState {
    /// Resets the cursor to the first equipment slot and clears any drag —
    /// ported from TS's `toggle()`'s open branch.
    fn reset(&mut self) {
        self.cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        self.drag = None;
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

fn equip_origin() -> (i32, i32) {
    let x = PANEL_X + (PANEL_W - (EQUIP_COLS * (SLOT_SIZE + SLOT_GAP) - SLOT_GAP)) / 2;
    (x, PANEL_Y + 44)
}

fn backpack_origin() -> (i32, i32) {
    let (_, equip_y) = equip_origin();
    let sep_y = equip_y + 2 * (SLOT_SIZE + SLOT_GAP) + 6;
    let x = PANEL_X + (PANEL_W - (BACKPACK_COLS * (SLOT_SIZE + SLOT_GAP) - SLOT_GAP)) / 2;
    (x, sep_y + 10)
}

fn slot_origin(pos: CursorPos) -> (i32, i32) {
    match pos.section {
        Section::Equipment => {
            let (ox, oy) = equip_origin();
            let col = pos.index as i32 % EQUIP_COLS;
            let row = pos.index as i32 / EQUIP_COLS;
            (
                ox + col * (SLOT_SIZE + SLOT_GAP),
                oy + row * (SLOT_SIZE + SLOT_GAP),
            )
        }
        Section::Backpack => {
            let (ox, oy) = backpack_origin();
            let col = pos.index as i32 % BACKPACK_COLS;
            let row = pos.index as i32 / BACKPACK_COLS;
            (
                ox + col * (SLOT_SIZE + SLOT_GAP),
                oy + row * (SLOT_SIZE + SLOT_GAP),
            )
        }
    }
}

/// Inclusive-min/exclusive-max hit test over both grids, equipment first —
/// ported from TS's `hitTest`.
fn hit_test(hud_x: f32, hud_y: f32) -> Option<CursorPos> {
    for index in 0..EQUIP_SLOTS.len() {
        let pos = CursorPos {
            section: Section::Equipment,
            index,
        };
        let (sx, sy) = slot_origin(pos);
        if in_slot(hud_x, hud_y, sx, sy) {
            return Some(pos);
        }
    }
    for index in 0..BACKPACK_SLOTS {
        let pos = CursorPos {
            section: Section::Backpack,
            index,
        };
        let (sx, sy) = slot_origin(pos);
        if in_slot(hud_x, hud_y, sx, sy) {
            return Some(pos);
        }
    }
    None
}

fn in_slot(hud_x: f32, hud_y: f32, sx: i32, sy: i32) -> bool {
    hud_x >= sx as f32
        && hud_x < (sx + SLOT_SIZE) as f32
        && hud_y >= sy as f32
        && hud_y < (sy + SLOT_SIZE) as f32
}

// ---------------------------------------------------------------------------
// Cursor navigation — ported line-for-line from TS's _moveLeft/_moveRight/
// _moveUp/_moveDown.
// ---------------------------------------------------------------------------

fn move_left(cursor: CursorPos) -> CursorPos {
    match cursor.section {
        Section::Equipment => {
            let (row, col) = (
                cursor.index / EQUIP_COLS as usize,
                cursor.index % EQUIP_COLS as usize,
            );
            if col > 0 {
                CursorPos {
                    section: Section::Equipment,
                    index: row * EQUIP_COLS as usize + col - 1,
                }
            } else {
                cursor
            }
        }
        Section::Backpack => {
            let (row, col) = (
                cursor.index / BACKPACK_COLS as usize,
                cursor.index % BACKPACK_COLS as usize,
            );
            if col > 0 {
                CursorPos {
                    section: Section::Backpack,
                    index: row * BACKPACK_COLS as usize + col - 1,
                }
            } else {
                cursor
            }
        }
    }
}

fn move_right(cursor: CursorPos) -> CursorPos {
    match cursor.section {
        Section::Equipment => {
            let (row, col) = (
                cursor.index / EQUIP_COLS as usize,
                cursor.index % EQUIP_COLS as usize,
            );
            if col < EQUIP_COLS as usize - 1 {
                CursorPos {
                    section: Section::Equipment,
                    index: row * EQUIP_COLS as usize + col + 1,
                }
            } else {
                cursor
            }
        }
        Section::Backpack => {
            let (row, col) = (
                cursor.index / BACKPACK_COLS as usize,
                cursor.index % BACKPACK_COLS as usize,
            );
            if col < BACKPACK_COLS as usize - 1 {
                CursorPos {
                    section: Section::Backpack,
                    index: row * BACKPACK_COLS as usize + col + 1,
                }
            } else {
                cursor
            }
        }
    }
}

fn move_up(cursor: CursorPos) -> CursorPos {
    match cursor.section {
        Section::Equipment => {
            let (row, col) = (
                cursor.index / EQUIP_COLS as usize,
                cursor.index % EQUIP_COLS as usize,
            );
            if row > 0 {
                CursorPos {
                    section: Section::Equipment,
                    index: col,
                }
            } else {
                cursor
            }
        }
        Section::Backpack => {
            let (row, col) = (
                cursor.index / BACKPACK_COLS as usize,
                cursor.index % BACKPACK_COLS as usize,
            );
            if row > 0 {
                CursorPos {
                    section: Section::Backpack,
                    index: (row - 1) * BACKPACK_COLS as usize + col,
                }
            } else {
                let equip_col = col.min(EQUIP_COLS as usize - 1);
                CursorPos {
                    section: Section::Equipment,
                    index: EQUIP_COLS as usize + equip_col,
                }
            }
        }
    }
}

fn move_down(cursor: CursorPos) -> CursorPos {
    match cursor.section {
        Section::Equipment => {
            let (row, col) = (
                cursor.index / EQUIP_COLS as usize,
                cursor.index % EQUIP_COLS as usize,
            );
            if row < 1 {
                CursorPos {
                    section: Section::Equipment,
                    index: EQUIP_COLS as usize + col,
                }
            } else {
                let backpack_col = col.min(BACKPACK_COLS as usize - 1);
                CursorPos {
                    section: Section::Backpack,
                    index: backpack_col,
                }
            }
        }
        Section::Backpack => {
            let (row, col) = (
                cursor.index / BACKPACK_COLS as usize,
                cursor.index % BACKPACK_COLS as usize,
            );
            if row < 2 {
                CursorPos {
                    section: Section::Backpack,
                    index: (row + 1) * BACKPACK_COLS as usize + col,
                }
            } else {
                cursor
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Action generation — ported from TS's _handleEnter/_handleDrop.
// ---------------------------------------------------------------------------

/// Sorted-list position of `instance_id` in the backpack — the index
/// `InventoryAction::Equip`/`Use` expect (see `player_controller.rs`'s
/// `process_inventory_action`), which is NOT the same as a slot number
/// whenever the backpack has gaps.
fn sorted_backpack_position(game: &GameState, instance_id: &str) -> Option<u32> {
    game.entity_registry
        .backpack_items()
        .iter()
        .position(|entity| entity.instance_id == instance_id)
        .map(|position| position as u32)
}

/// `pub(crate)`: reused by `hud.rs`'s mini inventory panel, whose mouse
/// interactions resolve to the exact same actions over different geometry.
pub(crate) fn handle_enter(
    cursor: CursorPos,
    game: &GameState,
    items: &ItemDatabase,
) -> Option<InventoryAction> {
    match cursor.section {
        Section::Equipment => {
            let slot = EQUIP_SLOTS[cursor.index];
            game.entity_registry.get_equipped(slot)?;
            let Some(free_slot) = game.entity_registry.next_backpack_slot() else {
                return Some(InventoryAction::Message {
                    text: "Backpack is full".to_string(),
                });
            };
            Some(InventoryAction::Unequip {
                equip_slot: slot,
                backpack_slot: free_slot,
            })
        }
        Section::Backpack => {
            let entity = game.entity_registry.backpack_item_at(cursor.index as u32)?;
            let def = items.get_item(&entity.item_id)?;
            let position = sorted_backpack_position(game, &entity.instance_id)?;
            if def.item_type == ItemType::Consumable {
                Some(InventoryAction::Use {
                    backpack_slot: position,
                })
            } else {
                Some(InventoryAction::Equip {
                    backpack_slot: position,
                    equip_slot: subtype_to_equip_slot(def.subtype, game),
                })
            }
        }
    }
}

pub(crate) fn handle_drop(
    cursor: CursorPos,
    game: &GameState,
    player_col: i64,
    player_row: i64,
) -> Option<InventoryAction> {
    let instance_id = match cursor.section {
        Section::Equipment => game
            .entity_registry
            .get_equipped(EQUIP_SLOTS[cursor.index])?
            .instance_id
            .clone(),
        Section::Backpack => game
            .entity_registry
            .backpack_item_at(cursor.index as u32)?
            .instance_id
            .clone(),
    };
    Some(InventoryAction::Drop {
        instance_id,
        col: player_col,
        row: player_row,
    })
}

// ---------------------------------------------------------------------------
// Drag state machine
// ---------------------------------------------------------------------------

pub(crate) fn valid_equip_slots_for_drag(
    item_type: ItemType,
    subtype: ItemSubtype,
    source: Section,
    game: &GameState,
) -> HashSet<usize> {
    let mut slots = HashSet::new();
    if item_type == ItemType::Consumable || source == Section::Equipment {
        return slots;
    }
    if let Some(index) = equip_slot_index(subtype_to_equip_slot(subtype, game)) {
        slots.insert(index);
    }
    if subtype == ItemSubtype::Ring {
        if let Some(index) = equip_slot_index(EquipSlot::Ring1) {
            slots.insert(index);
        }
        if let Some(index) = equip_slot_index(EquipSlot::Ring2) {
            slots.insert(index);
        }
    }
    slots
}

fn drag_start(
    state: &mut InventoryOverlayState,
    hud_x: f32,
    hud_y: f32,
    game: &GameState,
    items: &ItemDatabase,
) {
    let Some(pos) = hit_test(hud_x, hud_y) else {
        return;
    };
    let entity = match pos.section {
        Section::Equipment => game.entity_registry.get_equipped(EQUIP_SLOTS[pos.index]),
        Section::Backpack => game.entity_registry.backpack_item_at(pos.index as u32),
    };
    let Some(entity) = entity else {
        return;
    };
    let Some(def) = items.get_item(&entity.item_id) else {
        return;
    };
    state.drag = Some(DragState {
        source: pos,
        item_id: entity.item_id.clone(),
        hud_x,
        hud_y,
        valid_equip_slots: valid_equip_slots_for_drag(
            def.item_type,
            def.subtype,
            pos.section,
            game,
        ),
    });
}

/// The equip/unequip/swap transition table this slice is built around —
/// ported from TS's `handleDragEnd`. Every arm is covered by its own test
/// in the `tests` module below, including the ring1-occupied/ring2-free
/// case `PHASE4-PLAN.md` singled out. `pub(crate)`: reused by the mini
/// panel's own drag-and-drop, which resolves through the identical table.
pub(crate) fn resolve_drag(
    source: CursorPos,
    target: CursorPos,
    game: &GameState,
    items: &ItemDatabase,
) -> Option<InventoryAction> {
    if source == target {
        return None;
    }
    match (source.section, target.section) {
        (Section::Backpack, Section::Equipment) => {
            let entity = game.entity_registry.backpack_item_at(source.index as u32)?;
            let def = items.get_item(&entity.item_id)?;
            if def.item_type == ItemType::Consumable {
                return None;
            }
            let target_slot = EQUIP_SLOTS[target.index];
            let correct_slot = subtype_to_equip_slot(def.subtype, game);
            let is_ring = def.subtype == ItemSubtype::Ring
                && (target_slot == EquipSlot::Ring1 || target_slot == EquipSlot::Ring2);
            if target_slot != correct_slot && !is_ring {
                return None;
            }
            let position = sorted_backpack_position(game, &entity.instance_id)?;
            Some(InventoryAction::Equip {
                backpack_slot: position,
                equip_slot: target_slot,
            })
        }
        (Section::Equipment, Section::Backpack) => {
            let slot = EQUIP_SLOTS[source.index];
            game.entity_registry.get_equipped(slot)?;
            if game
                .entity_registry
                .backpack_item_at(target.index as u32)
                .is_some()
            {
                return None;
            }
            Some(InventoryAction::Unequip {
                equip_slot: slot,
                backpack_slot: target.index as u32,
            })
        }
        (Section::Backpack, Section::Backpack) => Some(InventoryAction::Swap {
            index_a: source.index as u32,
            index_b: target.index as u32,
        }),
        (Section::Equipment, Section::Equipment) => None,
    }
}

fn handle_mouse_click(
    hud_pos: (f32, f32),
    is_drop: bool,
    state: &mut InventoryOverlayState,
    game: &GameState,
    items: &ItemDatabase,
    player_cell: (i64, i64),
) -> Option<InventoryAction> {
    let pos = hit_test(hud_pos.0, hud_pos.1)?;
    state.cursor = pos;
    if is_drop {
        handle_drop(pos, game, player_cell.0, player_cell.1)
    } else {
        handle_enter(pos, game, items)
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Dependencies the input system needs beyond the keyboard/mouse/overlay
/// flag, bundled to stay under the argument-count lint.
#[derive(SystemParam)]
pub struct InventoryInputEffects<'w, 's> {
    state: ResMut<'w, InventoryOverlayState>,
    session: ResMut<'w, Session>,
    items: Res<'w, ItemDb>,
    players: Query<'w, 's, &'static Player>,
    item_render: GroundItemRender<'w, 's>,
    hud: ResMut<'w, HudState>,
}

/// Opens on `KeyI` from the dungeon, closes on `KeyI`/`Escape` while open;
/// while open, routes keyboard and mouse input to action generation and
/// dispatches the result through [`crate::session::apply_inventory_action`]
/// — ported from `inputSystem.ts`'s `KeyI` case plus the `inventoryOverlay`
/// branch of its `keydownHandler`, and `hudCanvas.ts`'s mouse listeners.
pub fn inventory_overlay_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<MouseState>,
    mut overlay: ResMut<ActiveOverlay>,
    transition: Res<Transition>,
    mut effects: InventoryInputEffects,
) {
    if *overlay != ActiveOverlay::Inventory {
        if transition.is_active() || *overlay != ActiveOverlay::None {
            return;
        }
        if keys.just_pressed(KeyCode::KeyI) {
            *overlay = ActiveOverlay::Inventory;
            effects.state.reset();
        }
        return;
    }

    if keys.just_pressed(KeyCode::KeyI) || keys.just_pressed(KeyCode::Escape) {
        *overlay = ActiveOverlay::None;
        effects.state.drag = None;
        return;
    }

    let Ok(player) = effects.players.single() else {
        return;
    };
    let player_state = player.grid_state();
    let (player_col, player_row) = (i64::from(player_state.col), i64::from(player_state.row));

    let mut action = None;
    {
        let game = &effects.session.game;
        let items = &effects.items.0;

        if keys.just_pressed(KeyCode::ArrowLeft) {
            effects.state.cursor = move_left(effects.state.cursor);
        }
        if keys.just_pressed(KeyCode::ArrowRight) {
            effects.state.cursor = move_right(effects.state.cursor);
        }
        if keys.just_pressed(KeyCode::ArrowUp) {
            effects.state.cursor = move_up(effects.state.cursor);
        }
        if keys.just_pressed(KeyCode::ArrowDown) {
            effects.state.cursor = move_down(effects.state.cursor);
        }
        if keys.just_pressed(KeyCode::Enter) {
            action = handle_enter(effects.state.cursor, game, items);
        }
        if action.is_none() && keys.just_pressed(KeyCode::KeyD) {
            action = handle_drop(effects.state.cursor, game, player_col, player_row);
        }

        if mouse.in_window {
            if effects.state.drag.is_some() {
                if let Some(drag) = effects.state.drag.as_mut() {
                    drag.hud_x = mouse.hud_x;
                    drag.hud_y = mouse.hud_y;
                }
            } else if let Some(pos) = hit_test(mouse.hud_x, mouse.hud_y) {
                effects.state.cursor = pos;
            }

            if mouse.left_just_pressed && effects.state.drag.is_none() {
                drag_start(&mut effects.state, mouse.hud_x, mouse.hud_y, game, items);
            }

            if mouse.left_just_released
                && let Some(drag) = effects.state.drag.take()
                && let Some(target) = hit_test(mouse.hud_x, mouse.hud_y)
                && action.is_none()
            {
                action = resolve_drag(drag.source, target, game, items);
            }

            if action.is_none() && mouse.left_double_clicked {
                action = handle_mouse_click(
                    (mouse.hud_x, mouse.hud_y),
                    false,
                    &mut effects.state,
                    game,
                    items,
                    (player_col, player_row),
                );
            }
            if action.is_none() && mouse.right_just_pressed {
                action = handle_mouse_click(
                    (mouse.hud_x, mouse.hud_y),
                    true,
                    &mut effects.state,
                    game,
                    items,
                    (player_col, player_row),
                );
            }
        }
    }

    if let Some(action) = action {
        crate::session::apply_inventory_action(
            &mut effects.session.game,
            &mut effects.item_render,
            &mut effects.hud,
            &action,
        );
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

fn draw_slot(canvas: &mut PixelCanvas, x: i32, y: i32) {
    canvas.fill_rect(x, y, SLOT_SIZE, SLOT_SIZE, SLOT_BG);
    canvas.stroke_rect(x, y, SLOT_SIZE, SLOT_SIZE, SLOT_BORDER);
}

fn slot_highlight(
    canvas: &mut PixelCanvas,
    x: i32,
    y: i32,
    is_cursor: bool,
    is_valid_drop: bool,
    is_backpack_drop_target: bool,
) {
    if is_cursor {
        canvas.fill_rect(x - 1, y - 1, SLOT_SIZE + 2, SLOT_SIZE + 2, CURSOR_FILL);
        canvas.stroke_rect(x - 1, y - 1, SLOT_SIZE + 2, SLOT_SIZE + 2, CURSOR_BORDER);
    }
    if is_valid_drop {
        canvas.fill_rect(x - 1, y - 1, SLOT_SIZE + 2, SLOT_SIZE + 2, VALID_DROP_FILL);
        canvas.stroke_rect(
            x - 1,
            y - 1,
            SLOT_SIZE + 2,
            SLOT_SIZE + 2,
            VALID_DROP_BORDER,
        );
    }
    if is_backpack_drop_target {
        canvas.fill_rect(
            x - 1,
            y - 1,
            SLOT_SIZE + 2,
            SLOT_SIZE + 2,
            BACKPACK_DROP_FILL,
        );
        canvas.stroke_rect(
            x - 1,
            y - 1,
            SLOT_SIZE + 2,
            SLOT_SIZE + 2,
            BACKPACK_DROP_BORDER,
        );
    }
}

pub fn draw_inventory_overlay(
    canvas: &mut PixelCanvas,
    state: &InventoryOverlayState,
    game: &GameState,
    items: &ItemDatabase,
    icons: &mut IconCache,
) {
    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, BACKDROP);
    canvas.fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BG);
    canvas.stroke_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BORDER);

    let title = "INVENTORY";
    let title_w = measure_pixel_text(title, 3);
    draw_pixel_text(
        canvas,
        title,
        PANEL_X + (PANEL_W - title_w) / 2,
        PANEL_Y + 14,
        TITLE_TEXT,
        3,
    );

    let gold_text = format!("{}G", game.player.gold);
    let gold_text_w = measure_pixel_text(&gold_text, 2);
    let gold_x = PANEL_X + PANEL_W - 16 - (10 + 4 + gold_text_w);
    canvas.fill_ellipse(
        (gold_x + 5) as f32,
        (PANEL_Y + 22) as f32,
        5.0,
        5.0,
        GOLD_COIN,
    );
    draw_pixel_text(canvas, &gold_text, gold_x + 14, PANEL_Y + 18, GOLD_COIN, 2);

    for (index, &slot) in EQUIP_SLOTS.iter().enumerate() {
        let pos = CursorPos {
            section: Section::Equipment,
            index,
        };
        let (sx, sy) = slot_origin(pos);
        let is_valid_drop = state.drag.as_ref().is_some_and(|drag| {
            drag.source.section == Section::Backpack && drag.valid_equip_slots.contains(&index)
        });
        slot_highlight(
            canvas,
            sx,
            sy,
            state.drag.is_none() && state.cursor == pos,
            is_valid_drop,
            false,
        );
        draw_slot(canvas, sx, sy);

        if let Some(entity) = game.entity_registry.get_equipped(slot) {
            draw_item_icon(
                canvas,
                icons,
                items,
                &entity.item_id,
                (sx, sy, SLOT_SIZE),
                ICON_FALLBACK,
                IconSampling::Native,
            );
        } else if let Some(ghost) = icons.get(crate::hud::paperdoll_path(slot)) {
            canvas.blit_icon(
                ghost,
                (
                    sx + PAPERDOLL_PAD,
                    sy + PAPERDOLL_PAD,
                    SLOT_SIZE - PAPERDOLL_PAD * 2,
                    SLOT_SIZE - PAPERDOLL_PAD * 2,
                ),
                PAPERDOLL_ALPHA,
            );
        }
    }

    let (_, equip_y) = equip_origin();
    let sep_y = equip_y + 2 * (SLOT_SIZE + SLOT_GAP) + 6;
    canvas.stroke_line(
        PANEL_X + 16,
        sep_y,
        PANEL_X + PANEL_W - 16,
        sep_y,
        PANEL_BORDER,
    );

    for index in 0..BACKPACK_SLOTS {
        let pos = CursorPos {
            section: Section::Backpack,
            index,
        };
        let (sx, sy) = slot_origin(pos);
        let is_backpack_drop_target = state.drag.as_ref().is_some_and(|drag| {
            !(drag.source.section == Section::Backpack && drag.source.index == index)
        });
        slot_highlight(
            canvas,
            sx,
            sy,
            state.drag.is_none() && state.cursor == pos,
            false,
            is_backpack_drop_target,
        );
        draw_slot(canvas, sx, sy);

        if let Some(entity) = game.entity_registry.backpack_item_at(index as u32) {
            draw_item_icon(
                canvas,
                icons,
                items,
                &entity.item_id,
                (sx, sy, SLOT_SIZE),
                ICON_FALLBACK,
                IconSampling::Native,
            );
        }
    }

    if state.drag.is_none() {
        let hovered = match state.cursor.section {
            Section::Equipment => game
                .entity_registry
                .get_equipped(EQUIP_SLOTS[state.cursor.index]),
            Section::Backpack => game
                .entity_registry
                .backpack_item_at(state.cursor.index as u32),
        };
        if let Some(entity) = hovered {
            let (sx, sy) = slot_origin(state.cursor);
            draw_item_tooltip(canvas, entity, game, items, sx + SLOT_SIZE + 4, sy);
        }
    }

    if let Some(drag) = &state.drag {
        draw_item_icon(
            canvas,
            icons,
            items,
            &drag.item_id,
            (
                drag.hud_x as i32 - SLOT_SIZE / 2,
                drag.hud_y as i32 - SLOT_SIZE / 2,
                SLOT_SIZE,
            ),
            ICON_FALLBACK,
            IconSampling::Native,
        );
    }

    let footer = "I/ESC CLOSE   DBLCLICK EQUIP   RCLICK DROP   DRAG TO MOVE";
    let footer_w = measure_pixel_text(footer, 1);
    draw_pixel_text(
        canvas,
        footer,
        PANEL_X + (PANEL_W - footer_w) / 2,
        PANEL_Y + PANEL_H - 14,
        HINT_TEXT,
        1,
    );
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

    fn item_db() -> ItemDatabase {
        let json = r#"{
            "version": "1",
            "items": [
                {"id":"sword_iron","name":"Iron Sword","type":"weapon","subtype":"sword","quality":"common","icon":"","weight":1,"value":1,"description":"","stats":{},"modifiers":[],"requirements":{}},
                {"id":"hp_potion","name":"Health Potion","type":"consumable","subtype":"health_potion","quality":"common","icon":"","weight":1,"value":1,"description":"","stats":{},"modifiers":[],"requirements":{}},
                {"id":"ring_gold","name":"Gold Ring","type":"accessory","subtype":"ring","quality":"common","icon":"","weight":1,"value":1,"description":"","stats":{},"modifiers":[],"requirements":{}}
            ]
        }"#;
        ItemDatabase::from_json(json).expect("fixture items parse")
    }

    fn equip(game: &mut GameState, item_id: &str, slot: EquipSlot) {
        game.entity_registry.create_item(
            item_id,
            ItemQuality::Common,
            ItemLocation::Equipped { slot },
            Vec::new(),
        );
    }

    fn stash(game: &mut GameState, item_id: &str, slot: u32) {
        game.entity_registry.create_item(
            item_id,
            ItemQuality::Common,
            ItemLocation::Backpack { slot },
            Vec::new(),
        );
    }

    // --- Cursor navigation ---

    #[test]
    fn arrow_right_stops_at_equipment_row_edge() {
        let mut cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        for _ in 0..4 {
            cursor = move_right(cursor);
        }
        assert_eq!(cursor.index, 4);
        assert_eq!(move_right(cursor), cursor);
    }

    #[test]
    fn arrow_left_is_noop_at_leftmost_column() {
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        assert_eq!(move_left(cursor), cursor);
    }

    #[test]
    fn arrow_down_from_equipment_row0_then_row1_reaches_backpack() {
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        let row1 = move_down(cursor);
        assert_eq!(row1.section, Section::Equipment);
        assert_eq!(row1.index, 5);
        let backpack = move_down(row1);
        assert_eq!(backpack.section, Section::Backpack);
        assert_eq!(backpack.index, 0);
    }

    #[test]
    fn arrow_up_from_equipment_row0_is_noop() {
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 2,
        };
        assert_eq!(move_up(cursor), cursor);
    }

    #[test]
    fn arrow_up_from_backpack_row0_jumps_to_equipment_row1() {
        let cursor = CursorPos {
            section: Section::Backpack,
            index: 2,
        };
        let equip = move_up(cursor);
        assert_eq!(equip.section, Section::Equipment);
        assert_eq!(equip.index, 5 + 2);
    }

    #[test]
    fn arrow_down_stops_at_backpack_last_row() {
        let cursor = CursorPos {
            section: Section::Backpack,
            index: 9,
        };
        assert_eq!(move_down(cursor), cursor);
    }

    // --- hit_test ---

    #[test]
    fn hit_test_finds_equipment_slot_zero() {
        let (x, y) = equip_origin();
        let pos = hit_test(x as f32 + 1.0, y as f32 + 1.0);
        assert_eq!(
            pos,
            Some(CursorPos {
                section: Section::Equipment,
                index: 0
            })
        );
    }

    #[test]
    fn hit_test_finds_backpack_slot_zero() {
        let (x, y) = backpack_origin();
        let pos = hit_test(x as f32 + 1.0, y as f32 + 1.0);
        assert_eq!(
            pos,
            Some(CursorPos {
                section: Section::Backpack,
                index: 0
            })
        );
    }

    #[test]
    fn hit_test_returns_none_off_any_slot() {
        assert_eq!(hit_test(0.0, 0.0), None);
    }

    // --- Enter (equip from backpack) ---

    #[test]
    fn enter_on_backpack_weapon_returns_equip_at_sorted_position() {
        let mut game = game();
        stash(&mut game, "sword_iron", 3);
        let items = item_db();
        let cursor = CursorPos {
            section: Section::Backpack,
            index: 3,
        };
        let action = handle_enter(cursor, &game, &items).expect("weapon should equip");
        assert_eq!(
            action,
            InventoryAction::Equip {
                backpack_slot: 0,
                equip_slot: EquipSlot::Weapon
            }
        );
    }

    #[test]
    fn enter_on_backpack_consumable_returns_use() {
        let mut game = game();
        stash(&mut game, "hp_potion", 0);
        let items = item_db();
        let cursor = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        assert_eq!(
            handle_enter(cursor, &game, &items),
            Some(InventoryAction::Use { backpack_slot: 0 })
        );
    }

    #[test]
    fn enter_on_empty_backpack_slot_is_none() {
        let game = game();
        let items = item_db();
        let cursor = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        assert_eq!(handle_enter(cursor, &game, &items), None);
    }

    // --- Enter (unequip) ---

    #[test]
    fn enter_on_occupied_equipment_slot_returns_unequip() {
        let mut game = game();
        equip(&mut game, "sword_iron", EquipSlot::Weapon);
        let items = item_db();
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        assert_eq!(
            handle_enter(cursor, &game, &items),
            Some(InventoryAction::Unequip {
                equip_slot: EquipSlot::Weapon,
                backpack_slot: 0
            })
        );
    }

    #[test]
    fn enter_on_empty_equipment_slot_is_none() {
        let game = game();
        let items = item_db();
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        assert_eq!(handle_enter(cursor, &game, &items), None);
    }

    #[test]
    fn enter_unequip_with_full_backpack_returns_message() {
        let mut game = game();
        equip(&mut game, "sword_iron", EquipSlot::Weapon);
        for slot in 0..12 {
            stash(&mut game, "hp_potion", slot);
        }
        let items = item_db();
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        let action = handle_enter(cursor, &game, &items).unwrap();
        assert!(matches!(action, InventoryAction::Message { .. }));
    }

    // --- Drop ---

    #[test]
    fn drop_on_equipped_item_carries_player_position() {
        let mut game = game();
        let entity = game.entity_registry.create_item(
            "sword_iron",
            ItemQuality::Common,
            ItemLocation::Equipped {
                slot: EquipSlot::Weapon,
            },
            Vec::new(),
        );
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        assert_eq!(
            handle_drop(cursor, &game, 3, 5),
            Some(InventoryAction::Drop {
                instance_id: entity.instance_id,
                col: 3,
                row: 5
            })
        );
    }

    #[test]
    fn drop_on_empty_equipment_slot_is_none() {
        let game = game();
        let cursor = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        assert_eq!(handle_drop(cursor, &game, 1, 1), None);
    }

    // --- Drag transition table (PHASE4-PLAN.md's explicit ask) ---

    #[test]
    fn drag_same_slot_is_noop() {
        let game = game();
        let items = item_db();
        let pos = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        assert_eq!(resolve_drag(pos, pos, &game, &items), None);
    }

    #[test]
    fn drag_backpack_to_correct_equipment_slot_equips() {
        let mut game = game();
        stash(&mut game, "sword_iron", 2);
        let items = item_db();
        let source = CursorPos {
            section: Section::Backpack,
            index: 2,
        };
        let target = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        assert_eq!(
            resolve_drag(source, target, &game, &items),
            Some(InventoryAction::Equip {
                backpack_slot: 0,
                equip_slot: EquipSlot::Weapon
            })
        );
    }

    #[test]
    fn drag_backpack_to_wrong_equipment_slot_is_noop() {
        let mut game = game();
        stash(&mut game, "sword_iron", 0);
        let items = item_db();
        let source = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        let target = CursorPos {
            section: Section::Equipment,
            index: 1, // head slot — a sword doesn't belong there
        };
        assert_eq!(resolve_drag(source, target, &game, &items), None);
    }

    #[test]
    fn drag_consumable_to_equipment_is_rejected() {
        let mut game = game();
        stash(&mut game, "hp_potion", 0);
        let items = item_db();
        let source = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        let target = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        assert_eq!(resolve_drag(source, target, &game, &items), None);
    }

    #[test]
    fn drag_ring_to_ring1_when_ring1_free_equips() {
        let mut game = game();
        stash(&mut game, "ring_gold", 0);
        let items = item_db();
        let source = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        let ring1_index = equip_slot_index(EquipSlot::Ring1).unwrap();
        let target = CursorPos {
            section: Section::Equipment,
            index: ring1_index,
        };
        assert_eq!(
            resolve_drag(source, target, &game, &items),
            Some(InventoryAction::Equip {
                backpack_slot: 0,
                equip_slot: EquipSlot::Ring1
            })
        );
    }

    /// PHASE4-PLAN.md singled this one out: dragging a ring onto ring2 when
    /// ring1 is occupied and ring2 is free must equip into ring2, even
    /// though `subtype_to_equip_slot`'s own "natural" answer for a fresh
    /// ring is ring1 (already occupied) — the `is_ring` override in
    /// `resolve_drag` exists specifically for this case.
    #[test]
    fn drag_ring_onto_free_ring2_equips_there_even_though_ring1_is_occupied() {
        let mut game = game();
        equip(&mut game, "ring_gold", EquipSlot::Ring1);
        stash(&mut game, "ring_gold", 0);
        let items = item_db();
        let source = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        let ring2_index = equip_slot_index(EquipSlot::Ring2).unwrap();
        let target = CursorPos {
            section: Section::Equipment,
            index: ring2_index,
        };
        assert_eq!(
            resolve_drag(source, target, &game, &items),
            Some(InventoryAction::Equip {
                backpack_slot: 0,
                equip_slot: EquipSlot::Ring2
            })
        );
    }

    #[test]
    fn drag_equipment_to_empty_backpack_slot_unequips() {
        let mut game = game();
        equip(&mut game, "sword_iron", EquipSlot::Weapon);
        let items = item_db();
        let source = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        let target = CursorPos {
            section: Section::Backpack,
            index: 4,
        };
        assert_eq!(
            resolve_drag(source, target, &game, &items),
            Some(InventoryAction::Unequip {
                equip_slot: EquipSlot::Weapon,
                backpack_slot: 4
            })
        );
    }

    #[test]
    fn drag_equipment_to_occupied_backpack_slot_is_noop() {
        let mut game = game();
        equip(&mut game, "sword_iron", EquipSlot::Weapon);
        stash(&mut game, "hp_potion", 4);
        let items = item_db();
        let source = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        let target = CursorPos {
            section: Section::Backpack,
            index: 4,
        };
        assert_eq!(resolve_drag(source, target, &game, &items), None);
    }

    #[test]
    fn drag_backpack_to_backpack_always_swaps() {
        let game = game();
        let items = item_db();
        let source = CursorPos {
            section: Section::Backpack,
            index: 0,
        };
        let target = CursorPos {
            section: Section::Backpack,
            index: 5,
        };
        assert_eq!(
            resolve_drag(source, target, &game, &items),
            Some(InventoryAction::Swap {
                index_a: 0,
                index_b: 5
            })
        );
    }

    #[test]
    fn drag_equipment_to_equipment_is_unsupported() {
        let mut game = game();
        equip(&mut game, "sword_iron", EquipSlot::Weapon);
        let items = item_db();
        let source = CursorPos {
            section: Section::Equipment,
            index: 0,
        };
        let target = CursorPos {
            section: Section::Equipment,
            index: 1,
        };
        assert_eq!(resolve_drag(source, target, &game, &items), None);
    }

    // --- Drag-start valid-target precomputation ---

    #[test]
    fn valid_equip_slots_for_ring_include_both_ring_slots() {
        let game = game();
        let slots = valid_equip_slots_for_drag(
            ItemType::Accessory,
            ItemSubtype::Ring,
            Section::Backpack,
            &game,
        );
        assert!(slots.contains(&equip_slot_index(EquipSlot::Ring1).unwrap()));
        assert!(slots.contains(&equip_slot_index(EquipSlot::Ring2).unwrap()));
    }

    #[test]
    fn valid_equip_slots_empty_for_consumable() {
        let game = game();
        let slots = valid_equip_slots_for_drag(
            ItemType::Consumable,
            ItemSubtype::HealthPotion,
            Section::Backpack,
            &game,
        );
        assert!(slots.is_empty());
    }

    #[test]
    fn valid_equip_slots_empty_when_dragging_from_equipment() {
        let game = game();
        let slots = valid_equip_slots_for_drag(
            ItemType::Weapon,
            ItemSubtype::Sword,
            Section::Equipment,
            &game,
        );
        assert!(slots.is_empty());
    }
}

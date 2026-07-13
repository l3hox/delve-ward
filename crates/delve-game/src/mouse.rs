//! Mouse position and button-edge tracking, consumed by the inventory
//! overlay and the mini inventory panel's mouse interactions.
//!
//! `left_double_clicked` reimplements the browser's native `dblclick`
//! event, which Bevy's `ButtonInput<MouseButton>` has no equivalent for:
//! `hudCanvas.ts`'s `attach()` wires drag start/end on every raw
//! mousedown/mouseup *and* a separate `dblclick` listener for the
//! equip/unequip/use action — the two fire independently per browser
//! semantics (a double-click also produces two same-slot drag attempts,
//! which the overlay's own "same slot — no-op" check absorbs). This module
//! reproduces just the timing: two `left_just_released` edges within
//! `DOUBLE_CLICK_WINDOW` seconds and `DOUBLE_CLICK_DISTANCE` HUD pixels of
//! each other. Neither constant has a TS source of truth — the browser's
//! own double-click detection is OS/UA-controlled, not a value in this
//! codebase — so these are reasonable defaults, not ported numbers.

use bevy::prelude::*;

const DOUBLE_CLICK_WINDOW: f32 = 0.4;
const DOUBLE_CLICK_DISTANCE: f32 = 4.0;

/// Cursor position in HUD-canvas space (`hud::screen_to_hud`'s 640x360
/// coordinate system, not window pixels) plus this-frame button edges —
/// mirrors the already-established `ButtonInput<KeyCode>` edge-query shape
/// so mouse-driven overlays read this the same way keyboard-driven ones
/// already read keys.
#[derive(Resource, Default)]
pub struct MouseState {
    pub hud_x: f32,
    pub hud_y: f32,
    /// `false` when the cursor is outside the window — `hud_x`/`hud_y` hold
    /// their last known position rather than resetting, matching how a
    /// stale-but-not-wrong hover position is preferable to snapping to a
    /// corner. Consumers should check this before trusting the coordinate.
    pub in_window: bool,
    pub left_just_pressed: bool,
    pub left_just_released: bool,
    pub right_just_pressed: bool,
    pub right_just_released: bool,
    /// True on the exact frame a second left-click lands within the
    /// double-click window/distance of the previous one — see the module
    /// doc comment.
    pub left_double_clicked: bool,
    last_click_at: Option<f32>,
    last_click_pos: Vec2,
}

pub fn track_mouse(
    time: Res<Time>,
    windows: Query<&Window>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<MouseState>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    state.left_just_pressed = buttons.just_pressed(MouseButton::Left);
    state.left_just_released = buttons.just_released(MouseButton::Left);
    state.right_just_pressed = buttons.just_pressed(MouseButton::Right);
    state.right_just_released = buttons.just_released(MouseButton::Right);
    state.left_double_clicked = false;

    match window.cursor_position() {
        Some(cursor) => {
            let hud = crate::hud::screen_to_hud(cursor, window);
            state.hud_x = hud.x;
            state.hud_y = hud.y;
            state.in_window = true;
        }
        None => state.in_window = false,
    }

    if state.left_just_released && state.in_window {
        let now = time.elapsed_secs();
        let pos = Vec2::new(state.hud_x, state.hud_y);
        let is_double = state.last_click_at.is_some_and(|last_at| {
            now - last_at <= DOUBLE_CLICK_WINDOW
                && pos.distance(state.last_click_pos) <= DOUBLE_CLICK_DISTANCE
        });
        if is_double {
            state.left_double_clicked = true;
            // Consumed — a third rapid click starts a fresh pair rather
            // than chaining into a second double-click.
            state.last_click_at = None;
        } else {
            state.last_click_at = Some(now);
            state.last_click_pos = pos;
        }
    }

    if state.left_just_pressed || state.right_just_pressed || state.left_double_clicked {
        debug!(
            "mouse: hud=({:.1},{:.1}) left={} right={} dbl={}",
            state.hud_x,
            state.hud_y,
            state.left_just_pressed,
            state.right_just_pressed,
            state.left_double_clicked
        );
    }
}

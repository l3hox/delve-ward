//! Mouse position and button-edge tracking — foundation for the
//! mouse-driven overlays landing in later phase-4 slices (inventory drag,
//! trading, dialog choice clicks). No consumer yet; `track_mouse` updates
//! [`MouseState`] and traces button edges so the plumbing is provably
//! correct ahead of anything depending on it.

use bevy::prelude::*;

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
}

pub fn track_mouse(
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

    match window.cursor_position() {
        Some(cursor) => {
            let hud = crate::hud::screen_to_hud(cursor, window);
            state.hud_x = hud.x;
            state.hud_y = hud.y;
            state.in_window = true;
        }
        None => state.in_window = false,
    }

    if state.left_just_pressed || state.right_just_pressed {
        debug!(
            "mouse: hud=({:.1},{:.1}) left={} right={}",
            state.hud_x, state.hud_y, state.left_just_pressed, state.right_just_pressed
        );
    }
}

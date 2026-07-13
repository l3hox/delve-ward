//! Sign and bookshelf read popup, ported from `hud/signOverlay.ts` and its
//! wiring in `game/inputSystem.ts` (the `sign_read`/`bookshelf_read` result
//! branches at lines 200-205, both calling the same `signOverlay.show`).
//! TS shares one overlay between signs and bookshelves — there is no
//! separate `bookshelfOverlay` — so this port does too, matching
//! `session.rs`'s existing comment that `BookshelfRead` "has no dedicated
//! visual state... beyond its own text popup" distinct from `SignRead`.
//!
//! TS's `SignOverlay` is a raw DOM panel (`document.createElement('div')`
//! with inline CSS), not a canvas draw function — there is nothing to port
//! 1:1, the same situation `dialog_overlay.rs`'s module doc already
//! resolved for `dialogOverlay.ts`. This is a fresh pixel-canvas panel
//! design carrying the same visual intent (a small centered card, warm
//! brown/gold palette, centered text, a dim italic-style hint line) rather
//! than a literal translation of CSS pixel values — `TEXT_WRAP_CHARS` in
//! particular has no TS equivalent to port, since DOM text wraps by pixel
//! width at a 13px monospace font with no character-count analog; the value
//! here is a readability choice sized to this panel's own width, the same
//! kind of judgment call `dialog_overlay.rs`'s `TEXT_WRAP_CHARS` already
//! made for its own (wider) panel.
//!
//! Dismiss is unconditional — TS's `_keyHandler` closes on *any* keydown
//! (no key filtering) while `visible`, and a click anywhere on the backdrop
//! also closes it (`container`'s own `click` listener). Both are ported
//! here as-is: any just-pressed key or a left click closes the popup.
//!
//! `sign_overlay_input` must run before `session::interact_input` in the
//! system chain, matching every other overlay-input system's position
//! there (`dialog_input`, `save_load_input`, etc.) — the Space press that
//! opens the popup (via `interact_input`, later in the same frame's chain)
//! must not be the same `just_pressed` press this system reads to close it
//! again; running first means this system still sees `ActiveOverlay::None`
//! on the opening frame and only starts checking for a dismiss key on the
//! next one, once the keypress that opened it has expired.

use crate::dialog_overlay::wrap_text;
use crate::hud::{HUD_HEIGHT, HUD_WIDTH};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::mouse::MouseState;
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use bevy::prelude::*;

const PANEL_W: i32 = 380;
const PANEL_PAD_Y: i32 = 18;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const LINE_HEIGHT: i32 = 12;
const HINT_GAP: i32 = 12;
const TEXT_WRAP_CHARS: usize = 44;

const BACKDROP: Rgba = Rgba::translucent(0, 0, 0, 0.6);
const PANEL_BG: Rgba = Rgba::opaque(0x1a, 0x12, 0x08);
const PANEL_BORDER: Rgba = Rgba::opaque(0x8a, 0x6a, 0x2a);
const BODY_TEXT: Rgba = Rgba::opaque(0xdd, 0xc8, 0xa0);
const HINT_TEXT: Rgba = Rgba::opaque(0x66, 0x58, 0x30);

/// The currently displayed text, if any. Whether the popup is open is
/// centralized in `ActiveOverlay::Sign`, not a field here, matching every
/// other phase-4/5/6 overlay resource's convention.
#[derive(Resource, Default)]
pub struct SignOverlayState {
    pub text: String,
}

/// Opens the popup with `text` — the `interact_input` call site's
/// `SignRead`/`BookshelfRead` arm should call this instead of (not in
/// addition to) the generic HUD toast, matching TS calling `signOverlay.show`
/// with no accompanying `hud.showMessage` for either result type.
pub fn open_sign(state: &mut SignOverlayState, overlay: &mut ActiveOverlay, text: &str) {
    state.text = text.to_string();
    *overlay = ActiveOverlay::Sign;
}

/// Uppercases (the pixel font has no lowercase glyphs, matching the rest of
/// this HUD) and word-wraps `text` to this panel's own width — the pure
/// helper backing [`draw_sign_overlay`]'s layout math, pinned by the tests
/// below since it has no upstream TS fixture to check against (DOM word-wrap
/// has no character-count equivalent to compare against).
fn body_lines(text: &str) -> Vec<String> {
    wrap_text(&text.to_uppercase(), TEXT_WRAP_CHARS)
}

fn panel_height(lines: &[String]) -> i32 {
    let body_h = lines.len() as i32 * LINE_HEIGHT;
    PANEL_PAD_Y * 2 + body_h + HINT_GAP + LINE_HEIGHT
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Any just-pressed key or a left click closes the popup — ported from
/// `signOverlay.ts`'s unconditional `_keyHandler` plus its backdrop `click`
/// listener. See the module doc for why this system's position in the chain
/// (before `session::interact_input`) matters.
pub fn sign_overlay_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<MouseState>,
    mut overlay: ResMut<ActiveOverlay>,
) {
    if *overlay != ActiveOverlay::Sign {
        return;
    }
    if keys.get_just_pressed().next().is_some() || mouse.left_just_pressed {
        *overlay = ActiveOverlay::None;
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

/// Draws the sign/bookshelf text popup, centered on screen — the canvas
/// equivalent of TS's `SignOverlay.show`.
pub fn draw_sign_overlay(canvas: &mut PixelCanvas, state: &SignOverlayState) {
    let lines = body_lines(&state.text);
    let panel_h = panel_height(&lines);
    let panel_y = (HUD_HEIGHT as i32 - panel_h) / 2;

    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, BACKDROP);
    canvas.fill_rect(PANEL_X, panel_y, PANEL_W, panel_h, PANEL_BG);
    canvas.stroke_rect(PANEL_X, panel_y, PANEL_W, panel_h, PANEL_BORDER);

    let mut cursor_y = panel_y + PANEL_PAD_Y;
    for line in &lines {
        let line_w = measure_pixel_text(line, 1);
        draw_pixel_text(
            canvas,
            line,
            PANEL_X + (PANEL_W - line_w) / 2,
            cursor_y,
            BODY_TEXT,
            1,
        );
        cursor_y += LINE_HEIGHT;
    }

    cursor_y += HINT_GAP;
    let hint = "PRESS ANY KEY TO CLOSE";
    let hint_w = measure_pixel_text(hint, 1);
    draw_pixel_text(
        canvas,
        hint,
        PANEL_X + (PANEL_W - hint_w) / 2,
        cursor_y,
        HINT_TEXT,
        1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_stays_on_one_line() {
        assert_eq!(
            body_lines("beware the darkness"),
            vec!["BEWARE THE DARKNESS"]
        );
    }

    #[test]
    fn long_text_wraps_across_multiple_lines() {
        let text =
            "the ancient wards protecting this crypt have long since faded into dust and shadow";
        let lines = body_lines(text);
        assert!(lines.len() > 1, "expected multiple lines, got {lines:?}");
        for line in &lines {
            assert!(
                line.chars().count() <= TEXT_WRAP_CHARS,
                "line exceeds wrap width: {line:?}"
            );
        }
    }

    #[test]
    fn empty_text_yields_one_empty_line() {
        assert_eq!(body_lines(""), vec![String::new()]);
    }
}

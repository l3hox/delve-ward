//! Quest log panel (`KeyJ`), ported from `hud/questLogOverlay.ts`. Purely
//! read-only — no interaction beyond open/close, the same shape as
//! `stats_panel.rs`.
//!
//! TS's `KeyJ` case in `inputSystem.ts` is a bare `show()`, not a
//! `toggle()` — but the input system's own top-of-handler guard
//! (`if (ctx.questLogOverlay.isOpen()) return;`) means `KeyJ` never
//! re-fires `show()` while open, and closing happens through the overlay's
//! own independent capture-phase listener (`Escape`, `j`, or `J`). Net
//! effect is toggle-like; ported here as the same `ActiveOverlay`-gated
//! open/close pair every other overlay in this port already uses
//! (`*overlay != QuestLog` gates opening, `KeyJ`/`Escape` close it while
//! open) rather than two separate code paths, since both produce identical
//! behavior and the single-pair version is what the rest of this codebase
//! already does. TS's own unused `toggle()` method (confirmed via grep:
//! never called from `main.ts`/`inputSystem.ts`) is not ported — dead code
//! in the source has no reason to become live code here.

use crate::dialog_overlay::wrap_text;
use crate::hud::{HUD_HEIGHT, HUD_WIDTH};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::transition::Transition;
use bevy::prelude::*;
use delve_core::quest_manager::QuestManager;

const PANEL_W: i32 = 420;
const PANEL_H: i32 = 300;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const PANEL_Y: i32 = (HUD_HEIGHT as i32 - PANEL_H) / 2;
const CONTENT_X: i32 = PANEL_X + 20;
const CONTENT_WRAP_CHARS: usize = 52;
const LINE_H: i32 = 12;

const BACKDROP: Rgba = Rgba::translucent(0, 0, 0, 0.85);
const PANEL_BG: Rgba = Rgba::translucent(10, 8, 12, 0.75);
const PANEL_BORDER: Rgba = Rgba::opaque(0x2a, 0x22, 0x30);
const TITLE_TEXT: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const SECTION_HEADER: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const QUEST_NAME: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const STAGE_TEXT: Rgba = Rgba::opaque(0xc0, 0xc0, 0xc0);
const FLAVOR_TEXT: Rgba = Rgba::opaque(0x77, 0x77, 0x66);
const COMPLETED_TEXT: Rgba = Rgba::opaque(0x6a, 0x8a, 0x5a);
const HINT_TEXT: Rgba = Rgba::opaque(0x66, 0x66, 0x66);

/// Opens from the dungeon on `KeyJ`; closes on `KeyJ` or Escape while open.
pub fn quest_log_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<ActiveOverlay>,
    transition: Res<Transition>,
) {
    if *overlay != ActiveOverlay::QuestLog {
        if transition.is_active() || *overlay != ActiveOverlay::None {
            return;
        }
        if keys.just_pressed(KeyCode::KeyJ) {
            *overlay = ActiveOverlay::QuestLog;
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyJ) || keys.just_pressed(KeyCode::Escape) {
        *overlay = ActiveOverlay::None;
    }
}

/// Quest ids in a stable, deterministic order for display — `QuestManager`
/// tracks them in a `HashMap`, which has no reliable iteration order across
/// runs (TS's `Map` preserves quest-start order instead). Sorting by id is
/// a disclosed rendering-order deviation, not a functional one: which
/// quests appear and what they say is unaffected, only the order they list
/// in.
fn sorted_quest_ids(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

pub fn draw_quest_log_overlay(canvas: &mut PixelCanvas, quests: &QuestManager) {
    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, BACKDROP);
    canvas.fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BG);
    canvas.stroke_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BORDER);

    let title = "QUEST LOG";
    let title_w = measure_pixel_text(title, 3);
    draw_pixel_text(
        canvas,
        title,
        PANEL_X + (PANEL_W - title_w) / 2,
        PANEL_Y + 14,
        TITLE_TEXT,
        3,
    );
    canvas.stroke_line(
        PANEL_X + 8,
        PANEL_Y + 34,
        PANEL_X + PANEL_W - 8,
        PANEL_Y + 34,
        PANEL_BORDER,
    );

    let mut y = PANEL_Y + 46;
    draw_pixel_text(canvas, "ACTIVE QUESTS", CONTENT_X, y, SECTION_HEADER, 1);
    y += LINE_H + 4;

    let active_ids = sorted_quest_ids(quests.get_active_quests());
    if active_ids.is_empty() {
        draw_pixel_text(canvas, "NO ACTIVE QUESTS", CONTENT_X, y, FLAVOR_TEXT, 1);
        y += LINE_H;
    } else {
        for quest_id in &active_ids {
            let Some(def) = quests.get_quest_def(quest_id) else {
                continue;
            };
            draw_pixel_text(
                canvas,
                &def.name.to_uppercase(),
                CONTENT_X,
                y,
                QUEST_NAME,
                1,
            );
            y += LINE_H;

            let stage_index = quests.get_stage_index(quest_id);
            if stage_index >= 0
                && let Some(stage) = def.stages.get(stage_index as usize)
            {
                for line in wrap_text(&stage.description.to_uppercase(), CONTENT_WRAP_CHARS) {
                    draw_pixel_text(canvas, &line, CONTENT_X + 6, y, STAGE_TEXT, 1);
                    y += LINE_H;
                }
            }

            for line in wrap_text(&def.description.to_uppercase(), CONTENT_WRAP_CHARS) {
                draw_pixel_text(canvas, &line, CONTENT_X + 6, y, FLAVOR_TEXT, 1);
                y += LINE_H;
            }
            y += 4;
        }
    }

    let completed_ids = sorted_quest_ids(quests.get_completed_quests());
    if !completed_ids.is_empty() {
        y += 6;
        draw_pixel_text(canvas, "COMPLETED QUESTS", CONTENT_X, y, SECTION_HEADER, 1);
        y += LINE_H + 4;
        for quest_id in &completed_ids {
            let name = quests
                .get_quest_def(quest_id)
                .map_or_else(|| quest_id.clone(), |def| def.name.clone());
            draw_pixel_text(
                canvas,
                &format!("\u{2713} {}", name.to_uppercase()),
                CONTENT_X,
                y,
                COMPLETED_TEXT,
                1,
            );
            y += LINE_H;
        }
    }

    let hint = "J / ESC: CLOSE";
    let hint_w = measure_pixel_text(hint, 1);
    draw_pixel_text(
        canvas,
        hint,
        PANEL_X + (PANEL_W - hint_w) / 2,
        PANEL_Y + PANEL_H - 18,
        HINT_TEXT,
        1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_quest_ids_returns_alphabetical_order() {
        let ids = vec![
            "kill_spider_queen".to_string(),
            "collect_lore".to_string(),
            "fetch_amulet".to_string(),
        ];
        assert_eq!(
            sorted_quest_ids(ids),
            vec![
                "collect_lore".to_string(),
                "fetch_amulet".to_string(),
                "kill_spider_queen".to_string(),
            ]
        );
    }
}

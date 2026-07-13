//! Read-only character stats panel (`KeyT`), ported from `hud/statsPanel.ts`.
//! No interaction beyond toggle+close — pure display over `get_effective_stats`
//! plus the base-only derived formulas TS hardcodes for its own "BASE" column
//! (distinct from, and not to be confused with, `get_effective_stats`'s own
//! modifier-aware formulas used for the "EFFECTIVE" column).

use crate::hud::{HUD_HEIGHT, HUD_WIDTH};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::transition::Transition;
use bevy::prelude::*;
use delve_core::game_state::GameState;

const PANEL_W: i32 = 420;
const PANEL_H: i32 = 300;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const PANEL_Y: i32 = (HUD_HEIGHT as i32 - PANEL_H) / 2;
const ROW_H: i32 = 20;
const ROWS_START_Y: i32 = PANEL_Y + 90;
const LABEL_X: i32 = PANEL_X + 30;
const BASE_X: i32 = PANEL_X + 180;
const EFF_X: i32 = PANEL_X + 290;

const BACKDROP: Rgba = Rgba::translucent(0, 0, 0, 0.85);
const PANEL_BG: Rgba = Rgba::translucent(10, 8, 12, 0.75);
const PANEL_BORDER: Rgba = Rgba::opaque(0x2a, 0x22, 0x30);
const TITLE_TEXT: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const TEXT_DIM: Rgba = Rgba::opaque(0x66, 0x66, 0x66);
const NEUTRAL: Rgba = Rgba::opaque(0xcc, 0xcc, 0xcc);
const POSITIVE: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const NEGATIVE: Rgba = Rgba::opaque(0xcc, 0x44, 0x44);

struct StatRow {
    label: &'static str,
    base: f64,
    effective: f64,
    suffix: &'static str,
}

/// Ported from TS's `KeyT` case: `statsPanel.toggle()` — an unconditional
/// toggle, not a separate open/close pair.
pub fn stats_panel_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<ActiveOverlay>,
    transition: Res<Transition>,
) {
    if *overlay != ActiveOverlay::StatsPanel {
        if transition.is_active() || *overlay != ActiveOverlay::None {
            return;
        }
        if keys.just_pressed(KeyCode::KeyT) {
            *overlay = ActiveOverlay::StatsPanel;
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyT) || keys.just_pressed(KeyCode::Escape) {
        *overlay = ActiveOverlay::None;
    }
}

fn draw_row(canvas: &mut PixelCanvas, row: &StatRow, y: i32) {
    draw_pixel_text(canvas, row.label, LABEL_X, y, NEUTRAL, 2);

    let base_str = format!("{}", row.base as i64);
    draw_pixel_text(canvas, &base_str, BASE_X, y, NEUTRAL, 2);
    if !row.suffix.is_empty() {
        let base_w = measure_pixel_text(&base_str, 2);
        draw_pixel_text(canvas, row.suffix, BASE_X + base_w + 3, y + 3, NEUTRAL, 1);
    }

    let diff = row.effective - row.base;
    let color = if diff > 0.0 {
        POSITIVE
    } else if diff < 0.0 {
        NEGATIVE
    } else {
        NEUTRAL
    };
    let eff_str = format!("{}", row.effective as i64);
    draw_pixel_text(canvas, &eff_str, EFF_X, y, color, 2);
    let mut native_x = EFF_X + measure_pixel_text(&eff_str, 2) + 3;
    if !row.suffix.is_empty() {
        draw_pixel_text(canvas, row.suffix, native_x, y + 3, color, 1);
        native_x += measure_pixel_text(row.suffix, 1) + 2;
    }
    if diff.abs() > f64::EPSILON {
        let diff_label = if diff > 0.0 {
            format!("(+{})", diff as i64)
        } else {
            format!("(-{})", (-diff) as i64)
        };
        draw_pixel_text(canvas, &diff_label, native_x, y + 3, color, 1);
    }
}

pub fn draw_stats_panel(canvas: &mut PixelCanvas, game: &GameState) {
    let effective = game.get_effective_stats();
    let base_atk = (game.player.str / 2.0).floor();
    let base_def = (game.player.vit / 4.0).floor();
    let base_hp = 40.0 + game.player.vit * 5.0;
    let base_crit = 5.0 + (game.player.dex / 3.0).floor();
    let base_dodge = ((game.player.dex - 5.0) / 4.0).floor().clamp(0.0, 25.0);

    let attribute_rows = [
        StatRow {
            label: "STR",
            base: game.player.str,
            effective: effective.effective_str,
            suffix: "",
        },
        StatRow {
            label: "DEX",
            base: game.player.dex,
            effective: effective.effective_dex,
            suffix: "",
        },
        StatRow {
            label: "VIT",
            base: game.player.vit,
            effective: effective.effective_vit,
            suffix: "",
        },
        StatRow {
            label: "WIS",
            base: game.player.wis,
            effective: effective.effective_wis,
            suffix: "",
        },
    ];
    let derived_rows = [
        StatRow {
            label: "ATK",
            base: base_atk,
            effective: effective.atk,
            suffix: "",
        },
        StatRow {
            label: "DEF",
            base: base_def,
            effective: effective.def,
            suffix: "",
        },
        StatRow {
            label: "HP",
            base: base_hp,
            effective: effective.max_hp,
            suffix: "",
        },
        StatRow {
            label: "CRIT",
            base: base_crit,
            effective: effective.crit_chance,
            suffix: "%",
        },
        StatRow {
            label: "DODGE",
            base: base_dodge,
            effective: effective.dodge_chance,
            suffix: "%",
        },
    ];

    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, BACKDROP);
    canvas.fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BG);
    canvas.stroke_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BORDER);

    let title = "CHARACTER STATS";
    let title_w = measure_pixel_text(title, 3);
    draw_pixel_text(
        canvas,
        title,
        PANEL_X + (PANEL_W - title_w) / 2,
        PANEL_Y + 16,
        TITLE_TEXT,
        3,
    );
    let subtitle = format!(
        "{}  LEVEL {}",
        game.player.player_name.to_uppercase(),
        game.player.level
    );
    let subtitle_w = measure_pixel_text(&subtitle, 1);
    draw_pixel_text(
        canvas,
        &subtitle,
        PANEL_X + (PANEL_W - subtitle_w) / 2,
        PANEL_Y + 46,
        TEXT_DIM,
        1,
    );

    let header_y = PANEL_Y + 68;
    draw_pixel_text(canvas, "BASE", BASE_X, header_y, TEXT_DIM, 2);
    draw_pixel_text(canvas, "EFFECTIVE", EFF_X, header_y, TEXT_DIM, 2);

    let mut y = ROWS_START_Y;
    for row in &attribute_rows {
        draw_row(canvas, row, y);
        y += ROW_H;
    }
    y += 4;
    canvas.stroke_line(PANEL_X + 20, y, PANEL_X + PANEL_W - 20, y, PANEL_BORDER);
    y += 10;
    for row in &derived_rows {
        draw_row(canvas, row, y);
        y += ROW_H;
    }

    let footer = "T  CLOSE";
    let footer_w = measure_pixel_text(footer, 2);
    draw_pixel_text(
        canvas,
        footer,
        PANEL_X + (PANEL_W - footer_w) / 2,
        PANEL_Y + PANEL_H - 22,
        TEXT_DIM,
        2,
    );
}

//! Character-creation screen shown at launch, ported from the TS
//! `CharacterCreationScreen`: spend a fixed point budget across STR/DEX/VIT/WIS
//! before spawning into the dungeon. The TS version mixes native canvas text
//! with the pixel font; here everything routes through the pixel font (hence
//! uppercase-only copy), but the panel/row geometry matches the TS numbers.

use crate::hud::{HUD_HEIGHT, HUD_WIDTH};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::save_load_overlay::SaveLoadOverlay;
use crate::session::Session;
use crate::transition::Transition;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

const STARTING_POINTS: i64 = 5;
const MIN_STAT: f64 = 1.0;

const PANEL_W: i32 = 420;
const PANEL_H: i32 = 270;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const PANEL_Y: i32 = (HUD_HEIGHT as i32 - PANEL_H) / 2;

const STAT_ROW_H: i32 = 36;
const STATS_START_Y: i32 = PANEL_Y + 72;

const OVERLAY: Rgba = Rgba::translucent(0, 0, 0, 0.85);
const PANEL_BG: Rgba = Rgba::translucent(10, 8, 12, 0.75);
const PANEL_BORDER: Rgba = Rgba::opaque(0x2a, 0x22, 0x30);
const GOLD: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const GOLD_HIGHLIGHT_FILL: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.12);
const GOLD_HIGHLIGHT_BORDER: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.4);
const TEXT_PRIMARY: Rgba = Rgba::opaque(0xcc, 0xcc, 0xcc);
const TEXT_DIM: Rgba = Rgba::opaque(0x66, 0x66, 0x66);
const READY_GREEN: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);

#[derive(Clone, Copy, PartialEq, Eq)]
enum CreationStat {
    Str,
    Dex,
    Vit,
    Wis,
}

const STATS: [CreationStat; 4] = [
    CreationStat::Str,
    CreationStat::Dex,
    CreationStat::Vit,
    CreationStat::Wis,
];

impl CreationStat {
    fn label(self) -> &'static str {
        match self {
            CreationStat::Str => "STR  STRENGTH",
            CreationStat::Dex => "DEX  DEXTERITY",
            CreationStat::Vit => "VIT  VITALITY",
            CreationStat::Wis => "WIS  WISDOM",
        }
    }

    fn description(self) -> &'static str {
        match self {
            CreationStat::Str => "MELEE DAMAGE",
            CreationStat::Dex => "CRIT AND DODGE CHANCE",
            CreationStat::Vit => "MAX HP",
            CreationStat::Wis => "MAGIC (NOT YET)",
        }
    }
}

/// Character-creation state. Blocks gameplay input and replaces the HUD
/// while `active`, mirroring the TS screen showing before the level loads.
#[derive(Resource)]
pub struct CharCreation {
    pub active: bool,
    name: String,
    str: f64,
    dex: f64,
    vit: f64,
    wis: f64,
    points_remaining: i64,
    selected: usize,
}

impl Default for CharCreation {
    fn default() -> Self {
        Self {
            active: true,
            name: "Adventurer".to_string(),
            str: 5.0,
            dex: 5.0,
            vit: 5.0,
            wis: 5.0,
            points_remaining: STARTING_POINTS,
            selected: 0,
        }
    }
}

impl CharCreation {
    fn stat_value(&self, stat: CreationStat) -> f64 {
        match stat {
            CreationStat::Str => self.str,
            CreationStat::Dex => self.dex,
            CreationStat::Vit => self.vit,
            CreationStat::Wis => self.wis,
        }
    }

    fn set_stat_value(&mut self, stat: CreationStat, value: f64) {
        match stat {
            CreationStat::Str => self.str = value,
            CreationStat::Dex => self.dex = value,
            CreationStat::Vit => self.vit = value,
            CreationStat::Wis => self.wis = value,
        }
    }

    fn return_point(&mut self, stat: CreationStat) {
        let value = self.stat_value(stat);
        if value > MIN_STAT {
            self.set_stat_value(stat, value - 1.0);
            self.points_remaining += 1;
        }
    }

    fn spend_point(&mut self, stat: CreationStat) {
        if self.points_remaining > 0 {
            let value = self.stat_value(stat);
            self.set_stat_value(stat, value + 1.0);
            self.points_remaining -= 1;
        }
    }
}

/// Gameplay systems check `blocked()` the same way they already check
/// `Transition::is_active` — character creation and the save/load overlay
/// are just other reasons input should not reach the dungeon yet, matching
/// TS's `anyOverlayOpen`. Systems that need `ResMut` access to
/// `SaveLoadOverlay`/`Transition` themselves (the overlay's own input
/// handler, the death check) can't use this — it would conflict with their
/// own `ResMut` borrow — and inline the same three-condition check instead.
#[derive(SystemParam)]
pub struct InputGate<'w> {
    transition: Res<'w, Transition>,
    creation: Res<'w, CharCreation>,
    save_load: Res<'w, SaveLoadOverlay>,
}

impl InputGate<'_> {
    pub fn blocked(&self) -> bool {
        self.transition.is_active() || self.creation.active || self.save_load.active
    }
}

pub fn char_creation_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut creation: ResMut<CharCreation>,
    mut session: ResMut<Session>,
) {
    if !creation.active {
        return;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        creation.selected = (creation.selected + STATS.len() - 1) % STATS.len();
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        creation.selected = (creation.selected + 1) % STATS.len();
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        let stat = STATS[creation.selected];
        creation.return_point(stat);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        let stat = STATS[creation.selected];
        creation.spend_point(stat);
    }
    if keys.just_pressed(KeyCode::Enter) && creation.points_remaining == 0 {
        session.game.apply_character_setup(
            creation.str,
            creation.dex,
            creation.vit,
            creation.wis,
            &creation.name,
        );
        creation.active = false;
    }
}

pub fn draw_char_creation(canvas: &mut PixelCanvas, creation: &CharCreation) {
    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, OVERLAY);
    canvas.fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BG);
    canvas.stroke_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BORDER);

    draw_centered(canvas, "DELVEWARD", PANEL_Y + 12, GOLD, 4);
    draw_centered(canvas, "CHOOSE YOUR ATTRIBUTES", PANEL_Y + 46, TEXT_DIM, 1);
    draw_centered(
        canvas,
        &format!("NAME: {}", creation.name.to_uppercase()),
        PANEL_Y + 62,
        TEXT_PRIMARY,
        1,
    );

    for (index, &stat) in STATS.iter().enumerate() {
        draw_stat_row(canvas, creation, stat, index);
    }

    let points_y = STATS_START_Y + STATS.len() as i32 * STAT_ROW_H + 8;
    let points_color = if creation.points_remaining > 0 {
        GOLD
    } else {
        READY_GREEN
    };
    draw_centered(
        canvas,
        &format!("POINTS REMAINING: {}", creation.points_remaining),
        points_y,
        points_color,
        1,
    );

    let (instructions, instructions_color) = if creation.points_remaining == 0 {
        (
            "UP/DOWN: SELECT   LEFT/RIGHT: ADJUST   ENTER: BEGIN",
            READY_GREEN,
        )
    } else {
        (
            "UP/DOWN: SELECT   LEFT/RIGHT: ADJUST   ENTER: BEGIN (SPEND ALL POINTS FIRST)",
            TEXT_DIM,
        )
    };
    draw_centered(
        canvas,
        instructions,
        PANEL_Y + PANEL_H - 22,
        instructions_color,
        1,
    );
}

fn draw_stat_row(
    canvas: &mut PixelCanvas,
    creation: &CharCreation,
    stat: CreationStat,
    index: usize,
) {
    let row_y = STATS_START_Y + index as i32 * STAT_ROW_H;
    let is_selected = index == creation.selected;

    if is_selected {
        canvas.fill_rect(
            PANEL_X + 4,
            row_y - 2,
            PANEL_W - 8,
            STAT_ROW_H - 4,
            GOLD_HIGHLIGHT_FILL,
        );
        canvas.stroke_rect(
            PANEL_X + 4,
            row_y - 2,
            PANEL_W - 8,
            STAT_ROW_H - 4,
            GOLD_HIGHLIGHT_BORDER,
        );
    }

    let label_color = if is_selected { GOLD } else { TEXT_PRIMARY };
    draw_pixel_text(
        canvas,
        stat.label(),
        PANEL_X + 48,
        row_y + 10,
        label_color,
        1,
    );
    draw_pixel_text(
        canvas,
        stat.description(),
        PANEL_X + 52,
        row_y + 22,
        TEXT_DIM,
        1,
    );

    draw_stat_controls(canvas, creation, stat, row_y, is_selected);
}

fn draw_stat_controls(
    canvas: &mut PixelCanvas,
    creation: &CharCreation,
    stat: CreationStat,
    row_y: i32,
    is_selected: bool,
) {
    let value = creation.stat_value(stat);
    let value_text = format!("{}", value as i64);
    let controls_x = PANEL_X + PANEL_W - 96;
    let value_y = row_y + 6;

    let can_decrease = value > MIN_STAT;
    draw_centered_x(
        canvas,
        "-",
        controls_x + 8,
        value_y + 10,
        control_color(can_decrease, is_selected),
        2,
    );

    let value_color = if is_selected { GOLD } else { TEXT_PRIMARY };
    draw_pixel_text(
        canvas,
        &value_text,
        controls_x + 20,
        value_y + 3,
        value_color,
        2,
    );

    let value_width = measure_pixel_text(&value_text, 2);
    let can_increase = creation.points_remaining > 0;
    draw_centered_x(
        canvas,
        "+",
        controls_x + 20 + value_width + 12,
        value_y + 10,
        control_color(can_increase, is_selected),
        2,
    );
}

fn control_color(enabled: bool, is_selected: bool) -> Rgba {
    if !enabled {
        TEXT_DIM
    } else if is_selected {
        GOLD
    } else {
        TEXT_PRIMARY
    }
}

/// Draw text horizontally centered on the HUD canvas.
fn draw_centered(canvas: &mut PixelCanvas, text: &str, y: i32, color: Rgba, scale: i32) {
    let width = measure_pixel_text(text, scale);
    draw_pixel_text(
        canvas,
        text,
        (HUD_WIDTH as i32 - width) / 2,
        y,
        color,
        scale,
    );
}

/// Draw text centered on an arbitrary x, matching the TS canvas
/// `textAlign = 'center'` buttons.
fn draw_centered_x(
    canvas: &mut PixelCanvas,
    text: &str,
    center_x: i32,
    y: i32,
    color: Rgba,
    scale: i32,
) {
    let width = measure_pixel_text(text, scale);
    draw_pixel_text(canvas, text, center_x - width / 2, y, color, scale);
}

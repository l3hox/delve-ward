//! Attribute allocation panel (`KeyL`), ported from `hud/attributePanel.ts`.
//! Reuses `char_creation.rs`'s staging pattern: points are held locally and
//! only flushed to `GameState::allocate_point` on a successful close, never
//! per keypress.

use crate::hud::{HUD_HEIGHT, HUD_WIDTH};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::session::Session;
use crate::transition::Transition;
use bevy::prelude::*;
use delve_core::game_state::GameState;
use delve_core::inventory_state::AllocatableStat;

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
const ROW_HIGHLIGHT_FILL: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.12);
const ROW_HIGHLIGHT_BORDER: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.4);
const TEXT_PRIMARY: Rgba = Rgba::opaque(0xcc, 0xcc, 0xcc);
const TEXT_DIM: Rgba = Rgba::opaque(0x66, 0x66, 0x66);
const READY_GREEN: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const PENDING_GREEN: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const EFFECTIVE_GREEN: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PanelMode {
    LevelUp,
    Stats,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stat {
    Str,
    Dex,
    Vit,
    Wis,
}

const STATS: [Stat; 4] = [Stat::Str, Stat::Dex, Stat::Vit, Stat::Wis];

impl Stat {
    fn label(self) -> &'static str {
        match self {
            Stat::Str => "STR  STRENGTH",
            Stat::Dex => "DEX  DEXTERITY",
            Stat::Vit => "VIT  VITALITY",
            Stat::Wis => "WIS  WISDOM",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Stat::Str => "MELEE DAMAGE & WEAPON REQS",
            Stat::Dex => "CRIT AND DODGE CHANCE",
            Stat::Vit => "MAX HP",
            Stat::Wis => "MAGIC (NOT YET)",
        }
    }

    fn base_value(self, game: &GameState) -> f64 {
        match self {
            Stat::Str => game.player.str,
            Stat::Dex => game.player.dex,
            Stat::Vit => game.player.vit,
            Stat::Wis => game.player.wis,
        }
    }

    fn effective_value(self, game: &GameState) -> f64 {
        let effective = game.get_effective_stats();
        match self {
            Stat::Str => effective.effective_str,
            Stat::Dex => effective.effective_dex,
            Stat::Vit => effective.effective_vit,
            Stat::Wis => effective.effective_wis,
        }
    }

    fn allocatable(self) -> AllocatableStat {
        match self {
            Stat::Str => AllocatableStat::Str,
            Stat::Dex => AllocatableStat::Dex,
            Stat::Vit => AllocatableStat::Vit,
            Stat::Wis => AllocatableStat::Wis,
        }
    }
}

/// Whether the panel is open is centralized in `ActiveOverlay::AttributePanel`,
/// not a field here, matching every other phase-4 overlay resource.
#[derive(Resource, Default)]
pub struct AttributePanelState {
    mode_levelup: bool,
    selected: usize,
    baseline: [f64; 4],
    pending: [i64; 4],
    total_points: i64,
}

impl AttributePanelState {
    fn mode(&self) -> PanelMode {
        if self.mode_levelup {
            PanelMode::LevelUp
        } else {
            PanelMode::Stats
        }
    }

    fn remaining(&self) -> i64 {
        self.total_points - self.pending.iter().sum::<i64>()
    }

    /// Ported from TS's `AttributePanel.open`: auto-selects levelup mode
    /// when points are available, stats mode otherwise.
    pub fn open(&mut self, game: &GameState) {
        self.selected = 0;
        if game.player.attribute_points > 0 {
            self.mode_levelup = true;
            self.total_points = game.player.attribute_points;
            self.baseline = [
                game.player.str,
                game.player.dex,
                game.player.vit,
                game.player.wis,
            ];
            self.pending = [0; 4];
        } else {
            self.mode_levelup = false;
        }
    }

    /// Ported from TS's `AttributePanel.tryClose`: blocked while levelup
    /// mode has unspent points, otherwise flushes every pending point
    /// through `GameState::allocate_point` (which already preserves the
    /// full-HP invariant on each VIT call, so no separate recompute pass is
    /// needed on top — verified by reasoning through repeated-call
    /// idempotency, since each call re-checks `hp == max_hp` against the
    /// state left by the previous one).
    pub fn try_close(&mut self, game: &mut GameState) -> bool {
        if self.mode() == PanelMode::LevelUp {
            if self.remaining() > 0 {
                return false;
            }
            for (index, stat) in STATS.iter().enumerate() {
                for _ in 0..self.pending[index] {
                    game.allocate_point(stat.allocatable());
                }
            }
        }
        true
    }

    fn cycle_up(&mut self) {
        self.selected = (self.selected + STATS.len() - 1) % STATS.len();
    }

    fn cycle_down(&mut self) {
        self.selected = (self.selected + 1) % STATS.len();
    }

    fn allocate(&mut self) {
        if self.mode() == PanelMode::LevelUp && self.remaining() > 0 {
            self.pending[self.selected] += 1;
        }
    }

    fn retract(&mut self) {
        if self.mode() == PanelMode::LevelUp && self.pending[self.selected] > 0 {
            self.pending[self.selected] -= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Opens on `KeyL` from the dungeon (auto-selecting levelup/stats mode),
/// closes on `KeyL`/`Escape` while open (blocked in levelup mode until all
/// points are spent) — ported from `inputSystem.ts`'s `KeyL` case plus the
/// `attributePanel` branch of its `keydownHandler`.
pub fn attribute_panel_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<ActiveOverlay>,
    mut state: ResMut<AttributePanelState>,
    mut session: ResMut<Session>,
    transition: Res<Transition>,
) {
    if *overlay != ActiveOverlay::AttributePanel {
        if transition.is_active() || *overlay != ActiveOverlay::None {
            return;
        }
        if keys.just_pressed(KeyCode::KeyL) {
            *overlay = ActiveOverlay::AttributePanel;
            state.open(&session.game);
        }
        return;
    }

    if keys.just_pressed(KeyCode::KeyL) || keys.just_pressed(KeyCode::Escape) {
        if state.try_close(&mut session.game) {
            *overlay = ActiveOverlay::None;
        }
        return;
    }

    if keys.just_pressed(KeyCode::ArrowUp) {
        state.cycle_up();
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        state.cycle_down();
    }
    if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::Enter) {
        state.allocate();
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        state.retract();
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

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

pub fn draw_attribute_panel(
    canvas: &mut PixelCanvas,
    state: &AttributePanelState,
    game: &GameState,
) {
    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, OVERLAY);
    canvas.fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BG);
    canvas.stroke_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BORDER);

    match state.mode() {
        PanelMode::LevelUp => draw_levelup(canvas, state, game),
        PanelMode::Stats => draw_stats(canvas, state, game),
    }
}

fn draw_levelup(canvas: &mut PixelCanvas, state: &AttributePanelState, game: &GameState) {
    draw_centered(canvas, "LEVEL UP", PANEL_Y + 12, GOLD, 3);

    let remaining = state.remaining();
    let subtitle = format!(
        "{}  LEVEL {}  {remaining} POINTS REMAINING",
        game.player.player_name.to_uppercase(),
        game.player.level
    );
    draw_centered(
        canvas,
        &subtitle,
        PANEL_Y + 46,
        if remaining > 0 { GOLD } else { READY_GREEN },
        1,
    );

    for (index, &stat) in STATS.iter().enumerate() {
        let row_y = STATS_START_Y + index as i32 * STAT_ROW_H;
        let is_selected = index == state.selected;
        if is_selected {
            canvas.fill_rect(
                PANEL_X + 4,
                row_y - 2,
                PANEL_W - 8,
                STAT_ROW_H - 4,
                ROW_HIGHLIGHT_FILL,
            );
            canvas.stroke_rect(
                PANEL_X + 4,
                row_y - 2,
                PANEL_W - 8,
                STAT_ROW_H - 4,
                ROW_HIGHLIGHT_BORDER,
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

        let pending = state.pending[index];
        let current_val = state.baseline[index] + pending as f64;
        let value_text = crate::hud::format_number(current_val);
        let controls_x = PANEL_X + PANEL_W - 96;
        let value_y = row_y + 6;
        draw_pixel_text(canvas, &value_text, controls_x, value_y + 3, label_color, 2);
        if pending > 0 {
            let value_w = measure_pixel_text(&value_text, 2);
            draw_pixel_text(
                canvas,
                &format!("+{pending}"),
                controls_x + value_w + 4,
                value_y + 3,
                PENDING_GREEN,
                2,
            );
        }
    }

    let all_spent = remaining == 0;
    let footer = if all_spent {
        "L/ENTER CONFIRM   LEFT/RIGHT ADJUST"
    } else {
        "SPEND ALL POINTS TO CLOSE   LEFT/RIGHT ADJUST"
    };
    draw_centered(
        canvas,
        footer,
        PANEL_Y + PANEL_H - 18,
        if all_spent { READY_GREEN } else { GOLD },
        1,
    );
}

fn draw_stats(canvas: &mut PixelCanvas, state: &AttributePanelState, game: &GameState) {
    draw_centered(canvas, "ATTRIBUTES", PANEL_Y + 12, GOLD, 3);
    draw_centered(
        canvas,
        &format!(
            "{}  LEVEL {}",
            game.player.player_name.to_uppercase(),
            game.player.level
        ),
        PANEL_Y + 46,
        TEXT_DIM,
        1,
    );

    for (index, &stat) in STATS.iter().enumerate() {
        let row_y = STATS_START_Y + index as i32 * STAT_ROW_H;
        let is_selected = index == state.selected;
        if is_selected {
            canvas.fill_rect(
                PANEL_X + 4,
                row_y - 2,
                PANEL_W - 8,
                STAT_ROW_H - 4,
                ROW_HIGHLIGHT_FILL,
            );
            canvas.stroke_rect(
                PANEL_X + 4,
                row_y - 2,
                PANEL_W - 8,
                STAT_ROW_H - 4,
                ROW_HIGHLIGHT_BORDER,
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

        let base_val = stat.base_value(game);
        let eff_val = stat.effective_value(game);
        let base_str = crate::hud::format_number(base_val);
        let controls_x = PANEL_X + PANEL_W - 96;
        let value_y = row_y + 6;
        draw_pixel_text(canvas, &base_str, controls_x, value_y + 3, label_color, 2);
        if (eff_val - base_val).abs() > f64::EPSILON {
            let base_w = measure_pixel_text(&base_str, 2);
            draw_pixel_text(
                canvas,
                &crate::hud::format_number(eff_val),
                controls_x + base_w + 6,
                value_y + 3,
                EFFECTIVE_GREEN,
                2,
            );
        }
    }

    draw_centered(canvas, "L CLOSE", PANEL_Y + PANEL_H - 18, TEXT_DIM, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_with_points(points: i64) -> GameState {
        let mut game = GameState::new(
            &[],
            None,
            "test_level",
            None,
            delve_core::game_state::GameStateDeps::default(),
            &mut || 0.0,
        );
        game.player.attribute_points = points;
        game
    }

    #[test]
    fn opens_in_stats_mode_without_points() {
        let game = game_with_points(0);
        let mut state = AttributePanelState::default();
        state.open(&game);
        assert_eq!(state.mode(), PanelMode::Stats);
    }

    #[test]
    fn opens_in_levelup_mode_with_points() {
        let game = game_with_points(3);
        let mut state = AttributePanelState::default();
        state.open(&game);
        assert_eq!(state.mode(), PanelMode::LevelUp);
    }

    #[test]
    fn try_close_succeeds_immediately_in_stats_mode() {
        let mut game = game_with_points(0);
        let mut state = AttributePanelState::default();
        state.open(&game);
        assert!(state.try_close(&mut game));
    }

    #[test]
    fn try_close_fails_in_levelup_mode_with_unspent_points() {
        let mut game = game_with_points(3);
        let mut state = AttributePanelState::default();
        state.open(&game);
        assert!(!state.try_close(&mut game));
    }

    #[test]
    fn try_close_succeeds_once_all_points_spent() {
        let mut game = game_with_points(3);
        let mut state = AttributePanelState::default();
        state.open(&game);
        state.allocate();
        state.allocate();
        state.allocate();
        assert!(state.try_close(&mut game));
        assert_eq!(game.player.attribute_points, 0);
    }

    #[test]
    fn allocation_does_not_touch_game_state_until_close() {
        let mut game = game_with_points(3);
        let str_before = game.player.str;
        let mut state = AttributePanelState::default();
        state.open(&game);
        state.allocate();
        state.allocate();
        assert!((game.player.str - str_before).abs() < f64::EPSILON);
        state.allocate();
        assert!(state.try_close(&mut game));
        assert!((game.player.str - (str_before + 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn cannot_allocate_more_than_remaining() {
        let mut game = game_with_points(2);
        let str_before = game.player.str;
        let mut state = AttributePanelState::default();
        state.open(&game);
        state.allocate();
        state.allocate();
        state.allocate(); // no-op, nothing remaining
        assert!(state.try_close(&mut game));
        assert!((game.player.str - (str_before + 2.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn retract_removes_a_pending_point_and_floors_at_baseline() {
        let mut game = game_with_points(3);
        let str_before = game.player.str;
        let mut state = AttributePanelState::default();
        state.open(&game);
        state.retract(); // no pending to remove — no-op
        state.allocate();
        state.allocate();
        state.retract();
        // Net +1 STR, then spend the remaining 2 on DEX.
        state.cycle_down();
        state.allocate();
        state.allocate();
        assert!(state.try_close(&mut game));
        assert!((game.player.str - (str_before + 1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn cycle_wraps_in_both_directions() {
        let game = game_with_points(1);
        let mut state = AttributePanelState::default();
        state.open(&game);
        state.cycle_up();
        assert_eq!(state.selected, STATS.len() - 1);
        state.cycle_down();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn vit_allocation_recalculates_max_hp_and_preserves_full_hp() {
        let mut game = game_with_points(3);
        game.player.hp = game.player.max_hp;
        let max_hp_before = game.player.max_hp;
        let mut state = AttributePanelState::default();
        state.open(&game);
        state.cycle_down();
        state.cycle_down(); // STR -> DEX -> VIT
        state.allocate();
        state.allocate();
        state.allocate();
        assert!(state.try_close(&mut game));
        assert!(game.player.max_hp > max_hp_before);
        assert!((game.player.hp - game.player.max_hp).abs() < f64::EPSILON);
    }

    #[test]
    fn resets_selected_stat_on_reopen() {
        let mut game = game_with_points(6);
        let mut state = AttributePanelState::default();
        state.open(&game);
        state.cycle_down();
        state.cycle_down();
        for _ in 0..6 {
            state.allocate();
        }
        assert!(state.try_close(&mut game));

        game.player.attribute_points = 3;
        state.open(&game);
        assert_eq!(state.selected, 0);
        let str_before = game.player.str;
        state.allocate();
        for _ in 0..2 {
            state.allocate();
        }
        assert!(state.try_close(&mut game));
        assert!((game.player.str - (str_before + 3.0)).abs() < f64::EPSILON);
    }
}

//! Save/load overlay and the unified death flow, ported from
//! `hud/saveLoadOverlay.ts` and its two call sites in `main.ts`
//! (`Escape` opens it in save mode; `gameState.hp <= 0` opens it in death
//! mode, or restarts directly when no saves exist).
//!
//! TS's overlay is entirely mouse-driven (per-slot Save/Load/Delete
//! buttons; only `Escape` is a keyboard affordance) — this port has no
//! mouse input wired yet, so it follows `char_creation.rs`'s established
//! keyboard-modal pattern instead: arrow keys move a slot cursor, dedicated
//! keys act on the selected slot. The underlying slot semantics (autosave
//! has no Save action; death mode hides Save/Delete; `Escape` always closes,
//! even in death mode, letting the death check reopen it next frame if
//! still dead) all match TS exactly.

use crate::hud::{HUD_HEIGHT, HUD_WIDTH, HudState};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::player::Player;
use crate::save_store::FileSaveStore;
use crate::session::{DungeonRes, LevelSnapshots, Session};
use crate::transition::Transition;
use bevy::prelude::*;
use delve_core::grid::Facing;
use delve_core::save_system::{
    AUTOSAVE_KEY, BuildSaveDataParams, SAVE_SLOT_KEYS, SlotMetadata, build_save_data,
    get_all_slot_metadata, load_from_slot, save_to_slot,
};
use std::collections::HashMap;

const SLOT_COUNT: usize = SAVE_SLOT_KEYS.len() + 1;

fn slot_key_at(index: usize) -> &'static str {
    if index < SAVE_SLOT_KEYS.len() {
        SAVE_SLOT_KEYS[index]
    } else {
        AUTOSAVE_KEY
    }
}

fn slot_label(index: usize) -> String {
    if index < SAVE_SLOT_KEYS.len() {
        format!("SLOT {}", index + 1)
    } else {
        "AUTOSAVE".to_string()
    }
}

/// Save/load modal data. Whether it's open is centralized in
/// `ActiveOverlay::SaveLoad`, not a field here. `is_death` mirrors TS's
/// `isDeath` flag: it hides the Save/Delete actions and shows the Restart
/// action instead.
#[derive(Resource, Default)]
pub struct SaveLoadOverlay {
    pub is_death: bool,
    pub selected: usize,
}

// ---------------------------------------------------------------------------
// Building and writing a save, shared by the autosave call in transition.rs
// and this overlay's manual-save key — ported from main.ts's `saveGame`.
// ---------------------------------------------------------------------------

fn current_timestamp_millis() -> i64 {
    // Matches the existing fallback style in `main.rs::setup`'s RNG seed:
    // silently fall back rather than propagate an error that can only occur
    // if the system clock is set before the Unix epoch.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Builds and writes a save to `slot_key`. Returns `false` (and lets the
/// caller show a failure message) on write failure, matching TS's
/// `saveToSlot` boolean result.
///
/// Takes the player's position/facing as plain values rather than reading a
/// live `Player` component: the autosave call in `transition.rs` runs
/// mid-swap, when the `Player` component still reflects the pre-transition
/// cell (`Commands`-driven component replacement is deferred) but the save
/// must record the new spawn position, matching TS's own autosave timing
/// (after `ctx.setLs(newLs)`, so `ls.player.getState()` already reflects the
/// new position there).
#[allow(clippy::too_many_arguments)]
pub fn save_game_to_slot(
    store: &mut FileSaveStore,
    slot_key: &str,
    session: &Session,
    player_col: i32,
    player_row: i32,
    player_facing: Facing,
    dungeon: &DungeonRes,
    snapshots: &LevelSnapshots,
    quests: &delve_core::quest_manager::QuestManager,
) -> bool {
    let data = build_save_data(BuildSaveDataParams {
        game_state: &session.game,
        player_col: i64::from(player_col),
        player_row: i64::from(player_row),
        player_facing,
        current_level_id: session.current_level_id.clone(),
        level_snapshots: &snapshots.0,
        dungeon: &dungeon.0,
        timestamp: current_timestamp_millis(),
        quests: Some(quests.get_serializable_state()),
    });
    save_to_slot(store, slot_key, &data)
}

// ---------------------------------------------------------------------------
// Death flow
// ---------------------------------------------------------------------------

/// The alive-to-dead transition, checked once per frame in the same gated
/// position TS's own single `if (gameState.hp <= 0)` check occupies (right
/// after all combat/status ticking for the frame) — not duplicated at every
/// site that can zero HP. Fires exactly once: opening the overlay (or
/// restarting immediately when no saves exist) makes the blocked-check true
/// on the next frame, which stops this system from running again until the
/// overlay closes with HP restored.
///
/// Takes `Transition`/`ActiveOverlay` directly rather than the shared
/// `InputGate` `SystemParam`, since this system also needs `ResMut` access
/// to `ActiveOverlay`/`Transition` themselves — `InputGate` borrowing those
/// immutably at the same time would conflict.
pub fn check_player_death(
    session: Res<Session>,
    mut overlay: ResMut<ActiveOverlay>,
    mut save_load: ResMut<SaveLoadOverlay>,
    save_store: Res<FileSaveStore>,
    mut transition: ResMut<Transition>,
) {
    if transition.is_active() || overlay.is_open() {
        return;
    }
    if session.game.player.hp > 0.0 {
        return;
    }
    info!("You died.");
    let has_saves = get_all_slot_metadata(&*save_store)
        .values()
        .any(Option::is_some);
    if has_saves {
        *overlay = ActiveOverlay::SaveLoad;
        save_load.is_death = true;
        save_load.selected = 0;
    } else {
        transition.begin_restart();
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn save_load_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<ActiveOverlay>,
    mut save_load: ResMut<SaveLoadOverlay>,
    mut transition: ResMut<Transition>,
    mut save_store: ResMut<FileSaveStore>,
    session: Res<Session>,
    dungeon: Res<DungeonRes>,
    snapshots: Res<LevelSnapshots>,
    players: Query<&Player>,
    mut hud: ResMut<HudState>,
    quests: Res<crate::dialog_overlay::QuestManagerRes>,
) {
    if *overlay != ActiveOverlay::SaveLoad {
        // Same two conditions `InputGate::blocked()` checks; inlined here
        // because this system also needs `ResMut<Transition>` below, which
        // would conflict with `InputGate`'s own `Res<Transition>`. Checking
        // `!= None` (not just "is it CharCreation") also blocks Escape from
        // stealing whatever other overlay a later slice adds.
        if transition.is_active() || *overlay != ActiveOverlay::None {
            return;
        }
        if keys.just_pressed(KeyCode::Escape) {
            *overlay = ActiveOverlay::SaveLoad;
            save_load.is_death = false;
            save_load.selected = 0;
        }
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        // Unconditional, even in death mode — matches TS's `_keyHandler`,
        // which has no death-mode exception. `check_player_death` reopens
        // this same frame's-worth-later if the player is still dead and
        // saves exist (or restarts if not), the same one-frame "flicker"
        // TS produces.
        *overlay = ActiveOverlay::None;
        return;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        save_load.selected = (save_load.selected + SLOT_COUNT - 1) % SLOT_COUNT;
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        save_load.selected = (save_load.selected + 1) % SLOT_COUNT;
    }

    let selected = save_load.selected;
    let slot_key = slot_key_at(selected);
    let is_autosave = selected == SAVE_SLOT_KEYS.len();

    // Save — manual slots only, hidden entirely in death mode, matching TS
    // (`if (!isAutosave) { saveBtn = ...}`, `if (this.isDeath) saveBtn.hide()`).
    if !save_load.is_death
        && !is_autosave
        && keys.just_pressed(KeyCode::KeyS)
        && let Ok(player) = players.single()
    {
        let player_state = player.grid_state();
        // TS's `onSave` shows "Game saved." unconditionally after calling
        // `saveGame`, even though `saveGame` itself already showed a
        // "Save failed" message on write failure — the second message
        // silently overwrites the first in both languages, ported
        // faithfully rather than fixed.
        let ok = save_game_to_slot(
            &mut save_store,
            slot_key,
            &session,
            player_state.col,
            player_state.row,
            player_state.facing,
            &dungeon,
            &snapshots,
            &quests.0,
        );
        if !ok {
            hud.show_message("Save failed — storage full!");
        }
        *overlay = ActiveOverlay::None;
        hud.show_message("Game saved.");
    }

    // Delete — hidden in death mode, matching TS.
    if !save_load.is_death
        && (keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace))
    {
        delve_core::save_system::delete_slot(&mut *save_store, slot_key);
    }

    // Load — any populated slot, any mode, matching TS's mode-independent
    // Load button.
    if keys.just_pressed(KeyCode::Enter) {
        let Some(data) = load_from_slot(&*save_store, slot_key) else {
            *overlay = ActiveOverlay::None;
            hud.show_message("Failed to load save.");
            return;
        };
        if data.dungeon_name != dungeon.0.name {
            *overlay = ActiveOverlay::None;
            hud.show_message("Save is from a different dungeon.");
            return;
        }
        *overlay = ActiveOverlay::None;
        transition.begin_load(data);
    }

    // Restart — death mode only, matching TS's Restart button visibility.
    if save_load.is_death && keys.just_pressed(KeyCode::KeyR) {
        *overlay = ActiveOverlay::None;
        transition.begin_restart();
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const PANEL_W: i32 = 480;
const PANEL_H: i32 = 300;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const PANEL_Y: i32 = (HUD_HEIGHT as i32 - PANEL_H) / 2;
const ROW_H: i32 = 32;
const ROWS_START_Y: i32 = PANEL_Y + 56;

const BACKDROP: Rgba = Rgba::translucent(0, 0, 0, 0.7);
const PANEL_BG: Rgba = Rgba::opaque(0x0f, 0x0e, 0x18);
const PANEL_BORDER: Rgba = Rgba::opaque(0x3a, 0x36, 0x50);
const TITLE_TEXT: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const DEATH_TITLE: Rgba = Rgba::opaque(0xcc, 0x33, 0x33);
const TEXT: Rgba = Rgba::opaque(0xc0, 0xc0, 0xc0);
const TEXT_EMPTY: Rgba = Rgba::opaque(0x44, 0x43, 0x5a);
const SELECTED_FILL: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.12);
const SELECTED_BORDER: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.4);
const HINT_TEXT: Rgba = Rgba::opaque(0x7a, 0x6a, 0x4a);

/// Epoch milliseconds -> "YYYY-MM-DD HH:MM" (UTC). Hand-rolled via Howard
/// Hinnant's `civil_from_days` algorithm rather than pulling in a datetime
/// crate for one save-slot label.
fn format_timestamp(epoch_millis: i64) -> String {
    let total_seconds = epoch_millis.div_euclid(1000);
    let days = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

fn slot_info_line(meta: Option<&SlotMetadata>) -> (String, Rgba) {
    match meta {
        Some(meta) => (
            format!(
                "{}  {}  {}  LV{}",
                format_timestamp(meta.saved_at),
                meta.player_name.to_uppercase(),
                meta.level_id.to_uppercase(),
                meta.character_level
            ),
            TEXT,
        ),
        None => ("EMPTY".to_string(), TEXT_EMPTY),
    }
}

pub fn draw_save_load_overlay(
    canvas: &mut PixelCanvas,
    overlay: &SaveLoadOverlay,
    metadata: &HashMap<String, Option<SlotMetadata>>,
) {
    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, BACKDROP);
    canvas.fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BG);
    canvas.stroke_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, PANEL_BORDER);

    let (title, title_color) = if overlay.is_death {
        ("YOU HAVE DIED. LOAD A SAVE?", DEATH_TITLE)
    } else {
        ("SAVE GAME", TITLE_TEXT)
    };
    draw_centered(canvas, title, PANEL_Y + 16, title_color, 2);

    for index in 0..SLOT_COUNT {
        let row_y = ROWS_START_Y + index as i32 * ROW_H;
        let is_selected = index == overlay.selected;
        if is_selected {
            canvas.fill_rect(
                PANEL_X + 4,
                row_y - 2,
                PANEL_W - 8,
                ROW_H - 4,
                SELECTED_FILL,
            );
            canvas.stroke_rect(
                PANEL_X + 4,
                row_y - 2,
                PANEL_W - 8,
                ROW_H - 4,
                SELECTED_BORDER,
            );
        }
        let label_color = if is_selected { TITLE_TEXT } else { TEXT };
        draw_pixel_text(
            canvas,
            &slot_label(index),
            PANEL_X + 16,
            row_y + 8,
            label_color,
            1,
        );
        let (info, info_color) =
            slot_info_line(metadata.get(slot_key_at(index)).and_then(Option::as_ref));
        draw_pixel_text(canvas, &info, PANEL_X + 96, row_y + 8, info_color, 1);
    }

    let hint = if overlay.is_death {
        "UP/DOWN: SELECT   ENTER: LOAD   R: RESTART"
    } else {
        "UP/DOWN: SELECT   ENTER: LOAD   S: SAVE   DEL: DELETE   ESC: CLOSE"
    };
    draw_centered(canvas, hint, PANEL_Y + PANEL_H - 20, HINT_TEXT, 1);
}

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

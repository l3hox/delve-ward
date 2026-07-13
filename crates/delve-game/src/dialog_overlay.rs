//! NPC dialog panel, ported from `hud/dialogOverlay.ts` and its wiring in
//! `main.ts`/`game/inputSystem.ts`. TS builds one DOM panel once and mutates
//! it in place; there is no canvas draw function to port 1:1, so the layout
//! here is a fresh HUD-canvas design carrying the same visual intent
//! (speaker name, body text, a vertical choice list, a hint line, bottom-
//! center placement) and the same warm bronze/gold palette as the TS CSS.
//!
//! `DialogEvent` handling (`StartQuest`/`AdvanceQuest`/`OpenShop`) replaces
//! TS's `setDialogHooks` callback registration — see `dialog_manager.rs`'s
//! module doc for why the hook pattern doesn't survive the port. Every place
//! a `Vec<DialogEvent>` comes back from `dialog_manager` (opening a dialog's
//! start node, selecting a choice, advancing linearly) routes through
//! [`apply_dialog_events`] so the three effect types behave identically
//! regardless of which of those three call sites produced them.

use crate::hud::{HUD_HEIGHT, HUD_WIDTH, HudState};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::session::Session;
use crate::trading_overlay::TradingOverlayState;
use bevy::prelude::*;
use delve_core::dialog_manager::{
    DialogEvent, DialogSession, advance_dialog, execute_effects, get_available_choices,
    get_current_node, select_choice, start_dialog,
};
use delve_core::dialogs::DialogTree;
use delve_core::npcs::{NpcDatabase, NpcDef};
use delve_core::quest_manager::{QuestManager, QuestStatus};
use std::collections::HashMap;

/// Wraps the core `QuestManager` as a Bevy resource, mirroring how
/// `enemies::EnemyDb`/`ground_items::ItemDb` wrap their own `delve-core`
/// databases.
#[derive(Resource, Default)]
pub struct QuestManagerRes(pub QuestManager);

/// Parsed dialog trees by dialog id, loaded and cached the first time an NPC
/// with that `dialog` field is interacted with — the synchronous-file-read
/// equivalent of TS's fetch-and-cache-by-id in `loadDialog`.
#[derive(Resource, Default)]
pub struct DialogTreeCache(HashMap<String, DialogTree>);

fn load_dialog_tree<'cache>(
    cache: &'cache mut DialogTreeCache,
    dialog_id: &str,
) -> Result<&'cache DialogTree, String> {
    if !cache.0.contains_key(dialog_id) {
        let path = crate::assets_dir().join(format!("data/dialogs/{dialog_id}.json"));
        let json = std::fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let tree = DialogTree::from_json(&json)?;
        cache.0.insert(dialog_id.to_string(), tree);
    }
    Ok(&cache.0[dialog_id])
}

/// The active dialog session, if any — whether the panel is open is
/// centralized in `ActiveOverlay::Dialog`, not a field here, matching the
/// convention every other phase-4 overlay resource follows.
#[derive(Resource, Default)]
pub struct DialogOverlayState {
    pub session: Option<DialogSession>,
    /// -1 means no choice highlighted, matching TS's `highlightedIndex`
    /// initial/reset value. Reset to -1 every time the current node changes
    /// (a fresh `show()` in TS), not just when the dialog first opens.
    pub highlighted: i32,
}

/// Dependencies the dialog-opening and dialog-input paths both need to apply
/// `DialogEvent`s the same way `main.ts`'s `setDialogHooks` callbacks did —
/// bundled so both call sites stay in sync without duplicating the
/// start-quest/advance-quest/open-shop match.
pub struct DialogEventSink<'a> {
    pub quests: &'a mut QuestManager,
    pub overlay: &'a mut ActiveOverlay,
    pub dialog_state: &'a mut DialogOverlayState,
    pub hud: &'a mut HudState,
    pub npc_db: &'a NpcDatabase,
    pub trading_state: &'a mut TradingOverlayState,
}

/// Applies every `DialogEvent` from a dialog step, in order — ported from
/// `main.ts:293-317`'s `setDialogHooks` callbacks.
fn apply_dialog_events(
    events: Vec<DialogEvent>,
    game: &mut delve_core::game_state::GameState,
    sink: &mut DialogEventSink,
) {
    for event in events {
        match event {
            DialogEvent::StartQuest(quest_id) => {
                sink.quests.start_quest(&quest_id);
                let name = sink
                    .quests
                    .get_quest_def(&quest_id)
                    .map_or_else(|| quest_id.clone(), |def| def.name.clone());
                sink.hud.show_message(&format!("Quest started: {name}"));
            }
            DialogEvent::AdvanceQuest(quest_id) => {
                sink.quests.advance_quest(&quest_id, game);
                let name = sink
                    .quests
                    .get_quest_def(&quest_id)
                    .map_or_else(|| quest_id.clone(), |def| def.name.clone());
                if sink.quests.get_status(&quest_id) == QuestStatus::Complete {
                    sink.hud.show_message(&format!("Quest complete: {name}"));
                } else {
                    sink.hud.show_message(&format!("Quest updated: {name}"));
                }
            }
            DialogEvent::OpenShop(npc_id) => {
                // TS: `const def = npcDatabase.getNpc(npcId); if (!def ||
                // !def.stock) return;` — bails silently (dialog stays open,
                // nothing changes) when the npc has no stock, matching
                // `main.ts:310-316`'s guard exactly rather than falling
                // back to some default trading state.
                match sink.npc_db.get_npc(&npc_id) {
                    Some(def) if def.stock.is_some() => {
                        sink.trading_state.npc_id = npc_id.clone();
                        *sink.overlay = ActiveOverlay::Trading;
                        sink.dialog_state.session = None;
                    }
                    _ => {
                        warn!("dialog requested trading for npc '{npc_id}', which has no stock");
                    }
                }
            }
        }
    }
}

/// Opens the dialog panel for an interacted NPC, ported from
/// `inputSystem.ts:218-237`'s `npc_interacted` case: loads the NPC's dialog
/// tree (cached after the first read), starts a session, runs the start
/// node's own entry effects (TS calls `executeEffects(startNode.effects,
/// ctx.gameState)` explicitly right after `startDialog` — `start_dialog`
/// itself does not run them), and opens the overlay. On a missing or
/// unparseable dialog file, shows the NPC's name with an ellipsis instead of
/// panicking, matching TS's `.catch` fallback.
#[allow(clippy::too_many_arguments)]
pub fn open_dialog_for_npc(
    npc_id: &str,
    npc_def: &NpcDef,
    cache: &mut DialogTreeCache,
    game: &mut delve_core::game_state::GameState,
    dialog_state: &mut DialogOverlayState,
    overlay: &mut ActiveOverlay,
    quests: &mut QuestManager,
    hud: &mut HudState,
    npc_db: &NpcDatabase,
    trading_state: &mut TradingOverlayState,
) {
    let tree = match load_dialog_tree(cache, &npc_def.dialog) {
        Ok(tree) => tree.clone(),
        Err(error) => {
            warn!("failed to load dialog '{}': {error}", npc_def.dialog);
            hud.show_message(&format!("{}: \"...\"", npc_def.name));
            return;
        }
    };

    let session = start_dialog(npc_id, tree);
    let events = match get_current_node(&session) {
        Some(node) => execute_effects(node.effects.as_deref(), game, npc_id),
        None => Vec::new(),
    };
    dialog_state.session = Some(session);
    dialog_state.highlighted = -1;
    *overlay = ActiveOverlay::Dialog;
    apply_dialog_events(
        events,
        game,
        &mut DialogEventSink {
            quests,
            overlay,
            dialog_state,
            hud,
            npc_db,
            trading_state,
        },
    );
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Dependencies `dialog_input` needs beyond the keyboard and the overlay
/// flag itself, bundled to stay under the argument-count lint.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DialogInputEffects<'w> {
    session: ResMut<'w, Session>,
    dialog_state: ResMut<'w, DialogOverlayState>,
    quests: ResMut<'w, QuestManagerRes>,
    hud: ResMut<'w, HudState>,
    npc_db: Res<'w, crate::npcs::NpcDb>,
    trading_state: ResMut<'w, TradingOverlayState>,
}

/// Self-contained keydown handling, ported from `dialogOverlay.ts:106-145`.
/// Escape always dismisses; with choices available, arrows cycle-and-wrap
/// the highlight, Enter confirms the highlighted choice (a no-op with
/// nothing highlighted, matching TS's `this.highlightedIndex >= 0` guard),
/// and digits 1-9 select directly regardless of highlight; with no choices,
/// any key advances.
pub fn dialog_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<ActiveOverlay>,
    mut effects: DialogInputEffects,
) {
    if *overlay != ActiveOverlay::Dialog {
        return;
    }
    if effects.dialog_state.session.is_none() {
        *overlay = ActiveOverlay::None;
        return;
    }

    if keys.just_pressed(KeyCode::Escape) {
        effects.dialog_state.session = None;
        *overlay = ActiveOverlay::None;
        return;
    }

    // Scoped so this immutable borrow of `dialog_state` ends before the
    // highlight-cycling logic below needs a mutable one.
    let choice_count = {
        let session = effects
            .dialog_state
            .session
            .as_ref()
            .expect("checked non-None above");
        get_available_choices(session, &effects.session.game, Some(&effects.quests.0)).len()
    };

    let chosen_index = if choice_count > 0 {
        if keys.just_pressed(KeyCode::ArrowDown) {
            let next = if effects.dialog_state.highlighted < choice_count as i32 - 1 {
                effects.dialog_state.highlighted + 1
            } else {
                0
            };
            effects.dialog_state.highlighted = next;
            None
        } else if keys.just_pressed(KeyCode::ArrowUp) {
            let next = if effects.dialog_state.highlighted > 0 {
                effects.dialog_state.highlighted - 1
            } else {
                choice_count as i32 - 1
            };
            effects.dialog_state.highlighted = next;
            None
        } else if keys.just_pressed(KeyCode::Enter) && effects.dialog_state.highlighted >= 0 {
            Some(effects.dialog_state.highlighted as usize)
        } else {
            digit_just_pressed(&keys)
        }
    } else {
        None
    };

    let advancing_without_choices = choice_count == 0 && any_key_just_pressed(&keys);

    if let Some(index) = chosen_index {
        apply_dialog_step(&mut effects, &mut overlay, |session, game, quests| {
            select_choice(session, index, game, Some(quests))
        });
    } else if advancing_without_choices {
        apply_dialog_step(&mut effects, &mut overlay, |session, game, _quests| {
            advance_dialog(session, game)
        });
    }
}

/// Digits 1-9 map to choice indices 0-8, matching TS's `parseInt(e.key)`
/// check (`num >= 1 && num <= 9`).
fn digit_just_pressed(keys: &ButtonInput<KeyCode>) -> Option<usize> {
    const DIGITS: [(KeyCode, usize); 9] = [
        (KeyCode::Digit1, 0),
        (KeyCode::Digit2, 1),
        (KeyCode::Digit3, 2),
        (KeyCode::Digit4, 3),
        (KeyCode::Digit5, 4),
        (KeyCode::Digit6, 5),
        (KeyCode::Digit7, 6),
        (KeyCode::Digit8, 7),
        (KeyCode::Digit9, 8),
    ];
    DIGITS
        .into_iter()
        .find(|(code, _)| keys.just_pressed(*code))
        .map(|(_, index)| index)
}

fn any_key_just_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.get_just_pressed().next().is_some()
}

/// Runs a session-mutating dialog step (`select_choice` or `advance_dialog`),
/// applies the events it returns, and either shows the new node or closes
/// the panel when the step ends the dialog — the shared tail of
/// `dialogOverlay.setOnChoiceSelected`/`setOnAdvance` in `inputSystem.ts`.
fn apply_dialog_step(
    effects: &mut DialogInputEffects,
    overlay: &mut ActiveOverlay,
    step: impl FnOnce(
        &mut DialogSession,
        &mut delve_core::game_state::GameState,
        &QuestManager,
    ) -> (Option<String>, Vec<DialogEvent>),
) {
    let Some(mut session) = effects.dialog_state.session.take() else {
        return;
    };
    let (next, events) = step(&mut session, &mut effects.session.game, &effects.quests.0);
    if next.is_some() {
        effects.dialog_state.session = Some(session);
        effects.dialog_state.highlighted = -1;
    } else {
        effects.dialog_state.session = None;
        *overlay = ActiveOverlay::None;
    }
    apply_dialog_events(
        events,
        &mut effects.session.game,
        &mut DialogEventSink {
            quests: &mut effects.quests.0,
            overlay,
            dialog_state: &mut effects.dialog_state,
            hud: &mut effects.hud,
            npc_db: &effects.npc_db.0,
            trading_state: &mut effects.trading_state,
        },
    );
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const PANEL_W: i32 = 440;
const PANEL_MARGIN_BOTTOM: i32 = 20;
const PANEL_PAD: i32 = 16;
const PANEL_X: i32 = (HUD_WIDTH as i32 - PANEL_W) / 2;
const LINE_HEIGHT: i32 = 12;
const TEXT_WRAP_CHARS: usize = 56;
const CHOICE_ROW_H: i32 = 16;

const BACKDROP: Rgba = Rgba::translucent(0, 0, 0, 0.5);
const PANEL_BG: Rgba = Rgba::opaque(0x22, 0x14, 0x08);
const PANEL_BORDER: Rgba = Rgba::opaque(0x8b, 0x69, 0x14);
const SPEAKER_TEXT: Rgba = Rgba::opaque(0xd4, 0xa8, 0x17);
const BODY_TEXT: Rgba = Rgba::opaque(0xe0, 0xd0, 0xb0);
const CHOICE_BG: Rgba = Rgba::translucent(0x8b, 0x69, 0x14, 0.15);
const CHOICE_BG_HIGHLIGHT: Rgba = Rgba::translucent(0x8b, 0x69, 0x14, 0.35);
const CHOICE_BORDER: Rgba = Rgba::opaque(0x5a, 0x45, 0x10);
const CHOICE_BORDER_HIGHLIGHT: Rgba = Rgba::opaque(0x8b, 0x69, 0x14);
const CHOICE_TEXT: Rgba = Rgba::opaque(0xd4, 0xc5, 0xa0);
const HINT_TEXT: Rgba = Rgba::opaque(0x7a, 0x6a, 0x4a);

/// Greedy word-wrap at `TEXT_WRAP_CHARS` columns — the pixel font has no
/// lowercase glyphs, so every line is drawn uppercased, matching the rest of
/// this HUD's text convention.
pub(crate) fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate_len = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if candidate_len > max_chars && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Draws the dialog panel from the current node's text and available
/// choices, bottom-center — the canvas equivalent of TS's `DialogOverlay.show`
/// (called on every node the session enters) plus `setHighlight`.
pub fn draw_dialog_overlay(
    canvas: &mut PixelCanvas,
    state: &DialogOverlayState,
    game: &delve_core::game_state::GameState,
    quests: &QuestManager,
) {
    let Some(session) = &state.session else {
        return;
    };
    let Some(node) = get_current_node(session) else {
        return;
    };
    let choices = get_available_choices(session, game, Some(quests));

    let body_lines = wrap_text(&node.text.to_uppercase(), TEXT_WRAP_CHARS);
    let speaker_h = if node.speaker.is_some() {
        LINE_HEIGHT
    } else {
        0
    };
    let body_h = body_lines.len() as i32 * LINE_HEIGHT;
    let choices_h = if choices.is_empty() {
        0
    } else {
        choices.len() as i32 * CHOICE_ROW_H + 6
    };
    let hint_h = LINE_HEIGHT + 6;
    let panel_h = PANEL_PAD * 2 + speaker_h + body_h + choices_h + hint_h;
    let panel_y = HUD_HEIGHT as i32 - PANEL_MARGIN_BOTTOM - panel_h;

    canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, BACKDROP);
    canvas.fill_rect(PANEL_X, panel_y, PANEL_W, panel_h, PANEL_BG);
    canvas.stroke_rect(PANEL_X, panel_y, PANEL_W, panel_h, PANEL_BORDER);

    let mut cursor_y = panel_y + PANEL_PAD;
    if let Some(speaker) = &node.speaker {
        draw_pixel_text(
            canvas,
            &speaker.to_uppercase(),
            PANEL_X + PANEL_PAD,
            cursor_y,
            SPEAKER_TEXT,
            1,
        );
        cursor_y += speaker_h;
    }
    for line in &body_lines {
        draw_pixel_text(canvas, line, PANEL_X + PANEL_PAD, cursor_y, BODY_TEXT, 1);
        cursor_y += LINE_HEIGHT;
    }

    if !choices.is_empty() {
        cursor_y += 6;
        for (index, choice) in choices.iter().enumerate() {
            let highlighted = state.highlighted == index as i32;
            let (bg, border) = if highlighted {
                (CHOICE_BG_HIGHLIGHT, CHOICE_BORDER_HIGHLIGHT)
            } else {
                (CHOICE_BG, CHOICE_BORDER)
            };
            let row_y = cursor_y + index as i32 * CHOICE_ROW_H;
            canvas.fill_rect(
                PANEL_X + PANEL_PAD,
                row_y,
                PANEL_W - PANEL_PAD * 2,
                CHOICE_ROW_H - 2,
                bg,
            );
            canvas.stroke_rect(
                PANEL_X + PANEL_PAD,
                row_y,
                PANEL_W - PANEL_PAD * 2,
                CHOICE_ROW_H - 2,
                border,
            );
            draw_pixel_text(
                canvas,
                &format!("{}. {}", index + 1, choice.text.to_uppercase()),
                PANEL_X + PANEL_PAD + 4,
                row_y + 3,
                CHOICE_TEXT,
                1,
            );
        }
        cursor_y += choices.len() as i32 * CHOICE_ROW_H;
    }

    cursor_y += 6;
    let hint = if choices.is_empty() {
        "PRESS ANY KEY TO CONTINUE"
    } else {
        "1-9, ARROWS + ENTER, OR ESC"
    };
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

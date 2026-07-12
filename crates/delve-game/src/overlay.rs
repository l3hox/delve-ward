//! Central "which modal is open" state, replacing a boolean flag on each
//! overlay's own resource. Mirrors TS's `anyOverlayOpen` (a derived OR
//! across every overlay's own `visible` flag) as a single enum instead — at
//! most one overlay is ever open, so the enum is the more precise model and
//! gives every future overlay (dialog, trading, quest log, inventory,
//! attribute, stats) one shared place to plug into rather than an N-way
//! `if`/`else if` chain repeated at every gate.
//!
//! Escape-handling stays distributed rather than centralized into a
//! separate dispatcher system: each overlay's own input system is still the
//! sole owner of what Escape does while it's the active variant (close, for
//! everything except character creation, which has no Escape binding in TS
//! at all — only Enter-with-points-spent closes it). A shared dispatcher
//! that also reacted to `just_pressed(Escape)` would risk a close-then-
//! reopen double-fire in the same frame (`just_pressed` stays true for
//! every system that runs before the next frame clears it), and with only
//! one Escape-consuming overlay today there's no actual routing ambiguity
//! to resolve yet — `ActiveOverlay`'s mutual exclusivity already *is* the
//! routing mechanism. Promote to a real dispatcher once a second overlay
//! (dialog, trading, quest log) also wants "Escape closes me."

use crate::transition::Transition;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// Which modal overlay currently owns input, if any. `CharCreation` is
/// folded in alongside the in-dungeon overlays (save/load now; dialog,
/// trading, quest log, inventory, attribute, stats in later phase-4 slices)
/// rather than kept on its own separate gate — one shared "what's blocking
/// gameplay" state for all of them.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveOverlay {
    /// The character-creation screen, shown at launch before the level
    /// loads. Not a `Default` — always set explicitly at startup, never
    /// re-entered afterward.
    CharCreation,
    /// The save/load modal — opened via Escape, or by
    /// `save_load_overlay::check_player_death` on death.
    SaveLoad,
    /// No overlay open; gameplay input reaches the dungeon.
    None,
}

impl ActiveOverlay {
    pub fn is_open(self) -> bool {
        self != ActiveOverlay::None
    }
}

/// Gameplay systems check `blocked()` the same way they already check
/// `Transition::is_active` — an open overlay is just another reason input
/// shouldn't reach the dungeon yet, matching TS's `anyOverlayOpen`. Systems
/// that need `ResMut` access to `ActiveOverlay`/`Transition` themselves (an
/// overlay's own input handler, the death check) can't use this — it would
/// conflict with their own `ResMut` borrow — and read `Res<ActiveOverlay>`/
/// compare directly instead.
#[derive(SystemParam)]
pub struct InputGate<'w> {
    transition: Res<'w, Transition>,
    overlay: Res<'w, ActiveOverlay>,
}

impl InputGate<'_> {
    pub fn blocked(&self) -> bool {
        self.transition.is_active() || self.overlay.is_open()
    }

    /// Overlay-only pause for per-frame tick systems: TS gates those on
    /// `anyOverlayOpen` alone and keeps them running through transition
    /// fades — only the enemy AI adds the transition condition (`blocked`).
    pub fn paused(&self) -> bool {
        self.overlay.is_open()
    }
}

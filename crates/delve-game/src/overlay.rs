//! Central "which modal is open" state, replacing a boolean flag on each
//! overlay's own resource. Mirrors TS's `anyOverlayOpen` (a derived OR
//! across every overlay's own `visible` flag) as a single enum instead — at
//! most one overlay is ever open, so the enum is the more precise model —
//! every overlay (dialog, trading, quest log, inventory, attribute, stats)
//! has one shared place to plug into rather than an N-way `if`/`else if`
//! chain repeated at every gate.
//!
//! Escape-handling stays distributed rather than centralized into a
//! separate dispatcher system: each overlay's own input system is still the
//! sole owner of what Escape does while it's the active variant (close, for
//! everything except character creation, which has no Escape binding in TS
//! at all — only Enter-with-points-spent closes it).
//!
//! Seven overlays now handle their own Escape (save/load, dialog, inventory,
//! attribute panel, stats panel, trading, quest log) — past the "once a
//! second overlay also wants it" point `PHASE4-PLAN.md` set as the revisit
//! trigger. A real
//! dispatcher was considered and rejected here: `ActiveOverlay`'s mutual
//! exclusivity already gives every overlay's own `if *overlay != Self {
//! return }` guard at Escape-check time, so no two systems can ever race to
//! consume the same keypress — a shared dispatcher wouldn't remove that
//! guard, it would just relocate it. And unlike the other four, closing the
//! attribute panel isn't unconditional (`try_close` blocks it while
//! levelup-mode points are unspent) and closing dialog clears session
//! state — a dispatcher would need `ResMut` access to every overlay's own
//! resource to replicate that, which is the same coupling this design
//! avoided by keeping each overlay's data in its own resource in the first
//! place. Consolidating N systems' worth of close logic into one function
//! with N resources in its `SystemParam` trades five small, independently
//! reviewable diffs for one large one, without eliminating any actual
//! risk — the double-fire hazard a dispatcher exists to avoid was never
//! about needing more overlays, it was about two systems reacting to the
//! same edge in the same frame, which distributed ownership already rules
//! out by construction.

use crate::transition::Transition;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// Which modal overlay currently owns input, if any. `CharCreation` is
/// folded in alongside the in-dungeon overlays (save/load, dialog now;
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
    /// The NPC dialog panel — opened by interacting with an NPC, closed by
    /// Escape, reaching a dialog-ending node, or `DialogEvent::OpenShop`
    /// handing off to the trading overlay.
    Dialog,
    /// The full-screen interactive inventory — opened by `KeyI`, closed by
    /// `KeyI` or Escape.
    Inventory,
    /// The attribute-allocation panel — opened by `KeyL` (auto-selecting
    /// levelup or read-only stats mode based on unspent points), closed by
    /// `KeyL` or Escape, though close is blocked while levelup-mode points
    /// remain unspent.
    AttributePanel,
    /// The read-only character stats panel — opened and closed by `KeyT`
    /// (an unconditional toggle) or closed by Escape.
    StatsPanel,
    /// The trading panel — opened only via `DialogEvent::OpenShop`, never
    /// directly by a key (TS has no keyboard binding that opens it either).
    /// Closed by Escape, its only keyboard binding.
    Trading,
    /// The quest log — opened by `KeyJ`, closed by `KeyJ` or Escape.
    QuestLog,
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

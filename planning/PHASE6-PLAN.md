# PHASE6-PLAN.md

Phase 6 work queue, derived from the full COMPLETED.md audit (every item below verified against both codebases, not inferred). Branch: `port/phase-6-parity-polish`.

## Landed

- Camera view-offset crop (`zones.rs`, SubCameraView, pinned pixel-math tests).
- Debug tooling (`debug.rs`: KeyM fullbright + coupled noclip, KeyY/KeyH layer fly, attack-key auto-kill; the projectile player-hit fullbright skip that was missing outright).
- Boulder push primitives in delve-core (`push_boulder`, `can_boulder_roll_to`, per-condition tests).
- PARITY-GAPS.md fully re-verified (Top Findings and ~40 table rows were phase-3-era stale).

## In flight

- Move-blocked handler half in `session.rs` — generalizes the blocked-move hook to all four directions with secret-wall / block-push / direct-boulder-push branches (design approved, core half committed).
- Multi-zone fullbright hold — TS `main.ts:1443` skips per-zone fog/ambient reapply while fullbright is on; ours needs the swap-and-restore equivalent in `zones.rs`.

## Queue (audit findings, in rough priority order)

1. **Sconce embers refresh on torch pickup.** TS re-calls `setSources` right after `extinguishSconce` in the `sconce_taken` handler (`inputSystem.ts:170-173`) — the `particles.rs` module doc's "stale-after-extinguish quirk preserved from TS" framing is wrong and needs rewriting; only non-interactive extinguish paths keep stale sources. Fix: rebuild the ember pool on `SconceTaken` (the `EmbersPending`/`init_embers` machinery already supports deferred rebuild; the old pool root must despawn first).
2. **Ramp geometry fixes cluster** (a COMPLETED.md subsection, absent wholesale): side walls built unconditionally instead of only against solid neighbors, side-fill texture sourced from the base cell instead of the top cell, adjacent ramp-top cells not excluded from wall generation. Needs TS's `RampCellInfo`/`mergeRampCell` reading; touches `ramps.rs` + `dungeon.rs`.
3. **Secret-wall opened state on level revisit** — `wall_entities.rs` always spawns closed; TS applies `opened` state post-build. Persistence-visible.
4. **Noclip doesn't bypass fall/hole detection** — TS's debugMove skips the fall trigger; ours falls while flying. `session.rs`/`player.rs`, after the handler half lands.
5. **Dynamic fog/ambient lerp on zone transitions** (`lerpEnvironment`) — `zones.rs`, after the fullbright hold lands.
6. **Sideways door slide** when the layer above has an open ceiling (`doors.rs` has only the vertical axis).
7. **Open-air layers skip perimeter walls** — `dungeon.rs::is_solid` treats out-of-bounds as solid unconditionally.
8. **Inventory overlay paperdoll fallback icons** for empty equip slots (the HUD mini-panel has them; the full overlay doesn't).
9. **Sign/bookshelf read popup** (`SignRead` only logs today; TS has signOverlay).

## Needs the user's eyes

- Interactive checklist in PROGRESS.md (per-level feature walkthrough).
- Side-by-side visual comparison vs the TS build; light re-tune (`AMBIENT_BRIGHTNESS`, `LUMENS_PER_THREE_UNIT`); evaluate whether the billboard PBR-vs-unlit-shader difference reads as a real visual difference.
- Debug keys (KeyM/KeyY/KeyH) — smoke tests can't press keys.

## Decisions / non-gaps for the record

- Pit-trap `forceRenderable` ceiling toggling: moot — this port never builds separate non-topmost ceilings, so the mechanism has nothing to act on.
- `drop_item` recalculating `max_hp` (TS doesn't): known deviation, decide port-faithful vs keep during the visual pass.
- Dialog hint text omits "or click": dialog mouse support is a recorded phase-6-polish deviation.
- JSON save export/import: browser-DOM-only, deliberately excluded (D8).

## Performance pass results (measured, phase 6)

Frame timing across the five heaviest shipped levels (uncapped, windowed, ~26s samples): every level stays under 2.7ms worst-case frame time — a fraction of the 16.7ms 60fps budget. ruins (largest total grid volume) is the relative low point at 2.65ms worst; tower (most layers, tiny grids) the fastest at 2.34ms. No observed bottleneck; no fixes required for the phase gate.

Two reasoned findings deferred as future follow-ups (real overhead, no observed cost today):

- `tick_enemies` rebuilds per-layer snapshots from scratch every tick — a full grid clone plus fresh door/thin-wall edge sets per layer per frame. Rust-specific borrow-checker overhead, not a TS cost match (TS reads live references). Fix shape: cache live grids and edge sets in resources invalidated by the existing WorldEvent signals (wall destroyed, door state changed) instead of rebuilding unconditionally.
- `draw_hud` allocates a fresh 900KB canvas buffer every frame; the full redraw matches TS but the allocation doesn't (a browser canvas backing store persists). Fix shape: persistent reused buffer in HudState, zeroed in place.

## Remaining phase items

- Final gate, merge, bundle rebuild.

# PROGRESS.md

Session-to-session state. Read this at the start of every session.

---

## Current Phase

**Phase 4: M3 parity, the living world.** Phases 0-3 are merged to main. Work happens on branch `port/phase-4-living-world` following `planning/PHASE4-PLAN.md`'s slice breakdown. See `planning/PORT-PLAN.md` for scope, `planning/DECISIONS.md` for resolved decisions, `planning/PARITY-GAPS.md` for the audited TS-vs-Rust gap inventory, and `planning/PHASE5-PLAN.md` for the next phase's plan.

## Next Steps

Landed on this branch:

- [x] Core: dialog and quest runtimes (`dialog_manager.rs`, `quest_manager.rs`), quest state threaded into dialog `questStage` conditions
- [x] Core: player controller tick — status-effect damage, temp buffs, hunger drain, starvation — and inventory-action dispatch (equip/unequip/use/drop/swap), `player_controller.rs`
- [x] Core: enemy spawner BFS placement (`spawners.rs`), boulder state machine (`boulders.rs`), and the environment zone map builder (`env_zones.rs`) — phase 5 core logic landing ahead of its shell, the same pattern that carried phase 3-5 core logic in ahead of schedule during phase 2
- [x] Game: signal entities render — levers, pressure plates, tripwires (`levers.rs`, `plates.rs`, `tripwires.rs`), wired to the signal manager
- [x] Game: projectile shell — fireballs and darts travel and render (`projectiles.rs`), trap launchers fire them, status effects tick and tint affected enemies (`status_effects.rs`), combat correctly pauses during transitions and character creation
- [x] Game: blocked-door retry cycle — a signal-driven close onto an occupied cell defers and retries every frame until the cell clears (`session.rs`'s `blocked_doors`/`tick_blocked_doors`), ported from the TS `blockedDoors` map
- [x] Game: HUD compass rose, minimap, and torch-fuel indicator; player vitals wired end to end — torch fuel drains outside dungeon/mist environments, hunger drains and shows on its own bar, status-effect and starvation damage flash the screen
- [x] Fix: stair cells no longer double-render flat floor/ceiling/wall geometry under the stepped stair mesh; corner-anchored box UVs so stair and door texture coursing matches the surrounding walls
- [x] Docs: `PORT-PLAN.md` has a home for every feature the parity audit found orphaned; `PHASE4-PLAN.md` and `PHASE5-PLAN.md` lay out the next two phases in slice-level detail

- [x] Game: environment objects — chests with animated lids and signal wiring, breakable and secret walls (wall-entity cells own their floor/ceiling; destruction persists across transitions via `destroyed_walls` replay), pushable blocks, signs
- [x] Game: save/load — file-backed slots under `saves/`, slot-picker overlay, autosave on level arrival, death/restart flow (`save_store.rs`, `save_load_overlay.rs`, `Transition::PendingAction`)
- [x] Game: combat feedback — sword swing overlay, enemy health bars (child billboards, logic unit-tested), enemy damage flash composed with the status tint in a single-writer system, hit shake, level-up toast
- [x] Adversarial review round 2 fixes: active-layer launcher fires delivered via `apply_world_events` (they were silently discarded), destroyed-wall grid replay, overlay-pause (`paused()`) vs transition-freeze (`blocked()`) gating per system, move-only pickups
- [x] Phase 3 gate: fmt/clippy/test green plus smoke runs, merged to main

Interactive verification still owed (needs human keys, flagged for the user): save/load round-trip through the overlay, death → restart flow, trap launchers firing after a lever/plate trigger, chest/block/secret-wall interactions.

Key Rust API notes for the shell work: `GameState::new` takes `GameStateDeps` (item DB `Arc`, enemy/npc registrar boxes — `EnemyDatabase`/`NpcDatabase` need registrar impls or wrappers) plus an injected `random` closure; TS callbacks became `WorldEvent`s drained via `gs.take_events()`; signal events are applied by `gs.handle_signal_events`; `interaction::interact` mirrors the TS use-key flow; `enemy_ai::update_enemies` takes a snapshot-based `is_door_open` closure (build it from door states before the call to avoid borrow conflicts). Pure-logic core modules that tick per-frame state (`player_controller`, `spawners`, `boulders`) take an injected read-only context struct for grid/database data GameState doesn't own itself, and return events for the shell to act on instead of driving rendering directly — `enemy_ai::EnemyUpdateContext` is the template every one of them follows.

## State

- Phase 1 merged 2026-07-12: `delve-game` walks `dungeon1.json` in first person. Procedural textures (software pixel canvas → Bevy images, nearest filtering), grid geometry (floors/walls/ceilings, charDef wall overrides, seeThrough support), tweened movement with the TS command queue, fog/ambient presets, flickering torch lights. `resolve_textures` ported into `delve-core` with tests.
- Phase 0 merged 2026-07-12: typed level/dungeon schema, ported level loader and validation (TS error/warning message parity), grid primitives with `PlayerState`, bit-exact `mulberry32`, parsing for items/enemies/npcs/loot/quests/dialogs. Assets gate test covers every file under `assets/levels/` and `assets/data/`.
- Deliberately deferred: the camera view-offset crop from the TS shell, and debug tooling (noclip, fullbright, auto-kill, layer-fly) — the two `PARITY-GAPS.md` findings that still have no phase assigned in `PORT-PLAN.md`. Every other audited gap now has a home: the inventory overlay and attribute/stats panels landed in phase 4's scope, compass/minimap/torch already shipped.
- Light intensity mapping (Three.js units → Bevy lumens/cd-m²) uses approximations: `AMBIENT_BRIGHTNESS` in `environment.rs`, `LUMENS_PER_THREE_UNIT` in `torch.rs`. Re-tune during the phase 6 side-by-side audit.
- Toolchain: Rust 1.95.0 (pinned), Bevy 0.19.0 (locked). Verify Bevy APIs against the local registry source at `~/.cargo/registry/src/*/bevy_*-0.19.0/` — that caught `PointLight.shadow_maps_enabled` and `AmbientLight` becoming a camera component.
- Parity target: TS `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`.
- Wasm check (phase 6 informational item, run 2026-07-12): both crates pass `cargo check --target wasm32-unknown-unknown` with zero warnings — the delve-core platform-clean claim holds, and delve-game type-checks too. Check-only: a real browser build would still need wasm-bindgen tooling and fetch-based asset loading (`assets_dir()` + `std::fs` in `main.rs` is desktop-only).

## Known Issues

- macOS launch path: use `scripts/bundle-macos.sh` + `open target/DelveWard.app`. macOS 26 denies activation to bare terminal binaries, so `cargo run` renders but never gets keyboard focus — fine for launch/init smoke tests only. Resolution details in `planning/ISSUE-macos-focus.md`. User confirmed movement works in the bundled build.
- Smoke test verified launch/init only (window + Metal renderer up, no panics, dungeon loads clean). Screen capture is blocked by macOS permissions in agent sessions, so brightness/fog tuning has not been visually compared against the TS build — user eyes welcome on `cargo run`.
- `npcs.json` defines `nameless_girl` with dialog `nameless_girl`, but no such dialog file exists — in the TS repo either (its dialog would 404 at runtime there too). The assets-gate test allowlists it; remove the allowance if a re-sync brings the file.
- `items.json` ships `armor_dragonscale_vest` with `"type": "armor-steel"`, outside the documented type union. Ported faithfully via the `Unknown` variant (see D14); the item matches no type filter, same as in TS.

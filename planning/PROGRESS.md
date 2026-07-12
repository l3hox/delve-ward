# PROGRESS.md

Session-to-session state. Read this at the start of every session.

---

## Current Phase

**Phase 2: M1 parity, the loot game.** Phases 0 and 1 are complete and merged. See `planning/PORT-PLAN.md` for scope and `planning/DECISIONS.md` for resolved decisions.

## Next Steps

Work in progress on branch `port/phase-2-loot-game`. The core logic port is COMPLETE — everything phase 2 (and much of phases 3-5) needs from `delve-core` exists with ported TS test suites, all gates green. What remains is the `delve-game` shell:

- [x] Core: status effects, entity registry, loot rolling (injected RNG), signal manager (event-based), full game state stack, combat, pathfinding, enemy AI + `create_enemy_instance` (`EnemyDatabase` implements `EnemyRegistrar`), player interaction module
- [ ] Game: doors/keys/stairs rendering (`doorRenderer.ts`, `stairRenderer.ts`, `keyRenderer.ts`), cross-level transitions (`game/transitionSystem.ts`), wired through `GameState` + `WorldEvent`s
- [ ] Game: ground item billboards (`groundItemRenderer.ts`, `itemSprites.ts`), enemy billboards (`enemyRenderer.ts`, `billboardMaterial.ts`), enemy AI ticking (`update_enemies` with `EnemyUpdateContext`), melee combat input (Space)
- [ ] Game: HUD — HP/XP bars, mini inventory panel, damage numbers (`src/hud/`)
- [ ] Game: character creation screen
- [ ] Phase 2 gate: fmt/clippy/test plus `cargo run` smoke test, merge to main

Key Rust API notes for the shell work: `GameState::new` takes `GameStateDeps` (item DB `Arc`, enemy/npc registrar boxes — `EnemyDatabase`/`NpcDatabase` need registrar impls or wrappers) plus an injected `random` closure; TS callbacks became `WorldEvent`s drained via `gs.take_events()`; signal events are applied by `gs.handle_signal_events`; `interaction::interact` mirrors the TS use-key flow; `enemy_ai::update_enemies` takes a snapshot-based `is_door_open` closure (build it from door states before the call to avoid borrow conflicts).

## State

- Phase 1 merged 2026-07-12: `delve-game` walks `dungeon1.json` in first person. Procedural textures (software pixel canvas → Bevy images, nearest filtering), grid geometry (floors/walls/ceilings, charDef wall overrides, seeThrough support), tweened movement with the TS command queue, fog/ambient presets, flickering torch lights. `resolve_textures` ported into `delve-core` with tests.
- Phase 0 merged 2026-07-12: typed level/dungeon schema, ported level loader and validation (TS error/warning message parity), grid primitives with `PlayerState`, bit-exact `mulberry32`, parsing for items/enemies/npcs/loot/quests/dialogs. Assets gate test covers every file under `assets/levels/` and `assets/data/`.
- Deliberately deferred: loot rolling (phase 2, with injected RNG), status-effect ticking (phase 3), quest/dialog runtime (phase 4), camera view-offset crop from the TS shell (with the HUD work), stairs camera pitch/y-offset (phase 2 stairs).
- Light intensity mapping (Three.js units → Bevy lumens/cd-m²) uses approximations: `AMBIENT_BRIGHTNESS` in `environment.rs`, `LUMENS_PER_THREE_UNIT` in `torch.rs`. Re-tune during the phase 6 side-by-side audit.
- Toolchain: Rust 1.95.0 (pinned), Bevy 0.19.0 (locked). Verify Bevy APIs against the local registry source at `~/.cargo/registry/src/*/bevy_*-0.19.0/` — that caught `PointLight.shadow_maps_enabled` and `AmbientLight` becoming a camera component.
- Parity target: TS `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`.

## Known Issues

- macOS keyboard focus: launched from a terminal on macOS 26, the window can come up without keyboard focus — keys then fall through and the system beeps. `claim_initial_focus` in `main.rs` re-asserts activation during a 2s launch grace period (winit `focus_window` → `activateIgnoringOtherApps` + `makeKeyAndOrderFront`). Verified to fire correctly; whether Tahoe grants activation needs an interactive run. `input_diagnostics` logs OS focus at info and keys at `RUST_LOG=delve_game=debug` for follow-up if it persists.
- Smoke test verified launch/init only (window + Metal renderer up, no panics, dungeon loads clean). Screen capture is blocked by macOS permissions in agent sessions, so brightness/fog tuning has not been visually compared against the TS build — user eyes welcome on `cargo run`.
- `npcs.json` defines `nameless_girl` with dialog `nameless_girl`, but no such dialog file exists — in the TS repo either (its dialog would 404 at runtime there too). The assets-gate test allowlists it; remove the allowance if a re-sync brings the file.
- `items.json` ships `armor_dragonscale_vest` with `"type": "armor-steel"`, outside the documented type union. Ported faithfully via the `Unknown` variant (see D14); the item matches no type filter, same as in TS.

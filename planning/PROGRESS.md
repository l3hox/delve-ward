# PROGRESS.md

Session-to-session state. Read this at the start of every session.

---

## Current Phase

**Phase 2: M1 parity, the loot game.** Phases 0 and 1 are complete and merged. See `planning/PORT-PLAN.md` for scope and `planning/DECISIONS.md` for resolved decisions.

## Next Steps

- [ ] Phase 2: doors, keys, locked doors, stairs with entity pairing and cross-level transitions
- [ ] Phase 2: items, inventory, equipment, character stats, loot tables (roll logic with injected RNG + deferred lootTable tests), ground item billboards
- [ ] Phase 2: enemies — billboard sprites, AI movement, melee combat, XP, death, loot drops (`src/enemies/`)
- [ ] Phase 2: HUD — HP/XP bars, mini inventory panel, damage numbers (`src/hud/`)
- [ ] Phase 2: character creation screen
- [ ] Phase 2 gate: fmt/clippy/test plus `cargo run` smoke test

## State

- Phase 1 merged 2026-07-12: `delve-game` walks `dungeon1.json` in first person. Procedural textures (software pixel canvas → Bevy images, nearest filtering), grid geometry (floors/walls/ceilings, charDef wall overrides, seeThrough support), tweened movement with the TS command queue, fog/ambient presets, flickering torch lights. `resolve_textures` ported into `delve-core` with tests.
- Phase 0 merged 2026-07-12: typed level/dungeon schema, ported level loader and validation (TS error/warning message parity), grid primitives with `PlayerState`, bit-exact `mulberry32`, parsing for items/enemies/npcs/loot/quests/dialogs. Assets gate test covers every file under `assets/levels/` and `assets/data/`.
- Deliberately deferred: loot rolling (phase 2, with injected RNG), status-effect ticking (phase 3), quest/dialog runtime (phase 4), camera view-offset crop from the TS shell (with the HUD work), stairs camera pitch/y-offset (phase 2 stairs).
- Light intensity mapping (Three.js units → Bevy lumens/cd-m²) uses approximations: `AMBIENT_BRIGHTNESS` in `environment.rs`, `LUMENS_PER_THREE_UNIT` in `torch.rs`. Re-tune during the phase 6 side-by-side audit.
- Toolchain: Rust 1.95.0 (pinned), Bevy 0.19.0 (locked). Verify Bevy APIs against the local registry source at `~/.cargo/registry/src/*/bevy_*-0.19.0/` — that caught `PointLight.shadow_maps_enabled` and `AmbientLight` becoming a camera component.
- Parity target: TS `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`.

## Known Issues

- Smoke test verified launch/init only (window + Metal renderer up, no panics, dungeon loads clean). Screen capture is blocked by macOS permissions in agent sessions, so brightness/fog tuning has not been visually compared against the TS build — user eyes welcome on `cargo run`.
- `npcs.json` defines `nameless_girl` with dialog `nameless_girl`, but no such dialog file exists — in the TS repo either (its dialog would 404 at runtime there too). The assets-gate test allowlists it; remove the allowance if a re-sync brings the file.
- `items.json` ships `armor_dragonscale_vest` with `"type": "armor-steel"`, outside the documented type union. Ported faithfully via the `Unknown` variant (see D14); the item matches no type filter, same as in TS.

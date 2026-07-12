# PROGRESS.md

Session-to-session state. Read this at the start of every session.

---

## Current Phase

**Phase 1: Walkable skeleton.** Phase 0 (data foundation) is complete and merged. See `planning/PORT-PLAN.md` for scope and `planning/DECISIONS.md` for resolved decisions.

## Next Steps

- [ ] Phase 1: Bevy app shell — window, nearest-neighbor texture loading, camera
- [ ] Phase 1: dungeon geometry from the grid — floors, walls, ceilings with per-cell textures (port `src/rendering/` basics and `src/core/textureResolver.ts`)
- [ ] Phase 1: grid movement — step, strafe, 90-degree turns, tween camera animation, movement blocking (port `src/core/player.ts`; core `PlayerState` already ported)
- [ ] Phase 1: atmosphere — ambient light, distance fog, player torch point light
- [ ] Phase 1 gate: walk around `assets/levels/dungeon1.json` in first person; `cargo run` smoke test

## State

- Phase 0 merged 2026-07-12: `delve-core` has the typed level/dungeon schema, the ported level loader and validation (error/warning message parity with TS), grid primitives with `PlayerState`, bit-exact `mulberry32`, and parsing for items/enemies/npcs/loot/quests/dialogs. Gate test validates every shipped file under `assets/levels/` and `assets/data/` — all pass with zero warnings.
- Loot rolling, status-effect ticking, and quest/dialog runtime state are deliberately deferred to their phases (2, 3, 4); only the data layer is ported so far. The `Math.random`-based lootTable tests port in phase 2 with an injected RNG.
- Repo seeded 2026-07-12: Cargo workspace (`delve-core`, `delve-game`), asset snapshot, planning docs, autonomy contract in CLAUDE.md.
- Toolchain: Rust 1.95.0 (pinned), Bevy 0.19 (pinned minor).
- Parity target: TS `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`.

## Known Issues

- `npcs.json` defines `nameless_girl` with dialog `nameless_girl`, but no such dialog file exists — in the TS repo either (its dialog would 404 at runtime there too). The assets-gate test allowlists it; remove the allowance if a re-sync brings the file.
- `items.json` ships `armor_dragonscale_vest` with `"type": "armor-steel"`, outside the documented type union. Ported faithfully via the `Unknown` variant (see D14); the item matches no type filter, same as in TS.

# PROGRESS.md

Session-to-session state. Read this at the start of every session.

---

## Current Phase

**Phase 0: Data foundation.** See `planning/PORT-PLAN.md` for scope and `planning/DECISIONS.md` for resolved decisions.

## Next Steps

- [ ] Phase 0: serde model of the dungeon/level schema (read `../DelveWard/DUNGEON-DESIGNER.md` first)
- [ ] Phase 0: level loader and validation ported from `src/level/levelLoader.ts` with its tests
- [ ] Phase 0: core primitives (grid, facing, layer coords, mulberry32) with tests
- [ ] Phase 0: item/enemy/npc/loot/quest/dialog data parsing with tests
- [ ] Phase 0 gate: every file in `assets/levels/` and `assets/data/` parses and validates

## State

- Repo seeded 2026-07-12: Cargo workspace (`delve-core`, `delve-game`), asset snapshot, planning docs, autonomy contract in CLAUDE.md.
- Toolchain: Rust 1.95.0 (pinned), Bevy 0.19 (pinned minor).
- Parity target: TS `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`.

## Known Issues

(none)

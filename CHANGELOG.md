# Changelog

All notable changes to this project are documented here, following [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added

- Phase 0 data foundation in `delve-core`: typed serde model of the dungeon/level schema (levels, layers, grids, charDefs, texture areas, entities, player starts), level and dungeon validation with entity migration ported from the TS `levelLoader` (matching error and warning messages), core primitives (facing, grid walkability, player grid movement, bit-exact `mulberry32`), and data parsing for items, enemies, NPCs, loot tables, quests, and dialogs. The vitest suites for grid and levelLoader are ported as Rust tests, and a gate test validates every shipped file in `assets/levels/` and `assets/data/`.
- Repository seed: Cargo workspace (`delve-core`, `delve-game`), Bevy 0.19 shell opening a native window, asset snapshot from the TypeScript original, planning documents (port plan, decisions, progress), and the autonomous session contract in CLAUDE.md.

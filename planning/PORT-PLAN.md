# PORT-PLAN.md

Phased port plan. Phases mirror the TS milestones because each was a validated, playable increment there. Every phase ends with the gates from CLAUDE.md passing on main.

TS source references are relative to `../DelveWard/`.

---

## Phase 0: Data foundation (delve-core only)

Goal: every shipped JSON file in `assets/` parses into typed Rust structs and validates.

- Serde model of the dungeon/level schema per `DUNGEON-DESIGNER.md`: dungeon file, levels, layers, grids, charDefs, texture areas, playerStart, and every entity type. Typed structs, no untyped `Value` traversal.
- Level loader and validation: port `src/level/levelLoader.ts` including entity migration (`migrateEntities`) and validation rules.
- Core primitives: grid and coordinates, `Facing`, `layerKey`, `resolveLayerCoord`, seeded `mulberry32` PRNG (port `src/core/random.ts` bit-exact).
- Data databases: port parsing for `assets/data/items.json`, `enemies.json`, `npcs.json`, `loot-tables.json`, quests, dialogs (`src/core/itemDatabase.ts`, `enemyDatabase.ts`, npc/dialog/quest managers' data layer).
- Gate addition: a test that loads and validates every file in `assets/levels/` and `assets/data/`.

## Phase 1: Walkable skeleton (first delve-game work)

Goal: walk around a loaded dungeon in first person.

- Bevy app: window, nearest-neighbor texture loading, camera.
- Dungeon geometry from the grid: floors, walls, ceilings with per-cell textures (port the basics of `src/rendering/` dungeon building and `src/core/textureResolver.ts`).
- Grid movement: step, strafe, 90-degree turns, tween camera animation, movement blocking (port `src/core/player.ts`).
- Basic atmosphere: ambient light, distance fog, player torch point light.
- Smoke content: `assets/levels/dungeon1.json`.

## Phase 2: M1 parity, the loot game

- Doors, keys, locked doors, stairs with entity pairing and cross-level transitions.
- Items, inventory, equipment, character stats, loot tables, ground item billboards.
- Enemies: billboard sprites, AI movement, melee combat, XP, death, loot drops (`src/enemies/`).
- HUD: HP/XP bars, mini inventory panel, damage numbers (`src/hud/`).
- Character creation screen.

## Phase 3: M2 parity, the dangerous dungeon

- Signal system: `SignalManager` with propagation, cycle detection, gate modes, absolute-time scheduling (`src/core/signalManager.ts`).
- Signal entities: levers, pressure plates, triggers, tripwires, standalone gates, all timed modes.
- Traps and projectiles: trap launchers, fireballs and darts, projectile layer handling.
- Status effects: poison, slow, burning, on player and enemies, with screen tints and icons.
- Player controller tick (`src/core/playerController.ts`): status-effect DoT, temp buffs, hunger/starvation accumulators, torch fuel drain, inventory action dispatch.
- Combat feedback: sword swing overlay, enemy health bars, enemy damage flash and hit shake, player damage flash, level-up notification.
- Environment entities: breakable walls, secret walls, pushable blocks, chests, signs.
- Save/load: full SaveData model, slots plus autosave, JSON files under `saves/`, save/load overlay. Death and restart flow.

## Phase 4: M3 parity, the living world

- NPCs: billboard rendering, interaction, flags (`src/npcs/`).
- Dialog system: dialog trees, conditions, effects, keyboard navigation.
- Quest system: quest JSON, state machine, rewards, quest log overlay.
- Trading overlay: buy/sell, merchant stock.
- Hunger system: drain, food, starvation, HUD bar.
- Dungeon objects: fountain, bookshelf, altar with timed buffs, barrel.
- Interactive overlays and full input map: mouse input, inventory overlay with drag/drop and quick-slot keys 1-8, attribute panel (L), stats panel, item tooltips, overlay Escape handling (see `planning/PARITY-GAPS.md` for the audited key map).

## Phase 5: M4 parity, the vertical world

- Layer system: multi-layer levels, hollow areas, per-layer entity state, cross-layer signals.
- Environment zones: outdoor preset, per-zone fog and ambient via multi-pass rendering with `RenderLayers`, boundary mesh splitting, skyboxes.
- Thin walls with edge blocking; ramps and stairs geometry with layer transitions; falling.
- Pit traps, decorative props, enemy spawners with BFS spawn placement.
- Forest rendering with instanced billboards; particles (dust, embers, drips, fireflies); light distance culling.

## Phase 6: Parity audit and polish

- Play `architects_tomb.json`, `ruins.json`, `tower.json`, and `stairs.json` end to end; compare against the TS build side by side.
- Audit against `../DelveWard/planning/COMPLETED.md` for anything missed.
- Camera view-offset crop from the TS shell; debug tooling (noclip, fullbright, auto-kill, layer-fly).
- Performance pass; a wasm compile check (non-gating, informational).

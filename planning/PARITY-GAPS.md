# PARITY-GAPS.md

Audit of `../DelveWard` (TS, parity target `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`) against this repo, run at the start of Phase 3. Every claim below was verified by reading both sides, not inferred from filenames. Read this alongside `PORT-PLAN.md` (phase scope) and `PROGRESS.md` (what's merged).

---

## Top findings

**A whole category of Phase-2-adjacent HUD/interaction affordances has no home in any PORT-PLAN.md phase.** Attribute allocation (KeyL), the read-only stats panel (KeyT), the interactive full-screen inventory overlay (equip/unequip/drop via drag-and-drop or click, not just walk-over pickup), compass rose, minimap, torch-fuel indicator, the real level-up toast, item tooltips, the sword-swing visual, enemy hit-shake/move-lerp, and the floating enemy health bar are all absent from TS `main`'s Phase 2 milestone as `PORT-PLAN.md` describes it, yet every one of them shipped in the TS build before or during that milestone. Two independent audits (core/game coverage and the COMPLETED.md cross-reference) converged on the same list from different angles. Concretely, this already produces a dead end in the merged code: `game_state.rs` grants `attribute_points` on level-up and `hud.rs` draws "PRESS 'L' TO LEVEL UP", but no `KeyL` handler exists anywhere — points can be earned but never spent. Equipment can currently only be equipped by walking over a ground item; there is no way to unequip, drop, or rearrange items from the UI.

**Core logic is frequently complete and tested while sitting completely disconnected from the game.** `quest_manager.rs` is a full, unit-tested port of `questManager.ts` with zero production call sites anywhere in the workspace — `game_state.rs` has no `QuestManager` field, so nothing can start or advance a quest. `dialog_manager.rs` is likewise a complete port, but its `questStage` condition is stuck on the TS module's own pre-override placeholder since nothing wires `quest_manager`'s evaluator into it, and nothing in `delve-game` ever opens a dialog session. `projectiles.rs` has a full parity-tested `ProjectileManager` port but is never referenced from `crates/delve-game/src` at all — no projectile ever spawns or renders. `status_effects.rs`/`status_effect_state.rs` tick correctly for enemies (via `enemy_ai.rs`) but are never applied to the player, because `playerController.ts` — the file that would call them — has no Rust port whatsoever, not even the tick orchestration. `save_system.rs`'s data model is complete but `apply_save_data` never calls `QuestManager::restore_state`, and its own doc comment claiming "no runtime quest manager exists" is now stale.

**`playerController.ts` is entirely unported**, not partially. Beyond the status-effect and quest gaps above, this means: hunger never drains, starvation never triggers, torch fuel never depletes despite being fully modeled in `game_state.rs`/`torch.rs`, and there is no dispatcher for HUD-driven inventory actions (equip/unequip/use/drop/swap) even though every one of those `GameState` methods already exists and is unit-tested.

**A rendering correctness bug already shipped in merged Phase 2 code**: `dungeon.rs`'s `spawn_dungeon` has no concept of a stair cell, so it renders a normal flat floor/ceiling/wall tile at every stair position in addition to `stairs.rs`'s stepped geometry drawn on top — TS's `buildDungeon` explicitly excludes stair cells from the flat pass. Separately, every billboard (enemies, items, keys, NPCs, forest) uses a standard PBR `StandardMaterial` in Rust versus TS's custom unlit distance-only shader (`billboardMaterial.ts`) — a systematic, cross-cutting visual difference worth a note for the Phase 6 side-by-side audit rather than a per-entity one.

**Input gaps mirror the HUD gaps exactly**: quick-slot digit keys (1-8) are unbound despite full core support (`use_consumable_from_registry`, `backpack_item_at`); KeyI/KeyT/KeyL/KeyJ and the overlay-context Escape are all unbound because the overlays themselves don't exist yet.

**One faithfulness deviation worth flagging for the Phase 6 audit**: Rust's `drop_item` (`game_state.rs`) recalculates `max_hp` after dropping an item; the TS `dropItem` does not. Low risk (dropping items rarely changes VIT-affecting equipment), but a real behavior difference, not just a missing feature.

---

## `src/core/` coverage

`PROGRESS.md`'s claim that "everything phase 2 (and much of phases 3-5) needs from `delve-core` exists" holds for all but `playerController.ts`, which has no Rust equivalent at all.

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| combat.ts | Ported | `combat.rs` | — | `calculate_damage`, `resolve_weapon_effect`, `player_attack`, `enemy_attack_player` all match; randomness injected via closure. |
| combatState.ts | Ported | `game_state.rs` | — | See deep dive. |
| dialogManager.ts | Ported (logic only) | `dialog_manager.rs` | 4 | Every export ported; not wired into `delve-game` — no NPC interaction opens a session, nothing renders dialog text. `questStage` condition still the hardcoded TS default placeholder. |
| entities.ts | Ported | `entities.rs` | — | `EntityRegistry`, `ItemLocation`, `EquipSlot`, CRUD/snapshot/restore all match. |
| gameState.ts | Ported | `game_state.rs` | — | Near-exhaustive: entity defs, signal wiring (now `WorldEvent`s instead of callbacks), level snapshots, lever/plate/trigger/tripwire/launcher, chest/fountain/altar/sconce interactions. |
| grid.ts | Ported | `grid.rs` | — | `Facing`, `is_walkable`, `PlayerState` (move/strafe/turn via `MoveRules` closures) all match. |
| inventoryState.ts | Ported | `inventory_state.rs` + `game_state.rs` | — | See deep dive. |
| itemDatabase.ts | Ported | `items.rs` | — | Synchronous load instead of `fetch`, correct for a non-browser runtime. |
| lootTable.ts | Ported | `loot.rs` | — | `roll_quality`/`roll_gold`/`roll_loot`/enchanted-modifier table with injected randomness. |
| playerController.ts | **Unported** | none | 2 (inventory dispatch) / 3 (status-effect tick) / 4 (hunger/starvation) | No Rust equivalent anywhere. `tickPlayerController`, `shouldDrainTorch`, `processInventoryAction` all missing; the `GameState` methods they'd call already exist and are unused. |
| projectileManager.ts | Ported | `projectiles.rs` | — | Includes `cellsOnPath` boundary walking and thin-wall/entity collision priority; hits returned as events. |
| questManager.ts | Ported (logic only) | `quest_manager.rs` | 4 | Complete and tested; zero production call sites. See deep dive. |
| random.ts | Ported | `random.rs` | — | Mulberry32 verified bit-exact against captured JS output. |
| saveSystem.ts | Partial | `save_system.rs` | 3 (data model done) / 4 (quest wiring) | `apply_save_data` doesn't call `QuestManager::restore_state`; the "no runtime quest manager exists" doc comment is stale. `export`/`importSaveFile` correctly excluded (browser DOM). |
| signalManager.ts | Ported | `signal_manager.rs` | — | Topological sort, delay/pulse gates, timed-source scheduling, save/load state. |
| statusEffects.ts | Ported | `status_effects.rs` | — | Tick/refresh/expiry semantics match. |
| statusEffectState.ts | Ported | `status_effect_state.rs` | — | Temp-buff replace-not-stack behavior matches. |
| textureNames.ts | Ported | `texture_names.rs` | — | `Set` membership replaced by helper functions. |
| textureResolver.ts | Ported | `texture_resolver.rs` | — | Four-layer resolution order matches, including boundary-inclusive area checks. |
| types.ts | Ported | `types.rs` | — | TS index-signature entity bag becomes a typed `props: Map<String, Value>`. |
| worldEntityState.ts | Ported | `game_state.rs` ("World entity facade") | — | Every accessor/mutator and both `parse*Entity` dispatch heuristics match; several tracked entity types (chests, thin walls, ramps, NPCs, fountains, altars, barrels, boulders) aren't rendered yet, which is the pending Phase 3-5 shell work, not a core gap. |

### Deep dive: `combatState.ts` vs `game_state.rs`

Every member landed as an inherent method on `GameState`, not on `combat.rs` (which corresponds to the separate, fully-ported `combat.ts`): `getEnemy`→`get_enemy`, `isEnemyAt`→`is_enemy_at`, `isBlockedByEnemy`→`is_blocked_by_enemy`, `moveEnemy`→`move_enemy`, `damageEnemy`→`damage_enemy`, `getEffectiveStats`→`get_effective_stats` (numerically identical formulas, including the dodge-chance `floor`/`clamp(0, 25)`), `getEffectiveAtk`/`getEffectiveDef`, `getEquippedWeaponDef`, `canEquipItem` (same per-stat requirement messages), `getTempBuffTotal`. No missing behavior; the only difference is architectural.

### Deep dive: `inventoryState.ts` vs `inventory_state.rs` + `game_state.rs`

`inventory_state.rs` holds the pure character-sheet primitives with no entity-registry/item-database dependency (`InventoryState` struct, `add_key`/`has_key`/`picked_up_keys`/`restore_picked_up_keys`, `xp_for_level`). Everything that needs the entity registry or item database landed on `GameState`: `pickupKeyAt`→`pickup_key_at`, `addXp`→`add_xp` (identical level-cap-15 loop, +3 points/level), `allocatePoint`→`allocate_point` (identical VIT full-HP preservation), `applyCharacterSetup`→`apply_character_setup`, `equipFromBackpack`/`unequipToBackpack`, `dropItem`→`drop_item` (see the max_hp deviation noted above), `pickupEquipmentAt`/`pickupConsumableAt`, `getPlayerState`/`restorePlayerState`. No missing behavior.

### Deep dive: `playerController.ts` vs `grid.rs`'s `PlayerState` / `delve-game/src/player.rs`

`grid.ts`'s `PlayerState` class (movement, `canMoveTo`, `getFacingCell`) is fully ported into `grid.rs`; `delve-game/src/player.rs`'s `Player` component correctly layers camera-tween/animation on top, matching the TS rendering-layer `Player` class. But `playerController.ts` is a *separate* TS file that is entirely unported in either crate: `tickPlayerController` (player status-effect damage-over-time, damage-flash timer, temp-buff tick, hunger drain every 10s, starvation damage every 3s), `shouldDrainTorch`, and `processInventoryAction` (HUD dispatch for equip/unequip/use/drop/swap) have no Rust call sites at all, though every underlying `GameState` method they'd call already exists.

### Deep dive: `questManager.rs` / `dialog_manager.rs` real state

`quest_manager.rs` is not a stub: every `QuestManager` method (`register_quest_def`, `getStatus`, `startQuest`, `advanceQuest` with reward application, `getStageIndex`, `getActiveQuests`/`getCompletedQuests`, serialize/restore, `installConditionEvaluator`) is present, correct, and covered by its own test file — it even improves on TS by returning `Result` instead of an unchecked cast on restore. The gap is integration: grep across `crates/` confirms zero production call sites outside its own tests and `dialog_manager.rs`'s test file. `game_state.rs` has no `QuestManager` field; `dialog_manager.rs`'s `QuestStage` evaluator arm is still the hardcoded placeholder; `save_system.rs` doesn't restore quest state; `delve-game` never constructs a `QuestManager`, loads a dialog tree, or opens a session. Both modules are correctly Phase 4 work — the only needed correction is `PROGRESS.md`'s stale claim that no runtime quest manager exists.

---

## `src/game/` orchestration coverage

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| assetCheck.ts | Unported | none (`assets_gate.rs` validates schema/parsing only, not PNG existence) | Phase 6 (closest fit; not named explicitly) | No proactive sweep over enemy sprite paths / item icon paths; Bevy's `asset_server.load` fails silently at runtime instead. |
| boulderSystem.ts | Unported | `game_state.rs` has `BoulderInstance`/`BoulderSpawnerInstance` data + signal wiring; no movement logic | Phase 3 (closest fit; boulders never named directly) | Roll/fall/idle state machine, hole detection, ramp descent, chest-crash-on-landing, chain-reaction pushing: all missing. |
| gameLoop.ts | Unported | none | Phase 3 (boulders) / Phase 5 (spawners) | `main.rs`'s Update chain already covers the TS per-frame loop's other pieces (`tick_game` ~ `signalManager.tick`, `tick_enemies` ~ `updateEnemies`); the boulder/spawner tick block plus `tickProjectiles`/`tickStatusEffects`/per-layer trap-launcher ticking (called directly by `main.ts`, not through `gameLoop.ts`) are all missing. |
| inputSystem.ts | Partially ported | `session.rs`, `enemies.rs`, `char_creation.rs` | Overlays: Phase 3. NPC dialog/quest log: Phase 4. Debug layer-fly: Phase 5. | See the keyboard map section below for the full breakdown. |
| levelSceneBuilder.ts | Partially ported | `level_scene.rs` + `dungeon.rs`/`doors.rs`/`stairs.rs`/`enemies.rs`/`ground_items.rs`/`keys.rs`/`sconces.rs` | Signal/environment entities: Phase 3. NPCs/dungeon objects: Phase 4. Layers/thin walls/ramps/props/spawners/boulders/forest/skybox/zones: Phase 5. | `dungeon.rs` never references `layers[...]` — multi-layer scene building hasn't started. Every other builder TS's `sceneUtils.ts`/`levelSceneBuilder.ts` wires together (plates, tripwires, levers, breakable/secret walls, blocks, chests, signs, fountains, bookshelves, altars, barrels, thin walls, ramps, props, spawners, boulders, NPCs, forest, trap launchers, skybox) is unwired. |
| lootSpawner.ts | Ported, for its one reachable call site | `ground_items.rs::spawn_loot` ← `loot.rs::roll_loot`, called from `enemies.rs::handle_kill` | New call sites land with Phase 3 | The function itself is a complete, faithful port. TS calls it from four sites (kill, chest, breakable-wall destroy, barrel destroy); Rust only has the kill site since the other three entity types don't exist yet. |
| projectileSystem.ts | Core logic ported, zero Bevy wiring | `projectiles.rs` (`ProjectileManager::update`, 23 parity tests) | Phase 3 | No file under `delve-game/src` mentions "projectile." Trap launchers (`tickTrapLaunchers`) are also unwired. |
| spawnerSystem.ts | Unported | `game_state.rs` has `SpawnerInstance` data + signal wiring only | Phase 5 (exact match: "enemy spawners with BFS spawn placement") | BFS candidate search, interval/max-active gating, spawn-then-register-mesh flow all missing. `create_enemy_instance`/`EnemyRegistrar` already exist, so the gap is specifically placement + timer. |
| statusEffectSystem.ts | Partial (enemy-side only) | `status_effects.rs`/`status_effect_state.rs`, consumed by `enemy_ai.rs` for enemies | Player ticking + tints/icons: Phase 3. Hunger/starvation: Phase 4. | Wraps `core/playerController.ts::tickPlayerController` — see the core deep dive; player can never actually become poisoned/slowed/burning since nothing calls `apply_effect` on the player. |
| transitionSystem.ts | Partially ported | `transition.rs` (`Transition`, `tick_transition`, `perform_level_swap`) | `restartLevel`/`loadGame`: Phase 3 (save/load overlay). Particle/ember rewiring on swap: Phase 5. Autosave-on-arrival: Phase 3. | Stair transitions are faithfully covered (fade, mid-fade swap, snapshot save/restore, stair-facing spawn offset, `reveal_around`). `restartLevel` (death fallback) and `loadGame` are entirely missing; `enemies.rs::tick_enemies` just logs "You died." with no restart trigger. |

---

## `src/rendering/`, `src/hud/`, `src/npcs/`, `src/enemies/`, `src/level/` coverage

Cross-cutting notes (apply across many rows below, not repeated per-row): every TS billboard uses a custom unlit distance-only shader (`billboardMaterial.ts`); Rust substitutes standard PBR `StandardMaterial` everywhere. `dungeon.rs` has no stair-cell awareness (see Top Findings). Attribute points are earned but unspendable (no `attributePanel.ts` port, no `KeyL` handler). Torch fuel is fully modeled in `game_state.rs` but never drained or displayed.

### `src/rendering/`

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| altarRenderer.ts | Unported | — | Phase 4 | `use_altar`/`get_altar` core logic exists, no mesh. |
| barrelRenderer.ts | Unported | — | Phase 4 | |
| billboardMaterial.ts | Partial | ad-hoc `StandardMaterial` per spawn site | Phase 6 | See cross-cutting note. |
| blockRenderer.ts | Unported | — | Phase 3 | `push_block`/`get_block` core logic exists, no mesh/tween. |
| bookshelfRenderer.ts | Unported | — | Phase 4 | |
| boulderAnimator.ts | Unported | — | Phase 3/5 | Roll/descend/fall tween. |
| boulderRenderer.ts | Unported | — | Phase 3/5 | `is_boulder_at` already blocks movement in `session.rs` — boulders are invisible obstacles today. |
| damageNumbers.ts | Ported | `damage_numbers.rs` | — | Float speed, lifetime, fade, outline all match. |
| doorAnimator.ts | Partial | `doors.rs` (`animate_door_panels`) | Phase 5 | Y-slide+bounce matches; x/z slide axes and `getOpenFraction()` (zone-boundary lights) unported. |
| doorRenderer.ts | Partial | `doors.rs` (`spawn_doors`) | Phase 5 | Frame/panel match; multi-pass zone splitting and boundary entrance lights unported. |
| dungeon.ts | Partial | `dungeon.rs` | Phase 3/5 | See Top Findings stair bug. Zone splitting, ramp wall/floor suppression, pit-trap toggling, hollow-layer detection unported. |
| enemyAnimator.ts | Unported | — | Unscoped — see Top Findings | Move lerp, hit-shake, lunge attack; `enemies.rs` snaps `Transform` with no interpolation. |
| enemyHealthBar.ts | Unported | — | Unscoped — see Top Findings | Floating HP bar above damaged enemies. |
| enemyRenderer.ts | Partial | `enemies.rs` (`spawn_enemy_billboards`) | — | Spawn/position/size match; see billboard-material note. |
| environment.ts | Partial | `environment.rs` | Phase 5 | Static fog/ambient presets match; per-area override, smooth lerp, multi-pass zone maps unported. |
| forestRenderer.ts | Unported | — | Phase 5 | |
| fountainRenderer.ts | Unported | — | Phase 4 | `use_fountain` core logic exists, no mesh. |
| groundItemRenderer.ts | Ported | `ground_items.rs` | — | Spread offsets, spawn/hide/reshow-remaining all match. |
| chestRenderer.ts | Unported | — | Phase 3 | `open_chest`/`get_chest` core logic exists, no mesh/tween. |
| itemSprites.ts | Ported (via Bevy AssetServer) | `ground_items.rs`, `hud.rs` (`IconCache`) | — | Manual TS caching replaced by Bevy asset caching. |
| keyRenderer.ts | Ported | `keys.rs` | — | Procedural gold-key drawing reproduced pixel-for-pixel. |
| leverAnimator.ts | Unported | — | Phase 3 | Pivot rotation tween. |
| leverRenderer.ts | Unported | — | Phase 3 | `activate_lever` already dispatches and `session.rs` reacts for doors, but the lever has no mesh. |
| npcRenderer.ts | Unported | — | Phase 4 | `npcs.rs` is data-only. |
| particles.ts | Unported | — | Phase 5 | Dust motes + sconce embers. |
| plateRenderer.ts | Unported | — | Phase 3 | Pressed/released texture swap. |
| player.ts | Partial | `player.rs` | Phase 5 | Movement/tween/queue/stair-pitch match. Falling physics, `debugNoClip`, per-layer `yOffset`, `onMoveBlocked` retry unported. |
| projectileRenderer.ts | Unported | — | Phase 3 | `projectiles.rs` core logic fully implemented and tested, nothing renders. |
| propRenderer.ts | Unported | — | Phase 5 | |
| rampRenderer.ts | Unported | — | Phase 5 | |
| sceneUtils.ts | Partial | `level_scene.rs` | Phase 3-5 | TS's master scene assembler; `spawn_level_scene` wires dungeon/stairs/doors/enemies/ground-items/keys/sconces only. |
| sconceRenderer.ts | Ported | `sconces.rs` | — | Meshes, material, light, extinguish, flicker constant all match. |
| signRenderer.ts | Unported | — | Phase 3 | `get_sign_on_wall`/`SignRead` return text (logged via `info!`), no mesh/popup. |
| skybox.ts | Unported | — | Phase 5 | |
| spawnerRenderer.ts | Unported | — | Phase 5 | |
| stairRenderer.ts | Ported | `stairs.rs` | — | Stepped geometry, depth fade, side walls match; see dungeon.ts stair-cell note. |
| swordSwing.ts | Unported | — | Unscoped — see Top Findings | Combat feedback is log lines + damage numbers only. |
| textures.ts | Partial | `textures.rs` | Phase 5 | Wall/floor/ceiling/door sets match exactly; thin-wall textures unported. |
| thinWallRenderer.ts | Unported | — | Phase 5 | |
| transitionOverlay.ts | Ported | `transition.rs` | — | Fade phase machine, speed, `isActive` gating match. |
| trapLauncherRenderer.ts | Unported | — | Phase 3 | |
| tripwireRenderer.ts | Unported | — | Phase 3 | |
| wallEntityRenderer.ts | Unported | — | Phase 3 | Breakable/secret wall reveal-on-open. |

### `src/hud/`

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| attributePanel.ts | Unported | — | Unscoped — see Top Findings | Points earned but unspendable. |
| compassRose.ts | Ported | `hud.rs` (`draw_compass`) | — | |
| dialogOverlay.ts | Unported | — | Phase 4 | |
| healthBar.ts | Ported | `hud.rs` (`draw_health_bar`) | — | Bar, low-HP pulse, heart icon, HP text match. |
| hudCanvas.ts | Partial | `hud.rs` (`draw_hud`) | — | TS orchestrator wires compass/torch/hunger/status-icons/minimap/stats-panel/inventory-overlay/attribute-panel/level-up-toast/sword-swing plus mouse drag-drop; `draw_hud` calls health/inventory/XP/level-up-hint/message only. |
| hudColors.ts | Partial | `hud.rs` (const palette) | — | Torch/hunger/compass/minimap colors unported. |
| hudFont.ts | Ported (superset) | `hud_font.rs` | — | Identical glyphs; Rust adds `-`/`+`/`:`/`(`/`)`. |
| hudLayout.ts | Partial | `hud.rs` (const layout) | — | COMPASS/MINIMAP/TORCH_BAR/HUNGER_BAR/STATUS_ICONS/INVENTORY_OVERLAY unported. |
| hungerBar.ts | Unported | — | Phase 4 | |
| characterCreation.ts | Ported | `char_creation.rs` | — | Same points budget/min-stat/panel geometry; pixel font substitutes native canvas text. |
| inventoryOverlay.ts | Unported | — | Unscoped — see Top Findings | Full-screen interactive inventory (cursor, drag-drop, tooltips); equip is walk-over-only in Rust. |
| inventoryPanel.ts | Partially ported | `hud.rs` (`draw_inventory_panel`) | Phase 4 | Drawing matches (key count, gold, equipment + paperdoll ghosts, cooldown overlay, backpack grid). The TS panel is additionally mouse-interactive (hover, drag-to-equip, double-click use, right-click drop); those land with the phase 4 overlay/mouse work. |
| itemTooltip.ts | Unported | — | Unscoped — see Top Findings | Depends on inventoryOverlay.ts's cursor state. |
| levelUpNotification.ts | Unported | — | Unscoped — see Top Findings | Not the same thing as `draw_level_up_hint` (a different, persistent prompt). |
| minimapRenderer.ts | Ported | `hud.rs` (`draw_minimap`) | — | |
| paperdollIcons.ts | Ported | `hud.rs` (`paperdoll_path`, `IconCache`) | — | Identical slot→path mapping. |
| questLogOverlay.ts | Unported | — | Phase 4 | |
| saveLoadOverlay.ts | Unported | — | Phase 3 | `save_system.rs` core fully implemented, no UI/keybinding. |
| signOverlay.ts | Unported | — | Phase 3 | `SignRead`/`BookshelfRead` results computed and logged, no popup. |
| statsPanel.ts | Unported | — | Unscoped — see Top Findings | |
| statusEffectIcons.ts | Unported | — | Phase 3 | |
| torchIndicator.ts | Ported | `hud.rs` (`draw_torch_indicator`) | — | Fuel drain itself is phase 3 (player controller wiring). |
| tradingOverlay.ts | Unported | — | Phase 4 | |
| xpBar.ts | Ported | `hud.rs` (`draw_xp_bar`) | — | LV label, MAX-at-cap, fill ratio, progress text match. |

Upstream content bug, inherited faithfully: `questgiver_hilda.json`'s "The spider queen is dead." choice requires the `spider_queen_killed` flag, but nothing in the TS reference sets it either (no kill-time flag mechanism exists in TS src/ or its data) — the bounty quest is uncompletable in both implementations. Report upstream rather than inventing a kill-flag hook here.

Deliberate shell deviations from the phase 4 review, kept: the trading overlay's click region is the whole row (TS binds only the small button; the row-click surfaces the same guard toasts TS's handler carries, so feedback is a superset); the inventory overlay clears drag state on close (TS keeps a stale drag across close/reopen — a quirk, not a behavior worth reproducing). Dialog choices are keyboard-only (TS also supports mouse hover/click) — joins the phase 6 polish list with the overlay PNG icons.

Known shell gap: TS pushes blocks both by face+interact and by walking into them (a `setOnMoveBlocked` movement hook); the Rust shell implements face+interact only — the movement-blocked hook doesn't exist in `player.rs` yet. The same missing hook covers player-initiated boulder pushing (walking into a pushable boulder, and block-push transferring momentum to an adjacent boulder): `BoulderInstance.pushable` exists in core but nothing in the shell reads it. Revisit both together when the move-blocked hook lands.

Latent core gap (phase 6 check): `EntityRegistry::ground_items(level_id, col, row)` ignores `layer_index` while `all_ground_items_for_level` filters by it — the after-pickup remainder query could cross layers at overlapping cells. No shipped level overlaps ground-item cells across layers today.

Benign warning, TS parity: loading a multi-layer level whose layer-0 signal entities target entities on other layers (e.g. test_m4g's levers → boulders on layers 3/4) logs `lever target "..." must reference an existing entity id — entity skipped`. Both loaders validate the top-level layer-0 *mirror* against layer-0-only ids (TS levelLoader.ts:649+684, Rust level_loader.rs:1326) while the per-layer validation uses the global id set. The runtime reads only `layer_def.entities`, so the skipped mirror entries are inert and the levers work.

Phase 5 note: TS has a latent ground-item key-format bug (initial meshes keyed unprefixed, pickup hides with a layer-prefixed key — level-authored items leave a ghost billboard after pickup). `ground_items.rs` keys consistently and does not reproduce it. When layer-aware keying lands in phase 5, decide whether to preserve that incidental correctness or match TS bit-for-bit.

### `src/npcs/`, `src/enemies/`, `src/level/`

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| npcDatabase.ts | Ported | `npcs.rs` | — | Data-loading/query layer matches; dialog linking and runtime interaction land with Phase 4's shell. |
| enemyAI.ts | Ported | `enemy_ai.rs` | — | Regen/status ticking, flee/chase/attack state machine, deaggro buffer, erratic movement match line-for-line. |
| enemyDatabase.ts | Ported | `enemies.rs` | — | Struct model and queries match. |
| enemyTypes.ts | Ported | `enemies.rs` (`create_enemy_instance`) | — | Field-for-field, including conditional regen-timer init. |
| pathfinding.ts | Ported | `pathfinding.rs` | — | `manhattanDistance` and BFS `findPath` match exactly. |
| interaction.ts | Ported | `interaction.rs` | — | Every branch matches including message text; `session.rs` only acts on door/sconce/lever results today (see rendering table for chest/NPC/fountain/altar/sign/bookshelf). |
| levelLoader.ts | Ported | `level_loader.rs` | — | Full parity test suite. |

---

## COMPLETED.md vs PORT-PLAN.md

Cross-reference of shipped TS milestones against this repo's phase prose. Rows are sorted plan-gap-first.

| Feature | Covered by PORT-PLAN.md? | Already in Rust? | Notes |
|---|---|---|---|
| Compass rose + minimap + exploration/fog-of-war reveal | NO — PLAN GAP | Partial (`explored_cells`/`reveal_around` in `game_state.rs`, no rendering) | `hud.rs` states its own scope as health/XP/mini-inventory/messages only. |
| Torch fuel HUD indicator + fuel-scaled light range/flicker + outdoor/mist fuel-skip rule | NO — PLAN GAP | Partial (`torch_fuel`/`drain_torch_fuel` in `game_state.rs`; `torch.rs` never reads it) | Phase 1's "player torch point light" bullet never mentions fuel. |
| Full-screen Inventory overlay (drag-and-drop equip/unequip/rearrange) | NO — PLAN GAP | NO | Phase 2's HUD bullet names only "mini inventory panel." |
| Debug commands: noclip, fullbright, auto-kill, layer-fly | NO — PLAN GAP | NO | No phase bullet anywhere mentions debug/QA tooling. |
| Camera asymmetric frustum crop / telephoto back-offset | NO — PLAN GAP | Partial (stair pitch-tilt done; view-offset crop absent) | Self-tracked in `PROGRESS.md`'s "Deliberately deferred" note, never promoted into a phase bullet. |
| Multi-layer dungeons, hollow areas, cross-layer signals | Phase 5 | Schema only (`LayerDef` in `types.rs`) | Runtime cross-layer logic not built. |
| Thin walls with edge blocking | Phase 5 | Partial (validated, `thin_wall_key` lookup exists) | No renderer. |
| Ramps/stairs geometry, layer transitions, falling | Phase 5 | Partial (ramp entity validated; `stairs.rs` only builds flat entity-paired stairs) | No ramp geometry or falling physics. |
| Pit traps (signal-driven) | Phase 5 | Validated in `level_loader.rs` only | No runtime state in `game_state.rs`. |
| Enemy spawners (BFS placement) | Phase 5 | Validated in `level_loader.rs` only | A `boulder_spawner` entity type is already validated too, absent from COMPLETED.md — COMPLETED.md itself lags current TS `main`. |
| Decorative props | Phase 5 | Validated in `level_loader.rs` only | No rendering. |
| Outdoor environment, skybox variants, multi-pass RenderLayers | Phase 5 | Schema only (`Environment`/`Skybox` enums) | No RenderLayers/skybox code yet. |
| Forest billboards; particles | Phase 5 | NO | Not started. |
| Signal system + lever/plate/trigger/tripwire/gate entities | Phase 3 | Core done (`signal_manager.rs`) | `delve-game` only renders doors so far. |
| Projectiles + trap launchers | Phase 3 | Core done (`projectiles.rs`) | No rendering. |
| Status effects (poison/slow/burning, tints, icons) | Phase 3 | Core done | No tint/icon rendering. |
| Environment entities (breakable/secret walls, blocks, chests, signs) | Phase 3 | Core done | No rendering. |
| Save/load (slots, autosave, export/import, overlay) | Phase 3 | Core done | File-backed store + overlay UI deferred to the Phase 3 shell. |
| NPCs (billboard, interaction, flags) | Phase 4 | Core done (`npcs.rs`) | No shell rendering. |
| Dialog system | Phase 4 | Core done | No overlay UI. |
| Quest system | Phase 4 | Core done | No overlay UI. |
| Doors/keys/locked doors/stairs, cross-level transitions | Phase 2 | Implemented | Matches `PROGRESS.md`'s claims. |
| Enemies: AI, special behaviors, melee, XP, death, loot | Phase 2 | Implemented | Regen/flee/erratic behaviors fully ported despite the plan bullet only naming "AI movement" generically. |

---

## Keyboard input map

TS dispatcher: `src/game/inputSystem.ts`. Nine other TS files register independent context-gated `keydown` listeners (`dialogOverlay.ts`, `saveLoadOverlay.ts`, `signOverlay.ts`, `questLogOverlay.ts`, `tradingOverlay.ts`, `characterCreation.ts`, plus editor-only files — out of scope per `DECISIONS.md` D2). On the Rust side every `KeyCode::` reference lives in `session.rs`, `enemies.rs`, and `char_creation.rs`; `hud.rs`/`main.rs` handle none.

| Key | TS action (context) | Rust status | Phase | Notes |
|---|---|---|---|---|
| W/↑, S/↓, A, D, Q/←, E/→ | Move/strafe/turn (dungeon) | Bound — same action | Phase 1 | `session.rs::player_input`, identical key set. |
| Space | Interact (doors, levers, sconces, chests, signs, bookshelves, fountains, altars, blocks, NPCs) | Bound to the same core call, incomplete response handling | Core done / Phase 3-4 rendering | `interaction::interact` covers all 14 result types; `interact_input` only reacts to door/sconce/lever results — the rest fall into the catch-all `_ => {}` since those entities aren't rendered yet. Messages go to `info!()`, not `HudState::show_message`. |
| F | Attack | Bound — same action | Phase 2 | |
| Digit1-Digit8 | Use backpack consumable at quick-slot N | **Unbound** | Phase 2 | Pure wiring gap — `use_consumable_from_registry`/`backpack_item_at` already exist. |
| KeyI | Toggle inventory overlay | **Unbound** | Unscoped — see Top Findings | No overlay state machine exists at all. |
| KeyL | Open attribute panel | **Unbound** | Unscoped — see Top Findings | The HUD hint is currently dead. |
| KeyT | Toggle stats panel | **Unbound** | Unscoped — see Top Findings | |
| Arrows/Enter/KeyD (inventory-overlay context) | Cursor, equip/unequip/use, drop | **Unbound** | Unscoped | Dead — overlay doesn't exist. |
| Arrows/Enter/KeyL/Escape (attribute-panel context) | Select stat, allocate, confirm/close | **Unbound** | Unscoped | Dead — panel doesn't exist. |
| KeyJ | Toggle quest log | **Unbound** | Phase 4 | |
| Escape (no overlay) | Open save/load overlay | **Unbound** | Phase 3 | Core ported, no UI/binding. |
| Escape (any panel context) | Close that panel | **Unbound** | Matches the panel's own phase | |
| Any key (sign overlay) | Dismiss sign text | **Unbound** | Phase 3 | |
| Escape (dialog overlay) | Dismiss dialog | **Unbound** | Phase 4 | |
| Arrows/Enter/Digit1-9 (dialog, has choices) | Highlight/select/confirm choice | **Unbound** | Phase 4 | |
| Any key (dialog, no choices) | Advance dialog | **Unbound** | Phase 4 | |
| KeyJ/Escape (quest log open) | Hide quest log | **Unbound** | Phase 4 | |
| Escape (trading overlay) | Hide trading overlay | **Unbound** | Phase 4 | Buy/sell itself is mouse-only in TS. |
| Escape (save/load overlay) | Hide overlay | **Unbound** | Phase 3 | Rest of the UI is mouse-only in TS. |
| Arrows/Enter (character creation) | Select/adjust/confirm | Bound — same action | Phase 2 | `char_creation.rs` matches the TS `handleKey` 1:1, including the points-spent gate on Enter. |
| KeyM | Debug: fullbright + noclip | **Unbound** | Unscoped — see Top Findings | Dev tooling, not in any phase. |
| KeyY / KeyH | Debug: fly to next/prev layer | **Unbound** | Phase 5 (layers) / dev tooling | |

### Mouse-driven TS input (tracked separately)

- **Inventory overlay**: hover-cursor, left-click equip/use, right-click drop, full drag-and-drop.
- **Trading overlay**: buy/sell and close are click-only; Escape only closes.
- **Save/load overlay**: Save/Load/Delete/Export/Import/Restart are click-only buttons; Escape only closes.
- **Dialog overlay**: choice buttons also support hover-highlight and click-to-select, redundant with the keyboard paths above.
- **Sign overlay**: clicking the backdrop also dismisses it, redundant with "any key" dismiss.

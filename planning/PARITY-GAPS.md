# PARITY-GAPS.md

Audit of `../DelveWard` (TS, parity target `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`) against this repo, run at the start of Phase 3. Every claim below was verified by reading both sides, not inferred from filenames. Read this alongside `PORT-PLAN.md` (phase scope) and `PROGRESS.md` (what's merged).

---

## Top findings

**Every HUD overlay, the full player-controller tick, and the quest/dialog runtime wiring landed in Phases 3-5.** Attribute allocation (`attribute_panel.rs`, `KeyL`), the read-only stats panel (`stats_panel.rs`, `KeyT`), the full-screen interactive inventory overlay with drag-and-drop equip/unequip/drop (`inventory_overlay.rs`, `KeyI`), item tooltips (`item_tooltip.rs`), the quest log (`quest_log_overlay.rs`, `KeyJ`), trading (`trading_overlay.rs`), dialog (`dialog_overlay.rs`), save/load with a unified death-and-restart flow (`save_load_overlay.rs`, `KeyR` restarts from death mode), and the level-up toast (`draw_level_up_toast` in `hud.rs`) are all built and wired. `QuestManagerRes` and `DialogManager` are real Bevy resources threaded through `dialog_overlay.rs`, `quest_log_overlay.rs`, and `save_load_overlay.rs` (which calls `quests.0.restore_state(...)` on load); `status_effects.rs`'s `tick_player_vitals` (hunger drain, starvation, damage-over-time, damage-flash timer) is registered in `main.rs`'s Update schedule and `apply_effect` is called for `HitType::Player` in `projectiles.rs`, so the player can be poisoned/slowed/burning same as an enemy. Quick-slot digit keys 1-8 and every overlay-toggle key are bound (see the keyboard map below). What used to be the majority of this document's open items is now closed; the pockets that remain are itemized in `src/hud/`/`src/core/`/`src/game/` row notes rather than called out here.

**Genuinely still open, verified against code**: the sign popup (`SignRead`'s text is logged via `info!`, no `SignOverlay`/popup exists — `signs.rs`'s own module doc confirms this); and `drop_item` (`game_state.rs`) recalculating `max_hp` after a drop where TS's `dropItem` does not (low risk, dropping rarely changes VIT-affecting gear, but a real behavior difference). Both remaining PORT-PLAN phase 6 tool items are implemented and wired: the camera's asymmetric frustum crop (`zones.rs`'s `camera_view_crop`/`apply_camera_view_crop`, registered in `main.rs`'s Update schedule, with unit tests pinning the exact TS `setViewOffset` math including its floor-toward-negative-infinity rounding) and debug tooling (`debug.rs` — `KeyM` fullbright with coupled noclip, `KeyY`/`KeyH` layer fly, attack-key auto-kill, registered at `main.rs`'s Update schedule head; `DebugFlags::fullbright` consulted at every `!debugFullbright` site TS has). The multi-zone fullbright hold (TS `main.ts:1443` skipping per-zone fog/ambient reapply while fullbright is on) is the one debug sub-item still in flight.

**One rendering claim from the Phase 2-era audit is also stale**: `dungeon.rs`'s `spawn_dungeon` now skips stair cells (`if stair_cells.contains(&key) || wall_entity_cells.contains(&key) { continue; }`), matching TS's `buildDungeon` exclusion — the flat-floor-under-stepped-stairs bug is fixed. What's still a real, disclosed difference: every billboard (enemies, items, keys, NPCs, forest) uses a standard PBR `StandardMaterial` versus TS's custom unlit distance-only shader (`billboardMaterial.ts`) — the front/back-face sidedness mismatch this caused was audited and fixed in Phase 5 (`fix: forest billboards light back faces correctly`, `fix: billboard material sidedness parity`); the deeper shader-technique difference itself remains open for Phase 6.

---

## `src/core/` coverage

`PROGRESS.md`'s claim that "everything phase 2 (and much of phases 3-5) needs from `delve-core` exists" now holds without exception — `playerController.ts` landed as `player_controller.rs` in Phase 3/4.

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| combat.ts | Ported | `combat.rs` | — | `calculate_damage`, `resolve_weapon_effect`, `player_attack`, `enemy_attack_player` all match; randomness injected via closure. |
| combatState.ts | Ported | `game_state.rs` | — | See deep dive. |
| dialogManager.ts | Ported and wired | `dialog_manager.rs` + `dialog_overlay.rs` | 4 | Every export ported and wired — NPC interaction opens a session, `dialog_overlay.rs` renders text/choices. |
| entities.ts | Ported | `entities.rs` | — | `EntityRegistry`, `ItemLocation`, `EquipSlot`, CRUD/snapshot/restore all match. |
| gameState.ts | Ported | `game_state.rs` | — | Near-exhaustive: entity defs, signal wiring (now `WorldEvent`s instead of callbacks), level snapshots, lever/plate/trigger/tripwire/launcher, chest/fountain/altar/sconce interactions. |
| grid.ts | Ported | `grid.rs` | — | `Facing`, `is_walkable`, `PlayerState` (move/strafe/turn via `MoveRules` closures) all match. |
| inventoryState.ts | Ported | `inventory_state.rs` + `game_state.rs` | — | See deep dive. |
| itemDatabase.ts | Ported | `items.rs` | — | Synchronous load instead of `fetch`, correct for a non-browser runtime. |
| lootTable.ts | Ported | `loot.rs` | — | `roll_quality`/`roll_gold`/`roll_loot`/enchanted-modifier table with injected randomness. |
| playerController.ts | Ported | `player_controller.rs` (`tick_player_controller`, `should_drain_torch`, `process_inventory_action` — same three names as TS) + `status_effects.rs`/`session.rs` shell wiring | — | `tick_player_controller` runs status damage-over-time, damage-flash timer, temp-buff tick, `HUNGER_DRAIN_INTERVAL = 10.0`/`STARVATION_INTERVAL = 3.0` (byte-identical to TS's 10s/3s), called from `status_effects.rs` in `main.rs`'s Update schedule. `should_drain_torch`/`process_inventory_action` called from `session.rs`. `apply_effect` is called for `HitType::Player` in `projectiles.rs`, so the player takes poison/slow/burning same as enemies. |
| projectileManager.ts | Ported | `projectiles.rs` | — | Includes `cellsOnPath` boundary walking and thin-wall/entity collision priority; hits returned as events. |
| questManager.ts | Ported and wired | `quest_manager.rs` | 4 | Complete and tested; `dialog_overlay::QuestManagerRes` is a live Bevy resource threaded through `dialog_overlay.rs`, `quest_log_overlay.rs`, and `save_load_overlay.rs`/`transition.rs` (`restore_state` on load). See deep dive. |
| random.ts | Ported | `random.rs` | — | Mulberry32 verified bit-exact against captured JS output. |
| saveSystem.ts | Ported and wired | `save_system.rs` (data model) + `transition.rs` (quest restore) | — | `apply_save_data` itself still doesn't touch quest state — by design, `QuestManager` is a delve-game-side resource, not owned by delve-core's pure save/load functions — but `transition.rs`'s `perform_load` calls `world.quests.0.restore_state(quest_data)` immediately after, so a loaded save's quest progress is restored end to end. `save_system.rs:384`'s doc comment ("no runtime quest manager exists") is stale and should be corrected in delve-core to describe the current split, not deleted outright. `export`/`importSaveFile` correctly excluded (browser DOM). |
| signalManager.ts | Ported | `signal_manager.rs` | — | Topological sort, delay/pulse gates, timed-source scheduling, save/load state. |
| statusEffects.ts | Ported | `status_effects.rs` | — | Tick/refresh/expiry semantics match. |
| statusEffectState.ts | Ported | `status_effect_state.rs` | — | Temp-buff replace-not-stack behavior matches. |
| textureNames.ts | Ported | `texture_names.rs` | — | `Set` membership replaced by helper functions. |
| textureResolver.ts | Ported | `texture_resolver.rs` | — | Four-layer resolution order matches, including boundary-inclusive area checks. |
| types.ts | Ported | `types.rs` | — | TS index-signature entity bag becomes a typed `props: Map<String, Value>`. |
| worldEntityState.ts | Ported | `game_state.rs` ("World entity facade") | — | Every accessor/mutator and both `parse*Entity` dispatch heuristics match; every tracked entity type (chests, thin walls, ramps, NPCs, fountains, altars, barrels, boulders) now renders too, as of the Phase 3-5 shell work. |

### Deep dive: `combatState.ts` vs `game_state.rs`

Every member landed as an inherent method on `GameState`, not on `combat.rs` (which corresponds to the separate, fully-ported `combat.ts`): `getEnemy`→`get_enemy`, `isEnemyAt`→`is_enemy_at`, `isBlockedByEnemy`→`is_blocked_by_enemy`, `moveEnemy`→`move_enemy`, `damageEnemy`→`damage_enemy`, `getEffectiveStats`→`get_effective_stats` (numerically identical formulas, including the dodge-chance `floor`/`clamp(0, 25)`), `getEffectiveAtk`/`getEffectiveDef`, `getEquippedWeaponDef`, `canEquipItem` (same per-stat requirement messages), `getTempBuffTotal`. No missing behavior; the only difference is architectural.

### Deep dive: `inventoryState.ts` vs `inventory_state.rs` + `game_state.rs`

`inventory_state.rs` holds the pure character-sheet primitives with no entity-registry/item-database dependency (`InventoryState` struct, `add_key`/`has_key`/`picked_up_keys`/`restore_picked_up_keys`, `xp_for_level`). Everything that needs the entity registry or item database landed on `GameState`: `pickupKeyAt`→`pickup_key_at`, `addXp`→`add_xp` (identical level-cap-15 loop, +3 points/level), `allocatePoint`→`allocate_point` (identical VIT full-HP preservation), `applyCharacterSetup`→`apply_character_setup`, `equipFromBackpack`/`unequipToBackpack`, `dropItem`→`drop_item` (see the max_hp deviation noted above), `pickupEquipmentAt`/`pickupConsumableAt`, `getPlayerState`/`restorePlayerState`. No missing behavior.

### Deep dive: `playerController.ts` vs `grid.rs`'s `PlayerState` / `player_controller.rs` / `delve-game/src/player.rs`

`grid.ts`'s `PlayerState` class (movement, `canMoveTo`, `getFacingCell`) is fully ported into `grid.rs`; `delve-game/src/player.rs`'s `Player` component correctly layers camera-tween/animation on top, matching the TS rendering-layer `Player` class. `playerController.ts` itself is now `player_controller.rs`, using the same three function names as TS: `tick_player_controller` (status damage-over-time, damage-flash timer, temp-buff tick, hunger drain via `HUNGER_DRAIN_INTERVAL = 10.0`, starvation via `STARVATION_INTERVAL = 3.0`, `PlayerTickState`'s accumulators matching TS's — called from `status_effects.rs` in `main.rs`'s Update schedule), `should_drain_torch` (environment-gated, called from `session.rs`), and `process_inventory_action` (equip/unequip/use/drop dispatch for the `InventoryAction` enum, called from `session.rs`, consumed by `inventory_overlay.rs`'s cursor/drag state). `debug_fullbright` suppresses status-effect and starvation damage matching the TS debug flag's intent, though the debug toggle itself (`KeyM`) is still unbound (see Top Findings).

### Deep dive: `questManager.rs` / `dialog_manager.rs` real state

`quest_manager.rs` is not a stub and is now fully wired: every `QuestManager` method (`register_quest_def`, `getStatus`, `startQuest`, `advanceQuest` with reward application, `getStageIndex`, `getActiveQuests`/`getCompletedQuests`, serialize/restore) is present, correct, and covered by its own test file — it even improves on TS by returning `Result` instead of an unchecked cast on restore. `dialog_overlay::QuestManagerRes` wraps it as a Bevy resource; `dialog_overlay.rs` passes a live `Some(quests)` into `get_available_choices` (`delve-core/src/dialog_manager.rs`'s `evaluate_condition`), so `questStage` dialog conditions read real quest progress, not the TS default evaluator's undiscovered-only placeholder; `quest_log_overlay.rs` and `save_load_overlay.rs` read the same resource; `transition.rs`'s `perform_load` calls `quests.0.restore_state(...)` after `apply_save_data` so a loaded save's quest progress survives (see the `saveSystem.ts` row above for why that's a separate call, not inside `apply_save_data` itself). Both modules were Phase 4 work and landed as described.

---

## `src/game/` orchestration coverage

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| assetCheck.ts | Unported | none (`assets_gate.rs` validates schema/parsing only, not PNG existence) | Phase 6 (closest fit; not named explicitly) | No proactive sweep over enemy sprite paths / item icon paths; Bevy's `asset_server.load` fails silently at runtime instead. |
| boulderSystem.ts | Ported | `boulders.rs` (delve-core: `tick_boulders`/`tick_boulder_spawners`) + `boulders.rs` (delve-game: render shell, `BoulderAnimator`/`animate_boulders`) | Phase 5 | Roll/fall/idle state machine, hole detection, ramp descent, chain-reaction pushing landed; wired into `main.rs`'s Update schedule (`tick_boulders_system`, `tick_boulder_spawners_system`). |
| gameLoop.ts | Partially ported | none (dispatch stays inline in `main.rs`) | Phase 3 (remaining pieces) | `main.rs`'s Update chain covers `tick_game` (~`signalManager.tick`), `tick_enemies` (~`updateEnemies`), and now the boulder/spawner tick block (`tick_boulders_system`, `tick_boulder_spawners_system`, `tick_spawners_system`) and `projectiles::tick_projectiles`. Per-layer trap-launcher ticking (`GameState::tick_trap_launchers` exists but has no call site) and player-side `tickStatusEffects` remain unwired. |
| inputSystem.ts | Partially ported | `session.rs`, `enemies.rs`, `char_creation.rs` | Overlays: Phase 3. NPC dialog/quest log: Phase 4. Debug layer-fly: Phase 5. | See the keyboard map section below for the full breakdown. |
| levelSceneBuilder.ts | Mostly ported | `level_scene.rs` + `dungeon.rs`/`doors.rs`/`stairs.rs`/`enemies.rs`/`ground_items.rs`/`keys.rs`/`sconces.rs`/`levers.rs`/`plates.rs`/`tripwires.rs`/`wall_entities.rs`/`chests.rs`/`blocks.rs`/`signs.rs`/`npcs.rs`/`fountains.rs`/`altars.rs`/`barrels.rs`/`bookshelves.rs`/`spawners.rs`/`boulders.rs`/`props.rs`/`thin_walls.rs`/`forest.rs`/`skybox.rs`/`ramps.rs`/`zones.rs` | Phase 5 (layers/zones) landed. | Multi-layer scene building is live — `spawn_level_scene` loops every layer and wires every entity type TS's builder does, each zone-tagged via `zones::tag_cell`/`tag_by_key`/`tag_forest`. Trap launchers still have no mesh/tick call site (see `gameLoop.ts` row above); hollow-layer (`openBottom`/`openTop`) detection lives in `dungeon.rs::VerticalOpenness`, applied to dungeon floors/ceilings, wall-entity reveal groups, and pit floors. |
| lootSpawner.ts | Ported, for its one reachable call site | `ground_items.rs::spawn_loot` ← `loot.rs::roll_loot`, called from `enemies.rs::handle_kill` | New call sites land with Phase 3 | The function itself is a complete, faithful port. TS calls it from four sites (kill, chest, breakable-wall destroy, barrel destroy); Rust only has the kill site since the other three entity types don't exist yet. |
| projectileSystem.ts | Core logic ported, wired | `projectiles.rs` (`ProjectileManager::update`, 23 parity tests) + `delve-game/src/projectiles.rs` (render shell, `tick_projectiles` in `main.rs`'s Update schedule) | — | Trap launchers (`tickTrapLaunchers`) remain unwired (see `gameLoop.ts` row). |
| spawnerSystem.ts | Ported | `spawners.rs` (delve-core: `tick_spawners`) + `spawners.rs` (delve-game: marker render shell) | Phase 5 | BFS candidate search, interval/max-active gating, spawn flow landed; wired via `tick_spawners_system` in `main.rs`'s Update schedule. |
| statusEffectSystem.ts | Ported, both sides | `status_effects.rs`/`status_effect_state.rs`, consumed by `enemy_ai.rs` for enemies and `player_controller.rs`'s `tick_player_controller` for the player | — | Wraps `core/playerController.ts::tickPlayerController` — see the core deep dive. `apply_effect` is called for `HitType::Player` in `projectiles.rs`, so the player is poisoned/slowed/burned same as enemies; screen-tint overlays and status icons render from `status_fx`. |
| transitionSystem.ts | Ported | `transition.rs` (`Transition`, `tick_transition`, `perform_level_swap`, `perform_restart`, `perform_load`) | — | Stair transitions, `restartLevel` (death fallback, `KeyR` in `save_load_overlay.rs`'s death mode), and `loadGame` are all wired. `save_load_overlay::check_player_death` is what now logs "You died." and opens the save/load overlay in death mode — not a dead end in `enemies.rs::tick_enemies` anymore. |

---

## `src/rendering/`, `src/hud/`, `src/npcs/`, `src/enemies/`, `src/level/` coverage

Cross-cutting note (applies across many rows below, not repeated per-row): every TS billboard uses a custom unlit distance-only shader (`billboardMaterial.ts`); Rust substitutes standard PBR `StandardMaterial` everywhere (sidedness parity fixed in Phase 5, the shader technique itself is still a Phase 6 item — see Top Findings). `dungeon.rs`'s stair-cell awareness, attribute-point spending, and torch-fuel drain/display are all implemented now — see Top Findings for what's genuinely still open (torch fuel doesn't scale light intensity, sign popup, debug tooling, camera frustum crop, the `drop_item` max_hp deviation).

### `src/rendering/`

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| altarRenderer.ts | Ported | `altars.rs` (`spawn_altars`, `mark_altar_used`) | — | Zone-tagged (Phase 5). |
| barrelRenderer.ts | Ported | `barrels.rs` (`spawn_barrels`, `despawn_barrel`) | — | Zone-tagged (Phase 5). |
| billboardMaterial.ts | Partial | ad-hoc `StandardMaterial` per spawn site | Phase 6 | See cross-cutting note. |
| blockRenderer.ts | Ported | `blocks.rs` (`spawn_blocks`, `animate_block_push`, `animate_blocks`) | — | Zone-tagged (Phase 5). |
| bookshelfRenderer.ts | Ported | `bookshelves.rs` (`spawn_bookshelves`) | — | Zone-tagged (Phase 5). |
| boulderAnimator.ts | Ported | `boulders.rs` (`BoulderAnimator`, `animate_boulders`) | Phase 5 | Roll/descend/fall tween landed. |
| boulderRenderer.ts | Ported | `boulders.rs` (`spawn_boulders`) | Phase 5 | Boulders render and animate; no longer invisible obstacles. |
| damageNumbers.ts | Ported | `damage_numbers.rs` | — | Float speed, lifetime, fade, outline all match. |
| doorAnimator.ts | Ported | `doors.rs` (`animate_door_panels`, `open_fraction`, `update_door_boundary_lights`) | Phase 5 | Y-slide, x/z slide axes, bounce, and the open fraction driving zone-boundary door lights all match. |
| doorRenderer.ts | Partial | `doors.rs` (`spawn_doors`) | Phase 5 | Frame/panel match; multi-pass zone splitting and boundary entrance lights unported. |
| dungeon.ts | Partial | `dungeon.rs` | Phase 3/5 | See Top Findings stair bug. Per-cell zone tagging, ramp wall/floor suppression, pit-trap floor toggling, hollow-layer (`openBottom`/`openTop`) detection, and the zone-boundary half-tile split (`ZoneSplit`) are all in `dungeon.rs`. |
| enemyAnimator.ts | Partial | `enemy_feedback.rs` (`EnemyHitShake`, `tick_enemy_hit_shake`) | — | Hit-shake and lunge-attack landed (module doc: "ported from `rendering/enemyAnimator.ts`'s hit-shake fields"). Smooth move lerp did not — `enemies.rs` still snaps `Transform` to the new grid position with no interpolation between cells. |
| enemyHealthBar.ts | Ported | `enemy_feedback.rs` (`spawn_health_bars`, `add_single_health_bar`) | — | Floating HP bar above damaged enemies, hidden at full HP. |
| enemyRenderer.ts | Partial | `enemies.rs` (`spawn_enemy_billboards`) | — | Spawn/position/size match; see billboard-material note. |
| environment.ts | Partial | `environment.rs` + `zones.rs` | Phase 5 | Static fog/ambient presets match; multi-pass zone maps (multi-camera architecture, `zones.rs`) landed. Smooth lerp between environments on transition is still unported. |
| forestRenderer.ts | Ported | `forest.rs` (rendering) + `delve-core/src/forest_placement.rs` (seeded placement math) | Phase 5 | Billboard-per-tree instead of TS's one-`InstancedMesh`-per-variant (disclosed mechanism deviation in `forest.rs`'s module doc); variant sizes, zone tagging (`tag_forest`), and camera-yaw facing all match. |
| fountainRenderer.ts | Ported | `fountains.rs` (`spawn_fountains`, `mark_fountain_used`) | — | Water-disc material is single-sided (`cull_mode` default, matching TS `FrontSide`) after the Phase 5 sidedness audit. |
| groundItemRenderer.ts | Ported | `ground_items.rs` | — | Spread offsets, spawn/hide/reshow-remaining all match. |
| chestRenderer.ts | Ported | `chests.rs` (`spawn_chests`, `open_chest_mesh`, `animate_chest_lids`) | — | Zone-tagged (Phase 5). |
| itemSprites.ts | Ported (via Bevy AssetServer) | `ground_items.rs`, `hud.rs` (`IconCache`) | — | Manual TS caching replaced by Bevy asset caching. |
| keyRenderer.ts | Ported | `keys.rs` | — | Procedural gold-key drawing reproduced pixel-for-pixel. |
| leverAnimator.ts | Ported | `levers.rs` (`animate_levers`) | — | Pivot rotation tween. |
| leverRenderer.ts | Ported | `levers.rs` (`spawn_levers`, `set_lever_target`) | — | Zone-tagged (Phase 5). |
| npcRenderer.ts | Unported | — | Phase 4 | `npcs.rs` is data-only. |
| particles.ts | Ported | `particles.rs` | Phase 5 | All four systems (dust motes, sconce embers, water drips, fireflies) plus light-distance culling landed and wired into `main.rs`'s Update schedule. |
| plateRenderer.ts | Ported | `plates.rs` (`spawn_plates`) | — | Zone-tagged (Phase 5). |
| player.ts | Partial | `player.rs` | Phase 5 | Movement/tween/queue/stair-pitch match. Falling physics (kinematic Y-channel, `is_falling`) and per-layer `y_offset` landed in Phase 5. `debugNoClip` and the `onMoveBlocked` retry hook (see the walk-into-block/boulder-push note below the rendering table) remain unported. |
| projectileRenderer.ts | Unported | — | Phase 3 | `projectiles.rs` core logic fully implemented and tested, nothing renders. |
| propRenderer.ts | Ported | `props.rs` | Phase 5 | |
| rampRenderer.ts | Ported | `ramps.rs` | Phase 5 | Geometry, movement rules, same-scene layer crossing landed. |
| sceneUtils.ts | Mostly ported | `level_scene.rs` | Phase 3-5 | TS's master scene assembler; `spawn_level_scene` now wires dungeon/stairs/doors/enemies/ground-items/keys/sconces/levers/plates/tripwires/wall-entities/chests/blocks/signs/npcs/fountains/altars/barrels/bookshelves/spawners/boulders/props/thin-walls/forest/skybox/particles per layer. Trap launchers are the remaining unwired entity type. |
| sconceRenderer.ts | Ported | `sconces.rs` | — | Meshes, material, light, extinguish, flicker constant all match. |
| signRenderer.ts | Partial | `signs.rs` | Phase 3 (popup)/5 (mesh) | Mesh, texture, wall-mounted orientation, and zone tagging landed in Phase 5 (`spawn_signs`, wired into `level_scene.rs`). `get_sign_on_wall`/`SignRead` still just return text (logged via `info!`) — no read popup/overlay. |
| skybox.ts | Ported | `skybox.rs` | Phase 5 | Sphere geometry/texture generation, zone tagging, camera-follow all landed and wired. |
| spawnerRenderer.ts | Ported | `spawners.rs` | Phase 5 | Marker decal render shell landed and wired via `tick_spawners_system`. |
| stairRenderer.ts | Ported | `stairs.rs` | — | Stepped geometry, depth fade, side walls match; see dungeon.ts stair-cell note. |
| swordSwing.ts | Ported | `hud.rs` (`trigger_sword_swing`, `draw_sword_swing`) | — | Triggered on player attack (`enemies.rs::kill_effects.hud.trigger_sword_swing`). |
| textures.ts | Partial | `textures.rs` + `thin_walls.rs` | Phase 5 | Wall/floor/ceiling/door sets match exactly; thin-wall textures (`stone_thin`/`iron_fence`/`wood_fence`/`railing`) landed in `thin_walls.rs` rather than `textures.rs::DungeonMaterials` (disclosed ownership tradeoff in `thin_walls.rs`'s module doc). |
| thinWallRenderer.ts | Ported | `thin_walls.rs` | Phase 5 | Geometry/orientation, same-texture-vs-mixed material split, and zone tagging all landed and wired. Edge blocking is wired for player movement, interaction, projectiles, and enemy pathfinding (`fix: enemy tick parity across layers, holes, thin walls, and attacks`). |
| transitionOverlay.ts | Ported | `transition.rs` | — | Fade phase machine, speed, `isActive` gating match. |
| trapLauncherRenderer.ts | Unported | — | Phase 3 | |
| tripwireRenderer.ts | Ported | `tripwires.rs` (`spawn_tripwires`) | — | Zone-tagged (Phase 5). |
| wallEntityRenderer.ts | Ported | `wall_entities.rs` (`spawn_wall_entities`, `reveal_wall_entity`) | — | Zone-tagged (Phase 5). |

### `src/hud/`

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| attributePanel.ts | Ported | `attribute_panel.rs` (`KeyL`) | — | Points earned via level-up are spendable — allocation, levelup/stats mode auto-select, close gated on all points spent in levelup mode. |
| compassRose.ts | Ported | `hud.rs` (`draw_compass`) | — | |
| dialogOverlay.ts | Ported | `dialog_overlay.rs` | — | Session state, choice highlight/select (arrows/Enter/digits/mouse), Escape dismiss. |
| healthBar.ts | Ported | `hud.rs` (`draw_health_bar`) | — | Bar, low-HP pulse, heart icon, HP text match. |
| hudCanvas.ts | Ported | `hud.rs` (`draw_hud`) | — | `draw_hud` now orchestrates the full TS list: compass, torch, hunger, status tints/icons, minimap, inventory panel, XP bar, level-up hint/toast, sword swing, message, damage flash, plus every overlay (save/load, dialog, inventory, attribute, stats, quest log, trading) drawn on top per `ActiveOverlay`. |
| hudColors.ts | Ported | `hud.rs` (const palette) | — | Torch/hunger/compass/minimap colors all in use by the now-wired panels. |
| hudFont.ts | Ported (superset) | `hud_font.rs` | — | Identical glyphs; Rust adds `-`/`+`/`:`/`(`/`)`. |
| hudLayout.ts | Ported | `hud.rs` (const layout) | — | COMPASS/MINIMAP/TORCH_BAR/HUNGER_BAR/STATUS_ICONS/INVENTORY_OVERLAY layout constants all backing real draw calls now. |
| hungerBar.ts | Ported | `hud.rs` (`draw_hunger_bar`) | — | |
| characterCreation.ts | Ported | `char_creation.rs` | — | Same points budget/min-stat/panel geometry; pixel font substitutes native canvas text. |
| inventoryOverlay.ts | Ported | `inventory_overlay.rs` (`KeyI`) | — | Full-screen interactive inventory: cursor, drag-and-drop equip/unequip/rearrange, double-click use, right-click drop, tooltips (`item_tooltip.rs`). Deliberate deviation from Phase 4 review: clears drag state on close (TS keeps a stale drag across close/reopen) — see the note below the rendering table. |
| inventoryPanel.ts | Partially ported | `hud.rs` (`draw_inventory_panel`) | — | Drawing matches (key count, gold, equipment + paperdoll ghosts, cooldown overlay, backpack grid). Mouse interaction on the mini-panel itself (hover, drag-to-equip, double-click use, right-click drop) is superseded by the full-screen `inventoryOverlay.ts` port rather than duplicated on the mini-panel, a reasonable scope choice not yet explicitly disclosed as such. |
| itemTooltip.ts | Ported | `item_tooltip.rs` | — | Quality-colored name, stat lines, comparison deltas, requirements, wrapped description — drawn by the inventory overlay next to the cursor-selected slot. |
| levelUpNotification.ts | Ported | `hud.rs` (`draw_level_up_toast`) | — | Distinct from `draw_level_up_hint` (the persistent "press L" prompt), matching TS's own `LevelUpNotification` vs a separate hint. |
| minimapRenderer.ts | Ported | `hud.rs` (`draw_minimap`) | — | |
| paperdollIcons.ts | Ported | `hud.rs` (`paperdoll_path`, `IconCache`) | — | Identical slot→path mapping. |
| questLogOverlay.ts | Ported | `quest_log_overlay.rs` (`KeyJ`) | — | |
| saveLoadOverlay.ts | Ported | `save_load_overlay.rs` (`Escape` to open, unified death mode with `KeyR` restart) | — | Save/Load/Delete/Export/Import/Restart bound to arrow-navigable rows in Rust rather than TS's click-only buttons — a UI adaptation, not a gap; see the keyboard map below. |
| signOverlay.ts | Unported | — | — | `SignRead`/`BookshelfRead` results computed and logged via `info!`, no popup — `signs.rs`'s own module doc confirms no dedicated visual state exists yet. |
| statsPanel.ts | Ported | `stats_panel.rs` (`KeyT`) | — | |
| statusEffectIcons.ts | Ported | `hud.rs` (`draw_status_icons`) | — | Active effect icons above the health bar, deduplicated by type. |
| torchIndicator.ts | Ported | `hud.rs` (`draw_torch_indicator`) | — | Fuel drain wired via `player_controller.rs`'s `should_drain_torch` + `session.rs`. |
| tradingOverlay.ts | Ported | `trading_overlay.rs` | — | Deliberate deviation from Phase 4 review: click region is the whole row (TS binds only the small button) — see the note below the rendering table. |
| xpBar.ts | Ported | `hud.rs` (`draw_xp_bar`) | — | LV label, MAX-at-cap, fill ratio, progress text match. |

Upstream content bug, inherited faithfully: `questgiver_hilda.json`'s "The spider queen is dead." choice requires the `spider_queen_killed` flag, but nothing in the TS reference sets it either (no kill-time flag mechanism exists in TS src/ or its data) — the bounty quest is uncompletable in both implementations. Report upstream rather than inventing a kill-flag hook here.

Deliberate shell deviations from the phase 4 review, kept: the trading overlay's click region is the whole row (TS binds only the small button; the row-click surfaces the same guard toasts TS's handler carries, so feedback is a superset); the inventory overlay clears drag state on close (TS keeps a stale drag across close/reopen — a quirk, not a behavior worth reproducing). Dialog choices are keyboard-only (TS also supports mouse hover/click) — joins the phase 6 polish list with the overlay PNG icons.

Known shell gap: TS pushes blocks both by face+interact and by walking into them (a `setOnMoveBlocked` movement hook); the Rust shell implements face+interact only — the movement-blocked hook doesn't exist in `player.rs` yet. The same missing hook covers player-initiated boulder pushing (walking into a pushable boulder, and block-push transferring momentum to an adjacent boulder): `BoulderInstance.pushable` exists in core but nothing in the shell reads it. Revisit both together when the move-blocked hook lands.

Latent core gap (phase 6 check): `EntityRegistry::ground_items(level_id, col, row)` ignores `layer_index` while `all_ground_items_for_level` filters by it — the after-pickup remainder query could cross layers at overlapping cells. No shipped level overlaps ground-item cells across layers today.

Benign warning, TS parity: loading a multi-layer level whose layer-0 signal entities target entities on other layers (e.g. test_m4g's levers → boulders on layers 3/4) logs `lever target "..." must reference an existing entity id — entity skipped`. Both loaders validate the top-level layer-0 *mirror* against layer-0-only ids (TS levelLoader.ts:649+684, Rust level_loader.rs:1326) while the per-layer validation uses the global id set. The runtime reads only `layer_def.entities`, so the skipped mirror entries are inert and the levers work.

Phase 5 note: TS has a latent ground-item key-format bug (initial meshes keyed unprefixed, pickup hides with a layer-prefixed key — level-authored items leave a ghost billboard after pickup). `ground_items.rs` keys consistently and does not reproduce it. When layer-aware keying lands in phase 5, decide whether to preserve that incidental correctness or match TS bit-for-bit.

Zone tagging (multi-zone levels): every cell-positioned entity type — signs, fountains, altars, barrels, bookshelves, boulders, ramps, props, and wall entities — is tagged to its own cell's zone via a spawn-time `zones::tag_cell` call inside each entity's own spawn function (col/row are in scope there). Sconces tag all four mesh children (bracket, arm, handle, head), not just the handle+head pair `SconceParts.torches` tracks for the extinguish-on-take toggle. TS's `enableAll` (shared-across-every-pass) set — stairs, trap launchers, sconce lights, health bars, and projectiles — stays untagged on the Rust side too, matching Bevy's default-layer-0 behavior. The zone-boundary split landed for dungeon geometry and for the door mesh; ramp landings are still whole — see the note below.

Half-tile infrastructure: the zone-boundary split landed in `dungeon.rs` (`ZoneSplit`/`half_mesh`) — a door cell that two zones meet inside cuts its floor, ceiling, and the walls crossing the boundary into halves tagged to each zone, with UVs squeezed so the texture scale matches whole tiles, putting the indoor/outdoor seam in the doorway rather than at a tile edge. `doorRenderer.ts`'s Z-split frame/panel and its boundary PointLight landed too (`doors.rs`): a boundary door cuts its pillars, lintel, and panel along the passage, tags the buttons by the side they face, and hangs an indoor-side light whose intensity follows the panel's travel. Still open on the same theme: the half tiles TS carves at ramp landings (`ramps.rs` module doc).

### `src/npcs/`, `src/enemies/`, `src/level/`

| File | Status | Rust location | Phase | Notes |
|---|---|---|---|---|
| npcDatabase.ts | Ported | `npcs.rs` | — | Data-loading/query layer matches; dialog linking and runtime interaction are wired via `dialog_overlay.rs`. |
| enemyAI.ts | Ported | `enemy_ai.rs` | — | Regen/status ticking, flee/chase/attack state machine, deaggro buffer, erratic movement match line-for-line. The game-side tick loops every layer with per-layer hole/edge-blocking snapshots, gates attacks to the player's layer, and rolls onHit status effects (`enemies.rs::tick_enemies`). |
| enemyDatabase.ts | Ported | `enemies.rs` | — | Struct model and queries match. |
| enemyTypes.ts | Ported | `enemies.rs` (`create_enemy_instance`) | — | Field-for-field, including conditional regen-timer init. |
| pathfinding.ts | Ported | `pathfinding.rs` | — | `manhattanDistance` and BFS `findPath` match exactly. |
| interaction.ts | Ported | `interaction.rs` | — | Every branch matches including message text; `session.rs::interact_input` reacts to door/sconce/lever/block/chest/NPC/fountain/altar results with real mesh/state updates (see the `Space` row in the keyboard map). Only `SignRead`/`BookshelfRead` have no dedicated visual state beyond the generic HUD toast, matching TS. |
| levelLoader.ts | Ported | `level_loader.rs` | — | Full parity test suite. |

---

## COMPLETED.md vs PORT-PLAN.md

Cross-reference of shipped TS milestones against this repo's phase prose. Rows are sorted plan-gap-first.

| Feature | Covered by PORT-PLAN.md? | Already in Rust? | Notes |
|---|---|---|---|
| Compass rose + minimap + exploration/fog-of-war reveal | NO — PLAN GAP | Implemented | `session.rs` calls `game.reveal_around` on movement; `hud.rs`'s minimap reads `explored_cells` for fog-of-war. Never promoted into a PORT-PLAN.md phase bullet despite being done. |
| Torch fuel HUD indicator + fuel-scaled light range/flicker + outdoor/mist fuel-skip rule | NO — PLAN GAP | Partial | Fuel drains (`player_controller.rs::should_drain_torch`, correctly skips outdoor/mist), and `hud.rs::draw_torch_indicator` displays it. `torch.rs`'s light intensity/flicker is still a flat constant (`FLICKER_BASE_INTENSITY`) — it does not read `torch_fuel` at all, so TS's fuel-scaled light distance (3-8) and flicker intensity is still missing. |
| Full-screen Inventory overlay (drag-and-drop equip/unequip/rearrange) | NO — PLAN GAP | Implemented | `inventory_overlay.rs`. Never promoted into a PORT-PLAN.md phase bullet despite being done. |
| Debug commands: noclip, fullbright, auto-kill, layer-fly | NO — PLAN GAP | NO | Still nothing — no `KeyM`/`KeyY`/`KeyH` binding anywhere; `boulders.rs`'s `debug_fullbright` field is a hardcoded stub. No phase bullet anywhere mentions debug/QA tooling. |
| Camera asymmetric frustum crop / telephoto back-offset | NO — PLAN GAP | Implemented | `zones.rs`'s `camera_view_crop`/`apply_camera_view_crop` (Bevy `SubCameraView`), wired into `main.rs`'s Update schedule, with unit tests pinning the exact TS `setViewOffset` math. Landed during this Phase 6 audit session — never promoted into a phase bullet despite being done. |
| Multi-layer dungeons, hollow areas, cross-layer signals | Phase 5 | Implemented (`level_scene.rs` per-layer spawn loop, all entity types; hollow-area detection in `dungeon.rs::VerticalOpenness`) | Runtime-live. |
| Thin walls with edge blocking | Phase 5 | Implemented (`thin_walls.rs` renders, zone-tags; edge blocking consulted by player movement, interaction, projectiles, and enemy pathfinding) | |
| Ramps/stairs geometry, layer transitions, falling | Phase 5 | Implemented (`ramps.rs` geometry/movement, `player.rs` falling with kinematic Y-channel, pit traps) | |
| Pit traps (signal-driven) | Phase 5 | Implemented (`PitTrapState` runtime + floor visibility toggle in `dungeon.rs`) | |
| Enemy spawners (BFS placement) | Phase 5 | Implemented (`spawners.rs` core `tick_spawners` + render shell, wired) | Boulder spawners (`boulders.rs`) landed alongside. |
| Decorative props | Phase 5 | Implemented (`props.rs`, zone-tagged) | |
| Outdoor environment, skybox variants, multi-pass RenderLayers | Phase 5 | Implemented (`zones.rs` multi-camera architecture, `skybox.rs`) | Zone-boundary splitting landed for dungeon geometry and doors, boundary door lights included; ramp landings remain whole. |
| Forest billboards; particles | Phase 5 | Implemented (`forest.rs` + `forest_placement.rs`; `particles.rs` — all four systems plus light culling) | |
| Signal system + lever/plate/trigger/tripwire/gate entities | Phase 3 | Implemented (`signal_manager.rs` + `doors.rs`/`levers.rs`/`plates.rs`/`tripwires.rs`) | Doors/levers/plates/tripwires all render; triggers and gates are logic-only invisible entities in TS too, so nothing to render there. |
| Projectiles + trap launchers | Phase 3 | Partial (`projectiles.rs` core + render shell implemented) | Projectiles render and are wired (`tick_projectiles` in `main.rs`). Trap launchers remain unwired — no mesh, no tick call site. |
| Status effects (poison/slow/burning, tints, icons) | Phase 3 | Implemented | Screen-tint overlays and status icons render (`draw_status_screen_tints`, `draw_status_icons` in `hud.rs`); applies to both player and enemies. |
| Environment entities (breakable/secret walls, blocks, chests, signs) | Phase 3 | Implemented | All four render (`wall_entities.rs`/`blocks.rs`/`chests.rs`/`signs.rs`). Sign read text still has no popup overlay (logged + HUD toast only). |
| Save/load (slots, autosave, export/import, overlay) | Phase 3 | Implemented | `save_load_overlay.rs` — slots, death mode with `KeyR` restart, arrow-navigable rows. |
| NPCs (billboard, interaction, flags) | Phase 4 | Implemented (`npcs.rs`) | Billboard rendering, interaction opening a dialog session, and flags all wired. |
| Dialog system | Phase 4 | Implemented | `dialog_overlay.rs` — full overlay UI, keyboard + mouse choice selection. |
| Quest system | Phase 4 | Implemented | `quest_log_overlay.rs` — full overlay UI; `QuestManagerRes` wired end to end including save/load restore. |
| Doors/keys/locked doors/stairs, cross-level transitions | Phase 2 | Implemented | Matches `PROGRESS.md`'s claims. |
| Enemies: AI, special behaviors, melee, XP, death, loot | Phase 2 | Implemented | Regen/flee/erratic behaviors fully ported despite the plan bullet only naming "AI movement" generically. |

---

## Keyboard input map

TS dispatcher: `src/game/inputSystem.ts`. Nine other TS files register independent context-gated `keydown` listeners (`dialogOverlay.ts`, `saveLoadOverlay.ts`, `signOverlay.ts`, `questLogOverlay.ts`, `tradingOverlay.ts`, `characterCreation.ts`, plus editor-only files — out of scope per `DECISIONS.md` D2). On the Rust side the same context-gated split holds: `session.rs`/`enemies.rs`/`char_creation.rs` handle dungeon/attack/character-creation input, and each overlay owns its own `KeyCode::` checks (`attribute_panel.rs`, `inventory_overlay.rs`, `quest_log_overlay.rs`, `save_load_overlay.rs`, `dialog_overlay.rs`, `stats_panel.rs`, `trading_overlay.rs`) rather than `hud.rs`/`main.rs` centralizing them.

| Key | TS action (context) | Rust status | Phase | Notes |
|---|---|---|---|---|
| W/↑, S/↓, A, D, Q/←, E/→ | Move/strafe/turn (dungeon) | Bound — same action | Phase 1 | `session.rs::player_input`, identical key set. |
| Space | Interact (doors, levers, sconces, chests, signs, bookshelves, fountains, altars, blocks, NPCs) | Bound — same action | — | `interaction::interact` covers all 14 result types; `interact_input` (`session.rs`) now reacts to door/sconce/lever/block/chest/NPC/fountain/altar results with real mesh/state updates, including opening a live dialog session on `NpcInteracted`. Only `SignRead`/`BookshelfRead` fall into the catch-all (no dedicated mesh update needed — see the `signOverlay.ts` row), and every message (including those) now also shows as a HUD toast via `effects.hud.show_message`, not just `info!()`. |
| F | Attack | Bound — same action | Phase 2 | |
| Digit1-Digit8 | Use backpack consumable at quick-slot N | Bound — same action | — | `session.rs`, `Digit1`-`Digit8` mapped to backpack slots 0-7. |
| KeyI | Toggle inventory overlay | Bound — same action | — | `inventory_overlay.rs`. |
| KeyL | Open attribute panel | Bound — same action | — | `attribute_panel.rs`; the HUD hint now leads somewhere. |
| KeyT | Toggle stats panel | Bound — same action | — | `stats_panel.rs`. |
| Arrows/Enter/KeyD (inventory-overlay context) | Cursor, equip/unequip/use, drop | Bound — same action | — | `inventory_overlay.rs`: arrow-key cursor, Enter equip/unequip/use, `KeyD` drop, plus full mouse drag-and-drop. |
| Arrows/Enter/KeyL/Escape (attribute-panel context) | Select stat, allocate, confirm/close | Bound — same action | — | `attribute_panel.rs`. |
| KeyJ | Toggle quest log | Bound — same action | — | `quest_log_overlay.rs`. |
| Escape (no overlay) | Open save/load overlay | Bound — same action | — | `save_load_overlay.rs`. |
| Escape (any panel context) | Close that panel | Bound — same action | — | Every overlay module (`attribute_panel.rs`, `stats_panel.rs`, `inventory_overlay.rs`, `quest_log_overlay.rs`, `trading_overlay.rs`, `dialog_overlay.rs`, `save_load_overlay.rs`) closes on Escape. |
| Any key (sign overlay) | Dismiss sign text | **Unbound** | — | No sign popup exists yet — see the `signOverlay.ts`/`signRenderer.ts` rows. |
| Escape (dialog overlay) | Dismiss dialog | Bound — same action | — | `dialog_overlay.rs`. |
| Arrows/Enter/Digit1-9 (dialog, has choices) | Highlight/select/confirm choice | Bound — same action | — | `dialog_overlay.rs`: `ArrowUp`/`ArrowDown` highlight, `Enter` confirms, digit keys select directly, mouse hover/click also supported. |
| Any key (dialog, no choices) | Advance dialog | Bound — same action | — | `dialog_overlay.rs`'s `any_key_just_pressed`. |
| KeyJ/Escape (quest log open) | Hide quest log | Bound — same action | — | |
| Escape (trading overlay) | Hide trading overlay | Bound — same action | — | Buy/sell itself is mouse-only in both. |
| Escape (save/load overlay) | Hide overlay | Bound — same action | — | Slot rows are additionally arrow-navigable in Rust (TS is click-only for the whole overlay) — a superset, not a gap. |
| Arrows/Enter (character creation) | Select/adjust/confirm | Bound — same action | — | `char_creation.rs` matches the TS `handleKey` 1:1, including the points-spent gate on Enter. |
| KeyR (save/load overlay, death mode) | Restart current level | Bound, no TS keyboard equivalent | — | `save_load_overlay.rs`'s death-mode restart; TS only exposes Restart as a click-only button (see the mouse-driven section below), so this is a Rust-side keyboard superset, not a divergence to fix. |
| KeyM | Debug: fullbright + noclip | **Unbound** | — | No `KeyM` binding anywhere; `boulders.rs`'s `debug_fullbright` field is a hardcoded `false` stub, not a real toggle. Dev tooling, not in any phase. |
| KeyY / KeyH | Debug: fly to next/prev layer | **Unbound** | — | Dev tooling, not in any phase. |

### Mouse-driven TS input (tracked separately)

- **Inventory overlay**: hover-cursor, left-click equip/use, right-click drop, full drag-and-drop.
- **Trading overlay**: buy/sell and close are click-only; Escape only closes.
- **Save/load overlay**: Save/Load/Delete/Export/Import/Restart are click-only buttons; Escape only closes.
- **Dialog overlay**: choice buttons also support hover-highlight and click-to-select, redundant with the keyboard paths above.
- **Sign overlay**: clicking the backdrop also dismisses it, redundant with "any key" dismiss.

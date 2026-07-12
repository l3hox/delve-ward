# PHASE5-PLAN.md

Implementation plan for Phase 5 (M4 parity, the vertical world) per `PORT-PLAN.md`: multi-layer levels, environment zones, thin walls, ramps, falling, pit traps, props, enemy spawners, boulders, forest, particles, light culling.

**Load-bearing finding**: `delve-core`'s layer model is already ahead of `delve-game`. `GameState::new` (`game_state.rs:858-891`) already builds one independent `LayerState` per `LayerDef` and parses every layer's entities into it. `mesh_key(layer_index, col, row)` and `layer_door_key(layer_index, key)` (`game_state.rs:57-64`) already exist, unused anywhere, and are byte-for-byte the same convention as TS's `meshKey`/`layerDoorKey`. `entity_by_id`-equivalent cross-layer signal wiring (`GameState.entity_by_id`, `signal_manager`) already works layer-agnostically, matching TS's flat entity-id signal graph. `find_stair_level` + `find_entity_layer_index` (`transition.rs`, `level_loader.rs:49-60`) already resolve a target *level and layer* together — `find_entity_layer_index` searches a single level's `layers` for an entity id, so the existing stairs cross-level transition path already generalizes to "same level, different layer" stairs with no core changes. **The gap is almost entirely in `delve-game`'s rendering and movement code, which only ever reads the active layer and has no Y-offset concept at all.**

---

## 1. Multi-layer rendering

TS builds every layer's full geometry and entity meshes into the scene simultaneously at level-load time (not lazy/per-active-layer), each Y-offset by `layer_index * LAYER_HEIGHT` (or an explicit `LayerDef.yOffset` override), and lets the player's vertical position/camera determine what's visually "current." `LAYER_HEIGHT = WALL_HEIGHT = 2.5` (`rendering/dungeon.ts:9-12`) — layers stack flush, floor of N+1 sits on ceiling of N. This value already exists in Rust as `dungeon::WALL_HEIGHT`; add `pub const LAYER_HEIGHT: f32 = WALL_HEIGHT;` for call-site clarity, matching TS's separate name for the same value.

Concrete changes:

- **`dungeon::spawn_dungeon`** gains `layer_index: usize` and `y_offset: f32` params. It currently reads `DungeonLevel.grid`/`.char_defs`/`.areas`/`.defaults` — these are validation-time convenience copies of `layers[0]`'s fields (see `types.rs:141,146` doc comments) and must be swapped for `level.layers[layer_index].grid`/`.entities` plus the level-wide `char_defs`/`defaults`/`areas` merged with any per-layer `LayerDef.defaults`/`.areas` override (`LayerDef` carries its own `defaults`/`areas` fields — verify the exact TS merge precedence in `sceneUtils.ts`/`texture_resolver.rs` at implementation time rather than guessing it here). All spawned mesh transforms get `+ y_offset` on Y. Hollow-area handling (`TextureArea.openBottom`/`openTop` skip floor/ceiling geometry) folds into this same per-cell loop.
- **Every existing `spawn_*` function** (`doors.rs`, `stairs.rs`, `enemies.rs`, `ground_items.rs`, `keys.rs`, `sconces.rs`, `levers.rs`, `plates.rs`, `tripwires.rs`) needs the same `layer_index`/`y_offset` treatment, reading from `level.layers[layer_index]`/`game.layers[layer_index]` instead of `game.active_layer()`. Their handle maps (`DoorPanels.by_key: HashMap<String, Entity>` etc.) currently key by plain `door_key(col,row)`, which collides across layers — re-key with `layer_door_key(layer_index, door_key(col,row))`, exactly mirroring TS's `mergeMap()` (`levelSceneBuilder.ts:240-244`).
- **`level_scene::spawn_level_scene`** gets a `for layer_index in 0..level.layers.len()` loop wrapping the existing per-entity-type calls, computing `y_offset = layer_def.y_offset.unwrap_or(layer_index as f64 * f64::from(LAYER_HEIGHT))`.
- **Gameplay logic needs no changes.** `GameState`'s `active_layer()`/`active_layer_mut()` accessors and every method built on them (`get_door`, `get_stair`, `is_edge_blocked`, etc.) already transparently resolve to whichever layer is active — the same proxy pattern as TS's `get activeLayer()`. Movement, interaction, combat, and signal systems in `session.rs`/`enemies.rs` need zero changes for correctness; only rendering-side systems that must resolve a `WorldEvent`'s `(col,row)` back to the right layer's spawned Bevy entity need the new layer-prefixed keys.
- **New core accessor needed**: nothing in `GameState` currently lets a caller peek a *non-active* layer without mutating `active_layer_index`. TS papers over this with a save/restore-`activeLayerIndex` pattern everywhere; that pattern doesn't work in Rust because the `MoveRules` closures (needed for ramps, §4) only get `&GameState`, not `&mut GameState`. Add `GameState::layer(&self, index: usize) -> Option<&LayerState>` (a thin `self.layers.get(index)`) in the Slice 0 core work below.

---

## 2. Environment zones in Bevy 0.19

TS uses **one camera, N sequential `renderer.render()` passes**, gated by `camera.layers` bitmask swaps, with `renderer.autoClear` disabled and a single upfront `clear(true,true,true)` so passes 2..N draw additively on the shared color+depth buffer (`main.ts:1433-1453`). Per-pass fog/background/ambient are scene-level object mutations between passes (`scene.fog = new THREE.Fog(...)`, `ambient.color.set(...)`), not per-camera state, because Three.js only has one camera here.

**Bevy's idiomatic equivalent is structurally different and better-suited: N camera *entities*, not N passes of one camera** — verified against the vendored Bevy 0.19.0 source, no invented APIs:

- `RenderLayers` (`bevy_camera-0.19.0/src/visibility/render_layers.rs:18`) is a `Component` attachable to both cameras and renderables; intersection-tested, not capped at a small bit count for the non-const constructors.
- `Camera.order: isize` (`bevy_camera-0.19.0/src/camera.rs:387`) — "cameras with a higher order are rendered later, and thus on top" — replaces TS's implicit loop-order-is-draw-order.
- `Camera.clear_color: ClearColorConfig` (`bevy_camera-0.19.0/src/camera.rs:400`, enum at `clear_color.rs:13-23`) — `ClearColorConfig::None`'s doc comment explicitly names "multiple cameras rendering to the same viewport" as its use case. This is the direct Bevy equivalent of TS's `autoClear=false` + single upfront clear.
- `DistanceFog` (`bevy_pbr-0.19.0/src/fog.rs:51`) and `AmbientLight` (`bevy_light-0.19.0/src/ambient_light.rs:9`, `#[require(Camera)]`) are both **per-camera components** already, confirmed by this project's own existing use in `main.rs`/`transition.rs`. This means per-zone fog/ambient is set once at camera-spawn time, not mutated every frame like TS.

**Design**: reserve zone index 0 as "shared, always visible" (mirrors TS's convention of leaving Three.js layer 0 unused by per-cell content — TS achieves the same effect differently, via `layers.enableAll()` on shared objects rather than a reserved bit, but a reserved shared layer is simpler and doesn't require tracking the level's max zone count). Assign each distinct `Environment` found on a layer's grid a 1-based zone index in first-encountered order (matches `buildEnvZoneMap`, `environment.ts:102-125`). Spawn one camera per zone, each tagged `RenderLayers::from_layers(&[0, zone_index])`, `Camera { order: zone_index as isize, clear_color: if zone_index == first_zone { ClearColorConfig::Custom(fog_color) } else { ClearColorConfig::None }, ..default() }`, plus that zone's `DistanceFog`/`AmbientLight` from `environment_config`. Tag "shared" entities (stairs, trap launchers, sconce lights, particles, damage numbers — the TS `layers.enableAll()` list) with `RenderLayers::layer(0)`. **Single-zone levels skip this entirely** and keep the current one-camera setup, matching TS's `!multiZone` fast path.

**Boundary splitting**: TS splits geometry only at door cells whose neighbor is a different zone (half-size floor/ceiling/wall quads, UVs rescaled to 0.5 — `dungeon.ts:16-36,187-341`); all other zone-boundary cells are whole-cell single-zone-tagged. Port this into the per-cell loop being refactored in §1: compute a zone map per layer (see below), and when spawning a door cell whose N/S/E/W neighbor differs in zone, emit split half-meshes instead of one whole mesh, each with its own `RenderLayers`. Door frame/panel splitting (`doorRenderer.ts:104-347`, including the boundary entrance `PointLight` tagged to the higher zone) belongs in `doors.rs`'s equivalent slice.

**Zone map builder**: pure grid-scan logic (given a layer's grid + char defs + areas + declared `Environment` → per-cell zone assignment + zone list), portable to `delve-core` with unit tests, independent of §1's rendering refactor — do this as its own parallel slice.

**Skybox**: TS's skybox is a `SphereGeometry(radius=180, ...)` with `BackSide`, `depthWrite:false`, `renderOrder:-1`, and a **procedurally-drawn 2D canvas texture** (not a cubemap) — the exact same "draw to a canvas, wrap as a texture" pattern this project already uses in `pixel_canvas.rs`/`textures.rs`. Bevy 0.19 does ship a native `Skybox` component (`bevy_light-0.19.0/src/probe.rs:227`), but it requires a **cubemap** `Handle<Image>`, which doesn't fit this project's procedural-2D-texture convention or TS's actual technique. **Recommend hand-building the inverted sphere** (consistent with the codebase's existing procedural-mesh pattern) over adopting Bevy's `Skybox` component. Radius 180 is safely inside the existing camera's `far: 200.0` (`main.rs`), matching TS's own explicit constraint. Three variants (`starry-night`/`daylight`/`sunset`) already have a matching Rust `Skybox` enum (`types.rs:18-26`) — port the three canvas-drawing routines into `pixel_canvas.rs`-style procedural texture functions. TS uses unseeded `Math.random()` for stars/clouds; the project's established convention (`D10` in `DECISIONS.md`) is to seed procedural textures with `mulberry32` keyed by texture name — follow that, not TS's unseeded version.

---

## 3. Falling + pit traps

**Core state already exists and needs no changes**: `PitTrapInstance`/`PitTrapState` (`game_state.rs:455-472`), signal wiring (`on_source_activated`, `sync_signal_receiver_states`), and `WorldEvent::PitTrapSignalChanged` are all already ported. The gap is entirely the trigger detection and the player-side fall animation.

**Player needs a second, additive Y channel.** TS keeps `yOffset` separate from `currentPos.y` (`camera.position.y = currentPos.y + yOffset`, `player.ts:317-318`); Rust's `Player::update` currently sets `transform.translation = self.current_pos` directly with no such channel (`player.rs:197`). Add `y_offset`/`target_y_offset: f32` fields to `Player`. **This same channel is shared by ramps (§4)** — ramps just lerp it at the normal tween alpha; falling drives it with real kinematic integration (below), gated by an `is_falling: bool` that takes priority over the ordinary lerp path.

**Fall trigger** (two paths, both TS `main.ts:646-681` and `:734-789`):
1. Post-move: after a successful step, check if the destination is a hole — layer-below cell not solid/opaque, or an open pit trap there. Landing layer = first solid-floored layer scanning downward, defaulting to layer 0.
2. Signal-driven: a pit trap opens under a currently-standing player (no move event) — same landing-layer computation, triggered directly from the `WorldEvent::PitTrapSignalChanged` handler.

Both need a shared `is_hole(col, row, layer_index)` check. TS actually has **two, slightly different** hole predicates (the simple fall-trigger one above, and the boulder system's `isHoleAt` in §5 which additionally treats a boulder/block already occupying the cell below as "plugging" the hole, and respects `TextureArea.openBottom`). **Recommend unifying these into one `GameState::is_hole` method** parameterized to match each caller's exact needs (e.g. an `ignore_occupants: bool` flag) rather than porting two near-duplicate predicates that can silently drift apart — flagged as a deliberate, disclosed deviation, not a silent one.

**Fall state machine** (`player.rs` extension): `pending_fall: Option<{landing_layer, total_distance}>` is set on trigger but doesn't start until the in-progress walk tween crosses 2/3 progress (`FALL_TRIGGER_PROGRESS = 0.667`) — this lets the player visually step into the pit before dropping. On crossing: clear the command queue (queued moves are discarded, not resumed after landing — verified, not a guess), set `is_falling = true`, `target_y_offset = landing_layer * LAYER_HEIGHT`, `target_pitch = FALL_CAMERA_PITCH (-0.4 rad)` (reuses the existing pitch-lerp channel unchanged). While falling, integrate `y_offset` kinematically each frame — `accel = 40 u/s²` up to `terminal_velocity = 20 u/s`, with the acceleration phase capped at `2 * LAYER_HEIGHT = 5.0` units of fall distance — bypassing the normal position lerp entirely for this channel. **Logical `col`/`row` never change during a fall**; only `y_offset` and, on landing, `active_layer_index` + the grid/walkable-set + `reveal_around`. Landing fires the equivalent of TS's `onFallLand` callback.

---

## 4. Ramps

Geometry: two styles, both single-cell-span (bottom layer L → top layer L+1, adjacent cell, direction = `facing`) — `'ramp'` is one sloped quad + triangular side fills (`rampRenderer.ts:196-240`); `'stairs'` is 8 stepped tread/riser quads (`RAMP_STEP_COUNT=8`, step height `LAYER_HEIGHT/8`). `RampInstance` (`game_state.rs:432-439`) has no explicit "to-layer" field — the top cell is always `ramp.col + facing_delta` one layer up, matching the existing per-layer storage partition. `CELL_SIZE`/`LAYER_HEIGHT` already match Rust's constants exactly — no new geometry constants needed beyond `LAYER_HEIGHT` itself.

**`is_ramp_accessible` MoveRules hook** (currently `None` in `session.rs`, per the existing stub): implement as a closure checking two directions:
- **Going up**: is there a ramp on the *current* layer at `(from_col, from_row)` whose `facing`-delta matches the move direction?
- **Going down**: peek *layer − 1*'s ramps (via the new `GameState::layer()` accessor from §1 — the closure only has `&GameState`, so a save/restore-active-layer-index dance TS uses isn't viable here) for one whose top cell equals `(from_col, from_row)` and whose facing is the exact reverse of the move.

This mirrors `PlayerState::can_move_to`'s existing "ramp-accessible is a fallback when unwalkable; edge-blocked only applies when walkable" ordering (`grid.rs:161-183`) — no changes needed there, it's already correctly shaped.

**Layer transition on ramp crossing**: unlike stairs (which trigger `Transition::begin` — a fade-out/scene-teardown/fade-in cross-*level* move), a ramp crossing is a same-level, same-scene, Y-shifted move, since §1 already renders every layer simultaneously. Add ramp-crossing detection to `session::on_player_moved` (the existing post-move hook): if the move crossed a ramp edge, update `active_layer_index`, the grid/walkable-set, and set the player's `target_y_offset` — using the **ordinary lerp**, not the fall's kinematic integration. **This must not touch `transition.rs`.**

---

## 5. Spawners + boulders — port strategy

Both systems are overwhelmingly pure logic with a thin Bevy coupling surface, following the same shape as the already-completed `projectiles.rs`/`player_controller.rs` ports (GameState + grid + injected RNG closure in, a decision/event out).

**Spawners — pure logic, `delve-core/src/spawner_system.rs` + tests**: the entire BFS candidate search (4-directional flood fill up to `spawn_radius` hops, candidate = unoccupied non-player non-block cell, traversal blocked by non-walkable or, for non-flying enemies, "hole" cells), the interval/max-active gating (`spawn_timer` accumulates and *subtracts* the interval on fire rather than resetting — overshoot carries over, port this exactly), and uniform-random candidate selection are all zero-Three.js-dependency (`spawnerSystem.ts:28-90`). **Shell-only**: marker mesh + procedural rune texture (`spawnerRenderer.ts`) and the actual enemy mesh spawn (reuses `enemies.rs`'s existing billboard spawn helper).

**Boulders — pure logic, `delve-core/src/boulder_system.rs` + tests**: the full state machine (`decideNext`, `boulderSystem.ts:211-316`) — falling→rolling transition with landing damage, hole-fall with `computeLandingLayer`, ramp descent via `checkRampDescent`, cell-entry classification (`canBoulderEnter`: blocked/kill_enemy/damage_enemy/damage_player/enter), chain-reaction pushing when hitting another boulder (note: `pushable` is **not** checked here — it's only consulted for player-initiated pushes elsewhere), just-landed blocked-ahead handling, dead-end turning, `isHoleAt` (unify with §3's `is_hole`), `crashChestIfAny`'s destroy-and-drop decision, and boulder-spawner interval/mode gating are all pure `GameState` + grid logic with a handful of callback hooks (kill/damage/loot) that become injected closures, matching the established pattern. **Shell-only**: sphere mesh + procedural rock texture + position/rotation tweening (`boulderRenderer.ts`/`boulderAnimator.ts`).

**The one real coupling point**: the TS tick only advances a boulder's logical state once its *visual* tween is at rest (`ls.boulderAnimator.getMode(...) !== 'rest'` gates re-entry). The Rust port needs an equivalent "is this boulder mid-tween" signal passed **into** the core tick from the Bevy-side animator — an injected `is_resting: &dyn Fn(&str) -> bool` closure, the same shape as `enemy_ai::update_enemies`'s injected `is_door_open`. Getting this gating wrong (checked too early/late relative to the animator's actual completion) is the single riskiest part of this port — see closing risks.

---

## 6. Slice plan

Single-writer-per-file discipline, same as the phase 3 work. "Parallel with" lists slices with no file or state overlap that can run concurrently.

| # | Slice | Files (owned) | Depends on | Parallel with |
|---|---|---|---|---|
| 0 | Core layer/hole accessors | `delve-core/src/game_state.rs` | — | 3, 4 |
| 1 | Multi-layer scene foundation | `delve-game/src/{dungeon,level_scene,doors,stairs,enemies,ground_items,keys,sconces,levers,plates,tripwires}.rs` | 0 | 2a, 3, 4 |
| 2a | Environment zone map builder (pure logic + tests) | `delve-core/src/environment_zones.rs` (new) | — | 0, 1, 3, 4 |
| 2b | Zone multi-camera rendering + boundary splitting | `delve-game/src/{environment,dungeon,doors}.rs`, new `zones.rs` | 1, 2a | 5, 9, 10, 11, 12 |
| 3 | Spawner system (pure logic + tests) | `delve-core/src/spawner_system.rs` (new) | — | 0, 1, 2a, 4 |
| 4 | Boulder system (pure logic + tests) | `delve-core/src/boulder_system.rs` (new) | 0 (for `is_hole`) | 1, 2a, 3 |
| 5 | Player Y-offset channel + ramp movement/rendering | `delve-game/src/{player,session,ramps.rs (new)}` | 0, 1 | 2b, 9, 10, 11, 12 |
| 6 | Falling + pit trap movement/rendering | `delve-game/src/{player,session,dungeon}.rs` | 5 | — (sequential after 5, touches `player.rs` again) |
| 7 | Thin wall rendering | `delve-game/src/thin_walls.rs` (new) | 1 | 5, 6, 9, 10, 11, 12 |
| 8 | Enemy spawner rendering shell | `delve-game/src/spawners.rs` (new) | 1, 3 | 5, 6, 7, 9, 10, 11, 12 |
| 9 | Boulder rendering shell | `delve-game/src/boulders.rs` (new) | 1, 4 | 2b, 5, 6, 7, 8, 10, 11, 12 |
| 10 | Decorative props rendering | `delve-game/src/props.rs` (new) | 1 | 2b, 5, 6, 7, 8, 9, 11, 12 |
| 11 | Forest instanced billboards | `delve-core/src/forest_placement.rs` (new, seeded scatter math) + `delve-game/src/forest.rs` (new) | 1, 2b (outdoor zones) | 2b, 5, 6, 7, 8, 9, 10, 12 |
| 12 | Skybox | `delve-game/src/skybox.rs` (new), `pixel_canvas.rs` (texture helpers) | — | everything |
| 13 | Particles (dust motes, embers, water drips, fireflies) + light distance culling | `delve-game/src/particles.rs` (new); embers extend `sconces.rs` | 1 (for embers' per-layer sconce sources); otherwise none | most of the above |

Slices 0, 2a, 3, 4 have zero rendering dependencies and should run first/in-parallel — they're the highest-leverage, lowest-risk work and unblock everything else. Slice 1 is the critical-path bottleneck: nearly every rendering slice (2b, 5 onward) depends on it, so it should be scheduled as early and as undistracted as possible. Slices 5→6 are intentionally sequential (both touch `player.rs`), not parallel, despite looking independent — same for any two slices that would otherwise both touch `dungeon.rs` after slice 1 lands (2b and 6 both need a `dungeon.rs` follow-up; sequence them). Light distance culling (13) has no dedicated TS research behind it yet in this plan — scope it with a quick source check at implementation time rather than the detail given to the rest of this document.

---

## Three riskiest design decisions

1. **N-camera-entities-per-zone rendering, replacing TS's single-camera/N-passes model.** This is a genuinely new rendering architecture for this codebase — the first departure from "one camera renders everything." Every API used (`RenderLayers`, `Camera.order`, `ClearColorConfig::None`, per-camera `DistanceFog`/`AmbientLight`) is verified against the vendored Bevy 0.19 source, but none of them have been exercised together in this project yet, and multi-camera-same-viewport stacking is exactly the kind of thing that looks correct on paper and produces subtle z-fighting or draw-order bugs in practice.
2. **A single shared Player Y-offset channel driving both ramp lerp and fall kinematic integration.** This matches TS's actual design (one `yOffset` field, two different drivers), but the state-machine interaction between "ordinary lerp toward a ramp's target Y" and "kinematic fall integration that bypasses lerp entirely" needs precise gating (`is_falling` must cleanly pre-empt the ramp path) — a level with a ramp immediately adjacent to a hole is the kind of edge case that will find any gating mistake.
3. **The boulder tick's injected `is_resting` closure**, needed because core logic must not advance a boulder's state until its Bevy-side visual tween finishes. If this closure reports "resting" even one frame early or late relative to the animator's actual completion, boulders will either skip a state (visually teleporting) or stall (never advancing) — this is the tightest core/shell coupling point in the whole phase.

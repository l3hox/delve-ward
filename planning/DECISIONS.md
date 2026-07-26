# DECISIONS.md

Resolved decisions for the port. Autonomous sessions follow these without asking. New decisions made mid-session are appended here with a one-line rationale.

---

## D1: Engine is Bevy, pinned to 0.19

Bevy has the strongest ecosystem, documentation, and agent familiarity of the Rust options. Its `RenderLayers` plus camera stacking map directly onto the Three.js layer-mask multi-pass zone rendering the TS game uses. The minor version is pinned in `Cargo.toml`; all code targets the 0.19 API, verified against docs.rs when uncertain (Bevy's API churns between minors and code written from memory of older versions is the main hallucination risk).

## D2: Scope is the game runtime only

The 2D dungeon editor, its dialog node-graph editor, its 3D preview, the Vite dev-server API endpoints, `editor.html`, and Playwright e2e are all out of scope. Content authoring stays in the TS repo's web editor. Both projects share the JSON schemas, so levels built there run here.

## D3: Platform is native desktop first

`cargo run` opens a native window. Wasm is not a build gate, but `delve-core` stays platform-clean (no filesystem access baked into logic, no native-only crates) so a wasm target stays cheap later.

## D4: UI is pixel-art in-engine

HUD and overlays (inventory, dialogs, trading, quest log, save/load) are ported from the TS canvas drawing code (`src/hud/`) to in-engine sprite/texture drawing. The retro look is preserved. No egui.

## D5: Workspace split mirrors the TS core/shell boundary

`delve-core` holds pure logic with no Bevy dependency; `delve-game` is the Bevy shell. This is the same separation the TS repo's M4.5 cleanup drives toward (core compiles standalone) and lets logic be ported and tested headless before rendering exists.

## D6: Port target is a fixed TS commit

Parity target: `../DelveWard` branch `main` at `9476c6526ef98b636992a2dfbac00a3853325bea`. The TS repo keeps evolving; this port does not chase it. Schema source of truth: `../DelveWard/DUNGEON-DESIGNER.md` at that commit. The TS repo is read-only from sessions in this repo.

## D7: Assets are a copied snapshot

`assets/` contains `sprites/`, `levels/`, and `data/` copied from the TS repo's `public/` at the target commit. `sprites/old/` was excluded (unreferenced by source or data). Re-sync is manual; when done, note the new source commit here.

## D8: Saves are JSON files

localStorage becomes JSON files under `saves/` (gitignored), keeping the TS SaveData schema so saves stay portable in principle. Slot semantics unchanged: manual slots plus autosave.

## D9: The vitest suite is the parity spec

Each ported module brings its `*.test.ts` tests along as Rust unit tests in the same change. Where TS tests cover behavior that lands in a later phase, the test is deferred to that phase, not dropped. The seeded `mulberry32` PRNG must be ported bit-exact so seeded behavior matches.

## D10: Procedural textures are regenerated at startup

The TS game generates its textures on 2D canvas (walls, floors, ceilings, later skyboxes and particle sprites). These are regenerated at startup by a small software pixel canvas in `delve-game` (alpha-blended rect fills, lines, ellipses) writing raw RGBA buffers wrapped as Bevy `Image` assets with nearest filtering — same drawing operations as the TS generators, no extra image crate. Randomness is seeded per texture name (`mulberry32`) so output is stable across runs; the TS original uses unseeded `Math.random`, so only the visual character must match, not exact pixels.

## D11: Commits are autonomous

Sessions commit and merge without waiting for staged review. The user reviews git history afterward. This overrides the global stage-for-review rule by explicit user request.

## D12: Level validation runs over raw JSON, output is typed

`validate_level`/`validate_dungeon` inspect `serde_json::Value` field by field so every error and warning message matches the TS loader exactly (the ported tests assert them), then decode the validated document into typed structs. Warnings are collected into a caller-provided `Vec<String>` instead of `console.warn`.

## D13: TS module singletons become explicit parameters

The TS enemy/npc database singletons consulted during validation become a `ValidationContext` holding optional id sets; `None` reproduces the unloaded-singleton behavior (enemy entities skipped, npc check bypassed). Databases parse via `from_json(&str)` constructors; "not loaded" is expressed as `Option<Database>` at the call site.

## D14: Out-of-union strings in shipped data parse as Unknown

`assets/data/items.json` ships one item with `"type": "armor-steel"`, outside the TS `ItemType` union; the TS runtime loads it anyway since unions are compile-time only. `ItemType`/`ItemSubtype` therefore carry a `#[serde(other)] Unknown` variant so such items load but match no type filter, same as the TS runtime.

## D15: Default dungeon is ruins.json, overridable by CLI argument

The TS shell's production fallback is `/levels/ruins.json` with a `?level=` URL override. The Rust shell mirrors both: `delve-game` loads `levels/ruins.json`, `delve-game <name>` loads `levels/<name>.json`. Ruins exercises stairs, transitions, and environment presets that the earlier `dungeon1.json` smoke level lacked.

## D16: All runtime visual randomness is seeded

D10's seeding convention extends past startup textures to every runtime visual RNG in `delve-game` — light flicker, particle spawn and drift — even where TS uses unseeded `Math.random()`. Each system seeds its own `Mulberry32` with a fixed per-system constant (first instances: `sconces.rs`'s `SconceFlicker` and `torch.rs`'s `TorchFlicker`), so behavior is reproducible across runs while only the visual character must match TS.

## D17: The player's grid copy is synced explicitly on every runtime mutation

TS hands its player controller the same `string[]` the game state mutates, so `damageBreakableWall`'s and `openSecretWall`'s in-place writes reach walkability checks for free. Rust can't share one `Vec<String>` between the `Session` resource and the `Player` component, so `Player` keeps a clone and every site that opens a cell calls `Player::open_cell` alongside the `GameState` call. Chosen over threading the grid through `Player`'s whole move API (`run`, `debug_move`, `update`, plus every call site) or wrapping it in shared interior mutability; the mutation sites are few and each is a single added line. New grid-mutating features must call both.

## D18: Ground item sprites render at double the TS size

`EQUIPMENT_SIZE`/`CONSUMABLE_SIZE` in `ground_items.rs` are 0.8/0.7 against TS's 0.4/0.35 — a deliberate art-direction deviation requested during playtesting, not a port bug. `height` still derives from `size` (`size / 2 + 0.02`), so the larger sprites rest on the floor unchanged. `SPREAD_RADIUS` stays at TS's 0.3, so several items dropped in one cell now overlap more than in TS; revisit only if that reads badly. The first intentional break from numeric parity (D9 covers faithful porting of behaviour).

## D19: The HUD canvas stores two pixels per drawing unit

TS's HUD is a 640x360 canvas stretched over the viewport with `image-rendering: pixelated`, which caps every icon at the canvas' own resolution: the 32x32 item PNGs had to be resampled down to a slot's 20x20 inner box, discarding detail the art contains. `PixelCanvas` now stores `HUD_SCALE` (2) pixels per drawing coordinate, so the drawing grid, layout constants, and on-screen appearance are all unchanged while `blit_icon` gets 40 stored pixels for a 32-pixel sprite.

`blit_icon` picks its filter by direction: bilinear interpolation when the sprite is smaller than its stored footprint, area averaging when larger (the 43x43 paperdoll silhouettes). Interpolation was chosen over point sampling on request: 32 pixels across 40 has no whole-number mapping, so point sampling doubles one pixel in five and leaves an uneven grid, which reads worse than the softness interpolation trades it for. Swapping `blit_bilinear_stored` for a point-sampled equivalent in that one branch reverts the look.

Measured on ruins, release build, vsync off: total frame time 2.2ms at scale 1 against 4.7ms at scale 2, both inside the 8.3ms a 120Hz display allows and well inside 16.7ms for 60. The cost is fill work over four times the pixels; an opaque-write fast path in `blend_stored_pixel` was tried and measured no better (1093us against 1084us for the HUD alone), so the cost is pixel volume rather than blend arithmetic. Setting `HUD_SCALE` to 1 restores TS's storage resolution and the lower cost; going past 2 buys nothing for 32x32 art and costs quadratically.

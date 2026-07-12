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

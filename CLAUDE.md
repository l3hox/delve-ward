# DelveWard-rust: CLAUDE.md

Auto-loaded at session start. Project identity, autonomy contract, workflow rules.

---

## Project Overview

**DelveWard-rust** is a Rust port of DelveWard, a grid-based first-person dungeon crawler (Grimrock-style). The TypeScript original lives at `../DelveWard` (Three.js, Vite). This repo ports the **game runtime only** to Rust with Bevy. The dungeon editor is not ported; content authoring stays in the TS repo's web editor, and both projects read the same JSON schemas.

Port target: full game-runtime parity with the TS repo `main` branch at commit `9476c6526ef98b636992a2dfbac00a3853325bea`. That covers everything playable through milestone M4: multi-layer dungeons, ramps, falling, thin walls, pit traps, spawners, signals, props, NPCs, dialogs, quests, trading, hunger, combat, status effects, save/load.

Solo side project by Jakub (senior backend engineer). Primary goal: building skills in agent-assisted end-to-end project development. Sessions in this repo run **fully autonomously**.

---

## Tech Stack

| Concern | Choice |
|---|---|
| Engine | Bevy 0.19 (pinned minor, native desktop window) |
| Language | Rust, toolchain pinned in `rust-toolchain.toml` |
| Data | serde + serde_json; JSON schemas identical to the TS repo |
| UI | In-engine pixel-art HUD and overlays, ported from the TS canvas drawing code. No egui. |
| Art | Sprite/texture snapshot copied from the TS repo into `assets/` |

---

## Architecture

- `crates/delve-core`: pure game logic. Level model, grid, game state, signals, combat, quests, save data. **Never depends on Bevy** or any rendering, windowing, or platform crate. Compiles and tests standalone.
- `crates/delve-game`: the Bevy application. Windowing, rendering, input, asset loading, HUD drawing. Consumes `delve-core`. Game rules never live here.
- `assets/`: snapshot of the TS repo's `public/` content (sprites, levels, data). Schema reference: `../DelveWard/DUNGEON-DESIGNER.md`.
- `planning/`: PROGRESS.md (state), PORT-PLAN.md (phases), DECISIONS.md (resolved decisions).

---

## Autonomy Contract

Sessions here run without user interaction. **Do not ask the user questions.**

- Pre-resolved decisions live in `planning/DECISIONS.md`. Follow them.
- When a new decision is needed, pick the smallest reasonable option consistent with existing decisions, append it to `planning/DECISIONS.md` with a one-line rationale, and continue.
- If something is genuinely blocked (missing asset, contradictory spec), record it under Known Issues in `planning/PROGRESS.md` and move on to the next task.
- Commit autonomously in this repo. The user reviews via git history, not staged diffs. This intentionally overrides the global stage-for-review rule.

---

## Session Workflow

On session start:

1. Read `planning/PROGRESS.md` for the current phase and next steps.
2. Read the current phase section of `planning/PORT-PLAN.md`.
3. Treat `../DelveWard/src` as the porting spec. It is **read-only**; never modify the TS repo.

During work:

- Port module by module. Port the matching vitest tests (`../DelveWard/src/**/*.test.ts`) as Rust unit tests in the same change; they are the parity spec.
- Faithful port: same mechanics, same numbers, same JSON schemas. If TS behavior looks like a bug, port it faithfully and note it in Known Issues.
- Bevy API targets 0.19 exactly. When unsure about an API, check docs.rs for 0.19 instead of writing from memory of older versions.

Phase gates, all required before merging a phase to main:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo run` smoke test whenever rendering or input changed

On phase end: update `planning/PROGRESS.md`, add a CHANGELOG.md entry, merge to main.

---

## Git Workflow

- Branch per phase (`port/phase-1-walkable-skeleton`), merge to main when gates pass.
- Conventional commits: `type: description`, lowercase, no scope.
- Main always compiles, always passes tests.
- Public remote: `l3hox/delve-ward`. Push `main`; the `port/phase-*` branches are local history.

---

## Benchmarking

Performance claims here are measured, never argued. Three rules learned the hard way:

- **Measure on an idle machine.** A baseline taken while anything else compiles is worthless — one such reading "proved" a 31% win that was 0.4% when re-run alone.
- **Put the probe around the whole thing you changed**, allocation included, or you measure the wrong span.
- **`tick_enemies` and other gated systems log nothing** in a smoke run: the game boots into character creation and they return early. Bypass the gate for the duration of the measurement.

Recipe: `PresentMode::AutoNoVsync` plus `FrameTimeDiagnosticsPlugin`, release build, ~16s run, take the median. Revert every probe before gating, and grep to confirm none survived.

Negative results are worth committing. Two allocation-shaped optimisations of `draw_hud` measured flat (D19, PHASE7-PLAN); recording that stopped a third attempt.

## Subagent Briefs

- Agents get their own worktree (`isolation: "worktree"`) so their gate runs don't collide, and disjoint file ownership stated explicitly.
- Agents never commit. The integrator verifies each claim against the TS source, re-runs gates in the main tree, then commits.
- **No screen capture.** One agent's `screencapture` caught the user's live desktop instead of the game window.
- Treat `PARITY-GAPS.md` as untrusted when working from it: it has carried claims that were false in both directions, and one propagated into a work plan before anyone checked the TS source.

---

## Coding Rules

- `#![forbid(unsafe_code)]` in every crate.
- Library code returns `Result`; `unwrap()`/`expect()` only in tests and `main.rs`. Use `Result<T, String>` until a richer error type is warranted; no anyhow/thiserror.
- Saves are JSON files under `saves/` (gitignored), same SaveData schema as the TS repo.
- Pixelart textures load with nearest-neighbor filtering.

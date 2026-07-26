# PHASE7-PLAN.md

Closing the remaining parity gaps found during the phase 6 playthrough. Every
item below was verified against both codebases before being listed — the
inventory in `PARITY-GAPS.md` had drifted, and correcting it is itself the
last item here.

Ordered so that low-risk correctness lands first, the two visible rendering
gaps next, and the changes that touch shared geometry or per-frame cost last,
where a regression is easiest to attribute.

Phase gate is unchanged: `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`, and a smoke run per
level whose rendering the change touches. Pure functions get unit tests; ECS
spawn paths are verified by smoke run, following this crate's existing split.

---

## 1. Logic deviations (small, testable, no rendering)

**`drop_item` recalculates `max_hp` — not a gap. Already matches TS.**
`gameState.ts:683` is `if (r) this.maxHp = this.getEffectiveStats().maxHp;`,
the same reassignment `game_state.rs:2756` makes. The long-standing
`PARITY-GAPS.md` claim that TS "does not" recalculate is false and was carried
forward here without being checked against the TS source. Closed by pinning the
real behaviour instead: `drop_item_recalculates_max_hp_from_effective_stats`
drops a VIT-bearing item and asserts `max_hp` returns to base, so the parity
holds if anyone later "fixes" it toward the phantom gap.

**`EntityRegistry::ground_items` ignores the layer.** `entities.rs:144` filters
on level/col/row only, while its sibling `all_ground_items_for_level` also
filters by layer. Add the layer parameter and thread it through callers. No
shipped level stacks ground items at the same cell across layers, so this is
latent — the test has to construct the overlap itself.

**No asset existence sweep.** TS's `assetCheck.ts` walks every enemy sprite and
item icon path at startup and reports missing files; ours validates schemas
only, so a missing PNG is a silent invisible sprite. Extend
`crates/delve-core/tests/assets_gate.rs` to assert every sprite path referenced
by `enemies.json`, `npcs.json`, and `items.json` resolves to a file on disk.
A test fits this port better than TS's runtime check: it fails the gate rather
than the player's session.

## 2. Torch fuel drives the torch light

Fuel drains and the HUD shows it, but `torch.rs` never reads `torch_fuel` — its
range and flicker are the full-fuel constants, so a dying torch lights the room
exactly like a fresh one. The mapping is `main.ts:1395-1409`:

```
fuel_ratio  = torch_fuel / max_torch_fuel
light_scale = if fuel_ratio >= 0.35 { 1 } else { fuel_ratio / 0.35 }
main range  = 4.5 + light_scale * 7.5      // 12.0 at full, our TORCH_RANGE
fill range  = 3.0 + light_scale * 6.0      //  9.0 at full, our FILL_RANGE
base flicker intensity = 1.2 + light_scale * 4.2   // 5.4 at full, our constant
flicker target = base + random * FLICKER_RANGE * light_scale
```

Our three constants are exactly the full-fuel values, which is good evidence the
port is otherwise faithful — the scaling is simply absent. `light_scale` is a
pure function of the ratio and gets unit tests at the 0.35 knee and either side
of it; the system change is then mechanical. Note the flicker *range* scales
too, so a guttering torch is both dimmer and steadier.

**Found while implementing this item, not yet done:** TS gates only the
*flicker* update behind `!anyOverlayOpen` — `torchLight.distance` and
`torchFillLight.distance` update unconditionally. `torch_update` has no overlay
gating at all, so the torch keeps flickering under an open overlay. Pre-existing
and separate from the fuel scaling; fold it in whenever `torch.rs` is next open.

## 3. Trap launcher meshes

Launchers tick, fire, and hit correctly — only the mesh is missing, so
projectiles emerge from blank wall. Port `trapLauncherRenderer.ts`: a dark iron
body (0.20 x 0.15 x 0.10) with a darker recessed nozzle (0.10 x 0.07 x 0.02)
mounted at `LAUNCHER_HEIGHT` 1.2, which is deliberately `PROJECTILE_HEIGHT` so
projectiles leave the nozzle rather than above or below it.

Two details worth carrying over verbatim: the launcher mounts against the wall
*opposite* its firing direction (it fires through the far wall, so the body sits
at its back), and TS puts launcher meshes in its `enableAll` set, meaning they
stay untagged by zone here — matching Bevy's default layer 0, as `zones.rs`
already documents for the rest of that set.

Spawn per layer from `level_scene.rs` alongside the other entity renderers.

## 4. Pit-trap chambers below an open pit

When a pit opens over solid rock, TS force-renders the cell below so the player
lands in a built chamber rather than blackness (`sceneUtils.ts:309-325` builds a
`forceRenderable` map; `dungeon.ts:137,354` treats those cells as renderable and
non-solid). Ours has neither half: `dungeon.rs`'s `is_solid_for_wall` documents
the omission already.

Port both halves together — the set is computed from the layer above's open
pits and passed into `spawn_dungeon`, where it forces the cell renderable and
its neighbours' walls to generate.

**Correction to this item as originally written.** It claimed that computing the
set once at scene build is "TS's limitation" and therefore faithful. It is not:
`main.ts:748-764`, inside `onPitTrapSignalChanged`, disposes and rebuilds the
whole layer below by calling `buildLayerDungeonGeometry` again, which recomputes
`forceRenderable` from current pit state. **TS builds the chamber on demand.**
The build-time-only behaviour is this port's limitation, inherited from that
rebuild being unported — and it means the landed half only ever fires for pits
authored `"state": "open"`. In `pit-traps.json` exactly one pit qualifies; the
other nine sit over rock but start closed, so they still open into blackness.

### Follow-ups this item uncovered — both still open

**The layer-below rebuild on pit signal** (`main.ts:748-764`). Without it the
chamber-building above is inert for signal-opened pits, which is most of them.
This is the piece that makes item 4 actually pay off in play.

**The pit-ceiling toggle** (`main.ts:739-746` against TS's `pitCeilingMap`,
built at `sceneUtils.ts:326-342`): cells two layers below a pit whose ceiling
hides while the pit is open. This port has no equivalent. The comment in
`session.rs` that used to justify skipping it was factually wrong — it claimed
non-topmost layers never spawn ceilings, but `dungeon.rs`'s `ceiling_enabled` is
unconditionally true below the top layer, so those ceilings do exist and simply
never toggle. Comment corrected; the feature is still missing, and
`pit-traps.json`'s open pit exercises it today.

## 5. Ramp landing half-tiles

TS carves half tiles where a ramp meets its landing: `RampCellInfo.floorKeepHalf`
renders one half of the floor, and `rampHalfWalls` keeps only the half of a wall
away from the ramp entrance (`dungeon.ts:202-221,363-401`). `ramps.rs` records
this as deliberately unported *because the half-tile geometry did not exist* —
that blocker is gone, since `dungeon.rs` now has `half_mesh` and the boundary
split that uses it.

Sequenced after item 4 because both add cases to the same per-cell floor and
wall paths in `spawn_dungeon`; doing them together risks conflating two sets of
geometry changes when a smoke run looks wrong.

## 6. Deferred performance work

**`draw_hud` allocates its canvas every frame.** Now a 3.7MB allocation at
`HUD_SCALE` 2 rather than the 900KB it was when the finding was first recorded
(PHASE6-PLAN). Keep a persistent buffer in `HudState` and zero it in place. The
measured HUD cost is ~3.5ms/frame; allocation is a fraction of that, so measure
before and after rather than assuming the win.

**`tick_enemies` rebuilds per-layer snapshots every tick** — a full grid clone
plus fresh door and thin-wall edge sets, per layer, per frame. The fix shape is
caches invalidated by the existing `WorldEvent`s (wall destroyed, door state
changed). Highest-risk item here: a missed invalidation is a subtle behaviour
bug, not a crash. Do it last, and only if a measurement on the heaviest level
justifies it — the phase 6 pass found no observed cost.

## 7. Refresh `PARITY-GAPS.md`

The file still lists the NPC renderer, projectile renderer, sign overlay, debug
tooling, environment lerping, `debugNoClip`, the move-blocked hook, and the
billboard shader as unported; all have landed. Rows touched during phase 6 and
the playthrough fixes were corrected in passing, but the file has never had a
full re-verification since the phase 6 audit.

Deliberately last: doing it after items 1-6 means one sweep against a settled
tree instead of two. The sweep is read-only against both codebases and should
re-verify every row rather than trusting the current text — the same discipline
that caught the drift in the first place.

---

## Explicitly out of scope

Unchanged by this phase, recorded so they aren't mistaken for oversights: the
dungeon editor (D2), browser-only JSON save export/import (D8), and a wasm
build target (D3 — `delve-core` stays platform-clean, but building for wasm is
not a gate).

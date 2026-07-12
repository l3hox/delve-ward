# PHASE4-PLAN.md

Implementation plan for Phase 4 (M3 parity, the living world) per `PORT-PLAN.md`. TS source references are relative to `../DelveWard/src/`. Read alongside `PARITY-GAPS.md` (coverage audit — some per-file rows are stale where a concurrent slice has since landed; this plan calls out where it disagrees with that document) and `DECISIONS.md` (D13 in particular, extended below).

Scope: NPCs, dialog system, quest runtime + log overlay, trading overlay, hunger system, dungeon objects (fountain/bookshelf/altar/barrel), and the full interactive-overlay input map (mouse, inventory overlay drag/drop, quick-slots, attribute panel, stats panel, item tooltips, overlay Escape handling).

---

## 1. Already-done groundwork

`delve-core` has landed, tested, and unwired: `dialog_manager.rs`, `quest_manager.rs`, `player_controller.rs`, `npcs.rs`, `save_system.rs`'s `QuestSaveState`. `dialogs.rs::DialogTree::from_json` and `quests.rs::QuestDef::from_json` already exist — dialog/quest JSON loading follows the same `read_asset` + `from_json` convention `main.rs::load_dungeon` already uses for items/enemies/npcs. `game_state.rs::damage_barrel` (returns `DamageOutcome`) and `get_barrel`/`get_fountain`/`use_fountain`/`get_altar`/`use_altar`/`get_bookshelf_on_wall` all exist. `interaction.rs`'s `InteractionType::NpcInteracted` already returns the NPC's `npc_id` (the *definition* id, e.g. `"merchant_gregor"`, not the instance id) via `InteractionResult.message` — this is the exact string `NpcDatabase::get_npc` expects.

Nothing in `delve-game` references `tick_player_controller`, `PlayerTickState`, `QuestManager`, or `DialogSession` yet — confirmed by grep, not assumed. NPCs are not rendered.

---

## 2. Architecture decisions

### 2.1 Overlay manager

The existing precedent is `char_creation.rs`: a `Resource` with an `active: bool`, a dedicated input system, a dedicated draw function called conditionally from `draw_hud`, and `InputGate` (a `SystemParam` bundling `Res<Transition>` + `Res<CharCreation>`) that gameplay systems check via `.blocked()`.

That pattern doesn't scale cleanly to seven more overlays (dialog, trading, quest log, inventory, attribute, stats, tooltip-is-not-modal) — N booleans across N resources can't express "at most one is open" and requires an N-way `if`/`else if` chain in both the input dispatcher and `draw_hud`. Recommendation: **one `ActiveOverlay` enum resource** replacing the boolean-per-resource approach for every *modal* overlay (tooltip is not modal — see 2.6):

```rust
#[derive(Resource, Default, PartialEq, Eq, Clone, Copy)]
pub enum ActiveOverlay {
    #[default]
    None,
    Dialog,
    Trading,
    QuestLog,
    Inventory,
    Attribute,
    Stats,
}
```

Each overlay's *data* (dialog session, trading NPC id, cursor position, drag state, pending attribute allocations) stays in its own resource, `Option`-wrapped or defaulted when not relevant — mirroring how TS's `visible: boolean` + injected references live per-overlay-instance while `anyOverlayOpen` is a derived OR across all of them. `ActiveOverlay::None` ⇔ TS's `!anyOverlayOpen`. `InputGate::blocked()` extends to `transition.is_active() || creation.active || overlay.0 != ActiveOverlay::None`.

One dispatcher system (`overlay_input`, replacing per-overlay input systems where they'd otherwise duplicate the same Escape-handling boilerplate) matches on `ActiveOverlay` and routes to the right handler; `draw_hud` matches on the same enum instead of chaining booleans. Opening overlay X when overlay Y is already open should replace, not stack (TS never shows two overlays at once either — every `show()` call is paired with hiding whatever's currently up, e.g. the dialog→trading handoff in 2.3).

Character creation is intentionally left out of this enum (it's pre-game setup, not an in-dungeon overlay, and already has its own working gate) — don't fold it in.

### 2.2 Mouse input in Bevy 0.19

Verified against the vendored source (`~/.cargo/registry/src/*/bevy_window-0.19.0`, `bevy_input-0.19.0`):

- `Query<&Window>` → `window.cursor_position() -> Option<Vec2>` gives the cursor in logical pixels relative to the window's top-left, `None` when the cursor is outside the window (`bevy_window-0.19.0/src/window.rs:620`).
- `Res<ButtonInput<MouseButton>>` mirrors the already-used `Res<ButtonInput<KeyCode>>` pattern exactly — `.just_pressed(MouseButton::Left)` / `.just_pressed(MouseButton::Right)` (`bevy_input-0.19.0/src/mouse.rs`). No new event-reading plumbing needed; `DefaultPlugins` registers this resource already (same plugin that already drives `ButtonInput<KeyCode>`).
- `Window::width()`/`height()` (`bevy_window-0.19.0/src/window.rs:562,570`) give the logical window size.

HUD-canvas mapping: the HUD image is rendered as an `ImageNode` with `NodeImageMode::Stretch` filling 100%×100% of the window (`hud.rs::setup_hud`), so the TS `_screenToHud` scale-by-both-axes conversion (`hudCanvas.ts`, confirmed in the inventory-overlay research: scales by `HUD_WIDTH/rect.width` and `HUD_HEIGHT/rect.height` independently, no aspect-ratio preservation) has a direct equivalent:

```rust
fn screen_to_hud(cursor: Vec2, window: &Window) -> Vec2 {
    Vec2::new(
        cursor.x / window.width() * HUD_WIDTH as f32,
        cursor.y / window.height() * HUD_HEIGHT as f32,
    )
}
```

Port this once as a shared `hud::screen_to_hud` function — every mouse-driven overlay (trading, inventory) needs it, matching TS's own "shared logic worth porting once" shape.

### 2.3 QuestManager + DialogSession integration

**Where they live**: `QuestManager` as a `Resource` (built once at startup in `main.rs::setup`, quest defs loaded via `read_asset("data/quests/{id}.json")` + `register_quest_def` for every quest the loaded dungeon can reference — TS's `Promise.all([questManager.loadQuest(...)])` upfront-load is a fetch-cache; the Rust equivalent is register-at-startup since there's no async fetch). `DialogSession` as `Option<DialogSession>` inside a small `DialogOverlayState` resource (holds the session, the current NPC def snapshot, and cursor/highlight state — see 2.1's per-overlay-data note).

**The `DialogEvent` return-value design already supersedes TS's callback-injection pattern.** TS's `onStartQuest`/`onAdvanceQuest`/`onOpenShop` are module-level nullable function slots installed once via `setDialogHooks` in `main.ts:293-317`, invoked synchronously from inside `dialogManager.ts`'s effect executors. `dialog_manager.rs::execute_effect`/`execute_effects` were already ported *without* that indirection — they return `Vec<DialogEvent>` (`StartQuest(String) | AdvanceQuest(String) | OpenShop(String)`) directly. No hook registration is needed on the Rust side; the dialog-overlay input handler just matches on the `Vec<DialogEvent>` returned from `select_choice`/`advance_dialog` after every call and acts on each variant inline:

```rust
for event in events {
    match event {
        DialogEvent::StartQuest(id) => { quests.start_quest(&id); hud.show_message(&format!("Quest started: {id}")); }
        DialogEvent::AdvanceQuest(id) => { quests.advance_quest(&id, &mut session.game); hud.show_message("Quest updated"); }
        DialogEvent::OpenShop(npc_id) => { overlay.0 = ActiveOverlay::Trading; trading.open(&npc_id, npc_def, ...); }
    }
}
```

This is the same call-site *behavior* as `main.ts:293-317`/`310-316` (quest calls + toast; dialog closes and trading opens on shop), just without the closure-registration ceremony — a legitimate simplification, not a deviation, since `dialog_manager.rs`'s public API already made the callback pattern unnecessary before this phase started.

**The `questStage` condition evaluator is the one place that genuinely needs new plumbing**, because unlike effects (fire-and-forget events), a *condition* needs to synchronously return a bool *during* `evaluate_condition`/`get_available_choices`, and quest status lives in `QuestManager`, which `dialog_manager.rs` currently has zero knowledge of (confirmed: no import). TS's mechanism (`dialogManager.ts:69-79`) is a module-level `Record<string, ConditionEvaluator>` dictionary mutated by `questManager.installConditionEvaluator()` at bootstrap — global mutable state, which `dialog_manager.rs`'s own doc comment already rules out for this crate.

**Recommendation: thread `Option<&QuestManager>` directly through, not a `dyn Fn` trait object.** `QuestManager::evaluate_quest_stage_condition(&self, quest_id: Option<&str>, stage: &str) -> bool` already exists with exactly the right shape. `QuestManager` and `dialog_manager` are both `delve-core` modules with no risk of a cyclic dependency (`quest_manager.rs` already depends on `game_state`; `dialog_manager` depending on `quest_manager` is a natural one-directional addition). This matches D13's actual precedent more literally than a trait would — D13 turned a TS singleton into an explicit *concrete* parameter (`ValidationContext`), not an abstract closure. A `dyn Fn`/trait indirection would only pay for itself if something outside `delve-core` needed to supply quest evaluation, which nothing does.

Proposed signature change (for whoever picks up the dialog-overlay slice — not made by this plan):

```rust
pub fn evaluate_condition(condition: &DialogCondition, game: &GameState, quests: Option<&QuestManager>) -> bool {
    match condition.condition_type {
        ...
        DialogConditionType::QuestStage => quests.is_some_and(|quests| {
            quests.evaluate_quest_stage_condition(condition.quest_id.as_deref(), condition.stage.as_deref().unwrap_or(""))
        }),
        ...
    }
}
```

`evaluate_conditions`, `get_available_choices`, `select_choice`, and `advance_dialog` all need the same new parameter threaded through (they currently call `evaluate_condition`/`evaluate_conditions` internally). `None` reproduces the TS pre-install default (`"undiscovered"` placeholder) exactly, the same `Option`-as-"not loaded" idiom D13 already established for enemy/npc registrars.

### 2.4 NPC billboards + interaction

Billboard geometry, sizing (`size ?? DEFAULT_SPRITE_SIZE`, `size*0.5 + y_offset`), and the "face the camera's yaw" convention are identical to enemies — reuse `billboard::FacesCamera` (already shared across enemies/ground-items/keys) rather than writing a fourth copy. New `npcs.rs` module (delve-game) mirrors `enemies.rs::spawn_enemy_billboards`/`keys.rs::spawn_keys` shape: `spawn_npc_billboards(commands, meshes, materials, asset_server, game, npc_db) -> NpcBillboards { by_key: HashMap<String, Entity> }`.

Move-blocking already works (`is_npc_at` is already in `session.rs::with_move_rules`'s `is_blocked` closure — confirmed in the existing file). Interaction: `interact_input`'s match on `InteractionType::NpcInteracted` gets a new arm — look up `npc_db.get_npc(&npc_id)` (the id from `result.message`), load `data/dialogs/{def.dialog}.json` via `read_asset` + `DialogTree::from_json` (cache in a `HashMap<String, DialogTree>` resource keyed by dialog id — TS's fetch-and-cache-by-id, adapted to synchronous file reads), call `start_dialog`, open the dialog overlay. On a missing/failed-to-parse dialog file, show a one-line HUD message with the NPC's name (`main.ts`'s fallback) rather than panicking — `Result`-propagate the parse error into a log + toast, matching `LibraryCodeNeverPanics`/`ExplicitErrorHandling` conventions already enforced elsewhere in this codebase.

### 2.5 Per-overlay test strategy

Every overlay in this phase splits into **pure logic** (hit-test math, price formulas, condition/action generation, tooltip content assembly) and **rendering** (the actual `draw_*` canvas calls). Following this codebase's existing convention (`hud.rs`'s draw functions are visually verified via `run`, not unit-tested; `char_creation.rs`'s stat-spend logic *is* unit-testable but currently isn't split out — this phase should split it where a TS test exists to port):

- **Dialog overlay**: no TS test file exists (confirmed) — no parity spec to port. Smoke-test via `run` only.
- **Trading overlay**: `tradingOverlay.test.ts` tests exactly two pure functions, `buyPrice`/`sellPrice` — port both as `delve-core` or `delve-game` pure functions (`buy_price(value, markup) -> ceil`, `sell_price(value) -> floor(value*0.5)`) with the same fixture values (10×1.5→15, 7×1.5=10.5→11, 10×0.5→5, 7×0.5=3.5→3, plus the zero-value and markup-1.0/3.0 cases). These belong in `delve-core` (pure arithmetic over `ItemDef.value`, no rendering) even though `tradingOverlay.ts` is a `src/hud/` file in TS — matches how `combat.rs`'s formulas already live in core despite being combat-*feedback*-adjacent.
- **Quest log**: no TS test file — smoke-test only. `QuestManager` itself is already fully tested.
- **Inventory overlay**: `inventoryOverlay.test.ts` covers `subtypeToEquipSlot`, cursor-navigation edge clamping, and `InventoryAction` generation from `Enter`/`KeyD` — port these as delve-game unit tests over synthetic cursor state (no Bevy app needed, same shape as the existing action-generation logic in `player_controller.rs`). **Hit-test and drag-state-machine math has no TS test coverage at all** (confirmed) — write new Rust-only tests directly from the formulas in 4.4 below; there's no upstream fixture to match against, so these are net-new coverage, not ports.
- **Item tooltip**: `itemTooltip.test.ts` covers `getQualityColor`/`getStatLines`/`getComparisonDeltas` as pure functions with concrete assertions — direct ports. The `drawItemTooltip`-equivalent function itself has no meaningful port of the TS "doesn't throw" mock-canvas test; verify via `run` instead.
- **Attribute panel**: `attributePanel.test.ts` has real behavioral coverage (unspent-points-blocks-close, Arrow semantics, stats-mode read-only, VIT-reallocation-preserves-full-HP) — port all of it against the staged local resource described in 4.6.

---

## 3. NPCs and dialog

| Component | TS source | Rust target | Notes |
|---|---|---|---|
| NPC billboard spawn/despawn | `rendering/npcRenderer.ts` | `delve-game/src/npcs.rs` | Same billboard pattern as enemies; `FacesCamera` reuse. |
| NPC interact → dialog start | `game/inputSystem.ts:218-237` | `session.rs::interact_input`, new `NpcInteracted` arm | Loads `data/dialogs/{dialog}.json`, calls `start_dialog`. |
| Dialog overlay render | `hud/dialogOverlay.ts` (DOM — no 1:1 render function, layout intent only) | `delve-game/src/dialog_overlay.rs` | Panel bottom-center, speaker/body/choices/hint regions — see layout constants in the research notes; must be redesigned as a canvas draw routine since TS builds DOM elements once and mutates them. |
| Dialog input | `dialogOverlay.ts:106-145` (self-contained keydown, capture phase) + click/hover | `dialog_overlay.rs` input system, routed via the `ActiveOverlay` dispatcher | Escape always dismisses; with choices: ArrowUp/Down cycle+wrap, Enter selects highlighted, digits 1-9 select directly; without choices: any key advances. Mouse hover sets highlight, click selects. |
| DialogEvent routing | `main.ts:293-317` (hook callbacks) | inline match on `Vec<DialogEvent>` after `select_choice`/`advance_dialog` (see 2.3) | No hook registration needed — already superseded. |
| `questStage` evaluator wiring | `questManager.ts:150-162` (`installConditionEvaluator`) | `Option<&QuestManager>` param threaded through `dialog_manager.rs` (see 2.3) | Signature change to `evaluate_condition` et al. |

---

## 4. Overlays

### 4.1 Trading

Panel: two columns (shop stock / player backpack), no keyboard row-navigation in TS (mouse-only besides Escape) — matches; don't invent keyboard nav that doesn't exist upstream. Buy: `ceil(item.value * (npc.markup ?? 1.5))`, backpack-full and insufficient-gold produce the exact TS message strings (`"Backpack is full!"`, `"Not enough gold!"`) via `hud.show_message`. Sell: `floor(item.value * 0.5)`, flat, no markup. On buy, the created item carries the item def's default modifiers (`def.modifiers` ids) — check `ItemDatabase`/`ItemDef` already exposes a modifiers field before assuming; if not, this is a small `delve-core` addition, not a `delve-game` one. Reuse `IconCache`/`hud.rs`'s existing item-icon-drawing helper for both columns' icons rather than writing a third icon-blit path (mini panel and this overlay both already draw item icons; consolidate).

Opened via `DialogEvent::OpenShop` (2.3) — never opened directly by player input. `ActiveOverlay::Trading` holds `npc_id: String` + a snapshot/reference of `NpcDef` (stock ids + markup) needed to render.

### 4.2 Quest log

`KeyJ` opens (`main.ts` calls `.show(questManager)`, not a toggle — confirm this is genuinely "open only," never a toggle-closed-on-second-press, before wiring `KeyJ` to fire in both directions); `Escape` or a second `KeyJ` closes. Active quests: name + current stage description (`def.stages[quests.get_stage_index(id)].description`) + the quest's overall flavor description, dimmed. Completed: name only, no stage text. No reward preview anywhere in TS — don't add one. Plain scroll, no pagination logic to port (a Rust port can either match with unbounded rendering + canvas clip, or defer scrolling until a level actually has enough quests to overflow 60% of panel height — note as an acceptable initial simplification either way since content-driven overflow isn't testable against current shipped `data/quests/`).

### 4.3 Inventory overlay (full-screen, distinct from the already-shipped mini panel)

This is a **separate, new UI surface** from `hud.rs`'s existing `draw_inventory_panel` (`SLOT_SIZE=24`, always visible, display-only) — the full overlay uses `SLOT_SIZE=28`, opens on `KeyI`, and is the *only* interactive inventory UI in either build. Keep `EQUIP_SLOTS` order (`Weapon, Head, Chest, Legs, Hands, Shield, Feet, Ring1, Ring2, Amulet`) identical between both — already matches TS exactly per direct comparison; the new overlay must index into the same order, not redefine it.

Panel: 460×310, centered in the 640×360 canvas. Equipment: 5 cols × 2 rows starting at `(x + (w - (5*(28+4)-4))/2, y+44)`. Backpack: 4 cols × 3 rows starting `10px` below a separator at `equip_start_y + 2*32 + 6`. Per-slot rect: `col*(28+4)`/`row*(28+4)` offset, inclusive-min/exclusive-max hit test.

Drag state machine (`DragState{source, item_id, hud_x, hud_y, valid_equip_slots}`): `mousedown` on an occupied slot starts a drag (precomputing valid equip-slot targets via `subtype_to_equip_slot` when dragging *from* backpack — both ring slots valid for ring subtype); `mouseup` on a target resolves to `Equip`/`Unequip`/`Swap`/no-op per the exact transition table in the research notes (same-slot = no-op; backpack→equipment rejects consumables and wrong slots except ring↔either-ring; equipment→backpack rejects an occupied target with no auto-displacement; backpack→backpack is always an unconditional `Swap`; equipment→equipment is unsupported). Double-click (not single-click) triggers the "Enter" action (equip/unequip/use, same as `KeyEnter`/gamepad-equivalent semantics in `_handleEnter`); right-click triggers `Drop` (same as `KeyD`). All four (`Equip`/`Unequip`/`Use`/`Drop`/`Swap`) route straight to the already-ported `player_controller::process_inventory_action` — this overlay is purely input→`InventoryAction` translation, no new `GameState` mutation logic needed.

Item tooltip (not modal, not part of `ActiveOverlay`): drawn every frame the cursor rests on an occupied slot (keyboard-cursor or mouse-hover, both feed the same `cursor` field) and hidden during an active drag — no hover-delay timer to port, TS has none. Content order: quality-colored name → type/subtype → non-zero stat lines → deltas vs. the equipped item in the same slot (skip if hovering the equipped item itself) → stat requirements (colored met/unmet against `get_effective_stats`) → wrapped description. Position: 4px right of the slot, same Y; clamp only the right edge (flip to the slot's left if it would overflow past `x=640`) — no vertical clamping in TS, don't add any.

### 4.4 Attribute panel (`KeyL`) and stats panel (`KeyT`)

**Attribute panel reuses `char_creation.rs`'s exact staging pattern** — that resource already stages STR/DEX/VIT/WIS locally and only calls a `GameState` mutation once, on confirm. The attribute panel needs the identical shape: a new resource holding `pending: [i64; 4]` (or four named fields) that starts at zero and is only flushed to `game.allocate_point(stat)` calls (one call per pending point) when the player closes with all points spent. Two modes: `levelup` (auto-opens when `attribute_points > 0`) vs `stats` (read-only, opens otherwise, no unspent-points gate). ArrowUp/Down cycle the selected stat (wrap); ArrowRight/Enter stage +1 (blocked at zero remaining); ArrowLeft retracts a *pending* point (floor at the stat's value before this session opened, never below — this needs a "baseline" snapshot taken at open time, not global game state, since `allocate_point` itself isn't called until confirm). Closing in `levelup` mode with points remaining is a no-op (panel stays open) — same block TS enforces. On successful close, recompute `max_hp` from `get_effective_stats()` preserving the full-HP invariant if the player was at max before a VIT change (this exact behavior — "recalculates max_hp preserving full-HP" — is *already* what `game_state.rs::allocate_point` does per the existing deep-dive audit in `PARITY-GAPS.md`; confirm at implementation time whether calling `allocate_point` once per pending point already produces the right cumulative result, or whether the panel needs its own final recompute pass on top).

Stats panel is pure display: base vs. effective (from `get_effective_stats`, already ported) for STR/DEX/VIT/WIS, plus derived ATK/DEF/HP/CRIT/DODGE computed with the TS formulas (`floor(str/2)`, `floor(vit/4)`, `40+vit*5`, `5+floor(dex/3)`, `clamp(0,25,floor((dex-5)/4))`) — these are the *base-only* formulas TS itself hardcodes for the display, separate from (and should not be confused with) `get_effective_stats`'s modifier-aware formulas used for the "effective" column. `KeyT` toggles (confirmed: `toggle()`/`isOpen()`, no separate open/close verbs); no in-panel interaction beyond toggle+close.

### 4.5 Hunger bar and hunger system

`player_controller::tick_player_controller` already implements hunger drain (`HUNGER_DRAIN_INTERVAL=10.0`) and starvation damage (`STARVATION_INTERVAL=3.0`) — this phase's job is calling it every frame from a new `session::tick_player` system (holding a `PlayerTickState` resource for the accumulators) and wiring `should_drain_torch`'s existing environment check alongside it if that's not already threaded through `torch.rs` (verify at implementation time — `torch.rs` currently drains fuel unconditionally per an earlier phase's `PROGRESS.md` note; confirm whether it already checks environment before this phase touches it, since that call site may already exist and just need the hunger tick added alongside it, not both built from scratch).

`hungerBar.ts` is a structural copy of the already-ported `torchIndicator.ts` (same bar geometry, same `0.2` low threshold) with two differences: a single-sine pulse (`sin(time*6)`) instead of torch's sine-product flicker, and bread-icon/hunger-color constants instead of flame/torch. Port as `draw_hunger_bar` adjacent to `draw_torch_indicator` in `hud.rs`, sharing the bar-drawing helper if one doesn't already exist as a function (check before extracting one net-new — `draw_torch_indicator` may already be structured so hunger can copy-paste-adapt without a shared helper, which would match the codebase's existing preference for direct small duplication over premature abstraction).

Player damage-flash timer (`PLAYER_DAMAGE_FLASH_DURATION` in `player_controller.rs`, already ported) needs a render hook — a full-screen red tint overlay drawn in `draw_hud` when `tick_state.player_damage_flash_timer > 0`, decaying over the timer's duration. Small, but don't skip it — it's part of `tick_player_controller`'s contract, not a separate optional feature.

### 4.6 Dungeon objects

All four (fountain, bookshelf, altar, barrel) are small (45-72 lines in TS) static-geometry spawns using the same `Cuboid`/`Cylinder` + `lambert` material pattern already established in `sconces.rs`/`levers.rs` — no new geometry techniques needed, no procedural textures. Fountain and altar each have a **one-shot** (not per-frame) visual toggle keyed off existing discrete state (`fountain.state == Used` hides the water mesh at spawn *and* on use; `altar.state == Active` gives the pillar emissive glow, removed permanently on use) — this is the same shape as the lever/plate/tripwire "hide/press mesh on state change" pattern from the phase-3 signal-entity work, not a new mechanism. **The altar's temp buff itself has no dedicated visual at all in TS** — it's handled entirely by the unrelated status-effect system; don't build a buff-duration progress ring or anything not present upstream.

Bookshelf is pure static geometry (wall-mounted box + 3 fixed book-spine boxes, facing via the shared `wall_direction` helper already made `pub(crate)` in `sconces.rs`). Barrel is spawn + despawn-on-destroy only — no partial-damage visual stage in TS, so don't build one; `damage_barrel` (already in `game_state.rs`, returns `DamageOutcome`) plus a `combat.rs` barrel-hit/barrel-destroy result type (verify these result variants exist before assuming — combat.rs's `CombatResultType` enum should be checked for barrel-specific variants at implementation time, since the research only confirmed the TS side and the core `damage_barrel` method, not whether `combat.rs`'s attack-resolution path already routes to it) drive the despawn, same call shape as `enemies.rs::handle_kill`'s despawn-then-loot flow.

---

## 5. Input map additions

Building on the already-audited table in `PARITY-GAPS.md` (verify against it directly rather than retyping the whole thing) — this phase's additions:

| Key/action | Context | Wires to |
|---|---|---|
| Digit1-Digit8 | dungeon | `InventoryAction::Use{backpack_slot: n-1}` via `use_consumable_from_registry`/`backpack_item_at` (already exist) |
| KeyI | dungeon | `ActiveOverlay::Inventory` |
| KeyL | dungeon | `ActiveOverlay::Attribute` (auto-selects `levelup` mode if `attribute_points > 0`) |
| KeyT | dungeon | `ActiveOverlay::Stats` |
| KeyJ | dungeon | `ActiveOverlay::QuestLog` |
| Arrows/Enter/KeyD | inventory overlay | cursor nav / equip-unequip-use / drop |
| Left-mouse down/up | inventory overlay | drag start/end |
| Double-click | inventory overlay | equip/unequip/use (same as Enter) |
| Right-click | inventory overlay | drop (same as KeyD) |
| Arrows/Enter/KeyL | attribute panel | select stat / allocate / (KeyL = confirm, same as opening key) |
| Escape | any overlay | close that overlay only (never a global "close everything") |
| Any key | dialog, no choices | advance |
| Arrows/Enter/Digit1-9 | dialog, has choices | highlight/select/confirm |
| Mouse hover/click | dialog choices | same as keyboard highlight/select |
| Mouse only | trading | buy/sell/close — no keyboard row nav exists upstream, don't add any |
| Escape | trading, quest log | close |

---

## 6. Slice breakdown

Ordered by dependency; slices in the same row have no file overlap and can run in parallel once their listed dependencies land.

| # | Slice | Files (new unless noted) | Depends on | Parallel-safe with |
|---|---|---|---|---|
| 1 | `ActiveOverlay` enum + `InputGate` extension | `session.rs` (edit), new `overlay.rs` or fold into `session.rs` | — | 2, 3 |
| 2 | NPC billboards + spawn/despawn wiring | `npcs.rs`, `level_scene.rs` (edit), `main.rs`/`transition.rs` (edit for resource insert/swap) | — | 1, 3 |
| 3 | `questStage` evaluator signature change | `delve-core/dialog_manager.rs` (edit) | — | 1, 2 |
| 4 | Dialog overlay render + input + `DialogEvent` routing | `dialog_overlay.rs` | 1, 2, 3 | 5, 6 |
| 5 | Trading overlay | `trading_overlay.rs` | 1, 4 (opened only via `DialogEvent::OpenShop`) | 6, 7 |
| 6 | Quest log overlay + `QuestManager` resource | `quest_log_overlay.rs`, `main.rs` (edit, quest-def loading) | 1, 3 | 5, 7 |
| 7 | Stats panel (read-only, simplest) | fold into `hud.rs` or new `stats_panel.rs` | 1 | 4, 5, 6, 8 |
| 8 | Attribute panel | new `attribute_panel.rs`, reuses the `char_creation.rs` staging pattern | 1 | 7 |
| 9 | Item tooltip (pure functions first, render second) | new `item_tooltip.rs` | — (data-only; can start immediately) | everything |
| 10 | Inventory overlay (hit-test + drag state machine + render) | new `inventory_overlay.rs` | 1, 2.2 (mouse plumbing), 9 (tooltip) | — (largest slice, do last so mouse plumbing and tooltip are stable) |
| 11 | Hunger bar + `tick_player_controller` wiring | `hud.rs` (edit), `session.rs` (edit, new `tick_player` system) | — | 1 through 10 |
| 12 | Dungeon objects: fountain, bookshelf, altar, barrel | `fountain.rs`, `bookshelf.rs`, `altar.rs`, `barrel.rs`, `level_scene.rs` (edit) | — | 1 through 11 (no shared files except `level_scene.rs`, which every phase-3/4 rendering slice already touches — coordinate that one file's edits, not the rest) |

Slices 9, 11, and 12 have no dependency on the overlay-manager work and are the safest to parallelize against slices 1-8 if multiple agents pick this phase up simultaneously — they touch entirely disjoint new files and only converge on `level_scene.rs`/`main.rs` for resource registration, which is the one recurring merge-conflict risk across this whole phase (every slice that spawns level-scoped entities needs a `LevelSceneHandles` field + a `main.rs`/`transition.rs` resource-swap line — same shape as the lever/plate/tripwire work from phase 3).

---

## Top 3 design risks

1. **The full inventory overlay and the mini inventory panel are two independently-laid-out UIs that must stay in sync on `EquipSlot` ordering and item-state truth, but share zero rendering code today.** A future edit to equip-slot order, added equipment slots, or a new item quality tier has to be applied in both `hud.rs::draw_inventory_panel` and the new overlay, with no compiler-enforced link between them beyond both importing the same `EquipSlot` enum. Recommend a shared `EQUIP_SLOTS` constant lives in one place (not duplicated) even though the two draw functions stay separate.

2. **The `questStage` evaluator signature change (2.3) touches `dialog_manager.rs`, which is otherwise stable, tested, ported code — a mid-phase edit to a foundational function's signature ripples through every call site (`evaluate_conditions`, `get_available_choices`, `select_choice`, `advance_dialog`) and their existing unit tests.** This should land as its own small, first slice (table row 3) with its test suite updated in the same change, *before* any overlay work starts consuming the new parameter — not bolted on opportunistically while building the dialog overlay itself, where a signature mismatch would be a much larger diff to review.

3. **Mouse-driven drag-and-drop (4.3) has zero TS test coverage to port against, unlike almost everything else in this phase.** The hit-test grid math and the equip/unequip/swap transition table in this plan were derived by reading `inventoryOverlay.ts` directly, not from an executable spec — there is real risk of a subtle off-by-one in the slot-index math or a missed edge case (e.g. dragging a ring onto an already-ring2-occupied slot when ring1 is free) that no existing test would catch on either side. This slice needs deliberately over-specified new Rust tests (one per transition-table row, not just happy-path) precisely because it's flying without a parity net.

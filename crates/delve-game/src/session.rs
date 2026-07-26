//! The running game session: `GameState` and the active grid as Bevy
//! resources, plus the input/update systems that connect the player
//! controller, interaction, and world events to rendering.

use crate::altars::{self, AltarHandles};
use crate::blocks::{self, BlockRender};
use crate::chests::{self, ChestHandles, ChestLid};
use crate::doors::{DoorPanel, DoorPanels};
use crate::dungeon::PitFloorHandles;
use crate::fountains::{self, FountainHandles};
use crate::ground_items::{self, GroundItemRender, LootTablesRes};
use crate::keys::{self, KeyBillboards};
use crate::levers::{self, LeverRender};
use crate::plates::{self, PlateRender};
use crate::player::Player;
use crate::sconces::{self, SconceRender};
use crate::transition::Transition;
use crate::tripwires::{self, TripwireHandles};
use crate::wall_entities::{self, WallEntityHandles};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::boulders::can_boulder_roll_to;
use delve_core::game_state::{
    DoorState, GameState, LeverState, MultiLayerSnapshot, PitTrapState, WorldEvent, door_key,
    layer_door_key,
};
use delve_core::grid::{Facing, MoveRules, build_walkable_set, is_walkable};
use delve_core::interaction::{InteractionType, interact};
use delve_core::player_controller::{InventoryAction, process_inventory_action};
use delve_core::random::Mulberry32;
use delve_core::types::{CharDef, Dungeon, DungeonLevel, Environment, TextureArea};
use std::collections::{HashMap, HashSet};

#[derive(Resource)]
pub struct Session {
    pub game: GameState,
    pub grid: Vec<String>,
    pub walkable: HashSet<char>,
    pub current_level_id: String,
    /// Level default environment plus per-area overrides, for cell-local
    /// checks like torch drain.
    pub environment: Environment,
    pub areas: Vec<TextureArea>,
    pub(crate) last_player_pose: (i32, i32, Facing),
}

impl Session {
    pub fn new(
        game: GameState,
        grid: Vec<String>,
        walkable: HashSet<char>,
        current_level_id: String,
        level: &delve_core::types::DungeonLevel,
        start: (i32, i32, Facing),
    ) -> Self {
        Self {
            game,
            grid,
            walkable,
            current_level_id,
            environment: level.environment.unwrap_or(Environment::Dungeon),
            areas: level.areas.clone().unwrap_or_default(),
            last_player_pose: start,
        }
    }
}

/// Seeded gameplay randomness (loot rolls, erratic movement, spawners).
#[derive(Resource)]
pub struct GameRng(pub Mulberry32);

/// The full loaded dungeon, kept for cross-level transitions.
#[derive(Resource)]
pub struct DungeonRes(pub Dungeon);

/// Departed levels' state, restored when the player returns.
#[derive(Resource, Default)]
pub struct LevelSnapshots(pub HashMap<String, MultiLayerSnapshot>);

/// Each level's grid as loaded from disk, captured once in `main.rs::setup`
/// before any runtime mutation (breakable walls, secret walls). Restart and
/// load both use this to reset a level's grid to its pristine state, ported
/// from TS's `originalGrids` map.
#[derive(Resource, Default)]
pub struct OriginalGrids(pub HashMap<String, Vec<String>>);

/// Build the TS movement callbacks from the game state and hand them to `f`.
fn with_move_rules<R>(game: &GameState, f: impl FnOnce(&MoveRules) -> R) -> R {
    let is_door_open = |col: i32, row: i32| game.is_door_open(i64::from(col), i64::from(row));
    let is_blocked = |col: i32, row: i32| {
        let (col, row) = (i64::from(col), i64::from(row));
        game.is_blocked_by_enemy(col, row)
            || game.is_block_at(col, row)
            || game.is_npc_at(col, row)
            || game.is_barrel_at(col, row)
            || game.is_boulder_at(col, row)
    };
    let is_edge_blocked = |from_col: i32, from_row: i32, to_col: i32, to_row: i32| {
        game.is_edge_blocked(
            i64::from(from_col),
            i64::from(from_row),
            i64::from(to_col),
            i64::from(to_row),
        )
    };
    // Ported from `main.ts`'s ramp-accessibility check (the `isRampAccessible`
    // callback `PlayerState` is constructed with): a move is ramp-accessible
    // either going up (a ramp based at the FROM cell, on the player's
    // current layer, facing this move's direction) or going down (a ramp on
    // the layer below whose top cell is the FROM cell, facing the exact
    // reverse of this move). Only `&GameState` is available here, so the
    // "going down" check uses `GameState::layer` to peek the layer below
    // read-only rather than TS's save/restore-`activeLayerIndex` dance,
    // which isn't viable without `&mut GameState`.
    let is_ramp_accessible = |from_col: i32, from_row: i32, to_col: i32, to_row: i32| {
        let delta = (to_col - from_col, to_row - from_row);
        let from_key = door_key(i64::from(from_col), i64::from(from_row));
        let going_up = game
            .active_layer()
            .ramps
            .get(&from_key)
            .is_some_and(|ramp| ramp.facing.delta() == delta);
        if going_up {
            return true;
        }
        game.active_layer_index
            .checked_sub(1)
            .and_then(|below_index| game.layer(below_index))
            .is_some_and(|below| {
                below.ramps.values().any(|ramp| {
                    let (rdx, rdy) = ramp.facing.delta();
                    (from_col, from_row) == (ramp.col as i32 + rdx, ramp.row as i32 + rdy)
                        && delta == (-rdx, -rdy)
                })
            })
    };
    f(&MoveRules {
        is_door_open: Some(&is_door_open),
        is_blocked: Some(&is_blocked),
        is_edge_blocked: Some(&is_edge_blocked),
        is_ramp_accessible: Some(&is_ramp_accessible),
    })
}

/// Finds the currently-loaded level by id — the same
/// `level.id.clone().unwrap_or_else(|| level.name.clone()) == id` lookup
/// `transition.rs` already repeats at each of its own call sites.
pub(crate) fn find_level_by_id<'a>(
    dungeon: &'a DungeonRes,
    level_id: &str,
) -> Option<&'a DungeonLevel> {
    dungeon
        .0
        .levels
        .iter()
        .find(|level| level.id.clone().unwrap_or_else(|| level.name.clone()) == level_id)
}

/// Whether `character` represents solid ground — TS's `ch === '#' ||
/// (def?.solid && !def?.seeThrough)`, the "not a hole" half of the
/// fall-trigger predicate in `main.ts:646-681`/`:770-783`. TS re-inlines the
/// same formula in `rendering/dungeon.ts:163-175` and
/// `rendering/wallEntityRenderer.ts:131-142` (the open-floor/open-ceiling
/// auto-detect); `dungeon.rs::VerticalOpenness` calls this one instead so
/// the fall trigger and the rendered holes can never disagree.
pub(crate) fn is_solid_floor_char(character: char, char_defs: &[CharDef]) -> bool {
    character == '#'
        || char_defs
            .iter()
            .find(|def| def.character == character)
            .is_some_and(|def| def.solid && def.see_through != Some(true))
}

/// Whether the layer directly below `layer_index` is open at `(col, row)` —
/// the player's own fall-trigger hole predicate. **Deliberately separate
/// from `boulders.rs::is_hole_at`**: that one additionally treats an
/// occupying boulder/block as "plugging" the hole and respects
/// `TextureArea.openBottom`, neither of which TS's player-side check does
/// (`main.ts:646-681`) — porting a second, simpler predicate here is the
/// faithful choice per `PHASE5-PLAN.md` §3, not a shortcut. Out-of-bounds or
/// a missing layer-below grid defaults to "not a hole", matching TS's own
/// guard-fails-closed behavior.
fn is_hole_below(level: &DungeonLevel, layer_index: usize, col: i64, row: i64) -> bool {
    let Some(below_index) = layer_index.checked_sub(1) else {
        return false;
    };
    let Some(below) = level.layers.get(below_index) else {
        return false;
    };
    let (Ok(row_usize), Ok(col_usize)) = (usize::try_from(row), usize::try_from(col)) else {
        return false;
    };
    let Some(character) = below
        .grid
        .get(row_usize)
        .and_then(|line| line.chars().nth(col_usize))
    else {
        return false;
    };
    !is_solid_floor_char(character, level.char_defs.as_deref().unwrap_or(&[]))
}

/// Scans downward from `current_layer - 1` for the first layer whose own
/// floor (the layer below *it*) is solid — the cell the player lands on top
/// of. Defaults to layer 0 (the ground floor) if nothing solid is found,
/// matching TS's `landingLayer` scan in `main.ts:664-676`/`:772-782`.
fn compute_landing_layer(level: &DungeonLevel, current_layer: usize, col: i64, row: i64) -> usize {
    for candidate in (1..current_layer).rev() {
        let Some(below) = level.layers.get(candidate - 1) else {
            continue;
        };
        let (Ok(row_usize), Ok(col_usize)) = (usize::try_from(row), usize::try_from(col)) else {
            continue;
        };
        let Some(character) = below
            .grid
            .get(row_usize)
            .and_then(|line| line.chars().nth(col_usize))
        else {
            continue;
        };
        if is_solid_floor_char(character, level.char_defs.as_deref().unwrap_or(&[])) {
            return candidate;
        }
    }
    0
}

/// Player + level context `apply_world_events` needs to trigger a fall from
/// a `WorldEvent::PitTrapSignalChanged { open: true, .. }` landing on the
/// player's own cell — ported from `onPitTrapSignalChanged`'s "player is
/// standing here and it just opened" branch (`main.ts:766-788`). `None`
/// callers (`tick_game`, which has no `Player` query of its own) still get
/// the floor-mesh toggle; only the fall-trigger half is skipped for them —
/// a purely-timed pit trap opening under a motionless, non-interacting
/// player is a deliberately deferred edge case (see the phase-5 report).
pub struct FallTriggerContext<'a> {
    pub player: &'a mut Player,
    pub level: &'a DungeonLevel,
}

/// Rendering-side handles for revealing a secret wall's floor/ceiling/inward
/// walls once it's opened.
#[derive(SystemParam)]
pub struct WallEntityRender<'w, 's> {
    pub handles: Res<'w, WallEntityHandles>,
    pub visibility: Query<'w, 's, &'static mut Visibility>,
}

/// Rendering handles [`move_with_blocked_handling`] needs beyond `Session`
/// itself: secret-wall reveal visuals, block-push mesh animation, plate
/// press/release visuals, and the HUD toast for a secret-wall reveal
/// message.
#[derive(SystemParam)]
pub struct MoveBlockedRender<'w, 's> {
    pub wall_entities: WallEntityRender<'w, 's>,
    pub block: BlockRender<'w, 's>,
    pub plate: PlateRender<'w, 's>,
    pub hud: ResMut<'w, crate::hud::HudState>,
}

/// TS's `directionFromDelta` (`main.ts:816-821`).
fn direction_from_delta(delta: (i32, i32)) -> Facing {
    match delta {
        (_, -1) => Facing::N,
        (_, 1) => Facing::S,
        (1, _) => Facing::E,
        _ => Facing::W,
    }
}

/// Ported from TS's `setOnMoveBlocked` handler (`main.ts:840-950`), fired
/// whenever `attempt` leaves the player's cell unchanged. Three branches,
/// each falling through to the next only if it doesn't apply:
///
/// 1. **Secret wall** (facing-gated — only `moveForward`'s own delta
///    qualifies, matching TS's `isForward` check): reveal it, show a
///    message, retry. Multi-layer neighbor rebuilds are skipped — see
///    `wall_entities`'s module doc comment.
/// 2. **Block push**: a boulder sitting past the block gets nudged instead
///    if it can actually roll there — the block stays put and there's no
///    retry either way, matching walking straight into a boulder. With no
///    boulder past it, the block itself is pushed if the destination is
///    walkable and clear of enemies/blocks/barrels/edges — animate the
///    mesh, press the destination plate if one's there, release the source
///    plate if it was active, then retry.
/// 3. **Direct boulder push**: unguarded — `push_boulder`'s own
///    `pushable && Idle` gate is the only check. An invalid push still
///    starts the boulder rolling; `tick_boulders` bounces or idles it back
///    on the next tick the same way it resolves every other non-idle
///    boulder, regardless of how it got that way.
///
/// `retry` in TS is a closure stored on `this` and invoked later from
/// inside the same callback scope that still holds `gameState`/`ls.player`
/// — awkward to express in Rust without interior mutability. The direct
/// translation is a second, ordinary call to `attempt` after the mutation
/// completes, once the first call's borrow of `session.game` has already
/// ended: no stored closures, the same snapshot-then-mutate shape the enemy
/// tick (and this function's forward-only, secret-wall-only predecessor)
/// already used.
/// A queued command that gets rejected when it finally drains
/// (`Player::update`'s internal replay) never reaches this handler — TS's
/// replayed closures re-invoke the move method, `onMoveBlocked` included, so
/// TS re-fires pushes on every drained attempt while this port fires once
/// per fresh, non-animating press (`main.ts:840-950`; `player.ts`'s queue).
/// A feel difference under held keys, not a correctness one — accepted over
/// threading render params into the drain path.
fn move_with_blocked_handling(
    session: &mut Session,
    player: &mut Player,
    delta: (i32, i32),
    attempt: fn(&mut Player, &MoveRules, bool),
    render: &mut MoveBlockedRender,
) {
    let before = player.grid_state();
    let (before_col, before_row, facing) = (before.col, before.row, before.facing);
    // Captured before the attempt: when a tween is mid-flight the attempt
    // only enqueues (position unchanged without anything being blocked),
    // and TS's onMoveBlocked never fires on enqueue.
    let was_animating = player.is_animating();

    with_move_rules(&session.game, |rules| attempt(player, rules, false));

    let after = player.grid_state();
    if (after.col, after.row) != (before_col, before_row) {
        return; // moved normally, nothing was blocking
    }
    if was_animating {
        return; // the command was enqueued, not attempted — nothing blocked
    }

    let (col, row) = (
        i64::from(before_col + delta.0),
        i64::from(before_row + delta.1),
    );
    let is_forward = delta == facing.delta();

    // --- Secret wall reveal (facing-gated) ---
    if is_forward {
        let should_open = session
            .game
            .get_secret_wall(col, row)
            .is_some_and(|wall| !wall.opened);
        if should_open {
            let (opened, persistent) = session.game.open_secret_wall(col, row, &mut session.grid);
            if opened {
                player.open_cell(col, row);
                wall_entities::reveal_wall_entity(
                    &render.wall_entities.handles,
                    &mut render.wall_entities.visibility,
                    &layer_door_key(session.game.active_layer_index, &door_key(col, row)),
                    persistent,
                );
                render.hud.show_message(if persistent {
                    "An illusionary wall!"
                } else {
                    "A secret passage!"
                });
                with_move_rules(&session.game, |rules| attempt(player, rules, false));
                return;
            }
        }
    }

    // --- Block push ---
    if session.game.get_block(col, row).is_some() {
        let (dest_col, dest_row) = (col + i64::from(delta.0), row + i64::from(delta.1));

        if session.game.is_boulder_at(dest_col, dest_row) {
            let beyond_col = dest_col + i64::from(delta.0);
            let beyond_row = dest_row + i64::from(delta.1);
            if can_boulder_roll_to(
                &session.game,
                &session.grid,
                &session.walkable,
                dest_col,
                dest_row,
                beyond_col,
                beyond_row,
            ) {
                session
                    .game
                    .push_boulder(dest_col, dest_row, direction_from_delta(delta));
            }
            return;
        }

        let (Ok(dest_col32), Ok(dest_row32)) = (i32::try_from(dest_col), i32::try_from(dest_row))
        else {
            return;
        };
        let is_door_open = |c: i32, r: i32| session.game.is_door_open(i64::from(c), i64::from(r));
        let can_push = is_walkable(
            &session.grid,
            dest_col32,
            dest_row32,
            &session.walkable,
            Some(&is_door_open),
            None,
        ) && !session.game.is_blocked_by_enemy(dest_col, dest_row)
            && !session.game.is_block_at(dest_col, dest_row)
            && !session.game.is_barrel_at(dest_col, dest_row)
            && !session.game.is_edge_blocked(col, row, dest_col, dest_row);
        if can_push {
            let src_key = door_key(col, row);
            let dest_key = door_key(dest_col, dest_row);
            let src_was_activated = session
                .game
                .active_layer()
                .plates
                .get(&src_key)
                .is_some_and(|plate| plate.activated);
            let dest_plate_exists = session.game.active_layer().plates.contains_key(&dest_key);

            session.game.push_block(col, row, dest_col, dest_row);

            let layer_index = session.game.active_layer_index;
            let from_render_key = layer_door_key(layer_index, &src_key);
            let to_render_key = layer_door_key(layer_index, &dest_key);
            blocks::animate_block_push(
                &mut render.block,
                &from_render_key,
                to_render_key.clone(),
                dest_col,
                dest_row,
            );

            let layer_y_offset = layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
            if dest_plate_exists {
                plates::press_plate(&mut render.plate, &to_render_key, layer_y_offset);
            }
            if src_was_activated {
                session.game.deactivate_pressure_plate(col, row);
                plates::release_plate(&mut render.plate, &from_render_key, layer_y_offset);
            }

            with_move_rules(&session.game, |rules| attempt(player, rules, false));
        }
        return;
    }

    // --- Direct boulder push (unguarded — push_boulder's own gate decides) ---
    if session.game.is_boulder_at(col, row) {
        session
            .game
            .push_boulder(col, row, direction_from_delta(delta));
    }
}

pub fn player_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    gate: crate::overlay::InputGate,
    mut players: Query<&mut Player>,
    mut render: MoveBlockedRender,
    debug_flags: Res<crate::debug::DebugFlags>,
) {
    if gate.blocked() {
        return;
    }
    let Ok(mut player) = players.single_mut() else {
        return;
    };
    // TS ties `debugNoClip` 1:1 to `debugFullbright` (`inputSystem.ts:342`)
    // rather than exposing a separate flag, so this reads the same one.
    let no_clip = debug_flags.fullbright;

    if keys.just_pressed(KeyCode::KeyW) || keys.just_pressed(KeyCode::ArrowUp) {
        if no_clip {
            with_move_rules(&session.game, |rules| player.move_forward(rules, true));
        } else {
            let delta = player.grid_state().facing.delta();
            move_with_blocked_handling(
                &mut session,
                &mut player,
                delta,
                Player::move_forward,
                &mut render,
            );
        }
    }
    if keys.just_pressed(KeyCode::KeyS) || keys.just_pressed(KeyCode::ArrowDown) {
        if no_clip {
            with_move_rules(&session.game, |rules| player.move_back(rules, true));
        } else {
            let (fdc, fdr) = player.grid_state().facing.delta();
            move_with_blocked_handling(
                &mut session,
                &mut player,
                (-fdc, -fdr),
                Player::move_back,
                &mut render,
            );
        }
    }
    if keys.just_pressed(KeyCode::KeyA) {
        if no_clip {
            with_move_rules(&session.game, |rules| player.strafe_left(rules, true));
        } else {
            let delta = player.grid_state().facing.turned_left().delta();
            move_with_blocked_handling(
                &mut session,
                &mut player,
                delta,
                Player::strafe_left,
                &mut render,
            );
        }
    }
    if keys.just_pressed(KeyCode::KeyD) {
        if no_clip {
            with_move_rules(&session.game, |rules| player.strafe_right(rules, true));
        } else {
            let delta = player.grid_state().facing.turned_right().delta();
            move_with_blocked_handling(
                &mut session,
                &mut player,
                delta,
                Player::strafe_right,
                &mut render,
            );
        }
    }
    with_move_rules(&session.game, |rules| {
        if keys.just_pressed(KeyCode::KeyQ) || keys.just_pressed(KeyCode::ArrowLeft) {
            player.turn_left(rules);
        }
        if keys.just_pressed(KeyCode::KeyE) || keys.just_pressed(KeyCode::ArrowRight) {
            player.turn_right(rules);
        }
    });
}

/// Switches the active layer and both the grid/walkable-set (via the given
/// mutable references) and `Player`'s own copy (`switch_grid`) to match —
/// the common core of TS's `onFallLand` and its ramp-crossing block, which
/// differ only in what runs afterward (falling always reveals around the
/// landing cell; ramp crossing never does, matching TS's own call sites —
/// see [`detect_ramp_crossing`]'s doc comment). Takes `grid`/`walkable` as
/// separate `&mut` (not `&mut Session`) so `on_player_moved`'s own
/// already-destructured `Session` fields can call this directly without
/// re-borrowing the whole struct. Returns whether the switch actually
/// happened (a missing level/layer leaves everything untouched).
pub(crate) fn switch_active_layer(
    game: &mut GameState,
    grid: &mut Vec<String>,
    walkable: &mut HashSet<char>,
    player: &mut Player,
    dungeon: &DungeonRes,
    level_id: &str,
    new_layer: usize,
) -> bool {
    game.active_layer_index = new_layer;
    let Some(level) = find_level_by_id(dungeon, level_id) else {
        return false;
    };
    let Some(layer_def) = level.layers.get(new_layer) else {
        return false;
    };
    *grid = layer_def.grid.clone();
    *walkable = build_walkable_set(
        level
            .char_defs
            .iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );
    let stairs_map = game
        .active_layer()
        .stairs
        .values()
        .map(|stair| (door_key(stair.col, stair.row), stair.direction))
        .collect();
    player.switch_grid(grid.clone(), walkable.clone(), stairs_map);
    true
}

/// The `onFallLand` callback's effects (`main.ts:702-708`): switch the
/// active layer and reveal around the landing cell. Reads
/// `session.current_level_id`'s layers fresh from `DungeonRes` rather than
/// carrying a snapshot from trigger time, since a fall can span several
/// frames.
fn land_on_layer(
    session: &mut Session,
    dungeon: &DungeonRes,
    player: &mut Player,
    landing_layer: usize,
) {
    let level_id = session.current_level_id.clone();
    if !switch_active_layer(
        &mut session.game,
        &mut session.grid,
        &mut session.walkable,
        player,
        dungeon,
        &level_id,
        landing_layer,
    ) {
        return;
    }
    let state = player.grid_state();
    let (col, row, facing) = (i64::from(state.col), i64::from(state.row), state.facing);
    session.game.reveal_around(col, row, facing, &session.grid);
}

/// Detects whether the just-completed move crossed a ramp edge, returning
/// the destination layer if so — ported from `main.ts:607-644`'s two
/// direction checks. TS runs this same-scene, Y-shifted layer switch
/// (`ls.player.targetYOffset = destLayer * LAYER_HEIGHT`, no fade) *after*
/// its `revealAround` call for the move, so the reveal still uses the
/// pre-crossing layer's grid that frame — this port preserves that exact
/// ordering by calling `switch_active_layer` from `on_player_moved` after
/// its own `reveal_around` call, not before.
fn detect_ramp_crossing(
    game: &GameState,
    level: Option<&DungeonLevel>,
    prev_col: i32,
    prev_row: i32,
    col: i32,
    row: i32,
) -> Option<usize> {
    let level = level?;
    let delta = (col - prev_col, row - prev_row);
    let current_layer = game.active_layer_index;

    // Going up: a ramp based at the cell we just left, facing this move.
    let src_key = door_key(i64::from(prev_col), i64::from(prev_row));
    if game
        .active_layer()
        .ramps
        .get(&src_key)
        .is_some_and(|ramp| ramp.facing.delta() == delta)
        && current_layer + 1 < level.layers.len()
    {
        return Some(current_layer + 1);
    }

    // Going down: a ramp on the layer below whose top cell is the cell we
    // just left, facing the exact reverse of this move.
    if current_layer > 0
        && let Some(below) = game.layer(current_layer - 1)
        && below.ramps.values().any(|ramp| {
            let (rdx, rdy) = ramp.facing.delta();
            (prev_col, prev_row) == (ramp.col as i32 + rdx, ramp.row as i32 + rdy)
                && delta == (-rdx, -rdy)
        })
    {
        return Some(current_layer - 1);
    }
    None
}

pub fn player_update(
    time: Res<Time>,
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut players: Query<(&mut Player, &mut Transform)>,
    debug_flags: Res<crate::debug::DebugFlags>,
) {
    let Ok((mut player, mut transform)) = players.single_mut() else {
        return;
    };
    let no_clip = debug_flags.fullbright;
    let landed = with_move_rules(&session.game, |rules| {
        player.update(time.delta_secs(), &mut transform, rules, no_clip)
    });
    if let Some(landing_layer) = landed {
        land_on_layer(&mut session, &dungeon, &mut player, landing_layer);
    }
}

/// Reveal explored cells, pick up keys and items, activate signal entities,
/// and start stair transitions whenever the player's logical cell or facing
/// changes.
#[allow(clippy::too_many_arguments)]
pub fn on_player_moved(
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut players: Query<&mut Player>,
    mut transition: ResMut<Transition>,
    mut item_render: GroundItemRender,
    mut key_billboards: ResMut<KeyBillboards>,
    mut hud: ResMut<crate::hud::HudState>,
    mut signal: SignalRenderState,
    debug_flags: Res<crate::debug::DebugFlags>,
) {
    let Ok(mut player) = players.single_mut() else {
        return;
    };
    let state = player.grid_state();
    let pose = (state.col, state.row, state.facing);
    if pose == session.last_player_pose {
        return;
    }
    let (prev_col, prev_row, _) = session.last_player_pose;
    session.last_player_pose = pose;
    // TS gates its momentary-source deactivate/activate calls on an actual
    // cell change (`col !== prevCol || row !== prevRow`) because its onMove
    // callback only fires on real moves; this port's on_player_moved also
    // runs for pure turns (its pose includes facing), so the same guard is
    // needed here to avoid re-toggling on every turn-in-place.
    let moved = pose.0 != prev_col || pose.1 != prev_row;
    let level_id = session.current_level_id.clone();
    let level = find_level_by_id(&dungeon, &level_id);

    let Session {
        game,
        grid,
        walkable,
        environment,
        areas,
        ..
    } = &mut *session;
    let (col, row) = (i64::from(pose.0), i64::from(pose.1));

    if moved {
        game.deactivate_pressure_plate(i64::from(prev_col), i64::from(prev_row));
        game.deactivate_trigger(i64::from(prev_col), i64::from(prev_row));

        // Safety: if the player ended up on a closed door cell, force it
        // open and hand it to the blocked-door retry cycle.
        let key = delve_core::game_state::door_key(col, row);
        let render_key = layer_door_key(game.active_layer_index, &key);
        let closed_underfoot = game
            .active_layer()
            .doors
            .get(&key)
            .is_some_and(|door| door.state == DoorState::Closed);
        if closed_underfoot {
            let door_layer_index = game.active_layer_index;
            set_door_state(game, door_layer_index, &key, DoorState::Open);
            set_panel_open(
                &signal.door_panels,
                &mut signal.panel_query,
                &render_key,
                true,
            );
            signal.blocked_doors.by_key.insert(
                render_key,
                BlockedDoor {
                    col,
                    row,
                    layer_index: door_layer_index,
                    timer: DOOR_RETRY_INTERVAL,
                },
            );
        }
    }

    // Reveal fires on turns too (the TS onTurn callback also reveals);
    // pickups and activations are move-only.
    game.reveal_around(col, row, pose.2, grid);

    if moved {
        if let Some(key_id) = game.pickup_key_at(col, row) {
            info!("Picked up key: {key_id}");
            keys::hide_key_mesh(
                &mut key_billboards,
                &mut item_render.commands,
                &layer_door_key(
                    game.active_layer_index,
                    &delve_core::game_state::door_key(col, row),
                ),
            );
        }
        ground_items::handle_pickups(game, &mut item_render, &mut hud, col, row);

        let key = delve_core::game_state::door_key(col, row);
        let render_key = layer_door_key(game.active_layer_index, &key);
        game.activate_trigger(col, row);
        if game.activate_tripwire(col, row) {
            tripwires::hide_tripwire_mesh(
                &signal.tripwires,
                &mut item_render.commands,
                &render_key,
            );
            hud.show_message("Oops! A tripwire!");
        }
        let pressed = game.activate_pressure_plate(col, row).is_some()
            && game
                .active_layer()
                .plates
                .get(&key)
                .is_some_and(|plate| plate.activated);
        if pressed {
            let layer_y_offset = game.active_layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
            plates::press_plate(&mut signal.plate, &render_key, layer_y_offset);
        }

        // Torch fuel drains one unit per step, except in environments with
        // their own light (open sky, luminous mist).
        let cell_environment =
            crate::environment::resolve_environment_at_cell(col, row, *environment, areas);
        if delve_core::player_controller::should_drain_torch(cell_environment) {
            game.drain_torch_fuel(1.0);
        }

        // Ramp crossing — a same-scene, Y-shifted layer switch (no fade,
        // unlike stairs — this must never touch `transition.rs`), ported
        // from `main.ts:607-644`. Checked before the hole-detection below,
        // matching TS's own block order.
        if let Some(new_layer) =
            detect_ramp_crossing(game, level, prev_col, prev_row, pose.0, pose.1)
            && switch_active_layer(
                game,
                grid,
                walkable,
                &mut player,
                &dungeon,
                &level_id,
                new_layer,
            )
        {
            player.set_target_y_offset(new_layer as f32 * crate::dungeon::LAYER_HEIGHT);
        }

        // Hole detection — falling through an open floor or an already-open
        // pit trap, ported from `main.ts:646-681`. TS's equivalent check
        // lives inside the `onMove` callback, never `onTurn`, so this stays
        // inside the `moved` block too. TS gates this specific block on
        // `!ls.player.debugNoClip` (`main.ts:647`) — flying through a hole
        // shouldn't drop you. The *other* `setPendingFall` call, a pit trap
        // opening via signal while you stand on it (`main.ts:766-788`,
        // ported at this file's `set_pending_fall` call inside
        // `apply_world_events`), has no such gate in TS and stays that way
        // here too — not every fall trigger is guarded, only this one.
        if !debug_flags.fullbright
            && game.active_layer_index > 0
            && let Some(level) = level
        {
            let key = delve_core::game_state::door_key(col, row);
            let is_hole = is_hole_below(level, game.active_layer_index, col, row)
                || game
                    .active_layer()
                    .pit_traps
                    .get(&key)
                    .is_some_and(|pit| pit.state == PitTrapState::Open);
            if is_hole {
                let landing_layer = compute_landing_layer(level, game.active_layer_index, col, row);
                player.set_pending_fall(landing_layer);
            }
        }
    }

    let events = game.take_events();
    let fall_trigger = level.map(|level| FallTriggerContext {
        player: &mut player,
        level,
    });
    apply_world_events(events, game, (col, row), &mut signal, fall_trigger);

    // TS resolves the stepped-on stair instance back to its level entity and
    // transitions toward that entity's `target` — the id of the stair at the
    // OTHER end (`main.ts:684-694` feeding `transitionSystem.ts:47`'s
    // `stairEntity.target`). Passing the stair's own id instead would send
    // the player right back in front of the stair they just entered.
    if moved
        && let Some(stair) = game.get_stair(col, row)
        && let Some(stair_id) = &stair.id
        && let Some(level) = level
        && let Some(target) = stair_target(level, stair_id)
    {
        transition.begin_stair(target);
    }
}

/// `main.ts:688-689`: find the entity whose id matches the stair instance's
/// (no type filter, matching TS), then read where it leads. A stair entity
/// without a `target` triggers no transition — TS would start a fade that
/// finds no destination and lands the player back where they stood, so the
/// observable behavior matches.
fn stair_target(level: &DungeonLevel, stair_id: &str) -> Option<String> {
    delve_core::level_loader::get_all_level_entities(level)
        .find(|entity| entity.id.as_deref() == Some(stair_id))
        .and_then(|entity| entity.prop_str("target"))
        .map(str::to_string)
}

/// Bundled rendering handles for door, lever, plate, and tripwire visuals,
/// grouped into one `SystemParam` so systems that touch signal-entity
/// rendering stay under the argument-count lint.
#[derive(SystemParam)]
pub struct SignalRenderState<'w, 's> {
    pub door_panels: Res<'w, DoorPanels>,
    pub panel_query: Query<'w, 's, &'static mut DoorPanel>,
    pub lever: LeverRender<'w, 's>,
    pub plate: PlateRender<'w, 's>,
    pub tripwires: Res<'w, TripwireHandles>,
    pub blocked_doors: ResMut<'w, BlockedDoors>,
    pub chest_handles: Res<'w, ChestHandles>,
    pub chest_lids: Query<'w, 's, &'static mut ChestLid>,
    pub projectiles: ResMut<'w, crate::projectiles::ProjectileManagerRes>,
    pub pit_floor_handles: Res<'w, PitFloorHandles>,
    // A second `Commands` alongside whatever the caller's own bundled
    // render-state structs already carry (e.g. `GroundItemRender`'s) is
    // fine — unlike `Query`, multiple `Commands` params never conflict.
    // Toggling visibility this way (insert/overwrite the component) instead
    // of through a `Query<&mut Visibility>` sidesteps a real conflict:
    // `SconceRender.visibility` (already in `interact_input`'s param set)
    // is a completely unfiltered `Query<&mut Visibility>`, so a second one
    // here — filtered or not — would be flagged as ambiguous access.
    pub commands: Commands<'w, 's>,
}

const DOOR_RETRY_INTERVAL: f32 = 1.5;

/// Doors a signal tried to close while the cell was occupied. The close is
/// held off and retried on a timer (with a panel bounce) until the cell
/// clears — a signal-driven close can never land on a standing player or
/// enemy, ported from the TS `blockedDoors` map.
#[derive(Resource, Default)]
pub struct BlockedDoors {
    by_key: HashMap<String, BlockedDoor>,
}

impl BlockedDoors {
    pub fn clear(&mut self) {
        self.by_key.clear();
    }
}

struct BlockedDoor {
    col: i64,
    row: i64,
    /// The layer this door lives on, recorded at block time — same-level
    /// falling can change `active_layer_index` before the retry fires, so
    /// the close must write through this recorded layer, not whichever
    /// layer happens to be active later.
    layer_index: usize,
    timer: f32,
}

fn is_door_cell_occupied(game: &GameState, player_cell: (i64, i64), col: i64, row: i64) -> bool {
    player_cell == (col, row) || game.get_enemy(col, row).is_some()
}

fn bounce_panel(panels: &DoorPanels, panel_query: &mut Query<&mut DoorPanel>, key: &str) {
    if let Some(&entity) = panels.by_key.get(key)
        && let Ok(mut panel) = panel_query.get_mut(entity)
    {
        panel.bounce();
    }
}

fn set_door_state(game: &mut GameState, layer_index: usize, key: &str, state: DoorState) {
    if let Some(layer) = game.layer_mut(layer_index)
        && let Some(door) = layer.doors.get_mut(key)
    {
        door.state = state;
    }
}

/// Retry pending blocked-door closes: once the cell clears, the door
/// actually closes; while it stays occupied, the panel bounces and the
/// timer re-arms.
fn tick_blocked_doors(
    game: &mut GameState,
    player_cell: (i64, i64),
    signal: &mut SignalRenderState,
    delta: f32,
) {
    let mut close_now = Vec::new();
    let mut bounce_now = Vec::new();
    for (render_key, entry) in &mut signal.blocked_doors.by_key {
        entry.timer -= delta;
        if entry.timer > 0.0 {
            continue;
        }
        if is_door_cell_occupied(game, player_cell, entry.col, entry.row) {
            entry.timer = DOOR_RETRY_INTERVAL;
            bounce_now.push(render_key.clone());
        } else {
            close_now.push(render_key.clone());
        }
    }
    for render_key in close_now {
        let Some(entry) = signal.blocked_doors.by_key.remove(&render_key) else {
            continue;
        };
        let key = delve_core::game_state::door_key(entry.col, entry.row);
        set_door_state(game, entry.layer_index, &key, DoorState::Closed);
        set_panel_open(
            &signal.door_panels,
            &mut signal.panel_query,
            &render_key,
            false,
        );
    }
    for render_key in bounce_now {
        bounce_panel(&signal.door_panels, &mut signal.panel_query, &render_key);
    }
}

/// Advance the game state's timed signals each frame and apply the
/// resulting world events (timed doors, levers, and plates). Paused while
/// character creation or a level transition is active — the TS loop pauses
/// its tick while overlays are open, and pausing across the swap keeps
/// timed state out of the mid-transition window.
pub fn tick_game(
    time: Res<Time>,
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut players: Query<&mut Player>,
    gate: crate::overlay::InputGate,
    mut signal: SignalRenderState,
) {
    if gate.paused() {
        return;
    }
    let delta = time.delta_secs();
    let player_cell = (
        i64::from(session.last_player_pose.0),
        i64::from(session.last_player_pose.1),
    );
    session.game.tick_signals(f64::from(delta));
    let events = session.game.take_events();
    let level_id = session.current_level_id.clone();
    let level = find_level_by_id(&dungeon, &level_id);
    let mut player_query = players.single_mut();
    let Session { game, .. } = &mut *session;
    // A purely-timed pit trap opening under a motionless, non-interacting
    // player now also triggers a fall, once both the level and the single
    // `Player` entity resolve — see `FallTriggerContext`'s doc comment for
    // the shape this mirrors from `on_player_moved`. Either failing to
    // resolve (no player entity yet, or an unknown level) still runs the
    // floor-mesh toggle, just not the fall trigger.
    let fall_trigger = match (&level, player_query.as_mut()) {
        (Some(level), Ok(player)) => Some(FallTriggerContext { player, level }),
        _ => None,
    };
    apply_world_events(events, game, player_cell, &mut signal, fall_trigger);
    tick_blocked_doors(game, player_cell, &mut signal, delta);
}

fn set_panel_open(
    panels: &DoorPanels,
    panel_query: &mut Query<&mut DoorPanel>,
    key: &str,
    open: bool,
) {
    if let Some(&entity) = panels.by_key.get(key)
        && let Ok(mut panel) = panel_query.get_mut(entity)
    {
        panel.set_open(open);
    }
}

/// Apply drained world events to rendering: door panels, lever resets, and
/// plate resets. Pit traps and spawners land with their phases. Signal
/// closes on occupied door cells are held open and retried via
/// [`BlockedDoors`].
pub fn apply_world_events(
    events: Vec<WorldEvent>,
    game: &mut GameState,
    player_cell: (i64, i64),
    signal: &mut SignalRenderState,
    mut fall_trigger: Option<FallTriggerContext>,
) {
    // Events are always drained the same frame they're produced, and
    // gameplay only ever mutates `active_layer_mut()`, so `active_layer_index`
    // unambiguously names the layer every event's `(col, row)` belongs to.
    let active_layer_index = game.active_layer_index;
    let layer_y_offset = active_layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
    for event in events {
        match event {
            WorldEvent::DoorSignalChanged { col, row, open } => {
                let key = delve_core::game_state::door_key(col, row);
                let render_key = layer_door_key(active_layer_index, &key);
                if open {
                    signal.blocked_doors.by_key.remove(&render_key);
                    set_panel_open(
                        &signal.door_panels,
                        &mut signal.panel_query,
                        &render_key,
                        true,
                    );
                } else if is_door_cell_occupied(game, player_cell, col, row) {
                    set_door_state(game, active_layer_index, &key, DoorState::Open);
                    signal.blocked_doors.by_key.insert(
                        render_key.clone(),
                        BlockedDoor {
                            col,
                            row,
                            layer_index: active_layer_index,
                            timer: DOOR_RETRY_INTERVAL,
                        },
                    );
                    bounce_panel(&signal.door_panels, &mut signal.panel_query, &render_key);
                } else {
                    signal.blocked_doors.by_key.remove(&render_key);
                    set_panel_open(
                        &signal.door_panels,
                        &mut signal.panel_query,
                        &render_key,
                        false,
                    );
                }
            }
            WorldEvent::LeverReset { col, row } => {
                levers::set_lever_target(
                    &mut signal.lever,
                    &layer_door_key(
                        active_layer_index,
                        &delve_core::game_state::door_key(col, row),
                    ),
                    LeverState::Up,
                );
            }
            WorldEvent::PlateReset { col, row } => {
                plates::release_plate(
                    &mut signal.plate,
                    &layer_door_key(
                        active_layer_index,
                        &delve_core::game_state::door_key(col, row),
                    ),
                    layer_y_offset,
                );
            }
            WorldEvent::ChestSignalChanged { col, row, open } => {
                let render_key = layer_door_key(active_layer_index, &door_key(col, row));
                if open {
                    chests::open_chest_mesh(
                        &signal.chest_handles,
                        &mut signal.chest_lids,
                        &render_key,
                    );
                } else {
                    chests::close_chest_mesh(
                        &signal.chest_handles,
                        &mut signal.chest_lids,
                        &render_key,
                    );
                }
            }
            // Active-layer launcher fires surface through whichever session
            // drain runs first; consuming them here is the only reliable
            // path (the projectile tick's own loop covers non-active layers).
            WorldEvent::LauncherFire { col, row } => {
                crate::projectiles::fire_launcher_at(
                    game,
                    &mut signal.projectiles.0,
                    game.active_layer_index,
                    col,
                    row,
                );
            }
            // Floor-mesh toggle, ported from TS's `onPitTrapSignalChanged`
            // (`main.ts:734-789`). Two things that function also does are
            // still unported, both of which need cells this port does build:
            //
            // - The ceiling toggle two layers below (`main.ts:739-746`
            //   against TS's `pitCeilingMap`). An earlier note here claimed
            //   this port never spawns ceilings for non-topmost layers, so
            //   there was nothing to toggle. That was wrong:
            //   `dungeon.rs`'s `ceiling_enabled` is unconditionally true
            //   below the top layer — `is_top_layer` only decides whether
            //   the TOP layer may opt out — so those ceilings do exist and
            //   simply never toggle.
            // - The layer-below geometry rebuild (`main.ts:748-764`), which
            //   recomputes the force-renderable set from current pit state.
            //   Without it, `dungeon.rs`'s chamber-building only ever fires
            //   for pits authored open, never for one a signal opens.
            WorldEvent::PitTrapSignalChanged { col, row, open } => {
                let render_key = layer_door_key(active_layer_index, &door_key(col, row));
                if let Some(&entity) = signal.pit_floor_handles.by_key.get(&render_key) {
                    signal.commands.entity(entity).insert(if open {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    });
                }
                // A pit that just opened exactly under the player, while
                // they aren't already falling, starts a fall immediately —
                // ported from `onPitTrapSignalChanged`'s `if (open &&
                // !ls.player.falling)` branch. `None` callers (`tick_game`)
                // get the floor toggle above but skip this half — see
                // `FallTriggerContext`'s doc comment.
                if open
                    && player_cell == (col, row)
                    && let Some(ctx) = fall_trigger.as_mut()
                    && !ctx.player.falling()
                {
                    let landing_layer =
                        compute_landing_layer(ctx.level, active_layer_index, col, row);
                    if landing_layer < active_layer_index {
                        ctx.player.set_pending_fall(landing_layer);
                    }
                }
            }
            // TS's `onSpawnerSignalChanged`/`onBoulderSpawnerSignalChanged`/
            // `onBoulderSignalChanged` are themselves no-op stubs
            // (`main.ts:795-802`): the `active`/state flip these events
            // announce already landed in `GameState` before the event fired
            // (`spawner.active = active`, etc.), and the resulting behavior
            // change (a spawner firing again, a boulder starting to roll)
            // surfaces through the next `tick_spawners`/`tick_boulders` call
            // instead — there's no separate visual for the signal itself to
            // drive here either, matching TS exactly rather than an
            // oversight.
            WorldEvent::SpawnerSignalChanged { .. }
            | WorldEvent::BoulderSpawnerSignalChanged { .. }
            | WorldEvent::BoulderSignalChanged { .. } => {}
        }
    }
}

/// Rendering and reward handles `interact_input` needs beyond signal-entity
/// visuals: loot spawning for opened chests, block-push animation, and the
/// HUD toast that stands in for TS's dedicated sign/message overlays until
/// those land (see the module doc comment on this file's `hud` usage).
#[derive(SystemParam)]
pub struct InteractEffects<'w, 's> {
    pub item_render: GroundItemRender<'w, 's>,
    pub loot_tables: Res<'w, LootTablesRes>,
    pub rng: ResMut<'w, GameRng>,
    pub blocks: BlockRender<'w, 's>,
    pub hud: ResMut<'w, crate::hud::HudState>,
    // `ActiveOverlay` lives here (not in a shared `InputGate`) because the
    // `NpcInteracted` arm below needs `ResMut` access to open the dialog
    // overlay — `InputGate` only offers `Res`, and borrowing both in the
    // same system would conflict, the same reason `save_load_overlay.rs`'s
    // `check_player_death` takes `ActiveOverlay` directly.
    pub overlay: ResMut<'w, crate::overlay::ActiveOverlay>,
    pub npc_db: Res<'w, crate::npcs::NpcDb>,
    pub dialog_cache: ResMut<'w, crate::dialog_overlay::DialogTreeCache>,
    pub dialog_state: ResMut<'w, crate::dialog_overlay::DialogOverlayState>,
    pub quests: ResMut<'w, crate::dialog_overlay::QuestManagerRes>,
    pub trading_state: ResMut<'w, crate::trading_overlay::TradingOverlayState>,
    pub sign_state: ResMut<'w, crate::sign_overlay::SignOverlayState>,
    pub fountain_handles: Res<'w, FountainHandles>,
    pub altar_handles: Res<'w, AltarHandles>,
    pub dungeon: Res<'w, DungeonRes>,
    // Read-only, and filtered `Without<PlateVisual>` so Bevy can prove this
    // is disjoint from `signal.plate.visuals`'s `Query<&mut
    // MeshMaterial3d<StandardMaterial>, With<PlateVisual>>` — an
    // unfiltered query here would conflict with it despite the two never
    // targeting the same entity, the same class of issue documented on
    // `enemy_feedback::CombatFeedback::bar_fills`. The emissive mutation
    // itself goes through `item_render.materials`, already present, rather
    // than a second `ResMut<Assets<StandardMaterial>>` field (which would
    // conflict on its own).
    pub pillar_materials: Query<
        'w,
        's,
        &'static MeshMaterial3d<StandardMaterial>,
        Without<crate::plates::PlateVisual>,
    >,
}

#[allow(clippy::too_many_arguments)]
pub fn interact_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut session: ResMut<Session>,
    transition: Res<Transition>,
    mut players: Query<&mut Player>,
    mut signal: SignalRenderState,
    mut sconce_render: SconceRender,
    mut effects: InteractEffects,
) {
    if transition.is_active() || effects.overlay.is_open() || !keys.just_pressed(KeyCode::Space) {
        return;
    }
    let Ok(mut player) = players.single_mut() else {
        return;
    };
    let level = find_level_by_id(&effects.dungeon, &session.current_level_id);
    let player_state = player.grid_state();
    let facing_cell = delve_core::grid::get_facing_cell(player_state);
    let facing_key =
        delve_core::game_state::door_key(i64::from(facing_cell.0), i64::from(facing_cell.1));
    // The facing cell is always on the player's own (active) layer —
    // interaction only ever targets what the player is standing in front
    // of.
    let facing_render_key = layer_door_key(session.game.active_layer_index, &facing_key);

    let result = {
        let Session { game, grid, .. } = &mut *session;
        interact(player_state, grid, game)
    };

    match result.result_type {
        InteractionType::DoorOpened => {
            set_panel_open(
                &signal.door_panels,
                &mut signal.panel_query,
                &facing_render_key,
                true,
            );
        }
        InteractionType::DoorClosed => {
            set_panel_open(
                &signal.door_panels,
                &mut signal.panel_query,
                &facing_render_key,
                false,
            );
        }
        InteractionType::DoorBlocked => {
            if let Some(&entity) = signal.door_panels.by_key.get(&facing_render_key)
                && let Ok(mut panel) = signal.panel_query.get_mut(entity)
            {
                panel.bounce();
            }
        }
        InteractionType::SconceTaken => {
            sconces::extinguish_sconce(
                &mut sconce_render,
                &layer_door_key(
                    session.game.active_layer_index,
                    &delve_core::game_state::door_key(
                        i64::from(player_state.col),
                        i64::from(player_state.row),
                    ),
                ),
            );
            // `inputSystem.ts:170-173` pairs extinguishSconce with an
            // immediate setSources re-collect; the deferred rebuild sees the
            // zeroed light intensity because extinguish_sconce mutates it
            // synchronously and init_embers runs later in PostUpdate.
            commands.insert_resource(crate::particles::EmbersPending);
        }
        InteractionType::LeverActivated => {
            for target in result.targets.iter().flatten() {
                let Some(position) = session.game.resolve_entity_position(target) else {
                    continue;
                };
                let (col, row, target_layer_index) =
                    (position.col, position.row, position.layer_index);
                let open = session.game.is_door_open(col, row);
                set_panel_open(
                    &signal.door_panels,
                    &mut signal.panel_query,
                    &layer_door_key(
                        target_layer_index,
                        &delve_core::game_state::door_key(col, row),
                    ),
                    open,
                );
            }
            let (lever_col, lever_row) = (i64::from(player_state.col), i64::from(player_state.row));
            if let Some(state) = session
                .game
                .get_lever(lever_col, lever_row)
                .map(|lever| lever.state)
            {
                levers::set_lever_target(
                    &mut signal.lever,
                    &layer_door_key(
                        session.game.active_layer_index,
                        &delve_core::game_state::door_key(lever_col, lever_row),
                    ),
                    state,
                );
            }
        }
        InteractionType::BlockPushed => {
            if let (Some(to_col), Some(to_row)) = (result.target_col, result.target_row) {
                let to_render_key =
                    layer_door_key(session.game.active_layer_index, &door_key(to_col, to_row));
                blocks::animate_block_push(
                    &mut effects.blocks,
                    &facing_render_key,
                    to_render_key.clone(),
                    to_col,
                    to_row,
                );
                // `inputSystem.ts:180-188`: press the destination plate the
                // push just activated, unconditionally deactivate the source
                // plate, release its visual if it exists and is no longer
                // active.
                let layer_y_offset =
                    session.game.active_layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
                let dest_activated = session
                    .game
                    .active_layer()
                    .plates
                    .get(&door_key(to_col, to_row))
                    .is_some_and(|plate| plate.activated);
                if dest_activated {
                    plates::press_plate(&mut signal.plate, &to_render_key, layer_y_offset);
                }
                let (facing_col, facing_row) = (i64::from(facing_cell.0), i64::from(facing_cell.1));
                session
                    .game
                    .deactivate_pressure_plate(facing_col, facing_row);
                let origin_released = session
                    .game
                    .active_layer()
                    .plates
                    .get(&facing_key)
                    .is_some_and(|plate| !plate.activated);
                if origin_released {
                    plates::release_plate(&mut signal.plate, &facing_render_key, layer_y_offset);
                }
            }
        }
        InteractionType::ChestOpened => {
            if let (Some(col), Some(row)) = (result.target_col, result.target_row) {
                chests::open_chest_mesh(
                    &signal.chest_handles,
                    &mut signal.chest_lids,
                    &facing_render_key,
                );
                let drops = session
                    .game
                    .get_chest(col, row)
                    .and_then(|chest| chest.drops.clone());
                let rng = &mut effects.rng.0;
                let mut random = || rng.next_f64();
                let chest_layer_index = session.game.active_layer_index;
                ground_items::spawn_loot(
                    &mut session.game,
                    &mut effects.item_render,
                    &effects.loot_tables.0,
                    "",
                    drops.as_ref(),
                    (col, row, chest_layer_index),
                    &mut random,
                );
            }
        }
        InteractionType::NpcInteracted => {
            // `result.message` carries the NPC *definition* id (see
            // `interaction.rs::interact`'s NPC branch) — matches TS's
            // `inputSystem.ts:218-237`, which only proceeds `if (npcDef)`
            // and otherwise silently does nothing.
            if let Some(npc_id) = &result.message
                && let Some(npc_def) = effects.npc_db.0.get_npc(npc_id).cloned()
            {
                crate::dialog_overlay::open_dialog_for_npc(
                    npc_id,
                    &npc_def,
                    &mut effects.dialog_cache,
                    &mut session.game,
                    &mut effects.dialog_state,
                    &mut effects.overlay,
                    &mut effects.quests.0,
                    &mut effects.hud,
                    &effects.npc_db.0,
                    &mut effects.trading_state,
                );
            }
        }
        InteractionType::FountainUsed => {
            if let (Some(col), Some(row)) = (result.target_col, result.target_row) {
                fountains::mark_fountain_used(
                    &effects.fountain_handles,
                    &mut sconce_render.visibility,
                    &layer_door_key(session.game.active_layer_index, &door_key(col, row)),
                );
            }
        }
        InteractionType::AltarActivated => {
            if let (Some(col), Some(row)) = (result.target_col, result.target_row) {
                altars::mark_altar_used(
                    &effects.altar_handles,
                    &effects.pillar_materials,
                    &mut effects.item_render.materials,
                    &layer_door_key(session.game.active_layer_index, &door_key(col, row)),
                );
            }
        }
        // Signs and bookshelves share one text popup — TS routes both
        // `sign_read` and `bookshelf_read` to the same `signOverlay.show`
        // (`inputSystem.ts:200-205`); neither has any mesh state to update.
        InteractionType::SignRead | InteractionType::BookshelfRead => {
            if let Some(message) = &result.message {
                crate::sign_overlay::open_sign(
                    &mut effects.sign_state,
                    &mut effects.overlay,
                    message,
                );
            }
        }
        _ => {}
    }
    // Every interaction message gets a HUD toast in addition to the log
    // line. Excluded: `NpcInteracted`, whose `message` carries the NPC
    // definition id (an internal lookup key, not user-facing text — see the
    // match arm above; TS's own dispatcher has no `npc_interacted` case in
    // this generic-toast position either), and `SignRead`/`BookshelfRead`,
    // which show the sign popup with no accompanying toast — TS's
    // dispatcher has no `hud.showMessage` for either result type.
    if result.result_type != InteractionType::NpcInteracted
        && result.result_type != InteractionType::SignRead
        && result.result_type != InteractionType::BookshelfRead
        && let Some(message) = &result.message
    {
        info!("{message}");
        effects.hud.show_message(message);
    }

    let player_cell = (i64::from(player_state.col), i64::from(player_state.row));
    let events = session.game.take_events();
    let fall_trigger = level.map(|level| FallTriggerContext {
        player: &mut player,
        level,
    });
    apply_world_events(
        events,
        &mut session.game,
        player_cell,
        &mut signal,
        fall_trigger,
    );
}

/// Dispatches one `InventoryAction` through `process_inventory_action`,
/// showing `Message` actions as a HUD toast (matching TS's inventory
/// overlay's own switch on the action's `type`, which never reaches
/// `processInventoryAction` for that variant) and respawning a dropped
/// item's world billboard on success. Shared by the full inventory overlay,
/// the mini panel's mouse interactions, and quick-slot digit keys.
///
/// `on_drop` only *records* the drop (it can't call back into `game`/
/// `item_render` itself — `process_inventory_action` already holds `game`
/// exclusively for the whole call, and the closure has no independent path
/// to either); the actual billboard respawn happens afterward, once that
/// borrow has ended, by re-reading the now-dropped item straight from the
/// registry.
pub(crate) fn apply_inventory_action(
    game: &mut GameState,
    item_render: &mut GroundItemRender,
    hud: &mut crate::hud::HudState,
    action: &InventoryAction,
) {
    if let InventoryAction::Message { text } = action {
        hud.show_message(text);
        return;
    }

    let mut dropped: Option<(String, i64, i64)> = None;
    {
        let mut on_drop = |instance_id: &str, col: i64, row: i64| {
            dropped = Some((instance_id.to_string(), col, row));
        };
        process_inventory_action(action, game, &mut on_drop);
    }
    if let Some((instance_id, ..)) = dropped
        && let Some(entity) = game.entity_registry.get_item(&instance_id).cloned()
        && let Some(def) = item_render.items.0.get_item(&entity.item_id)
    {
        let kind = ground_items::ItemKind::of(def.item_type);
        ground_items::add_single_item_mesh(item_render, kind, &entity, game.active_layer_index);
    }
}

/// `Digit1`-`Digit8` use the consumable in that backpack slot directly —
/// ported from `inputSystem.ts`'s quick-slot case, which calls
/// `useConsumableFromRegistry` straight from the keyboard handler rather
/// than going through `processInventoryAction`'s `Use` arm (that arm exists
/// for the inventory overlay/mini panel's own Enter/double-click actions,
/// which resolve to a *sorted-list* position; quick slots key directly off
/// the physical backpack slot number instead, matching TS's
/// `getBackpackItemAt(slotNum)` call).
pub fn quick_slot_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    gate: crate::overlay::InputGate,
) {
    if gate.blocked() {
        return;
    }
    const DIGIT_SLOTS: [(KeyCode, u32); 8] = [
        (KeyCode::Digit1, 0),
        (KeyCode::Digit2, 1),
        (KeyCode::Digit3, 2),
        (KeyCode::Digit4, 3),
        (KeyCode::Digit5, 4),
        (KeyCode::Digit6, 5),
        (KeyCode::Digit7, 6),
        (KeyCode::Digit8, 7),
    ];
    for (key, slot) in DIGIT_SLOTS {
        if !keys.just_pressed(key) {
            continue;
        }
        let instance_id = session
            .game
            .entity_registry
            .backpack_item_at(slot)
            .map(|entity| entity.instance_id.clone());
        if let Some(instance_id) = instance_id {
            session.game.use_consumable_from_registry(&instance_id);
        }
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use delve_core::level_loader::{ValidationContext, validate_dungeon_str};

    fn ruins_level(level_id: &str) -> DungeonLevel {
        let path = crate::assets_dir().join("levels/ruins.json");
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let mut warnings = Vec::new();
        let dungeon = validate_dungeon_str(
            &json,
            "ruins.json",
            &ValidationContext::default(),
            &mut warnings,
        )
        .expect("shipped ruins.json validates");
        dungeon
            .levels
            .into_iter()
            .find(|level| level.id.as_deref() == Some(level_id))
            .unwrap_or_else(|| panic!("ruins.json has {level_id}"))
    }

    /// Stepping onto ruins' `stairs_1` must transition toward `stairs_2` on
    /// the other level — the entity's `target` — not back toward `stairs_1`
    /// itself, which would respawn the player in front of the stair they
    /// just entered and read as "stairs don't go anywhere."
    #[test]
    fn stair_target_resolves_to_the_other_end_not_the_stepped_on_stair() {
        let forest = ruins_level("level_forest");
        assert_eq!(stair_target(&forest, "stairs_1"), Some("stairs_2".into()));
        assert_eq!(stair_target(&forest, "stairs_3"), Some("stairs_4".into()));
    }

    #[test]
    fn stair_target_is_none_for_an_unknown_entity_id() {
        let forest = ruins_level("level_forest");
        assert_eq!(stair_target(&forest, "no_such_stair"), None);
    }
}

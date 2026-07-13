//! Boulder state machine, ported from the TS `boulderSystem`: rolling,
//! falling through holes, descending ramps, chain-pushing other boulders,
//! and crashing chests. Pure logic — grid/area data lives outside
//! `GameState` (in the loaded level), so it's injected via
//! [`BoulderContext`], the same shape `enemy_ai::EnemyUpdateContext` and
//! `spawners::SpawnerContext` use. State transitions are returned as
//! [`BoulderEvent`]s instead of driving a Bevy animator directly.

use crate::game_state::{
    BoulderInstance, BoulderState, GameState, IntervalMode, PitTrapState, door_key, layer_door_key,
};
use crate::grid::{Facing, cell_at};
use crate::loot::DropsOverride;
use crate::player_controller::{PLAYER_DAMAGE_FLASH_DURATION, PlayerTickState};
use crate::types::{CharDef, EnemyInstance, LayerDef, TextureArea};
use std::collections::HashSet;

/// Read-only per-tick inputs the shell must supply.
pub struct BoulderContext<'a> {
    pub layer_defs: &'a [LayerDef],
    /// Level-wide area fallback, used when a layer has no `areas` of its own.
    pub level_areas: &'a [TextureArea],
    pub char_defs: &'a [CharDef],
    pub walkable: &'a HashSet<char>,
    /// `GameState::active_layer_index` before this tick — the layer the
    /// player currently occupies.
    pub player_layer: usize,
    pub player_col: i64,
    pub player_row: i64,
    pub debug_fullbright: bool,
    /// Whether the boulder keyed `layer_door_key(layer_index, cell_key)` is
    /// currently mid-tween in the shell's animator. A boulder only advances
    /// once this returns `true` for its *current* key — call it with the
    /// boulder's key from *before* any transition this tick, exactly
    /// mirroring the TS check (`boulderAnimator.getMode(prefKey) !== 'rest'`
    /// gates re-entry). If the shell reports "resting" even one frame
    /// early or late relative to the animator's actual tween completion,
    /// a boulder will either skip a state (visually teleporting) or stall
    /// (never advancing) — this is the tightest core/shell coupling point
    /// in the whole module.
    pub is_resting: &'a dyn Fn(&str) -> bool,
}

fn layer_grid<'a>(context: &'a BoulderContext, layer_index: usize) -> Option<&'a Vec<String>> {
    context.layer_defs.get(layer_index).map(|def| &def.grid)
}

fn resolve_areas<'a>(context: &'a BoulderContext, layer_index: usize) -> &'a [TextureArea] {
    context
        .layer_defs
        .get(layer_index)
        .and_then(|def| def.areas.as_deref())
        .unwrap_or(context.level_areas)
}

fn is_solid_wall(character: char, char_defs: &[CharDef]) -> bool {
    character == '#'
        || char_defs
            .iter()
            .find(|def| def.character == character)
            .is_some_and(|def| def.solid && def.see_through != Some(true))
}

/// A cell is a hole when the layer below is missing or non-solid, subject
/// to a `TextureArea.openBottom` override (last-matching-area wins) and an
/// open pit trap at the cell (both override to a hole), and finally
/// un-overridden back to "not a hole" if the layer below already has a
/// boulder or block plugging it.
///
/// This intentionally does not share code with `spawners::is_traversal_hole`
/// — see that function's doc comment for why porting both separately is the
/// faithful choice here.
fn is_hole_at(
    game: &mut GameState,
    context: &BoulderContext,
    col: i64,
    row: i64,
    layer_index: usize,
) -> bool {
    if layer_index == 0 {
        return false;
    }

    let mut hole = layer_grid(context, layer_index - 1).is_none_or(|below_grid| {
        let (Ok(c), Ok(r)) = (i32::try_from(col), i32::try_from(row)) else {
            return true;
        };
        cell_at(below_grid, c, r)
            .is_none_or(|character| !is_solid_wall(character, context.char_defs))
    });

    for area in resolve_areas(context, layer_index) {
        if col >= i64::from(area.from_col)
            && col <= i64::from(area.to_col)
            && row >= i64::from(area.from_row)
            && row <= i64::from(area.to_row)
            && let Some(open_bottom) = area.open_bottom
        {
            hole = open_bottom;
        }
    }

    let saved = game.active_layer_index;
    game.active_layer_index = layer_index;
    let pit_open = game
        .active_layer()
        .pit_traps
        .get(&door_key(col, row))
        .is_some_and(|pit| pit.state == PitTrapState::Open);
    game.active_layer_index = saved;
    if pit_open {
        hole = true;
    }

    if hole {
        let saved = game.active_layer_index;
        game.active_layer_index = layer_index - 1;
        if game
            .active_layer()
            .boulders
            .contains_key(&door_key(col, row))
            || game.is_block_at(col, row)
        {
            hole = false;
        }
        game.active_layer_index = saved;
    }

    hole
}

fn compute_landing_layer(
    game: &mut GameState,
    context: &BoulderContext,
    col: i64,
    row: i64,
    from_layer: usize,
) -> usize {
    for layer_index in (1..from_layer).rev() {
        if !is_hole_at(game, context, col, row, layer_index) {
            return layer_index;
        }
    }
    0
}

/// TS's `canBoulderRollTo` (`main.ts:826-838`): a narrower upfront
/// validity check than [`can_boulder_enter`]'s own per-tick resolution
/// below — that one also classifies what happens if an enemy or the player
/// is standing in the destination cell (`EnterResult::KillEnemy` /
/// `DamageEnemy` / `DamagePlayer`), outcomes `canBoulderRollTo` has no
/// equivalent for. Kept as its own function rather than reusing
/// `can_boulder_enter` for that reason, not just to avoid a shared-context
/// dependency.
///
/// Used by the move-blocked push handler to decide, before committing to a
/// block push, whether a boulder sitting one cell beyond the block can
/// actually roll into the cell beyond *that*. The direct "walk straight
/// into a boulder" push has no equivalent upfront check at all
/// (`main.ts:944-949`) — [`tick_boulders`] re-validates every non-idle
/// boulder on its next tick regardless of how it became non-idle, so an
/// invalid direct push just bounces or idles the same way any other
/// blocked roll does.
#[must_use]
pub fn can_boulder_roll_to(
    game: &GameState,
    grid: &[String],
    walkable: &HashSet<char>,
    from_col: i64,
    from_row: i64,
    to_col: i64,
    to_row: i64,
) -> bool {
    let (Ok(col), Ok(row)) = (i32::try_from(to_col), i32::try_from(to_row)) else {
        return false;
    };
    let Some(character) = cell_at(grid, col, row) else {
        return false;
    };
    if !walkable.contains(&character) {
        return false;
    }
    if game.get_door(to_col, to_row).is_some() && !game.is_door_open(to_col, to_row) {
        return false;
    }
    if game.is_block_at(to_col, to_row) || game.is_boulder_at(to_col, to_row) {
        return false;
    }
    if game.is_edge_blocked(from_col, from_row, to_col, to_row) {
        return false;
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnterResult {
    Enter,
    KillEnemy,
    DamageEnemy,
    DamagePlayer,
    Blocked,
}

/// Classifies what happens if a boulder steps into `(nc, nr)` on
/// `layer_index`. Assumes `game.active_layer_index == layer_index` already
/// (the caller's loop sets this once per layer, matching TS).
fn can_boulder_enter(
    game: &GameState,
    context: &BoulderContext,
    layer_index: usize,
    nc: i64,
    nr: i64,
    boulder: &BoulderInstance,
) -> EnterResult {
    let Some(grid) = layer_grid(context, layer_index) else {
        return EnterResult::Blocked;
    };
    let (Ok(c), Ok(r)) = (i32::try_from(nc), i32::try_from(nr)) else {
        return EnterResult::Blocked;
    };
    let Some(character) = cell_at(grid, c, r) else {
        return EnterResult::Blocked;
    };
    if !context.walkable.contains(&character) {
        return EnterResult::Blocked;
    }
    if game.get_door(nc, nr).is_some() && !game.is_door_open(nc, nr) {
        return EnterResult::Blocked;
    }
    if game.is_block_at(nc, nr) || game.is_boulder_at(nc, nr) {
        return EnterResult::Blocked;
    }
    if game.active_layer().enemies.contains_key(&door_key(nc, nr)) {
        return if boulder.insta_kill_enemies {
            EnterResult::KillEnemy
        } else {
            EnterResult::DamageEnemy
        };
    }
    if layer_index == context.player_layer && context.player_col == nc && context.player_row == nr {
        return EnterResult::DamagePlayer;
    }
    EnterResult::Enter
}

/// A ramp on the layer below whose top cell is `(col, row)` and whose "up"
/// direction is opposite `direction` lets the boulder descend one layer.
fn check_ramp_descent(
    game: &mut GameState,
    col: i64,
    row: i64,
    layer_index: usize,
    direction: Facing,
) -> Option<(i64, i64)> {
    if layer_index == 0 {
        return None;
    }
    let saved = game.active_layer_index;
    game.active_layer_index = layer_index - 1;
    let (bdc, bdr) = direction.delta();
    let result = game.active_layer().ramps.values().find_map(|ramp| {
        let (rdc, rdr) = ramp.facing.delta();
        let top_col = ramp.col + i64::from(rdc);
        let top_row = ramp.row + i64::from(rdr);
        (top_col == col
            && top_row == row
            && i64::from(bdc) == -i64::from(rdc)
            && i64::from(bdr) == -i64::from(rdr))
        .then_some((ramp.col, ramp.row))
    });
    game.active_layer_index = saved;
    result
}

fn deactivate_boulder_triggers(game: &mut GameState, col: i64, row: i64) {
    game.deactivate_pressure_plate(col, row);
    game.deactivate_trigger(col, row);
}

fn activate_boulder_triggers(game: &mut GameState, col: i64, row: i64) -> (bool, bool) {
    game.activate_trigger(col, row);
    let tripwire_activated = game.activate_tripwire(col, row);
    let plate_targets = game.activate_pressure_plate(col, row);
    let plate_activated = plate_targets.is_some()
        && game
            .active_layer()
            .plates
            .get(&door_key(col, row))
            .is_some_and(|plate| plate.activated);
    (tripwire_activated, plate_activated)
}

fn crash_chest_if_any(
    game: &mut GameState,
    col: i64,
    row: i64,
    layer_index: usize,
) -> Option<BoulderEvent> {
    let drops = game.destroy_chest(col, row)?;
    Some(BoulderEvent::ChestCrashed {
        col,
        row,
        layer_index,
        drops,
    })
}

struct MoveOutcome {
    new_key: String,
    tripwire_activated: bool,
    plate_activated: bool,
    chest_crashed: Option<BoulderEvent>,
}

/// Removes the boulder at `old_key` on `old_layer`, re-inserts it at
/// `(new_col, new_row)` on `new_layer`, and fires the trigger/chest side
/// effects of leaving and entering those cells.
fn transfer_boulder_to_layer(
    game: &mut GameState,
    old_key: &str,
    old_layer: usize,
    new_col: i64,
    new_row: i64,
    new_layer: usize,
) -> Option<MoveOutcome> {
    game.active_layer_index = old_layer;
    let mut boulder = game.active_layer_mut().boulders.remove(old_key)?;
    let (old_col, old_row) = (boulder.col, boulder.row);
    deactivate_boulder_triggers(game, old_col, old_row);

    boulder.col = new_col;
    boulder.row = new_row;
    game.active_layer_index = new_layer;
    let new_key = door_key(new_col, new_row);
    game.active_layer_mut()
        .boulders
        .insert(new_key.clone(), boulder);

    let chest_crashed = crash_chest_if_any(game, new_col, new_row, new_layer);
    let (tripwire_activated, plate_activated) = activate_boulder_triggers(game, new_col, new_row);

    Some(MoveOutcome {
        new_key,
        tripwire_activated,
        plate_activated,
        chest_crashed,
    })
}

/// Same as [`transfer_boulder_to_layer`] but without a layer switch;
/// `game.active_layer_index` must already equal `layer_index`.
fn move_boulder_same_layer(
    game: &mut GameState,
    old_key: &str,
    layer_index: usize,
    new_col: i64,
    new_row: i64,
) -> Option<MoveOutcome> {
    let mut boulder = game.active_layer_mut().boulders.remove(old_key)?;
    let (old_col, old_row) = (boulder.col, boulder.row);
    deactivate_boulder_triggers(game, old_col, old_row);

    boulder.col = new_col;
    boulder.row = new_row;
    let new_key = door_key(new_col, new_row);
    game.active_layer_mut()
        .boulders
        .insert(new_key.clone(), boulder);

    let chest_crashed = crash_chest_if_any(game, new_col, new_row, layer_index);
    let (tripwire_activated, plate_activated) = activate_boulder_triggers(game, new_col, new_row);

    Some(MoveOutcome {
        new_key,
        tripwire_activated,
        plate_activated,
        chest_crashed,
    })
}

/// Raw HP subtraction matching the TS `damageEnemyByBoulder` exactly — this
/// deliberately does not call `GameState::damage_enemy`, which also resets
/// the enemy's regen-pause timer; TS's boulder damage never touches that.
fn damage_enemy_by_boulder(
    game: &mut GameState,
    col: i64,
    row: i64,
    damage: f64,
    layer_index: usize,
) -> Option<BoulderEvent> {
    let key = door_key(col, row);
    let enemy = game.active_layer_mut().enemies.get_mut(&key)?;
    enemy.hp -= damage;
    let killed = enemy.hp <= 0.0;
    let enemy_snapshot = enemy.clone();
    if killed {
        game.active_layer_mut().enemies.remove(&key);
    }
    Some(BoulderEvent::EnemyDamaged {
        key,
        col,
        row,
        damage,
        killed,
        enemy: enemy_snapshot,
        layer_index,
    })
}

fn kill_enemy_at(
    game: &mut GameState,
    col: i64,
    row: i64,
    layer_index: usize,
) -> Option<BoulderEvent> {
    let key = door_key(col, row);
    let enemy = game.active_layer_mut().enemies.remove(&key)?;
    Some(BoulderEvent::EnemyInstaKilled {
        key,
        col,
        row,
        enemy,
        layer_index,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoulderTransitionKind {
    Rolled,
    Fell,
    Descended,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoulderMoved {
    pub kind: BoulderTransitionKind,
    pub old_key: String,
    pub old_layer_index: usize,
    pub new_key: String,
    pub new_layer_index: usize,
    pub col: i64,
    pub row: i64,
    pub direction: Facing,
    pub tripwire_activated: bool,
    pub plate_activated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoulderEvent {
    Moved(BoulderMoved),
    Spawned {
        key: String,
        layer_index: usize,
        col: i64,
        row: i64,
        direction: Facing,
    },
    ChestCrashed {
        col: i64,
        row: i64,
        layer_index: usize,
        drops: Option<DropsOverride>,
    },
    EnemyDamaged {
        key: String,
        col: i64,
        row: i64,
        damage: f64,
        killed: bool,
        enemy: EnemyInstance,
        layer_index: usize,
    },
    EnemyInstaKilled {
        key: String,
        col: i64,
        row: i64,
        enemy: EnemyInstance,
        layer_index: usize,
    },
}

#[allow(clippy::too_many_arguments)]
fn push_moved(
    events: &mut Vec<BoulderEvent>,
    kind: BoulderTransitionKind,
    old_key: String,
    old_layer_index: usize,
    new_layer_index: usize,
    outcome: MoveOutcome,
    col: i64,
    row: i64,
    direction: Facing,
) {
    if let Some(event) = outcome.chest_crashed {
        events.push(event);
    }
    events.push(BoulderEvent::Moved(BoulderMoved {
        kind,
        old_key,
        old_layer_index,
        new_key: outcome.new_key,
        new_layer_index,
        col,
        row,
        direction,
        tripwire_activated: outcome.tripwire_activated,
        plate_activated: outcome.plate_activated,
    }));
}

#[allow(clippy::too_many_lines)]
fn decide_next(
    game: &mut GameState,
    context: &BoulderContext,
    tick_state: &mut PlayerTickState,
    events: &mut Vec<BoulderEvent>,
    key: &str,
    layer_index: usize,
) {
    let Some(boulder) = game.active_layer().boulders.get(key).cloned() else {
        return;
    };
    let just_landed = boulder.state == BoulderState::Falling;

    if just_landed {
        if layer_index == context.player_layer
            && context.player_col == boulder.col
            && context.player_row == boulder.row
        {
            let damage = if context.debug_fullbright {
                0.0
            } else {
                boulder.fall_damage
            };
            game.player.hp = (game.player.hp - damage).max(0.0);
            tick_state.player_damage_flash_timer = PLAYER_DAMAGE_FLASH_DURATION;
        }
        if boulder.insta_kill_enemies {
            if let Some(event) = kill_enemy_at(game, boulder.col, boulder.row, layer_index) {
                events.push(event);
            }
        } else if let Some(event) = damage_enemy_by_boulder(
            game,
            boulder.col,
            boulder.row,
            boulder.fall_damage,
            layer_index,
        ) {
            events.push(event);
        }
        if let Some(entry) = game.active_layer_mut().boulders.get_mut(key) {
            entry.state = BoulderState::Rolling;
        }
    }

    if is_hole_at(game, context, boulder.col, boulder.row, layer_index) {
        let landing_layer =
            compute_landing_layer(game, context, boulder.col, boulder.row, layer_index);
        let Some(outcome) = transfer_boulder_to_layer(
            game,
            key,
            layer_index,
            boulder.col,
            boulder.row,
            landing_layer,
        ) else {
            return;
        };
        if let Some(entry) = game.active_layer_mut().boulders.get_mut(&outcome.new_key) {
            entry.state = BoulderState::Falling;
        }
        if let Some(event) = outcome.chest_crashed {
            events.push(event);
        }
        events.push(BoulderEvent::Moved(BoulderMoved {
            kind: BoulderTransitionKind::Fell,
            old_key: key.to_string(),
            old_layer_index: layer_index,
            new_key: outcome.new_key,
            new_layer_index: landing_layer,
            col: boulder.col,
            row: boulder.row,
            direction: boulder.direction,
            tripwire_activated: outcome.tripwire_activated,
            plate_activated: outcome.plate_activated,
        }));
        return;
    }

    if let Some((bottom_col, bottom_row)) = check_ramp_descent(
        game,
        boulder.col,
        boulder.row,
        layer_index,
        boulder.direction,
    ) {
        let new_layer = layer_index - 1;
        let Some(outcome) =
            transfer_boulder_to_layer(game, key, layer_index, bottom_col, bottom_row, new_layer)
        else {
            return;
        };
        push_moved(
            events,
            BoulderTransitionKind::Descended,
            key.to_string(),
            layer_index,
            new_layer,
            outcome,
            bottom_col,
            bottom_row,
            boulder.direction,
        );
        return;
    }

    let (dcol, drow) = boulder.direction.delta();
    let (nc, nr) = (boulder.col + i64::from(dcol), boulder.row + i64::from(drow));

    if just_landed
        && can_boulder_enter(game, context, layer_index, nc, nr, &boulder) == EnterResult::Blocked
    {
        let blocker_key = door_key(nc, nr);
        if let Some(blocker) = game.active_layer().boulders.get(&blocker_key).cloned() {
            let beyond = can_boulder_enter(
                game,
                context,
                layer_index,
                nc + i64::from(dcol),
                nr + i64::from(drow),
                &blocker,
            );
            if beyond != EnterResult::Blocked
                && let Some(entry) = game.active_layer_mut().boulders.get_mut(&blocker_key)
            {
                entry.direction = boulder.direction;
                entry.state = BoulderState::Rolling;
            }
        }
        let reverse = boulder.direction.turned_left().turned_left();
        let (vdc, vdr) = reverse.delta();
        if can_boulder_enter(
            game,
            context,
            layer_index,
            boulder.col + i64::from(vdc),
            boulder.row + i64::from(vdr),
            &boulder,
        ) != EnterResult::Blocked
        {
            if let Some(entry) = game.active_layer_mut().boulders.get_mut(key) {
                entry.direction = reverse;
            }
            return;
        }
        if let Some(entry) = game.active_layer_mut().boulders.get_mut(key) {
            entry.state = BoulderState::Idle;
        }
        return;
    }

    let next_key = door_key(nc, nr);
    if let Some(next_boulder) = game.active_layer().boulders.get(&next_key).cloned() {
        let beyond = can_boulder_enter(
            game,
            context,
            layer_index,
            nc + i64::from(dcol),
            nr + i64::from(drow),
            &next_boulder,
        );
        if beyond != EnterResult::Blocked
            && let Some(entry) = game.active_layer_mut().boulders.get_mut(&next_key)
        {
            entry.direction = boulder.direction;
            entry.state = BoulderState::Rolling;
        }
        if let Some(entry) = game.active_layer_mut().boulders.get_mut(key) {
            entry.state = BoulderState::Idle;
        }
        return;
    }

    let result = can_boulder_enter(game, context, layer_index, nc, nr, &boulder);
    match result {
        EnterResult::KillEnemy => {
            if let Some(event) = kill_enemy_at(game, nc, nr, layer_index) {
                events.push(event);
            }
        }
        EnterResult::DamageEnemy => {
            if let Some(event) =
                damage_enemy_by_boulder(game, nc, nr, boulder.roll_damage, layer_index)
            {
                events.push(event);
            }
        }
        EnterResult::DamagePlayer => {
            let damage = if context.debug_fullbright {
                0.0
            } else {
                boulder.roll_damage
            };
            game.player.hp = (game.player.hp - damage).max(0.0);
            tick_state.player_damage_flash_timer = PLAYER_DAMAGE_FLASH_DURATION;
        }
        EnterResult::Enter | EnterResult::Blocked => {}
    }

    if matches!(
        result,
        EnterResult::Enter
            | EnterResult::KillEnemy
            | EnterResult::DamageEnemy
            | EnterResult::DamagePlayer
    ) {
        let Some(outcome) = move_boulder_same_layer(game, key, layer_index, nc, nr) else {
            return;
        };
        push_moved(
            events,
            BoulderTransitionKind::Rolled,
            key.to_string(),
            layer_index,
            layer_index,
            outcome,
            nc,
            nr,
            boulder.direction,
        );
        return;
    }

    let left_dir = boulder.direction.turned_left();
    let right_dir = boulder.direction.turned_right();
    let (ldc, ldr) = left_dir.delta();
    let (rdc, rdr) = right_dir.delta();
    let left_open = can_boulder_enter(
        game,
        context,
        layer_index,
        boulder.col + i64::from(ldc),
        boulder.row + i64::from(ldr),
        &boulder,
    ) != EnterResult::Blocked;
    let right_open = can_boulder_enter(
        game,
        context,
        layer_index,
        boulder.col + i64::from(rdc),
        boulder.row + i64::from(rdr),
        &boulder,
    ) != EnterResult::Blocked;

    let Some(entry) = game.active_layer_mut().boulders.get_mut(key) else {
        return;
    };
    if left_open && right_open {
        entry.state = BoulderState::Idle;
    } else if left_open {
        entry.direction = left_dir;
    } else if right_open {
        entry.direction = right_dir;
    } else {
        entry.state = BoulderState::Idle;
    }
}

/// Ticks every boulder on every layer in two passes, matching
/// `boulderSystem.ts:318-349` exactly: idle boulders standing over a fresh
/// hole start falling first, then every non-idle boulder (including ones
/// that just started falling in pass one) advances via [`decide_next`].
/// Both passes re-snapshot each layer's boulder keys independently, so a
/// boulder that changes layer in pass one may be visited again — or
/// skipped — in pass two depending on iteration order, exactly as in TS.
pub fn tick_boulders(
    game: &mut GameState,
    context: &BoulderContext,
    tick_state: &mut PlayerTickState,
) -> Vec<BoulderEvent> {
    let saved_layer = game.active_layer_index;
    let mut events = Vec::new();

    for layer_index in 0..game.layers.len() {
        game.active_layer_index = layer_index;
        let boulder_keys: Vec<String> = game.active_layer().boulders.keys().cloned().collect();
        for key in boulder_keys {
            game.active_layer_index = layer_index;
            let Some(boulder) = game.active_layer().boulders.get(&key) else {
                continue;
            };
            if boulder.state != BoulderState::Idle {
                continue;
            }
            if !(context.is_resting)(&layer_door_key(layer_index, &key)) {
                continue;
            }
            let (col, row, direction) = (boulder.col, boulder.row, boulder.direction);
            if !is_hole_at(game, context, col, row, layer_index) {
                continue;
            }
            let landing_layer = compute_landing_layer(game, context, col, row, layer_index);
            let Some(outcome) =
                transfer_boulder_to_layer(game, &key, layer_index, col, row, landing_layer)
            else {
                continue;
            };
            if let Some(entry) = game.active_layer_mut().boulders.get_mut(&outcome.new_key) {
                entry.state = BoulderState::Falling;
            }
            if let Some(event) = outcome.chest_crashed {
                events.push(event);
            }
            events.push(BoulderEvent::Moved(BoulderMoved {
                kind: BoulderTransitionKind::Fell,
                old_key: key,
                old_layer_index: layer_index,
                new_key: outcome.new_key,
                new_layer_index: landing_layer,
                col,
                row,
                direction,
                tripwire_activated: outcome.tripwire_activated,
                plate_activated: outcome.plate_activated,
            }));
        }
    }

    for layer_index in 0..game.layers.len() {
        game.active_layer_index = layer_index;
        let boulder_keys: Vec<String> = game.active_layer().boulders.keys().cloned().collect();
        for key in boulder_keys {
            game.active_layer_index = layer_index;
            let Some(boulder) = game.active_layer().boulders.get(&key) else {
                continue;
            };
            if boulder.state == BoulderState::Idle {
                continue;
            }
            if !(context.is_resting)(&layer_door_key(layer_index, &key)) {
                continue;
            }
            decide_next(game, context, tick_state, &mut events, &key, layer_index);
        }
    }

    game.active_layer_index = saved_layer;
    events
}

/// Ticks every boulder spawner on every layer: advances timers, rolls the
/// next interval on fire (fixed or random-between-min-and-max), and spawns
/// a fresh rolling boulder at the spawner's own cell if it's unoccupied.
pub fn tick_boulder_spawners(
    game: &mut GameState,
    delta: f64,
    random: &mut dyn FnMut() -> f64,
) -> Vec<BoulderEvent> {
    let saved_layer = game.active_layer_index;
    let mut events = Vec::new();

    for layer_index in 0..game.layers.len() {
        game.active_layer_index = layer_index;
        let spawner_keys: Vec<String> = game
            .active_layer()
            .boulder_spawners
            .keys()
            .cloned()
            .collect();
        for spawner_key in spawner_keys {
            let Some(spawner) = game
                .active_layer_mut()
                .boulder_spawners
                .get_mut(&spawner_key)
            else {
                continue;
            };
            if !spawner.active {
                continue;
            }
            spawner.spawn_timer += delta;
            if spawner.spawn_timer < spawner.next_interval {
                continue;
            }
            spawner.spawn_timer -= spawner.next_interval;
            spawner.next_interval = if spawner.interval_mode == IntervalMode::Random {
                spawner.interval_min
                    + random() * (spawner.interval_max - spawner.interval_min).max(0.0)
            } else {
                spawner.interval
            };

            let col = spawner.col;
            let row = spawner.row;
            let direction = spawner.direction;
            let roll_damage = spawner.roll_damage;
            let fall_damage = spawner.fall_damage;
            let insta_kill_enemies = spawner.insta_kill_enemies;
            let pushable = spawner.pushable;

            let cell_key = door_key(col, row);
            if game.active_layer().boulders.contains_key(&cell_key) {
                continue;
            }

            let new_boulder = BoulderInstance {
                id: None,
                col,
                row,
                direction,
                state: BoulderState::Rolling,
                gate_mode: None,
                roll_damage,
                fall_damage,
                insta_kill_enemies,
                pushable,
            };
            game.active_layer_mut()
                .boulders
                .insert(cell_key.clone(), new_boulder);

            events.push(BoulderEvent::Spawned {
                key: cell_key,
                layer_index,
                col,
                row,
                direction,
            });
        }
    }

    game.active_layer_index = saved_layer;
    events
}

//! Real-time enemy AI — pure logic. Behaviors (regen, flee, erratic, fly)
//! come from the enemy database; randomness is injected.

use crate::enemies::EnemyDatabase;
use crate::game_state::{GameState, door_key};
use crate::grid::is_walkable;
use crate::pathfinding::{find_path, manhattan_distance};
use crate::status_effects::{get_slow_multiplier, tick_effects};
use crate::types::EnemyAiState;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnemyActionType {
    Idle,
    Move,
    Attack,
    Regen,
    StatusDamage,
    StatusKill,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnemyAction {
    pub enemy_key: String,
    pub action_type: EnemyActionType,
    pub from_col: i64,
    pub from_row: i64,
    pub to_col: Option<i64>,
    pub to_row: Option<i64>,
}

const DEAGGRO_BUFFER: i64 = 2;
const CARDINAL_DIRS: [(i64, i64); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

pub struct EnemyUpdateContext<'a> {
    pub player_col: i64,
    pub player_row: i64,
    pub grid: &'a [String],
    pub walkable: &'a HashSet<char>,
    /// Door-open lookup; snapshot door state before calling (enemies never
    /// open doors mid-update).
    pub is_door_open: &'a dyn Fn(i64, i64) -> bool,
    pub is_hole: Option<&'a dyn Fn(i64, i64) -> bool>,
    pub is_edge_blocked: Option<&'a dyn Fn(i64, i64, i64, i64) -> bool>,
    pub enemies: &'a EnemyDatabase,
}

fn cell_walkable(context: &EnemyUpdateContext, col: i64, row: i64) -> bool {
    let (Ok(col32), Ok(row32)) = (i32::try_from(col), i32::try_from(row)) else {
        return false;
    };
    let door_open = |c: i32, r: i32| (context.is_door_open)(i64::from(c), i64::from(r));
    is_walkable(
        context.grid,
        col32,
        row32,
        context.walkable,
        Some(&door_open),
        None,
    )
}

fn passable_for_enemy(
    context: &EnemyUpdateContext,
    game_state: &GameState,
    occupied: &HashSet<String>,
    can_fly: bool,
    col: i64,
    row: i64,
    self_cell: Option<(i64, i64)>,
) -> bool {
    if occupied.contains(&door_key(col, row)) && self_cell != Some((col, row)) {
        return false;
    }
    if game_state.is_block_at(col, row) {
        return false;
    }
    if !cell_walkable(context, col, row) {
        return false;
    }
    can_fly || !context.is_hole.is_some_and(|hole| hole(col, row))
}

/// Tick all enemies by `delta` seconds. Each enemy accumulates time and acts
/// when its move timer reaches its move interval. Moves are applied to
/// `game_state`; the returned actions drive rendering and combat.
#[allow(clippy::too_many_lines)]
pub fn update_enemies(
    game_state: &mut GameState,
    context: &EnemyUpdateContext,
    delta: f64,
    random: &mut dyn FnMut() -> f64,
) -> Vec<EnemyAction> {
    let mut actions = Vec::new();

    // Track occupied cells (prevent stacking).
    let mut occupied: HashSet<String> = game_state
        .active_layer()
        .enemies
        .values()
        .map(|enemy| door_key(enemy.col, enemy.row))
        .collect();

    // Process in deterministic order.
    let mut sorted_keys: Vec<String> = game_state.active_layer().enemies.keys().cloned().collect();
    sorted_keys.sort();

    for key in sorted_keys {
        // Stage 1: timers, status effects, and state transitions (mutable).
        let Some(enemy) = game_state
            .layers
            .get_mut(game_state.active_layer_index)
            .and_then(|layer| layer.enemies.get_mut(&key))
        else {
            continue;
        };

        let dist = manhattan_distance(enemy.col, enemy.row, context.player_col, context.player_row);

        let regen_behavior = context.enemies.get_behavior(&enemy.enemy_type, "regen");
        if let Some(regen) = regen_behavior
            && let (Some(mut regen_timer), Some(regen_pause)) =
                (enemy.regen_timer, enemy.regen_pause_timer)
        {
            if regen_pause > 0.0 {
                enemy.regen_pause_timer = Some((regen_pause - delta).max(0.0));
            } else if enemy.hp < enemy.max_hp {
                regen_timer += delta;
                let tick_interval = regen
                    .params
                    .get("tickInterval")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(1.0);
                if regen_timer >= tick_interval {
                    regen_timer -= tick_interval;
                    let hp_per_tick = regen
                        .params
                        .get("hpPerTick")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    enemy.hp = (enemy.hp + hp_per_tick).min(enemy.max_hp);
                    actions.push(EnemyAction {
                        enemy_key: key.clone(),
                        action_type: EnemyActionType::Regen,
                        from_col: enemy.col,
                        from_row: enemy.row,
                        to_col: None,
                        to_row: None,
                    });
                }
                enemy.regen_timer = Some(regen_timer);
            }
        }

        if !enemy.status_effects.is_empty() {
            let result = tick_effects(&mut enemy.status_effects, delta);
            if result.damage > 0.0 {
                enemy.hp -= result.damage;
                actions.push(EnemyAction {
                    enemy_key: key.clone(),
                    action_type: EnemyActionType::StatusDamage,
                    from_col: enemy.col,
                    from_row: enemy.row,
                    to_col: None,
                    to_row: None,
                });
            }
            enemy.status_effects.retain(|effect| effect.remaining > 0.0);
            if enemy.hp <= 0.0 {
                actions.push(EnemyAction {
                    enemy_key: key.clone(),
                    action_type: EnemyActionType::StatusKill,
                    from_col: enemy.col,
                    from_row: enemy.row,
                    to_col: None,
                    to_row: None,
                });
                continue;
            }
        }

        let flee_behavior = context.enemies.get_behavior(&enemy.enemy_type, "flee");
        let flee_hp_threshold = flee_behavior
            .and_then(|behavior| behavior.params.get("hpThreshold"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        if flee_behavior.is_some()
            && enemy.hp < enemy.max_hp * flee_hp_threshold
            && enemy.ai_state != EnemyAiState::Flee
        {
            enemy.ai_state = EnemyAiState::Flee;
        }

        if enemy.ai_state == EnemyAiState::Idle && dist <= enemy.aggro_range as i64 {
            enemy.ai_state = EnemyAiState::Chase;
        } else if enemy.ai_state == EnemyAiState::Chase
            && dist > enemy.aggro_range as i64 + DEAGGRO_BUFFER
        {
            enemy.ai_state = EnemyAiState::Idle;
        }

        if enemy.ai_state == EnemyAiState::Chase && dist <= 1 {
            enemy.ai_state = EnemyAiState::Attack;
        } else if enemy.ai_state == EnemyAiState::Attack && dist > 1 {
            enemy.ai_state =
                if flee_behavior.is_some() && enemy.hp < enemy.max_hp * flee_hp_threshold {
                    EnemyAiState::Flee
                } else {
                    EnemyAiState::Chase
                };
        }

        // Flee at multiplied speed; the slow effect slows enemy movement.
        let flee_speed_multiplier = flee_behavior
            .and_then(|behavior| behavior.params.get("speedMultiplier"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0);
        let mut effective_interval =
            if enemy.ai_state == EnemyAiState::Flee && flee_behavior.is_some() {
                enemy.move_interval / flee_speed_multiplier
            } else {
                enemy.move_interval
            };
        effective_interval *= get_slow_multiplier(&enemy.status_effects);

        enemy.move_timer += delta;
        if enemy.move_timer < effective_interval {
            continue;
        }
        enemy.move_timer = 0.0;

        let (enemy_col, enemy_row, enemy_type, ai_state) = (
            enemy.col,
            enemy.row,
            enemy.enemy_type.clone(),
            enemy.ai_state,
        );

        if ai_state == EnemyAiState::Attack {
            actions.push(EnemyAction {
                enemy_key: key.clone(),
                action_type: EnemyActionType::Attack,
                from_col: enemy_col,
                from_row: enemy_row,
                to_col: None,
                to_row: None,
            });
            continue;
        }

        let can_fly = context
            .enemies
            .get_enemy(&enemy_type)
            .is_some_and(|def| def.fly == Some(true));

        // Stage 2: movement decisions (shared borrows only).
        if ai_state == EnemyAiState::Flee {
            let best_cell = {
                let shared: &GameState = &*game_state;
                let mut best: Option<(i64, i64)> = None;
                let mut best_dist = -1;
                for (dcol, drow) in CARDINAL_DIRS {
                    let (next_col, next_row) = (enemy_col + dcol, enemy_row + drow);
                    if context
                        .is_edge_blocked
                        .is_some_and(|blocked| blocked(enemy_col, enemy_row, next_col, next_row))
                    {
                        continue;
                    }
                    if !passable_for_enemy(
                        context, shared, &occupied, can_fly, next_col, next_row, None,
                    ) {
                        continue;
                    }
                    let candidate_dist = manhattan_distance(
                        next_col,
                        next_row,
                        context.player_col,
                        context.player_row,
                    );
                    if candidate_dist > best_dist {
                        best_dist = candidate_dist;
                        best = Some((next_col, next_row));
                    }
                }
                best
            };

            if let Some((to_col, to_row)) = best_cell {
                occupied.remove(&door_key(enemy_col, enemy_row));
                occupied.insert(door_key(to_col, to_row));
                actions.push(EnemyAction {
                    enemy_key: key.clone(),
                    action_type: EnemyActionType::Move,
                    from_col: enemy_col,
                    from_row: enemy_row,
                    to_col: Some(to_col),
                    to_row: Some(to_row),
                });
                game_state.move_enemy(enemy_col, enemy_row, to_col, to_row);
            } else {
                // Cornered — fight back.
                if let Some(enemy) = game_state.active_layer_mut().enemies.get_mut(&key) {
                    enemy.ai_state = EnemyAiState::Attack;
                }
                actions.push(EnemyAction {
                    enemy_key: key.clone(),
                    action_type: EnemyActionType::Attack,
                    from_col: enemy_col,
                    from_row: enemy_row,
                    to_col: None,
                    to_row: None,
                });
            }
            continue;
        }

        if ai_state == EnemyAiState::Chase {
            let step = {
                let shared: &GameState = &*game_state;
                let is_passable = |col: i64, row: i64| {
                    passable_for_enemy(
                        context,
                        shared,
                        &occupied,
                        can_fly,
                        col,
                        row,
                        Some((enemy_col, enemy_row)),
                    )
                };
                let path = find_path(
                    context.grid,
                    enemy_col,
                    enemy_row,
                    context.player_col,
                    context.player_row,
                    &is_passable,
                    context.is_edge_blocked,
                );

                path.filter(|path| path.len() > 1).map(|path| {
                    let mut step = path[0];
                    let erratic_chance = context
                        .enemies
                        .get_behavior(&enemy_type, "erratic")
                        .and_then(|behavior| behavior.params.get("chance"))
                        .and_then(serde_json::Value::as_f64);
                    if let Some(chance) = erratic_chance
                        && random() < chance
                    {
                        let mut candidates = Vec::new();
                        for (dcol, drow) in CARDINAL_DIRS {
                            let (next_col, next_row) = (enemy_col + dcol, enemy_row + drow);
                            if context.is_edge_blocked.is_some_and(|blocked| {
                                blocked(enemy_col, enemy_row, next_col, next_row)
                            }) {
                                continue;
                            }
                            if passable_for_enemy(
                                context, shared, &occupied, can_fly, next_col, next_row, None,
                            ) {
                                candidates.push((next_col, next_row));
                            }
                        }
                        if !candidates.is_empty() {
                            let index = (random() * candidates.len() as f64).floor() as usize;
                            step = candidates[index.min(candidates.len() - 1)];
                        }
                    }
                    step
                })
            };

            if let Some((to_col, to_row)) = step {
                let step_key = door_key(to_col, to_row);
                if step_key != door_key(context.player_col, context.player_row)
                    && !occupied.contains(&step_key)
                {
                    occupied.remove(&door_key(enemy_col, enemy_row));
                    occupied.insert(step_key);
                    actions.push(EnemyAction {
                        enemy_key: key.clone(),
                        action_type: EnemyActionType::Move,
                        from_col: enemy_col,
                        from_row: enemy_row,
                        to_col: Some(to_col),
                        to_row: Some(to_row),
                    });
                    game_state.move_enemy(enemy_col, enemy_row, to_col, to_row);
                    continue;
                }
            }
        }

        // Idle or couldn't move — no action emitted.
    }

    actions
}

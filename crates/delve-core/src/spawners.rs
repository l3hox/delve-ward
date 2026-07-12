//! Enemy spawner BFS placement and activation, ported from the TS
//! `spawnerSystem`. Pure logic: `GameState` owns spawner/enemy state, but
//! grid text and the enemy database live outside it (grids belong to the
//! loaded level, not the game state), so both are injected via
//! [`SpawnerContext`] — the same shape `enemy_ai::EnemyUpdateContext` uses.
//! Spawns are returned as [`SpawnResult`]s instead of mutating a scene
//! directly, so the shell builds meshes from them.

use crate::enemies::EnemyDatabase;
use crate::game_state::{GameState, door_key};
use crate::grid::cell_at;
use crate::types::{CharDef, EnemyInstance};
use std::collections::{HashSet, VecDeque};

/// Read-only per-tick inputs the shell must supply.
pub struct SpawnerContext<'a> {
    /// One grid per dungeon layer, indexed by layer index.
    pub layer_grids: &'a [Vec<String>],
    pub char_defs: &'a [CharDef],
    pub walkable: &'a HashSet<char>,
    pub enemies: &'a EnemyDatabase,
    /// `GameState::active_layer_index` before this tick — the layer the
    /// player currently occupies, used to exclude their cell from spawn
    /// candidates on that one layer.
    pub player_layer: usize,
    pub player_col: i64,
    pub player_row: i64,
}

/// A newly spawned enemy, returned so the shell can build its mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnResult {
    pub layer_index: usize,
    pub cell_key: String,
    pub enemy: EnemyInstance,
}

fn is_solid_wall(character: char, char_defs: &[CharDef]) -> bool {
    character == '#'
        || char_defs
            .iter()
            .find(|def| def.character == character)
            .is_some_and(|def| def.solid && def.see_through != Some(true))
}

/// A cell is a hole, for BFS traversal purposes, when the layer below is
/// missing or its cell isn't a solid, non-see-through wall/floor character.
///
/// This intentionally does not share code with `boulders::is_hole_at`:
/// the TS `spawnerSystem.ts` hole check only ever looks at the layer
/// below's grid character, while `boulderSystem.ts`'s `isHoleAt` also
/// consults `TextureArea.openBottom`, open pit traps, and whether a
/// boulder/block already plugs the hole. Unifying them would either make
/// spawner placement react to pit traps/area overrides it never did in TS,
/// or strip boulder falling of checks it depends on — porting both
/// separately, faithfully, avoids that silent behavior drift.
fn is_traversal_hole(
    below_grid: Option<&Vec<String>>,
    col: i32,
    row: i32,
    char_defs: &[CharDef],
) -> bool {
    let Some(below_grid) = below_grid else {
        return true;
    };
    cell_at(below_grid, col, row).is_none_or(|character| !is_solid_wall(character, char_defs))
}

const CARDINAL_DELTAS: [(i64, i64); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];

/// BFS candidate search from a spawner's own cell out to `spawn_radius`
/// hops, matching `spawnerSystem.ts:51-77` exactly: cells are candidates
/// (spawn-eligible) when unoccupied and reachable only through walkable,
/// non-hole (for non-flying enemies) cells.
#[allow(clippy::too_many_arguments)]
fn find_spawn_candidates(
    game: &GameState,
    context: &SpawnerContext,
    layer_index: usize,
    layer_grid: &[String],
    below_grid: Option<&Vec<String>>,
    spawner_col: i64,
    spawner_row: i64,
    spawn_radius: f64,
    can_fly: bool,
) -> Vec<(i64, i64)> {
    let mut candidates = Vec::new();
    let mut visited: HashSet<(i64, i64)> = HashSet::from([(spawner_col, spawner_row)]);
    let mut queue: VecDeque<(i64, i64, f64)> = VecDeque::from([(spawner_col, spawner_row, 0.0)]);

    while let Some((col, row, dist)) = queue.pop_front() {
        if dist > 0.0 {
            let key = door_key(col, row);
            let occupied = game.active_layer().enemies.contains_key(&key)
                || game.is_block_at(col, row)
                || (layer_index == context.player_layer
                    && col == context.player_col
                    && row == context.player_row);
            if !occupied {
                candidates.push((col, row));
            }
        }
        if dist >= spawn_radius {
            continue;
        }
        for (dcol, drow) in CARDINAL_DELTAS {
            let (next_col, next_row) = (col + dcol, row + drow);
            if visited.contains(&(next_col, next_row)) {
                continue;
            }
            let Ok(nc) = i32::try_from(next_col) else {
                continue;
            };
            let Ok(nr) = i32::try_from(next_row) else {
                continue;
            };
            let Some(character) = cell_at(layer_grid, nc, nr) else {
                continue;
            };
            if !context.walkable.contains(&character) {
                continue;
            }
            if !can_fly && is_traversal_hole(below_grid, nc, nr, context.char_defs) {
                continue;
            }
            visited.insert((next_col, next_row));
            queue.push_back((next_col, next_row, dist + 1.0));
        }
    }
    candidates
}

/// Ticks every spawner on every layer: advances timers, and where due and
/// under `max_active`, BFS-searches for a spawn cell and creates an enemy
/// there. Mutates `game`'s active layer index while iterating layers
/// (restored before returning), matching the TS `savedLayer` pattern.
pub fn tick_spawners(
    game: &mut GameState,
    context: &SpawnerContext,
    delta: f64,
    random: &mut dyn FnMut() -> f64,
) -> Vec<SpawnResult> {
    let saved_layer = game.active_layer_index;
    let mut results = Vec::new();

    for layer_index in 0..game.layers.len() {
        game.active_layer_index = layer_index;
        let Some(layer_grid) = context.layer_grids.get(layer_index) else {
            continue;
        };
        let below_grid = layer_index
            .checked_sub(1)
            .and_then(|below| context.layer_grids.get(below));

        let spawner_keys: Vec<String> = game.active_layer().spawners.keys().cloned().collect();
        for spawner_key in spawner_keys {
            let Some(spawner) = game.active_layer_mut().spawners.get_mut(&spawner_key) else {
                continue;
            };
            if !spawner.active {
                continue;
            }
            spawner.spawn_timer += delta;
            if spawner.spawn_timer < spawner.interval {
                continue;
            }
            spawner.spawn_timer -= spawner.interval;

            let spawner_col = spawner.col;
            let spawner_row = spawner.row;
            let spawn_radius = spawner.spawn_radius;
            let max_active = spawner.max_active;
            let enemy_type = spawner.enemy_type.clone();
            let spawner_id = spawner.id.clone();

            // `Option<String>` equality matches JS `===` on possibly-`undefined`
            // ids: an id-less spawner (`None`) counts every id-less enemy
            // toward its own cap, same as TS's `enemy.spawnerId === spawner.id`.
            let alive_count = game
                .active_layer()
                .enemies
                .values()
                .filter(|enemy| enemy.spawner_id == spawner_id)
                .count();
            if (alive_count as f64) >= max_active {
                continue;
            }

            let can_fly = context
                .enemies
                .get_enemy(&enemy_type)
                .is_some_and(|def| def.fly == Some(true));

            let candidates = find_spawn_candidates(
                game,
                context,
                layer_index,
                layer_grid,
                below_grid,
                spawner_col,
                spawner_row,
                spawn_radius,
                can_fly,
            );
            if candidates.is_empty() {
                continue;
            }
            let index = (random() * candidates.len() as f64).floor() as usize;
            let (spawn_col, spawn_row) = candidates[index.min(candidates.len() - 1)];

            let Ok(mut enemy) =
                context
                    .enemies
                    .create_enemy_instance(spawn_col, spawn_row, &enemy_type)
            else {
                continue;
            };
            enemy.spawner_id = spawner_id;
            let cell_key = door_key(spawn_col, spawn_row);
            game.active_layer_mut()
                .enemies
                .insert(cell_key.clone(), enemy.clone());

            results.push(SpawnResult {
                layer_index,
                cell_key,
                enemy,
            });
        }
    }

    game.active_layer_index = saved_layer;
    results
}

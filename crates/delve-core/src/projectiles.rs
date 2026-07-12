//! Projectile system: owns all active projectiles and drives their
//! movement, collision, and lifecycle. Projectiles are transient runtime
//! objects, not level entities; positions are fractional and projectiles
//! move through the grid in real time.

use crate::grid::Facing;
use serde::{Deserialize, Serialize};

/// Offset from the mount cell toward the wall a launcher sits on, applied at
/// spawn so projectiles appear to leave the wall face rather than the cell
/// center.
const WALL_OFFSET: f64 = 0.45;

/// Safety guard against runaway boundary-crossing loops in `cells_on_path`.
const MAX_PATH_CELLS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DamageType {
    Physical,
    Fire,
}

/// Projectile origin. M4 adds `Player` and `Enemy` variants; only traps fire
/// projectiles today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectileSource {
    Trap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitType {
    Wall,
    Door,
    Player,
    Enemy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Projectile {
    pub id: String,
    pub col: f64,
    pub row: f64,
    pub direction: Facing,
    pub speed: f64,
    pub damage: f64,
    pub damage_type: DamageType,
    /// Stub for the Phase C status-effect system, e.g. `"burning"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_effect: Option<String>,
    pub source: ProjectileSource,
    pub projectile_type: String,
    pub traveled: f64,
    pub max_range: f64,
    pub layer_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileStats {
    pub speed: f64,
    pub damage: f64,
    pub damage_type: DamageType,
    pub max_range: f64,
    pub status_effect: Option<&'static str>,
}

/// Stats lookup for a projectile type, or `None` for an unknown type.
#[must_use]
pub fn projectile_stats(projectile_type: &str) -> Option<ProjectileStats> {
    match projectile_type {
        "dart" => Some(ProjectileStats {
            speed: 8.0,
            damage: 3.0,
            damage_type: DamageType::Physical,
            max_range: 20.0,
            status_effect: None,
        }),
        "arrow" => Some(ProjectileStats {
            speed: 6.0,
            damage: 5.0,
            damage_type: DamageType::Physical,
            max_range: 15.0,
            status_effect: None,
        }),
        "fireball" => Some(ProjectileStats {
            speed: 4.0,
            damage: 8.0,
            damage_type: DamageType::Fire,
            max_range: 10.0,
            status_effect: Some("burning"),
        }),
        _ => None,
    }
}

pub struct SpawnOptions<'a> {
    pub col: i64,
    pub row: i64,
    pub direction: Facing,
    pub projectile_type: &'a str,
    pub source: Option<ProjectileSource>,
    pub max_range: Option<f64>,
    pub layer_index: Option<usize>,
}

/// Collision predicates and player position for one `update` tick. The
/// game-state shell owns level queries; the manager never reaches into
/// level state directly.
pub struct ProjectileUpdateContext<'a> {
    pub is_walkable: &'a dyn Fn(i64, i64) -> bool,
    pub is_door_open: &'a dyn Fn(i64, i64) -> bool,
    pub player_col: i64,
    pub player_row: i64,
    pub is_enemy_at: Option<&'a dyn Fn(i64, i64) -> bool>,
    pub is_block_at: Option<&'a dyn Fn(i64, i64) -> bool>,
    pub is_solid_edge_blocked: Option<&'a dyn Fn(i64, i64, i64, i64) -> bool>,
    pub layer_filter: Option<usize>,
}

/// A projectile came to rest against `col`/`row` this tick. The manager has
/// already removed it from the active set; the renderer/game-state shell
/// applies damage and effects from this event.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileHitEvent {
    pub projectile: Projectile,
    pub col: i64,
    pub row: i64,
    pub hit_type: HitType,
}

#[derive(Debug)]
pub struct ProjectileManager {
    // Insertion order preserved, matching JS Map iteration order.
    projectiles: Vec<Projectile>,
    next_id: i64,
}

impl Default for ProjectileManager {
    fn default() -> Self {
        Self {
            projectiles: Vec::new(),
            next_id: 1,
        }
    }
}

impl ProjectileManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns a projectile offset toward the wall its launcher mounts on.
    /// Errors for an unknown `projectile_type`.
    pub fn spawn(&mut self, opts: SpawnOptions) -> Result<Projectile, String> {
        let Some(stats) = projectile_stats(opts.projectile_type) else {
            return Err(format!(
                "Unknown projectile type: '{}'",
                opts.projectile_type
            ));
        };

        let id = format!("proj_{}", self.next_id);
        self.next_id += 1;

        let (offset_col, offset_row) = match opts.direction {
            Facing::N => (0.0, WALL_OFFSET),
            Facing::S => (0.0, -WALL_OFFSET),
            Facing::E => (-WALL_OFFSET, 0.0),
            Facing::W => (WALL_OFFSET, 0.0),
        };

        let projectile = Projectile {
            id,
            col: opts.col as f64 + 0.5 + offset_col,
            row: opts.row as f64 + 0.5 + offset_row,
            direction: opts.direction,
            speed: stats.speed,
            damage: stats.damage,
            damage_type: stats.damage_type,
            status_effect: stats.status_effect.map(str::to_string),
            source: opts.source.unwrap_or(ProjectileSource::Trap),
            projectile_type: opts.projectile_type.to_string(),
            traveled: 0.0,
            max_range: opts.max_range.unwrap_or(stats.max_range),
            layer_index: opts.layer_index.unwrap_or(0),
        };

        self.projectiles.push(projectile.clone());
        Ok(projectile)
    }

    /// Advances every projectile by `delta` seconds, walking every integer
    /// cell boundary crossed so fast projectiles cannot tunnel through thin
    /// obstacles in a single tick. Projectiles that collide or exceed
    /// `max_range` are removed; only collisions are reported (range expiry
    /// is silent, matching the TS behavior).
    pub fn update(
        &mut self,
        delta: f64,
        context: &ProjectileUpdateContext,
    ) -> Vec<ProjectileHitEvent> {
        let mut events = Vec::new();
        let mut to_remove: Vec<String> = Vec::new();

        for projectile in &mut self.projectiles {
            if context
                .layer_filter
                .is_some_and(|filter| filter != projectile.layer_index)
            {
                continue;
            }

            let (direction_col, direction_row) = projectile.direction.delta();
            let move_distance = projectile.speed * delta;
            let start_col = projectile.col;
            let start_row = projectile.row;
            let step_col = f64::from(direction_col) * move_distance;
            let step_row = f64::from(direction_row) * move_distance;
            let end_col = start_col + step_col;
            let end_row = start_row + step_row;

            let cells = cells_on_path(start_col, start_row, end_col, end_row);

            let mut collided = false;
            let mut previous_cell: Option<(i64, i64)> = None;

            for (cell_col, cell_row, distance_at_entry) in cells {
                let crossed_thin_wall = previous_cell.is_some_and(|(from_col, from_row)| {
                    context.is_solid_edge_blocked.is_some_and(|is_blocked| {
                        is_blocked(from_col, from_row, cell_col, cell_row)
                    })
                });
                previous_cell = Some((cell_col, cell_row));

                let hit_type = if crossed_thin_wall {
                    Some(HitType::Wall)
                } else {
                    check_cell_collision(cell_col, cell_row, context)
                };

                if let Some(hit_type) = hit_type {
                    let fraction = distance_at_entry / move_distance;
                    projectile.col = start_col + step_col * fraction;
                    projectile.row = start_row + step_row * fraction;
                    projectile.traveled += distance_at_entry;
                    events.push(ProjectileHitEvent {
                        projectile: projectile.clone(),
                        col: cell_col,
                        row: cell_row,
                        hit_type,
                    });
                    to_remove.push(projectile.id.clone());
                    collided = true;
                    break;
                }
            }

            if collided {
                continue;
            }

            projectile.col = end_col;
            projectile.row = end_row;
            projectile.traveled += move_distance;

            if projectile.traveled >= projectile.max_range {
                to_remove.push(projectile.id.clone());
            }
        }

        self.projectiles.retain(|p| !to_remove.contains(&p.id));
        events
    }

    #[must_use]
    pub fn get_all(&self) -> &[Projectile] {
        &self.projectiles
    }

    pub fn remove_by_id(&mut self, id: &str) {
        self.projectiles.retain(|p| p.id != id);
    }

    pub fn clear(&mut self) {
        self.projectiles.clear();
    }

    /// Snapshot of all active projectiles, independent of the manager's
    /// internal state.
    #[must_use]
    pub fn save_state(&self) -> Vec<Projectile> {
        self.projectiles.clone()
    }

    /// Replaces all active projectiles with `projectiles`. Entries sharing
    /// an id keep the position of their first occurrence, matching JS
    /// `Map.set` semantics.
    pub fn load_state(&mut self, projectiles: Vec<Projectile>) {
        self.projectiles.clear();
        for projectile in projectiles {
            if let Some(existing) = self.projectiles.iter_mut().find(|p| p.id == projectile.id) {
                *existing = projectile;
            } else {
                self.projectiles.push(projectile);
            }
        }
    }
}

/// Returns the HitType if a collision is detected in `col`/`row`, `None`
/// otherwise. Wall and door checks take priority over entity checks.
fn check_cell_collision(col: i64, row: i64, context: &ProjectileUpdateContext) -> Option<HitType> {
    if !(context.is_walkable)(col, row) {
        return Some(HitType::Wall);
    }
    if !(context.is_door_open)(col, row) {
        return Some(HitType::Door);
    }
    if context
        .is_block_at
        .is_some_and(|is_block_at| is_block_at(col, row))
    {
        return Some(HitType::Wall); // blocks stop projectiles like walls
    }
    if col == context.player_col && row == context.player_row {
        return Some(HitType::Player);
    }
    if context
        .is_enemy_at
        .is_some_and(|is_enemy_at| is_enemy_at(col, row))
    {
        return Some(HitType::Enemy);
    }
    None
}

/// Every integer cell entered while moving from `(start_col, start_row)` to
/// `(end_col, end_row)`, in traversal order, paired with how far into the
/// move each boundary was crossed. Movement is axis-aligned, so boundaries
/// are walked one integer step at a time instead of jumping straight to the
/// end cell — this is what stops fast projectiles from tunnelling through a
/// cell in a single tick.
///
/// The start cell is included at distance `0` unless a crossing already
/// lands there, so spawn-in-cell collisions (a player standing on the
/// launch cell) are still detected.
fn cells_on_path(
    start_col: f64,
    start_row: f64,
    end_col: f64,
    end_row: f64,
) -> Vec<(i64, i64, f64)> {
    let mut result = Vec::new();

    let delta_col = end_col - start_col;
    let delta_row = end_row - start_row;
    let total_distance = if delta_col != 0.0 {
        delta_col.abs()
    } else {
        delta_row.abs()
    };

    if total_distance == 0.0 {
        return result;
    }

    let mut crossings: Vec<(i64, i64, f64)> = Vec::new();

    if delta_col != 0.0 {
        let boundary_step = if delta_col > 0.0 { 1.0 } else { -1.0 };
        let mut boundary = if delta_col > 0.0 {
            start_col.ceil()
        } else {
            start_col.floor()
        };
        loop {
            let distance = (boundary - start_col).abs();
            if distance > total_distance + 1e-9 {
                break;
            }
            let cell_col = if delta_col > 0.0 {
                boundary
            } else {
                boundary - 1.0
            };
            let cell_row = start_row.floor();
            crossings.push((cell_col as i64, cell_row as i64, distance));
            if crossings.len() > MAX_PATH_CELLS {
                break;
            }
            boundary += boundary_step;
        }
    } else {
        let boundary_step = if delta_row > 0.0 { 1.0 } else { -1.0 };
        let mut boundary = if delta_row > 0.0 {
            start_row.ceil()
        } else {
            start_row.floor()
        };
        loop {
            let distance = (boundary - start_row).abs();
            if distance > total_distance + 1e-9 {
                break;
            }
            let cell_col = start_col.floor();
            let cell_row = if delta_row > 0.0 {
                boundary
            } else {
                boundary - 1.0
            };
            crossings.push((cell_col as i64, cell_row as i64, distance));
            if crossings.len() > MAX_PATH_CELLS {
                break;
            }
            boundary += boundary_step;
        }
    }

    let start_cell = (start_col.floor() as i64, start_row.floor() as i64);
    let already_has_start = crossings
        .first()
        .is_some_and(|(_, _, distance)| *distance == 0.0);
    if !already_has_start {
        result.push((start_cell.0, start_cell.1, 0.0));
    }
    result.extend(crossings);

    result
}

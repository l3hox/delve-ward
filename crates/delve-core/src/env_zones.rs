//! Environment zone map builder, ported from the TS `rendering/environment.ts`
//! `buildEnvZoneMap`/`buildEnvZoneMapWithExistingZones`: assigns each
//! distinct [`Environment`] present on a layer's grid a 1-based zone index
//! for multi-pass rendering, in first-encountered (row-major) order.
//!
//! [`resolve_environment_at_cell`] duplicates `delve-game::environment`'s
//! function of the same name rather than importing it — `delve-core` never
//! depends on `delve-game`. The two copies are meant to be consolidated once
//! the shell's zone-rendering slice lands and can call into this one
//! instead.

use crate::types::{Environment, TextureArea};
use std::collections::HashMap;

/// The environment at a cell: the level default, overridden by the last
/// matching area that declares one.
#[must_use]
pub fn resolve_environment_at_cell(
    col: i64,
    row: i64,
    level_environment: Environment,
    areas: &[TextureArea],
) -> Environment {
    let mut environment = level_environment;
    for area in areas {
        if let Some(area_environment) = area.environment
            && col >= i64::from(area.from_col)
            && col <= i64::from(area.to_col)
            && row >= i64::from(area.from_row)
            && row <= i64::from(area.to_row)
        {
            environment = area_environment;
        }
    }
    environment
}

/// `Environment` doesn't derive `Hash` (it's a small, closed 4-variant
/// enum), so zone-index lookups use a linear scan over the already-tiny
/// `zones` list instead of a `HashMap<Environment, _>`.
fn zone_index_for(zones: &[Environment], environment: Environment) -> Option<usize> {
    zones
        .iter()
        .position(|&zone| zone == environment)
        .map(|position| position + 1)
}

fn cell_key(col: usize, row: usize) -> String {
    format!("{col},{row}")
}

fn grid_dimensions(grid: &[String]) -> (usize, usize) {
    let rows = grid.len();
    let cols = grid.first().map_or(0, |row| row.chars().count());
    (rows, cols)
}

/// Cell → 1-based zone index, the distinct environments in
/// first-encountered (row-major) order, and whether more than one zone is
/// present (multi-pass rendering is only needed when `multi_zone` is true).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvZoneMap {
    pub zone_by_cell: HashMap<String, usize>,
    pub zones: Vec<Environment>,
    pub multi_zone: bool,
}

/// Ported from `buildEnvZoneMap`.
#[must_use]
pub fn build_env_zone_map(
    grid: &[String],
    level_environment: Environment,
    areas: &[TextureArea],
) -> EnvZoneMap {
    let mut zone_by_cell = HashMap::new();
    let mut zones: Vec<Environment> = Vec::new();

    let (rows, cols) = grid_dimensions(grid);
    for row in 0..rows {
        for col in 0..cols {
            let environment =
                resolve_environment_at_cell(col as i64, row as i64, level_environment, areas);
            let index = match zone_index_for(&zones, environment) {
                Some(index) => index,
                None => {
                    zones.push(environment);
                    zones.len()
                }
            };
            zone_by_cell.insert(cell_key(col, row), index);
        }
    }

    let multi_zone = zones.len() > 1;
    EnvZoneMap {
        zone_by_cell,
        zones,
        multi_zone,
    }
}

/// Ported from `buildEnvZoneMapWithExistingZones`: reuses a zone-index
/// assignment already established elsewhere (so every layer of the same
/// level agrees on which zone index means which environment) instead of
/// rediscovering indices in this grid's own first-encountered order.
/// Environments absent from `existing_zones` default to zone 1, matching
/// TS's `?? 1` fallback exactly.
#[must_use]
pub fn build_env_zone_map_with_existing_zones(
    grid: &[String],
    level_environment: Environment,
    areas: &[TextureArea],
    existing_zones: &[Environment],
) -> HashMap<String, usize> {
    let mut zone_by_cell = HashMap::new();
    let (rows, cols) = grid_dimensions(grid);
    for row in 0..rows {
        for col in 0..cols {
            let environment =
                resolve_environment_at_cell(col as i64, row as i64, level_environment, areas);
            let index = zone_index_for(existing_zones, environment).unwrap_or(1);
            zone_by_cell.insert(cell_key(col, row), index);
        }
    }
    zone_by_cell
}

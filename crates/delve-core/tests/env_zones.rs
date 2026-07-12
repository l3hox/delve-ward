//! From-scratch behavioral spec for `env_zones.rs` — no TS test file covers
//! `buildEnvZoneMap`/`buildEnvZoneMapWithExistingZones`
//! (`rendering/environment.ts`), so these cases are derived directly from
//! reading the TS implementation rather than ported from an existing suite.

use delve_core::env_zones::{
    build_env_zone_map, build_env_zone_map_with_existing_zones, resolve_environment_at_cell,
};
use delve_core::types::{Environment, TextureArea, TextureSet};

fn grid(rows: &[&str]) -> Vec<String> {
    rows.iter().map(ToString::to_string).collect()
}

fn area(
    from_col: i32,
    to_col: i32,
    from_row: i32,
    to_row: i32,
    environment: Environment,
) -> TextureArea {
    TextureArea {
        from_col,
        to_col,
        from_row,
        to_row,
        environment: Some(environment),
        open_bottom: None,
        open_top: None,
        textures: TextureSet::default(),
    }
}

fn grid_3x3() -> Vec<String> {
    grid(&["...", "...", "..."])
}

// ---------------------------------------------------------------------------
// resolve_environment_at_cell
// ---------------------------------------------------------------------------

#[test]
fn resolve_environment_at_cell_uses_level_default_with_no_areas() {
    assert_eq!(
        resolve_environment_at_cell(0, 0, Environment::Dungeon, &[]),
        Environment::Dungeon
    );
}

#[test]
fn resolve_environment_at_cell_uses_the_last_matching_area() {
    let areas = [
        area(0, 2, 0, 2, Environment::Mist),
        area(1, 1, 1, 1, Environment::Forest),
    ];
    // (1,1) is covered by both areas; the later one in the list wins.
    assert_eq!(
        resolve_environment_at_cell(1, 1, Environment::Dungeon, &areas),
        Environment::Forest
    );
    // (0,0) is only covered by the first area.
    assert_eq!(
        resolve_environment_at_cell(0, 0, Environment::Dungeon, &areas),
        Environment::Mist
    );
}

// ---------------------------------------------------------------------------
// build_env_zone_map — 1. single-environment level (no multipass)
// ---------------------------------------------------------------------------

#[test]
fn single_environment_level_has_exactly_one_zone_and_no_multipass() {
    let map = build_env_zone_map(&grid_3x3(), Environment::Dungeon, &[]);
    assert_eq!(map.zones, vec![Environment::Dungeon]);
    assert!(!map.multi_zone);
    assert_eq!(map.zone_by_cell.len(), 9);
    assert!(map.zone_by_cell.values().all(|&zone| zone == 1));
}

// ---------------------------------------------------------------------------
// build_env_zone_map — 2. area override creates a second zone, in
// first-encountered (row-major) order
// ---------------------------------------------------------------------------

#[test]
fn area_override_creates_a_second_zone_in_first_encountered_order() {
    // Cell (0,0) — the very first cell scanned — is overridden to Outdoor;
    // everything else stays the level default, Dungeon.
    let areas = [area(0, 0, 0, 0, Environment::Outdoor)];
    let map = build_env_zone_map(&grid_3x3(), Environment::Dungeon, &areas);

    assert!(map.multi_zone);
    // Outdoor is encountered first (at (0,0)), so it claims zone 1; Dungeon
    // is encountered next (at (1,0)) and claims zone 2.
    assert_eq!(map.zones, vec![Environment::Outdoor, Environment::Dungeon]);
    assert_eq!(map.zone_by_cell["0,0"], 1);
    assert_eq!(map.zone_by_cell["1,0"], 2);
    assert_eq!(map.zone_by_cell["2,2"], 2);
}

// ---------------------------------------------------------------------------
// build_env_zone_map — 3. overlapping areas: last one wins
// ---------------------------------------------------------------------------

#[test]
fn overlapping_areas_the_last_one_in_the_list_wins() {
    let areas = [
        area(0, 2, 0, 2, Environment::Mist),   // whole grid
        area(1, 1, 1, 1, Environment::Forest), // just the center cell, listed after Mist
    ];
    let map = build_env_zone_map(&grid_3x3(), Environment::Dungeon, &areas);

    assert_eq!(map.zones, vec![Environment::Mist, Environment::Forest]);
    assert_eq!(map.zone_by_cell["0,0"], 1); // Mist only
    assert_eq!(map.zone_by_cell["1,1"], 2); // Mist, then overwritten by Forest
    assert_eq!(map.zone_by_cell["2,2"], 1); // Mist only
}

// ---------------------------------------------------------------------------
// build_env_zone_map — 4. out-of-area cells fall back to the level default
// ---------------------------------------------------------------------------

#[test]
fn cells_outside_any_area_use_the_level_environment() {
    // The area only covers the top-left cell.
    let areas = [area(0, 0, 0, 0, Environment::Outdoor)];
    let map = build_env_zone_map(&grid_3x3(), Environment::Dungeon, &areas);

    assert_eq!(map.zone_by_cell["0,0"], 1); // inside the area: Outdoor
    assert_eq!(map.zone_by_cell["1,0"], 2); // outside the area: Dungeon
    assert_eq!(map.zone_by_cell["0,1"], 2);
    assert_eq!(map.zone_by_cell["2,2"], 2);
}

// ---------------------------------------------------------------------------
// build_env_zone_map_with_existing_zones
// ---------------------------------------------------------------------------

#[test]
fn with_existing_zones_reuses_the_supplied_zone_indices() {
    // Zone indices established by another layer: Dungeon=1, Outdoor=2 — the
    // reverse of the order this grid alone would discover them in.
    let existing_zones = [Environment::Dungeon, Environment::Outdoor];
    let areas = [area(0, 0, 0, 0, Environment::Outdoor)];
    let zone_by_cell = build_env_zone_map_with_existing_zones(
        &grid_3x3(),
        Environment::Dungeon,
        &areas,
        &existing_zones,
    );

    assert_eq!(zone_by_cell["0,0"], 2); // Outdoor, per the existing assignment
    assert_eq!(zone_by_cell["1,0"], 1); // Dungeon, per the existing assignment
}

#[test]
fn with_existing_zones_defaults_unknown_environments_to_zone_one() {
    // Forest never appears in `existing_zones`, so every Forest cell falls
    // back to zone 1, matching TS's `zoneIndex.get(env) ?? 1`.
    let existing_zones = [Environment::Dungeon];
    let areas = [area(0, 0, 0, 0, Environment::Forest)];
    let zone_by_cell = build_env_zone_map_with_existing_zones(
        &grid_3x3(),
        Environment::Dungeon,
        &areas,
        &existing_zones,
    );

    assert_eq!(zone_by_cell["0,0"], 1); // Forest, defaulted
    assert_eq!(zone_by_cell["1,0"], 1); // Dungeon, the real zone 1
}

#[test]
fn with_existing_zones_matches_grid_dimensions_of_a_differently_shaped_layer() {
    // A layer's own grid can be a different size than the one that
    // established the zone list — the function only needs a grid and areas,
    // no cross-layer coupling beyond the zone-index list.
    let wide_grid = grid(&["....", "...."]);
    let zone_by_cell = build_env_zone_map_with_existing_zones(
        &wide_grid,
        Environment::Dungeon,
        &[],
        &[Environment::Dungeon],
    );
    assert_eq!(zone_by_cell.len(), 8);
    assert!(zone_by_cell.values().all(|&zone| zone == 1));
}

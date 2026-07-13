//! `rendering/forestRenderer.ts` has no dedicated vitest suite — these are
//! this port's own parity spec. Pins exact seeded output (not just counts)
//! against real forest cells from the shipped `forest_test.json`, plus
//! synthetic-grid edge cases exercising the char-selection gate directly.

use delve_core::forest_placement::compute_forest_placements;
use delve_core::level_loader::{ValidationContext, validate_dungeon_str};
use delve_core::types::CharDef;

const FOREST_TEST_JSON: &str = include_str!("../../../assets/levels/forest_test.json");
const CELL_SIZE: f64 = 2.0;

fn rows(lines: &[&str]) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

fn forest_clearing_level() -> delve_core::types::DungeonLevel {
    let mut warnings = Vec::new();
    let dungeon = validate_dungeon_str(
        FOREST_TEST_JSON,
        "forest_test.json",
        &ValidationContext::default(),
        &mut warnings,
    )
    .expect("shipped forest_test.json validates");
    dungeon
        .levels
        .into_iter()
        .next()
        .expect("forest_test.json has a level")
}

#[test]
fn shipped_forest_test_json_grid_and_chardefs_match_expectations() {
    let level = forest_clearing_level();
    let grid = &level.layers[0].grid;
    assert_eq!(grid[0], "FFFFFFFFFFFF");
    assert_eq!(grid[1], "FTTTTTTTTTTF");
    assert_eq!(grid.len(), 10);
    let char_defs = level.char_defs.as_deref().unwrap_or(&[]);
    let t_def = char_defs
        .iter()
        .find(|def| def.character == 'T')
        .expect("T charDef present");
    assert!(t_def.solid);
    assert_eq!(t_def.see_through, Some(true));
    assert_eq!(t_def.textures.wall_texture.as_deref(), Some("forest"));
    let f_def = char_defs
        .iter()
        .find(|def| def.character == 'F')
        .expect("F charDef present");
    assert!(f_def.solid);
    assert_eq!(f_def.see_through, None);
    assert_eq!(f_def.textures.wall_texture.as_deref(), Some("forest"));
}

/// Pins the exact seeded output for cell (col=1, row=1) — the top-left `T`
/// in `forest_test.json`'s grid, and the first forest-fill cell
/// `compute_forest_placements` visits (row 0 is all `F`; row 1 col 0 is
/// `F`; col 1 is the first `T`) — so its tree(s) are exactly the leading
/// entries of the returned Vec. `seed = 1*9173 + 1*5381 = 14554` draws a
/// `tree_count` of 1 for this specific seed (confirmed: the second Vec
/// entry's `x` already falls in the next cell's jitter range). Values
/// below are this module's own computed output; trusted because
/// `Mulberry32` is independently verified bit-exact against the TS runtime
/// (`random.rs`'s `matches_js_reference_seed_*` tests) — pinning here
/// checks this module's call sequence and formulas, not the RNG itself.
#[test]
fn pins_exact_output_for_a_real_forest_cell() {
    let level = forest_clearing_level();
    let grid = &level.layers[0].grid;
    let char_defs = level.char_defs.as_deref().unwrap_or(&[]);

    let placements = compute_forest_placements(grid, char_defs, CELL_SIZE);
    let tree = placements
        .first()
        .expect("cell (1,1) has at least one tree");

    assert_eq!(tree.variant_index, 1);
    assert!((tree.x - 3.740_554_943_028_837_6).abs() < 1e-12);
    assert!((tree.y - 1.5).abs() < 1e-12);
    assert!((tree.z - 3.351_224_432_559_684).abs() < 1e-12);

    // cx = col*CELL_SIZE + CELL_SIZE/2 = 1*2 + 1 = 3.0, same for cz — the
    // second placement belongs to the next forest-fill cell (col=2, row=1),
    // confirming cell (1,1) drew exactly one tree for this seed.
    let next = &placements[1];
    assert!((next.x - 3.0).abs() > 0.85 || (next.z - 3.0).abs() > 0.85);
}

#[test]
fn empty_grid_produces_no_placements() {
    let grid = rows(&["....", "....", "....", "...."]);
    let char_defs: Vec<CharDef> = Vec::new();
    let placements = compute_forest_placements(&grid, &char_defs, CELL_SIZE);
    assert!(placements.is_empty());
}

#[test]
fn grid_with_no_forest_fill_chars_produces_no_placements() {
    let grid = rows(&["####", "#..#", "#..#", "####"]);
    let char_defs = vec![simple_char_def('#', true, None, None)];
    let placements = compute_forest_placements(&grid, &char_defs, CELL_SIZE);
    assert!(placements.is_empty());
}

/// The disabled-border-pass case: `solid && wallTexture == "forest"` but
/// NOT `seeThrough` — TS builds this into `forestChars`, never consults it.
#[test]
fn solid_forest_wall_without_see_through_gets_no_trees() {
    let grid = rows(&["F"]);
    let char_defs = vec![simple_char_def('F', true, None, Some("forest"))];
    let placements = compute_forest_placements(&grid, &char_defs, CELL_SIZE);
    assert!(placements.is_empty());
}

#[test]
fn see_through_wall_with_non_forest_texture_gets_no_trees() {
    let grid = rows(&["T"]);
    let char_defs = vec![simple_char_def('T', true, Some(true), Some("stone"))];
    let placements = compute_forest_placements(&grid, &char_defs, CELL_SIZE);
    assert!(placements.is_empty());
}

#[test]
fn non_solid_see_through_forest_char_gets_no_trees() {
    let grid = rows(&["T"]);
    let char_defs = vec![simple_char_def('T', false, Some(true), Some("forest"))];
    let placements = compute_forest_placements(&grid, &char_defs, CELL_SIZE);
    assert!(placements.is_empty());
}

#[test]
fn solid_see_through_forest_char_gets_one_to_three_trees() {
    let grid = rows(&["T"]);
    let char_defs = vec![simple_char_def('T', true, Some(true), Some("forest"))];
    let placements = compute_forest_placements(&grid, &char_defs, CELL_SIZE);
    assert!((1..=3).contains(&placements.len()));
    for placement in &placements {
        assert!(placement.variant_index <= 3);
    }
}

fn simple_char_def(
    character: char,
    solid: bool,
    see_through: Option<bool>,
    wall_texture: Option<&str>,
) -> CharDef {
    let json = serde_json::json!({
        "char": character.to_string(),
        "solid": solid,
        "seeThrough": see_through,
        "wallTexture": wall_texture,
    });
    serde_json::from_value(json).expect("synthetic CharDef parses")
}

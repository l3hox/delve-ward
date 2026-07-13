//! Deterministic forest tree placement, ported from `rendering/forestRenderer.ts`'s
//! `buildForestMeshes` — the pure cell-selection and per-cell RNG-draw math
//! only. Sprite loading, `InstancedMesh` construction, and the per-frame
//! camera-facing rotation (`updateForestBillboards`) are delve-game/
//! rendering concerns and stay out of this module; `forest.rs` consumes
//! [`compute_forest_placements`]'s plain [`TreePlacement`] structs.
//!
//! TS builds two char sets: `forestChars` (solid, `wallTexture == "forest"`,
//! not `seeThrough`) and `forestFillChars` (the same, but `seeThrough`).
//! `forestChars` would feed a "border pass" that places trees on walkable
//! cells adjacent to a forest cell — that pass is disabled in TS behind a
//! `// TODO: Border pass disabled` comment and never runs. Only
//! `forestFillChars` cells ever get trees today. Ported to match that live
//! behavior exactly: `forestChars` isn't reproduced here at all, since
//! nothing reachable ever consults it.

use crate::random::Mulberry32;
use crate::types::CharDef;
use std::collections::HashSet;

const FILL_PADDING: f64 = 0.15;
const FILL_TREE_COUNT_MIN: u32 = 1;
const FILL_TREE_COUNT_MAX: u32 = 3;

/// Ported from TS's `VARIANT_SPECS` array — index order matters (it's what
/// `variant_index` selects into) but only each variant's `height` affects
/// placement math (`wy = spec.height / 2`). Sprite paths and widths are a
/// rendering concern; `forest.rs` keeps its own table indexed the same way.
const VARIANT_HEIGHTS: [f64; 4] = [2.85, 3.0, 2.7, 2.1];

/// One tree's world-space placement. `variant_index` is TS's index into
/// `VARIANT_SPECS` (0=oak-thin, 1=oak, 2=birch, 3=bushes) — `forest.rs`
/// looks up its own sprite/width table with the same index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreePlacement {
    pub variant_index: usize,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// TS: `def.solid && def.wallTexture === 'forest'` gates both char sets;
/// `def.seeThrough` splits into `forestFillChars` (this predicate) versus
/// the unused `forestChars`.
fn is_forest_fill_char(def: &CharDef) -> bool {
    def.solid
        && def.see_through == Some(true)
        && def.textures.wall_texture.as_deref() == Some("forest")
}

fn forest_fill_chars(char_defs: &[CharDef]) -> HashSet<char> {
    char_defs
        .iter()
        .filter(|def| is_forest_fill_char(def))
        .map(|def| def.character)
        .collect()
}

/// Every forest-fill cell's tree placements, in the same row-major
/// `(row, col)` traversal order TS's own double loop uses — a caller that
/// needs TS's per-variant instance ordering (`variantPositions[i].push`)
/// can filter this list by `variant_index` and get the identical order back,
/// since within-cell tree order (and cell traversal order) is preserved.
///
/// `cell_size` is injected rather than assumed, matching how every other
/// delve-core module stays dimension-agnostic and leaves world-unit
/// constants (`CELL_SIZE`) to delve-game.
#[must_use]
pub fn compute_forest_placements(
    grid: &[String],
    char_defs: &[CharDef],
    cell_size: f64,
) -> Vec<TreePlacement> {
    let fill_chars = forest_fill_chars(char_defs);
    if fill_chars.is_empty() {
        return Vec::new();
    }

    let half_cell = cell_size / 2.0 - FILL_PADDING;
    let mut placements = Vec::new();

    for (row, line) in grid.iter().enumerate() {
        for (col, character) in line.chars().enumerate() {
            if !fill_chars.contains(&character) {
                continue;
            }

            let cx = col as f64 * cell_size + cell_size / 2.0;
            let cz = row as f64 * cell_size + cell_size / 2.0;
            let seed = col as i64 * 9173 + row as i64 * 5381;
            let mut rng = Mulberry32::new(seed as u32);

            let tree_range = f64::from(FILL_TREE_COUNT_MAX - FILL_TREE_COUNT_MIN + 1);
            let tree_count = FILL_TREE_COUNT_MIN + (rng.next_f64() * tree_range).floor() as u32;

            for _ in 0..tree_count {
                let variant_index =
                    (rng.next_f64() * VARIANT_HEIGHTS.len() as f64).floor() as usize;
                let wx = cx + (rng.next_f64() * 2.0 - 1.0) * half_cell;
                let wz = cz + (rng.next_f64() * 2.0 - 1.0) * half_cell;
                let wy = VARIANT_HEIGHTS[variant_index] / 2.0;
                placements.push(TreePlacement {
                    variant_index,
                    x: wx,
                    y: wy,
                    z: wz,
                });
            }
        }
    }

    placements
}

//! Ramp geometry, ported from the TS `rampRenderer`: a single-cell-span
//! connector from a layer's floor up to the layer above's floor, in two
//! styles — a smooth sloped quad (`RampStyle::Ramp`) or an 8-step staircase
//! (`RampStyle::Stairs`).
//!
//! Each side of the ramp is a triangular fill under the slope plus,
//! conditionally, a full-height rectangle covering the far half of the top
//! cell — TS's `buildTriangularSide`. That rectangle only renders on a side
//! whose top-cell neighbor is actually solid (`isWallAt`, ported here as
//! [`is_wall_at`]), textured from the *top* cell's resolved wall texture,
//! not the ramp's own base cell — the ramp's base-cell texture only feeds
//! the slope/tread surface. A neighbor that's itself another ramp's top
//! cell never counts as solid (`rampTopCells` in TS, `ramp_top_cells` here)
//! — two ramps landing on adjacent cells don't wall each other off.
//!
//! `dungeon.rs`'s own per-cell pass already handles the ramp's *base*
//! cell — full ceiling and the wall in the ramp's own facing direction both
//! skipped there (`spawn_dungeon`'s `ramp_base_cells` param) — matching TS's
//! `mergeRampCell(doorKey(ramp.col, ramp.row), { wallDirs: [ramp.facing],
//! skipCeiling: true, skipFloor: false })`. This module's own base-cell
//! character lookup already relies on the base cell being non-renderable in
//! that pass to source the ramp's own textures.
//!
//! **Deliberately not ported — needs half-tile geometry `dungeon.rs` has no
//! primitive for yet** (verified this doesn't block the three behaviors
//! above, which are entirely within this module and the ramp's own top
//! cell): TS's *second* `mergeRampCell` call in `sceneUtils.ts::buildRampInfo`
//! (the top cell's own `wallDirs`/`keepHalf`, halving its wall facing back
//! down the ramp rather than skipping it outright), the `rampHalfWalls` map
//! (halving neighbors' walls that face a ramp's top cell), and the
//! layer-above's half-floor patch at the landing (`buildRampInfo`'s
//! `if (li > 0)` peek). All three are cosmetic — avoiding a wall or floor
//! visually clipping through the ramp's rising geometry right at the
//! landing — not gameplay-relevant, and remain a disclosed, bounded scope
//! decision rather than a silent gap.

use crate::dungeon::{CELL_SIZE, LAYER_HEIGHT, LayerSpawn, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use crate::zones::{self, LevelZones};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, RampStyle};
use delve_core::grid::{Facing, build_walkable_set};
use delve_core::texture_resolver::resolve_textures;
use std::collections::HashSet;

const RAMP_STEP_COUNT: usize = 8;
const RAMP_STEP_HEIGHT: f32 = LAYER_HEIGHT / RAMP_STEP_COUNT as f32;
const RAMP_STEP_DEPTH: f32 = CELL_SIZE / RAMP_STEP_COUNT as f32;

/// Canonical orientation: bottom cell at +Z (south), top cell at -Z (north)
/// — `facing = N` needs no rotation, matching TS's `FACING_ROTATION`.
fn facing_rotation(facing: Facing) -> f32 {
    match facing {
        Facing::N => 0.0,
        Facing::E => -std::f32::consts::FRAC_PI_2,
        Facing::S => std::f32::consts::PI,
        Facing::W => std::f32::consts::FRAC_PI_2,
    }
}

/// `(left_offset, right_offset)` from the top cell — TS's
/// `TOP_CELL_SIDE_OFFSETS`. Canonical orientation puts left at -X, right at
/// +X; each facing rotates which grid direction that corresponds to.
fn top_cell_side_offsets(facing: Facing) -> ((i32, i32), (i32, i32)) {
    match facing {
        Facing::N => ((-1, 0), (1, 0)),
        Facing::S => ((1, 0), (-1, 0)),
        Facing::E => ((0, -1), (0, 1)),
        Facing::W => ((0, 1), (0, -1)),
    }
}

/// TS's `isWallAt` (`rampRenderer.ts:419-424`): whether `(col, row)` should
/// render as solid for a ramp's own side-wall decision. Out-of-bounds
/// (either axis) counts as solid, matching TS's row/col range checks; a
/// cell that's another ramp's own top cell never counts as solid even if
/// its character isn't in `walkable` — TS's inline comment: "adjacent ramp
/// top cell — not a wall".
fn is_wall_at(
    grid: &[String],
    walkable: &HashSet<char>,
    col: i32,
    row: i32,
    ramp_top_cells: &HashSet<(i32, i32)>,
) -> bool {
    let Ok(row_index) = usize::try_from(row) else {
        return true;
    };
    let Some(line) = grid.get(row_index) else {
        return true;
    };
    let Ok(col_index) = usize::try_from(col) else {
        return true;
    };
    let Some(character) = line.chars().nth(col_index) else {
        return true;
    };
    if ramp_top_cells.contains(&(col, row)) {
        return false;
    }
    !walkable.contains(&character)
}

/// Appends a quad (two triangles, `a-b-c` and `a-c-d`) to a raw
/// `TriangleList` vertex buffer — no index buffer, matching TS's own
/// `pushQuad` helper this mirrors. Passing `c == d` (or `b == c`) collapses
/// the second (or first) triangle to zero area, the same degenerate-quad
/// trick TS uses to build a triangle without a separate code path.
#[allow(clippy::too_many_arguments)]
fn push_quad(
    positions: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    normals: &mut Vec<[f32; 3]>,
    corners: [[f32; 3]; 4],
    uv_corners: [[f32; 2]; 4],
    normal: [f32; 3],
) {
    let [a, b, c, d] = corners;
    let [uv_a, uv_b, uv_c, uv_d] = uv_corners;
    positions.extend([a, b, c, a, c, d]);
    uvs.extend([uv_a, uv_b, uv_c, uv_a, uv_c, uv_d]);
    normals.extend([normal; 6]);
}

fn build_mesh(positions: Vec<[f32; 3]>, uvs: Vec<[f32; 2]>, normals: Vec<[f32; 3]>) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
}

/// The sloped surface, canonical orientation — ported from TS's
/// `buildSmoothRamp`'s `slopeVerts`/`slopeUVs`/`slopeNorms`.
fn build_slope_mesh() -> Mesh {
    let half = CELL_SIZE / 2.0;
    let slope_length = (CELL_SIZE * CELL_SIZE + LAYER_HEIGHT * LAYER_HEIGHT).sqrt();
    let v_scale = slope_length / CELL_SIZE;
    let normal = [0.0, CELL_SIZE / slope_length, LAYER_HEIGHT / slope_length];

    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut normals = Vec::new();
    push_quad(
        &mut positions,
        &mut uvs,
        &mut normals,
        [
            [-half, 0.0, half],
            [half, 0.0, half],
            [half, LAYER_HEIGHT, -half],
            [-half, LAYER_HEIGHT, -half],
        ],
        [[0.0, 0.0], [1.0, 0.0], [1.0, v_scale], [0.0, v_scale]],
        normal,
    );
    build_mesh(positions, uvs, normals)
}

/// Triangular fill under the slope on one side (`x = -half` or `+half`)
/// plus, when `include_top_cell_wall` is set, a full-height rectangle
/// covering the far half of the top cell beyond it — TS's
/// `buildTriangularSide`, including its conditional "far half of the top
/// cell" rectangle (previously dropped here; see the module doc comment for
/// what still is).
fn build_side_fill_mesh(x: f32, include_top_cell_wall: bool) -> Mesh {
    let half = CELL_SIZE / 2.0;
    let nx = if x < 0.0 { -1.0 } else { 1.0 };
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut normals = Vec::new();
    if x < 0.0 {
        push_quad(
            &mut positions,
            &mut uvs,
            &mut normals,
            [
                [x, 0.0, half],
                [x, LAYER_HEIGHT, -half],
                [x, LAYER_HEIGHT, -half],
                [x, 0.0, -half],
            ],
            [[0.0, 0.0], [1.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            [nx, 0.0, 0.0],
        );
    } else {
        push_quad(
            &mut positions,
            &mut uvs,
            &mut normals,
            [
                [x, 0.0, half],
                [x, 0.0, -half],
                [x, LAYER_HEIGHT, -half],
                [x, LAYER_HEIGHT, -half],
            ],
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [1.0, 1.0]],
            [nx, 0.0, 0.0],
        );
    }
    if include_top_cell_wall {
        let z_near = 0.0;
        let z_far = -CELL_SIZE;
        if x < 0.0 {
            push_quad(
                &mut positions,
                &mut uvs,
                &mut normals,
                [
                    [x, 0.0, z_near],
                    [x, WALL_HEIGHT, z_near],
                    [x, WALL_HEIGHT, z_far],
                    [x, 0.0, z_far],
                ],
                [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
                [nx, 0.0, 0.0],
            );
        } else {
            push_quad(
                &mut positions,
                &mut uvs,
                &mut normals,
                [
                    [x, 0.0, z_far],
                    [x, WALL_HEIGHT, z_far],
                    [x, WALL_HEIGHT, z_near],
                    [x, 0.0, z_near],
                ],
                [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]],
                [nx, 0.0, 0.0],
            );
        }
    }
    build_mesh(positions, uvs, normals)
}

#[allow(clippy::too_many_arguments)]
fn spawn_smooth_ramp(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    parent: Entity,
    slope_material: Handle<StandardMaterial>,
    side_material: Handle<StandardMaterial>,
    has_left_top_wall: bool,
    has_right_top_wall: bool,
) -> Vec<Entity> {
    let slope = commands
        .spawn((
            Mesh3d(meshes.add(build_slope_mesh())),
            MeshMaterial3d(slope_material),
            Transform::IDENTITY,
        ))
        .id();
    commands.entity(parent).add_child(slope);
    let mut pieces = vec![slope];

    for (x, include_top_cell_wall) in [
        (-CELL_SIZE / 2.0, has_left_top_wall),
        (CELL_SIZE / 2.0, has_right_top_wall),
    ] {
        let side = commands
            .spawn((
                Mesh3d(meshes.add(build_side_fill_mesh(x, include_top_cell_wall))),
                MeshMaterial3d(side_material.clone()),
                Transform::IDENTITY,
            ))
            .id();
        commands.entity(parent).add_child(side);
        pieces.push(side);
    }
    pieces
}

/// Stepped treads as stacked ascending boxes, mirroring `stairs.rs`'s own
/// box-primitive convention (rather than TS's fully custom tread/riser
/// vertex buffers) — full `CELL_SIZE` width throughout, matching TS's own
/// ramp-stairs treads exactly (unlike `stairs.rs`'s *narrower* stair treads,
/// which is a different, pre-existing simplification specific to stairs).
#[allow(clippy::too_many_arguments)]
fn spawn_stepped_ramp(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    parent: Entity,
    step_material: Handle<StandardMaterial>,
    side_material: Handle<StandardMaterial>,
    has_left_top_wall: bool,
    has_right_top_wall: bool,
) -> Vec<Entity> {
    let half = CELL_SIZE / 2.0;
    let mut pieces = Vec::with_capacity(RAMP_STEP_COUNT + 2);
    for step in 0..RAMP_STEP_COUNT {
        let step_z = half - RAMP_STEP_DEPTH / 2.0 - step as f32 * RAMP_STEP_DEPTH;
        let height = (step as f32 + 1.0) * RAMP_STEP_HEIGHT;
        let tread = commands
            .spawn((
                Mesh3d(meshes.add(Cuboid::new(CELL_SIZE, height, RAMP_STEP_DEPTH))),
                MeshMaterial3d(step_material.clone()),
                Transform::from_xyz(0.0, height / 2.0, step_z),
            ))
            .id();
        commands.entity(parent).add_child(tread);
        pieces.push(tread);
    }
    for (x, include_top_cell_wall) in [
        (-CELL_SIZE / 2.0, has_left_top_wall),
        (CELL_SIZE / 2.0, has_right_top_wall),
    ] {
        let side = commands
            .spawn((
                Mesh3d(meshes.add(build_side_fill_mesh(x, include_top_cell_wall))),
                MeshMaterial3d(side_material.clone()),
                Transform::IDENTITY,
            ))
            .id();
        commands.entity(parent).add_child(side);
        pieces.push(side);
    }
    pieces
}

/// Spawns every ramp on `layer_state`'s own layer — ported from
/// `rampRenderer.ts::buildRampMeshes`/`buildSingleRamp`.
///
/// Ported faithfully: the sloped/stepped ramp mesh itself, positioned at the
/// midpoint between the bottom and top cell centers and rotated to the
/// ramp's facing (`buildSingleRamp`'s exact placement math); each side's
/// conditional top-cell wall panel, included only where that side's
/// neighbor is actually solid and textured from the top cell; the ramp's
/// own base cell getting neither a ceiling nor a wall in its facing
/// direction, handled by `dungeon.rs`'s `spawn_dungeon` via its
/// `ramp_base_cells` param rather than here.
///
/// Deliberately not ported — see the module doc comment for the half-tile
/// boundary: the top cell's own half-wall facing back down the ramp, and
/// the layer-above's half-floor patch at the landing spot.
pub fn spawn_ramps(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer_spawn: &LayerSpawn,
    layer_state: &LayerState,
    zones: &LevelZones,
) {
    let (layer_defaults, layer_areas) = layer_spawn.texture_style();
    let grid = &layer_spawn.layer_def.grid;
    let char_defs = layer_spawn.level.char_defs.as_deref();
    let walkable = build_walkable_set(
        char_defs
            .into_iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );

    // Every ramp's top cell, collected once — TS's own pre-loop pass in
    // `buildRampMeshes`, so a ramp's side-wall decision about a neighbor
    // that happens to be a *different* ramp's landing never treats that
    // neighbor as solid.
    let ramp_top_cells: HashSet<(i32, i32)> = layer_state
        .ramps
        .values()
        .map(|ramp| {
            let (dcol, drow) = ramp.facing.delta();
            (ramp.col as i32 + dcol, ramp.row as i32 + drow)
        })
        .collect();

    for ramp in layer_state.ramps.values() {
        let character = grid
            .get(ramp.row as usize)
            .and_then(|line| line.chars().nth(ramp.col as usize))
            .unwrap_or('.');
        let textures = resolve_textures(
            ramp.col as i32,
            ramp.row as i32,
            character,
            layer_defaults,
            char_defs,
            layer_areas,
        );
        let step_material = materials.floor(&textures.floor);

        let (dcol, drow) = ramp.facing.delta();
        let top_col = ramp.col as i32 + dcol;
        let top_row = ramp.row as i32 + drow;
        let (left_offset, right_offset) = top_cell_side_offsets(ramp.facing);
        let has_left_top_wall = is_wall_at(
            grid,
            &walkable,
            top_col + left_offset.0,
            top_row + left_offset.1,
            &ramp_top_cells,
        );
        let has_right_top_wall = is_wall_at(
            grid,
            &walkable,
            top_col + right_offset.0,
            top_row + right_offset.1,
            &ramp_top_cells,
        );

        // Side-fill texture comes from the top cell, not the ramp's own
        // base cell — matches the half-walls `dungeon.rs` would render for
        // that solid cell if the ramp weren't carving it open.
        let top_character = grid
            .get(top_row as usize)
            .and_then(|line| line.chars().nth(top_col as usize))
            .unwrap_or('#');
        let top_textures = resolve_textures(
            top_col,
            top_row,
            top_character,
            layer_defaults,
            char_defs,
            layer_areas,
        );
        let side_material = materials.wall_repeat(&top_textures.wall);

        let center_x = ramp.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = ramp.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let (offset_x, offset_z) = match ramp.facing {
            Facing::N => (0.0, -CELL_SIZE / 2.0),
            Facing::S => (0.0, CELL_SIZE / 2.0),
            Facing::E => (CELL_SIZE / 2.0, 0.0),
            Facing::W => (-CELL_SIZE / 2.0, 0.0),
        };

        let parent = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(
                    center_x + offset_x,
                    layer_spawn.y_offset,
                    center_z + offset_z,
                )
                .with_rotation(Quat::from_rotation_y(facing_rotation(ramp.facing))),
                Visibility::default(),
            ))
            .id();

        let pieces = match ramp.style {
            RampStyle::Ramp => spawn_smooth_ramp(
                commands,
                meshes,
                parent,
                step_material,
                side_material,
                has_left_top_wall,
                has_right_top_wall,
            ),
            RampStyle::Stairs => spawn_stepped_ramp(
                commands,
                meshes,
                parent,
                step_material,
                side_material,
                has_left_top_wall,
                has_right_top_wall,
            ),
        };
        for piece in pieces {
            zones::tag_cell(
                commands,
                zones,
                layer_spawn.index,
                piece,
                ramp.col,
                ramp.row,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> Vec<String> {
        rows.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn is_wall_at_is_true_out_of_bounds() {
        let grid = grid(&["##", "##"]);
        let walkable = HashSet::from(['.']);
        let ramp_top_cells = HashSet::new();
        assert!(is_wall_at(&grid, &walkable, -1, 0, &ramp_top_cells));
        assert!(is_wall_at(&grid, &walkable, 0, -1, &ramp_top_cells));
        assert!(is_wall_at(&grid, &walkable, 5, 0, &ramp_top_cells));
        assert!(is_wall_at(&grid, &walkable, 0, 5, &ramp_top_cells));
    }

    #[test]
    fn is_wall_at_reads_the_walkable_set() {
        let grid = grid(&["#."]);
        let walkable = HashSet::from(['.']);
        let ramp_top_cells = HashSet::new();
        assert!(is_wall_at(&grid, &walkable, 0, 0, &ramp_top_cells));
        assert!(!is_wall_at(&grid, &walkable, 1, 0, &ramp_top_cells));
    }

    #[test]
    fn is_wall_at_excludes_another_ramps_top_cell_even_when_solid() {
        let grid = grid(&["#"]);
        let walkable = HashSet::from(['.']);
        let ramp_top_cells = HashSet::from([(0, 0)]);
        assert!(!is_wall_at(&grid, &walkable, 0, 0, &ramp_top_cells));
    }

    #[test]
    fn top_cell_side_offsets_are_perpendicular_to_facing() {
        assert_eq!(top_cell_side_offsets(Facing::N), ((-1, 0), (1, 0)));
        assert_eq!(top_cell_side_offsets(Facing::S), ((1, 0), (-1, 0)));
        assert_eq!(top_cell_side_offsets(Facing::E), ((0, -1), (0, 1)));
        assert_eq!(top_cell_side_offsets(Facing::W), ((0, 1), (0, -1)));
    }
}

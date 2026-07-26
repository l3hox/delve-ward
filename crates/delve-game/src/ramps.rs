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
//! Every cell a ramp touches other than the ramp mesh itself is built by
//! `dungeon.rs`'s ordinary per-cell pass, from the [`RampCellInfo`] map
//! [`build_ramp_info`] produces here — the same split TS draws between
//! `sceneUtils.ts::buildRampInfo` and the geometry decisions
//! `rendering/dungeon.ts` makes from its output.

use crate::dungeon::{CELL_SIZE, LAYER_HEIGHT, LayerSpawn, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use crate::zones::{self, LevelZones};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, RampInstance, RampStyle, door_key};
use delve_core::grid::{Facing, build_walkable_set};
use delve_core::texture_resolver::resolve_textures;
use std::collections::{HashMap, HashSet};

const RAMP_STEP_COUNT: usize = 8;
const RAMP_STEP_HEIGHT: f32 = LAYER_HEIGHT / RAMP_STEP_COUNT as f32;
const RAMP_STEP_DEPTH: f32 = CELL_SIZE / RAMP_STEP_COUNT as f32;

fn opposite(facing: Facing) -> Facing {
    match facing {
        Facing::N => Facing::S,
        Facing::S => Facing::N,
        Facing::E => Facing::W,
        Facing::W => Facing::E,
    }
}

/// The cell a ramp lands in: one step along its facing from its base cell.
fn landing_cell(ramp: &RampInstance) -> (i64, i64) {
    let (dcol, drow) = ramp.facing.delta();
    (ramp.col + i64::from(dcol), ramp.row + i64::from(drow))
}

/// How one cell's floor, ceiling, and walls deviate from ordinary geometry
/// because a ramp reaches it — TS's `RampCellInfo`
/// (`rendering/dungeon.ts:98-106`) with its sibling `rampHalfWalls` map
/// (`:109-112`) folded into the same record. TS keys that map separately only
/// because the cells it names are the landing's lateral *neighbours* rather
/// than the ramp's own cells; a record carrying nothing but an override reads
/// identically to no record at all at every one of `spawn_dungeon`'s use
/// sites.
///
/// TS's `skipFloor` has no counterpart here: all three of `buildRampInfo`'s
/// merge calls pass it `false` and the merge only ever ORs, so nothing can
/// ever set it.
#[derive(Default)]
pub struct RampCellInfo {
    /// Wall faces this cell does not build at all — TS's `wallDirs`.
    pub suppressed_walls: Vec<Facing>,
    pub skip_ceiling: bool,
    /// The floor builds only the half lying this way — TS's `floorKeepHalf`.
    pub keep_floor_half: Option<Facing>,
    /// The two wall faces perpendicular to this direction build only the half
    /// lying this way — TS's `keepHalf`.
    pub keep_wall_half: Option<Facing>,
    /// Per-face override of [`Self::keep_wall_half`], applied ahead of it and
    /// without its perpendicularity rule — TS's `rampHalfWalls` entries.
    pub wall_half_overrides: HashMap<Facing, Facing>,
}

impl RampCellInfo {
    /// Which half of `face`'s wall survives, when the wall is halved at all.
    /// TS reads the per-face override first and falls back to the cell-wide
    /// keep, which applies only to the two faces perpendicular to it — an
    /// east or west keep halves the north and south faces, a north or south
    /// keep the east and west ones
    /// (`rendering/dungeon.ts:396-399,409-412,422-425,435-438`).
    pub fn wall_half(&self, face: Facing) -> Option<Facing> {
        if let Some(keep) = self.wall_half_overrides.get(&face) {
            return Some(*keep);
        }
        let keep = self.keep_wall_half?;
        let perpendicular = match face {
            Facing::N | Facing::S => matches!(keep, Facing::E | Facing::W),
            Facing::E | Facing::W => matches!(keep, Facing::N | Facing::S),
        };
        perpendicular.then_some(keep)
    }
}

fn suppress_wall(cell: &mut RampCellInfo, face: Facing) {
    if !cell.suppressed_walls.contains(&face) {
        cell.suppressed_walls.push(face);
    }
}

/// Everything one layer's ordinary cell geometry owes to ramps — ported from
/// `rendering/sceneUtils.ts:46-116`'s `buildRampInfo`, in its order.
///
/// Three cell roles. A ramp's base cell loses its ceiling, which the ramp
/// rises through, and the wall it climbs out of. The cell it lands in loses
/// the wall facing back down the climb, and halves the two walls flanking
/// that climb so only the far half past the ramp mouth stands; the two cells
/// flanking the landing halve the wall they turn toward it for the same
/// reason. Those are all on the ramp's own layer.
///
/// The third role is not: a ramp climbs a full `LAYER_HEIGHT`, so it arrives
/// at the floor level of the layer *above* it. `ramps_on_layer_below` are the
/// ramps whose landing cells are cells of this layer, and that floor keeps
/// only the half ahead of the ramp — the near half is the hole the climb
/// arrives through, and a whole tile there would cap the ramp off.
///
/// Ramps iterate in whatever order their map yields, which decides only which
/// of two ramps landing in the same cell wins the half it keeps. TS resolves
/// that by insertion order; no shipped level has two ramps sharing a landing.
pub fn build_ramp_info<'a>(
    ramps_on_layer: impl IntoIterator<Item = &'a RampInstance>,
    ramps_on_layer_below: impl IntoIterator<Item = &'a RampInstance>,
) -> HashMap<String, RampCellInfo> {
    let own_layer: Vec<&RampInstance> = ramps_on_layer.into_iter().collect();
    let mut cells: HashMap<String, RampCellInfo> = HashMap::new();

    for ramp in &own_layer {
        let base = cells.entry(door_key(ramp.col, ramp.row)).or_default();
        suppress_wall(base, ramp.facing);
        base.skip_ceiling = true;

        let (landing_col, landing_row) = landing_cell(ramp);
        let landing = cells.entry(door_key(landing_col, landing_row)).or_default();
        suppress_wall(landing, opposite(ramp.facing));
        if landing.keep_wall_half.is_none() {
            landing.keep_wall_half = Some(ramp.facing);
        }
    }

    for ramp in ramps_on_layer_below {
        let (landing_col, landing_row) = landing_cell(ramp);
        let landing = cells.entry(door_key(landing_col, landing_row)).or_default();
        suppress_wall(landing, opposite(ramp.facing));
        if landing.keep_floor_half.is_none() {
            landing.keep_floor_half = Some(ramp.facing);
        }
    }

    for ramp in &own_layer {
        let (landing_col, landing_row) = landing_cell(ramp);
        // The two cells flanking the landing, and the face each turns toward
        // it — a climb running north or south is flanked east and west, and
        // the other way round for one running east or west.
        let flanks = if matches!(ramp.facing, Facing::N | Facing::S) {
            [
                ((landing_col + 1, landing_row), Facing::W),
                ((landing_col - 1, landing_row), Facing::E),
            ]
        } else {
            [
                ((landing_col, landing_row + 1), Facing::N),
                ((landing_col, landing_row - 1), Facing::S),
            ]
        };
        for ((col, row), face) in flanks {
            // TS's `Map.set` — a later ramp overwrites an earlier one here,
            // the opposite of the first-wins merge the fields above use.
            cells
                .entry(door_key(col, row))
                .or_default()
                .wall_half_overrides
                .insert(face, ramp.facing);
        }
    }

    cells
}

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
/// neighbor is actually solid and textured from the top cell.
///
/// Everything a ramp changes about ordinary cell geometry — the base cell's
/// missing ceiling and wall, the landing's half walls and half floor — comes
/// from [`build_ramp_info`] and is built by `dungeon.rs`, not here.
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

    fn ramp(col: i64, row: i64, facing: Facing) -> RampInstance {
        RampInstance {
            id: None,
            col,
            row,
            facing,
            style: RampStyle::Ramp,
        }
    }

    #[test]
    fn a_ramp_opens_its_own_cells_ceiling_and_the_wall_it_climbs_out_of() {
        let cells = build_ramp_info(&[ramp(4, 6, Facing::N)], &[]);
        let base = &cells[&door_key(4, 6)];
        assert_eq!(base.suppressed_walls, vec![Facing::N]);
        assert!(base.skip_ceiling);
        assert_eq!(base.keep_floor_half, None);
        assert_eq!(base.keep_wall_half, None);
    }

    /// On the ramp's own layer the landing keeps its floor and ceiling whole:
    /// what it loses is the wall facing back down the climb, and half of each
    /// wall beside it.
    #[test]
    fn a_landing_on_the_ramps_own_layer_keeps_the_wall_halves_past_the_climb() {
        let cells = build_ramp_info(&[ramp(4, 6, Facing::N)], &[]);
        let landing = &cells[&door_key(4, 5)];
        assert_eq!(landing.suppressed_walls, vec![Facing::S]);
        assert_eq!(landing.keep_wall_half, Some(Facing::N));
        assert!(!landing.skip_ceiling);
        assert_eq!(landing.keep_floor_half, None);
    }

    /// The override is always set on a face perpendicular to the half it
    /// keeps, which is what puts the surviving half in the wall's own plane.
    #[test]
    fn the_cells_flanking_a_landing_halve_the_wall_they_turn_toward_it() {
        let climbing_north = build_ramp_info(&[ramp(4, 6, Facing::N)], &[]);
        assert_eq!(
            climbing_north[&door_key(5, 5)].wall_half(Facing::W),
            Some(Facing::N)
        );
        assert_eq!(
            climbing_north[&door_key(3, 5)].wall_half(Facing::E),
            Some(Facing::N)
        );

        let climbing_east = build_ramp_info(&[ramp(4, 6, Facing::E)], &[]);
        assert_eq!(
            climbing_east[&door_key(5, 7)].wall_half(Facing::N),
            Some(Facing::E)
        );
        assert_eq!(
            climbing_east[&door_key(5, 5)].wall_half(Facing::S),
            Some(Facing::E)
        );
    }

    /// The half-floor patch belongs to the landing cell on the layer *above*
    /// the ramp, never to the ramp's base cell — the climb arrives at that
    /// layer's floor level halfway across the landing.
    #[test]
    fn a_ramp_from_the_layer_below_halves_the_floor_it_lands_on() {
        let cells = build_ramp_info(&[], &[ramp(4, 6, Facing::N)]);
        let landing = &cells[&door_key(4, 5)];
        assert_eq!(landing.keep_floor_half, Some(Facing::N));
        assert_eq!(landing.suppressed_walls, vec![Facing::S]);
        assert_eq!(
            landing.keep_wall_half, None,
            "only a ramp on this layer halves a landing's walls"
        );
        assert!(
            !cells.contains_key(&door_key(4, 6)),
            "the cell over the ramp's base is ordinary geometry"
        );
    }

    /// Every facing keeps the half it climbs toward, in the cell one step
    /// that way — the pairing that decides whether the hole is in front of
    /// the ramp mouth or behind it.
    #[test]
    fn each_facing_halves_the_landing_one_step_ahead_of_it() {
        for (facing, landing_col, landing_row) in [
            (Facing::N, 4, 5),
            (Facing::S, 4, 7),
            (Facing::E, 5, 6),
            (Facing::W, 3, 6),
        ] {
            let cells = build_ramp_info(&[], &[ramp(4, 6, facing)]);
            assert_eq!(
                cells[&door_key(landing_col, landing_row)].keep_floor_half,
                Some(facing),
            );
        }
    }

    /// `stairs.json` chains ramps: layer 1's ramp at (3,1) starts in the very
    /// cell layer 0's ramp at (2,1) lands in, so that cell is both a base and
    /// a landing and loses the walls at both ends of the climb.
    #[test]
    fn a_ramp_starting_where_the_one_below_lands_merges_both_roles() {
        let cells = build_ramp_info(&[ramp(3, 1, Facing::E)], &[ramp(2, 1, Facing::E)]);
        let shared = &cells[&door_key(3, 1)];
        assert!(shared.skip_ceiling);
        assert_eq!(shared.suppressed_walls, vec![Facing::E, Facing::W]);
        assert_eq!(shared.keep_floor_half, Some(Facing::E));
        assert_eq!(shared.keep_wall_half, None);
    }

    #[test]
    fn wall_half_prefers_a_per_face_override_over_the_cell_wide_keep() {
        let mut info = RampCellInfo {
            keep_wall_half: Some(Facing::N),
            ..Default::default()
        };
        info.wall_half_overrides.insert(Facing::E, Facing::S);
        assert_eq!(info.wall_half(Facing::E), Some(Facing::S));
        assert_eq!(info.wall_half(Facing::W), Some(Facing::N));
    }

    /// A keep direction halves only the walls it runs along; the wall it
    /// points at is either suppressed outright or stands whole.
    #[test]
    fn a_cell_wide_keep_halves_only_the_two_faces_perpendicular_to_it() {
        let keeping_north = RampCellInfo {
            keep_wall_half: Some(Facing::N),
            ..Default::default()
        };
        assert_eq!(keeping_north.wall_half(Facing::E), Some(Facing::N));
        assert_eq!(keeping_north.wall_half(Facing::W), Some(Facing::N));
        assert_eq!(keeping_north.wall_half(Facing::N), None);
        assert_eq!(keeping_north.wall_half(Facing::S), None);

        let keeping_east = RampCellInfo {
            keep_wall_half: Some(Facing::E),
            ..Default::default()
        };
        assert_eq!(keeping_east.wall_half(Facing::N), Some(Facing::E));
        assert_eq!(keeping_east.wall_half(Facing::S), Some(Facing::E));
        assert_eq!(keeping_east.wall_half(Facing::E), None);
        assert_eq!(keeping_east.wall_half(Facing::W), None);
    }

    #[test]
    fn a_cell_no_ramp_reaches_has_no_entry_at_all() {
        assert!(build_ramp_info(&[], &[]).is_empty());
    }
}

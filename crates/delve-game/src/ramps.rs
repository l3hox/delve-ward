//! Ramp geometry, ported from the TS `rampRenderer`: a single-cell-span
//! connector from a layer's floor up to the layer above's floor, in two
//! styles — a smooth sloped quad (`RampStyle::Ramp`) or an 8-step staircase
//! (`RampStyle::Stairs`).
//!
//! TS builds every piece (slope, triangular side fills, stepped
//! tread/riser/side-profile geometry) as fully custom hand-vertexed
//! `BufferGeometry`, and coordinates with `dungeon.ts`'s per-cell floor/wall
//! loop via `rampOpenCells`/`rampHalfWalls` maps to carve half-tiles and
//! half-walls out of the *neighboring* cells the ramp's mesh overlaps.
//! `dungeon.rs` has no half-tile primitive at all yet (environment-zone
//! boundary splitting, the other TS feature that needs one, hasn't landed
//! either) — building that shared infrastructure is out of this slice's
//! scope, so this port takes the same shape `stairs.rs` already
//! established: the ramp's own spawn function owns 100% of its cell's
//! geometry (here, just the ramp's own base cell — see [`spawn_ramps`]'s
//! doc comment for exactly what's ported vs deliberately simplified).
//!
//! Side walls and the triangular under-slope fill are simplified from TS's
//! exact hand-vertexed composite (triangle + neighboring half-cell
//! rectangle) to a single triangular fill sized to the slope alone —
//! visually equivalent for the slope's own footprint, just without TS's
//! extra far-half wall segment beyond the ramp into the top cell. Not
//! ported: the `rampHalfWalls` carve-outs in cells flanking the ramp's top
//! landing, and the top-layer's own half-floor patch where the ramp's top
//! surface meets it (`buildRampInfo`'s `if (li > 0)` peek in
//! `sceneUtils.ts`) — both are cosmetic-only (avoid a wall/floor visually
//! clipping through the ramp's rising geometry at the landing), not
//! gameplay-relevant. Flagged in the phase-5 report as a disclosed,
//! bounded scope decision, not a silent gap.

use crate::dungeon::{CELL_SIZE, LAYER_HEIGHT, LayerSpawn};
use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use crate::zones::{self, LevelZones};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, RampStyle};
use delve_core::grid::Facing;
use delve_core::texture_resolver::resolve_textures;

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

/// Triangular fill under the slope on one side (`x = -half` or `+half`) —
/// simplified from TS's `buildTriangularSide` (drops its extra "far half of
/// the top cell" rectangle; see the module doc comment).
fn build_side_fill_mesh(x: f32) -> Mesh {
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
    build_mesh(positions, uvs, normals)
}

fn spawn_smooth_ramp(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    parent: Entity,
    slope_material: Handle<StandardMaterial>,
    side_material: Handle<StandardMaterial>,
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

    for x in [-CELL_SIZE / 2.0, CELL_SIZE / 2.0] {
        let side = commands
            .spawn((
                Mesh3d(meshes.add(build_side_fill_mesh(x))),
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
fn spawn_stepped_ramp(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    parent: Entity,
    step_material: Handle<StandardMaterial>,
    side_material: Handle<StandardMaterial>,
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
    for x in [-CELL_SIZE / 2.0, CELL_SIZE / 2.0] {
        let side = commands
            .spawn((
                Mesh3d(meshes.add(build_side_fill_mesh(x))),
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
/// ramp's facing (`buildSingleRamp`'s exact placement math).
///
/// Deliberately simplified (see the module doc comment for why): the
/// triangular side fills drop TS's extra far-half-of-top-cell wall
/// rectangle; the ramp's own base cell still gets a full ceiling and a full
/// wall in its facing direction from `dungeon.rs`'s normal per-cell pass
/// (TS suppresses both there) — `spawn_dungeon`'s `ramp_base_cells` param
/// handles that suppression instead, see `dungeon.rs`; the top cell's own
/// half-floor/half-wall carve-outs and the layer-above's half-floor patch at
/// the landing spot are not ported at all.
pub fn spawn_ramps(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer_spawn: &LayerSpawn,
    layer_state: &LayerState,
    zones: &LevelZones,
) {
    let (layer_defaults, layer_areas) = layer_spawn.texture_style();
    for ramp in layer_state.ramps.values() {
        let character = layer_spawn
            .layer_def
            .grid
            .get(ramp.row as usize)
            .and_then(|line| line.chars().nth(ramp.col as usize))
            .unwrap_or('.');
        let textures = resolve_textures(
            ramp.col as i32,
            ramp.row as i32,
            character,
            layer_defaults,
            layer_spawn.level.char_defs.as_deref(),
            layer_areas,
        );
        let step_material = materials.floor(&textures.floor);
        let side_material = materials.wall_repeat(&textures.wall);

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
            RampStyle::Ramp => {
                spawn_smooth_ramp(commands, meshes, parent, step_material, side_material)
            }
            RampStyle::Stairs => {
                spawn_stepped_ramp(commands, meshes, parent, step_material, side_material)
            }
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

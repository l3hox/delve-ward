//! Stair rendering: stepped floor and ceiling slabs, side walls, and a black
//! back wall per stair cell, ported from the TS stairRenderer. Geometry is
//! built in canonical orientation (approach from south) and rotated by the
//! stair's facing; vertex colors fade to black toward the far end.

use crate::doors::fix_box_uvs;
use crate::dungeon::{CELL_SIZE, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, StairDirection};
use delve_core::grid::Facing;
use delve_core::texture_resolver::resolve_textures;
use std::collections::HashSet;
use std::f32::consts::{FRAC_PI_2, PI};

const STEP_COUNT: usize = 4;
const STEP_HEIGHT: f32 = 0.25;
const STEP_DEPTH: f32 = CELL_SIZE / STEP_COUNT as f32;
const STEP_WIDTH: f32 = CELL_SIZE * 0.85;
/// Fills the gap from step edge to cell edge.
const SIDE_WALL_THICKNESS: f32 = (CELL_SIZE - STEP_WIDTH) / 2.0;

/// Canonical geometry approaches from the south; rotate for other facings.
fn facing_rotation(facing: Facing) -> f32 {
    match facing {
        Facing::S => 0.0,
        Facing::E => FRAC_PI_2,
        Facing::N => PI,
        Facing::W => -FRAC_PI_2,
    }
}

/// Left/right neighbor offsets from the player's perspective looking into
/// the stairs (facing = approach direction).
fn side_offsets(facing: Facing) -> ((i64, i64), (i64, i64)) {
    match facing {
        Facing::S => ((-1, 0), (1, 0)),
        Facing::N => ((1, 0), (-1, 0)),
        Facing::E => ((0, -1), (0, 1)),
        Facing::W => ((0, 1), (0, -1)),
    }
}

fn cell_char(grid: &[String], col: i64, row: i64) -> Option<char> {
    let line = grid.get(usize::try_from(row).ok()?)?;
    line.chars().nth(usize::try_from(col).ok()?)
}

fn is_wall_neighbor(grid: &[String], walkable: &HashSet<char>, col: i64, row: i64) -> bool {
    cell_char(grid, col, row).is_none_or(|ch| !walkable.contains(&ch))
}

/// Vertex colors that fade to black with depth into the stairwell. `mesh_z`
/// is the mesh's position in group-local Z; +CELL_SIZE/2 (approach) is
/// bright, -CELL_SIZE/2 (far end) is dark.
fn apply_depth_fade(mesh: &mut Mesh, mesh_z: f32) {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return;
    };
    let half_cell = CELL_SIZE / 2.0;
    let colors: Vec<[f32; 4]> = positions
        .iter()
        .map(|position| {
            let brightness = ((mesh_z + position[2] + half_cell) / CELL_SIZE).clamp(0.0, 1.0);
            [brightness, brightness, brightness, 1.0]
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
}

/// Side walls span two floors: the wall texture repeats per WALL_HEIGHT
/// vertically and per CELL_SIZE horizontally. Corner-anchored like the TS
/// UV fix, so coursing aligns with the flat wall tiles.
fn fix_side_wall_uvs(mesh: &mut Mesh) {
    let positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return,
    };
    let normals: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return,
    };
    let half_extents = crate::doors::half_extents(&positions);
    let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
    else {
        return;
    };
    for (index, uv) in uvs.iter_mut().enumerate() {
        let normal = normals[index];
        let position = positions[index];
        let (u_axis, v_axis) = if normal[0].abs() > 0.5 {
            (2, 1) // ±x faces: U across depth, V across height
        } else if normal[1].abs() > 0.5 {
            (0, 2) // ±y faces: U across width, V across depth
        } else {
            (0, 1) // ±z faces: U across width, V across height
        };
        let v_reference = if v_axis == 1 { WALL_HEIGHT } else { CELL_SIZE };
        uv[0] = (position[u_axis] + half_extents[u_axis]) / CELL_SIZE;
        uv[1] = (position[v_axis] + half_extents[v_axis]) / v_reference;
    }
}

fn spawn_child(
    commands: &mut Commands,
    parent: Entity,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    transform: Transform,
) {
    let child = commands
        .spawn((Mesh3d(mesh), MeshMaterial3d(material), transform))
        .id();
    commands.entity(parent).add_child(child);
}

pub fn spawn_stairs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
    grid: &[String],
    walkable: &HashSet<char>,
) {
    let (layer_defaults, layer_areas) = layer_spawn.texture_style();
    let layer_y_offset = layer_spawn.y_offset;

    for stair in layer_state.stairs.values() {
        let (col, row) = (stair.col, stair.row);
        let character = cell_char(grid, col, row).unwrap_or('.');
        let textures = resolve_textures(
            col as i32,
            row as i32,
            character,
            layer_defaults,
            layer_spawn.level.char_defs.as_deref(),
            layer_areas,
        );
        let step_material = materials.floor(&textures.floor);
        let side_material = materials.wall_repeat(&textures.wall);
        let ceiling_material = materials.ceiling(&textures.ceiling);

        let (left, right) = side_offsets(stair.facing);
        let has_left_wall = is_wall_neighbor(grid, walkable, col + left.0, row + left.1);
        let has_right_wall = is_wall_neighbor(grid, walkable, col + right.0, row + right.1);
        let is_down = stair.direction == StairDirection::Down;

        let center_x = col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let parent = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(center_x, layer_y_offset, center_z)
                    .with_rotation(Quat::from_rotation_y(facing_rotation(stair.facing))),
                Visibility::default(),
            ))
            .id();

        for step in 0..STEP_COUNT {
            let step_z = CELL_SIZE / 2.0 - STEP_DEPTH / 2.0 - step as f32 * STEP_DEPTH;

            // Floor: descending thin slabs, or ascending slabs grown from y=0.
            let (height, center_y) = if is_down {
                (
                    STEP_HEIGHT,
                    -(step as f32) * STEP_HEIGHT - STEP_HEIGHT / 2.0,
                )
            } else {
                let height = (step as f32 + 1.0) * STEP_HEIGHT;
                (height, height / 2.0)
            };
            let mut mesh = Mesh::from(Cuboid::new(STEP_WIDTH, height, STEP_DEPTH));
            fix_box_uvs(&mut mesh, CELL_SIZE, false);
            apply_depth_fade(&mut mesh, step_z);
            spawn_child(
                commands,
                parent,
                meshes.add(mesh),
                step_material.clone(),
                Transform::from_xyz(0.0, center_y, step_z),
            );

            // Ceiling: mirrors the floor steps.
            let ceiling_bottom = if is_down {
                WALL_HEIGHT - step as f32 * STEP_HEIGHT
            } else {
                WALL_HEIGHT + step as f32 * STEP_HEIGHT
            };
            let mut mesh = Mesh::from(Cuboid::new(STEP_WIDTH, STEP_HEIGHT, STEP_DEPTH));
            fix_box_uvs(&mut mesh, CELL_SIZE, false);
            apply_depth_fade(&mut mesh, step_z);
            spawn_child(
                commands,
                parent,
                meshes.add(mesh),
                ceiling_material.clone(),
                Transform::from_xyz(0.0, ceiling_bottom + STEP_HEIGHT / 2.0, step_z),
            );
        }

        // Side walls, only against wall neighbors; they span two floors.
        let side_wall_y = if is_down { 0.0 } else { WALL_HEIGHT };
        for (present, direction) in [(has_left_wall, -1.0), (has_right_wall, 1.0)] {
            if !present {
                continue;
            }
            let mut mesh = Mesh::from(Cuboid::new(
                SIDE_WALL_THICKNESS,
                WALL_HEIGHT * 2.0,
                CELL_SIZE,
            ));
            fix_side_wall_uvs(&mut mesh);
            apply_depth_fade(&mut mesh, 0.0);
            spawn_child(
                commands,
                parent,
                meshes.add(mesh),
                side_material.clone(),
                Transform::from_xyz(
                    direction * (STEP_WIDTH / 2.0 + SIDE_WALL_THICKNESS / 2.0),
                    side_wall_y,
                    0.0,
                ),
            );
        }

        // Darkness beyond the stairwell, covering two floors.
        spawn_child(
            commands,
            parent,
            meshes.add(Mesh::from(Rectangle::new(STEP_WIDTH, WALL_HEIGHT * 2.0))),
            materials.stair_back.clone(),
            Transform::from_xyz(
                0.0,
                if is_down { 0.0 } else { WALL_HEIGHT },
                -CELL_SIZE / 2.0,
            ),
        );
    }
}

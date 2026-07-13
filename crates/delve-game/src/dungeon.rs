//! Dungeon geometry: floors, walls, and ceilings built from the level grid,
//! ported from the TS `buildDungeon` basics. Later phases add stairs, ramps,
//! environment zones, and wall entities.

use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use bevy::prelude::*;
use delve_core::texture_resolver::resolve_textures;
use delve_core::types::{CharDef, DungeonLevel, LayerDef};
use std::collections::HashSet;
use std::f32::consts::{FRAC_PI_2, PI};

pub const CELL_SIZE: f32 = 2.0;
pub const WALL_HEIGHT: f32 = 2.5;
pub const EYE_HEIGHT: f32 = WALL_HEIGHT * 0.65;
/// Layers stack flush: the floor of layer N+1 sits on the ceiling of layer N.
pub const LAYER_HEIGHT: f32 = WALL_HEIGHT;

/// Which layer a spawn call is building for: the level-wide and per-layer
/// definitions (for texture default/area/ceiling resolution), the layer's
/// index (for handle-map key prefixing via `layer_door_key`), and its Y
/// offset (for transform placement) — bundled so the multi-layer spawn
/// functions this slice's per-layer loop drives stay under the
/// argument-count lint.
pub struct LayerSpawn<'a> {
    pub level: &'a DungeonLevel,
    pub layer_def: &'a LayerDef,
    pub index: usize,
    pub y_offset: f32,
}

impl LayerSpawn<'_> {
    /// Per-layer `defaults`/`areas` override the level-wide ones when
    /// present, matching TS's `ld.defaults ?? level.defaults` /
    /// `ld.areas ?? level.areas`.
    pub(crate) fn texture_style(
        &self,
    ) -> (
        Option<&delve_core::types::TextureSet>,
        Option<&[delve_core::types::TextureArea]>,
    ) {
        (
            self.layer_def
                .defaults
                .as_ref()
                .or(self.level.defaults.as_ref()),
            self.layer_def
                .areas
                .as_deref()
                .or(self.level.areas.as_deref()),
        )
    }
}

// Rendering counterpart to walkability: OOB cells are solid boundary walls.
fn is_solid(grid: &[Vec<char>], col: i32, row: i32, renderable: &HashSet<char>) -> bool {
    if row < 0 || row as usize >= grid.len() {
        return true;
    }
    if col < 0 || col as usize >= grid[0].len() {
        return true;
    }
    !renderable.contains(&grid[row as usize][col as usize])
}

// Wall faces against a solid charDef neighbor use that neighbor's wallTexture.
fn wall_material_for_face(
    grid: &[Vec<char>],
    neighbor_col: i32,
    neighbor_row: i32,
    fallback: &str,
    char_defs: &[CharDef],
) -> String {
    if neighbor_row < 0
        || neighbor_row as usize >= grid.len()
        || neighbor_col < 0
        || neighbor_col as usize >= grid[0].len()
    {
        return fallback.to_string();
    }
    let neighbor_char = grid[neighbor_row as usize][neighbor_col as usize];
    if let Some(def) = char_defs.iter().find(|def| def.character == neighbor_char)
        && def.solid
        && let Some(texture) = &def.textures.wall_texture
    {
        return texture.clone();
    }
    fallback.to_string()
}

pub fn spawn_dungeon(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer: &LayerSpawn,
    stair_cells: &HashSet<String>,
    wall_entity_cells: &HashSet<String>,
) {
    let grid: Vec<Vec<char>> = layer
        .layer_def
        .grid
        .iter()
        .map(|row| row.chars().collect())
        .collect();
    let char_defs: &[CharDef] = layer.level.char_defs.as_deref().unwrap_or(&[]);
    let (layer_defaults, layer_areas) = layer.texture_style();
    // No-ceiling only applies to the topmost layer — a lower layer's ceiling
    // is physically the floor of the layer above it, so it always renders.
    let is_top_layer = layer.index + 1 == layer.level.layers.len();
    let ceiling_enabled = if is_top_layer {
        layer
            .layer_def
            .ceiling
            .or(layer.level.ceiling)
            .unwrap_or(true)
    } else {
        true
    };
    let layer_y_offset = layer.y_offset;

    let mut renderable: HashSet<char> = HashSet::from(['.']);
    for def in char_defs {
        if !def.solid || def.see_through == Some(true) {
            renderable.insert(def.character);
        }
    }

    let tile_mesh = meshes.add(Rectangle::new(CELL_SIZE, CELL_SIZE));
    let wall_mesh = meshes.add(Rectangle::new(CELL_SIZE, WALL_HEIGHT));

    for (row_index, row) in grid.iter().enumerate() {
        for (col_index, &cell_char) in row.iter().enumerate() {
            if !renderable.contains(&cell_char) {
                continue;
            }
            let col = col_index as i32;
            let row = row_index as i32;
            let center_x = col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
            let center_z = row as f32 * CELL_SIZE + CELL_SIZE / 2.0;

            let key = delve_core::game_state::door_key(i64::from(col), i64::from(row));

            // Stair cells own their floor, ceiling, and walls (stepped
            // geometry plus side/back walls come from the stair renderer).
            // Wall-entity cells (breakable/secret walls) own theirs too —
            // see `wall_entities::spawn_wall_entities`.
            if stair_cells.contains(&key) || wall_entity_cells.contains(&key) {
                continue;
            }

            let textures = resolve_textures(
                col,
                row,
                cell_char,
                layer_defaults,
                layer.level.char_defs.as_deref(),
                layer_areas,
            );

            commands.spawn((
                LevelEntity,
                Mesh3d(tile_mesh.clone()),
                MeshMaterial3d(materials.floor(&textures.floor)),
                Transform::from_xyz(center_x, layer_y_offset, center_z)
                    .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
            ));

            if ceiling_enabled {
                commands.spawn((
                    LevelEntity,
                    Mesh3d(tile_mesh.clone()),
                    MeshMaterial3d(materials.ceiling(&textures.ceiling)),
                    Transform::from_xyz(center_x, WALL_HEIGHT + layer_y_offset, center_z)
                        .with_rotation(Quat::from_rotation_x(FRAC_PI_2)),
                ));
            }

            // (rotation, neighbor delta, wall plane center offset)
            let faces = [
                (0.0, (0, -1), (0.0, -CELL_SIZE / 2.0)),
                (PI, (0, 1), (0.0, CELL_SIZE / 2.0)),
                (-FRAC_PI_2, (1, 0), (CELL_SIZE / 2.0, 0.0)),
                (FRAC_PI_2, (-1, 0), (-CELL_SIZE / 2.0, 0.0)),
            ];
            for (rotation_y, (dcol, drow), (offset_x, offset_z)) in faces {
                if !is_solid(&grid, col + dcol, row + drow, &renderable) {
                    continue;
                }
                let wall_texture = wall_material_for_face(
                    &grid,
                    col + dcol,
                    row + drow,
                    &textures.wall,
                    char_defs,
                );
                commands.spawn((
                    LevelEntity,
                    Mesh3d(wall_mesh.clone()),
                    MeshMaterial3d(materials.wall(&wall_texture)),
                    Transform::from_xyz(
                        center_x + offset_x,
                        WALL_HEIGHT / 2.0 + layer_y_offset,
                        center_z + offset_z,
                    )
                    .with_rotation(Quat::from_rotation_y(rotation_y)),
                ));
            }
        }
    }
}

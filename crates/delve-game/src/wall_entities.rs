//! Breakable and secret wall geometry: each cell owns an outward-facing
//! wall (always visible, blended with the corridor) and a hidden
//! floor+ceiling+inward-wall group revealed once broken/opened — ported
//! from the TS `wallEntityRenderer`. Spawned per layer (each layer's own
//! cells, keyed and Y-offset like every other multi-layer spawn function).
//! Multi-layer open-top/open-bottom auto-detection and neighboring-layer
//! geometry rebuilds specifically are still skipped — TS's own hollow-area
//! handling, not ported anywhere in this renderer yet (see `dungeon.rs`).

use crate::dungeon::{CELL_SIZE, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use bevy::prelude::*;
use delve_core::game_state::{door_key, layer_door_key};
use delve_core::texture_resolver::resolve_textures;
use delve_core::types::CharDef;
use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, PI};

/// Entities revealed once a wall-entity cell is broken or opened: floor,
/// ceiling, and any inward-facing walls against solid (non-entity)
/// neighbors. `outward_walls` (the faces visible from the corridor before
/// opening) are tracked separately since a persistent (illusory) secret
/// wall keeps them visible even after opening.
#[derive(Default)]
struct WallEntityCell {
    outward_walls: Vec<Entity>,
    hidden: Vec<Entity>,
}

#[derive(Resource, Default)]
pub struct WallEntityHandles {
    by_key: HashMap<String, WallEntityCell>,
}

impl WallEntityHandles {
    /// Merges another layer's spawn result in, keyed by its own
    /// layer-prefixed keys — `WallEntityCell` stays private to this module,
    /// so callers merge whole handle sets rather than reaching into the map.
    pub(crate) fn extend(&mut self, other: Self) {
        self.by_key.extend(other.by_key);
    }
}

/// Reveals the floor/ceiling/inward walls for a broken or opened cell.
/// `persistent` illusory secret walls keep their outward-facing wall
/// visible — the passage is walkable but still looks solid — matching the
/// TS `if (!result.persistent) entry.wallGroup.visible = false;` check.
pub fn reveal_wall_entity(
    handles: &WallEntityHandles,
    visibility: &mut Query<&mut Visibility>,
    key: &str,
    persistent: bool,
) {
    let Some(cell) = handles.by_key.get(key) else {
        return;
    };
    for &entity in &cell.hidden {
        if let Ok(mut visible) = visibility.get_mut(entity) {
            *visible = Visibility::Visible;
        }
    }
    if !persistent {
        for &entity in &cell.outward_walls {
            if let Ok(mut visible) = visibility.get_mut(entity) {
                *visible = Visibility::Hidden;
            }
        }
    }
}

fn cell_char(grid: &[Vec<char>], col: i32, row: i32) -> Option<char> {
    grid.get(usize::try_from(row).ok()?)?
        .get(usize::try_from(col).ok()?)
        .copied()
}

/// Builds the wall-entity geometry for every breakable/secret wall cell.
/// `cells` is the union of both entity kinds, keyed by `door_key`, since TS
/// builds them through the same renderer with no texture override for
/// either kind — a breakable wall looks identical to a plain wall too.
pub fn spawn_wall_entities(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer_spawn: &crate::dungeon::LayerSpawn,
    grid: &[String],
    cells: &HashMap<String, (i64, i64)>,
) -> WallEntityHandles {
    let mut handles = WallEntityHandles::default();
    if cells.is_empty() {
        return handles;
    }

    let char_defs: &[CharDef] = layer_spawn.level.char_defs.as_deref().unwrap_or(&[]);
    let (layer_defaults, layer_areas) = layer_spawn.texture_style();
    // No-ceiling only applies to the topmost layer, matching `dungeon.rs`.
    let is_top_layer = layer_spawn.index + 1 == layer_spawn.level.layers.len();
    let ceiling_enabled = if is_top_layer {
        layer_spawn
            .layer_def
            .ceiling
            .or(layer_spawn.level.ceiling)
            .unwrap_or(true)
    } else {
        true
    };
    let layer_y_offset = layer_spawn.y_offset;
    let grid_chars: Vec<Vec<char>> = grid.iter().map(|row| row.chars().collect()).collect();

    let mut renderable: HashSet<char> = HashSet::from(['.']);
    for def in char_defs {
        if !def.solid || def.see_through == Some(true) {
            renderable.insert(def.character);
        }
    }

    let tile_mesh = meshes.add(Rectangle::new(CELL_SIZE, CELL_SIZE));
    let wall_mesh = meshes.add(Rectangle::new(CELL_SIZE, WALL_HEIGHT));

    for (key, &(col, row)) in cells {
        let (col32, row32) = (col as i32, row as i32);
        let center_x = col32 as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = row32 as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let own_char = cell_char(&grid_chars, col32, row32).unwrap_or('#');
        let own_textures = resolve_textures(
            col32,
            row32,
            own_char,
            layer_defaults,
            layer_spawn.level.char_defs.as_deref(),
            layer_areas,
        );

        let mut cell = WallEntityCell::default();

        // (rotation, neighbor delta, wall plane center offset) — same
        // face table `dungeon::spawn_dungeon` uses.
        let faces = [
            (0.0, (0, -1), (0.0, -CELL_SIZE / 2.0)),
            (PI, (0, 1), (0.0, CELL_SIZE / 2.0)),
            (-FRAC_PI_2, (1, 0), (CELL_SIZE / 2.0, 0.0)),
            (FRAC_PI_2, (-1, 0), (-CELL_SIZE / 2.0, 0.0)),
        ];
        for (rotation_y, (dcol, drow), (offset_x, offset_z)) in faces {
            let (neighbor_col, neighbor_row) = (col32 + dcol, row32 + drow);
            let neighbor_char = cell_char(&grid_chars, neighbor_col, neighbor_row);
            let neighbor_walkable =
                neighbor_char.is_some_and(|character| renderable.contains(&character));
            let neighbor_is_entity =
                cells.contains_key(&door_key(i64::from(neighbor_col), i64::from(neighbor_row)));

            if neighbor_walkable {
                // Wall face visible from the walkable cell — owned by this
                // entity until opened, textured to match that corridor.
                let Some(neighbor_char) = neighbor_char else {
                    continue;
                };
                let neighbor_textures = resolve_textures(
                    neighbor_col,
                    neighbor_row,
                    neighbor_char,
                    layer_defaults,
                    layer_spawn.level.char_defs.as_deref(),
                    layer_areas,
                );
                let entity = commands
                    .spawn((
                        LevelEntity,
                        Mesh3d(wall_mesh.clone()),
                        MeshMaterial3d(materials.wall(&neighbor_textures.wall)),
                        Transform::from_xyz(
                            center_x + offset_x,
                            WALL_HEIGHT / 2.0 + layer_y_offset,
                            center_z + offset_z,
                        )
                        .with_rotation(Quat::from_rotation_y(rotation_y)),
                    ))
                    .id();
                cell.outward_walls.push(entity);
            } else if !neighbor_is_entity {
                // Solid neighbor that isn't another wall entity — revealed
                // once opened, facing inward (opposite the outward rotation).
                let entity = commands
                    .spawn((
                        LevelEntity,
                        Visibility::Hidden,
                        Mesh3d(wall_mesh.clone()),
                        MeshMaterial3d(materials.wall(&own_textures.wall)),
                        Transform::from_xyz(
                            center_x + offset_x,
                            WALL_HEIGHT / 2.0 + layer_y_offset,
                            center_z + offset_z,
                        )
                        .with_rotation(Quat::from_rotation_y(rotation_y + PI)),
                    ))
                    .id();
                cell.hidden.push(entity);
            }
        }

        let floor = commands
            .spawn((
                LevelEntity,
                Visibility::Hidden,
                Mesh3d(tile_mesh.clone()),
                MeshMaterial3d(materials.floor(&own_textures.floor)),
                Transform::from_xyz(center_x, layer_y_offset, center_z)
                    .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
            ))
            .id();
        cell.hidden.push(floor);

        if ceiling_enabled {
            let ceiling = commands
                .spawn((
                    LevelEntity,
                    Visibility::Hidden,
                    Mesh3d(tile_mesh.clone()),
                    MeshMaterial3d(materials.ceiling(&own_textures.ceiling)),
                    Transform::from_xyz(center_x, WALL_HEIGHT + layer_y_offset, center_z)
                        .with_rotation(Quat::from_rotation_x(FRAC_PI_2)),
                ))
                .id();
            cell.hidden.push(ceiling);
        }

        handles
            .by_key
            .insert(layer_door_key(layer_spawn.index, key), cell);
    }

    handles
}

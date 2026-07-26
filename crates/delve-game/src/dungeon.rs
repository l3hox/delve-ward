//! Dungeon geometry: floors, walls, and ceilings built from the level grid,
//! ported from the TS `buildDungeon` basics. Later phases add stairs, ramps,
//! environment zones, and wall entities.

use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use crate::zones::{self, LevelZones};
use bevy::prelude::*;
use delve_core::game_state::{LayerState, PitTrapState, door_key, layer_door_key};
use delve_core::grid::Facing;
use delve_core::texture_resolver::resolve_textures;
use delve_core::types::{CharDef, DungeonLevel, LayerDef};
use std::collections::{HashMap, HashSet};
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

/// Ported from TS's `solidCheck` (`rendering/dungeon.ts:350-356`): identical
/// to [`is_solid`] except an out-of-bounds neighbor is treated as open, not
/// solid, when this layer's own ceiling is disabled — an open-air top layer
/// has no boundary walls at its grid edges, matching TS's own comment there
/// ("OOB neighbors are treated as non-solid so no walls are generated at the
/// perimeter"). A layer with its ceiling on keeps every OOB neighbor solid,
/// identical to plain `is_solid`. TS's `forceRenderable` half of the same
/// closure (pit-trap fall-through cells on the layer below forced walkable)
/// has no Rust caller-side data yet and isn't ported here — a separate,
/// pre-existing gap from this one.
fn is_solid_for_wall(
    grid: &[Vec<char>],
    col: i32,
    row: i32,
    renderable: &HashSet<char>,
    ceiling_enabled: bool,
) -> bool {
    let out_of_bounds =
        row < 0 || row as usize >= grid.len() || col < 0 || col as usize >= grid[0].len();
    if !ceiling_enabled && out_of_bounds {
        return false;
    }
    is_solid(grid, col, row, renderable)
}

/// Per-cell vertical openness for one layer, ported from the
/// isOpenBottom/isOpenTop block TS repeats in `rendering/dungeon.ts:157-185`
/// and `rendering/wallEntityRenderer.ts:125-150`: a cell's floor is skipped
/// when the layer below's cell at the same coordinates is not a solid wall
/// (that's a hole — the fall system uses the identical predicate, so
/// wherever the player can fall, no floor renders), and its ceiling is
/// skipped when the layer above's cell is open the same way. An area's
/// explicit `openBottom`/`openTop` overrides the auto-detect in both
/// directions (`true` forces the surface open, `false` forces it closed).
pub(crate) struct VerticalOpenness<'a> {
    above: Option<&'a [String]>,
    below: Option<&'a [String]>,
    areas: Option<&'a [delve_core::types::TextureArea]>,
    char_defs: &'a [CharDef],
}

impl<'a> VerticalOpenness<'a> {
    pub(crate) fn for_layer(layer: &'a LayerSpawn<'a>, char_defs: &'a [CharDef]) -> Self {
        let (_, areas) = layer.texture_style();
        Self {
            above: layer
                .level
                .layers
                .get(layer.index + 1)
                .map(|layer_def| layer_def.grid.as_slice()),
            below: layer
                .index
                .checked_sub(1)
                .and_then(|below_index| layer.level.layers.get(below_index))
                .map(|layer_def| layer_def.grid.as_slice()),
            areas,
            char_defs,
        }
    }

    pub(crate) fn open_bottom(&self, col: usize, row: usize) -> bool {
        self.resolve(self.below, col, row, |area| area.open_bottom)
    }

    pub(crate) fn open_top(&self, col: usize, row: usize) -> bool {
        self.resolve(self.above, col, row, |area| area.open_top)
    }

    fn resolve(
        &self,
        adjacent: Option<&[String]>,
        col: usize,
        row: usize,
        area_flag: impl Fn(&delve_core::types::TextureArea) -> Option<bool>,
    ) -> bool {
        let mut open = adjacent_cell_is_open(adjacent, col, row, self.char_defs);
        let (col, row) = (col as i32, row as i32);
        for area in self.areas.unwrap_or(&[]) {
            if col >= area.from_col
                && col <= area.to_col
                && row >= area.from_row
                && row <= area.to_row
                && let Some(explicit) = area_flag(area)
            {
                open = explicit;
            }
        }
        open
    }
}

/// TS bounds-checks against the adjacent grid's first row (`row <
/// grid.length && col < grid[0].length`) — a cell past those bounds, or with
/// no adjacent layer at all, is NOT open (the surface renders). Inside
/// bounds, a row shorter than row 0 indexes to `undefined` in TS, which its
/// solid-wall formula treats as open; `chars().nth` returning `None` maps to
/// the same answer here.
fn adjacent_cell_is_open(
    adjacent: Option<&[String]>,
    col: usize,
    row: usize,
    char_defs: &[CharDef],
) -> bool {
    let Some(grid) = adjacent else {
        return false;
    };
    if row >= grid.len()
        || grid
            .first()
            .is_none_or(|first| col >= first.chars().count())
    {
        return false;
    }
    !grid[row]
        .chars()
        .nth(col)
        .is_some_and(|character| crate::session::is_solid_floor_char(character, char_defs))
}

/// Which way a cell's geometry is cut when the zone changes across it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitAxis {
    /// The boundary runs east-west; the cell splits into north and south
    /// halves, and the walls that cross it are the ones running along Z.
    NorthSouth,
    /// The boundary runs north-south; the cell splits into west and east
    /// halves, and the walls crossing it run along X.
    WestEast,
}

/// A door cell that two environment zones meet inside, and the zone each half
/// belongs to.
///
/// Ported from `rendering/dungeon.ts:187-200`. Environment zones are rendered
/// by separate cameras, so the seam between indoors and outdoors falls
/// wherever the geometry is tagged — with whole cells, that puts it at a tile
/// edge, one full cell to whichever side. TS splits the door cell itself so
/// the seam lands in the doorway, which is where a doorway between two
/// environments should change.
struct ZoneSplit {
    axis: SplitAxis,
    /// Zone for the north (or west) half.
    leading: usize,
    /// Zone for the south (or east) half.
    trailing: usize,
}

/// TS checks the four neighbours in order and takes the first whose zone
/// differs, so a cell between two different zones resolves by that order
/// rather than being split twice. The half nearest the differing neighbour
/// takes that neighbour's zone; the other keeps the cell's own.
fn zone_split(
    zone_of_neighbor: impl Fn(i64, i64) -> Option<usize>,
    own_zone: usize,
) -> Option<ZoneSplit> {
    let north = zone_of_neighbor(0, -1);
    let south = zone_of_neighbor(0, 1);
    let east = zone_of_neighbor(1, 0);
    let west = zone_of_neighbor(-1, 0);

    if let Some(zone) = north.filter(|zone| *zone != own_zone) {
        return Some(ZoneSplit {
            axis: SplitAxis::NorthSouth,
            leading: zone,
            trailing: own_zone,
        });
    }
    if let Some(zone) = south.filter(|zone| *zone != own_zone) {
        return Some(ZoneSplit {
            axis: SplitAxis::NorthSouth,
            leading: own_zone,
            trailing: zone,
        });
    }
    if let Some(zone) = east.filter(|zone| *zone != own_zone) {
        return Some(ZoneSplit {
            axis: SplitAxis::WestEast,
            leading: own_zone,
            trailing: zone,
        });
    }
    if let Some(zone) = west.filter(|zone| *zone != own_zone) {
        return Some(ZoneSplit {
            axis: SplitAxis::WestEast,
            leading: zone,
            trailing: own_zone,
        });
    }
    None
}

/// A rectangle whose UVs are squeezed onto the half of the texture the piece
/// covers, so a split tile keeps the texture scale of a whole one — TS scales
/// the same attribute on its shared half-tile geometries
/// (`rendering/dungeon.ts:18-36`). `uv_axis` is 0 for U, 1 for V.
fn half_mesh(width: f32, height: f32, uv_axis: usize) -> Mesh {
    let mut mesh = Mesh::from(Rectangle::new(width, height));
    if let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
    {
        for uv in uvs.iter_mut() {
            uv[uv_axis] *= 0.5;
        }
    }
    mesh
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

/// Every cell's floor, ceiling, and walls are tagged as one unit to that
/// cell's own environment zone (`zones::tag_cell`, a no-op when `zones` isn't
/// multi-zone) — except door cells that two zones meet inside, whose geometry
/// is cut in half so the seam falls in the doorway (see [`ZoneSplit`]).
#[allow(clippy::too_many_arguments)]
pub fn spawn_dungeon(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer: &LayerSpawn,
    stair_cells: &HashSet<String>,
    wall_entity_cells: &HashSet<String>,
    pit_trap_cells: &HashSet<String>,
    ramp_base_cells: &HashMap<String, Facing>,
    zones: &LevelZones,
    door_cells: &HashSet<String>,
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
    // Halves for door cells that two zones meet inside — see [`ZoneSplit`].
    let half_tile_north_south = meshes.add(half_mesh(CELL_SIZE, CELL_SIZE / 2.0, 1));
    let half_tile_west_east = meshes.add(half_mesh(CELL_SIZE / 2.0, CELL_SIZE, 0));
    let half_wall_mesh = meshes.add(half_mesh(CELL_SIZE / 2.0, WALL_HEIGHT, 0));
    let openness = VerticalOpenness::for_layer(layer, char_defs);

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

            // A door standing on a zone boundary has its geometry cut so the
            // seam falls inside the doorway rather than at a tile edge.
            let split = if door_cells.contains(&key) {
                zones
                    .zone_at(layer.index, i64::from(col), i64::from(row))
                    .and_then(|own| {
                        zone_split(
                            |dcol, drow| {
                                zones.zone_at(
                                    layer.index,
                                    i64::from(col) + dcol,
                                    i64::from(row) + drow,
                                )
                            },
                            own,
                        )
                    })
            } else {
                None
            };
            // The two halves' offsets from the cell centre, along whichever
            // axis the split runs.
            let half_offset = CELL_SIZE / 4.0;

            // Pit-trap cells keep their normal ceiling/walls here but get a
            // separate, toggleable floor tile from `spawn_pit_floors`
            // instead of this one — TS's `onPitTrapSignalChanged` only ever
            // toggles the floor mesh's visibility, never the surrounding
            // ceiling/walls for that cell.
            if !pit_trap_cells.contains(&key) && !openness.open_bottom(col_index, row_index) {
                let floor_material = materials.floor(&textures.floor);
                let flat = Quat::from_rotation_x(-FRAC_PI_2);
                if let Some(split) = &split {
                    let (mesh, leading_position, trailing_position) = match split.axis {
                        SplitAxis::NorthSouth => (
                            half_tile_north_south.clone(),
                            Vec3::new(center_x, layer_y_offset, center_z - half_offset),
                            Vec3::new(center_x, layer_y_offset, center_z + half_offset),
                        ),
                        SplitAxis::WestEast => (
                            half_tile_west_east.clone(),
                            Vec3::new(center_x - half_offset, layer_y_offset, center_z),
                            Vec3::new(center_x + half_offset, layer_y_offset, center_z),
                        ),
                    };
                    for (position, zone) in [
                        (leading_position, split.leading),
                        (trailing_position, split.trailing),
                    ] {
                        let half = commands
                            .spawn((
                                LevelEntity,
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(floor_material.clone()),
                                Transform::from_translation(position).with_rotation(flat),
                            ))
                            .id();
                        zones::tag_zone(commands, half, zone);
                    }
                } else {
                    let floor = commands
                        .spawn((
                            LevelEntity,
                            Mesh3d(tile_mesh.clone()),
                            MeshMaterial3d(floor_material),
                            Transform::from_xyz(center_x, layer_y_offset, center_z)
                                .with_rotation(flat),
                        ))
                        .id();
                    zones::tag_cell(
                        commands,
                        zones,
                        layer.index,
                        floor,
                        i64::from(col),
                        i64::from(row),
                    );
                }
            }

            // A ramp based at this cell rises up through where the ceiling
            // would be — TS's `buildRampInfo` marks every ramp base cell
            // `skipCeiling: true` unconditionally.
            let ramp_facing = ramp_base_cells.get(&key).copied();
            if ceiling_enabled && ramp_facing.is_none() && !openness.open_top(col_index, row_index)
            {
                let ceiling_material = materials.ceiling(&textures.ceiling);
                let ceiling_y = WALL_HEIGHT + layer_y_offset;
                let facing_down = Quat::from_rotation_x(FRAC_PI_2);
                if let Some(split) = &split {
                    let (mesh, leading_position, trailing_position) = match split.axis {
                        SplitAxis::NorthSouth => (
                            half_tile_north_south.clone(),
                            Vec3::new(center_x, ceiling_y, center_z - half_offset),
                            Vec3::new(center_x, ceiling_y, center_z + half_offset),
                        ),
                        SplitAxis::WestEast => (
                            half_tile_west_east.clone(),
                            Vec3::new(center_x - half_offset, ceiling_y, center_z),
                            Vec3::new(center_x + half_offset, ceiling_y, center_z),
                        ),
                    };
                    for (position, zone) in [
                        (leading_position, split.leading),
                        (trailing_position, split.trailing),
                    ] {
                        let half = commands
                            .spawn((
                                LevelEntity,
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(ceiling_material.clone()),
                                Transform::from_translation(position).with_rotation(facing_down),
                            ))
                            .id();
                        zones::tag_zone(commands, half, zone);
                    }
                } else {
                    let ceiling = commands
                        .spawn((
                            LevelEntity,
                            Mesh3d(tile_mesh.clone()),
                            MeshMaterial3d(ceiling_material),
                            Transform::from_xyz(center_x, ceiling_y, center_z)
                                .with_rotation(facing_down),
                        ))
                        .id();
                    zones::tag_cell(
                        commands,
                        zones,
                        layer.index,
                        ceiling,
                        i64::from(col),
                        i64::from(row),
                    );
                }
            }

            // (rotation, neighbor delta, wall plane center offset, the axis the
            // wall runs along — a wall only splits when the zone boundary
            // crosses its length, so north/south walls answer to an east-west
            // boundary and east/west walls to a north-south one)
            let faces = [
                (0.0, (0, -1), (0.0, -CELL_SIZE / 2.0), SplitAxis::WestEast),
                (PI, (0, 1), (0.0, CELL_SIZE / 2.0), SplitAxis::WestEast),
                (
                    -FRAC_PI_2,
                    (1, 0),
                    (CELL_SIZE / 2.0, 0.0),
                    SplitAxis::NorthSouth,
                ),
                (
                    FRAC_PI_2,
                    (-1, 0),
                    (-CELL_SIZE / 2.0, 0.0),
                    SplitAxis::NorthSouth,
                ),
            ];
            for (rotation_y, (dcol, drow), (offset_x, offset_z), split_when) in faces {
                // The wall a ramp opens through is skipped too — TS's
                // `rampDirs?.includes(direction)` check.
                if ramp_facing.is_some_and(|facing| facing.delta() == (dcol, drow)) {
                    continue;
                }
                // A face toward a breakable or secret wall belongs to that
                // entity, which owns both the closed and opened look of its own
                // cell (`wall_entities.rs`). TS skips it here for the same
                // reason (`rendering/dungeon.ts:393,406`'s `skipN`/`skipS` and
                // their east/west siblings); without the skip a second,
                // permanent wall stands in the same plane and the cell still
                // looks solid after the entity's own wall is hidden — walkable
                // but visually intact.
                let neighbor_key =
                    delve_core::game_state::door_key(i64::from(col + dcol), i64::from(row + drow));
                if wall_entity_cells.contains(&neighbor_key) {
                    continue;
                }
                if !is_solid_for_wall(&grid, col + dcol, row + drow, &renderable, ceiling_enabled) {
                    continue;
                }
                let wall_texture = wall_material_for_face(
                    &grid,
                    col + dcol,
                    row + drow,
                    &textures.wall,
                    char_defs,
                );
                let wall_material = materials.wall(&wall_texture);
                let wall_y = WALL_HEIGHT / 2.0 + layer_y_offset;
                let rotation = Quat::from_rotation_y(rotation_y);
                if let Some(split) = split.as_ref().filter(|split| split.axis == split_when) {
                    let (leading_position, trailing_position) = match split.axis {
                        SplitAxis::NorthSouth => (
                            Vec3::new(
                                center_x + offset_x,
                                wall_y,
                                center_z + offset_z - half_offset,
                            ),
                            Vec3::new(
                                center_x + offset_x,
                                wall_y,
                                center_z + offset_z + half_offset,
                            ),
                        ),
                        SplitAxis::WestEast => (
                            Vec3::new(
                                center_x + offset_x - half_offset,
                                wall_y,
                                center_z + offset_z,
                            ),
                            Vec3::new(
                                center_x + offset_x + half_offset,
                                wall_y,
                                center_z + offset_z,
                            ),
                        ),
                    };
                    for (position, zone) in [
                        (leading_position, split.leading),
                        (trailing_position, split.trailing),
                    ] {
                        let half = commands
                            .spawn((
                                LevelEntity,
                                Mesh3d(half_wall_mesh.clone()),
                                MeshMaterial3d(wall_material.clone()),
                                Transform::from_translation(position).with_rotation(rotation),
                            ))
                            .id();
                        zones::tag_zone(commands, half, zone);
                    }
                    continue;
                }
                let wall = commands
                    .spawn((
                        LevelEntity,
                        Mesh3d(wall_mesh.clone()),
                        MeshMaterial3d(wall_material),
                        Transform::from_xyz(center_x + offset_x, wall_y, center_z + offset_z)
                            .with_rotation(rotation),
                    ))
                    .id();
                zones::tag_cell(
                    commands,
                    zones,
                    layer.index,
                    wall,
                    i64::from(col),
                    i64::from(row),
                );
            }
        }
    }
}

/// Floor tile entities for pit-trap cells, layer-door-keyed — hidden when
/// the trap opens, shown when it closes, ported from TS's `pitFloorMap`.
/// Spawned separately from `spawn_dungeon`'s main per-cell pass (which
/// excludes these cells' floor specifically, not their ceiling/walls) so
/// each tile can be found and toggled later.
#[derive(Resource, Default)]
pub struct PitFloorHandles {
    pub by_key: HashMap<String, Entity>,
}

pub fn spawn_pit_floors(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer: &LayerSpawn,
    layer_state: &LayerState,
) -> PitFloorHandles {
    let mut handles = PitFloorHandles::default();
    if layer_state.pit_traps.is_empty() {
        return handles;
    }
    let (layer_defaults, layer_areas) = layer.texture_style();
    let tile_mesh = meshes.add(Rectangle::new(CELL_SIZE, CELL_SIZE));
    let char_defs: &[CharDef] = layer.level.char_defs.as_deref().unwrap_or(&[]);
    let openness = VerticalOpenness::for_layer(layer, char_defs);

    for pit in layer_state.pit_traps.values() {
        // TS only tracks a pit floor mesh when `buildDungeon` built one —
        // an open-bottom pit cell gets no floor tile at all
        // (`rendering/dungeon.ts:204,255`).
        if openness.open_bottom(pit.col as usize, pit.row as usize) {
            continue;
        }
        let (col, row) = (pit.col as i32, pit.row as i32);
        let cell_char = layer
            .layer_def
            .grid
            .get(row as usize)
            .and_then(|line| line.chars().nth(col as usize))
            .unwrap_or('.');
        let textures = resolve_textures(
            col,
            row,
            cell_char,
            layer_defaults,
            layer.level.char_defs.as_deref(),
            layer_areas,
        );
        let center_x = col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let visibility = if pit.state == PitTrapState::Open {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        let entity = commands
            .spawn((
                LevelEntity,
                Mesh3d(tile_mesh.clone()),
                MeshMaterial3d(materials.floor(&textures.floor)),
                Transform::from_xyz(center_x, layer.y_offset, center_z)
                    .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
                visibility,
            ))
            .id();
        handles.by_key.insert(
            layer_door_key(layer.index, &door_key(pit.col, pit.row)),
            entity,
        );
    }
    handles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> Vec<Vec<char>> {
        rows.iter().map(|row| row.chars().collect()).collect()
    }

    #[test]
    fn is_solid_for_wall_treats_out_of_bounds_as_open_when_ceiling_is_disabled() {
        let grid = grid(&["##", "##"]);
        let renderable = HashSet::from(['.']);
        assert!(!is_solid_for_wall(&grid, -1, 0, &renderable, false));
        assert!(!is_solid_for_wall(&grid, 0, -1, &renderable, false));
        assert!(!is_solid_for_wall(&grid, 5, 0, &renderable, false));
        assert!(!is_solid_for_wall(&grid, 0, 5, &renderable, false));
    }

    #[test]
    fn is_solid_for_wall_keeps_out_of_bounds_solid_when_ceiling_is_enabled() {
        let grid = grid(&["##", "##"]);
        let renderable = HashSet::from(['.']);
        assert!(is_solid_for_wall(&grid, -1, 0, &renderable, true));
        assert!(is_solid_for_wall(&grid, 5, 0, &renderable, true));
    }

    #[test]
    fn is_solid_for_wall_still_reads_in_bounds_cells_normally_when_ceiling_is_disabled() {
        let grid = grid(&["#."]);
        let renderable = HashSet::from(['.']);
        assert!(is_solid_for_wall(&grid, 0, 0, &renderable, false));
        assert!(!is_solid_for_wall(&grid, 1, 0, &renderable, false));
    }

    #[test]
    fn is_solid_for_wall_matches_is_solid_exactly_when_ceiling_is_enabled() {
        let grid = grid(&["#.", ".#"]);
        let renderable = HashSet::from(['.']);
        for row in -1..3 {
            for col in -1..3 {
                assert_eq!(
                    is_solid_for_wall(&grid, col, row, &renderable, true),
                    is_solid(&grid, col, row, &renderable),
                );
            }
        }
    }

    fn lines(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|row| (*row).to_string()).collect()
    }

    /// A doorway between an indoor and an outdoor zone splits across the
    /// passage, and the half nearest the differing neighbour takes that
    /// neighbour's zone — that is what puts the seam in the doorway instead of
    /// a tile edge.
    #[test]
    fn a_zone_boundary_to_the_north_splits_the_cell_north_to_south() {
        let split = zone_split(|dcol, drow| ((dcol, drow) == (0, -1)).then_some(4), 1)
            .expect("a differing neighbour splits the cell");
        assert!(split.axis == SplitAxis::NorthSouth);
        assert_eq!(split.leading, 4, "north half joins the northern zone");
        assert_eq!(split.trailing, 1, "south half keeps the cell's own");
    }

    /// The mirror case: the differing neighbour is south, so the halves swap
    /// which zone each takes while the axis stays the same.
    #[test]
    fn a_zone_boundary_to_the_south_keeps_the_axis_and_swaps_the_halves() {
        let split = zone_split(|dcol, drow| ((dcol, drow) == (0, 1)).then_some(4), 1)
            .expect("a differing neighbour splits the cell");
        assert!(split.axis == SplitAxis::NorthSouth);
        assert_eq!(split.leading, 1);
        assert_eq!(split.trailing, 4);
    }

    #[test]
    fn a_zone_boundary_to_the_west_splits_the_cell_west_to_east() {
        let split = zone_split(|dcol, drow| ((dcol, drow) == (-1, 0)).then_some(2), 5)
            .expect("a differing neighbour splits the cell");
        assert!(split.axis == SplitAxis::WestEast);
        assert_eq!(split.leading, 2, "west half joins the western zone");
        assert_eq!(split.trailing, 5);
    }

    /// A cell whose neighbours are all in its own zone is not a boundary and
    /// keeps its geometry whole.
    #[test]
    fn a_cell_inside_one_zone_is_not_split() {
        assert!(zone_split(|_, _| Some(3), 3).is_none());
        assert!(zone_split(|_, _| None, 3).is_none());
    }

    /// TS resolves in north, south, east, west order and stops at the first
    /// differing neighbour, so a corner cell splits once rather than twice.
    #[test]
    fn the_first_differing_neighbour_wins_over_later_ones() {
        let split = zone_split(
            |dcol, drow| match (dcol, drow) {
                (0, 1) => Some(7),
                (1, 0) => Some(9),
                _ => Some(1),
            },
            1,
        )
        .expect("two differing neighbours still split the cell");
        assert!(
            split.axis == SplitAxis::NorthSouth,
            "south is checked before east"
        );
        assert_eq!(split.trailing, 7);
    }

    fn open_area(
        open_bottom: Option<bool>,
        open_top: Option<bool>,
    ) -> delve_core::types::TextureArea {
        delve_core::types::TextureArea {
            from_col: 0,
            to_col: 9,
            from_row: 0,
            to_row: 9,
            environment: None,
            open_bottom,
            open_top,
            textures: delve_core::types::TextureSet {
                wall_texture: None,
                floor_texture: None,
                ceiling_texture: None,
            },
        }
    }

    #[test]
    fn floor_is_open_over_a_walkable_below_cell_and_closed_over_solid_rock() {
        let below = lines(&["#."]);
        let openness = VerticalOpenness {
            above: None,
            below: Some(&below),
            areas: None,
            char_defs: &[],
        };
        assert!(!openness.open_bottom(0, 0));
        assert!(openness.open_bottom(1, 0));
    }

    #[test]
    fn surfaces_stay_closed_when_there_is_no_adjacent_layer() {
        let openness = VerticalOpenness {
            above: None,
            below: None,
            areas: None,
            char_defs: &[],
        };
        assert!(!openness.open_bottom(0, 0));
        assert!(!openness.open_top(0, 0));
    }

    /// A solid-but-seeThrough charDef in the layer above is not a solid wall
    /// under TS's formula (`def.solid && !def.seeThrough`), so the ceiling
    /// under it opens; a plain solid charDef keeps it closed.
    #[test]
    fn ceiling_openness_honors_see_through_char_defs() {
        let char_defs = [
            CharDef {
                character: 'w',
                solid: true,
                see_through: Some(true),
                textures: delve_core::types::TextureSet {
                    wall_texture: None,
                    floor_texture: None,
                    ceiling_texture: None,
                },
            },
            CharDef {
                character: 'r',
                solid: true,
                see_through: None,
                textures: delve_core::types::TextureSet {
                    wall_texture: None,
                    floor_texture: None,
                    ceiling_texture: None,
                },
            },
        ];
        let above = lines(&["wr"]);
        let openness = VerticalOpenness {
            above: Some(&above),
            below: None,
            areas: None,
            char_defs: &char_defs,
        };
        assert!(openness.open_top(0, 0));
        assert!(!openness.open_top(1, 0));
    }

    /// Explicit area flags override the auto-detect in both directions:
    /// `openBottom: false` closes a floor the below-grid says is open, and
    /// `openTop: true` opens a ceiling that has no above-grid evidence.
    #[test]
    fn area_flags_override_the_auto_detect() {
        let below = lines(&[".."]);
        let areas = [open_area(Some(false), Some(true))];
        let openness = VerticalOpenness {
            above: None,
            below: Some(&below),
            areas: Some(&areas),
            char_defs: &[],
        };
        assert!(!openness.open_bottom(0, 0));
        assert!(openness.open_top(0, 0));
    }

    /// TS bounds-checks column against the adjacent grid's FIRST row
    /// (`col < grid[0].length`), then indexes the actual row — a cell past
    /// row 0's width is closed, while an in-bounds column over a shorter
    /// ragged row reads `undefined` and counts as open.
    #[test]
    fn adjacent_bounds_follow_ts_first_row_indexing() {
        let below = lines(&["###", "#"]);
        let openness = VerticalOpenness {
            above: None,
            below: Some(&below),
            areas: None,
            char_defs: &[],
        };
        assert!(!openness.open_bottom(3, 0));
        assert!(!openness.open_bottom(0, 2));
        assert!(openness.open_bottom(1, 1));
    }
}

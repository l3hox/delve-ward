//! Door rendering: a stone frame per door cell plus a sliding panel that
//! animates open/closed, ported from the TS door renderer and animator.
//!
//! A door standing where two environment zones meet is cut along the passage
//! it blocks, each half tagged to the zone on its own side, so the frame and
//! panel are shaded by the environment the viewer is standing in rather than
//! by whichever one owns the cell. That doorway also carries a light on its
//! indoor side which brightens as the door opens — see [`DoorBoundaryLight`].
use crate::dungeon::{CELL_SIZE, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use crate::zones::{self, LevelZones};
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use delve_core::game_state::{DoorState, LayerState, door_key, layer_door_key};
use delve_core::types::CharDef;
use std::collections::{HashMap, HashSet};
use std::f32::consts::FRAC_PI_2;

const FRAME_DEPTH: f32 = 0.15;
const FRAME_WIDTH: f32 = 0.15;
const PANEL_DEPTH: f32 = 0.08;
const BUTTON_SIZE: f32 = 0.06;
const BUTTON_DEPTH: f32 = 0.03;
/// Slightly above center, near eye level.
const BUTTON_HEIGHT: f32 = 1.1;
const SLIDE_SPEED: f32 = 5.0;
const BOUNCE_FRACTION: f32 = 0.2;
const BOUNCE_SPEED: f32 = 3.0;

/// NS = door faces N-S (blocks E-W passage), EW = door faces E-W (blocks N-S).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorOrientation {
    NS,
    EW,
}

/// Which world axis a door panel slides along when opening. `Y` (the
/// default) slides the panel up into the ceiling void; `X`/`Z` slide it
/// sideways into the wall plane when there's nothing solid above to slide
/// into, ported from `doorAnimator.ts`'s `SlideAxis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideAxis {
    X,
    Y,
    Z,
}

impl SlideAxis {
    fn component_index(self) -> usize {
        match self {
            SlideAxis::X => 0,
            SlideAxis::Y => 1,
            SlideAxis::Z => 2,
        }
    }
}

/// Panel entities by door cell key, for open/close animation lookups.
#[derive(Resource, Default)]
pub struct DoorPanels {
    pub by_key: HashMap<String, Entity>,
}

#[derive(Debug, Clone, Copy)]
pub struct DoorBounce {
    reopening: bool,
    bounce_val: f32,
}

#[derive(Component)]
pub struct DoorPanel {
    pub axis: SlideAxis,
    pub closed_val: f32,
    pub open_val: f32,
    pub target_val: f32,
    bounce: Option<DoorBounce>,
}

/// The light TS hangs in a zone-boundary doorway (`doorRenderer.ts:338-347`),
/// tagged to the indoor zone only, so an opening door spills light into the
/// room rather than out into the open air. Its brightness tracks how far the
/// panel has actually travelled, not whether the door is logically open, so
/// it comes up and down with the slide (`main.ts:1226-1231`).
#[derive(Component)]
pub struct DoorBoundaryLight {
    /// The panel whose travel drives this light.
    pub panel: Entity,
}

/// TS's `PointLight(0xffeedd, 3, 10, 2)`, whose intensity the game loop then
/// overwrites every frame with `fraction * 2` — so 3 is never actually seen
/// and 2 is the real maximum.
const BOUNDARY_LIGHT_MAX_INTENSITY: f32 = 2.0;
const BOUNDARY_LIGHT_RANGE: f32 = 10.0;
const BOUNDARY_LIGHT_COLOR: Color = Color::srgb_u8(0xff, 0xee, 0xdd);
/// Below this the light is switched off outright rather than left at a
/// vanishingly dim value (`main.ts:1230`).
const BOUNDARY_LIGHT_MIN_FRACTION: f32 = 0.01;

impl DoorPanel {
    pub fn set_open(&mut self, open: bool) {
        self.target_val = if open { self.open_val } else { self.closed_val };
    }

    /// How far the panel has slid from closed to open, 0 to 1 — ported from
    /// `doorAnimator.ts`'s `getOpenFraction`, including its degenerate-range
    /// guard. Reads the live transform rather than `target_val`, so it follows
    /// the animation instead of jumping with the state change.
    pub fn open_fraction(&self, transform: &Transform) -> f32 {
        let range = self.open_val - self.closed_val;
        if range.abs() < 0.001 {
            return 0.0;
        }
        let current = transform.translation[self.axis.component_index()];
        ((current - self.closed_val) / range).clamp(0.0, 1.0)
    }

    /// Animate the door closing 20% then bouncing back to open.
    pub fn bounce(&mut self) {
        let range = self.open_val - self.closed_val;
        self.bounce = Some(DoorBounce {
            reopening: false,
            bounce_val: self.open_val - range * BOUNCE_FRACTION,
        });
        self.target_val = self.open_val;
    }
}

/// The zones on either side of a door, in the frame's own local space where
/// -Z and +Z are the two ends of the passage it blocks.
///
/// Ported from `doorRenderer.ts:234-254`. Which world neighbours those ends
/// correspond to depends on the door's orientation, because an NS door's
/// frame is rotated a quarter turn: an EW door looks north and south, an NS
/// door west and east. A door is only split when both sides resolve to zones
/// and those zones differ.
fn door_split_zones(
    zones: &LevelZones,
    layer_index: usize,
    orientation: DoorOrientation,
    col: i64,
    row: i64,
) -> Option<(usize, usize)> {
    let (negative_z, positive_z) = if orientation == DoorOrientation::EW {
        ((col, row - 1), (col, row + 1))
    } else {
        ((col - 1, row), (col + 1, row))
    };
    let negative_zone = zones.zone_at(layer_index, negative_z.0, negative_z.1)?;
    let positive_zone = zones.zone_at(layer_index, positive_z.0, positive_z.1)?;
    (negative_zone != positive_zone).then_some((negative_zone, positive_zone))
}

/// Scale box UVs so each face samples texture proportional to its size,
/// preventing squeeze on thin dimensions. When `full_front_back` is set the
/// ±z faces keep their default 0..1 mapping (used for door panels).
///
/// Like the TS renderers, UVs are corner-anchored: each face's UV range
/// starts at 0 and spans size/reference, so texture coursing lines up with
/// the flat wall tiles.
pub(crate) fn fix_box_uvs(mesh: &mut Mesh, reference_size: f32, full_front_back: bool) {
    let positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return,
    };
    let normals: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return,
    };
    let half_extents = half_extents(&positions);
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
            if full_front_back {
                continue;
            }
            (0, 1) // ±z faces: U across width, V across height
        };
        uv[0] = (position[u_axis] + half_extents[u_axis]) / reference_size;
        uv[1] = (position[v_axis] + half_extents[v_axis]) / reference_size;
    }
}

/// Per-axis half sizes of a mesh centered on its origin.
pub(crate) fn half_extents(positions: &[[f32; 3]]) -> [f32; 3] {
    let mut extents = [0.0_f32; 3];
    for position in positions {
        for (axis, extent) in extents.iter_mut().enumerate() {
            *extent = extent.max(position[axis].abs());
        }
    }
    extents
}

fn box_mesh(
    meshes: &mut Assets<Mesh>,
    width: f32,
    height: f32,
    depth: f32,
    full_front_back: bool,
) -> Handle<Mesh> {
    let mut mesh = Mesh::from(Cuboid::new(width, height, depth));
    fix_box_uvs(&mut mesh, CELL_SIZE, full_front_back);
    meshes.add(mesh)
}

pub fn detect_door_orientation(
    grid: &[String],
    col: i64,
    row: i64,
    walkable: &HashSet<char>,
) -> DoorOrientation {
    let cell = |c: i64, r: i64| -> Option<char> {
        let line = grid.get(usize::try_from(r).ok()?)?;
        line.chars().nth(usize::try_from(c).ok()?)
    };
    let solid = |c: i64, r: i64| cell(c, r).is_none_or(|ch| !walkable.contains(&ch));

    if solid(col + 1, row) && solid(col - 1, row) {
        // Walls E and W: passage runs N-S, door faces E-W.
        return DoorOrientation::EW;
    }
    if solid(col, row - 1) && solid(col, row + 1) {
        return DoorOrientation::NS;
    }
    DoorOrientation::NS
}

/// Whether this layer renders its own ceiling geometry, mirroring
/// `dungeon.rs`'s `ceiling_enabled`: a lower layer's ceiling doubles as the
/// floor of the layer above it, so only the topmost layer can go
/// ceiling-less.
fn layer_has_ceiling(layer_spawn: &crate::dungeon::LayerSpawn) -> bool {
    let is_top_layer = layer_spawn.index + 1 == layer_spawn.level.layers.len();
    if is_top_layer {
        layer_spawn
            .layer_def
            .ceiling
            .or(layer_spawn.level.ceiling)
            .unwrap_or(true)
    } else {
        true
    }
}

/// Whether a door's panel has nothing solid to slide up into: either this
/// layer has no ceiling at all, or the layer above has no solid cell
/// directly over the door. Ported from `levelSceneBuilder.ts`'s per-door
/// `ceilingOpenAbove` check.
fn ceiling_open_above(
    has_ceiling: bool,
    above_grid: Option<&[String]>,
    char_defs: &[CharDef],
    col: i64,
    row: i64,
) -> bool {
    if !has_ceiling {
        return true;
    }
    let Some(above_char) = above_grid.and_then(|grid| {
        let line = grid.get(usize::try_from(row).ok()?)?;
        line.chars().nth(usize::try_from(col).ok()?)
    }) else {
        return false;
    };
    let is_solid_wall = above_char == '#'
        || char_defs
            .iter()
            .any(|def| def.character == above_char && def.solid && def.see_through != Some(true));
    !is_solid_wall
}

/// The axis a door panel slides along when opening, ported from
/// `levelSceneBuilder.ts`'s per-door `slideAxis` selection: with nothing
/// solid above to slide into, the panel slides sideways along its own width
/// axis (NS-oriented doors along Z, EW-oriented doors along X) instead of up.
fn slide_axis_for(orientation: DoorOrientation, ceiling_open_above: bool) -> SlideAxis {
    if !ceiling_open_above {
        return SlideAxis::Y;
    }
    if orientation == DoorOrientation::NS {
        SlideAxis::Z
    } else {
        SlideAxis::X
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_doors(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
    grid: &[String],
    walkable: &HashSet<char>,
    zones: &LevelZones,
) -> DoorPanels {
    let mut panels = DoorPanels::default();
    let layer_index = layer_spawn.index;
    let layer_y_offset = layer_spawn.y_offset;
    let has_ceiling = layer_has_ceiling(layer_spawn);
    let above_grid = layer_spawn
        .level
        .layers
        .get(layer_spawn.index + 1)
        .map(|layer_def| layer_def.grid.as_slice());
    let char_defs: &[CharDef] = layer_spawn.level.char_defs.as_deref().unwrap_or(&[]);

    let panel_width = CELL_SIZE - FRAME_WIDTH * 2.0;
    let panel_height = WALL_HEIGHT - FRAME_WIDTH;

    let pillar_mesh = box_mesh(meshes, FRAME_WIDTH, WALL_HEIGHT, FRAME_DEPTH, false);
    let lintel_mesh = box_mesh(meshes, CELL_SIZE, FRAME_WIDTH, FRAME_DEPTH, false);
    let button_mesh = box_mesh(meshes, BUTTON_SIZE, BUTTON_SIZE, BUTTON_DEPTH, false);
    let panel_mesh = box_mesh(meshes, panel_width, panel_height, PANEL_DEPTH, true);
    // Half-depth counterparts for doors cut along the passage — each piece
    // keeps the full cross-section and half the depth, so the pair occupies
    // exactly the space the whole piece did.
    let half_frame_depth = FRAME_DEPTH / 2.0;
    let half_pillar_mesh = box_mesh(meshes, FRAME_WIDTH, WALL_HEIGHT, half_frame_depth, false);
    let half_lintel_mesh = box_mesh(meshes, CELL_SIZE, FRAME_WIDTH, half_frame_depth, false);
    let half_panel_mesh = box_mesh(meshes, panel_width, panel_height, PANEL_DEPTH / 2.0, true);

    for door in layer_state.doors.values() {
        let center_x = door.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = door.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let orientation = detect_door_orientation(grid, door.col, door.row, walkable);
        let rotation = if orientation == DoorOrientation::NS {
            Quat::from_rotation_y(FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };

        // Frame: two pillars and a lintel, plus call buttons on manual doors.
        let frame = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(center_x, layer_y_offset, center_z).with_rotation(rotation),
                Visibility::default(),
            ))
            .id();
        let split = door_split_zones(zones, layer_index, orientation, door.col, door.row);
        // Each half sits a quarter-depth either side of the frame's centre
        // plane, which is where the two zones meet.
        let half_offset = half_frame_depth / 2.0;

        let pillar_x = panel_width / 2.0 + FRAME_WIDTH / 2.0;
        for x in [-pillar_x, pillar_x] {
            if let Some((negative_zone, positive_zone)) = split {
                for (z, zone) in [(-half_offset, negative_zone), (half_offset, positive_zone)] {
                    let half = commands
                        .spawn((
                            Mesh3d(half_pillar_mesh.clone()),
                            MeshMaterial3d(materials.door_frame.clone()),
                            Transform::from_xyz(x, WALL_HEIGHT / 2.0, z),
                        ))
                        .id();
                    commands.entity(frame).add_child(half);
                    zones::tag_zone(commands, half, zone);
                }
            } else {
                let pillar = commands
                    .spawn((
                        Mesh3d(pillar_mesh.clone()),
                        MeshMaterial3d(materials.door_frame.clone()),
                        Transform::from_xyz(x, WALL_HEIGHT / 2.0, 0.0),
                    ))
                    .id();
                commands.entity(frame).add_child(pillar);
                zones::tag_cell(commands, zones, layer_index, pillar, door.col, door.row);
            }
        }
        let lintel_y = WALL_HEIGHT - FRAME_WIDTH / 2.0;
        if let Some((negative_zone, positive_zone)) = split {
            for (z, zone) in [(-half_offset, negative_zone), (half_offset, positive_zone)] {
                let half = commands
                    .spawn((
                        Mesh3d(half_lintel_mesh.clone()),
                        MeshMaterial3d(materials.door_frame.clone()),
                        Transform::from_xyz(0.0, lintel_y, z),
                    ))
                    .id();
                commands.entity(frame).add_child(half);
                zones::tag_zone(commands, half, zone);
            }
        } else {
            let lintel = commands
                .spawn((
                    Mesh3d(lintel_mesh.clone()),
                    MeshMaterial3d(materials.door_frame.clone()),
                    Transform::from_xyz(0.0, lintel_y, 0.0),
                ))
                .id();
            commands.entity(frame).add_child(lintel);
            zones::tag_cell(commands, zones, layer_index, lintel, door.col, door.row);
        }

        if !door.mechanical {
            for z in [
                FRAME_DEPTH / 2.0 + BUTTON_DEPTH / 2.0,
                -(FRAME_DEPTH / 2.0 + BUTTON_DEPTH / 2.0),
            ] {
                let button = commands
                    .spawn((
                        Mesh3d(button_mesh.clone()),
                        MeshMaterial3d(materials.door_button.clone()),
                        Transform::from_xyz(-pillar_x, BUTTON_HEIGHT, z),
                    ))
                    .id();
                commands.entity(frame).add_child(button);
                // A button is small enough to leave whole; it joins the zone
                // on the side it faces (`doorRenderer.ts:262-268`).
                match split {
                    Some((negative_zone, positive_zone)) => {
                        let zone = if z < 0.0 {
                            negative_zone
                        } else {
                            positive_zone
                        };
                        zones::tag_zone(commands, button, zone);
                    }
                    None => {
                        zones::tag_cell(commands, zones, layer_index, button, door.col, door.row);
                    }
                }
            }
        }

        // Panel: slides up when open, unless there's nothing solid above to
        // slide into, in which case it slides sideways into the wall plane.
        let material = if door.key_id.is_some() {
            materials.locked_door.clone()
        } else {
            materials.door.clone()
        };
        let slide_axis = slide_axis_for(
            orientation,
            ceiling_open_above(has_ceiling, above_grid, char_defs, door.col, door.row),
        );
        let (closed_val, open_val) = match slide_axis {
            SlideAxis::Y => (
                panel_height / 2.0 + layer_y_offset,
                WALL_HEIGHT + panel_height / 2.0 + layer_y_offset,
            ),
            SlideAxis::X => (center_x, center_x + panel_width + 0.05),
            SlideAxis::Z => (center_z, center_z - panel_width - 0.05),
        };
        let is_open = door.state == DoorState::Open;
        let target_val = if is_open { open_val } else { closed_val };
        let mut start = Vec3::new(center_x, panel_height / 2.0 + layer_y_offset, center_z);
        start[slide_axis.component_index()] = target_val;
        let panel_transform = Transform::from_translation(start).with_rotation(rotation);
        let door_panel = DoorPanel {
            axis: slide_axis,
            closed_val,
            open_val,
            target_val,
            bounce: None,
        };
        let panel = if let Some((negative_zone, positive_zone)) = split {
            // The animated entity carries no mesh of its own; the two halves
            // ride along as children, so the slide still moves one transform.
            let panel_root = commands
                .spawn((
                    LevelEntity,
                    panel_transform,
                    Visibility::default(),
                    door_panel,
                ))
                .id();
            let panel_half_offset = PANEL_DEPTH / 4.0;
            for (z, zone) in [
                (-panel_half_offset, negative_zone),
                (panel_half_offset, positive_zone),
            ] {
                let half = commands
                    .spawn((
                        Mesh3d(half_panel_mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_xyz(0.0, 0.0, z),
                    ))
                    .id();
                commands.entity(panel_root).add_child(half);
                zones::tag_zone(commands, half, zone);
            }
            panel_root
        } else {
            let panel = commands
                .spawn((
                    LevelEntity,
                    Mesh3d(panel_mesh.clone()),
                    MeshMaterial3d(material),
                    panel_transform,
                    door_panel,
                ))
                .id();
            zones::tag_cell(commands, zones, layer_index, panel, door.col, door.row);
            panel
        };
        panels.by_key.insert(
            layer_door_key(layer_index, &door_key(door.col, door.row)),
            panel,
        );

        if let Some((negative_zone, positive_zone)) = split {
            // TS picks the higher zone index as the indoor side
            // (`doorRenderer.ts:340`), which holds because
            // `buildEnvZoneMap` numbers zones in the order it first meets
            // them, starting from the level's own default environment.
            let indoor_zone = negative_zone.max(positive_zone);
            let light = commands
                .spawn((
                    LevelEntity,
                    DoorBoundaryLight { panel },
                    PointLight {
                        color: BOUNDARY_LIGHT_COLOR,
                        intensity: 0.0,
                        range: BOUNDARY_LIGHT_RANGE,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(center_x, WALL_HEIGHT * 0.7 + layer_y_offset, center_z),
                    Visibility::Hidden,
                ))
                .id();
            zones::tag_zone(commands, light, indoor_zone);
        }
    }

    panels
}

/// Drives each boundary doorway's light from how far its panel has slid, so
/// the light comes up as the door opens and fades as it closes
/// (`main.ts:1226-1231`). Intensity converts through
/// [`LUMENS_PER_THREE_UNIT`] like every other light in this port.
pub fn update_door_boundary_lights(
    mut lights: Query<(&DoorBoundaryLight, &mut PointLight, &mut Visibility)>,
    panels: Query<(&DoorPanel, &Transform)>,
) {
    for (boundary, mut light, mut visibility) in &mut lights {
        let Ok((panel, transform)) = panels.get(boundary.panel) else {
            continue;
        };
        let fraction = panel.open_fraction(transform);
        light.intensity =
            fraction * BOUNDARY_LIGHT_MAX_INTENSITY * crate::torch::LUMENS_PER_THREE_UNIT;
        *visibility = if fraction > BOUNDARY_LIGHT_MIN_FRACTION {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

pub fn animate_door_panels(time: Res<Time>, mut panels: Query<(&mut DoorPanel, &mut Transform)>) {
    let step = SLIDE_SPEED * time.delta_secs();
    let bounce_step = BOUNCE_SPEED * time.delta_secs();

    for (mut panel, mut transform) in &mut panels {
        let axis = panel.axis.component_index();
        let current = transform.translation[axis];

        if let Some(bounce) = panel.bounce {
            if bounce.reopening {
                let direction = if panel.open_val < current { -1.0 } else { 1.0 };
                let next = current + direction * bounce_step;
                if (direction < 0.0 && next <= panel.open_val)
                    || (direction > 0.0 && next >= panel.open_val)
                {
                    transform.translation[axis] = panel.open_val;
                    panel.bounce = None;
                } else {
                    transform.translation[axis] = next;
                }
            } else {
                let direction = if bounce.bounce_val < current {
                    -1.0
                } else {
                    1.0
                };
                let next = current + direction * bounce_step;
                if (direction < 0.0 && next <= bounce.bounce_val)
                    || (direction > 0.0 && next >= bounce.bounce_val)
                {
                    transform.translation[axis] = bounce.bounce_val;
                    panel.bounce = Some(DoorBounce {
                        reopening: true,
                        bounce_val: bounce.bounce_val,
                    });
                } else {
                    transform.translation[axis] = next;
                }
            }
            continue;
        }

        if (current - panel.target_val).abs() < 0.001 {
            continue;
        }
        transform.translation[axis] = if current < panel.target_val {
            (current + step).min(panel.target_val)
        } else {
            (current - step).max(panel.target_val)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> Vec<String> {
        rows.iter().map(ToString::to_string).collect()
    }

    /// A panel that hasn't moved reads as shut, one at its stop as fully
    /// open, and halfway as half — the curve the boundary light follows.
    #[test]
    fn open_fraction_tracks_the_panel_along_its_slide() {
        let panel = DoorPanel {
            axis: SlideAxis::Y,
            closed_val: 1.0,
            open_val: 3.0,
            target_val: 3.0,
            bounce: None,
        };
        let at = |y: f32| panel.open_fraction(&Transform::from_xyz(0.0, y, 0.0));
        assert_eq!(at(1.0), 0.0);
        assert_eq!(at(2.0), 0.5);
        assert_eq!(at(3.0), 1.0);
    }

    /// The bounce animation drives the panel past its stops, and a door with
    /// nowhere to slide has no range at all; neither may produce a fraction
    /// outside 0..1 for the light to multiply by.
    #[test]
    fn open_fraction_stays_within_range() {
        let panel = DoorPanel {
            axis: SlideAxis::Y,
            closed_val: 1.0,
            open_val: 3.0,
            target_val: 3.0,
            bounce: None,
        };
        assert_eq!(
            panel.open_fraction(&Transform::from_xyz(0.0, 0.0, 0.0)),
            0.0
        );
        assert_eq!(
            panel.open_fraction(&Transform::from_xyz(0.0, 9.0, 0.0)),
            1.0
        );

        let degenerate = DoorPanel {
            axis: SlideAxis::Y,
            closed_val: 2.0,
            open_val: 2.0,
            target_val: 2.0,
            bounce: None,
        };
        assert_eq!(
            degenerate.open_fraction(&Transform::from_xyz(0.0, 2.0, 0.0)),
            0.0
        );
    }

    #[test]
    fn ceiling_open_above_is_true_when_layer_has_no_ceiling() {
        assert!(ceiling_open_above(false, None, &[], 0, 0));
    }

    #[test]
    fn ceiling_open_above_is_false_when_no_layer_above_exists() {
        assert!(!ceiling_open_above(true, None, &[], 0, 0));
    }

    #[test]
    fn ceiling_open_above_is_false_under_a_hash_wall() {
        let above = grid(&["#."]);
        assert!(!ceiling_open_above(true, Some(&above), &[], 0, 0));
    }

    #[test]
    fn ceiling_open_above_is_true_under_a_walkable_cell() {
        let above = grid(&["#."]);
        assert!(ceiling_open_above(true, Some(&above), &[], 1, 0));
    }

    #[test]
    fn ceiling_open_above_is_false_under_a_solid_opaque_char_def() {
        let above = grid(&["W"]);
        let wall = CharDef {
            character: 'W',
            solid: true,
            see_through: None,
            textures: delve_core::types::TextureSet::default(),
        };
        assert!(!ceiling_open_above(true, Some(&above), &[wall], 0, 0));
    }

    #[test]
    fn ceiling_open_above_is_true_under_a_solid_see_through_char_def() {
        let above = grid(&["W"]);
        let grate = CharDef {
            character: 'W',
            solid: true,
            see_through: Some(true),
            textures: delve_core::types::TextureSet::default(),
        };
        assert!(ceiling_open_above(true, Some(&above), &[grate], 0, 0));
    }

    #[test]
    fn ceiling_open_above_is_false_when_door_cell_out_of_bounds() {
        let above = grid(&["#"]);
        assert!(!ceiling_open_above(true, Some(&above), &[], 5, 5));
        assert!(!ceiling_open_above(true, Some(&above), &[], -1, 0));
    }

    #[test]
    fn slide_axis_for_stays_vertical_with_something_solid_above() {
        assert_eq!(slide_axis_for(DoorOrientation::NS, false), SlideAxis::Y);
        assert_eq!(slide_axis_for(DoorOrientation::EW, false), SlideAxis::Y);
    }

    #[test]
    fn slide_axis_for_goes_sideways_along_the_doors_own_width_axis() {
        assert_eq!(slide_axis_for(DoorOrientation::NS, true), SlideAxis::Z);
        assert_eq!(slide_axis_for(DoorOrientation::EW, true), SlideAxis::X);
    }
}

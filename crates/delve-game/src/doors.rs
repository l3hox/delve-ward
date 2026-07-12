//! Door rendering: a stone frame per door cell plus a sliding panel that
//! animates open/closed, ported from the TS door renderer and animator.
//! Environment-zone splitting arrives with phase 5.

use crate::dungeon::{CELL_SIZE, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::textures::DungeonMaterials;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use delve_core::game_state::{DoorState, GameState, door_key};
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

/// Panel entities by door cell key, for open/close animation lookups.
#[derive(Resource, Default)]
pub struct DoorPanels {
    pub by_key: HashMap<String, Entity>,
}

#[derive(Debug, Clone, Copy)]
pub struct DoorBounce {
    reopening: bool,
    bounce_y: f32,
}

#[derive(Component)]
pub struct DoorPanel {
    pub closed_y: f32,
    pub open_y: f32,
    pub target_y: f32,
    bounce: Option<DoorBounce>,
}

impl DoorPanel {
    pub fn set_open(&mut self, open: bool) {
        self.target_y = if open { self.open_y } else { self.closed_y };
    }

    /// Animate the door closing 20% then bouncing back to open.
    pub fn bounce(&mut self) {
        let range = self.open_y - self.closed_y;
        self.bounce = Some(DoorBounce {
            reopening: false,
            bounce_y: self.open_y - range * BOUNCE_FRACTION,
        });
        self.target_y = self.open_y;
    }
}

/// Scale box UVs so each face samples texture proportional to its size,
/// preventing squeeze on thin dimensions. When `full_front_back` is set the
/// ±z faces keep their default 0..1 mapping (used for door panels).
pub(crate) fn fix_box_uvs(mesh: &mut Mesh, reference_size: f32, full_front_back: bool) {
    let positions: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return,
    };
    let normals: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        _ => return,
    };
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
        // Map local position to 0..(size/reference) per axis.
        uv[0] = position[u_axis] / reference_size + 0.5;
        uv[1] = position[v_axis] / reference_size + 0.5;
    }
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

pub fn spawn_doors(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &DungeonMaterials,
    game_state: &GameState,
    grid: &[String],
    walkable: &HashSet<char>,
) -> DoorPanels {
    let mut panels = DoorPanels::default();

    let panel_width = CELL_SIZE - FRAME_WIDTH * 2.0;
    let panel_height = WALL_HEIGHT - FRAME_WIDTH;

    let pillar_mesh = box_mesh(meshes, FRAME_WIDTH, WALL_HEIGHT, FRAME_DEPTH, false);
    let lintel_mesh = box_mesh(meshes, CELL_SIZE, FRAME_WIDTH, FRAME_DEPTH, false);
    let button_mesh = box_mesh(meshes, BUTTON_SIZE, BUTTON_SIZE, BUTTON_DEPTH, false);
    let panel_mesh = box_mesh(meshes, panel_width, panel_height, PANEL_DEPTH, true);

    for door in game_state.active_layer().doors.values() {
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
                Transform::from_xyz(center_x, 0.0, center_z).with_rotation(rotation),
                Visibility::default(),
            ))
            .id();
        let pillar_x = panel_width / 2.0 + FRAME_WIDTH / 2.0;
        for x in [-pillar_x, pillar_x] {
            let pillar = commands
                .spawn((
                    Mesh3d(pillar_mesh.clone()),
                    MeshMaterial3d(materials.door_frame.clone()),
                    Transform::from_xyz(x, WALL_HEIGHT / 2.0, 0.0),
                ))
                .id();
            commands.entity(frame).add_child(pillar);
        }
        let lintel = commands
            .spawn((
                Mesh3d(lintel_mesh.clone()),
                MeshMaterial3d(materials.door_frame.clone()),
                Transform::from_xyz(0.0, WALL_HEIGHT - FRAME_WIDTH / 2.0, 0.0),
            ))
            .id();
        commands.entity(frame).add_child(lintel);

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
            }
        }

        // Panel: slides up when open.
        let material = if door.key_id.is_some() {
            materials.locked_door.clone()
        } else {
            materials.door.clone()
        };
        let closed_y = panel_height / 2.0;
        let open_y = WALL_HEIGHT + panel_height / 2.0;
        let is_open = door.state == DoorState::Open;
        let start_y = if is_open { open_y } else { closed_y };
        let panel = commands
            .spawn((
                LevelEntity,
                Mesh3d(panel_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(center_x, start_y, center_z).with_rotation(rotation),
                DoorPanel {
                    closed_y,
                    open_y,
                    target_y: start_y,
                    bounce: None,
                },
            ))
            .id();
        panels.by_key.insert(door_key(door.col, door.row), panel);
    }

    panels
}

pub fn animate_door_panels(time: Res<Time>, mut panels: Query<(&mut DoorPanel, &mut Transform)>) {
    let step = SLIDE_SPEED * time.delta_secs();
    let bounce_step = BOUNCE_SPEED * time.delta_secs();

    for (mut panel, mut transform) in &mut panels {
        let current = transform.translation.y;

        if let Some(bounce) = panel.bounce {
            if bounce.reopening {
                let direction = if panel.open_y < current { -1.0 } else { 1.0 };
                let next = current + direction * bounce_step;
                if (direction < 0.0 && next <= panel.open_y)
                    || (direction > 0.0 && next >= panel.open_y)
                {
                    transform.translation.y = panel.open_y;
                    panel.bounce = None;
                } else {
                    transform.translation.y = next;
                }
            } else {
                let direction = if bounce.bounce_y < current { -1.0 } else { 1.0 };
                let next = current + direction * bounce_step;
                if (direction < 0.0 && next <= bounce.bounce_y)
                    || (direction > 0.0 && next >= bounce.bounce_y)
                {
                    transform.translation.y = bounce.bounce_y;
                    panel.bounce = Some(DoorBounce {
                        reopening: true,
                        bounce_y: bounce.bounce_y,
                    });
                } else {
                    transform.translation.y = next;
                }
            }
            continue;
        }

        if (current - panel.target_y).abs() < 0.001 {
            continue;
        }
        transform.translation.y = if current < panel.target_y {
            (current + step).min(panel.target_y)
        } else {
            (current - step).max(panel.target_y)
        };
    }
}

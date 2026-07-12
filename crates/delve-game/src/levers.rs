//! Wall levers, ported from the TS leverRenderer/leverAnimator: a metal base
//! plate with a pivoting handle and knob that swings between an up and down
//! angle when activated.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::sconces::wall_direction;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::{GameState, LeverState};
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;

const LEVER_HEIGHT: f32 = 1.2;
const BASE_SIZE: f32 = 0.15;
const HANDLE_LENGTH: f32 = 0.3;
const HANDLE_RADIUS: f32 = 0.02;

/// ~60 degrees above/below horizontal.
const ANGLE_UP: f32 = -1.047;
const ANGLE_DOWN: f32 = 1.047;
const LEVER_SPEED: f32 = 6.0;

fn angle_for(state: LeverState) -> f32 {
    if state == LeverState::Up {
        ANGLE_UP
    } else {
        ANGLE_DOWN
    }
}

/// The pivot's current and target rotation angle around the local X axis.
#[derive(Component)]
pub struct LeverPivot {
    pub current: f32,
    pub target: f32,
}

/// Pivot entities by lever cell key.
#[derive(Resource, Default)]
pub struct LeverHandles {
    pub by_key: HashMap<String, Entity>,
}

pub fn spawn_levers(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    game: &GameState,
) -> LeverHandles {
    let mut handles = LeverHandles::default();
    if game.active_layer().levers.is_empty() {
        return handles;
    }

    let base_mesh = meshes.add(Cuboid::new(BASE_SIZE, BASE_SIZE, 0.04));
    let handle_mesh = meshes.add(
        Cylinder::new(HANDLE_RADIUS, HANDLE_LENGTH)
            .mesh()
            .resolution(6),
    );
    let knob_mesh = meshes.add(Sphere { radius: 0.04 }.mesh().uv(6, 6));

    let lambert = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let base_material = materials.add(lambert(Color::srgb_u8(0x55, 0x55, 0x55)));
    let handle_material = materials.add(lambert(Color::srgb_u8(0x88, 0x66, 0x44)));
    let knob_material = materials.add(lambert(Color::srgb_u8(0x44, 0x44, 0x44)));

    for (key, lever) in &game.active_layer().levers {
        let center_x = lever.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = lever.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let (dir_x, dir_z, rotation_y) = wall_direction(lever.wall);
        let offset_dist = CELL_SIZE / 2.0 - 0.02;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(
                    center_x + dir_x * offset_dist,
                    LEVER_HEIGHT,
                    center_z + dir_z * offset_dist,
                )
                .with_rotation(Quat::from_rotation_y(rotation_y)),
                Visibility::default(),
            ))
            .id();

        let base = commands
            .spawn((
                Mesh3d(base_mesh.clone()),
                MeshMaterial3d(base_material.clone()),
                Transform::default(),
            ))
            .id();
        commands.entity(group).add_child(base);

        let angle = angle_for(lever.state);
        let pivot = commands
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.02).with_rotation(Quat::from_rotation_x(angle)),
                Visibility::default(),
                LeverPivot {
                    current: angle,
                    target: angle,
                },
            ))
            .id();
        commands.entity(group).add_child(pivot);

        let handle = commands
            .spawn((
                Mesh3d(handle_mesh.clone()),
                MeshMaterial3d(handle_material.clone()),
                Transform::from_xyz(0.0, 0.0, HANDLE_LENGTH / 2.0)
                    .with_rotation(Quat::from_rotation_x(FRAC_PI_2)),
            ))
            .id();
        commands.entity(pivot).add_child(handle);

        let knob = commands
            .spawn((
                Mesh3d(knob_mesh.clone()),
                MeshMaterial3d(knob_material.clone()),
                Transform::from_xyz(0.0, 0.0, HANDLE_LENGTH),
            ))
            .id();
        commands.entity(pivot).add_child(knob);

        handles.by_key.insert(key.clone(), pivot);
    }
    handles
}

/// Rendering-side handles for setting a lever's target angle from game logic
/// (interaction or a timed signal reset).
#[derive(SystemParam)]
pub struct LeverRender<'w, 's> {
    pub handles: Res<'w, LeverHandles>,
    pub pivots: Query<'w, 's, &'static mut LeverPivot>,
}

pub fn set_lever_target(render: &mut LeverRender, key: &str, state: LeverState) {
    if let Some(&entity) = render.handles.by_key.get(key)
        && let Ok(mut pivot) = render.pivots.get_mut(entity)
    {
        pivot.target = angle_for(state);
    }
}

/// Swing every lever pivot toward its target angle at `LEVER_SPEED`
/// radians/second.
pub fn animate_levers(time: Res<Time>, mut pivots: Query<(&mut LeverPivot, &mut Transform)>) {
    let step = LEVER_SPEED * time.delta_secs();
    for (mut pivot, mut transform) in &mut pivots {
        let diff = pivot.target - pivot.current;
        if diff.abs() < 0.01 {
            pivot.current = pivot.target;
        } else {
            pivot.current += diff.signum() * step.min(diff.abs());
        }
        transform.rotation = Quat::from_rotation_x(pivot.current);
    }
}

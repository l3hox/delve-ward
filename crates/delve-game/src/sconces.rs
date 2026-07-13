//! Wall sconces, ported from the TS sconceRenderer: an iron bracket and
//! torch per sconce with a flickering point light; taking the torch
//! (Space at the sconce wall) leaves the bare bracket and kills the light.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::torch::LUMENS_PER_THREE_UNIT;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::GameState;
use delve_core::grid::Facing;
use delve_core::random::Mulberry32;
use std::collections::HashMap;

const SCONCE_HEIGHT: f32 = 1.4;
const TORCH_COLOR: Color = Color::srgb_u8(0xff, 0x99, 0x4d);
const TORCH_INTENSITY: f32 = 2.5;
/// ~5 cells radius.
const TORCH_DISTANCE: f32 = 10.0;
const FLICKER_RANGE: f32 = 0.8;
const FLICKER_SPEED: f32 = 0.25;
const FLICKER_LERP: f32 = 0.15;

/// Wall direction: outward offset from the cell center plus the rotation
/// that faces the sconce into the room. Shared with levers, which mount on
/// walls the same way.
pub(crate) fn wall_direction(wall: Facing) -> (f32, f32, f32) {
    match wall {
        Facing::N => (0.0, -1.0, 0.0),
        Facing::S => (0.0, 1.0, std::f32::consts::PI),
        Facing::E => (1.0, 0.0, -std::f32::consts::FRAC_PI_2),
        Facing::W => (-1.0, 0.0, std::f32::consts::FRAC_PI_2),
    }
}

/// The torch handle and flame entities per sconce (hidden when taken) and
/// the light entities driven by the flicker.
#[derive(Resource, Default)]
pub struct SconceParts {
    pub torches: HashMap<String, [Entity; 2]>,
    pub lights: HashMap<String, Entity>,
}

#[derive(Component)]
pub struct SconceLight;

struct FlickerTarget {
    target: f32,
    timer: f32,
}

/// Per-light flicker state with random phase offsets so sconces don't
/// flicker in sync.
#[derive(Resource)]
pub struct SconceFlicker {
    rng: Mulberry32,
    targets: HashMap<String, FlickerTarget>,
}

impl Default for SconceFlicker {
    fn default() -> Self {
        Self {
            rng: Mulberry32::new(0x5C04_CE55),
            targets: HashMap::new(),
        }
    }
}

pub fn spawn_sconces(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    game: &GameState,
) -> SconceParts {
    let mut parts = SconceParts::default();
    if game.active_layer().sconces.is_empty() {
        return parts;
    }

    let bracket_mesh = meshes.add(Cuboid::new(0.08, 0.12, 0.15));
    let arm_mesh = meshes.add(Cuboid::new(0.04, 0.04, 0.18));
    let handle_mesh = meshes.add(
        ConicalFrustum {
            radius_top: 0.03,
            radius_bottom: 0.025,
            height: 0.35,
        }
        .mesh()
        .resolution(6),
    );
    let head_mesh = meshes.add(
        Cone {
            radius: 0.06,
            height: 0.12,
        }
        .mesh()
        .resolution(6),
    );

    let lambert = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let bracket_material = materials.add(lambert(Color::srgb_u8(0x44, 0x44, 0x44)));
    let arm_material = materials.add(lambert(Color::srgb_u8(0x55, 0x55, 0x55)));
    let handle_material = materials.add(lambert(Color::srgb_u8(0x66, 0x44, 0x22)));
    let flame_material = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0xff, 0xaa, 0x44),
        unlit: true,
        fog_enabled: false,
        ..default()
    });
    let dead_flame_material = materials.add(lambert(Color::srgb_u8(0x33, 0x22, 0x11)));

    for (key, sconce) in &game.active_layer().sconces {
        let center_x = sconce.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = sconce.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let (dir_x, dir_z, rotation_y) = wall_direction(sconce.wall);
        let offset_dist = CELL_SIZE / 2.0 - 0.02;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(
                    center_x + dir_x * offset_dist,
                    SCONCE_HEIGHT,
                    center_z + dir_z * offset_dist,
                )
                .with_rotation(Quat::from_rotation_y(rotation_y)),
                Visibility::default(),
            ))
            .id();

        let bracket = commands
            .spawn((
                Mesh3d(bracket_mesh.clone()),
                MeshMaterial3d(bracket_material.clone()),
                Transform::default(),
            ))
            .id();
        let arm = commands
            .spawn((
                Mesh3d(arm_mesh.clone()),
                MeshMaterial3d(arm_material.clone()),
                Transform::from_xyz(0.0, 0.0, 0.12),
            ))
            .id();
        let handle = commands
            .spawn((
                Mesh3d(handle_mesh.clone()),
                MeshMaterial3d(handle_material.clone()),
                Transform::from_xyz(0.0, 0.15, 0.2).with_rotation(Quat::from_rotation_x(0.3)),
            ))
            .id();
        let head = commands
            .spawn((
                Mesh3d(head_mesh.clone()),
                MeshMaterial3d(if sconce.lit {
                    flame_material.clone()
                } else {
                    dead_flame_material.clone()
                }),
                Transform::from_xyz(0.0, 0.35, 0.25),
            ))
            .id();
        commands
            .entity(group)
            .add_children(&[bracket, arm, handle, head]);
        parts.torches.insert(key.clone(), [handle, head]);

        if sconce.lit {
            let light = commands
                .spawn((
                    LevelEntity,
                    SconceLight,
                    PointLight {
                        color: TORCH_COLOR,
                        intensity: TORCH_INTENSITY * LUMENS_PER_THREE_UNIT,
                        range: TORCH_DISTANCE,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    // Slightly in front of the sconce, toward the room center.
                    Transform::from_xyz(
                        center_x + dir_x * (offset_dist - 0.4),
                        SCONCE_HEIGHT + 0.3,
                        center_z + dir_z * (offset_dist - 0.4),
                    ),
                ))
                .id();
            parts.lights.insert(key.clone(), light);
        }
    }
    parts
}

/// Rendering-side handles for extinguishing a sconce when its torch is taken.
#[derive(SystemParam)]
pub struct SconceRender<'w, 's> {
    pub parts: ResMut<'w, SconceParts>,
    pub visibility: Query<'w, 's, &'static mut Visibility>,
    pub lights: Query<'w, 's, &'static mut PointLight, With<SconceLight>>,
}

/// Hide the torch handle and flame (bracket and arm remain) and kill the
/// light.
pub fn extinguish_sconce(render: &mut SconceRender, key: &str) {
    if let Some(entities) = render.parts.torches.get(key) {
        for &entity in entities {
            if let Ok(mut visibility) = render.visibility.get_mut(entity) {
                *visibility = Visibility::Hidden;
            }
        }
    }
    if let Some(&entity) = render.parts.lights.get(key)
        && let Ok(mut light) = render.lights.get_mut(entity)
    {
        light.intensity = 0.0;
    }
}

pub fn sconce_flicker(
    time: Res<Time>,
    mut flicker: ResMut<SconceFlicker>,
    parts: Res<SconceParts>,
    mut lights: Query<&mut PointLight, With<SconceLight>>,
    gate: crate::overlay::InputGate,
) {
    // TS ticks sconce flicker inside the same overlay-paused block as the
    // other per-frame systems.
    if gate.paused() {
        return;
    }
    let SconceFlicker { rng, targets } = &mut *flicker;
    for (key, &entity) in &parts.lights {
        let Ok(mut light) = lights.get_mut(entity) else {
            continue;
        };
        if light.intensity == 0.0 {
            continue;
        }
        let state = targets.entry(key.clone()).or_insert_with(|| FlickerTarget {
            target: TORCH_INTENSITY,
            timer: rng.next_f64() as f32 * FLICKER_SPEED,
        });
        state.timer -= time.delta_secs();
        if state.timer <= 0.0 {
            state.target = TORCH_INTENSITY + (rng.next_f64() as f32 - 0.5) * FLICKER_RANGE * 2.0;
            state.timer = 0.03 + rng.next_f64() as f32 * FLICKER_SPEED;
        }
        let target_lumens = state.target * LUMENS_PER_THREE_UNIT;
        light.intensity += (target_lumens - light.intensity) * FLICKER_LERP;
    }
}

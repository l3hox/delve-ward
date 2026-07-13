//! Chest rendering: a wooden body with a hinged lid that swings open,
//! ported from the TS chestRenderer and its open/close lid animator.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use crate::textures::{canvas_to_image, seed_for};
use bevy::prelude::*;
use delve_core::game_state::{ChestState, LayerState, layer_door_key};
use delve_core::grid::Facing;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

const CHEST_WIDTH: f32 = 0.6;
const CHEST_DEPTH: f32 = 0.4;
const CHEST_BODY_HEIGHT: f32 = 0.3;
const CHEST_LID_HEIGHT: f32 = 0.12;
const CHEST_Y: f32 = 0.01; // just above floor
const CLASP_SIZE: (f32, f32, f32) = (0.08, 0.06, 0.02);
const OPEN_ANGLE: f32 = -FRAC_PI_4;
/// Matches the TS lid animation's 400ms duration as a constant angular speed.
const LID_SPEED: f32 = FRAC_PI_4 / 0.4;
const CLASP_COLOR: Color = Color::srgb_u8(0xcc, 0xaa, 0x33);

fn facing_rotation(facing: Facing) -> f32 {
    match facing {
        Facing::S => 0.0,
        Facing::W => FRAC_PI_2,
        Facing::N => PI,
        Facing::E => -FRAC_PI_2,
    }
}

fn generate_chest_texture(rng: &mut CanvasRng, is_lid: bool) -> PixelCanvas {
    const SIZE: i32 = 32;
    let mut canvas = PixelCanvas::new(SIZE as usize);
    let (base_red, base_green, base_blue) = if is_lid { (100, 60, 30) } else { (80, 45, 20) };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let variance = rng.below(20);
            canvas.fill_rect(
                x,
                y,
                1,
                1,
                Rgba::opaque(
                    (base_red + variance) as u8,
                    (base_green + variance) as u8,
                    (base_blue + variance) as u8,
                ),
            );
        }
    }
    let grain = Rgba::translucent(
        (base_red - 20).max(0) as u8,
        (base_green - 15).max(0) as u8,
        (base_blue - 10).max(0) as u8,
        0.3,
    );
    let mut y = 4;
    while y < SIZE {
        canvas.stroke_line(0, y + rng.below(2), SIZE, y + rng.below(2), grain);
        y += 6;
    }
    if !is_lid {
        canvas.fill_rect(0, 14, SIZE, 3, Rgba::translucent(60, 60, 65, 0.5));
    }
    canvas
}

/// The lid pivot's current and target swing angle (radians around X).
#[derive(Component)]
pub struct ChestLid {
    current_angle: f32,
    target_angle: f32,
}

/// Chest lid-pivot entities by cell key, for open/close animation lookups.
#[derive(Resource, Default)]
pub struct ChestHandles {
    pub by_key: HashMap<String, Entity>,
}

pub fn spawn_chests(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
) -> ChestHandles {
    let mut handles = ChestHandles::default();
    if layer_state.chests.is_empty() {
        return handles;
    }

    let lambert = |image: Handle<Image>| StandardMaterial {
        base_color_texture: Some(image),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let mut body_rng = CanvasRng::new(seed_for("chest_body"));
    let body_image = images.add(canvas_to_image(generate_chest_texture(
        &mut body_rng,
        false,
    )));
    let body_material = materials.add(lambert(body_image));
    let mut lid_rng = CanvasRng::new(seed_for("chest_lid"));
    let lid_image = images.add(canvas_to_image(generate_chest_texture(&mut lid_rng, true)));
    let lid_material = materials.add(lambert(lid_image));
    let clasp_material = materials.add(StandardMaterial {
        base_color: CLASP_COLOR,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    });

    let body_mesh = meshes.add(Cuboid::new(CHEST_WIDTH, CHEST_BODY_HEIGHT, CHEST_DEPTH));
    let lid_mesh = meshes.add(Cuboid::new(CHEST_WIDTH, CHEST_LID_HEIGHT, CHEST_DEPTH));
    let (clasp_width, clasp_height, clasp_depth) = CLASP_SIZE;
    let clasp_mesh = meshes.add(Cuboid::new(clasp_width, clasp_height, clasp_depth));

    for (key, chest) in &layer_state.chests {
        let center_x = chest.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = chest.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(center_x, CHEST_Y + layer_spawn.y_offset, center_z)
                    .with_rotation(Quat::from_rotation_y(facing_rotation(chest.facing))),
                Visibility::default(),
            ))
            .id();

        let body = commands
            .spawn((
                Mesh3d(body_mesh.clone()),
                MeshMaterial3d(body_material.clone()),
                Transform::from_xyz(0.0, CHEST_BODY_HEIGHT / 2.0, 0.0),
            ))
            .id();
        commands.entity(group).add_child(body);

        let clasp = commands
            .spawn((
                Mesh3d(clasp_mesh.clone()),
                MeshMaterial3d(clasp_material.clone()),
                Transform::from_xyz(0.0, CHEST_BODY_HEIGHT / 2.0, CHEST_DEPTH / 2.0 + 0.01),
            ))
            .id();
        commands.entity(group).add_child(clasp);

        let start_angle = if chest.state == ChestState::Open {
            OPEN_ANGLE
        } else {
            0.0
        };
        let lid_pivot = commands
            .spawn((
                Transform::from_xyz(0.0, CHEST_BODY_HEIGHT, -CHEST_DEPTH / 2.0)
                    .with_rotation(Quat::from_rotation_x(start_angle)),
                Visibility::default(),
                ChestLid {
                    current_angle: start_angle,
                    target_angle: start_angle,
                },
            ))
            .id();
        commands.entity(group).add_child(lid_pivot);

        let lid = commands
            .spawn((
                Mesh3d(lid_mesh.clone()),
                MeshMaterial3d(lid_material.clone()),
                Transform::from_xyz(0.0, CHEST_LID_HEIGHT / 2.0, CHEST_DEPTH / 2.0),
            ))
            .id();
        commands.entity(lid_pivot).add_child(lid);

        handles
            .by_key
            .insert(layer_door_key(layer_spawn.index, key), lid_pivot);
    }

    handles
}

pub fn animate_chest_lids(time: Res<Time>, mut lids: Query<(&mut ChestLid, &mut Transform)>) {
    let step = LID_SPEED * time.delta_secs();
    for (mut lid, mut transform) in &mut lids {
        if (lid.current_angle - lid.target_angle).abs() < 0.001 {
            continue;
        }
        lid.current_angle = if lid.current_angle < lid.target_angle {
            (lid.current_angle + step).min(lid.target_angle)
        } else {
            (lid.current_angle - step).max(lid.target_angle)
        };
        transform.rotation = Quat::from_rotation_x(lid.current_angle);
    }
}

fn set_chest_target(
    handles: &ChestHandles,
    lids: &mut Query<&mut ChestLid>,
    key: &str,
    open: bool,
) {
    let Some(&entity) = handles.by_key.get(key) else {
        return;
    };
    if let Ok(mut lid) = lids.get_mut(entity) {
        lid.target_angle = if open { OPEN_ANGLE } else { 0.0 };
    }
}

pub fn open_chest_mesh(handles: &ChestHandles, lids: &mut Query<&mut ChestLid>, key: &str) {
    set_chest_target(handles, lids, key, true);
}

pub fn close_chest_mesh(handles: &ChestHandles, lids: &mut Query<&mut ChestLid>, key: &str) {
    set_chest_target(handles, lids, key, false);
}

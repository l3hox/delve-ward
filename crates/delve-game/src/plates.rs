//! Pressure plates, ported from the TS plateRenderer: a floor slab with a
//! procedurally noisy texture that sinks and swaps to a cracked, flatter
//! texture while pressed.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use crate::textures::{canvas_to_image, seed_for};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::GameState;
use std::collections::HashMap;

const PLATE_SIZE: f32 = 0.8;
const PLATE_HEIGHT: f32 = 0.02;
/// Just above the floor.
const PLATE_Y: f32 = 0.01;
/// Sunk into the floor while pressed.
const PLATE_Y_PRESSED: f32 = -0.005;
const CANVAS_SIZE: usize = 32;

fn generate_plate_texture(rng: &mut CanvasRng, pressed: bool) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(CANVAS_SIZE);
    let (base, range) = if pressed { (40, 10) } else { (60, 20) };

    for y in 0..CANVAS_SIZE as i32 {
        for x in 0..CANVAS_SIZE as i32 {
            let v = base + rng.below(range);
            canvas.fill_rect(
                x,
                y,
                1,
                1,
                Rgba::opaque(v as u8, (v - 5) as u8, (v - 10) as u8),
            );
        }
    }

    if pressed {
        let crack = Rgba::translucent(20, 18, 16, 0.4);
        canvas.stroke_line(8, 0, 10, 32, crack);
        canvas.stroke_line(22, 0, 20, 32, crack);
    } else {
        let light_bevel = Rgba::translucent(120, 115, 110, 0.5);
        canvas.fill_rect(0, 0, 32, 2, light_bevel);
        canvas.fill_rect(0, 0, 2, 32, light_bevel);
        let dark_bevel = Rgba::translucent(20, 18, 16, 0.5);
        canvas.fill_rect(0, 30, 32, 2, dark_bevel);
        canvas.fill_rect(30, 0, 2, 32, dark_bevel);
    }
    canvas
}

/// Marks a plate's floor mesh so its render query stays disjoint from other
/// `Transform`/material queries in the same system.
#[derive(Component)]
pub struct PlateVisual;

/// Plate mesh entities by cell key, plus the two swappable materials.
#[derive(Resource, Default)]
pub struct PlateHandles {
    pub by_key: HashMap<String, Entity>,
    pub normal_material: Handle<StandardMaterial>,
    pub pressed_material: Handle<StandardMaterial>,
}

pub fn spawn_plates(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    game: &GameState,
) -> PlateHandles {
    let mut handles = PlateHandles::default();
    if game.active_layer().plates.is_empty() {
        return handles;
    }

    let mesh = meshes.add(Cuboid::new(PLATE_SIZE, PLATE_HEIGHT, PLATE_SIZE));
    let lambert_texture = |image: Handle<Image>| StandardMaterial {
        base_color_texture: Some(image),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let mut normal_rng = CanvasRng::new(seed_for("pressure_plate"));
    let normal_image = images.add(canvas_to_image(generate_plate_texture(
        &mut normal_rng,
        false,
    )));
    let normal_material = materials.add(lambert_texture(normal_image));
    let mut pressed_rng = CanvasRng::new(seed_for("pressure_plate_pressed"));
    let pressed_image = images.add(canvas_to_image(generate_plate_texture(
        &mut pressed_rng,
        true,
    )));
    let pressed_material = materials.add(lambert_texture(pressed_image));

    for (key, plate) in &game.active_layer().plates {
        let center_x = plate.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = plate.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let material = if plate.activated {
            pressed_material.clone()
        } else {
            normal_material.clone()
        };
        let y = if plate.activated {
            PLATE_Y_PRESSED
        } else {
            PLATE_Y
        };

        let entity = commands
            .spawn((
                LevelEntity,
                PlateVisual,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(center_x, y, center_z),
            ))
            .id();
        handles.by_key.insert(key.clone(), entity);
    }

    handles.normal_material = normal_material;
    handles.pressed_material = pressed_material;
    handles
}

/// Rendering-side handles for pressing/releasing a plate from game logic.
#[derive(SystemParam)]
pub struct PlateRender<'w, 's> {
    pub handles: Res<'w, PlateHandles>,
    pub visuals: Query<
        'w,
        's,
        (
            &'static mut MeshMaterial3d<StandardMaterial>,
            &'static mut Transform,
        ),
        With<PlateVisual>,
    >,
}

pub fn press_plate(render: &mut PlateRender, key: &str) {
    let Some(&entity) = render.handles.by_key.get(key) else {
        return;
    };
    let pressed_material = render.handles.pressed_material.clone();
    if let Ok((mut material, mut transform)) = render.visuals.get_mut(entity) {
        material.0 = pressed_material;
        transform.translation.y = PLATE_Y_PRESSED;
    }
}

pub fn release_plate(render: &mut PlateRender, key: &str) {
    let Some(&entity) = render.handles.by_key.get(key) else {
        return;
    };
    let normal_material = render.handles.normal_material.clone();
    if let Ok((mut material, mut transform)) = render.visuals.get_mut(entity) {
        material.0 = normal_material;
        transform.translation.y = PLATE_Y;
    }
}

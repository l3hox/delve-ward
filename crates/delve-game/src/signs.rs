//! Sign rendering: a static parchment plane mounted on a wall, ported from
//! the TS signRenderer. Signs never change after spawn, so there is no
//! handle map or per-frame system — reading one is handled entirely by
//! `interaction::interact`'s `SignRead` result (the text already lives on
//! `SignInstance`).

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use crate::textures::{canvas_to_image, seed_for};
use crate::zones::{self, LevelZones};
use bevy::prelude::*;
use delve_core::game_state::LayerState;
use delve_core::grid::Facing;
use std::f32::consts::{FRAC_PI_2, PI};

const SIGN_WIDTH: f32 = 0.4;
const SIGN_HEIGHT: f32 = 0.3;
const SIGN_Y: f32 = 1.1; // slightly below eye level
const OFFSET_DIST: f32 = CELL_SIZE / 2.0 - 0.01; // just off the wall surface

/// (dx, dz, rotation around Y) for the wall a sign is mounted on.
fn wall_direction(wall: Facing) -> (f32, f32, f32) {
    match wall {
        Facing::N => (0.0, -1.0, 0.0),
        Facing::S => (0.0, 1.0, PI),
        Facing::E => (1.0, 0.0, -FRAC_PI_2),
        Facing::W => (-1.0, 0.0, FRAC_PI_2),
    }
}

fn generate_sign_texture(rng: &mut CanvasRng) -> PixelCanvas {
    const SIZE: i32 = 32;
    let mut canvas = PixelCanvas::new(SIZE as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let variance = rng.below(15);
            canvas.fill_rect(
                x,
                y,
                1,
                1,
                Rgba::opaque(
                    (200 + variance) as u8,
                    (180 + variance) as u8,
                    (140 + variance) as u8,
                ),
            );
        }
    }
    canvas.stroke_rect(1, 1, SIZE - 2, SIZE - 2, Rgba::translucent(80, 60, 30, 0.6));
    let text_hint = Rgba::translucent(60, 40, 20, 0.3);
    let mut y = 8;
    while y < 28 {
        canvas.fill_rect(5, y, 22, 1, text_hint);
        y += 5;
    }
    canvas
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_signs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
    zones: &LevelZones,
) {
    if layer_state.signs.is_empty() {
        return;
    }

    let mut rng = CanvasRng::new(seed_for("sign"));
    let image = images.add(canvas_to_image(generate_sign_texture(&mut rng)));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(image),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        cull_mode: None,
        ..default()
    });
    let mesh = meshes.add(Rectangle::new(SIGN_WIDTH, SIGN_HEIGHT));

    for sign in layer_state.signs.values() {
        let center_x = sign.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = sign.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let (dx, dz, rotation_y) = wall_direction(sign.wall);

        let entity = commands
            .spawn((
                LevelEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(
                    center_x + dx * OFFSET_DIST,
                    SIGN_Y + layer_spawn.y_offset,
                    center_z + dz * OFFSET_DIST,
                )
                .with_rotation(Quat::from_rotation_y(rotation_y)),
            ))
            .id();
        zones::tag_cell(
            commands,
            zones,
            layer_spawn.index,
            entity,
            sign.col,
            sign.row,
        );
    }
}

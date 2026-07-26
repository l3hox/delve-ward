//! Key billboards, ported from the TS keyRenderer: a procedural gold-key
//! sprite per unpicked key, hidden on walk-over pickup.

use crate::billboard::FacesCamera;
use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::textures::canvas_to_image;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, layer_door_key};
use std::collections::HashMap;

const KEY_SIZE: f32 = 0.4;
/// Center of billboard just above ground.
const KEY_HEIGHT: f32 = KEY_SIZE / 2.0 + 0.02;
const CANVAS_SIZE: usize = 32;

/// Key billboard entities by the game state's key map key.
#[derive(Resource, Default)]
pub struct KeyBillboards {
    pub by_key: HashMap<String, Entity>,
}

/// Gold key on a transparent background, ported from the TS canvas drawing.
fn generate_key_texture() -> PixelCanvas {
    let mut canvas = PixelCanvas::new(CANVAS_SIZE);
    let gold = Rgba::opaque(0xda, 0xa5, 0x20);
    let black = Rgba::opaque(0x00, 0x00, 0x00);

    // Key ring with a dark hole.
    canvas.fill_ellipse(10.0, 16.0, 6.0, 6.0, gold);
    canvas.fill_ellipse(10.0, 16.0, 3.0, 3.0, black);

    // Shaft and teeth.
    canvas.fill_rect(16, 14, 12, 4, gold);
    canvas.fill_rect(24, 18, 4, 4, gold);
    canvas.fill_rect(20, 18, 3, 3, gold);
    canvas
}

pub fn spawn_keys(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    layer: &LayerState,
    layer_index: usize,
    layer_y_offset: f32,
) -> KeyBillboards {
    let mut billboards = KeyBillboards::default();
    if layer.keys.is_empty() {
        return billboards;
    }

    let texture = images.add(canvas_to_image(generate_key_texture()));
    let mesh = meshes.add(Rectangle::new(KEY_SIZE, KEY_SIZE));

    for (map_key, key_instance) in &layer.keys {
        if key_instance.picked_up {
            continue;
        }
        let center_x = key_instance.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = key_instance.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        // One material per key rather than one shared by all of them:
        // `billboard::apply_neutral_lighting` writes each sprite's own
        // brightness into its `base_color`, so a shared handle would leave
        // every key lit for whichever one was written last.
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(texture.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Mask(0.5),
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        let entity = commands
            .spawn((
                LevelEntity,
                FacesCamera,
                crate::billboard::NeutrallyLit,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(center_x, KEY_HEIGHT + layer_y_offset, center_z),
            ))
            .id();
        billboards
            .by_key
            .insert(layer_door_key(layer_index, map_key), entity);
    }
    billboards
}

pub fn hide_key_mesh(billboards: &mut KeyBillboards, commands: &mut Commands, key: &str) {
    if let Some(entity) = billboards.by_key.remove(key) {
        commands.entity(entity).despawn();
    }
}

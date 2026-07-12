//! Pushable block rendering: a stone cube that slides to its new cell when
//! pushed, ported from the TS blockRenderer.

use crate::dungeon::{CELL_SIZE, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use crate::textures::{canvas_to_image, seed_for};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::GameState;
use std::collections::HashMap;

const BLOCK_SIZE: f32 = CELL_SIZE * 0.85;
const BLOCK_HEIGHT: f32 = WALL_HEIGHT * 0.7;
/// Matches the TS push animation's 300ms duration as a constant linear speed.
const PUSH_SPEED: f32 = CELL_SIZE / 0.3;

fn generate_block_texture(rng: &mut CanvasRng) -> PixelCanvas {
    const SIZE: i32 = 32;
    let mut canvas = PixelCanvas::new(SIZE as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let v = 80 + rng.below(30);
            canvas.fill_rect(
                x,
                y,
                1,
                1,
                Rgba::opaque(v as u8, (v - 5).max(0) as u8, (v - 8).max(0) as u8),
            );
        }
    }
    let light_bevel = Rgba::translucent(140, 135, 130, 0.4);
    canvas.fill_rect(0, 0, SIZE, 2, light_bevel);
    canvas.fill_rect(0, 0, 2, SIZE, light_bevel);
    let dark_bevel = Rgba::translucent(30, 28, 26, 0.4);
    canvas.fill_rect(0, SIZE - 2, SIZE, 2, dark_bevel);
    canvas.fill_rect(SIZE - 2, 0, 2, SIZE, dark_bevel);
    canvas
}

/// The block's current slide target; `animate_blocks` eases translation
/// toward it every frame.
#[derive(Component)]
pub struct PushableBlock {
    target_x: f32,
    target_z: f32,
}

/// Block mesh entities by cell key, re-keyed as blocks are pushed.
#[derive(Resource, Default)]
pub struct BlockHandles {
    pub by_key: HashMap<String, Entity>,
}

pub fn spawn_blocks(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    game: &GameState,
) -> BlockHandles {
    let mut handles = BlockHandles::default();
    if game.active_layer().blocks.is_empty() {
        return handles;
    }

    let mut rng = CanvasRng::new(seed_for("block"));
    let image = images.add(canvas_to_image(generate_block_texture(&mut rng)));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(image),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    });
    let mesh = meshes.add(Cuboid::new(BLOCK_SIZE, BLOCK_HEIGHT, BLOCK_SIZE));

    for (key, block) in &game.active_layer().blocks {
        let center_x = block.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = block.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let entity = commands
            .spawn((
                LevelEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(center_x, BLOCK_HEIGHT / 2.0, center_z),
                PushableBlock {
                    target_x: center_x,
                    target_z: center_z,
                },
            ))
            .id();
        handles.by_key.insert(key.clone(), entity);
    }

    handles
}

/// Rendering-side handles for animating a push from game logic.
#[derive(SystemParam)]
pub struct BlockRender<'w, 's> {
    pub handles: ResMut<'w, BlockHandles>,
    pub blocks: Query<'w, 's, &'static mut PushableBlock>,
}

/// Re-keys the pushed block's mesh and sets its new slide target — the
/// per-frame ease toward that target happens in [`animate_blocks`].
pub fn animate_block_push(
    render: &mut BlockRender,
    from_key: &str,
    to_key: String,
    to_col: i64,
    to_row: i64,
) {
    let Some(entity) = render.handles.by_key.remove(from_key) else {
        return;
    };
    render.handles.by_key.insert(to_key, entity);
    if let Ok(mut block) = render.blocks.get_mut(entity) {
        block.target_x = to_col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        block.target_z = to_row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
    }
}

pub fn animate_blocks(time: Res<Time>, mut blocks: Query<(&PushableBlock, &mut Transform)>) {
    let step = PUSH_SPEED * time.delta_secs();
    for (block, mut transform) in &mut blocks {
        let current = transform.translation;
        let target = Vec3::new(block.target_x, current.y, block.target_z);
        let remaining = target - current;
        let distance = remaining.length();
        if distance < 0.001 {
            continue;
        }
        let move_amount = step.min(distance);
        transform.translation += remaining.normalize() * move_amount;
    }
}

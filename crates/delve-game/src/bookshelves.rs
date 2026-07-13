//! Bookshelf rendering: a wall-mounted case with three book spines — pure
//! static geometry, no interactive visual state at all (reading one is
//! handled entirely by `interaction::interact`'s `BookshelfRead` result,
//! same as `signs.rs`) — ported from `rendering/bookshelfRenderer.ts`.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::sconces::wall_direction;
use bevy::prelude::*;
use delve_core::game_state::GameState;

const BODY_SIZE: (f32, f32, f32) = (1.2, 1.8, 0.2);
const SPINE_SIZE: (f32, f32, f32) = (1.0, 0.1, 0.02);
const SPINE_Z: f32 = 0.11;
const SPINE_Y_OFFSETS: [f32; 3] = [-0.3, 0.0, 0.3];
const SHELF_Y: f32 = 0.9;

pub fn spawn_bookshelves(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    game: &GameState,
) {
    if game.active_layer().bookshelves.is_empty() {
        return;
    }

    let lambert = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let (body_w, body_h, body_d) = BODY_SIZE;
    let body_mesh = meshes.add(Cuboid::new(body_w, body_h, body_d));
    let body_material = materials.add(lambert(Color::srgb_u8(0x4a, 0x30, 0x20)));
    let (spine_w, spine_h, spine_d) = SPINE_SIZE;
    let spine_mesh = meshes.add(Cuboid::new(spine_w, spine_h, spine_d));
    let spine_materials = [
        materials.add(lambert(Color::srgb_u8(0xcc, 0x33, 0x33))),
        materials.add(lambert(Color::srgb_u8(0x33, 0x66, 0xcc))),
        materials.add(lambert(Color::srgb_u8(0x33, 0x99, 0x33))),
    ];

    for shelf in game.active_layer().bookshelves.values() {
        let center_x = shelf.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = shelf.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let (dir_x, dir_z, rotation_y) = wall_direction(shelf.wall);
        let offset_dist = CELL_SIZE / 2.0 - 0.1;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(
                    center_x + dir_x * offset_dist,
                    SHELF_Y,
                    center_z + dir_z * offset_dist,
                )
                .with_rotation(Quat::from_rotation_y(rotation_y)),
                Visibility::default(),
            ))
            .id();

        let body = commands
            .spawn((
                Mesh3d(body_mesh.clone()),
                MeshMaterial3d(body_material.clone()),
                Transform::default(),
            ))
            .id();
        commands.entity(group).add_child(body);

        for (index, &y_offset) in SPINE_Y_OFFSETS.iter().enumerate() {
            let spine = commands
                .spawn((
                    Mesh3d(spine_mesh.clone()),
                    MeshMaterial3d(spine_materials[index].clone()),
                    Transform::from_xyz(0.0, y_offset, SPINE_Z),
                ))
                .id();
            commands.entity(group).add_child(spine);
        }
    }
}

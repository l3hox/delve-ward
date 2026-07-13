//! NPC billboards, ported from the TS `rendering/npcRenderer.ts`: one static
//! sprite per NPC instance, facing the camera the same way enemy and key
//! billboards do. NPCs never move or despawn during a level's lifetime, so
//! there is no per-frame tick here — only spawn, matching how `keys.rs` has
//! no tick either.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, layer_door_key};
use delve_core::npcs::{DEFAULT_NPC_SPRITE_SIZE, NpcDatabase};
use std::collections::HashMap;

/// Shared NPC definitions, mirroring `enemies::EnemyDb`'s wrapper shape.
#[derive(Resource)]
pub struct NpcDb(pub std::sync::Arc<NpcDatabase>);

/// NPC billboard entities by the game state's NPC map key.
#[derive(Resource, Default)]
pub struct NpcBillboards {
    pub by_key: HashMap<String, Entity>,
}

fn sprite_asset_path(sprite_path: &str) -> String {
    sprite_path.trim_start_matches('/').to_string()
}

pub fn spawn_npc_billboards(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
    database: &NpcDatabase,
) -> NpcBillboards {
    let mut billboards = NpcBillboards::default();
    for (key, npc) in &layer_state.npcs {
        let def = database.get_npc(&npc.npc_id);
        let size = def
            .and_then(|def| def.sprite.size)
            .unwrap_or(DEFAULT_NPC_SPRITE_SIZE) as f32;
        let sprite_y_offset = def.and_then(|def| def.sprite.y_offset).unwrap_or(0.0) as f32;
        let texture_path = def
            .map(|def| sprite_asset_path(&def.sprite.path))
            .unwrap_or_else(|| "sprites/merchant.png".to_string());

        let material = materials.add(StandardMaterial {
            base_color_texture: Some(asset_server.load(texture_path)),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            ..default()
        });
        let center_x = npc.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = npc.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let entity = commands
            .spawn((
                LevelEntity,
                crate::billboard::FacesCamera,
                Mesh3d(meshes.add(Rectangle::new(size, size))),
                MeshMaterial3d(material),
                Transform::from_xyz(
                    center_x,
                    size * 0.5 + sprite_y_offset + layer_spawn.y_offset,
                    center_z,
                ),
            ))
            .id();
        billboards
            .by_key
            .insert(layer_door_key(layer_spawn.index, key), entity);
    }
    billboards
}

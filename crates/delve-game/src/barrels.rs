//! Barrel rendering: spawn-only geometry, despawned entirely on destroy —
//! no partial-damage visual stage in TS, so none is built here — ported
//! from `rendering/barrelRenderer.ts`.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use bevy::prelude::*;
use delve_core::game_state::{LayerState, layer_door_key};
use std::collections::HashMap;

/// TS's cylinder tapers slightly (top radius 0.38, bottom 0.34); Bevy's
/// `Cylinder` primitive has a single radius, so `spawn_barrels` averages
/// the two — a small cosmetic simplification with no gameplay dependency
/// on the exact profile.
const BODY_TOP_RADIUS: f32 = 0.38;
const BODY_BOTTOM_RADIUS: f32 = 0.34;
const BODY_HEIGHT: f32 = 0.8;
const BAND_RADIUS: f32 = 0.39;
const BAND_HEIGHT: f32 = 0.03;
const BAND_Y_OFFSETS: [f32; 2] = [-0.18, 0.18];
const BARREL_Y: f32 = 0.4;

/// Whole-barrel group entities by cell key, despawned on destroy.
#[derive(Resource, Default)]
pub struct BarrelHandles {
    by_key: HashMap<String, Entity>,
}

impl BarrelHandles {
    pub(crate) fn extend(&mut self, other: Self) {
        self.by_key.extend(other.by_key);
    }
}

pub fn spawn_barrels(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
) -> BarrelHandles {
    let mut handles = BarrelHandles::default();
    if layer_state.barrels.is_empty() {
        return handles;
    }

    let lambert = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let body_radius = (BODY_TOP_RADIUS + BODY_BOTTOM_RADIUS) / 2.0;
    let body_mesh = meshes.add(Cylinder::new(body_radius, BODY_HEIGHT));
    let body_material = materials.add(lambert(Color::srgb_u8(0x8b, 0x5e, 0x3c)));
    let band_mesh = meshes.add(Cylinder::new(BAND_RADIUS, BAND_HEIGHT));
    let band_material = materials.add(lambert(Color::srgb_u8(0x33, 0x33, 0x33)));

    for (key, barrel) in &layer_state.barrels {
        let center_x = barrel.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = barrel.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(center_x, BARREL_Y + layer_spawn.y_offset, center_z),
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

        for &y_offset in &BAND_Y_OFFSETS {
            let band = commands
                .spawn((
                    Mesh3d(band_mesh.clone()),
                    MeshMaterial3d(band_material.clone()),
                    Transform::from_xyz(0.0, y_offset, 0.0),
                ))
                .id();
            commands.entity(group).add_child(band);
        }

        handles
            .by_key
            .insert(layer_door_key(layer_spawn.index, key), group);
    }

    handles
}

/// Removes a destroyed barrel's mesh entirely — ported from TS's
/// `barrelMeshes.group.remove`/`meshMap.delete` pair in `inputSystem.ts`.
pub fn despawn_barrel(handles: &mut BarrelHandles, commands: &mut Commands, key: &str) {
    if let Some(entity) = handles.by_key.remove(key) {
        commands.entity(entity).despawn();
    }
}

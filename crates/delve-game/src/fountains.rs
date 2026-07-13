//! Fountain rendering: basin + pedestal + a translucent water disc, hidden
//! at spawn (or on use) — ported from `rendering/fountainRenderer.ts`. No
//! per-frame animation exists in TS (no ripple/shimmer) — the water disc is
//! a one-shot visibility toggle, the same shape as the lever/plate/tripwire
//! "hide/press mesh on state change" pattern from the phase-3 signal-entity
//! work.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::zones::{self, LevelZones};
use bevy::prelude::*;
use delve_core::game_state::{LayerState, UsableState, layer_door_key};
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;

const BASIN_RADIUS: f32 = 0.35;
const BASIN_HEIGHT: f32 = 0.5;
const BASIN_Y: f32 = 0.25;
const PEDESTAL_RADIUS: f32 = 0.08;
const PEDESTAL_HEIGHT: f32 = 0.6;
const PEDESTAL_Y: f32 = 0.3;
const WATER_RADIUS: f32 = 0.3;
const WATER_Y: f32 = 0.51;
const WATER_ALPHA: f32 = 0.6;

/// Water disc entities by fountain cell key, hidden on use.
#[derive(Resource, Default)]
pub struct FountainHandles {
    water_by_key: HashMap<String, Entity>,
}

impl FountainHandles {
    pub(crate) fn extend(&mut self, other: Self) {
        self.water_by_key.extend(other.water_by_key);
    }
}

pub fn spawn_fountains(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
    zones: &LevelZones,
) -> FountainHandles {
    let mut handles = FountainHandles::default();
    if layer_state.fountains.is_empty() {
        return handles;
    }

    let lambert = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let basin_mesh = meshes.add(Cylinder::new(BASIN_RADIUS, BASIN_HEIGHT));
    let basin_material = materials.add(lambert(Color::srgb_u8(0x88, 0x88, 0x88)));
    let pedestal_mesh = meshes.add(Cylinder::new(PEDESTAL_RADIUS, PEDESTAL_HEIGHT));
    let pedestal_material = materials.add(lambert(Color::srgb_u8(0x99, 0x99, 0x99)));
    let water_mesh = meshes.add(Circle::new(WATER_RADIUS));
    let water_material = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x44, 0x88, 0xcc).with_alpha(WATER_ALPHA),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    for (key, fountain) in &layer_state.fountains {
        let center_x = fountain.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = fountain.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(center_x, layer_spawn.y_offset, center_z),
                Visibility::default(),
            ))
            .id();

        let basin = commands
            .spawn((
                Mesh3d(basin_mesh.clone()),
                MeshMaterial3d(basin_material.clone()),
                Transform::from_xyz(0.0, BASIN_Y, 0.0),
            ))
            .id();
        commands.entity(group).add_child(basin);
        zones::tag_cell(
            commands,
            zones,
            layer_spawn.index,
            basin,
            fountain.col,
            fountain.row,
        );

        let pedestal = commands
            .spawn((
                Mesh3d(pedestal_mesh.clone()),
                MeshMaterial3d(pedestal_material.clone()),
                Transform::from_xyz(0.0, PEDESTAL_Y, 0.0),
            ))
            .id();
        commands.entity(group).add_child(pedestal);
        zones::tag_cell(
            commands,
            zones,
            layer_spawn.index,
            pedestal,
            fountain.col,
            fountain.row,
        );

        let water_visibility = if fountain.state == UsableState::Used {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        let water = commands
            .spawn((
                Mesh3d(water_mesh.clone()),
                MeshMaterial3d(water_material.clone()),
                Transform::from_xyz(0.0, WATER_Y, 0.0)
                    .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
                water_visibility,
            ))
            .id();
        commands.entity(group).add_child(water);
        zones::tag_cell(
            commands,
            zones,
            layer_spawn.index,
            water,
            fountain.col,
            fountain.row,
        );

        handles
            .water_by_key
            .insert(layer_door_key(layer_spawn.index, key), water);
    }

    handles
}

/// Hides a fountain's water disc on use — ported from TS's `markFountainUsed`.
pub fn mark_fountain_used(
    handles: &FountainHandles,
    visibility: &mut Query<&mut Visibility>,
    key: &str,
) {
    if let Some(&entity) = handles.water_by_key.get(key)
        && let Ok(mut visible) = visibility.get_mut(entity)
    {
        *visible = Visibility::Hidden;
    }
}

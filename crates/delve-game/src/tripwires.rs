//! Tripwires, ported from the TS tripwireRenderer: a thin, near-invisible
//! wire strung wall-to-wall across a cell. Hidden (not despawned, matching
//! the TS `mesh.visible = false`) once triggered — tripwires never reset.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use bevy::prelude::*;
use delve_core::game_state::{GameState, TripwireOrientation};
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;

const WIRE_HEIGHT: f32 = 0.25;
const WIRE_RADIUS: f32 = 0.008;

/// Wire mesh entities by cell key. Cells whose tripwire is already
/// `triggered` at spawn time are skipped entirely, matching the TS renderer.
#[derive(Resource, Default)]
pub struct TripwireHandles {
    pub by_key: HashMap<String, Entity>,
}

pub fn spawn_tripwires(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    game: &GameState,
) -> TripwireHandles {
    let mut handles = TripwireHandles::default();
    if game.active_layer().tripwires.is_empty() {
        return handles;
    }

    let mesh = meshes.add(Cylinder::new(WIRE_RADIUS, CELL_SIZE).mesh().resolution(4));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x44, 0x44, 0x44).with_alpha(0.1),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        fog_enabled: false,
        ..default()
    });

    for (key, tripwire) in &game.active_layer().tripwires {
        if tripwire.triggered {
            continue;
        }
        let center_x = tripwire.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = tripwire.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;

        // A vertical cylinder rotated flat (around Z) runs along X by
        // default; an extra Y rotation swings it to run along Z for NS.
        let extra_y = if tripwire.orientation == TripwireOrientation::NS {
            FRAC_PI_2
        } else {
            0.0
        };
        let rotation = Quat::from_rotation_y(extra_y) * Quat::from_rotation_z(FRAC_PI_2);

        let entity = commands
            .spawn((
                LevelEntity,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(center_x, WIRE_HEIGHT, center_z).with_rotation(rotation),
            ))
            .id();
        handles.by_key.insert(key.clone(), entity);
    }
    handles
}

pub fn hide_tripwire_mesh(handles: &TripwireHandles, commands: &mut Commands, key: &str) {
    if let Some(&entity) = handles.by_key.get(key) {
        commands.entity(entity).insert(Visibility::Hidden);
    }
}

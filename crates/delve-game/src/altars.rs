//! Altar rendering: a platform with a pillar that glows while the altar is
//! ready and goes dark once used — ported from `rendering/altarRenderer.ts`.
//! The temp buff itself has no dedicated visual anywhere in TS (it's
//! handled entirely by the unrelated status-effect system) — this module
//! only owns the glow.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use bevy::prelude::*;
use delve_core::game_state::{GameState, UsableState};
use std::collections::HashMap;

const PLATFORM_SIZE: (f32, f32, f32) = (1.2, 0.2, 1.2);
const PILLAR_SIZE: (f32, f32, f32) = (0.5, 0.55, 0.5);
const PILLAR_Y: f32 = 0.375;
const GROUP_Y: f32 = 0.1;
/// TS's `0x443300` at `emissiveIntensity: 0.5` — component-wise sRGB→linear
/// gamma-decoded, then scaled by the intensity. No prior-art conversion for
/// material emissive elsewhere in this codebase (`torch.rs`'s
/// `LUMENS_PER_THREE_UNIT` only covers point-light intensity), so this is a
/// one-off approximation rather than a shared constant; the glow is
/// decorative, not a value anything else depends on.
const GLOW_EMISSIVE: LinearRgba = LinearRgba::rgb(0.029, 0.017, 0.0);

/// Pillar entities by altar cell key, their glow removed on use.
#[derive(Resource, Default)]
pub struct AltarHandles {
    pillar_by_key: HashMap<String, Entity>,
}

pub fn spawn_altars(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    game: &GameState,
) -> AltarHandles {
    let mut handles = AltarHandles::default();
    if game.active_layer().altars.is_empty() {
        return handles;
    }

    let (platform_w, platform_h, platform_d) = PLATFORM_SIZE;
    let platform_mesh = meshes.add(Cuboid::new(platform_w, platform_h, platform_d));
    let platform_material = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x77, 0x77, 0x77),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    });
    let (pillar_w, pillar_h, pillar_d) = PILLAR_SIZE;
    let pillar_mesh = meshes.add(Cuboid::new(pillar_w, pillar_h, pillar_d));

    for (key, altar) in &game.active_layer().altars {
        let center_x = altar.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = altar.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(center_x, GROUP_Y, center_z),
                Visibility::default(),
            ))
            .id();

        let platform = commands
            .spawn((
                Mesh3d(platform_mesh.clone()),
                MeshMaterial3d(platform_material.clone()),
                Transform::default(),
            ))
            .id();
        commands.entity(group).add_child(platform);

        // Each altar gets its own material instance (not shared) so
        // `mark_altar_used` can mutate just this pillar's emissive.
        let pillar_material = materials.add(StandardMaterial {
            base_color: Color::srgb_u8(0x99, 0x99, 0x99),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            emissive: if altar.state == UsableState::Active {
                GLOW_EMISSIVE
            } else {
                LinearRgba::NONE
            },
            ..default()
        });
        let pillar = commands
            .spawn((
                Mesh3d(pillar_mesh.clone()),
                MeshMaterial3d(pillar_material),
                Transform::from_xyz(0.0, PILLAR_Y, 0.0),
            ))
            .id();
        commands.entity(group).add_child(pillar);

        handles.pillar_by_key.insert(key.clone(), pillar);
    }

    handles
}

/// Turns off a used altar's pillar glow — ported from TS's `markAltarUsed`.
/// `pillar_materials`' `Without<crate::plates::PlateVisual>` filter isn't
/// meaningful here (a pillar is never a plate visual) — it exists so the
/// caller's query can be proven disjoint from `PlateRender`'s own mutable
/// `MeshMaterial3d` query; see the field doc comment on
/// `session::InteractEffects::pillar_materials`.
pub fn mark_altar_used(
    handles: &AltarHandles,
    pillar_materials: &Query<
        &MeshMaterial3d<StandardMaterial>,
        Without<crate::plates::PlateVisual>,
    >,
    materials: &mut Assets<StandardMaterial>,
    key: &str,
) {
    let Some(&entity) = handles.pillar_by_key.get(key) else {
        return;
    };
    let Ok(material_handle) = pillar_materials.get(entity) else {
        return;
    };
    if let Some(mut material) = materials.get_mut(&material_handle.0) {
        material.emissive = LinearRgba::NONE;
    }
}

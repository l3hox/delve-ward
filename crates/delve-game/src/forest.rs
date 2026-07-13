//! Forest tree billboards, ported from `rendering/forestRenderer.ts`'s
//! rendering half — `delve_core::forest_placement::compute_forest_placements`
//! owns the pure cell-selection and per-cell RNG-draw math this module
//! consumes rather than recomputes.
//!
//! **Mechanism deviation, disclosed**: TS builds one `THREE.InstancedMesh`
//! per variant (up to four per layer) and rewrites every instance's matrix
//! in place each frame (`updateForestBillboards`). Bevy 0.19 has no
//! comparably lightweight primitive for an ad hoc, runtime-sized instance
//! batch, and shipped forest content is small (`forest_test.json`'s entire
//! grid yields well under a hundred trees) — so this module spawns one
//! entity per tree instead, the same pattern already used for enemy/
//! ground-item/key/NPC billboards. Per-variant mesh and material handles are
//! still cached and reused across every tree of that variant, so the asset
//! count stays at one geometry and one material per variant regardless of
//! tree count, matching TS's one-`InstancedMesh`-per-variant intent even
//! though the entity count doesn't.
//!
//! `updateForestBillboards`'s rotation is `camera.rotation.y` applied
//! identically to every instance — Y-axis only, driven by the camera's yaw,
//! no per-instance facing toward the camera position. That is exactly
//! `crate::billboard::face_billboards`'s own semantics (also `camera`
//! Y-yaw-only, applied to every `FacesCamera` entity), so trees are tagged
//! `FacesCamera` and reuse that system rather than getting a forest-specific
//! one.

use crate::billboard::FacesCamera;
use crate::dungeon::{CELL_SIZE, LayerSpawn};
use crate::level_scene::LevelEntity;
use bevy::prelude::*;
use delve_core::forest_placement::compute_forest_placements;
use delve_core::types::CharDef;

struct VariantSpec {
    path: &'static str,
    size: f32,
}

/// Ported from TS's `VARIANT_SPECS`. Index order matches
/// `delve_core::forest_placement`'s own `VARIANT_HEIGHTS` table (0=oak-thin,
/// 1=oak, 2=birch, 3=bushes) — `TreePlacement::variant_index` selects into
/// both tables identically. Every entry's TS width equals its height, so
/// this port keeps one `size` field rather than two.
const VARIANT_SPECS: [VariantSpec; 4] = [
    VariantSpec {
        path: "sprites/props/oak-thin.png",
        size: 2.85,
    },
    VariantSpec {
        path: "sprites/props/oak.png",
        size: 3.0,
    },
    VariantSpec {
        path: "sprites/props/birch.png",
        size: 2.7,
    },
    VariantSpec {
        path: "sprites/props/bushes.png",
        size: 2.1,
    },
];

/// Spawns every tree this layer's grid places, returning their entities so
/// the caller can zone-tag the whole batch (`zones::tag_forest` — forest
/// trees are tagged as one group per layer, not per cell, matching TS's own
/// `ldForestMeshes.group.traverse(...)` call in `buildLevelScene`).
pub fn spawn_forest(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    layer_spawn: &LayerSpawn,
) -> Vec<Entity> {
    let char_defs: &[CharDef] = layer_spawn.level.char_defs.as_deref().unwrap_or(&[]);
    let placements =
        compute_forest_placements(&layer_spawn.layer_def.grid, char_defs, f64::from(CELL_SIZE));

    let mut mesh_cache: [Option<Handle<Mesh>>; VARIANT_SPECS.len()] = Default::default();
    let mut material_cache: [Option<Handle<StandardMaterial>>; VARIANT_SPECS.len()] =
        Default::default();
    let mut entities = Vec::with_capacity(placements.len());

    for placement in &placements {
        let spec = &VARIANT_SPECS[placement.variant_index];
        let mesh = mesh_cache[placement.variant_index]
            .get_or_insert_with(|| meshes.add(Rectangle::new(spec.size, spec.size)))
            .clone();
        let material = material_cache[placement.variant_index]
            .get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color_texture: Some(asset_server.load(spec.path)),
                    perceptual_roughness: 1.0,
                    metallic: 0.0,
                    reflectance: 0.0,
                    alpha_mode: AlphaMode::Mask(0.5),
                    // TS's DoubleSide; in Bevy `double_sided` (normal
                    // flipping for back faces) is separate from disabling
                    // culling, and both are needed or a back-facing quad
                    // lights with the un-flipped front normal.
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })
            })
            .clone();

        let entity = commands
            .spawn((
                LevelEntity,
                FacesCamera,
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(
                    placement.x as f32,
                    placement.y as f32 + layer_spawn.y_offset,
                    placement.z as f32,
                ),
            ))
            .id();
        entities.push(entity);
    }

    entities
}

//! Decorative prop rendering (pillars, rubble, stalactites/stalagmites,
//! statues, crate stacks, banners) — static per-layer geometry with no
//! runtime visual state, ported from `rendering/propRenderer.ts`. TS's
//! `meshMap` has no consumer outside scene building (confirmed: nothing
//! calls `.propMeshes.meshMap.get(...)` anywhere, unlike keys/items/
//! tripwires/plates, which gameplay events do look up later) — so this
//! module returns nothing, no handle map, no resource wiring.
//!
//! Wall-mounted `banner`s use the same `WALL_DIR` table as `sconces.rs`'s
//! `wall_direction` — verified by reading `propRenderer.ts` directly rather
//! than assumed, since `bookshelfRenderer.ts` taught us these tables can
//! diverge (bookshelf's is rotated 180° from sconce's). `propRenderer.ts`'s
//! table is byte-for-byte identical to sconce's, so this reuses it instead
//! of porting a third identical copy.

use crate::dungeon::{CELL_SIZE, LayerSpawn, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::sconces::wall_direction;
use bevy::prelude::*;
use delve_core::game_state::LayerState;
use delve_core::grid::Facing;

const PILLAR_RADIUS: f32 = 0.25;

const RUBBLE_SIZE: (f32, f32, f32) = (0.15, 0.1, 0.12);
const RUBBLE_Y: f32 = 0.05;

const STALACTITE_RADIUS: f32 = 0.15;
const STALACTITE_HEIGHT: f32 = 0.6;
const STALACTITE_Y: f32 = WALL_HEIGHT - 0.3;

const STALAGMITE_RADIUS: f32 = 0.18;
const STALAGMITE_HEIGHT: f32 = 0.5;
const STALAGMITE_Y: f32 = 0.25;

const PEDESTAL_SIZE: (f32, f32, f32) = (0.5, 0.3, 0.5);
const PEDESTAL_Y: f32 = 0.15;
const TORSO_SIZE: (f32, f32, f32) = (0.3, 0.5, 0.2);
const TORSO_Y: f32 = 0.55;
const HEAD_SIZE: (f32, f32, f32) = (0.2, 0.2, 0.2);
const HEAD_Y: f32 = 0.9;

const CRATE_BOTTOM_SIZE: (f32, f32, f32) = (0.6, 0.4, 0.5);
const CRATE_BOTTOM_Y: f32 = 0.2;
const CRATE_TOP_SIZE: (f32, f32, f32) = (0.45, 0.35, 0.4);
/// Not centered on the bottom crate — matches TS's `top.position.set(0.05, 0.575, 0)`.
const CRATE_TOP_OFFSET: (f32, f32, f32) = (0.05, 0.575, 0.0);

const BANNER_SIZE: (f32, f32) = (0.5, 0.7);
const BANNER_Y: f32 = WALL_HEIGHT * 0.65;

fn lambert(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    }
}

/// `statue`/`crate_stack` only — `prop.rotation` is a quarter-turn count
/// (`(prop.rotation ?? 0) * Math.PI / 2` in TS), not raw degrees or
/// radians. Every other kind ignores the field entirely.
fn quarter_turn(rotation: Option<i64>) -> f32 {
    rotation.unwrap_or(0) as f32 * std::f32::consts::FRAC_PI_2
}

/// Deterministic per-piece scatter offset for `rubble`'s 4 pieces, ported
/// verbatim (including operator order) from `propRenderer.ts`'s inline
/// offset table so the same cell always scatters identically.
fn rubble_offset(col: i64, row: i64, index: usize) -> (f32, f32) {
    let terms: [(i64, i64); 4] = [
        (col * 7 + row * 13, col * 11 + row * 7),
        (col * 13 + row * 3, col * 5 + row * 11),
        (col * 3 + row * 17, col * 17 + row * 5),
        (col * 19 + row * 2, col * 2 + row * 19),
    ];
    let (x_term, z_term) = terms[index];
    (
        (x_term % 5) as f32 / 10.0 - 0.25,
        (z_term % 5) as f32 / 10.0 - 0.25,
    )
}

/// TS: `propGroup.rotation.y = (prop.rotation ?? 0) * Math.PI / 2;`,
/// set only for `statue`/`crate_stack` — every other kind's group stays
/// unrotated (including `banner`, whose rotation lives on its own mesh via
/// `wall_direction` instead).
fn group_rotation(prop_id: &str, rotation: Option<i64>) -> Quat {
    if matches!(prop_id, "statue" | "crate_stack") {
        Quat::from_rotation_y(quarter_turn(rotation))
    } else {
        Quat::IDENTITY
    }
}

pub fn spawn_props(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &LayerSpawn,
) {
    if layer_state.props.is_empty() {
        return;
    }

    let pillar_mesh = meshes.add(
        Cylinder::new(PILLAR_RADIUS, WALL_HEIGHT)
            .mesh()
            .resolution(8),
    );
    let pillar_material = materials.add(lambert(Color::srgb_u8(0x88, 0x88, 0x88)));

    let (rw, rh, rd) = RUBBLE_SIZE;
    let rubble_mesh = meshes.add(Cuboid::new(rw, rh, rd));
    let rubble_material = materials.add(lambert(Color::srgb_u8(0x66, 0x66, 0x66)));

    let stalactite_mesh = meshes.add(
        Cone {
            radius: STALACTITE_RADIUS,
            height: STALACTITE_HEIGHT,
        }
        .mesh()
        .resolution(6),
    );
    let stalagmite_mesh = meshes.add(
        Cone {
            radius: STALAGMITE_RADIUS,
            height: STALAGMITE_HEIGHT,
        }
        .mesh()
        .resolution(6),
    );
    // TS reuses `stalactiteMat` for stalagmites too — one material, two
    // meshes, not a separate `stalagmiteMat`.
    let stalactite_material = materials.add(lambert(Color::srgb_u8(0x77, 0x77, 0x66)));

    let (pw, ph, pd) = PEDESTAL_SIZE;
    let pedestal_mesh = meshes.add(Cuboid::new(pw, ph, pd));
    let pedestal_material = materials.add(lambert(Color::srgb_u8(0x55, 0x55, 0x55)));
    let (tw, th, td) = TORSO_SIZE;
    let torso_mesh = meshes.add(Cuboid::new(tw, th, td));
    let (hw, hh, hd) = HEAD_SIZE;
    let head_mesh = meshes.add(Cuboid::new(hw, hh, hd));
    let statue_material = materials.add(lambert(Color::srgb_u8(0x77, 0x77, 0x77)));

    let (cbw, cbh, cbd) = CRATE_BOTTOM_SIZE;
    let crate_bottom_mesh = meshes.add(Cuboid::new(cbw, cbh, cbd));
    let crate_bottom_material = materials.add(lambert(Color::srgb_u8(0x8b, 0x69, 0x14)));
    let (ctw, cth, ctd) = CRATE_TOP_SIZE;
    let crate_top_mesh = meshes.add(Cuboid::new(ctw, cth, ctd));
    let crate_top_material = materials.add(lambert(Color::srgb_u8(0x9b, 0x79, 0x24)));

    let (bw, bh) = BANNER_SIZE;
    let banner_mesh = meshes.add(Rectangle::new(bw, bh));
    // The only prop material that isn't a plain Lambert cuboid/cone —
    // TS sets `side: THREE.DoubleSide` since a single-sided plane mounted
    // flush against a wall would otherwise cull away from the room side.
    let banner_material = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(0x8b, 0x1a, 0x1a),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        double_sided: true,
        cull_mode: None,
        ..default()
    });

    for prop in layer_state.props.values() {
        let prop_id = prop.prop_id.as_str();
        // TS's `default: break;` — an unrecognized propId renders nothing
        // rather than falling back to some default shape.
        if !matches!(
            prop_id,
            "pillar" | "rubble" | "stalactite" | "stalagmite" | "statue" | "crate_stack" | "banner"
        ) {
            continue;
        }

        let center_x = prop.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = prop.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let root = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(center_x, layer_spawn.y_offset, center_z)
                    .with_rotation(group_rotation(prop_id, prop.rotation)),
                Visibility::default(),
            ))
            .id();

        match prop_id {
            "pillar" => {
                let mesh = commands
                    .spawn((
                        Mesh3d(pillar_mesh.clone()),
                        MeshMaterial3d(pillar_material.clone()),
                        Transform::from_xyz(0.0, WALL_HEIGHT / 2.0, 0.0),
                    ))
                    .id();
                commands.entity(root).add_child(mesh);
            }
            "rubble" => {
                for index in 0..4 {
                    let (dx, dz) = rubble_offset(prop.col, prop.row, index);
                    let mesh = commands
                        .spawn((
                            Mesh3d(rubble_mesh.clone()),
                            MeshMaterial3d(rubble_material.clone()),
                            Transform::from_xyz(dx, RUBBLE_Y, dz),
                        ))
                        .id();
                    commands.entity(root).add_child(mesh);
                }
            }
            "stalactite" => {
                let mesh = commands
                    .spawn((
                        Mesh3d(stalactite_mesh.clone()),
                        MeshMaterial3d(stalactite_material.clone()),
                        // Cone points up by default; flip it to hang down.
                        Transform::from_xyz(0.0, STALACTITE_Y, 0.0)
                            .with_rotation(Quat::from_rotation_z(std::f32::consts::PI)),
                    ))
                    .id();
                commands.entity(root).add_child(mesh);
            }
            "stalagmite" => {
                let mesh = commands
                    .spawn((
                        Mesh3d(stalagmite_mesh.clone()),
                        MeshMaterial3d(stalactite_material.clone()),
                        Transform::from_xyz(0.0, STALAGMITE_Y, 0.0),
                    ))
                    .id();
                commands.entity(root).add_child(mesh);
            }
            "statue" => {
                let pedestal = commands
                    .spawn((
                        Mesh3d(pedestal_mesh.clone()),
                        MeshMaterial3d(pedestal_material.clone()),
                        Transform::from_xyz(0.0, PEDESTAL_Y, 0.0),
                    ))
                    .id();
                let torso = commands
                    .spawn((
                        Mesh3d(torso_mesh.clone()),
                        MeshMaterial3d(statue_material.clone()),
                        Transform::from_xyz(0.0, TORSO_Y, 0.0),
                    ))
                    .id();
                let head = commands
                    .spawn((
                        Mesh3d(head_mesh.clone()),
                        MeshMaterial3d(statue_material.clone()),
                        Transform::from_xyz(0.0, HEAD_Y, 0.0),
                    ))
                    .id();
                commands.entity(root).add_children(&[pedestal, torso, head]);
            }
            "crate_stack" => {
                let (ox, oy, oz) = CRATE_TOP_OFFSET;
                let bottom = commands
                    .spawn((
                        Mesh3d(crate_bottom_mesh.clone()),
                        MeshMaterial3d(crate_bottom_material.clone()),
                        Transform::from_xyz(0.0, CRATE_BOTTOM_Y, 0.0),
                    ))
                    .id();
                let top = commands
                    .spawn((
                        Mesh3d(crate_top_mesh.clone()),
                        MeshMaterial3d(crate_top_material.clone()),
                        Transform::from_xyz(ox, oy, oz),
                    ))
                    .id();
                commands.entity(root).add_children(&[bottom, top]);
            }
            "banner" => {
                // TS: `prop.wall ?? 'N'` — the only prop kind that reads
                // `wall` at all; every other kind ignores it even if present.
                let (dir_x, dir_z, wall_rotation_y) =
                    wall_direction(prop.wall.unwrap_or(Facing::N));
                let offset_dist = CELL_SIZE / 2.0 - 0.02;
                let mesh = commands
                    .spawn((
                        Mesh3d(banner_mesh.clone()),
                        MeshMaterial3d(banner_material.clone()),
                        Transform::from_xyz(dir_x * offset_dist, BANNER_Y, dir_z * offset_dist)
                            .with_rotation(Quat::from_rotation_y(wall_rotation_y)),
                    ))
                    .id();
                commands.entity(root).add_child(mesh);
            }
            // Filtered by the `matches!` guard above — every reachable
            // `prop_id` value has its own arm.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_offset(actual: (f32, f32), expected: (f32, f32)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-6 && (actual.1 - expected.1).abs() < 1e-6,
            "offset {actual:?} != expected {expected:?}"
        );
    }

    /// Hand-computed from `propRenderer.ts`'s inline offset table for
    /// col=3, row=5: piece 0 x-term `3*7+5*13 = 86`, `86 % 5 = 1`,
    /// `1/10 - 0.25 = -0.15`, and so on term by term.
    #[test]
    fn rubble_offsets_match_the_ts_scatter_table_for_a_reference_cell() {
        assert_offset(rubble_offset(3, 5, 0), (-0.15, 0.05));
        assert_offset(rubble_offset(3, 5, 1), (0.15, -0.25));
        assert_offset(rubble_offset(3, 5, 2), (0.15, -0.15));
        assert_offset(rubble_offset(3, 5, 3), (-0.05, -0.15));
    }

    #[test]
    fn rubble_offsets_stay_within_the_quarter_cell_scatter_bounds() {
        for index in 0..4 {
            let (x, z) = rubble_offset(12, 7, index);
            assert!((-0.25..=0.25).contains(&x));
            assert!((-0.25..=0.25).contains(&z));
        }
    }

    #[test]
    fn quarter_turn_converts_rotation_counts_to_radians() {
        assert_eq!(quarter_turn(None), 0.0);
        assert_eq!(quarter_turn(Some(1)), std::f32::consts::FRAC_PI_2);
        assert_eq!(quarter_turn(Some(3)), 3.0 * std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn only_statues_and_crate_stacks_honor_the_rotation_field() {
        let quarter = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert_eq!(group_rotation("statue", Some(1)), quarter);
        assert_eq!(group_rotation("crate_stack", Some(1)), quarter);
        assert_eq!(group_rotation("pillar", Some(1)), Quat::IDENTITY);
        assert_eq!(group_rotation("banner", Some(1)), Quat::IDENTITY);
    }
}

//! Trap launcher bodies, ported from the TS `trapLauncherRenderer`: a dark
//! iron block mounted against the wall behind the launcher, with a darker
//! slab sitting proud of its front face that reads as the nozzle opening.
//! Purely decorative — firing, ticking, and hit resolution all live in
//! `projectiles.rs`; this module only gives the shots something to leave.
//!
//! Deliberately untagged by environment zone. TS traverses the whole trap
//! launcher group calling `child.layers.enableAll()`
//! (`levelSceneBuilder.ts:533`), which in this port means leaving the meshes
//! on Bevy's default render layer 0 — already in every zone camera's
//! `[0, zone]` set. See `zones.rs`, which documents that convention for the
//! rest of TS's `enableAll` list.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::sconces::wall_direction;
use bevy::prelude::*;
use delve_core::game_state::LayerState;
use delve_core::grid::Facing;

/// Deliberately equal to `projectiles::PROJECTILE_HEIGHT` so a shot leaves
/// the nozzle rather than above or below it (`trapLauncherRenderer.ts:6`).
const LAUNCHER_HEIGHT: f32 = 1.2;

const BODY_WIDTH: f32 = 0.20;
const BODY_HEIGHT: f32 = 0.15;
const BODY_DEPTH: f32 = 0.10;

const NOZZLE_WIDTH: f32 = 0.10;
const NOZZLE_HEIGHT: f32 = 0.07;
/// Thin dark slab, sat proud of the body face rather than sunk into it.
const NOZZLE_DEPTH: f32 = 0.02;

/// The wall the body is bolted to. A launcher stands in a walkable cell and
/// fires *through* the wall ahead of it, so the body sits against the wall
/// OPPOSITE the firing direction — `trapLauncherRenderer.ts:31`'s `OPPOSITE`
/// map, `{ N: 'S', S: 'N', E: 'W', W: 'E' }`.
fn mount_wall(facing: Facing) -> Facing {
    match facing {
        Facing::N => Facing::S,
        Facing::E => Facing::W,
        Facing::S => Facing::N,
        Facing::W => Facing::E,
    }
}

/// The unit offset from the cell center toward the mount wall, plus the Y
/// rotation that turns the body's local +Z face (where the nozzle sits) down
/// the firing direction. Pure so the opposite-wall composition can be pinned
/// by tests — it is the one piece here that silently produces a launcher
/// embedded in the wrong wall when reversed.
fn mount_direction(facing: Facing) -> (f32, f32, f32) {
    wall_direction(mount_wall(facing))
}

pub fn spawn_trap_launchers(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    layer: &LayerState,
    layer_y_offset: f32,
) {
    if layer.trap_launchers.is_empty() {
        return;
    }

    let body_mesh = meshes.add(Cuboid::new(BODY_WIDTH, BODY_HEIGHT, BODY_DEPTH));
    let nozzle_mesh = meshes.add(Cuboid::new(NOZZLE_WIDTH, NOZZLE_HEIGHT, NOZZLE_DEPTH));

    let lambert = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        ..default()
    };
    let body_material = materials.add(lambert(Color::srgb_u8(0x33, 0x33, 0x33)));
    let nozzle_material = materials.add(lambert(Color::srgb_u8(0x11, 0x11, 0x11)));

    for launcher in layer.trap_launchers.values() {
        let center_x = launcher.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = launcher.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let (dir_x, dir_z, rotation_y) = mount_direction(launcher.facing);
        // Centers the body so its back face lands flush on the wall surface.
        let offset_dist = CELL_SIZE / 2.0 - BODY_DEPTH / 2.0;

        let group = commands
            .spawn((
                LevelEntity,
                Transform::from_xyz(
                    center_x + dir_x * offset_dist,
                    LAUNCHER_HEIGHT + layer_y_offset,
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
        let nozzle = commands
            .spawn((
                Mesh3d(nozzle_mesh.clone()),
                MeshMaterial3d(nozzle_material.clone()),
                Transform::from_xyz(0.0, 0.0, BODY_DEPTH / 2.0 + NOZZLE_DEPTH / 2.0),
            ))
            .id();
        commands.entity(group).add_children(&[body, nozzle]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_FACING: [Facing; 4] = [Facing::N, Facing::E, Facing::S, Facing::W];

    /// TS's `OPPOSITE` table verbatim (`trapLauncherRenderer.ts:31`).
    #[test]
    fn mount_wall_is_the_wall_behind_the_firing_direction() {
        assert_eq!(mount_wall(Facing::N), Facing::S);
        assert_eq!(mount_wall(Facing::S), Facing::N);
        assert_eq!(mount_wall(Facing::E), Facing::W);
        assert_eq!(mount_wall(Facing::W), Facing::E);
    }

    /// The offset must push the body *backwards*, away from where it shoots:
    /// a north-firing launcher hangs on the south wall. Checked against
    /// `Facing::delta`, the core's own firing-direction source, so a reversed
    /// mapping fails here rather than showing up as a launcher buried in the
    /// wall its darts pass through.
    #[test]
    fn mount_direction_offsets_opposite_the_firing_direction() {
        for facing in EVERY_FACING {
            let (dir_x, dir_z, _) = mount_direction(facing);
            let (fire_col, fire_row) = facing.delta();
            assert_eq!(
                (dir_x, dir_z),
                (-(fire_col as f32), -(fire_row as f32)),
                "{facing:?} launcher must mount on the wall behind it"
            );
        }
    }

    /// The nozzle slab sits at the body's local +Z; after the group's Y
    /// rotation that face must aim down the firing direction, or projectiles
    /// leave out of the back of the launcher.
    #[test]
    fn mount_direction_aims_the_nozzle_face_down_the_firing_direction() {
        for facing in EVERY_FACING {
            let (_, _, rotation_y) = mount_direction(facing);
            let nozzle = Quat::from_rotation_y(rotation_y) * Vec3::Z;
            let (fire_col, fire_row) = facing.delta();
            assert!(
                (nozzle.x - fire_col as f32).abs() < 1e-5
                    && (nozzle.z - fire_row as f32).abs() < 1e-5,
                "{facing:?} nozzle points {nozzle:?}, expected ({fire_col}, {fire_row})"
            );
        }
    }
}

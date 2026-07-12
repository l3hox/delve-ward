//! Camera-facing billboards: one marker and one system shared by enemy,
//! ground item, and key sprites.

use crate::player::Player;
use bevy::prelude::*;

#[derive(Component)]
pub struct FacesCamera;

/// All marked sprites face the camera's view plane (its yaw, not the camera
/// point).
pub fn face_billboards(
    cameras: Query<&Transform, (With<Player>, Without<FacesCamera>)>,
    mut billboards: Query<&mut Transform, With<FacesCamera>>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    let (yaw, _, _) = camera.rotation.to_euler(EulerRot::YXZ);
    for mut transform in &mut billboards {
        transform.rotation = Quat::from_rotation_y(yaw);
    }
}

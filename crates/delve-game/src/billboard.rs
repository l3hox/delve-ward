//! Camera-facing billboards: one marker and one system shared by enemy,
//! ground item, and key sprites, plus the neutral lighting those sprites use
//! in place of ordinary shading.

use crate::player::Player;
use crate::torch::LUMENS_PER_THREE_UNIT;
use bevy::prelude::*;

#[derive(Component)]
pub struct FacesCamera;

/// Rec. 601 luma weights, the ones TS's billboard shader uses to reduce a
/// light's color to a single brightness.
const LUMA: Vec3 = Vec3::new(0.299, 0.587, 0.114);

/// TS's own ceiling on accumulated brightness, "to avoid overexposure near
/// light sources" (`rendering/billboardMaterial.ts:65`).
const MAX_INTENSITY: f32 = 1.2;

/// A sprite lit by brightness alone, never by the color of the light.
///
/// TS gives every item, enemy, and NPC billboard a hand-written shader
/// (`rendering/billboardMaterial.ts`) that sums each light's *luminance* —
/// `dot(light.color, LUMA)` — and multiplies the texture by that one scalar.
/// The sprite therefore dims with distance from a torch but never takes on
/// its color. Ordinary shading multiplies by the light's color instead, which
/// is why these sprites turned brown near a torch: the art was being tinted by
/// firelight the original deliberately desaturates.
///
/// The shader has no per-fragment variation at all — it lights from the
/// object's center, with no surface-normal term (its own comment: "no NdotL,
/// no per-vertex variation") — so the same result comes from computing one
/// scalar per sprite on the CPU and handing it to an unlit material as its
/// base color. That keeps the port to a system plus a component instead of a
/// second shader, and unlit materials still receive distance fog
/// (`main_pass_post_lighting_processing` in Bevy's own `pbr_functions.wgsl`
/// applies fog outside the lighting branch), which the TS shader also does.
#[derive(Component)]
pub struct NeutrallyLit;

/// THREE's `getDistanceAttenuation` with its default decay of 2
/// (`lights_pars_begin.glsl.js:56-68`): inverse-square falloff, then a
/// quartic window that eases the light out to nothing at its cutoff distance
/// instead of clipping.
fn distance_attenuation(distance: f32, cutoff: f32) -> f32 {
    let falloff = 1.0 / (distance * distance).max(0.01);
    if cutoff <= 0.0 {
        return falloff;
    }
    let ratio = (distance / cutoff).clamp(0.0, 1.0);
    let window = (1.0 - ratio.powi(4)).clamp(0.0, 1.0);
    falloff * window * window
}

fn luminance(color: Color) -> f32 {
    let linear = color.to_linear();
    Vec3::new(linear.red, linear.green, linear.blue).dot(LUMA)
}

/// Recomputes each neutrally lit sprite's brightness from the lights around
/// it. Point-light intensities divide back out of [`LUMENS_PER_THREE_UNIT`]
/// so the sum stays in the Three.js units TS's clamp was written against.
pub fn apply_neutral_lighting(
    ambient: Single<&AmbientLight>,
    lights: Query<(&PointLight, &GlobalTransform)>,
    sprites: Query<(&GlobalTransform, &MeshMaterial3d<StandardMaterial>), With<NeutrallyLit>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ambient_intensity = luminance(ambient.color) * ambient.brightness / LUMENS_PER_THREE_UNIT;
    for (sprite_transform, material_handle) in &sprites {
        let center = sprite_transform.translation();
        let mut intensity = ambient_intensity;
        for (light, light_transform) in &lights {
            let distance = center.distance(light_transform.translation());
            intensity += luminance(light.color)
                * (light.intensity / LUMENS_PER_THREE_UNIT)
                * distance_attenuation(distance, light.range);
        }
        let intensity = intensity.min(MAX_INTENSITY);
        if let Some(mut material) = materials.get_mut(&material_handle.0) {
            material.base_color = Color::linear_rgb(intensity, intensity, intensity);
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::{LUMA, distance_attenuation, luminance};
    use bevy::prelude::Color;

    /// The whole point of the neutral shader: lights of equal brightness but
    /// different hue must light a sprite identically, so a warm torch
    /// brightens the art without staining it. Pure red and the green that
    /// carries the same luma weight are the sharpest case.
    #[test]
    fn equal_luminance_lights_of_different_hue_are_interchangeable() {
        let red = luminance(Color::linear_rgb(1.0, 0.0, 0.0));
        let matched_green = luminance(Color::linear_rgb(0.0, LUMA.x / LUMA.y, 0.0));
        assert!(
            (red - matched_green).abs() < 1e-6,
            "{red} vs {matched_green}"
        );
    }

    /// Inverse-square between the near clamp and the cutoff.
    #[test]
    fn attenuation_falls_off_with_the_square_of_distance() {
        let near = distance_attenuation(1.0, 0.0);
        let far = distance_attenuation(2.0, 0.0);
        assert!((near / far - 4.0).abs() < 1e-4, "{near} vs {far}");
    }

    /// THREE's quartic window drives the light to exactly nothing at its
    /// cutoff rather than clipping, so a sprite doesn't pop as it leaves a
    /// torch's range.
    #[test]
    fn attenuation_reaches_zero_at_the_cutoff() {
        assert_eq!(distance_attenuation(12.0, 12.0), 0.0);
        assert!(distance_attenuation(11.5, 12.0) > 0.0);
        assert!(distance_attenuation(11.5, 12.0) < distance_attenuation(6.0, 12.0));
    }

    /// Past the cutoff the window is clamped, never negative — a squared
    /// negative would otherwise make distant lights brighten again.
    #[test]
    fn attenuation_stays_zero_beyond_the_cutoff() {
        assert_eq!(distance_attenuation(50.0, 12.0), 0.0);
    }
}

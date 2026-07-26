//! Player torch: a main point light at the camera and a fill light pushed
//! toward the cell center, with random flicker. Range and flicker intensity
//! scale with `torch_fuel`, so a guttering torch lights less of the room and
//! flickers less wildly than a fresh one — the mapping is `main.ts:1395-1409`.

use crate::player::Player;
use crate::session::Session;
use bevy::prelude::*;
use delve_core::random::Mulberry32;

const TORCH_COLOR: Color = Color::srgb_u8(0xff, 0x99, 0x4d);
const TORCH_OFFSET_Y: f32 = 0.3;
/// Below this fuel ratio, torch range and flicker intensity fade linearly
/// toward zero; at or above it they stay at the full-fuel values.
const FUEL_FADE_THRESHOLD: f32 = 0.35;
const TORCH_RANGE_BASE: f32 = 4.5;
const TORCH_RANGE_SCALE: f32 = 7.5;
const FILL_RANGE_BASE: f32 = 3.0;
const FILL_RANGE_SCALE: f32 = 6.0;
const FLICKER_INTENSITY_BASE: f32 = 1.2;
const FLICKER_INTENSITY_SCALE: f32 = 4.2;
/// Full-fuel (`light_scale` == 1) ranges from the TS torch distance
/// formulas, used for the initial spawn before the first fuel read.
const TORCH_RANGE: f32 = TORCH_RANGE_BASE + TORCH_RANGE_SCALE;
const FILL_RANGE: f32 = FILL_RANGE_BASE + FILL_RANGE_SCALE;
const FLICKER_BASE_INTENSITY: f32 = FLICKER_INTENSITY_BASE + FLICKER_INTENSITY_SCALE;
/// Lumens per Three.js intensity unit — visual approximation, re-tuned in the
/// phase 6 parity audit.
pub(crate) const LUMENS_PER_THREE_UNIT: f32 = 12_000.0;
const FLICKER_RANGE: f32 = 1.2;
const FLICKER_MIN_INTERVAL: f32 = 0.04;
const FLICKER_INTERVAL_RANGE: f32 = 0.15;
const FLICKER_LERP: f32 = 0.2;

/// TS's `lightScale` (`main.ts:1395-1409`): the torch holds full brightness
/// above `FUEL_FADE_THRESHOLD` fuel, then fades linearly toward the
/// guttering minimum as fuel drains further.
fn light_scale(fuel_ratio: f32) -> f32 {
    if fuel_ratio >= FUEL_FADE_THRESHOLD {
        1.0
    } else {
        fuel_ratio / FUEL_FADE_THRESHOLD
    }
}

#[derive(Component)]
pub struct TorchMain;

#[derive(Component)]
pub struct TorchFill;

#[derive(Resource)]
pub struct TorchFlicker {
    rng: Mulberry32,
    target: f32,
    timer: f32,
}

impl Default for TorchFlicker {
    fn default() -> Self {
        Self {
            rng: Mulberry32::new(0x7011_C4ED),
            target: FLICKER_BASE_INTENSITY,
            timer: 0.0,
        }
    }
}

pub fn spawn_torch(commands: &mut Commands) {
    commands.spawn((
        TorchMain,
        PointLight {
            color: TORCH_COLOR,
            intensity: FLICKER_BASE_INTENSITY * LUMENS_PER_THREE_UNIT,
            range: TORCH_RANGE,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::default(),
    ));
    commands.spawn((
        TorchFill,
        PointLight {
            color: TORCH_COLOR,
            intensity: FLICKER_BASE_INTENSITY * 0.6 * LUMENS_PER_THREE_UNIT,
            range: FILL_RANGE,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::default(),
    ));
    commands.init_resource::<TorchFlicker>();
}

#[allow(clippy::type_complexity)]
pub fn torch_update(
    time: Res<Time>,
    session: Res<Session>,
    mut flicker: ResMut<TorchFlicker>,
    camera: Query<&Transform, (With<Player>, Without<TorchMain>, Without<TorchFill>)>,
    mut torch_main: Query<
        (&mut Transform, &mut PointLight),
        (With<TorchMain>, Without<TorchFill>, Without<Player>),
    >,
    mut torch_fill: Query<
        (&mut Transform, &mut PointLight),
        (With<TorchFill>, Without<TorchMain>, Without<Player>),
    >,
) {
    let Ok(camera_transform) = camera.single() else {
        return;
    };
    let Ok((mut main_transform, mut main_light)) = torch_main.single_mut() else {
        return;
    };
    let Ok((mut fill_transform, mut fill_light)) = torch_fill.single_mut() else {
        return;
    };

    let camera_pos = camera_transform.translation;
    main_transform.translation = camera_pos + Vec3::Y * TORCH_OFFSET_Y;

    // Fill light pushed forward from the cell center (opposite the camera back offset).
    let (yaw, _, _) = camera_transform.rotation.to_euler(EulerRot::YXZ);
    fill_transform.translation = Vec3::new(
        camera_pos.x - yaw.sin() * 0.7 * 2.0,
        camera_pos.y + TORCH_OFFSET_Y,
        camera_pos.z - yaw.cos() * 0.7 * 2.0,
    );

    let status_fx = &session.game.status_fx;
    let fuel_ratio = (status_fx.torch_fuel / status_fx.max_torch_fuel) as f32;
    let scale = light_scale(fuel_ratio);
    main_light.range = TORCH_RANGE_BASE + scale * TORCH_RANGE_SCALE;
    fill_light.range = FILL_RANGE_BASE + scale * FILL_RANGE_SCALE;

    flicker.timer -= time.delta_secs();
    if flicker.timer <= 0.0 {
        let flicker_base_intensity = FLICKER_INTENSITY_BASE + scale * FLICKER_INTENSITY_SCALE;
        flicker.target =
            flicker_base_intensity + flicker.rng.next_f64() as f32 * FLICKER_RANGE * scale;
        flicker.timer =
            FLICKER_MIN_INTERVAL + flicker.rng.next_f64() as f32 * FLICKER_INTERVAL_RANGE;
    }
    let target_lumens = flicker.target * LUMENS_PER_THREE_UNIT;
    main_light.intensity += (target_lumens - main_light.intensity) * FLICKER_LERP;
    fill_light.intensity = main_light.intensity * 0.6;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the full-fuel case: a fresh torch must still light the room at
    /// exactly the pre-fuel-scaling constants (`TORCH_RANGE` 12.0,
    /// `FILL_RANGE` 9.0, `FLICKER_BASE_INTENSITY` 5.4).
    #[test]
    fn light_scale_at_full_fuel_is_one() {
        assert_eq!(light_scale(1.0), 1.0);
    }

    /// The knee sits at the ratio itself: TS's `>=` comparison keeps 0.35
    /// at full brightness rather than the start of the fade.
    #[test]
    fn light_scale_at_fade_knee_is_one() {
        assert_eq!(light_scale(FUEL_FADE_THRESHOLD), 1.0);
    }

    /// One step below the knee must already be fading, catching a regression
    /// where the comparison direction or threshold value drifts.
    #[test]
    fn light_scale_just_below_knee_fades_linearly() {
        let ratio = FUEL_FADE_THRESHOLD - 0.01;
        assert_eq!(light_scale(ratio), ratio / FUEL_FADE_THRESHOLD);
    }

    /// An empty torch collapses range and flicker intensity toward their
    /// minimums rather than the full-fuel constants.
    #[test]
    fn light_scale_at_zero_fuel_is_zero() {
        assert_eq!(light_scale(0.0), 0.0);
    }

    /// Across the whole valid fuel domain the scale is a fraction: it must
    /// never dip negative (which would shrink range/intensity below their
    /// documented minimums) or exceed one (which would out-range a full
    /// torch).
    #[test]
    fn light_scale_stays_within_zero_and_one_across_fuel_range() {
        let steps = 200;
        for step in 0..=steps {
            let ratio = step as f32 / steps as f32;
            let scale = light_scale(ratio);
            assert!(
                (0.0..=1.0).contains(&scale),
                "light_scale({ratio}) = {scale} is outside [0, 1]"
            );
        }
    }
}

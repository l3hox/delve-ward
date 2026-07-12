//! Player torch: a main point light at the camera and a fill light pushed
//! toward the cell center, with random flicker. Fuel scaling arrives with the
//! phase 2 game state.

use crate::player::Player;
use bevy::prelude::*;
use delve_core::random::Mulberry32;

const TORCH_COLOR: Color = Color::srgb_u8(0xff, 0x99, 0x4d);
const TORCH_OFFSET_Y: f32 = 0.3;
/// Full-fuel ranges from the TS torch distance formulas.
const TORCH_RANGE: f32 = 12.0;
const FILL_RANGE: f32 = 9.0;
/// Lumens per Three.js intensity unit — visual approximation, re-tuned in the
/// phase 6 parity audit.
const LUMENS_PER_THREE_UNIT: f32 = 12_000.0;
const FLICKER_BASE_INTENSITY: f32 = 5.4;
const FLICKER_RANGE: f32 = 1.2;
const FLICKER_MIN_INTERVAL: f32 = 0.04;
const FLICKER_INTERVAL_RANGE: f32 = 0.15;
const FLICKER_LERP: f32 = 0.2;

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

    flicker.timer -= time.delta_secs();
    if flicker.timer <= 0.0 {
        flicker.target = FLICKER_BASE_INTENSITY + flicker.rng.next_f64() as f32 * FLICKER_RANGE;
        flicker.timer =
            FLICKER_MIN_INTERVAL + flicker.rng.next_f64() as f32 * FLICKER_INTERVAL_RANGE;
    }
    let target_lumens = flicker.target * LUMENS_PER_THREE_UNIT;
    main_light.intensity += (target_lumens - main_light.intensity) * FLICKER_LERP;
    fill_light.intensity = main_light.intensity * 0.6;
}

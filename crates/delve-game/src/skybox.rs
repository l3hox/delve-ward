//! Skybox rendering: an inverted sphere (radius 180, inside the camera's
//! `far: 200.0`) wrapping a procedurally-drawn 2D texture, ported from
//! `rendering/skybox.ts`. Deliberately not Bevy's cubemap `Skybox`
//! component (see PHASE5-PLAN.md, section 2). Texture drawing is seeded
//! with `mulberry32` keyed by variant name per decision D10, replacing
//! TS's unseeded `Math.random()`.
//!
//! TS builds the sphere as `MeshBasicMaterial({ side: BackSide, fog:
//! false, depthWrite: false })` with `renderOrder: -1`. The closest Bevy
//! 0.19 equivalents: `side: BackSide` (render the sphere's inward face,
//! the one visible from inside it) is `cull_mode: Some(Face::Front)`,
//! `fog: false` is `fog_enabled: false`, and `unlit: true` matches
//! `MeshBasicMaterial`'s lighting-independent shading. `StandardMaterial`
//! exposes no per-material depth-write toggle; ordinary depth testing
//! already draws the sphere behind everything else, since its radius
//! (180) exceeds every level's geometry, so `depthWrite: false` and
//! `renderOrder: -1` (a three.js draw-order optimization) have no Bevy
//! equivalent and are dropped without a visible difference.
//!
//! `main.ts`'s render loop recenters the skybox mesh on the camera every
//! frame (`skyboxMesh.position.copy(camera.position)`) so its bounds
//! never come into view as the player walks. `follow_skybox_camera` ports
//! that behavior; it is not yet wired into the app's system schedule
//! (owned by `main.rs`, outside this module's scope).

use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use crate::player::Player;
use crate::textures::{canvas_to_image, seed_for};
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use delve_core::types::{DungeonLevel, Skybox};

const SKYBOX_RADIUS: f32 = 180.0;
const SPHERE_SECTORS: u32 = 32;
const SPHERE_STACKS: u32 = 16;
const TEXTURE_SIZE: usize = 1024;

/// Marker on the spawned skybox sphere.
#[derive(Component)]
pub(crate) struct SkyboxRoot;

pub fn spawn_skybox(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    level: &DungeonLevel,
) {
    let Some(skybox) = level.skybox else {
        return;
    };

    let canvas = generate_texture(skybox);
    let image = images.add(canvas_to_image(canvas));
    let mesh = meshes.add(
        Sphere::new(SKYBOX_RADIUS)
            .mesh()
            .uv(SPHERE_SECTORS, SPHERE_STACKS),
    );
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(image),
        unlit: true,
        fog_enabled: false,
        cull_mode: Some(Face::Front),
        ..default()
    });

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::default(),
        SkyboxRoot,
        LevelEntity,
    ));
}

/// Recenters the skybox sphere on the player every frame, matching
/// `main.ts`'s `skyboxMesh.position.copy(camera.position)` so the sphere's
/// bounds never come into view as the player walks the level.
///
/// Not yet registered in `main.rs`'s `Update` schedule; that wiring is
/// outside this module's ownership for this slice.
#[allow(dead_code)]
pub fn follow_skybox_camera(
    player: Query<&Transform, (With<Player>, Without<SkyboxRoot>)>,
    mut skyboxes: Query<&mut Transform, (With<SkyboxRoot>, Without<Player>)>,
) {
    let Ok(player_transform) = player.single() else {
        return;
    };
    let Ok(mut skybox_transform) = skyboxes.single_mut() else {
        return;
    };
    skybox_transform.translation = player_transform.translation;
}

fn generate_texture(skybox: Skybox) -> PixelCanvas {
    let mut rng = CanvasRng::new(seed_for(variant_seed_name(skybox)));
    match skybox {
        Skybox::StarryNight => generate_starry_night(&mut rng),
        Skybox::Daylight => generate_daylight(&mut rng),
        Skybox::Sunset => generate_sunset(&mut rng),
    }
}

fn variant_seed_name(skybox: Skybox) -> &'static str {
    match skybox {
        Skybox::StarryNight => "skybox-starry-night",
        Skybox::Daylight => "skybox-daylight",
        Skybox::Sunset => "skybox-sunset",
    }
}

fn generate_starry_night(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(TEXTURE_SIZE);
    fill_vertical_gradient(&mut canvas, &[(0.0, hex(0x01010a)), (1.0, hex(0x03030e))]);

    let extent = TEXTURE_SIZE as f32;
    for _ in 0..1200 {
        let x = random_offset(rng, 0.0, extent);
        let y = random_offset(rng, 0.0, extent);
        let radius = random_offset(rng, 0.3, 0.7);
        let brightness = 150 + rng.below(105);
        let color = Rgba::opaque(
            brightness as u8,
            brightness as u8,
            (brightness + 20).min(255) as u8,
        );
        canvas.fill_ellipse(x, y, radius, radius, color);
    }
    canvas
}

fn generate_daylight(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(TEXTURE_SIZE);
    fill_vertical_gradient(
        &mut canvas,
        &[
            (0.0, hex(0x3366aa)),
            (0.4, hex(0x5588cc)),
            (0.7, hex(0x88bbdd)),
            (1.0, hex(0xaaccee)),
        ],
    );

    let extent = TEXTURE_SIZE as f32;
    let cloud_color = Rgba::translucent(255, 255, 255, 0.3);
    for _ in 0..25 {
        let center_x = random_offset(rng, 0.0, extent);
        let center_y = random_offset(rng, extent * 0.1, extent * 0.5);
        let radius_x = random_offset(rng, 30.0, 60.0);
        let radius_y = random_offset(rng, 10.0, 20.0);
        canvas.fill_ellipse(center_x, center_y, radius_x, radius_y, cloud_color);
    }
    canvas
}

fn generate_sunset(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(TEXTURE_SIZE);
    fill_vertical_gradient(
        &mut canvas,
        &[
            (0.0, hex(0x1a2244)),
            (0.3, hex(0x443366)),
            (0.5, hex(0xcc6633)),
            (0.7, hex(0xee8866)),
            (1.0, hex(0xffaa88)),
        ],
    );

    let extent = TEXTURE_SIZE as f32;
    let cloud_color = Rgba::translucent(255, 200, 150, 0.25);
    for _ in 0..20 {
        let center_x = random_offset(rng, 0.0, extent);
        let center_y = random_offset(rng, extent * 0.15, extent * 0.4);
        let radius_x = random_offset(rng, 25.0, 50.0);
        let radius_y = random_offset(rng, 8.0, 15.0);
        canvas.fill_ellipse(center_x, center_y, radius_x, radius_y, cloud_color);
    }
    canvas
}

/// `base + Math.random() * span`, the shape of every random draw in
/// `rendering/skybox.ts`'s texture generators.
fn random_offset(rng: &mut CanvasRng, base: f32, span: f32) -> f32 {
    base + (rng.random() * f64::from(span)) as f32
}

/// Vertical linear gradient across the canvas height, matching
/// `CanvasRenderingContext2D`'s `createLinearGradient` plus `addColorStop`
/// piecewise interpolation between `stops` (ascending 0.0-1.0 offsets).
fn fill_vertical_gradient(canvas: &mut PixelCanvas, stops: &[(f32, Rgba)]) {
    let height = canvas.height() as i32;
    let width = canvas.width() as i32;
    for y in 0..height {
        let offset = y as f32 / (height - 1) as f32;
        canvas.fill_rect(0, y, width, 1, gradient_color_at(stops, offset));
    }
}

fn gradient_color_at(stops: &[(f32, Rgba)], offset: f32) -> Rgba {
    let mut lower = stops[0];
    for &(stop_offset, stop_color) in stops {
        if stop_offset > offset {
            let (lower_offset, lower_color) = lower;
            let span = stop_offset - lower_offset;
            let local = if span > 0.0 {
                (offset - lower_offset) / span
            } else {
                0.0
            };
            return lerp_rgba(lower_color, stop_color, local);
        }
        lower = (stop_offset, stop_color);
    }
    lower.1
}

fn lerp_rgba(start: Rgba, end: Rgba, t: f32) -> Rgba {
    let lerp_channel = |from: u8, to: u8| -> u8 {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * t).round() as u8
    };
    Rgba::opaque(
        lerp_channel(start.red, end.red),
        lerp_channel(start.green, end.green),
        lerp_channel(start.blue, end.blue),
    )
}

const fn hex(rgb: u32) -> Rgba {
    Rgba::opaque(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

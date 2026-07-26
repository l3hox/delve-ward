//! Floating damage numbers, ported from the TS DamageNumberManager:
//! world-space billboards that rise, grow slightly, and fade out. Digits are
//! drawn with the shared pixel font instead of the browser's monospace.

use crate::billboard::FacesCamera;
use crate::dungeon::CELL_SIZE;
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use crate::textures::canvas_to_image;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};

const FLOAT_SPEED: f32 = 1.5;
const LIFETIME: f32 = 0.7;
/// World units.
const NUMBER_SIZE: f32 = 0.5;
const CANVAS_SIZE: usize = 64;
const TEXT_SCALE: i32 = 8;

/// Ports the `depthTest: false` on TS's damage-number `SpriteMaterial`
/// (`rendering/damageNumbers.ts:23`), the one material in the TS renderer
/// that opts out of depth testing (its health-bar sprites keep
/// `depthTest: true`). Without it a number spawned at a cell's center is
/// hidden by that cell's own geometry — striking a breakable wall reads as
/// the wall not reacting at all, since the wall face stands between the
/// camera and the number.
///
/// Bevy has no per-material depth-test flag, so the pipeline descriptor is
/// edited directly: an always-passing compare with no depth write, matching
/// what THREE's flag sets on its own pipeline state. Carries no bindings of
/// its own — the base `StandardMaterial`'s shaders and data are reused
/// verbatim (`MaterialExtension`'s shader hooks all default to the base).
#[derive(Asset, AsBindGroup, Reflect, Clone, Default)]
pub struct DrawOnTop {}

impl MaterialExtension for DrawOnTop {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_compare = Some(CompareFunction::Always);
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// The damage number's material type: a standard material that ignores the
/// depth buffer. Aliased because the full generic pair appears in every
/// `SystemParam` that spawns a number.
pub type DamageNumberMaterial = ExtendedMaterial<StandardMaterial, DrawOnTop>;

#[derive(Component)]
pub struct DamageNumber {
    age: f32,
    material: Handle<DamageNumberMaterial>,
}

fn number_texture(damage: f64) -> PixelCanvas {
    let mut canvas = PixelCanvas::with_dimensions(CANVAS_SIZE, CANVAS_SIZE);
    let text = if (damage - damage.round()).abs() < 1e-9 {
        format!("{}", damage.round() as i64)
    } else {
        format!("{damage:.1}")
    };
    let text_x = (CANVAS_SIZE as i32 - measure_pixel_text(&text, TEXT_SCALE)) / 2;
    let text_y = (CANVAS_SIZE as i32 - 5 * TEXT_SCALE) / 2;

    let black = Rgba::opaque(0, 0, 0);
    for (dx, dy) in [
        (-2, 0),
        (2, 0),
        (0, -2),
        (0, 2),
        (-2, -2),
        (2, -2),
        (-2, 2),
        (2, 2),
    ] {
        draw_pixel_text(
            &mut canvas,
            &text,
            text_x + dx,
            text_y + dy,
            black,
            TEXT_SCALE,
        );
    }
    draw_pixel_text(
        &mut canvas,
        &text,
        text_x,
        text_y,
        Rgba::opaque(0xff, 0xff, 0xff),
        TEXT_SCALE,
    );
    canvas
}

pub fn spawn_damage_number(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<DamageNumberMaterial>,
    damage: f64,
    (col, row): (i64, i64),
    layer_y_offset: f32,
) {
    let texture = images.add(canvas_to_image(number_texture(damage)));
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            base_color_texture: Some(texture),
            unlit: true,
            fog_enabled: false,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        },
        extension: DrawOnTop::default(),
    });
    let center_x = col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
    let center_z = row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
    commands.spawn((
        LevelEntity,
        FacesCamera,
        DamageNumber {
            age: 0.0,
            material: material.clone(),
        },
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(material),
        Transform::from_xyz(center_x, 1.4 + layer_y_offset, center_z)
            .with_scale(Vec3::splat(NUMBER_SIZE)),
    ));
}

pub fn update_damage_numbers(
    time: Res<Time>,
    mut commands: Commands,
    mut materials: ResMut<Assets<DamageNumberMaterial>>,
    mut numbers: Query<(Entity, &mut DamageNumber, &mut Transform)>,
) {
    let delta = time.delta_secs();
    for (entity, mut number, mut transform) in &mut numbers {
        number.age += delta;
        if number.age >= LIFETIME {
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation.y += FLOAT_SPEED * delta;

        let t = number.age / LIFETIME;
        let opacity = if t < 0.4 { 1.0 } else { 1.0 - (t - 0.4) / 0.6 };
        if let Some(mut material) = materials.get_mut(&number.material) {
            material.base.base_color = Color::WHITE.with_alpha(opacity);
        }
        let scale = NUMBER_SIZE * (1.0 + t * 0.3);
        transform.scale = Vec3::splat(scale);
    }
}

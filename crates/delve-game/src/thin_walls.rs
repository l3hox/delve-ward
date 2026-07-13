//! Thin wall rendering: per-edge wall planes on a cell's south or east
//! border, ported from `rendering/thinWallRenderer.ts`. Each wall is one
//! double-sided plane when its front and back use the same texture, or two
//! single-sided planes facing opposite directions when they differ —
//! matching TS's `alphaTest: 0.5` cutout (`AlphaMode::Mask(0.5)` here) so
//! fence/railing gaps read as see-through rather than alpha-blended (TS's
//! own comment: "avoids multi-zone blending artifacts").
//!
//! Texture generation (`stone_thin`/`iron_fence`/`wood_fence`/`railing`,
//! ported from `rendering/textures.ts`'s `generateThinWallTexture`) lives
//! locally in this module rather than in `textures.rs::DungeonMaterials`,
//! since that file is out of this module's ownership for this slice; see
//! the completion report for the tradeoff. Seeded with `mulberry32` keyed
//! by texture name per decision D10, replacing TS's unseeded
//! `Math.random()`. `iron_fence` and `railing` draw no random pixels at
//! all (TS uses only fixed coordinates for both), so their generators take
//! no RNG.

use crate::dungeon::{CELL_SIZE, LayerSpawn, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use crate::textures::{canvas_to_image, seed_for};
use crate::zones::{self, LevelZones};
use bevy::prelude::*;
use delve_core::game_state::{LayerState, ThinWallHeight, ThinWallSide};
use std::collections::HashMap;

const THIN_SIZE: usize = 128;

pub fn spawn_thin_walls(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &LayerSpawn,
    zones: &LevelZones,
) {
    if layer_state.thin_walls.is_empty() {
        return;
    }

    let plane_full = meshes.add(Rectangle::new(CELL_SIZE, WALL_HEIGHT));
    let plane_half = meshes.add(Rectangle::new(CELL_SIZE, WALL_HEIGHT * 0.5));
    let mut image_cache: HashMap<String, Handle<Image>> = HashMap::new();

    for wall in layer_state.thin_walls.values() {
        let full_height = matches!(wall.height, ThinWallHeight::Full);
        let height = if full_height {
            WALL_HEIGHT
        } else {
            WALL_HEIGHT * 0.5
        };
        let y_center = height / 2.0;
        let plane = if full_height {
            plane_full.clone()
        } else {
            plane_half.clone()
        };

        let center_x = wall.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = wall.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;

        let root = commands
            .spawn((
                LevelEntity,
                wall_transform(wall.wall, center_x, center_z, layer_spawn.y_offset),
                Visibility::default(),
            ))
            .id();

        // TS compares the *cached texture objects* (`backTex === frontTex`),
        // which are memoized by name — so this is a name comparison, not
        // just "is textureBack absent": an explicit `textureBack` equal to
        // `texture` still takes the single-double-sided-mesh path.
        let same_texture = wall
            .texture_back
            .as_deref()
            .is_none_or(|back| back == wall.texture);
        let front_image = thin_wall_image(&mut image_cache, images, &wall.texture);

        if same_texture {
            let front_material = thin_wall_material(materials, front_image, true);
            let mesh = commands
                .spawn((
                    Mesh3d(plane),
                    MeshMaterial3d(front_material),
                    Transform::from_xyz(0.0, y_center, 0.0),
                ))
                .id();
            commands.entity(root).add_child(mesh);
            // `RenderLayers` doesn't propagate from parent to children in
            // Bevy 0.19 (confirmed against the vendored visibility source —
            // `check_visibility_cpu_culling` reads `Option<&RenderLayers>`
            // directly off the entity being culled, with no ancestor walk),
            // so the mesh itself needs tagging, not just `root`.
            zones::tag_cell(commands, zones, layer_spawn.index, mesh, wall.col, wall.row);
        } else {
            let back_name = wall.texture_back.as_deref().unwrap_or(&wall.texture);
            let back_image = thin_wall_image(&mut image_cache, images, back_name);
            // TS's `side: THREE.FrontSide` is the engine default — Bevy's
            // default `cull_mode` (`Some(Face::Back)`) already matches it,
            // so these two materials skip the double-sided override below.
            let front_material = thin_wall_material(materials, front_image, false);
            let back_material = thin_wall_material(materials, back_image, false);

            let front_mesh = commands
                .spawn((
                    Mesh3d(plane.clone()),
                    MeshMaterial3d(front_material),
                    Transform::from_xyz(0.0, y_center, 0.0),
                ))
                .id();
            let back_mesh = commands
                .spawn((
                    Mesh3d(plane),
                    MeshMaterial3d(back_material),
                    Transform::from_xyz(0.0, y_center, 0.0)
                        .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                ))
                .id();
            commands.entity(root).add_children(&[front_mesh, back_mesh]);
            zones::tag_cell(
                commands,
                zones,
                layer_spawn.index,
                front_mesh,
                wall.col,
                wall.row,
            );
            zones::tag_cell(
                commands,
                zones,
                layer_spawn.index,
                back_mesh,
                wall.col,
                wall.row,
            );
        }
    }
}

/// TS: `wallGroup.position.set(...)` / `.rotation.y = ...` for the `'S'`
/// and `'E'` cases in `buildSingleThinWall` — the wall sits on the cell's
/// south or east edge, rotated so its front face points back into the
/// owning cell. `y_offset` is the layer's world Y (TS applies this at the
/// group level in `levelSceneBuilder.ts`, not per-wall).
fn wall_transform(wall: ThinWallSide, center_x: f32, center_z: f32, y_offset: f32) -> Transform {
    match wall {
        ThinWallSide::S => Transform::from_xyz(center_x, y_offset, center_z + CELL_SIZE / 2.0)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
        ThinWallSide::E => Transform::from_xyz(center_x + CELL_SIZE / 2.0, y_offset, center_z)
            .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)),
    }
}

fn thin_wall_image(
    cache: &mut HashMap<String, Handle<Image>>,
    images: &mut Assets<Image>,
    texture_name: &str,
) -> Handle<Image> {
    if let Some(handle) = cache.get(texture_name) {
        return handle.clone();
    }
    let mut rng = CanvasRng::new(seed_for(texture_name));
    let canvas = generate_thin_wall_texture(texture_name, &mut rng);
    let handle = images.add(canvas_to_image(canvas));
    cache.insert(texture_name.to_string(), handle.clone());
    handle
}

/// `double_sided` sets both `double_sided` and `cull_mode: None` together —
/// TS's `side: THREE.DoubleSide` case only. The single-sided case leaves
/// both at their Bevy defaults, matching `side: THREE.FrontSide`.
fn thin_wall_material(
    materials: &mut Assets<StandardMaterial>,
    image: Handle<Image>,
    double_sided: bool,
) -> Handle<StandardMaterial> {
    let mut material = StandardMaterial {
        base_color_texture: Some(image),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Mask(0.5),
        ..default()
    };
    if double_sided {
        material.double_sided = true;
        material.cull_mode = None;
    }
    materials.add(material)
}

fn generate_thin_wall_texture(name: &str, rng: &mut CanvasRng) -> PixelCanvas {
    match name {
        "stone_thin" => generate_stone_thin(rng),
        "iron_fence" => generate_iron_fence(),
        "wood_fence" => generate_wood_fence(rng),
        "railing" => generate_railing(),
        _ => generate_fallback(),
    }
}

fn generate_stone_thin(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(THIN_SIZE);
    let size = THIN_SIZE as i32;

    for y in 0..size {
        for x in 0..size {
            let color = Rgba::opaque(rng.vary(112, 12), rng.vary(112, 10), rng.vary(112, 10));
            canvas.fill_rect(x, y, 1, 1, color);
        }
    }

    for _ in 0..12 {
        let patch_x = rng.below(size);
        let patch_y = rng.below(size);
        let value = rng.vary(150, 10);
        let width = 8 + rng.below(10);
        let height = 6 + rng.below(6);
        canvas.fill_rect(
            patch_x,
            patch_y,
            width,
            height,
            Rgba::translucent(value, value, value, 0.3),
        );
    }

    let mortar_rows = [0, 26, 52, 78, 104];
    let mortar_color = Rgba::translucent(80, 80, 80, 0.75);
    for &mortar_y in &mortar_rows {
        canvas.fill_rect(0, mortar_y, size, 2, mortar_color);
    }

    for (band, &top) in mortar_rows.iter().enumerate() {
        let offset = if band % 2 == 0 { 0 } else { 32 };
        let bottom = mortar_rows.get(band + 1).copied().unwrap_or(size);
        let mut vertical_x = offset;
        while vertical_x < size {
            canvas.fill_rect(vertical_x, top, 2, bottom - top, mortar_color);
            vertical_x += 64;
        }
    }

    canvas
}

fn generate_iron_fence() -> PixelCanvas {
    let mut canvas = PixelCanvas::new(THIN_SIZE);
    let size = THIN_SIZE as i32;

    let bar_width = 8;
    for &bar_x in &[20, 46, 72, 98] {
        canvas.fill_rect(bar_x, 0, bar_width, size, hex(0x55_55_55));
        canvas.fill_rect(bar_x, 0, 2, size, hex(0x66_66_66));
        canvas.fill_rect(bar_x + bar_width - 2, 0, 2, size, hex(0x3a_3a_3a));
    }

    for &cross_y in &[32, 96] {
        canvas.fill_rect(0, cross_y, size, 8, hex(0x4a_4a_4a));
        canvas.fill_rect(0, cross_y, size, 2, hex(0x5a_5a_5a));
    }

    canvas
}

fn generate_wood_fence(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(THIN_SIZE);
    let size = THIN_SIZE as i32;
    let plank_width = 26;
    let grain_color = Rgba::translucent(80, 50, 24, 0.25);
    let outline_color = hex(0x50_33_18);

    for &plank_x in &[4, 34, 64, 94] {
        for y in 0..size {
            for x in plank_x..plank_x + plank_width {
                let color = Rgba::opaque(rng.vary(107, 10), rng.vary(68, 8), rng.vary(35, 6));
                canvas.fill_rect(x, y, 1, 1, color);
            }
        }

        let mut grain_y = 6;
        while grain_y < size {
            canvas.fill_rect(plank_x, grain_y, plank_width, 1, grain_color);
            grain_y += 8 + rng.below(4);
        }

        canvas.fill_rect(plank_x, 0, 2, size, outline_color);
        canvas.fill_rect(plank_x + plank_width - 2, 0, 2, size, outline_color);
    }

    canvas
}

fn generate_railing() -> PixelCanvas {
    let mut canvas = PixelCanvas::new(THIN_SIZE);
    let size = THIN_SIZE as i32;

    let bar_height = 10;
    for &bar_y in &[28, 90] {
        canvas.fill_rect(0, bar_y, size, bar_height, hex(0x50_50_50));
        canvas.fill_rect(0, bar_y, size, 2, hex(0x66_66_66));
        canvas.fill_rect(0, bar_y + bar_height - 2, size, 2, hex(0x38_38_38));
    }

    for &support_x in &[16, 48, 80, 112] {
        canvas.fill_rect(support_x, 0, 4, size, hex(0x48_48_48));
    }

    canvas
}

fn generate_fallback() -> PixelCanvas {
    let mut canvas = PixelCanvas::new(THIN_SIZE);
    let size = THIN_SIZE as i32;
    canvas.fill_rect(0, 0, size, size, hex(0x88_88_88));
    canvas
}

const fn hex(rgb: u32) -> Rgba {
    Rgba::opaque(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> (u8, u8, u8, u8) {
        let offset = (y * width + x) * 4;
        (
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        )
    }

    /// TS's own comment: "'S' edge... Front should face north (-Z
    /// direction, into the owning cell)."
    #[test]
    fn wall_transform_south_sits_on_the_south_edge_facing_into_the_cell() {
        let transform = wall_transform(ThinWallSide::S, 3.0, 5.0, 1.25);
        assert_eq!(
            transform.translation,
            Vec3::new(3.0, 1.25, 5.0 + CELL_SIZE / 2.0)
        );
        let facing = transform.rotation * Vec3::Z;
        assert!((facing - Vec3::NEG_Z).length() < 1e-6);
    }

    /// TS's own comment: "'E' edge... Front should face west (-X
    /// direction, into the owning cell)."
    #[test]
    fn wall_transform_east_sits_on_the_east_edge_facing_into_the_cell() {
        let transform = wall_transform(ThinWallSide::E, 3.0, 5.0, 1.25);
        assert_eq!(
            transform.translation,
            Vec3::new(3.0 + CELL_SIZE / 2.0, 1.25, 5.0)
        );
        let facing = transform.rotation * Vec3::Z;
        assert!((facing - Vec3::NEG_X).length() < 1e-6);
    }

    #[test]
    fn generate_iron_fence_leaves_gaps_transparent_and_bars_opaque() {
        let canvas = generate_iron_fence();
        let width = canvas.width();
        let bytes = canvas.into_rgba_bytes();
        // Background gap, away from any bar or crossbar — TS's
        // `ctx.clearRect` leaves it fully transparent for the alphaTest
        // cutout to discard.
        assert_eq!(pixel(&bytes, width, 0, 0), (0, 0, 0, 0));
        // x=23 sits inside the first bar (x=20..28) but outside its
        // 2px highlight (x=20-21) and shadow (x=26-27) edges, and y=64
        // is outside both crossbars (y=32-40, y=96-104) — pure base fill.
        assert_eq!(pixel(&bytes, width, 23, 64), (0x55, 0x55, 0x55, 255));
    }

    #[test]
    fn unknown_texture_name_falls_back_to_solid_gray() {
        let mut rng = CanvasRng::new(0);
        let canvas = generate_thin_wall_texture("nonexistent", &mut rng);
        let width = canvas.width();
        let bytes = canvas.into_rgba_bytes();
        assert_eq!(pixel(&bytes, width, 0, 0), (0x88, 0x88, 0x88, 255));
        assert_eq!(pixel(&bytes, width, 64, 64), (0x88, 0x88, 0x88, 255));
    }
}

//! Procedural wall/floor/ceiling textures, regenerated at startup from the
//! same drawing operations as the TS canvas generators. Randomness is seeded
//! per texture name so output is stable across runs (the TS original uses
//! unseeded `Math.random`; only the visual character must match).

use crate::pixel_canvas::{CanvasRng, PixelCanvas, Rgba};
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

const SIZE: usize = 64;

fn seed_for(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

fn base_noise(
    canvas: &mut PixelCanvas,
    rng: &mut CanvasRng,
    red: i32,
    green: i32,
    blue: i32,
    spans: (i32, i32, i32),
) {
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let color = Rgba::opaque(
                rng.vary(red, spans.0),
                rng.vary(green, spans.1),
                rng.vary(blue, spans.2),
            );
            canvas.fill_rect(x, y, 1, 1, color);
        }
    }
}

fn mortar_grid(
    canvas: &mut PixelCanvas,
    rows: &[i32],
    thickness: i32,
    offset_step: i32,
    color: Rgba,
) {
    for &row in rows {
        canvas.fill_rect(0, row, SIZE as i32, thickness, color);
    }
    for (band, &top) in rows.iter().enumerate() {
        let offset = if band % 2 == 0 { 0 } else { offset_step };
        let bottom = rows.get(band + 1).copied().unwrap_or(SIZE as i32);
        let mut x = offset;
        while x < SIZE as i32 {
            canvas.fill_rect(x, top, thickness, bottom - top, color);
            x += 32;
        }
    }
}

fn generate_stone_wall(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 140, 120, 100, (14, 12, 12));
    mortar_grid(
        &mut canvas,
        &[0, 16, 32, 48],
        1,
        16,
        Rgba::translucent(40, 34, 28, 0.6),
    );
    canvas
}

fn generate_brick_wall(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 160, 90, 60, (16, 10, 8));
    mortar_grid(
        &mut canvas,
        &[0, 12, 24, 36, 48, 60],
        2,
        16,
        Rgba::translucent(60, 50, 40, 0.7),
    );
    canvas
}

fn generate_mossy_wall(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 140, 120, 100, (14, 12, 12));
    mortar_grid(
        &mut canvas,
        &[0, 16, 32, 48],
        1,
        16,
        Rgba::translucent(40, 34, 28, 0.6),
    );
    for _ in 0..80 {
        let moss_x = rng.below(SIZE as i32);
        let moss_y = 32 + rng.below(32);
        let color = Rgba::translucent(rng.vary(50, 15), rng.vary(100, 20), rng.vary(40, 10), 0.7);
        let size = 1 + rng.below(3);
        canvas.fill_rect(moss_x, moss_y, size, size, color);
    }
    canvas
}

fn generate_wood_wall(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 120, 80, 45, (10, 8, 6));
    let grain = Rgba::translucent(80, 55, 30, 0.4);
    let mut x = 0;
    while x < SIZE as i32 {
        let end_x = x + i32::from(rng.vary(0, 2));
        canvas.stroke_line(x, 0, end_x, SIZE as i32 - 1, grain);
        x += 4 + rng.below(4);
    }
    for _ in 0..3 {
        let knot_x = 8 + rng.below(SIZE as i32 - 16);
        let knot_y = 8 + rng.below(SIZE as i32 - 16);
        canvas.fill_ellipse(
            knot_x as f32,
            knot_y as f32,
            3.0,
            4.0,
            Rgba::translucent(60, 40, 20, 0.6),
        );
    }
    canvas
}

fn generate_forest_wall(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 40, 65, 30, (12, 18, 10));
    for trunk_x in [6, 22, 40, 55] {
        let width = 6 + rng.below(3);
        for y in 0..SIZE as i32 {
            for dx in 0..width {
                let color = Rgba::opaque(rng.vary(35, 8), rng.vary(22, 6), rng.vary(12, 4));
                canvas.fill_rect(trunk_x + dx, y, 1, 1, color);
            }
        }
    }
    canvas.fill_rect(
        0,
        42,
        SIZE as i32,
        SIZE as i32 - 42,
        Rgba::translucent(10, 20, 8, 0.35),
    );
    canvas.fill_rect(0, 0, SIZE as i32, 20, Rgba::translucent(70, 110, 40, 0.15));
    for _ in 0..40 {
        let leaf_x = rng.below(SIZE as i32);
        let leaf_y = rng.below(SIZE as i32);
        let bright = rng.random() > 0.5;
        let color = if bright {
            Rgba::translucent(rng.vary(50, 12), rng.vary(110, 20), rng.vary(30, 10), 0.75)
        } else {
            Rgba::translucent(rng.vary(20, 8), rng.vary(55, 12), rng.vary(15, 6), 0.75)
        };
        let size = 2 + rng.below(4);
        canvas.fill_rect(leaf_x, leaf_y, size, size, color);
    }
    canvas
}

fn generate_stone_tile_floor(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 70, 62, 54, (10, 8, 8));
    let line = Rgba::translucent(28, 22, 16, 0.7);
    canvas.fill_rect(0, 0, SIZE as i32, 1, line);
    canvas.fill_rect(0, 32, SIZE as i32, 1, line);
    canvas.fill_rect(0, 0, 1, SIZE as i32, line);
    canvas.fill_rect(32, 0, 1, SIZE as i32, line);
    canvas
}

fn generate_dirt_floor(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 95, 70, 40, (18, 14, 10));
    for _ in 0..20 {
        let pebble_x = rng.below(SIZE as i32);
        let pebble_y = rng.below(SIZE as i32);
        let bright = rng.random() > 0.5;
        let color = if bright {
            Rgba::opaque(rng.vary(120, 10), rng.vary(95, 8), rng.vary(60, 6))
        } else {
            Rgba::opaque(rng.vary(60, 8), rng.vary(45, 6), rng.vary(25, 4))
        };
        canvas.fill_rect(pebble_x, pebble_y, 2, 2, color);
    }
    canvas
}

fn generate_cobblestone_floor(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 90, 82, 74, (12, 10, 10));
    let outline = Rgba::translucent(30, 25, 20, 0.6);
    const STONES: [[f32; 4]; 20] = [
        [4.0, 4.0, 10.0, 8.0],
        [20.0, 2.0, 12.0, 10.0],
        [38.0, 4.0, 10.0, 9.0],
        [52.0, 3.0, 10.0, 8.0],
        [2.0, 16.0, 11.0, 9.0],
        [16.0, 15.0, 13.0, 10.0],
        [34.0, 16.0, 10.0, 8.0],
        [48.0, 14.0, 12.0, 10.0],
        [6.0, 28.0, 10.0, 9.0],
        [22.0, 27.0, 11.0, 10.0],
        [38.0, 28.0, 12.0, 8.0],
        [54.0, 26.0, 8.0, 10.0],
        [1.0, 40.0, 12.0, 9.0],
        [18.0, 39.0, 10.0, 10.0],
        [32.0, 40.0, 13.0, 9.0],
        [50.0, 38.0, 11.0, 10.0],
        [4.0, 52.0, 11.0, 10.0],
        [20.0, 50.0, 12.0, 11.0],
        [36.0, 52.0, 10.0, 9.0],
        [50.0, 51.0, 12.0, 10.0],
    ];
    for [stone_x, stone_y, width, height] in STONES {
        canvas.stroke_ellipse(
            stone_x + width / 2.0,
            stone_y + height / 2.0,
            width / 2.0,
            height / 2.0,
            outline,
        );
    }
    canvas
}

fn generate_grass_floor(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 55, 100, 35, (14, 20, 10));
    for _ in 0..50 {
        let tuft_x = rng.below(SIZE as i32);
        let tuft_y = rng.below(SIZE as i32);
        let color = Rgba::translucent(rng.vary(30, 8), rng.vary(70, 12), rng.vary(20, 6), 0.8);
        let size = 2 + rng.below(2);
        canvas.fill_rect(tuft_x, tuft_y, size, size, color);
    }
    for _ in 0..8 {
        let patch_x = rng.below(SIZE as i32);
        let patch_y = rng.below(SIZE as i32);
        let color = Rgba::translucent(rng.vary(130, 16), rng.vary(105, 14), rng.vary(45, 10), 0.5);
        let width = 3 + rng.below(4);
        let height = 2 + rng.below(3);
        canvas.fill_rect(patch_x, patch_y, width, height, color);
    }
    canvas
}

fn generate_dark_rock_ceiling(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 48, 42, 36, (8, 6, 6));
    let crack = Rgba::translucent(16, 12, 8, 0.5);
    canvas.stroke_line(8, 0, 24, 28, crack);
    canvas.stroke_line(24, 28, 56, 36, crack);
    canvas.stroke_line(40, 4, 36, 52, crack);
    canvas.stroke_line(36, 52, 60, 60, crack);
    canvas
}

fn generate_wooden_beams_ceiling(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 55, 38, 22, (8, 6, 5));
    let beam = Rgba::translucent(40, 28, 16, 0.8);
    canvas.fill_rect(0, 10, SIZE as i32, 6, beam);
    canvas.fill_rect(0, 48, SIZE as i32, 6, beam);
    let highlight = Rgba::translucent(80, 60, 35, 0.4);
    canvas.fill_rect(0, 10, SIZE as i32, 1, highlight);
    canvas.fill_rect(0, 48, SIZE as i32, 1, highlight);
    canvas
}

fn generate_canopy_ceiling(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 25, 55, 18, (8, 14, 6));
    for _ in 0..25 {
        let patch_x = rng.below(SIZE as i32);
        let patch_y = rng.below(SIZE as i32);
        let color = Rgba::translucent(rng.vary(40, 10), rng.vary(80, 16), rng.vary(28, 8), 0.6);
        let width = 4 + rng.below(8);
        let height = 3 + rng.below(6);
        canvas.fill_rect(patch_x, patch_y, width, height, color);
    }
    for _ in 0..5 {
        let spot_x = rng.below(SIZE as i32);
        let spot_y = rng.below(SIZE as i32);
        let radius = (1 + rng.below(3)) as f32;
        canvas.fill_ellipse(
            spot_x as f32,
            spot_y as f32,
            radius,
            radius,
            Rgba::translucent(80, 120, 50, 0.7),
        );
    }
    canvas
}

pub(crate) fn canvas_to_image(canvas: PixelCanvas) -> Image {
    let size = canvas.size() as u32;
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        canvas.into_rgba_bytes(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

fn generate_door(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 100, 65, 35, (10, 8, 6));
    let line = Rgba::translucent(40, 25, 12, 0.7);
    canvas.fill_rect(0, 0, 2, SIZE as i32, line);
    canvas.fill_rect(SIZE as i32 - 2, 0, 2, SIZE as i32, line);
    canvas.fill_rect(21, 0, 1, SIZE as i32, line);
    canvas.fill_rect(42, 0, 1, SIZE as i32, line);
    canvas.fill_rect(0, 0, SIZE as i32, 2, line);
    canvas.fill_rect(0, SIZE as i32 - 2, SIZE as i32, 2, line);
    canvas.fill_rect(0, 20, SIZE as i32, 2, line);
    canvas.fill_rect(0, 42, SIZE as i32, 2, line);
    canvas
}

fn generate_locked_door(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 70, 45, 25, (8, 6, 5));
    let line = Rgba::translucent(30, 18, 8, 0.8);
    canvas.fill_rect(0, 0, 2, SIZE as i32, line);
    canvas.fill_rect(SIZE as i32 - 2, 0, 2, SIZE as i32, line);
    canvas.fill_rect(21, 0, 1, SIZE as i32, line);
    canvas.fill_rect(42, 0, 1, SIZE as i32, line);
    canvas.fill_rect(0, 0, SIZE as i32, 2, line);
    canvas.fill_rect(0, SIZE as i32 - 2, SIZE as i32, 2, line);
    canvas.fill_rect(0, 20, SIZE as i32, 2, line);
    canvas.fill_rect(0, 42, SIZE as i32, 2, line);

    let band = Rgba::translucent(120, 120, 130, 0.6);
    canvas.fill_rect(0, 10, SIZE as i32, 3, band);
    canvas.fill_rect(0, 50, SIZE as i32, 3, band);

    let stud = Rgba::translucent(160, 155, 150, 0.8);
    for stud_x in [6, 30, 54] {
        for stud_y in [10, 50] {
            canvas.fill_rect(stud_x, stud_y, 3, 3, stud);
        }
    }

    let keyhole = Rgba::translucent(20, 15, 10, 0.9);
    canvas.fill_ellipse(32.0, 32.0, 4.0, 4.0, keyhole);
    canvas.fill_rect(31, 32, 3, 8, keyhole);
    canvas
}

fn generate_door_frame(rng: &mut CanvasRng) -> PixelCanvas {
    let mut canvas = PixelCanvas::new(SIZE);
    base_noise(&mut canvas, rng, 110, 105, 100, (12, 10, 10));
    let chisel = Rgba::translucent(60, 55, 50, 0.5);
    for _ in 0..15 {
        let scratch_x = rng.below(SIZE as i32);
        let scratch_y = rng.below(SIZE as i32);
        let length = 2 + rng.below(4);
        let end_x = scratch_x + i32::from(rng.vary(0, 2));
        canvas.stroke_line(scratch_x, scratch_y, end_x, scratch_y + length, chisel);
    }
    canvas
}

fn image_with_repeat(canvas: PixelCanvas) -> Image {
    let mut image = canvas_to_image(canvas);
    image.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        ..bevy::image::ImageSamplerDescriptor::nearest()
    });
    image
}

type Generator = fn(&mut CanvasRng) -> PixelCanvas;

const WALL_GENERATORS: [(&str, Generator); 5] = [
    ("stone", generate_stone_wall),
    ("brick", generate_brick_wall),
    ("mossy", generate_mossy_wall),
    ("wood", generate_wood_wall),
    ("forest", generate_forest_wall),
];

const FLOOR_GENERATORS: [(&str, Generator); 4] = [
    ("stone_tile", generate_stone_tile_floor),
    ("dirt", generate_dirt_floor),
    ("cobblestone", generate_cobblestone_floor),
    ("grass", generate_grass_floor),
];

const CEILING_GENERATORS: [(&str, Generator); 3] = [
    ("dark_rock", generate_dark_rock_ceiling),
    ("wooden_beams", generate_wooden_beams_ceiling),
    ("canopy", generate_canopy_ceiling),
];

/// Cached dungeon surface materials, keyed by texture name, plus the door
/// material set and stair extras.
#[derive(Resource)]
pub struct DungeonMaterials {
    walls: HashMap<String, Handle<StandardMaterial>>,
    /// Same wall textures with repeat wrapping, for stair side walls that
    /// tile beyond 0..1 UVs.
    walls_repeat: HashMap<String, Handle<StandardMaterial>>,
    floors: HashMap<String, Handle<StandardMaterial>>,
    ceilings: HashMap<String, Handle<StandardMaterial>>,
    pub door: Handle<StandardMaterial>,
    pub locked_door: Handle<StandardMaterial>,
    pub door_frame: Handle<StandardMaterial>,
    pub door_button: Handle<StandardMaterial>,
    /// Pure black unlit darkness beyond stairwells.
    pub stair_back: Handle<StandardMaterial>,
}

impl DungeonMaterials {
    pub fn generate(images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) -> Self {
        let build = |generators: &[(&str, Generator)]| {
            let mut map = HashMap::new();
            for (name, generator) in generators {
                let mut rng = CanvasRng::new(seed_for(name));
                let image = images.add(canvas_to_image(generator(&mut rng)));
                let material = materials.add(StandardMaterial {
                    base_color_texture: Some(image),
                    perceptual_roughness: 1.0,
                    metallic: 0.0,
                    reflectance: 0.0,
                    ..default()
                });
                map.insert((*name).to_string(), material);
            }
            map
        };
        let (walls, floors, ceilings) = {
            let mut build_set = build;
            let walls = build_set(&WALL_GENERATORS);
            let floors = build_set(&FLOOR_GENERATORS);
            let ceilings = build_set(&CEILING_GENERATORS);
            (walls, floors, ceilings)
        };

        let lambert = |image: Handle<Image>| StandardMaterial {
            base_color_texture: Some(image),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            ..default()
        };

        let mut walls_repeat = HashMap::new();
        for (name, generator) in &WALL_GENERATORS {
            let mut rng = CanvasRng::new(seed_for(name));
            let image = images.add(image_with_repeat(generator(&mut rng)));
            walls_repeat.insert((*name).to_string(), materials.add(lambert(image)));
        }
        let mut door_rng = CanvasRng::new(seed_for("door"));
        let door_image = images.add(canvas_to_image(generate_door(&mut door_rng)));
        let mut locked_rng = CanvasRng::new(seed_for("locked_door"));
        let locked_image = images.add(canvas_to_image(generate_locked_door(&mut locked_rng)));
        let mut frame_rng = CanvasRng::new(seed_for("door_frame"));
        let frame_image = images.add(image_with_repeat(generate_door_frame(&mut frame_rng)));

        Self {
            walls,
            walls_repeat,
            floors,
            ceilings,
            door: materials.add(StandardMaterial {
                cull_mode: None,
                ..lambert(door_image)
            }),
            locked_door: materials.add(StandardMaterial {
                cull_mode: None,
                ..lambert(locked_image)
            }),
            door_frame: materials.add(lambert(frame_image)),
            door_button: materials.add(StandardMaterial {
                base_color: Color::srgb_u8(0xcc, 0x88, 0x33),
                perceptual_roughness: 1.0,
                metallic: 0.0,
                reflectance: 0.0,
                ..default()
            }),
            stair_back: materials.add(StandardMaterial {
                base_color: Color::BLACK,
                unlit: true,
                ..default()
            }),
        }
    }

    pub fn wall(&self, name: &str) -> Handle<StandardMaterial> {
        self.walls
            .get(name)
            .or_else(|| self.walls.get("stone"))
            .expect("stone wall material exists")
            .clone()
    }

    pub fn wall_repeat(&self, name: &str) -> Handle<StandardMaterial> {
        self.walls_repeat
            .get(name)
            .or_else(|| self.walls_repeat.get("stone"))
            .expect("stone repeat wall material exists")
            .clone()
    }

    pub fn floor(&self, name: &str) -> Handle<StandardMaterial> {
        self.floors
            .get(name)
            .or_else(|| self.floors.get("stone_tile"))
            .expect("stone_tile floor material exists")
            .clone()
    }

    pub fn ceiling(&self, name: &str) -> Handle<StandardMaterial> {
        self.ceilings
            .get(name)
            .or_else(|| self.ceilings.get("dark_rock"))
            .expect("dark_rock ceiling material exists")
            .clone()
    }
}

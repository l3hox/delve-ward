//! Minimal software canvas for regenerating the TS 2D-canvas procedural
//! textures. Supports the subset of operations the texture generators use:
//! alpha-blended rect fills, 1px lines, ellipse strokes and fills.

use delve_core::random::Mulberry32;

#[derive(Clone, Copy)]
pub struct Rgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: f32,
}

impl Rgba {
    pub const fn opaque(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: 1.0,
        }
    }

    pub const fn translucent(red: u8, green: u8, blue: u8, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// A decoded RGBA image used as a blit source (item icons, paperdoll art).
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct PixelCanvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl PixelCanvas {
    pub fn new(size: usize) -> Self {
        Self::with_dimensions(size, size)
    }

    pub fn with_dimensions(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width * height * 4],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn into_rgba_bytes(self) -> Vec<u8> {
        self.pixels
    }

    /// Source-over blend of `color` onto the pixel at (x, y).
    fn blend_pixel(&mut self, x: i32, y: i32, color: Rgba) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let offset = (y as usize * self.width + x as usize) * 4;
        let source_alpha = color.alpha.clamp(0.0, 1.0);
        let destination_alpha = f32::from(self.pixels[offset + 3]) / 255.0;
        let out_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
        if out_alpha <= 0.0 {
            return;
        }
        let blend_channel = |source: u8, destination: u8| -> u8 {
            let source = f32::from(source);
            let destination = f32::from(destination);
            let value = (source * source_alpha
                + destination * destination_alpha * (1.0 - source_alpha))
                / out_alpha;
            value.round().clamp(0.0, 255.0) as u8
        };
        self.pixels[offset] = blend_channel(color.red, self.pixels[offset]);
        self.pixels[offset + 1] = blend_channel(color.green, self.pixels[offset + 1]);
        self.pixels[offset + 2] = blend_channel(color.blue, self.pixels[offset + 2]);
        self.pixels[offset + 3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgba) {
        for py in y..y + height {
            for px in x..x + width {
                self.blend_pixel(px, py, color);
            }
        }
    }

    /// 1px rectangle outline, matching canvas strokeRect with lineWidth 1.
    pub fn stroke_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Rgba) {
        self.fill_rect(x, y, width, 1, color);
        self.fill_rect(x, y + height - 1, width, 1, color);
        self.fill_rect(x, y, 1, height, color);
        self.fill_rect(x + width - 1, y, 1, height, color);
    }

    /// Nearest-neighbor blit of an RGBA source image scaled into the target
    /// rectangle, with an extra alpha multiplier (canvas globalAlpha).
    pub fn blit_scaled(&mut self, source: &RgbaImage, target: (i32, i32, i32, i32), alpha: f32) {
        let (target_x, target_y, target_w, target_h) = target;
        if target_w <= 0 || target_h <= 0 || source.width == 0 || source.height == 0 {
            return;
        }
        for py in 0..target_h {
            for px in 0..target_w {
                let source_x = (px * source.width as i32 / target_w)
                    .clamp(0, source.width as i32 - 1) as usize;
                let source_y = (py * source.height as i32 / target_h)
                    .clamp(0, source.height as i32 - 1) as usize;
                let offset = (source_y * source.width + source_x) * 4;
                let color = Rgba {
                    red: source.pixels[offset],
                    green: source.pixels[offset + 1],
                    blue: source.pixels[offset + 2],
                    alpha: f32::from(source.pixels[offset + 3]) / 255.0 * alpha,
                };
                self.blend_pixel(target_x + px, target_y + py, color);
            }
        }
    }

    /// 1px Bresenham line, matching canvas strokes with lineWidth 1.
    pub fn stroke_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgba) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let step_x = if x0 < x1 { 1 } else { -1 };
        let step_y = if y0 < y1 { 1 } else { -1 };
        let mut error = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.blend_pixel(x, y, color);
            if x == x1 && y == y1 {
                break;
            }
            let doubled = 2 * error;
            if doubled >= dy {
                error += dy;
                x += step_x;
            }
            if doubled <= dx {
                error += dx;
                y += step_y;
            }
        }
    }

    pub fn stroke_ellipse(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        color: Rgba,
    ) {
        let circumference = 2.0 * std::f32::consts::PI * radius_x.max(radius_y).max(1.0);
        let steps = (circumference * 2.0).ceil() as i32;
        let mut previous: Option<(i32, i32)> = None;
        for step in 0..=steps {
            let angle = 2.0 * std::f32::consts::PI * step as f32 / steps as f32;
            let x = (center_x + radius_x * angle.cos()).round() as i32;
            let y = (center_y + radius_y * angle.sin()).round() as i32;
            if previous != Some((x, y)) {
                self.blend_pixel(x, y, color);
                previous = Some((x, y));
            }
        }
    }

    pub fn fill_ellipse(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius_x: f32,
        radius_y: f32,
        color: Rgba,
    ) {
        let min_x = (center_x - radius_x).floor() as i32;
        let max_x = (center_x + radius_x).ceil() as i32;
        let min_y = (center_y - radius_y).floor() as i32;
        let max_y = (center_y + radius_y).ceil() as i32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let nx = (x as f32 + 0.5 - center_x) / radius_x;
                let ny = (y as f32 + 0.5 - center_y) / radius_y;
                if nx * nx + ny * ny <= 1.0 {
                    self.blend_pixel(x, y, color);
                }
            }
        }
    }
}

/// Canvas-equivalent random helpers over the seeded PRNG.
pub struct CanvasRng(pub Mulberry32);

impl CanvasRng {
    pub fn new(seed: u32) -> Self {
        Self(Mulberry32::new(seed))
    }

    pub fn random(&mut self) -> f64 {
        self.0.next_f64()
    }

    /// `Math.floor(Math.random() * n)`
    pub fn below(&mut self, n: i32) -> i32 {
        (self.random() * f64::from(n)).floor() as i32
    }

    /// Vary a base colour component by +/-amount, clamped 0-255.
    pub fn vary(&mut self, base: i32, amount: i32) -> u8 {
        let offset = (self.random() * f64::from(amount * 2)).floor() as i32 - amount;
        (base + offset).clamp(0, 255) as u8
    }
}

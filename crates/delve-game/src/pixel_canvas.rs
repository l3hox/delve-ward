//! Minimal software canvas for regenerating the TS 2D-canvas procedural
//! textures. Supports the subset of operations the texture generators use:
//! alpha-blended rect fills, 1px lines, ellipse strokes and fills.

use delve_core::random::Mulberry32;

#[derive(Debug, Clone, Copy, PartialEq)]
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

/// The averaged color of every source pixel covering one target pixel, where
/// `(index, count)` locates that pixel along each axis of the target
/// rectangle. This is the sampling `ctx.drawImage` performs with
/// `imageSmoothingEnabled` at its default `true`.
///
/// Colors are accumulated premultiplied by alpha and then un-premultiplied,
/// so a transparent source pixel lowers coverage without dragging its
/// (arbitrary) color into the sprite's edges — the dark-fringe artifact a
/// naive average produces. Returns `None` when the span is empty or fully
/// transparent, meaning nothing should be written.
fn area_average(
    source: &RgbaImage,
    (x_index, x_count): (i32, i32),
    (y_index, y_count): (i32, i32),
    alpha: f32,
) -> Option<Rgba> {
    // Ceiling division over non-negative values; `i32::div_ceil` is still
    // unstable on this toolchain. Each span is at least one pixel wide so
    // upscaling stays defined.
    let span_end = |numerator: i32, denominator: i32| (numerator + denominator - 1) / denominator;
    let span_top = y_index * source.height as i32 / y_count;
    let span_bottom = span_end((y_index + 1) * source.height as i32, y_count).max(span_top + 1);
    let span_left = x_index * source.width as i32 / x_count;
    let span_right = span_end((x_index + 1) * source.width as i32, x_count).max(span_left + 1);

    let mut red = 0.0;
    let mut green = 0.0;
    let mut blue = 0.0;
    let mut coverage = 0.0;
    let mut samples = 0.0;
    for source_y in span_top..span_bottom.min(source.height as i32) {
        for source_x in span_left..span_right.min(source.width as i32) {
            let offset = (source_y as usize * source.width + source_x as usize) * 4;
            let source_alpha = f32::from(source.pixels[offset + 3]) / 255.0;
            red += f32::from(source.pixels[offset]) * source_alpha;
            green += f32::from(source.pixels[offset + 1]) * source_alpha;
            blue += f32::from(source.pixels[offset + 2]) * source_alpha;
            coverage += source_alpha;
            samples += 1.0;
        }
    }
    if samples == 0.0 || coverage == 0.0 {
        return None;
    }
    Some(Rgba {
        // Un-premultiply: divide by accumulated alpha, not the sample count.
        red: (red / coverage).round() as u8,
        green: (green / coverage).round() as u8,
        blue: (blue / coverage).round() as u8,
        alpha: coverage / samples * alpha,
    })
}

pub struct PixelCanvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    /// Physical pixels per drawing coordinate. Every primitive takes
    /// coordinates in the canvas's own drawing space and expands each one into
    /// a `scale` x `scale` block of stored pixels, so raising it multiplies
    /// the stored resolution without moving or resizing anything drawn: with
    /// the result upscaled by a nearest-neighbour sampler, one drawing
    /// coordinate covers exactly the same screen area either way.
    ///
    /// The point is headroom for content that has finer detail than the
    /// drawing grid — [`PixelCanvas::blit_icon`] writes stored pixels directly,
    /// so a 32x32 sprite reaches a 20-unit slot whole instead of being averaged
    /// down to 20 pixels. Texture generators keep `scale` at 1, where the block
    /// is one pixel and this is all a no-op.
    scale: usize,
}

impl PixelCanvas {
    pub fn new(size: usize) -> Self {
        Self::with_dimensions(size, size)
    }

    pub fn with_dimensions(width: usize, height: usize) -> Self {
        Self::supersampled(width, height, 1)
    }

    /// A canvas whose drawing space is `width` x `height` but which stores
    /// `scale` physical pixels per coordinate in each axis.
    pub fn supersampled(width: usize, height: usize, scale: usize) -> Self {
        let scale = scale.max(1);
        Self {
            width: width * scale,
            height: height * scale,
            pixels: vec![0; width * scale * height * scale * 4],
            scale,
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

    /// Source-over blend of `color` onto the drawing coordinate (x, y), which
    /// covers a `scale` x `scale` block of stored pixels.
    fn blend_pixel(&mut self, x: i32, y: i32, color: Rgba) {
        if self.scale == 1 {
            self.blend_stored_pixel(x, y, color);
            return;
        }
        let scale = self.scale as i32;
        for offset_y in 0..scale {
            for offset_x in 0..scale {
                self.blend_stored_pixel(x * scale + offset_x, y * scale + offset_y, color);
            }
        }
    }

    /// Source-over blend onto one stored pixel, bypassing [`Self::scale`].
    fn blend_stored_pixel(&mut self, x: i32, y: i32, color: Rgba) {
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

    /// Area-averaged blit over stored pixels: [`area_average`] resolved at the
    /// canvas's full stored resolution rather than its coarser drawing grid,
    /// so a sprite that can't fit whole still keeps as much detail as the
    /// canvas can hold. Reached through [`Self::blit_icon`] when the sprite is
    /// larger than its stored footprint.
    fn blit_area_averaged_stored(
        &mut self,
        source: &RgbaImage,
        stored: (i32, i32, i32, i32),
        alpha: f32,
    ) {
        let (stored_x, stored_y, stored_w, stored_h) = stored;
        if stored_w <= 0 || stored_h <= 0 || source.width == 0 || source.height == 0 {
            return;
        }
        for py in 0..stored_h {
            for px in 0..stored_w {
                if let Some(color) = area_average(source, (px, stored_w), (py, stored_h), alpha) {
                    self.blend_stored_pixel(stored_x + px, stored_y + py, color);
                }
            }
        }
    }

    /// Fills the drawing-space rectangle `target` with `source` at the finest
    /// resolution this canvas can hold, choosing the sampling by direction.
    ///
    /// The rectangle covers `target * scale` stored pixels, so at
    /// `hud::HUD_SCALE` 2 a 20-unit item slot has 40 of them for a 32x32
    /// sprite. That is an *upscale*, where point sampling reaches every source
    /// pixel — nothing is dropped and pixel-art edges stay hard, which
    /// averaging would soften. Only when the sprite is larger than its stored
    /// footprint does it need averaging, and then
    /// [`Self::blit_area_averaged_stored`] keeps the detail that point sampling
    /// would throw away.
    ///
    /// Either way the sprite fills the slot exactly as before, at the same
    /// on-screen size: `scale` changes how finely it is stored, not how large
    /// it is drawn. Returns whether the upscale path was taken.
    pub fn blit_icon(
        &mut self,
        source: &RgbaImage,
        target: (i32, i32, i32, i32),
        alpha: f32,
    ) -> bool {
        let (target_x, target_y, target_w, target_h) = target;
        if target_w <= 0 || target_h <= 0 || source.width == 0 || source.height == 0 {
            return false;
        }
        let stored = (
            target_x * self.scale as i32,
            target_y * self.scale as i32,
            target_w * self.scale as i32,
            target_h * self.scale as i32,
        );
        let fits = stored.2 as usize >= source.width && stored.3 as usize >= source.height;
        if fits {
            self.blit_nearest_stored(source, stored, alpha);
        } else {
            self.blit_area_averaged_stored(source, stored, alpha);
        }
        fits
    }

    /// Point-sampled blit over stored pixels. Only meaningful as an upscale —
    /// see [`Self::blit_icon`], its one caller, for why that is the right
    /// sampling there.
    fn blit_nearest_stored(
        &mut self,
        source: &RgbaImage,
        stored: (i32, i32, i32, i32),
        alpha: f32,
    ) {
        let (stored_x, stored_y, stored_w, stored_h) = stored;
        for py in 0..stored_h {
            let source_y = (py as usize * source.height / stored_h as usize).min(source.height - 1);
            for px in 0..stored_w {
                let source_x =
                    (px as usize * source.width / stored_w as usize).min(source.width - 1);
                let offset = (source_y * source.width + source_x) * 4;
                let color = Rgba {
                    red: source.pixels[offset],
                    green: source.pixels[offset + 1],
                    blue: source.pixels[offset + 2],
                    alpha: f32::from(source.pixels[offset + 3]) / 255.0 * alpha,
                };
                if color.alpha <= 0.0 {
                    continue;
                }
                self.blend_stored_pixel(stored_x + px, stored_y + py, color);
            }
        }
    }

    /// Nearest-neighbor blit of `source`, matching the canvas
    /// `translate(pivot); rotate(angle); drawImage(source, offset.0, offset.1, draw_size.0, draw_size.1)`
    /// sequence: `source` is scaled to `draw_size` and its top-left corner
    /// placed at `offset` in the pivot's rotated local space, then
    /// alpha-blended onto the canvas. Implemented as an inverse-transform
    /// sample over the rotated bounding box, since this canvas has no
    /// notion of a transform stack to push/pop like a real 2D context.
    pub fn blit_rotated(
        &mut self,
        source: &RgbaImage,
        pivot: (f32, f32),
        angle: f32,
        offset: (f32, f32),
        draw_size: (f32, f32),
        alpha: f32,
    ) {
        let (pivot_x, pivot_y) = pivot;
        let (offset_x, offset_y) = offset;
        let (draw_w, draw_h) = draw_size;
        if draw_w <= 0.0 || draw_h <= 0.0 || source.width == 0 || source.height == 0 {
            return;
        }
        let (sin_a, cos_a) = angle.sin_cos();
        let corners = [
            (offset_x, offset_y),
            (offset_x + draw_w, offset_y),
            (offset_x, offset_y + draw_h),
            (offset_x + draw_w, offset_y + draw_h),
        ];
        let screen_corners = corners.map(|(x, y)| {
            (
                pivot_x + x * cos_a - y * sin_a,
                pivot_y + x * sin_a + y * cos_a,
            )
        });
        let min_x = screen_corners
            .iter()
            .fold(f32::MAX, |acc, corner| acc.min(corner.0))
            .floor() as i32;
        let max_x = screen_corners
            .iter()
            .fold(f32::MIN, |acc, corner| acc.max(corner.0))
            .ceil() as i32;
        let min_y = screen_corners
            .iter()
            .fold(f32::MAX, |acc, corner| acc.min(corner.1))
            .floor() as i32;
        let max_y = screen_corners
            .iter()
            .fold(f32::MIN, |acc, corner| acc.max(corner.1))
            .ceil() as i32;

        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let delta_x = px as f32 + 0.5 - pivot_x;
                let delta_y = py as f32 + 0.5 - pivot_y;
                let local_x = delta_x * cos_a + delta_y * sin_a;
                let local_y = -delta_x * sin_a + delta_y * cos_a;
                let source_x = (local_x - offset_x) / draw_w * source.width as f32;
                let source_y = (local_y - offset_y) / draw_h * source.height as f32;
                if source_x < 0.0
                    || source_y < 0.0
                    || source_x >= source.width as f32
                    || source_y >= source.height as f32
                {
                    continue;
                }
                let source_offset = (source_y as usize * source.width + source_x as usize) * 4;
                let color = Rgba {
                    red: source.pixels[source_offset],
                    green: source.pixels[source_offset + 1],
                    blue: source.pixels[source_offset + 2],
                    alpha: f32::from(source.pixels[source_offset + 3]) / 255.0 * alpha,
                };
                self.blend_pixel(px, py, color);
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

#[cfg(test)]
mod tests {
    use super::{PixelCanvas, RgbaImage};

    fn pixel_at(canvas: &PixelCanvas, x: usize, y: usize) -> (u8, u8, u8, u8) {
        let offset = (y * canvas.width() + x) * 4;
        (
            canvas.pixels[offset],
            canvas.pixels[offset + 1],
            canvas.pixels[offset + 2],
            canvas.pixels[offset + 3],
        )
    }

    /// A sprite too large for its slot takes the area-averaged fallback: one
    /// black and one white source pixel per target pixel must average to mid
    /// grey. Nearest sampling would return whichever pixel it happened to land
    /// on instead, which is how source detail goes missing on downscale.
    #[test]
    fn downscale_averages_the_source_pixels_it_covers() {
        let source = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![0, 0, 0, 255, 255, 255, 255, 255],
        };
        let mut canvas = PixelCanvas::with_dimensions(1, 1);
        canvas.blit_icon(&source, (0, 0, 1, 1), 1.0);
        let (red, green, blue, alpha) = pixel_at(&canvas, 0, 0);
        assert_eq!((red, green, blue), (128, 128, 128));
        assert_eq!(alpha, 255);
    }

    /// A transparent neighbour must lower coverage without tinting the colour
    /// toward its own RGB — the dark-fringe artifact that averaging
    /// non-premultiplied values produces. The source's transparent pixel is
    /// deliberately bright green so a naive average would be visible.
    #[test]
    fn transparent_source_pixels_do_not_bleed_their_color() {
        let source = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![200, 40, 40, 255, 0, 255, 0, 0],
        };
        let mut canvas = PixelCanvas::with_dimensions(1, 1);
        canvas.blit_icon(&source, (0, 0, 1, 1), 1.0);
        let (red, green, blue, alpha) = pixel_at(&canvas, 0, 0);
        assert_eq!((red, green, blue), (200, 40, 40), "opaque color preserved");
        assert_eq!(alpha, 128, "coverage halved by the transparent neighbour");
    }

    /// A slot with room to spare magnifies by a whole number, so each source
    /// pixel becomes an exact block rather than an unevenly stretched one.
    #[test]
    fn upscale_replicates_source_pixels() {
        let source = RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![10, 20, 30, 255, 200, 210, 220, 255],
        };
        let mut canvas = PixelCanvas::with_dimensions(4, 2);
        canvas.blit_icon(&source, (0, 0, 4, 2), 1.0);
        for y in 0..2 {
            assert_eq!(pixel_at(&canvas, 0, y), (10, 20, 30, 255));
            assert_eq!(pixel_at(&canvas, 1, y), (10, 20, 30, 255));
            assert_eq!(pixel_at(&canvas, 2, y), (200, 210, 220, 255));
            assert_eq!(pixel_at(&canvas, 3, y), (200, 210, 220, 255));
        }
    }

    /// The item slot's exact case, and the whole point of the supersampled
    /// canvas: a 32x32 sprite in a 20-unit slot at `HUD_SCALE` 2 has 40 stored
    /// pixels to fill, so every one of the 32 source columns must appear
    /// somewhere in the output. At `HUD_SCALE` 1 the same sprite has only 20 to
    /// work with and cannot.
    #[test]
    fn a_supersampled_slot_keeps_every_source_column() {
        let mut pixels = Vec::with_capacity(32 * 4);
        for x in 0..32 {
            // Distinct value per column: a dropped column is a missing value.
            pixels.extend_from_slice(&[(x * 8) as u8, 0, 0, 255]);
        }
        let source = RgbaImage {
            width: 32,
            height: 1,
            pixels,
        };

        let mut canvas = PixelCanvas::supersampled(20, 1, 2);
        assert!(
            canvas.blit_icon(&source, (0, 0, 20, 1), 1.0),
            "40 stored pixels should take the upscale path"
        );
        let stored: Vec<u8> = (0..40).map(|x| pixel_at(&canvas, x, 0).0).collect();
        for x in 0..32u8 {
            assert!(
                stored.contains(&(x * 8)),
                "source column {x} never reached the canvas: {stored:?}"
            );
        }
    }

    /// The 32-into-20 case without the supersampling headroom: every source
    /// column still has to reach the target, so no run of identical output
    /// columns can appear where the source alternates.
    #[test]
    fn a_thirty_two_pixel_sprite_keeps_detail_in_a_twenty_pixel_slot() {
        let mut pixels = Vec::with_capacity(32 * 4);
        for x in 0..32 {
            let value = if x % 2 == 0 { 0 } else { 255 };
            pixels.extend_from_slice(&[value, value, value, 255]);
        }
        let source = RgbaImage {
            width: 32,
            height: 1,
            pixels,
        };
        let mut canvas = PixelCanvas::with_dimensions(20, 1);
        canvas.blit_icon(&source, (0, 0, 20, 1), 1.0);
        let columns: Vec<u8> = (0..20).map(|x| pixel_at(&canvas, x, 0).0).collect();
        assert!(
            columns.iter().any(|&value| value > 0 && value < 255),
            "expected blended columns, got {columns:?}"
        );
    }
}

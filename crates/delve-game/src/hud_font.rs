//! Minimal pixel font for the HUD, ported from the TS hudFont: 3x5 pixel
//! glyphs scaled up. Each glyph row packs 3 bits (bit 2 = left column).

use crate::pixel_canvas::{PixelCanvas, Rgba};

type Glyph = [u8; 5];

fn glyph(character: char) -> Option<Glyph> {
    Some(match character {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b001, 0b001],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '/' => [0b001, 0b001, 0b010, 0b100, 0b100],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        '+' => [0b010, 0b010, 0b111, 0b010, 0b010],
        ':' => [0b000, 0b010, 0b000, 0b010, 0b000],
        '(' => [0b010, 0b100, 0b100, 0b100, 0b010],
        ')' => [0b010, 0b001, 0b001, 0b001, 0b010],
        'N' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'E' => [0b111, 0b100, 0b111, 0b100, 0b111],
        'S' => [0b111, 0b100, 0b111, 0b001, 0b111],
        'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'P' => [0b111, 0b101, 0b111, 0b100, 0b100],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        'x' => [0b000, 0b101, 0b010, 0b101, 0b000],
        'K' => [0b101, 0b110, 0b100, 0b110, 0b101],
        'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
        'R' => [0b111, 0b101, 0b111, 0b110, 0b101],
        'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
        'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        'V' => [0b101, 0b101, 0b101, 0b010, 0b010],
        'C' => [0b111, 0b100, 0b100, 0b100, 0b111],
        'F' => [0b111, 0b100, 0b111, 0b100, 0b100],
        'G' => [0b111, 0b100, 0b101, 0b101, 0b111],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'O' => [0b111, 0b101, 0b101, 0b101, 0b111],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
        'J' => [0b001, 0b001, 0b001, 0b101, 0b111],
        'Q' => [0b111, 0b101, 0b101, 0b110, 0b011],
        _ => return None,
    })
}

/// Draw a text string using the pixel font; `scale` is the pixel size
/// multiplier (1 = 3x5 actual pixels). Unknown characters advance as spaces.
pub fn draw_pixel_text(
    canvas: &mut PixelCanvas,
    text: &str,
    x: i32,
    y: i32,
    color: Rgba,
    scale: i32,
) {
    let mut cursor_x = x;
    for character in text.chars() {
        let Some(rows) = glyph(character) else {
            cursor_x += 4 * scale;
            continue;
        };
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..3 {
                if bits & (4 >> column) != 0 {
                    canvas.fill_rect(
                        cursor_x + column * scale,
                        y + row as i32 * scale,
                        scale,
                        scale,
                        color,
                    );
                }
            }
        }
        cursor_x += 4 * scale; // 3px glyph + 1px spacing
    }
}

/// The pixel width of a text string at a given scale.
pub fn measure_pixel_text(text: &str, scale: i32) -> i32 {
    text.chars().count() as i32 * 4 * scale - scale
}

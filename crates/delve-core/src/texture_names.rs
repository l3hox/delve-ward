//! Texture name constants shared by the level validator and the renderer.

pub const WALL_TEXTURES: [&str; 5] = ["stone", "brick", "mossy", "wood", "forest"];
pub const FLOOR_TEXTURES: [&str; 4] = ["stone_tile", "dirt", "cobblestone", "grass"];
pub const CEILING_TEXTURES: [&str; 3] = ["dark_rock", "wooden_beams", "canopy"];

#[must_use]
pub fn is_wall_texture(name: &str) -> bool {
    WALL_TEXTURES.contains(&name)
}

#[must_use]
pub fn is_floor_texture(name: &str) -> bool {
    FLOOR_TEXTURES.contains(&name)
}

#[must_use]
pub fn is_ceiling_texture(name: &str) -> bool {
    CEILING_TEXTURES.contains(&name)
}

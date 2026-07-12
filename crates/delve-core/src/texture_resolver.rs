//! Per-cell texture resolution: hard-coded defaults, then level defaults,
//! then charDef overrides, then areas (later entries win).

use crate::types::{CharDef, TextureArea, TextureSet};

pub const DEFAULT_WALL: &str = "stone";
pub const DEFAULT_FLOOR: &str = "stone_tile";
pub const DEFAULT_CEILING: &str = "dark_rock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTextures {
    pub wall: String,
    pub floor: String,
    pub ceiling: String,
}

#[must_use]
pub fn resolve_textures(
    col: i32,
    row: i32,
    character: char,
    defaults: Option<&TextureSet>,
    char_defs: Option<&[CharDef]>,
    areas: Option<&[TextureArea]>,
) -> ResolvedTextures {
    let mut wall = DEFAULT_WALL.to_string();
    let mut floor = DEFAULT_FLOOR.to_string();
    let mut ceiling = DEFAULT_CEILING.to_string();

    let apply = |set: &TextureSet, wall: &mut String, floor: &mut String, ceiling: &mut String| {
        if let Some(texture) = &set.wall_texture {
            *wall = texture.clone();
        }
        if let Some(texture) = &set.floor_texture {
            *floor = texture.clone();
        }
        if let Some(texture) = &set.ceiling_texture {
            *ceiling = texture.clone();
        }
    };

    if let Some(defaults) = defaults {
        apply(defaults, &mut wall, &mut floor, &mut ceiling);
    }

    if let Some(def) = char_defs.and_then(|defs| defs.iter().find(|def| def.character == character))
    {
        apply(&def.textures, &mut wall, &mut floor, &mut ceiling);
    }

    if let Some(areas) = areas {
        for area in areas {
            if col >= area.from_col
                && col <= area.to_col
                && row >= area.from_row
                && row <= area.to_row
            {
                apply(&area.textures, &mut wall, &mut floor, &mut ceiling);
            }
        }
    }

    ResolvedTextures {
        wall,
        floor,
        ceiling,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture_set(wall: Option<&str>, floor: Option<&str>, ceiling: Option<&str>) -> TextureSet {
        TextureSet {
            wall_texture: wall.map(ToString::to_string),
            floor_texture: floor.map(ToString::to_string),
            ceiling_texture: ceiling.map(ToString::to_string),
        }
    }

    fn area(bounds: (i32, i32, i32, i32), wall: Option<&str>) -> TextureArea {
        TextureArea {
            from_col: bounds.0,
            to_col: bounds.1,
            from_row: bounds.2,
            to_row: bounds.3,
            environment: None,
            open_bottom: None,
            open_top: None,
            textures: texture_set(wall, None, None),
        }
    }

    #[test]
    fn hard_coded_defaults_without_overrides() {
        let resolved = resolve_textures(0, 0, '.', None, None, None);
        assert_eq!(resolved.wall, "stone");
        assert_eq!(resolved.floor, "stone_tile");
        assert_eq!(resolved.ceiling, "dark_rock");
    }

    #[test]
    fn level_defaults_override_hard_coded() {
        let defaults = texture_set(Some("brick"), Some("dirt"), None);
        let resolved = resolve_textures(0, 0, '.', Some(&defaults), None, None);
        assert_eq!(resolved.wall, "brick");
        assert_eq!(resolved.floor, "dirt");
        assert_eq!(resolved.ceiling, "dark_rock");
    }

    #[test]
    fn char_defs_override_level_defaults() {
        let defaults = texture_set(Some("brick"), None, None);
        let char_defs = vec![CharDef {
            character: 'b',
            solid: false,
            see_through: None,
            textures: texture_set(Some("mossy"), None, None),
        }];
        let resolved = resolve_textures(0, 0, 'b', Some(&defaults), Some(&char_defs), None);
        assert_eq!(resolved.wall, "mossy");
        let other = resolve_textures(0, 0, '.', Some(&defaults), Some(&char_defs), None);
        assert_eq!(other.wall, "brick");
    }

    #[test]
    fn areas_override_everything_and_later_entries_win() {
        let defaults = texture_set(Some("brick"), None, None);
        let areas = vec![
            area((0, 5, 0, 5), Some("wood")),
            area((2, 3, 2, 3), Some("mossy")),
        ];
        let inside_both = resolve_textures(2, 2, '.', Some(&defaults), None, Some(&areas));
        assert_eq!(inside_both.wall, "mossy");
        let inside_first = resolve_textures(0, 0, '.', Some(&defaults), None, Some(&areas));
        assert_eq!(inside_first.wall, "wood");
        let outside = resolve_textures(9, 9, '.', Some(&defaults), None, Some(&areas));
        assert_eq!(outside.wall, "brick");
    }
}

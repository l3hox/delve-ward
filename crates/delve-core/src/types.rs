//! Typed model of the dungeon/level JSON schema. Field names mirror the JSON
//! (camelCase) via serde renames; the schema reference is DUNGEON-DESIGNER.md
//! in the TS repo.

use crate::grid::Facing;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Dungeon,
    Mist,
    Forest,
    Outdoor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Skybox {
    #[serde(rename = "starry-night")]
    StarryNight,
    #[serde(rename = "daylight")]
    Daylight,
    #[serde(rename = "sunset")]
    Sunset,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_texture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_texture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling_texture: Option<String>,
}

/// A custom grid character definition. Level-global.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharDef {
    #[serde(rename = "char")]
    pub character: char,
    pub solid: bool,
    /// Solid but renders floor/ceiling, no wall faces toward renderable neighbors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub see_through: Option<bool>,
    #[serde(flatten)]
    pub textures: TextureSet,
}

/// A rectangular texture/environment override region within one layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureArea {
    pub from_col: i32,
    pub to_col: i32,
    pub from_row: i32,
    pub to_row: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    /// Skip floor geometry (hollow area — see through to the layer below).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_bottom: Option<bool>,
    /// Skip ceiling geometry (hollow area — see through to the layer above).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_top: Option<bool>,
    #[serde(flatten)]
    pub textures: TextureSet,
}

/// One entity placed on the grid. `col`/`row`/`type` are common to all entity
/// types; type-specific fields (keyId, targets, state, ...) live in `props`,
/// preserved verbatim for the systems that consume them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    #[serde(rename = "type")]
    pub entity_type: String,
    #[serde(flatten)]
    pub props: Map<String, Value>,
}

impl Entity {
    #[must_use]
    pub fn prop_str(&self, key: &str) -> Option<&str> {
        self.props.get(key).and_then(Value::as_str)
    }

    #[must_use]
    pub fn prop_f64(&self, key: &str) -> Option<f64> {
        self.props.get(key).and_then(Value::as_f64)
    }

    #[must_use]
    pub fn prop_bool(&self, key: &str) -> Option<bool> {
        self.props.get(key).and_then(Value::as_bool)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Explicit Y offset; default is `index * LAYER_HEIGHT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_offset: Option<f64>,
    pub grid: Vec<String>,
    pub entities: Vec<Entity>,
    /// Render ceiling geometry (default: true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<TextureSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub areas: Option<Vec<TextureArea>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStart {
    pub col: i32,
    pub row: i32,
    pub facing: Facing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_index: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DungeonLevel {
    /// Stable identifier for save/load keying; required in dungeon files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    /// Convenience copy of `layers[0].grid`, set during validation.
    pub grid: Vec<String>,
    /// Single-level mode only; dungeon files carry a dungeon-level start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_start: Option<PlayerStart>,
    /// Convenience copy of `layers[0]`'s valid entities, set during validation.
    pub entities: Vec<Entity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling: Option<bool>,
    /// Procedural skybox visible through ceiling openings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skybox: Option<Skybox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dust_motes: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_drips: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fireflies: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<TextureSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_defs: Option<Vec<CharDef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub areas: Option<Vec<TextureArea>>,
    pub layers: Vec<LayerDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DungeonPlayerStart {
    pub level_id: String,
    pub col: i32,
    pub row: i32,
    pub facing: Facing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_index: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dungeon {
    pub name: String,
    pub levels: Vec<DungeonLevel>,
    pub player_start: DungeonPlayerStart,
}

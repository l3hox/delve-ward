//! Ported from `src/level/levelLoader.test.ts` in the TS repo. Error and
//! warning message assertions use substring matching, mirroring vitest's
//! `toThrow(string)` semantics.

use delve_core::level_loader::{
    ValidationContext, migrate_entities, validate_dungeon, validate_level,
};
use delve_core::types::{Dungeon, DungeonLevel};
use serde_json::{Value, json};

fn check_level(data: Value) -> (Result<DungeonLevel, String>, Vec<String>) {
    let mut warnings = Vec::new();
    let result = validate_level(data, "test", &ValidationContext::default(), &mut warnings);
    (result, warnings)
}

fn level_ok(data: Value) -> DungeonLevel {
    let (result, _) = check_level(data);
    result.expect("level should validate")
}

fn level_err(data: Value, expected: &str) {
    let (result, _) = check_level(data);
    let error = result.expect_err("level should be rejected");
    assert!(
        error.contains(expected),
        "expected error containing {expected:?}, got {error:?}"
    );
}

fn check_dungeon(data: Value) -> (Result<Dungeon, String>, Vec<String>) {
    let mut warnings = Vec::new();
    let result = validate_dungeon(data, "test", &ValidationContext::default(), &mut warnings);
    (result, warnings)
}

fn dungeon_err(data: Value, expected: &str) {
    let (result, _) = check_dungeon(data);
    let error = result.expect_err("dungeon should be rejected");
    assert!(
        error.contains(expected),
        "expected error containing {expected:?}, got {error:?}"
    );
}

fn has_warning(warnings: &[String], expected: &str) -> bool {
    warnings.iter().any(|warning| warning.contains(expected))
}

/// Mirrors the TS `validLevel` test helper: a 3x3 box with overrides merged in
/// and a single layer derived from the grid/entities/defaults/areas overrides.
fn valid_level(overrides: Value) -> Value {
    let overrides = overrides.as_object().cloned().unwrap_or_default();
    let grid = overrides
        .get("grid")
        .cloned()
        .unwrap_or_else(|| json!(["###", "#.#", "###"]));
    let entities = overrides
        .get("entities")
        .cloned()
        .unwrap_or_else(|| json!([]));

    let mut layer = json!({ "id": "0", "grid": grid, "entities": entities });
    if let Some(defaults) = overrides.get("defaults") {
        layer["defaults"] = defaults.clone();
    }
    if let Some(areas) = overrides.get("areas") {
        layer["areas"] = areas.clone();
    }
    let layers = overrides
        .get("layers")
        .cloned()
        .unwrap_or_else(|| json!([layer]));

    let mut base = json!({
        "name": "Test",
        "grid": grid,
        "playerStart": { "col": 1, "row": 1, "facing": "N" },
        "entities": entities,
        "layers": layers,
    });
    let base_object = base.as_object_mut().expect("base is an object");
    for (key, value) in overrides {
        if key == "layers" {
            continue;
        }
        base_object.insert(key, value);
    }
    base
}

// --- validateLevel ---

#[test]
fn accepts_a_valid_level() {
    let level = level_ok(valid_level(json!({})));
    assert_eq!(level.name, "Test");
    assert_eq!(level.grid.len(), 3);
    assert!(matches!(
        level.player_start.map(|start| start.facing),
        Some(delve_core::grid::Facing::N)
    ));
    assert!(level.entities.is_empty());
}

#[test]
fn rejects_non_object_data() {
    level_err(json!(null), "is not an object");
    level_err(json!("string"), "is not an object");
    level_err(json!(42), "is not an object");
}

#[test]
fn rejects_missing_or_non_string_name() {
    level_err(
        valid_level(json!({ "name": null })),
        "\"name\" must be a string",
    );
    level_err(
        valid_level(json!({ "name": 123 })),
        "\"name\" must be a string",
    );
}

#[test]
fn rejects_missing_or_empty_grid_in_layer() {
    level_err(
        valid_level(json!({ "layers": [{ "id": "0", "entities": [] }] })),
        "\"grid\" must be a non-empty array",
    );
    level_err(
        valid_level(json!({ "layers": [{ "id": "0", "grid": [], "entities": [] }] })),
        "\"grid\" must be a non-empty array",
    );
    level_err(
        valid_level(json!({ "layers": [{ "id": "0", "grid": [1, 2], "entities": [] }] })),
        "\"grid\" must be a non-empty array",
    );
}

#[test]
fn rejects_grid_rows_with_inconsistent_lengths() {
    level_err(
        valid_level(json!({ "grid": ["###", "#.", "###"] })),
        "all grid rows must be the same length",
    );
}

#[test]
fn rejects_unknown_cell_characters() {
    level_err(
        valid_level(json!({ "grid": ["###", "#X#", "###"] })),
        "unknown cell character 'X'",
    );
}

#[test]
fn rejects_missing_or_invalid_player_start() {
    level_err(
        valid_level(json!({ "playerStart": null })),
        "\"playerStart\" must be an object",
    );
    level_err(
        valid_level(json!({ "playerStart": "bad" })),
        "\"playerStart\" must be an object",
    );
}

#[test]
fn rejects_player_start_with_non_numeric_col_row() {
    level_err(
        valid_level(json!({ "playerStart": { "col": "a", "row": 1, "facing": "N" } })),
        "must have numeric col and row",
    );
}

#[test]
fn rejects_invalid_facing() {
    level_err(
        valid_level(json!({ "playerStart": { "col": 1, "row": 1, "facing": "X" } })),
        "\"playerStart.facing\" must be one of",
    );
}

#[test]
fn rejects_player_start_out_of_grid_bounds() {
    level_err(
        valid_level(json!({ "playerStart": { "col": 10, "row": 1, "facing": "N" } })),
        "is out of grid bounds",
    );
}

#[test]
fn warns_not_throws_for_player_start_on_a_wall_cell() {
    let (result, warnings) = check_level(valid_level(
        json!({ "playerStart": { "col": 0, "row": 0, "facing": "N" } }),
    ));
    assert!(result.is_ok());
    assert!(has_warning(&warnings, "not a walkable tile"));
}

#[test]
fn rejects_missing_entities_in_layer() {
    level_err(
        valid_level(json!({ "layers": [{ "id": "0", "grid": ["###", "#.#", "###"] }] })),
        "\"entities\" must be an array",
    );
    level_err(
        valid_level(
            json!({ "layers": [{ "id": "0", "grid": ["###", "#.#", "###"], "entities": "bad" }] }),
        ),
        "\"entities\" must be an array",
    );
}

// --- defaults validation ---

#[test]
fn accepts_valid_defaults() {
    let level = level_ok(valid_level(json!({
        "defaults": { "wallTexture": "brick", "floorTexture": "dirt", "ceilingTexture": "wooden_beams" },
    })));
    let defaults = level.defaults.expect("defaults present");
    assert_eq!(defaults.wall_texture.as_deref(), Some("brick"));
    assert_eq!(defaults.floor_texture.as_deref(), Some("dirt"));
    assert_eq!(defaults.ceiling_texture.as_deref(), Some("wooden_beams"));
}

#[test]
fn accepts_missing_defaults() {
    let level = level_ok(valid_level(json!({})));
    assert!(level.defaults.is_none());
}

#[test]
fn rejects_non_object_defaults() {
    level_err(
        valid_level(json!({ "defaults": "bad" })),
        "\"defaults\" must be an object",
    );
    level_err(
        valid_level(json!({ "defaults": [1] })),
        "\"defaults\" must be an object",
    );
}

#[test]
fn rejects_unknown_textures_in_defaults() {
    level_err(
        valid_level(json!({ "defaults": { "wallTexture": "marble" } })),
        "defaults has unknown wallTexture \"marble\"",
    );
    level_err(
        valid_level(json!({ "defaults": { "floorTexture": "lava" } })),
        "defaults has unknown floorTexture \"lava\"",
    );
    level_err(
        valid_level(json!({ "defaults": { "ceilingTexture": "glass" } })),
        "defaults has unknown ceilingTexture \"glass\"",
    );
}

// --- areas validation ---

#[test]
fn accepts_valid_areas() {
    let level = level_ok(valid_level(json!({
        "areas": [ { "fromCol": 1, "toCol": 1, "fromRow": 1, "toRow": 1, "wallTexture": "brick" } ],
    })));
    assert_eq!(level.areas.expect("areas present").len(), 1);
}

#[test]
fn accepts_missing_areas() {
    let level = level_ok(valid_level(json!({})));
    assert!(level.areas.is_none());
}

#[test]
fn rejects_non_array_areas() {
    level_err(
        valid_level(json!({ "areas": "bad" })),
        "\"areas\" must be an array",
    );
    level_err(
        valid_level(json!({ "areas": {} })),
        "\"areas\" must be an array",
    );
}

#[test]
fn rejects_non_object_entries_in_areas() {
    level_err(
        valid_level(json!({ "areas": [42] })),
        "areas[0] must be an object",
    );
    level_err(
        valid_level(json!({ "areas": [null] })),
        "areas[0] must be an object",
    );
    level_err(
        valid_level(json!({ "areas": [[1, 1]] })),
        "areas[0] must be an object",
    );
}

#[test]
fn rejects_missing_coordinate_fields_in_areas() {
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 1, "toCol": 1, "fromRow": 1, "wallTexture": "brick" } ],
        })),
        "must have numeric fromCol, toCol, fromRow, toRow",
    );
}

#[test]
fn rejects_inverted_coordinate_ranges_in_areas() {
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 2, "toCol": 1, "fromRow": 1, "toRow": 1, "wallTexture": "brick" } ],
        })),
        "fromCol > toCol or fromRow > toRow",
    );
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 1, "toCol": 1, "fromRow": 2, "toRow": 1, "wallTexture": "brick" } ],
        })),
        "fromCol > toCol or fromRow > toRow",
    );
}

#[test]
fn rejects_out_of_bounds_areas() {
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 0, "toCol": 10, "fromRow": 0, "toRow": 0, "wallTexture": "brick" } ],
        })),
        "is out of grid bounds",
    );
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 0, "toCol": 0, "fromRow": 0, "toRow": 10, "wallTexture": "brick" } ],
        })),
        "is out of grid bounds",
    );
}

#[test]
fn rejects_areas_with_no_textures_specified() {
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 1, "toCol": 1, "fromRow": 1, "toRow": 1 } ],
        })),
        "must specify at least one texture",
    );
}

#[test]
fn rejects_unknown_textures_in_areas() {
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 1, "toCol": 1, "fromRow": 1, "toRow": 1, "wallTexture": "marble" } ],
        })),
        "unknown wallTexture \"marble\"",
    );
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 1, "toCol": 1, "fromRow": 1, "toRow": 1, "floorTexture": "lava" } ],
        })),
        "unknown floorTexture \"lava\"",
    );
    level_err(
        valid_level(json!({
            "areas": [ { "fromCol": 1, "toCol": 1, "fromRow": 1, "toRow": 1, "ceilingTexture": "glass" } ],
        })),
        "unknown ceilingTexture \"glass\"",
    );
}

// --- charDefs validation ---

#[test]
fn accepts_valid_walkable_char_def() {
    let level = level_ok(valid_level(json!({
        "charDefs": [ { "char": "b", "solid": false, "wallTexture": "brick", "floorTexture": "stone_tile" } ],
        "grid": ["###", "#b#", "###"],
    })));
    assert_eq!(level.char_defs.expect("charDefs present").len(), 1);
}

#[test]
fn accepts_valid_solid_char_def() {
    let level = level_ok(valid_level(json!({
        "charDefs": [ { "char": "@", "solid": true, "wallTexture": "wood" } ],
        "grid": ["###", "#.#", "###"],
    })));
    assert_eq!(level.char_defs.expect("charDefs present").len(), 1);
}

#[test]
fn accepts_missing_char_defs() {
    let level = level_ok(valid_level(json!({})));
    assert!(level.char_defs.is_none());
}

#[test]
fn rejects_non_array_char_defs() {
    level_err(
        valid_level(json!({ "charDefs": "bad" })),
        "\"charDefs\" must be an array",
    );
    level_err(
        valid_level(json!({ "charDefs": {} })),
        "\"charDefs\" must be an array",
    );
}

#[test]
fn rejects_non_object_char_defs_entry() {
    level_err(
        valid_level(json!({ "charDefs": [42] })),
        "charDefs[0] must be an object",
    );
    level_err(
        valid_level(json!({ "charDefs": [null] })),
        "charDefs[0] must be an object",
    );
    level_err(
        valid_level(json!({ "charDefs": [["b"]] })),
        "charDefs[0] must be an object",
    );
}

#[test]
fn rejects_multi_char_char_in_char_defs() {
    level_err(
        valid_level(json!({ "charDefs": [ { "char": "bb", "solid": false } ] })),
        "charDefs[0].char must be a single character",
    );
}

#[test]
fn rejects_built_in_char_conflict_in_char_defs() {
    level_err(
        valid_level(json!({ "charDefs": [ { "char": "#", "solid": true } ] })),
        "charDefs[0].char '#' conflicts with built-in character",
    );
    level_err(
        valid_level(json!({ "charDefs": [ { "char": ".", "solid": false } ] })),
        "charDefs[0].char '.' conflicts with built-in character",
    );
    level_err(
        valid_level(json!({ "charDefs": [ { "char": " ", "solid": true } ] })),
        "charDefs[0].char ' ' conflicts with built-in character",
    );
}

#[test]
fn rejects_duplicate_char_in_char_defs() {
    level_err(
        valid_level(json!({
            "charDefs": [
                { "char": "b", "solid": false },
                { "char": "b", "solid": true },
            ],
        })),
        "charDefs[1].char 'b' is a duplicate",
    );
}

#[test]
fn rejects_missing_or_non_boolean_solid_in_char_defs() {
    level_err(
        valid_level(json!({ "charDefs": [ { "char": "b" } ] })),
        "charDefs[0].solid must be a boolean",
    );
    level_err(
        valid_level(json!({ "charDefs": [ { "char": "b", "solid": "yes" } ] })),
        "charDefs[0].solid must be a boolean",
    );
}

#[test]
fn rejects_unknown_texture_names_in_char_defs() {
    level_err(
        valid_level(
            json!({ "charDefs": [ { "char": "b", "solid": false, "wallTexture": "marble" } ] }),
        ),
        "charDefs[0] has unknown wallTexture \"marble\"",
    );
    level_err(
        valid_level(
            json!({ "charDefs": [ { "char": "b", "solid": false, "floorTexture": "lava" } ] }),
        ),
        "charDefs[0] has unknown floorTexture \"lava\"",
    );
    level_err(
        valid_level(
            json!({ "charDefs": [ { "char": "b", "solid": false, "ceilingTexture": "glass" } ] }),
        ),
        "charDefs[0] has unknown ceilingTexture \"glass\"",
    );
}

#[test]
fn accepts_grid_with_char_def_characters() {
    let level = level_ok(valid_level(json!({
        "charDefs": [
            { "char": "b", "solid": false, "wallTexture": "brick" },
            { "char": "@", "solid": true, "wallTexture": "wood" },
        ],
        "grid": ["#@#", "#b#", "###"],
        "playerStart": { "col": 1, "row": 1, "facing": "N" },
    })));
    assert_eq!(level.grid[0], "#@#");
}

#[test]
fn rejects_unknown_chars_in_grid_even_with_char_defs() {
    level_err(
        valid_level(json!({
            "charDefs": [ { "char": "b", "solid": false } ],
            "grid": ["###", "#X#", "###"],
        })),
        "unknown cell character 'X'",
    );
}

#[test]
fn accepts_player_start_on_walkable_char_def_cell() {
    let level = level_ok(valid_level(json!({
        "charDefs": [ { "char": "b", "solid": false, "wallTexture": "brick" } ],
        "grid": ["###", "#b#", "###"],
        "playerStart": { "col": 1, "row": 1, "facing": "N" },
    })));
    assert_eq!(level.player_start.expect("start present").col, 1);
}

#[test]
fn warns_not_throws_for_player_start_on_solid_char_def_cell() {
    let (result, warnings) = check_level(valid_level(json!({
        "charDefs": [ { "char": "@", "solid": true, "wallTexture": "wood" } ],
        "grid": ["###", "#@#", "###"],
        "playerStart": { "col": 1, "row": 1, "facing": "N" },
    })));
    assert!(result.is_ok());
    assert!(has_warning(&warnings, "not a walkable tile"));
}

// --- entity validation ---

fn door_level(entities: Value) -> Value {
    valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "E" },
        "entities": entities,
    }))
}

#[test]
fn accepts_valid_door_entity() {
    let level = level_ok(door_level(json!([
        { "col": 2, "row": 1, "type": "door", "state": "closed" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn accepts_valid_door_entity_with_key_id() {
    let level = level_ok(door_level(json!([
        { "col": 2, "row": 1, "type": "door", "state": "closed", "keyId": "gold_key" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn skips_door_entity_with_invalid_state() {
    let (result, warnings) = check_level(door_level(json!([
        { "col": 2, "row": 1, "type": "door", "state": "broken" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn skips_door_entity_on_non_walkable_cell() {
    let (result, warnings) = check_level(valid_level(json!({
        "grid": ["#####", "#.#.#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "E" },
        "entities": [ { "col": 2, "row": 1, "type": "door", "state": "closed" } ],
    })));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn accepts_valid_key_entity() {
    let level = level_ok(door_level(json!([
        { "col": 1, "row": 2, "type": "key", "keyId": "gold_key" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn skips_key_entity_without_key_id() {
    let (result, warnings) = check_level(door_level(json!([
        { "col": 1, "row": 2, "type": "key" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn accepts_valid_lever_entity() {
    let level = level_ok(valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": [
            { "col": 2, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 2, "row": 2, "type": "lever", "id": "lever_1", "targets": ["door_1"] },
        ],
    })));
    assert_eq!(level.entities.len(), 2);
}

#[test]
fn accepts_valid_lever_entity_using_legacy_target_door() {
    let level = level_ok(valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": [
            { "col": 2, "row": 1, "type": "door", "state": "closed" },
            { "col": 2, "row": 2, "type": "lever", "targetDoor": "2,1" },
        ],
    })));
    assert_eq!(level.entities.len(), 2);
    let lever = level
        .entities
        .iter()
        .find(|entity| entity.entity_type == "lever")
        .expect("lever present");
    let targets = lever
        .props
        .get("targets")
        .and_then(Value::as_array)
        .expect("targets array");
    assert_eq!(targets.len(), 1);
    assert!(targets[0].is_string());
    assert!(lever.props.get("target").is_none());
    assert!(lever.props.get("targetDoor").is_none());
}

#[test]
fn skips_lever_with_no_target_and_no_target_door() {
    let (result, warnings) = check_level(valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": [ { "col": 2, "row": 2, "type": "lever" } ],
    })));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn accepts_lever_with_empty_targets_array() {
    let level = level_ok(valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": [ { "col": 2, "row": 2, "type": "lever", "id": "lever_1", "targets": [] } ],
    })));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn skips_lever_target_referencing_missing_entity_id() {
    let (result, warnings) = check_level(valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": [
            { "col": 2, "row": 2, "type": "lever", "id": "lever_1", "targets": ["no_such_door"] },
        ],
    })));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn accepts_lever_with_multiple_targets() {
    let level = level_ok(valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": [
            { "col": 1, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
            { "col": 2, "row": 1, "type": "door", "state": "closed", "id": "door_2" },
            { "col": 1, "row": 2, "type": "lever", "id": "lever_1", "targets": ["door_1", "door_2"] },
        ],
    })));
    assert_eq!(level.entities.len(), 3);
}

#[test]
fn rejects_duplicate_entity_ids() {
    level_err(
        valid_level(json!({
            "grid": ["#####", "#...#", "#...#", "#####"],
            "playerStart": { "col": 1, "row": 1, "facing": "S" },
            "entities": [
                { "col": 2, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
                { "col": 1, "row": 2, "type": "door", "state": "closed", "id": "door_1" },
            ],
        })),
        "duplicate entity id \"door_1\"",
    );
}

#[test]
fn accepts_valid_pressure_plate_entity() {
    let level = level_ok(door_level(json!([
        { "col": 2, "row": 1, "type": "door", "state": "closed", "id": "door_1" },
        { "col": 1, "row": 2, "type": "pressure_plate", "id": "plate_1", "targets": ["door_1"] },
    ])));
    assert_eq!(level.entities.len(), 2);
}

#[test]
fn skips_pressure_plate_with_no_target() {
    let (result, warnings) = check_level(door_level(json!([
        { "col": 1, "row": 2, "type": "pressure_plate" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn accepts_pressure_plate_with_empty_targets_array() {
    let level = level_ok(door_level(json!([
        { "col": 1, "row": 2, "type": "pressure_plate", "id": "plate_1", "targets": [] },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn skips_pressure_plate_target_referencing_missing_entity_id() {
    let (result, warnings) = check_level(door_level(json!([
        { "col": 1, "row": 2, "type": "pressure_plate", "id": "plate_1", "targets": ["no_such_door"] },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn skips_entity_with_out_of_bounds_position() {
    let (result, warnings) = check_level(door_level(json!([
        { "col": 20, "row": 1, "type": "door", "state": "closed" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn rejects_entity_without_numeric_col_row() {
    level_err(
        door_level(json!([ { "col": "a", "row": 1, "type": "door" } ])),
        "must have numeric col and row",
    );
}

#[test]
fn rejects_non_object_entity() {
    level_err(door_level(json!([42])), "entities[0] must be an object");
}

// --- migrateEntities ---

fn find_entity<'a>(entities: &'a [Value], entity_type: &str) -> &'a Value {
    entities
        .iter()
        .find(|entity| entity["type"] == entity_type)
        .expect("entity present")
}

#[test]
fn migrate_converts_legacy_target_door_to_targets() {
    let mut entities = vec![
        json!({ "col": 3, "row": 1, "type": "door", "state": "closed" }),
        json!({ "col": 1, "row": 2, "type": "lever", "targetDoor": "3,1" }),
    ];
    migrate_entities(&mut entities);
    let door_id = find_entity(&entities, "door")["id"].clone();
    assert!(door_id.is_string());
    let lever = find_entity(&entities, "lever");
    assert_eq!(lever["targets"], json!([door_id]));
    assert!(lever.get("target").is_none());
    assert!(lever.get("targetDoor").is_none());
}

#[test]
fn migrate_converts_legacy_target_door_for_pressure_plate() {
    let mut entities = vec![
        json!({ "col": 2, "row": 2, "type": "door", "state": "closed" }),
        json!({ "col": 1, "row": 1, "type": "pressure_plate", "targetDoor": "2,2" }),
    ];
    migrate_entities(&mut entities);
    let door_id = find_entity(&entities, "door")["id"].clone();
    let plate = find_entity(&entities, "pressure_plate");
    assert_eq!(plate["targets"], json!([door_id]));
    assert!(plate.get("target").is_none());
    assert!(plate.get("targetDoor").is_none());
}

#[test]
fn migrate_converts_existing_target_field_to_targets_array() {
    let mut entities = vec![
        json!({ "col": 3, "row": 1, "type": "door", "state": "closed", "id": "my_door" }),
        json!({ "col": 1, "row": 2, "type": "lever", "id": "my_lever", "target": "my_door", "targetDoor": "3,1" }),
    ];
    migrate_entities(&mut entities);
    let lever = find_entity(&entities, "lever");
    assert_eq!(lever["targets"], json!(["my_door"]));
    assert!(lever.get("target").is_none());
    assert!(lever.get("targetDoor").is_none());
}

#[test]
fn migrate_keeps_existing_door_ids() {
    let mut entities =
        vec![json!({ "col": 3, "row": 1, "type": "door", "state": "closed", "id": "existing_id" })];
    migrate_entities(&mut entities);
    assert_eq!(find_entity(&entities, "door")["id"], json!("existing_id"));
}

#[test]
fn migrate_generates_non_colliding_ids() {
    let mut entities = vec![
        json!({ "col": 1, "row": 1, "type": "door", "state": "closed", "id": "door_1" }),
        json!({ "col": 2, "row": 1, "type": "door", "state": "closed" }),
    ];
    migrate_entities(&mut entities);
    let ids: Vec<&Value> = entities
        .iter()
        .filter_map(|entity| entity.get("id"))
        .collect();
    let unique: std::collections::HashSet<String> = ids.iter().map(|id| id.to_string()).collect();
    assert_eq!(unique.len(), ids.len());
    let unnamed_door = entities
        .iter()
        .find(|entity| entity["col"] == 2)
        .expect("second door present");
    assert_eq!(unnamed_door["id"], json!("door_2"));
}

// --- stair entity validation ---

fn stair_level(entities: Value) -> Value {
    valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": entities,
    }))
}

#[test]
fn accepts_valid_stairs_down_entity() {
    let level = level_ok(stair_level(json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stair_up_1" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn accepts_valid_stairs_up_entity() {
    let level = level_ok(stair_level(json!([
        { "col": 2, "row": 3, "type": "stairs", "direction": "up", "facing": "N", "target": "stair_down_1" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn skips_stairs_entity_on_non_walkable_cell() {
    let (result, warnings) = check_level(valid_level(json!({
        "grid": ["#####", "#.#.#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "E" },
        "entities": [
            { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stair_up_1" },
        ],
    })));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn skips_stairs_without_direction() {
    let (result, warnings) = check_level(stair_level(json!([
        { "col": 2, "row": 1, "type": "stairs", "facing": "S", "target": "stair_up_1" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn skips_stairs_with_invalid_direction() {
    let (result, warnings) = check_level(stair_level(json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "left", "facing": "S", "target": "stair_up_1" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn skips_stairs_without_facing() {
    let (result, warnings) = check_level(stair_level(json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "target": "stair_up_1" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn skips_stairs_with_invalid_facing() {
    let (result, warnings) = check_level(stair_level(json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "X", "target": "stair_up_1" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn skips_stairs_without_target() {
    let (result, warnings) = check_level(stair_level(json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

// --- validateDungeon ---

fn valid_dungeon_level(id: &str, overrides: Value) -> Value {
    let overrides = overrides.as_object().cloned().unwrap_or_default();
    let grid = overrides
        .get("grid")
        .cloned()
        .unwrap_or_else(|| json!(["#####", "#...#", "#...#", "#...#", "#####"]));
    let entities = overrides
        .get("entities")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let layers = overrides
        .get("layers")
        .cloned()
        .unwrap_or_else(|| json!([{ "id": "0", "grid": grid, "entities": entities }]));

    let mut base = json!({
        "id": id,
        "name": format!("Level {id}"),
        "grid": grid,
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": entities,
        "layers": layers,
    });
    let base_object = base.as_object_mut().expect("base is an object");
    for (key, value) in overrides {
        if key == "layers" {
            continue;
        }
        base_object.insert(key, value);
    }
    base
}

fn valid_dungeon(overrides: Value) -> Value {
    let overrides = overrides.as_object().cloned().unwrap_or_default();
    let mut base = json!({
        "name": "Test Dungeon",
        "playerStart": { "levelId": "level1", "col": 1, "row": 1, "facing": "S" },
        "levels": [
            valid_dungeon_level("level1", json!({})),
            valid_dungeon_level("level2", json!({})),
        ],
    });
    let base_object = base.as_object_mut().expect("base is an object");
    for (key, value) in overrides {
        base_object.insert(key, value);
    }
    base
}

#[test]
fn accepts_valid_dungeon() {
    let (result, _) = check_dungeon(valid_dungeon(json!({})));
    let dungeon = result.expect("dungeon ok");
    assert_eq!(dungeon.name, "Test Dungeon");
    assert_eq!(dungeon.levels.len(), 2);
    assert_eq!(dungeon.levels[0].id.as_deref(), Some("level1"));
}

#[test]
fn dungeon_rejects_non_object_data() {
    dungeon_err(json!(null), "is not an object");
    dungeon_err(json!("string"), "is not an object");
    dungeon_err(json!(42), "is not an object");
}

#[test]
fn dungeon_rejects_missing_name() {
    dungeon_err(
        valid_dungeon(json!({ "name": null })),
        "\"name\" must be a string",
    );
    dungeon_err(
        valid_dungeon(json!({ "name": 123 })),
        "\"name\" must be a string",
    );
}

#[test]
fn dungeon_rejects_missing_or_empty_levels_array() {
    dungeon_err(
        valid_dungeon(json!({ "levels": null })),
        "\"levels\" must be a non-empty array",
    );
    dungeon_err(
        valid_dungeon(json!({ "levels": [] })),
        "\"levels\" must be a non-empty array",
    );
    dungeon_err(
        valid_dungeon(json!({ "levels": "bad" })),
        "\"levels\" must be a non-empty array",
    );
}

#[test]
fn dungeon_rejects_duplicate_level_ids() {
    dungeon_err(
        valid_dungeon(json!({
            "levels": [
                valid_dungeon_level("level1", json!({})),
                valid_dungeon_level("level1", json!({})),
            ],
        })),
        "duplicate level id \"level1\"",
    );
}

#[test]
fn dungeon_rejects_level_without_id() {
    let mut level_no_id = valid_dungeon_level("level1", json!({}));
    level_no_id.as_object_mut().expect("object").remove("id");
    dungeon_err(
        valid_dungeon(json!({
            "levels": [level_no_id, valid_dungeon_level("level2", json!({}))],
        })),
        "must have a non-empty string \"id\"",
    );
}

#[test]
fn warns_for_stair_target_not_found_on_any_level() {
    let (result, warnings) = check_dungeon(valid_dungeon(json!({
        "levels": [
            valid_dungeon_level("level1", json!({
                "entities": [
                    { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "nonexistent_stair", "id": "stair_down_1" },
                ],
            })),
            valid_dungeon_level("level2", json!({})),
        ],
    })));
    assert!(result.is_ok());
    assert!(has_warning(
        &warnings,
        "target \"nonexistent_stair\" does not match any stair entity in the dungeon"
    ));
}

#[test]
fn warns_for_stair_target_that_is_not_stairs() {
    let (result, warnings) = check_dungeon(valid_dungeon(json!({
        "levels": [
            valid_dungeon_level("level1", json!({
                "entities": [
                    { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "some_door", "id": "stair_down_1" },
                ],
            })),
            valid_dungeon_level("level2", json!({
                "entities": [
                    { "col": 2, "row": 1, "type": "door", "state": "closed", "id": "some_door" },
                ],
            })),
        ],
    })));
    assert!(result.is_ok());
    assert!(has_warning(
        &warnings,
        "target \"some_door\" is not a stairs entity"
    ));
}

#[test]
fn accepts_a_stair_pair_on_the_same_level_without_warnings() {
    let grid = json!(["#####", "#...#", "#...#", "#...#", "#####"]);
    let entities = json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stair_b", "id": "stair_a" },
        { "col": 2, "row": 3, "type": "stairs", "direction": "up", "facing": "N", "target": "stair_a", "id": "stair_b" },
    ]);
    let (result, warnings) = check_dungeon(json!({
        "name": "Test Dungeon",
        "playerStart": { "levelId": "level1", "col": 1, "row": 1, "facing": "S" },
        "levels": [
            { "id": "level1", "name": "Level level1", "grid": grid, "entities": entities,
              "layers": [{ "id": "0", "grid": grid, "entities": entities }] },
        ],
    }));
    assert!(result.is_ok());
    assert!(
        warnings.is_empty(),
        "expected no warnings, got {warnings:?}"
    );
}

#[test]
fn warns_for_stair_spawn_position_out_of_bounds() {
    let grid1 = json!(["#####", "#...#", "#...#", "#...#", "#####"]);
    let entities1 = json!([
        { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stair_up_1", "id": "stair_down_1" },
    ]);
    let grid2 = json!([".....", "#####"]);
    let entities2 = json!([
        { "col": 1, "row": 0, "type": "stairs", "direction": "up", "facing": "N", "target": "stair_down_1", "id": "stair_up_1" },
    ]);
    let (result, warnings) = check_dungeon(json!({
        "name": "Test Dungeon",
        "playerStart": { "levelId": "level1", "col": 1, "row": 1, "facing": "S" },
        "levels": [
            { "id": "level1", "name": "Level level1", "grid": grid1, "entities": entities1,
              "layers": [{ "id": "0", "grid": grid1, "entities": entities1 }] },
            { "id": "level2", "name": "Level level2", "grid": grid2, "entities": entities2,
              "layers": [{ "id": "0", "grid": grid2, "entities": entities2 }] },
        ],
    }));
    assert!(result.is_ok());
    assert!(has_warning(
        &warnings,
        "spawn position (1,-1) is out of bounds on level \"level2\""
    ));
}

#[test]
fn warns_for_stair_spawn_position_not_walkable() {
    let (result, warnings) = check_dungeon(valid_dungeon(json!({
        "levels": [
            valid_dungeon_level("level1", json!({
                "entities": [
                    { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stair_up_1", "id": "stair_down_1" },
                ],
            })),
            valid_dungeon_level("level2", json!({
                "entities": [
                    { "col": 1, "row": 1, "type": "stairs", "direction": "up", "facing": "N", "target": "stair_down_1", "id": "stair_up_1" },
                ],
            })),
        ],
    })));
    assert!(result.is_ok());
    assert!(has_warning(
        &warnings,
        "spawn position (1,0) is not walkable on level \"level2\""
    ));
}

#[test]
fn accepts_stairs_with_valid_cross_references() {
    let (result, _) = check_dungeon(valid_dungeon(json!({
        "levels": [
            valid_dungeon_level("level1", json!({
                "entities": [
                    { "col": 2, "row": 1, "type": "stairs", "direction": "down", "facing": "S", "target": "stair_up_1", "id": "stair_down_1" },
                ],
            })),
            valid_dungeon_level("level2", json!({
                "entities": [
                    { "col": 2, "row": 3, "type": "stairs", "direction": "up", "facing": "N", "target": "stair_down_1", "id": "stair_up_1" },
                ],
            })),
        ],
    })));
    let dungeon = result.expect("dungeon ok");
    assert_eq!(dungeon.levels[0].entities.len(), 1);
    assert_eq!(dungeon.levels[1].entities.len(), 1);
}

// --- Phase D: new entity type validation ---

fn phase_d_level(entities: Value) -> Value {
    valid_level(json!({
        "grid": ["#####", "#...#", "#...#", "#...#", "#####"],
        "playerStart": { "col": 1, "row": 1, "facing": "S" },
        "entities": entities,
    }))
}

#[test]
fn breakable_wall_on_solid_cell_with_hp_passes() {
    let level = level_ok(phase_d_level(json!([
        { "col": 0, "row": 0, "type": "breakable_wall", "hp": 30 },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn breakable_wall_on_walkable_cell_skipped_with_warning() {
    let (result, warnings) = check_level(phase_d_level(json!([
        { "col": 1, "row": 1, "type": "breakable_wall", "hp": 30 },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn breakable_wall_missing_hp_skipped_with_warning() {
    let (result, warnings) = check_level(phase_d_level(json!([
        { "col": 0, "row": 0, "type": "breakable_wall" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn secret_wall_on_solid_cell_passes() {
    let level = level_ok(phase_d_level(json!([
        { "col": 0, "row": 0, "type": "secret_wall" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn secret_wall_on_walkable_cell_skipped_with_warning() {
    let (result, warnings) = check_level(phase_d_level(json!([
        { "col": 1, "row": 1, "type": "secret_wall" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn block_on_walkable_cell_passes() {
    let level = level_ok(phase_d_level(json!([
        { "col": 1, "row": 1, "type": "block" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn block_on_solid_cell_skipped_with_warning() {
    let (result, warnings) = check_level(phase_d_level(json!([
        { "col": 0, "row": 0, "type": "block" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn chest_on_walkable_cell_passes() {
    let level = level_ok(phase_d_level(json!([
        { "col": 2, "row": 1, "type": "chest", "state": "closed" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn chest_with_invalid_state_skipped_with_warning() {
    let (result, warnings) = check_level(phase_d_level(json!([
        { "col": 2, "row": 1, "type": "chest", "state": "broken" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

#[test]
fn sign_with_wall_and_text_passes() {
    let level = level_ok(phase_d_level(json!([
        { "col": 2, "row": 1, "type": "sign", "wall": "N", "text": "Read me" },
    ])));
    assert_eq!(level.entities.len(), 1);
}

#[test]
fn sign_with_empty_text_skipped_with_warning() {
    let (result, warnings) = check_level(phase_d_level(json!([
        { "col": 2, "row": 1, "type": "sign", "wall": "N", "text": "" },
    ])));
    assert!(result.expect("level ok").entities.is_empty());
    assert!(!warnings.is_empty());
}

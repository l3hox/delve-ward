//! Level and dungeon validation, ported from the TS `levelLoader`.
//!
//! Validation runs against raw JSON so that error and warning messages match
//! the TS implementation exactly (they are asserted by the ported tests), then
//! decodes the validated document into the typed model from [`crate::types`].
//! Recoverable entity problems produce a warning and skip the entity; structural
//! problems fail the whole load.

use crate::grid::walkable_cells;
use crate::texture_names::{is_ceiling_texture, is_floor_texture, is_wall_texture};
use crate::types::{Dungeon, DungeonLevel, DungeonPlayerStart, Entity};
use serde_json::{Map, Value};
use std::collections::HashSet;

const VALID_FACINGS: [&str; 4] = ["N", "E", "S", "W"];
const VALID_ENVIRONMENTS: [&str; 4] = ["dungeon", "mist", "forest", "outdoor"];
const VALID_SIGNAL_MODES: [&str; 4] = ["toggle", "momentary", "one_shot", "timed"];
const VALID_GATE_TYPES: [&str; 6] = ["and", "or", "not", "delay", "pulse_edge", "pulse_repeat"];
const BUILTIN_CHARS: [char; 3] = ['.', '#', ' '];

/// Databases the entity validator checks references against. `None` mirrors the
/// TS singletons being not-yet-loaded: enemies then never resolve (all enemy
/// entities are skipped) while the npc id check is skipped entirely.
#[derive(Default, Clone, Copy)]
pub struct ValidationContext<'a> {
    pub enemy_ids: Option<&'a HashSet<String>>,
    pub npc_ids: Option<&'a HashSet<String>>,
}

/// Get all entities from a level across all layers.
pub fn get_all_level_entities(level: &DungeonLevel) -> impl Iterator<Item = &Entity> {
    level.layers.iter().flat_map(|layer| layer.entities.iter())
}

/// Resolve a layer coordinate (numeric ID like 0, 1, -1) to an array index.
/// Returns 0 if not found.
#[must_use]
pub fn resolve_layer_coord(level: &DungeonLevel, coord: i32) -> usize {
    let id = coord.to_string();
    level
        .layers
        .iter()
        .position(|layer| layer.id.as_deref() == Some(&id))
        .unwrap_or(0)
}

/// Find which layer index an entity is on (by id). Returns 0 if not found.
#[must_use]
pub fn find_entity_layer_index(level: &DungeonLevel, entity_id: &str) -> usize {
    level
        .layers
        .iter()
        .position(|layer| {
            layer
                .entities
                .iter()
                .any(|entity| entity.id.as_deref() == Some(entity_id))
        })
        .unwrap_or(0)
}

/// JS truthiness for a JSON value (`undefined` maps to `None`).
fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// Format a JSON value the way a JS template literal would.
fn display_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number
            .as_i64()
            .map_or_else(|| number.to_string(), |integer| integer.to_string()),
        other => other.to_string(),
    }
}

fn display_prop(map: &Map<String, Value>, key: &str) -> String {
    map.get(key)
        .map_or_else(|| "undefined".to_string(), display_value)
}

fn is_number(map: &Map<String, Value>, key: &str) -> bool {
    matches!(map.get(key), Some(Value::Number(_)))
}

fn as_string<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn as_f64(map: &Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(Value::as_f64)
}

/// Integral col/row of an entity or start position, if in integer range.
fn integer_coords(map: &Map<String, Value>) -> Option<(i64, i64)> {
    let col = map.get("col")?.as_i64()?;
    let row = map.get("row")?.as_i64()?;
    Some((col, row))
}

fn cell_of(grid_chars: &[Vec<char>], col: i64, row: i64) -> Option<char> {
    let row_chars = grid_chars.get(usize::try_from(row).ok()?)?;
    row_chars.get(usize::try_from(col).ok()?).copied()
}

fn validate_textures(entry: &Map<String, Value>, label: &str, source: &str) -> Result<(), String> {
    if let Some(value) = entry.get("wallTexture")
        && !value.as_str().is_some_and(is_wall_texture)
    {
        return Err(format!(
            "Level {source}: {label} has unknown wallTexture \"{}\"",
            display_value(value)
        ));
    }
    if let Some(value) = entry.get("floorTexture")
        && !value.as_str().is_some_and(is_floor_texture)
    {
        return Err(format!(
            "Level {source}: {label} has unknown floorTexture \"{}\"",
            display_value(value)
        ));
    }
    if let Some(value) = entry.get("ceilingTexture")
        && !value.as_str().is_some_and(is_ceiling_texture)
    {
        return Err(format!(
            "Level {source}: {label} has unknown ceilingTexture \"{}\"",
            display_value(value)
        ));
    }
    Ok(())
}

/// Backward-compat preprocessor: converts legacy `targetDoor: "col,row"` to
/// `target: entityId` and auto-assigns IDs to doors that lack them.
/// Mutates the entity values in place.
pub fn migrate_entities(entities: &mut [Value]) {
    let mut used_ids: HashSet<String> = HashSet::new();
    for entity in entities.iter() {
        if let Some(id) = entity.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            used_ids.insert(id.to_string());
        }
    }

    let mut door_counter: u32 = 1;
    for entity in entities.iter_mut() {
        let Some(object) = entity.as_object_mut() else {
            continue;
        };
        if as_string(object, "type") == Some("door") && !is_truthy(object.get("id")) {
            while used_ids.contains(&format!("door_{door_counter}")) {
                door_counter += 1;
            }
            let id = format!("door_{door_counter}");
            used_ids.insert(id.clone());
            door_counter += 1;
            object.insert("id".to_string(), Value::String(id));
        }
    }

    let mut door_position_to_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for entity in entities.iter() {
        let Some(object) = entity.as_object() else {
            continue;
        };
        if as_string(object, "type") == Some("door")
            && is_truthy(object.get("id"))
            && let Some(id) = as_string(object, "id")
        {
            let position = format!(
                "{},{}",
                display_prop(object, "col"),
                display_prop(object, "row")
            );
            door_position_to_id.insert(position, id.to_string());
        }
    }

    for entity in entities.iter_mut() {
        let Some(object) = entity.as_object_mut() else {
            continue;
        };
        let entity_type = as_string(object, "type");
        if entity_type != Some("lever") && entity_type != Some("pressure_plate") {
            continue;
        }
        if is_truthy(object.get("targetDoor")) && !is_truthy(object.get("target")) {
            let door_id = as_string(object, "targetDoor")
                .and_then(|position| door_position_to_id.get(position).cloned());
            if let Some(door_id) = door_id {
                object.insert("target".to_string(), Value::String(door_id));
                object.remove("targetDoor");
            }
        }
        if is_truthy(object.get("target")) && !is_truthy(object.get("targets")) {
            let target = object.remove("target").expect("target checked truthy");
            object.insert("targets".to_string(), Value::Array(vec![target]));
            object.remove("targetDoor");
        }
    }
}

fn check_targets(
    entity: &Map<String, Value>,
    prefix: &str,
    type_name: &str,
    entity_ids: &HashSet<String>,
) -> Result<(), String> {
    let Some(Value::Array(targets)) = entity.get("targets") else {
        return Err(format!("{prefix} {type_name} must have a targets array"));
    };
    for target in targets {
        let Some(target_id) = target.as_str().filter(|id| !id.is_empty()) else {
            return Err(format!(
                "{prefix} {type_name} targets must contain non-empty strings"
            ));
        };
        if !entity_ids.contains(target_id) {
            return Err(format!(
                "{prefix} {type_name} target \"{target_id}\" must reference an existing entity id"
            ));
        }
    }
    Ok(())
}

fn check_signal_mode(
    entity: &Map<String, Value>,
    prefix: &str,
    type_name: &str,
) -> Result<(), String> {
    if let Some(mode) = entity.get("signalMode")
        && !mode
            .as_str()
            .is_some_and(|m| VALID_SIGNAL_MODES.contains(&m))
    {
        return Err(format!(
            "{prefix} {type_name} signalMode must be one of {}",
            VALID_SIGNAL_MODES.join(", ")
        ));
    }
    if entity.get("signalDuration").is_some() && !is_number(entity, "signalDuration") {
        return Err(format!(
            "{prefix} {type_name} signalDuration must be a number"
        ));
    }
    if entity.get("signalDelay").is_some() && !is_number(entity, "signalDelay") {
        return Err(format!("{prefix} {type_name} signalDelay must be a number"));
    }
    Ok(())
}

fn optional_positive_number(
    entity: &Map<String, Value>,
    key: &str,
    error: String,
) -> Result<(), String> {
    match entity.get(key) {
        None => Ok(()),
        Some(value) => match value.as_f64() {
            Some(number) if number > 0.0 => Ok(()),
            _ => Err(error),
        },
    }
}

fn optional_non_negative_number(
    entity: &Map<String, Value>,
    key: &str,
    error: String,
) -> Result<(), String> {
    match entity.get(key) {
        None => Ok(()),
        Some(value) => match value.as_f64() {
            Some(number) if number >= 0.0 => Ok(()),
            _ => Err(error),
        },
    }
}

fn optional_number(entity: &Map<String, Value>, key: &str, error: String) -> Result<(), String> {
    if entity.get(key).is_some() && !is_number(entity, key) {
        return Err(error);
    }
    Ok(())
}

fn optional_bool(entity: &Map<String, Value>, key: &str, error: String) -> Result<(), String> {
    if let Some(value) = entity.get(key)
        && !value.is_boolean()
    {
        return Err(error);
    }
    Ok(())
}

fn optional_wall_direction(
    entity: &Map<String, Value>,
    key: &str,
    error: String,
) -> Result<(), String> {
    if let Some(value) = entity.get(key)
        && !value.as_str().is_some_and(|v| VALID_FACINGS.contains(&v))
    {
        return Err(error);
    }
    Ok(())
}

fn optional_gate_mode(entity: &Map<String, Value>, error: String) -> Result<(), String> {
    if let Some(value) = entity.get("gateMode")
        && !value
            .as_str()
            .is_some_and(|v| ["or", "and", "xor"].contains(&v))
    {
        return Err(error);
    }
    Ok(())
}

/// Level-global character sets derived from built-ins plus charDefs.
struct LevelCharSets {
    known: HashSet<char>,
    walkable: HashSet<char>,
}

/// Per-grid inputs shared by the entity validators.
struct GridContext<'a> {
    grid_chars: &'a [Vec<char>],
    walkable_chars: &'a HashSet<char>,
}

impl GridContext<'_> {
    fn row_len(&self) -> usize {
        grid_chars_of(self.grid_chars)
    }
}

/// Validate a single entity. Returns an error message if invalid, `Ok` if valid.
/// Invalid entities are skipped in the game but preserved in the JSON for editor use.
#[allow(clippy::too_many_lines)]
fn validate_entity(
    entity: &Map<String, Value>,
    index: usize,
    grid: &GridContext,
    entity_ids: &HashSet<String>,
    source: &str,
    ctx: &ValidationContext,
) -> Result<(), String> {
    let grid_chars = grid.grid_chars;
    let row_len = grid.row_len();
    let walkable_chars = grid.walkable_chars;
    let prefix = format!("Level {source}: entities[{index}]");

    let out_of_bounds = format!(
        "{prefix} ({},{}) is out of grid bounds",
        display_prop(entity, "col"),
        display_prop(entity, "row")
    );
    let Some((col, row)) = integer_coords(entity) else {
        return Err(out_of_bounds);
    };
    if row < 0 || row as usize >= grid_chars.len() || col < 0 || col as usize >= row_len {
        return Err(out_of_bounds);
    }
    let cell = cell_of(grid_chars, col, row);

    let cell_walkable = cell.is_some_and(|c| walkable_chars.contains(&c));
    let check_walkable = |type_name: &str| -> Result<(), String> {
        if cell_walkable {
            Ok(())
        } else {
            Err(format!(
                "{prefix} {type_name} must be on a walkable cell, found '{}'",
                cell.map_or_else(|| "undefined".to_string(), |c| c.to_string())
            ))
        }
    };

    match entity.get("type").and_then(Value::as_str).unwrap_or("") {
        "door" => {
            check_walkable("door")?;
            if let Some(state) = entity.get("state")
                && !state
                    .as_str()
                    .is_some_and(|s| ["open", "closed"].contains(&s))
            {
                return Err(format!("{prefix} door state must be open or closed"));
            }
            if entity.get("keyId").is_some() && as_string(entity, "keyId").is_none() {
                return Err(format!("{prefix} door keyId must be a string"));
            }
            optional_gate_mode(
                entity,
                format!("{prefix} door gateMode must be one of or, and, xor"),
            )?;
        }
        "key" => {
            if as_string(entity, "keyId").is_none() {
                return Err(format!("{prefix} key must have a string keyId"));
            }
            check_walkable("key")?;
        }
        "lever" => {
            check_targets(entity, &prefix, "lever", entity_ids)?;
            optional_wall_direction(
                entity,
                "wall",
                format!("{prefix} lever wall must be N, S, E, or W"),
            )?;
            check_signal_mode(entity, &prefix, "lever")?;
        }
        "pressure_plate" => {
            check_targets(entity, &prefix, "pressure_plate", entity_ids)?;
            check_walkable("pressure_plate")?;
            check_signal_mode(entity, &prefix, "pressure_plate")?;
        }
        "trigger" => {
            check_targets(entity, &prefix, "trigger", entity_ids)?;
            check_walkable("trigger")?;
            check_signal_mode(entity, &prefix, "trigger")?;
        }
        "tripwire" => {
            check_targets(entity, &prefix, "tripwire", entity_ids)?;
            check_walkable("tripwire")?;
            optional_number(
                entity,
                "visibilityThreshold",
                format!("{prefix} tripwire visibilityThreshold must be a number"),
            )?;
            if let Some(orientation) = entity.get("orientation")
                && !orientation
                    .as_str()
                    .is_some_and(|o| ["EW", "NS"].contains(&o))
            {
                return Err(format!(
                    "{prefix} tripwire orientation must be \"EW\" or \"NS\""
                ));
            }
        }
        "gate" => {
            check_targets(entity, &prefix, "gate", entity_ids)?;
            if !as_string(entity, "gateType").is_some_and(|g| VALID_GATE_TYPES.contains(&g)) {
                return Err(format!(
                    "{prefix} gate must have a valid gateType ({})",
                    VALID_GATE_TYPES.join(", ")
                ));
            }
            optional_number(
                entity,
                "delay",
                format!("{prefix} gate delay must be a number"),
            )?;
            optional_number(
                entity,
                "interval",
                format!("{prefix} gate interval must be a number"),
            )?;
        }
        "trap_launcher" => {
            check_walkable("trap_launcher")?;
            if !as_string(entity, "facing").is_some_and(|f| VALID_FACINGS.contains(&f)) {
                return Err(format!(
                    "{prefix} trap_launcher must have facing \"N\", \"S\", \"E\", or \"W\""
                ));
            }
            if !as_string(entity, "projectileType")
                .is_some_and(|p| ["dart", "arrow", "fireball"].contains(&p))
            {
                return Err(format!(
                    "{prefix} trap_launcher must have projectileType \"dart\", \"arrow\", or \"fireball\""
                ));
            }
            if !as_f64(entity, "reloadTime").is_some_and(|t| t > 0.0) {
                return Err(format!(
                    "{prefix} trap_launcher must have a positive number reloadTime"
                ));
            }
            optional_number(
                entity,
                "maxRange",
                format!("{prefix} trap_launcher maxRange must be a number"),
            )?;
        }
        "enemy" => {
            let Some(enemy_type) = as_string(entity, "enemyType") else {
                return Err(format!("{prefix} enemy must have a string enemyType"));
            };
            let known = ctx.enemy_ids.is_some_and(|ids| ids.contains(enemy_type));
            if !known {
                return Err(format!(
                    "{prefix} enemy has unknown enemyType \"{enemy_type}\""
                ));
            }
            check_walkable("enemy")?;
        }
        "torch_sconce" => {
            check_walkable("torch_sconce")?;
            optional_wall_direction(
                entity,
                "wall",
                format!("{prefix} torch_sconce wall must be N, S, E, or W"),
            )?;
        }
        "equipment" => {
            if as_string(entity, "itemId").is_none() {
                return Err(format!("{prefix} equipment must have a string itemId"));
            }
            check_walkable("equipment")?;
        }
        "consumable" => {
            if as_string(entity, "itemId").is_none() {
                return Err(format!("{prefix} consumable must have a string itemId"));
            }
            check_walkable("consumable")?;
        }
        "stairs" => {
            if !as_string(entity, "direction").is_some_and(|d| ["up", "down"].contains(&d)) {
                return Err(format!(
                    "{prefix} stairs must have direction \"up\" or \"down\""
                ));
            }
            if !as_string(entity, "facing").is_some_and(|f| VALID_FACINGS.contains(&f)) {
                return Err(format!(
                    "{prefix} stairs must have facing \"N\", \"S\", \"E\", or \"W\""
                ));
            }
            check_walkable("stairs")?;
            if as_string(entity, "target").is_none() {
                return Err(format!(
                    "{prefix} stairs must have a string target (paired stair entity ID)"
                ));
            }
        }
        "breakable_wall" => {
            if cell_walkable {
                return Err(format!(
                    "{prefix} breakable_wall must be on a solid cell, found '{}'",
                    cell.map_or_else(|| "undefined".to_string(), |c| c.to_string())
                ));
            }
            if !as_f64(entity, "hp").is_some_and(|hp| hp > 0.0) {
                return Err(format!("{prefix} breakable_wall must have a positive hp"));
            }
        }
        "secret_wall" if cell_walkable => {
            return Err(format!(
                "{prefix} secret_wall must be on a solid cell, found '{}'",
                cell.map_or_else(|| "undefined".to_string(), |c| c.to_string())
            ));
        }
        "block" => {
            check_walkable("block")?;
        }
        "chest" => {
            check_walkable("chest")?;
            if let Some(state) = entity.get("state")
                && !state
                    .as_str()
                    .is_some_and(|s| ["closed", "open", "locked"].contains(&s))
            {
                return Err(format!(
                    "{prefix} chest state must be closed, open, or locked"
                ));
            }
            if let Some(facing) = entity.get("facing")
                && !facing.as_str().is_some_and(|f| VALID_FACINGS.contains(&f))
            {
                return Err(format!("{prefix} chest facing must be N, S, E, or W"));
            }
            if entity.get("keyId").is_some() && as_string(entity, "keyId").is_none() {
                return Err(format!("{prefix} chest keyId must be a string"));
            }
            optional_gate_mode(
                entity,
                format!("{prefix} chest gateMode must be one of or, and, xor"),
            )?;
            if entity.get("targets").is_some() {
                check_targets(entity, &prefix, "chest", entity_ids)?;
            }
        }
        "sign" => {
            check_walkable("sign")?;
            optional_wall_direction(
                entity,
                "wall",
                format!("{prefix} sign wall must be N, S, E, or W"),
            )?;
            if as_string(entity, "text").is_none_or(|t| t.is_empty()) {
                return Err(format!("{prefix} sign must have non-empty text"));
            }
        }
        "npc" => {
            let Some(npc_id) = as_string(entity, "npcId") else {
                return Err(format!("{prefix} npc must have a string npcId"));
            };
            if let Some(npc_ids) = ctx.npc_ids
                && !npc_ids.contains(npc_id)
            {
                return Err(format!("{prefix} npc has unknown npcId \"{npc_id}\""));
            }
            check_walkable("npc")?;
        }
        "fountain" => {
            check_walkable("fountain")?;
            optional_positive_number(
                entity,
                "healAmount",
                format!("{prefix} fountain healAmount must be a positive number"),
            )?;
        }
        "bookshelf" => {
            check_walkable("bookshelf")?;
            optional_wall_direction(
                entity,
                "wall",
                format!("{prefix} bookshelf wall must be N, S, E, or W"),
            )?;
            if entity.get("text").is_some() && as_string(entity, "text").is_none() {
                return Err(format!("{prefix} bookshelf text must be a string"));
            }
        }
        "altar" => {
            check_walkable("altar")?;
            if let Some(buff_type) = entity.get("buffType")
                && !buff_type
                    .as_str()
                    .is_some_and(|b| ["atk", "def", "str", "dex", "vit", "wis"].contains(&b))
            {
                return Err(format!(
                    "{prefix} altar buffType must be one of atk, def, str, dex, vit, wis"
                ));
            }
            optional_positive_number(
                entity,
                "buffAmount",
                format!("{prefix} altar buffAmount must be a positive number"),
            )?;
            optional_positive_number(
                entity,
                "buffDuration",
                format!("{prefix} altar buffDuration must be a positive number"),
            )?;
        }
        "barrel" => {
            check_walkable("barrel")?;
            optional_positive_number(
                entity,
                "hp",
                format!("{prefix} barrel hp must be a positive number"),
            )?;
        }
        "thin_wall" => {
            check_walkable("thin_wall")?;
            if !as_string(entity, "wall").is_some_and(|w| ["S", "E"].contains(&w)) {
                return Err(format!(
                    "{prefix} thin_wall wall must be 'S' or 'E', got '{}'",
                    display_prop(entity, "wall")
                ));
            }
            if let Some(height) = entity.get("height")
                && !height
                    .as_str()
                    .is_some_and(|h| ["full", "half"].contains(&h))
            {
                return Err(format!(
                    "{prefix} thin_wall height must be 'full' or 'half', got '{}'",
                    display_prop(entity, "height")
                ));
            }
            optional_bool(
                entity,
                "solid",
                format!("{prefix} thin_wall solid must be boolean"),
            )?;
        }
        "ramp" => {
            check_walkable("ramp")?;
            if let Some(facing) = entity.get("facing")
                && !facing.as_str().is_some_and(|f| VALID_FACINGS.contains(&f))
            {
                return Err(format!("{prefix} ramp facing must be N, S, E, or W"));
            }
            if let Some(style) = entity.get("style")
                && !style
                    .as_str()
                    .is_some_and(|s| ["ramp", "stairs"].contains(&s))
            {
                return Err(format!("{prefix} ramp style must be 'ramp' or 'stairs'"));
            }
        }
        "prop" => {
            check_walkable("prop")?;
            const PROP_IDS: [&str; 7] = [
                "pillar",
                "rubble",
                "stalactite",
                "stalagmite",
                "statue",
                "crate_stack",
                "banner",
            ];
            let prop_id = entity.get("propId");
            if !is_truthy(prop_id)
                || !prop_id
                    .and_then(Value::as_str)
                    .is_some_and(|p| PROP_IDS.contains(&p))
            {
                return Err(format!(
                    "{prefix} prop must have a valid propId ({})",
                    PROP_IDS.join(", ")
                ));
            }
            if prop_id.and_then(Value::as_str) == Some("banner") {
                optional_wall_direction(
                    entity,
                    "wall",
                    format!("{prefix} banner wall must be N, S, E, or W"),
                )?;
            }
            if let Some(rotation) = entity.get("rotation")
                && !rotation
                    .as_f64()
                    .is_some_and(|r| [0.0, 1.0, 2.0, 3.0].contains(&r))
            {
                return Err(format!("{prefix} prop rotation must be 0, 1, 2, or 3"));
            }
        }
        "pit_trap" => {
            check_walkable("pit_trap")?;
            if let Some(state) = entity.get("state")
                && !state
                    .as_str()
                    .is_some_and(|s| ["closed", "open"].contains(&s))
            {
                return Err(format!(
                    "{prefix} pit_trap state must be 'closed' or 'open'"
                ));
            }
            optional_gate_mode(
                entity,
                format!("{prefix} pit_trap gateMode must be 'or', 'and', or 'xor'"),
            )?;
        }
        "spawner" => {
            check_walkable("spawner")?;
            if entity.get("enemyType").is_some() && as_string(entity, "enemyType").is_none() {
                return Err(format!("{prefix} spawner enemyType must be a string"));
            }
            if let Some(max_active) = entity.get("maxActive")
                && !max_active.as_f64().is_some_and(|m| m >= 1.0)
            {
                return Err(format!(
                    "{prefix} spawner maxActive must be a positive number"
                ));
            }
            optional_positive_number(
                entity,
                "interval",
                format!("{prefix} spawner interval must be a positive number"),
            )?;
            if let Some(radius) = entity.get("spawnRadius")
                && !radius.as_f64().is_some_and(|r| r >= 1.0)
            {
                return Err(format!("{prefix} spawner spawnRadius must be >= 1"));
            }
            optional_gate_mode(
                entity,
                format!("{prefix} spawner gateMode must be 'or', 'and', or 'xor'"),
            )?;
            optional_bool(
                entity,
                "visible",
                format!("{prefix} spawner visible must be a boolean"),
            )?;
        }
        "boulder" => {
            check_walkable("boulder")?;
            optional_wall_direction(
                entity,
                "direction",
                format!("{prefix} boulder direction must be 'N', 'S', 'E', or 'W'"),
            )?;
            if let Some(state) = entity.get("state")
                && !state
                    .as_str()
                    .is_some_and(|s| ["idle", "rolling", "falling"].contains(&s))
            {
                return Err(format!(
                    "{prefix} boulder state must be 'idle', 'rolling', or 'falling'"
                ));
            }
            optional_non_negative_number(
                entity,
                "rollDamage",
                format!("{prefix} boulder rollDamage must be >= 0"),
            )?;
            optional_non_negative_number(
                entity,
                "fallDamage",
                format!("{prefix} boulder fallDamage must be >= 0"),
            )?;
            optional_bool(
                entity,
                "instaKillEnemies",
                format!("{prefix} boulder instaKillEnemies must be a boolean"),
            )?;
            optional_bool(
                entity,
                "pushable",
                format!("{prefix} boulder pushable must be a boolean"),
            )?;
            optional_gate_mode(
                entity,
                format!("{prefix} boulder gateMode must be 'or', 'and', or 'xor'"),
            )?;
        }
        "boulder_spawner" => {
            optional_wall_direction(
                entity,
                "direction",
                format!("{prefix} boulder_spawner direction must be 'N', 'S', 'E', or 'W'"),
            )?;
            if let Some(mode) = entity.get("intervalMode")
                && !mode
                    .as_str()
                    .is_some_and(|m| ["fixed", "random"].contains(&m))
            {
                return Err(format!(
                    "{prefix} boulder_spawner intervalMode must be 'fixed' or 'random'"
                ));
            }
            optional_positive_number(
                entity,
                "interval",
                format!("{prefix} boulder_spawner interval must be a positive number"),
            )?;
            optional_positive_number(
                entity,
                "intervalMin",
                format!("{prefix} boulder_spawner intervalMin must be a positive number"),
            )?;
            optional_positive_number(
                entity,
                "intervalMax",
                format!("{prefix} boulder_spawner intervalMax must be a positive number"),
            )?;
            if let (Some(minimum), Some(maximum)) =
                (as_f64(entity, "intervalMin"), as_f64(entity, "intervalMax"))
                && maximum < minimum
            {
                return Err(format!(
                    "{prefix} boulder_spawner intervalMax must be >= intervalMin"
                ));
            }
            optional_non_negative_number(
                entity,
                "rollDamage",
                format!("{prefix} boulder_spawner rollDamage must be >= 0"),
            )?;
            optional_non_negative_number(
                entity,
                "fallDamage",
                format!("{prefix} boulder_spawner fallDamage must be >= 0"),
            )?;
            optional_bool(
                entity,
                "instaKillEnemies",
                format!("{prefix} boulder_spawner instaKillEnemies must be a boolean"),
            )?;
            optional_bool(
                entity,
                "pushable",
                format!("{prefix} boulder_spawner pushable must be a boolean"),
            )?;
            optional_gate_mode(
                entity,
                format!("{prefix} boulder_spawner gateMode must be 'or', 'and', or 'xor'"),
            )?;
        }
        _ => {}
    }

    Ok(())
}

fn grid_chars_of(grid: &[Vec<char>]) -> usize {
    grid.first().map_or(0, Vec::len)
}

fn require_string_grid(layer: &Map<String, Value>, prefix: &str) -> Result<Vec<Vec<char>>, String> {
    let valid = matches!(layer.get("grid"), Some(Value::Array(rows))
        if !rows.is_empty() && rows.iter().all(Value::is_string));
    if !valid {
        return Err(format!(
            "{prefix}: \"grid\" must be a non-empty array of strings"
        ));
    }
    let Some(Value::Array(rows)) = layer.get("grid") else {
        unreachable!("checked above");
    };
    let grid_chars: Vec<Vec<char>> = rows
        .iter()
        .map(|row| row.as_str().unwrap_or("").chars().collect())
        .collect();
    let row_len = grid_chars_of(&grid_chars);
    if !grid_chars.iter().all(|row| row.len() == row_len) {
        return Err(format!("{prefix}: all grid rows must be the same length"));
    }
    Ok(grid_chars)
}

fn validate_areas(
    areas_value: &Value,
    grid_chars: &[Vec<char>],
    error_prefix: &str,
    texture_source: &str,
) -> Result<(), String> {
    let Value::Array(areas) = areas_value else {
        return Err(format!("{error_prefix}: \"areas\" must be an array"));
    };
    let row_len = grid_chars_of(grid_chars);
    let row_count = grid_chars.len();
    for (index, area) in areas.iter().enumerate() {
        let Value::Object(entry) = area else {
            return Err(format!("{error_prefix}: areas[{index}] must be an object"));
        };
        if !is_number(entry, "fromCol")
            || !is_number(entry, "toCol")
            || !is_number(entry, "fromRow")
            || !is_number(entry, "toRow")
        {
            return Err(format!(
                "{error_prefix}: areas[{index}] must have numeric fromCol, toCol, fromRow, toRow"
            ));
        }
        let from_col = as_f64(entry, "fromCol").unwrap_or(0.0);
        let to_col = as_f64(entry, "toCol").unwrap_or(0.0);
        let from_row = as_f64(entry, "fromRow").unwrap_or(0.0);
        let to_row = as_f64(entry, "toRow").unwrap_or(0.0);
        if from_col > to_col || from_row > to_row {
            return Err(format!(
                "{error_prefix}: areas[{index}] has fromCol > toCol or fromRow > toRow"
            ));
        }
        #[allow(clippy::cast_precision_loss)]
        if from_col < 0.0
            || to_col >= row_len as f64
            || from_row < 0.0
            || to_row >= row_count as f64
        {
            return Err(format!(
                "{error_prefix}: areas[{index}] is out of grid bounds"
            ));
        }
        if let Some(environment) = entry.get("environment")
            && !environment
                .as_str()
                .is_some_and(|e| VALID_ENVIRONMENTS.contains(&e))
        {
            return Err(format!(
                "{error_prefix}: areas[{index}].environment must be one of {}",
                VALID_ENVIRONMENTS.join(", ")
            ));
        }
        if let Some(open_bottom) = entry.get("openBottom")
            && !open_bottom.is_boolean()
        {
            return Err(format!(
                "{error_prefix}: areas[{index}].openBottom must be a boolean"
            ));
        }
        if let Some(open_top) = entry.get("openTop")
            && !open_top.is_boolean()
        {
            return Err(format!(
                "{error_prefix}: areas[{index}].openTop must be a boolean"
            ));
        }
        if entry.get("wallTexture").is_none()
            && entry.get("floorTexture").is_none()
            && entry.get("ceilingTexture").is_none()
            && entry.get("environment").is_none()
            && !is_truthy(entry.get("openBottom"))
            && !is_truthy(entry.get("openTop"))
        {
            return Err(format!(
                "{error_prefix}: areas[{index}] must specify at least one texture, an environment, or a hollow flag"
            ));
        }
        validate_textures(entry, &format!("areas[{index}]"), texture_source)?;
    }
    Ok(())
}

/// Validate entities in place: structural problems are fatal, per-entity
/// problems warn and drop the entity. Returns the surviving entities.
fn validate_entity_list(
    entities: &[Value],
    grid: &GridContext,
    entity_ids: &HashSet<String>,
    structural_prefix: &str,
    entity_source: &str,
    ctx: &ValidationContext,
    warnings: &mut Vec<String>,
) -> Result<Vec<Value>, String> {
    let mut valid_entities = Vec::new();
    for (index, entity) in entities.iter().enumerate() {
        let Value::Object(object) = entity else {
            return Err(format!(
                "{structural_prefix}: entities[{index}] must be an object"
            ));
        };
        if !is_number(object, "col") || !is_number(object, "row") {
            return Err(format!(
                "{structural_prefix}: entities[{index}] must have numeric col and row"
            ));
        }
        if as_string(object, "type").is_none() {
            return Err(format!(
                "{structural_prefix}: entities[{index}] must have a string type"
            ));
        }
        match validate_entity(object, index, grid, entity_ids, entity_source, ctx) {
            Ok(()) => valid_entities.push(entity.clone()),
            Err(error) => warnings.push(format!("{error} — entity skipped")),
        }
    }
    Ok(valid_entities)
}

/// Validate a single layer definition. charDefs are level-global and passed in
/// as `known_chars`/`walkable_chars`. Entities must already be migrated and
/// their IDs collected into `global_entity_ids`.
fn validate_layer_def(
    layer: &mut Map<String, Value>,
    layer_index: usize,
    source: &str,
    global_entity_ids: &HashSet<String>,
    chars: &LevelCharSets,
    ctx: &ValidationContext,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let prefix = format!("Level {source} layers[{layer_index}]");

    let grid_chars = require_string_grid(layer, &prefix)?;

    for row in &grid_chars {
        for &character in row {
            if !chars.known.contains(&character) {
                return Err(format!("{prefix}: unknown cell character '{character}'"));
            }
        }
    }

    let Some(Value::Array(entities)) = layer.get("entities") else {
        return Err(format!("{prefix}: \"entities\" must be an array"));
    };
    let valid_entities = validate_entity_list(
        entities,
        &GridContext {
            grid_chars: &grid_chars,
            walkable_chars: &chars.walkable,
        },
        global_entity_ids,
        &prefix,
        &format!("{source} layers[{layer_index}]"),
        ctx,
        warnings,
    )?;
    layer.insert("entities".to_string(), Value::Array(valid_entities));

    if let Some(defaults) = layer.get("defaults") {
        let Value::Object(defaults) = defaults else {
            return Err(format!("{prefix}: \"defaults\" must be an object"));
        };
        validate_textures(
            defaults,
            "defaults",
            &format!("{source} layers[{layer_index}]"),
        )?;
    }

    if let Some(areas) = layer.get("areas") {
        validate_areas(
            areas,
            &grid_chars,
            &prefix,
            &format!("{source} layers[{layer_index}]"),
        )?;
    }

    if let Some(ceiling) = layer.get("ceiling")
        && !ceiling.is_boolean()
    {
        return Err(format!("{prefix}: \"ceiling\" must be a boolean"));
    }

    if let Some(y_offset) = layer.get("yOffset")
        && !y_offset.is_number()
    {
        return Err(format!("{prefix}: \"yOffset\" must be a number"));
    }

    Ok(())
}

fn validate_char_defs(
    obj: &Map<String, Value>,
    source: &str,
    walkable_chars: &mut HashSet<char>,
) -> Result<HashSet<char>, String> {
    let mut char_def_chars: HashSet<char> = HashSet::new();
    let Some(char_defs_value) = obj.get("charDefs") else {
        return Ok(char_def_chars);
    };
    let Value::Array(char_defs) = char_defs_value else {
        return Err(format!("Level {source}: \"charDefs\" must be an array"));
    };

    for (index, entry) in char_defs.iter().enumerate() {
        let Value::Object(def) = entry else {
            return Err(format!(
                "Level {source}: charDefs[{index}] must be an object"
            ));
        };

        let single_char = as_string(def, "char").and_then(|text| {
            let mut units = text.encode_utf16();
            let first = units.next();
            if units.next().is_none() {
                first.and_then(|unit| char::decode_utf16([unit]).next()?.ok())
            } else {
                None
            }
        });
        let Some(character) = single_char else {
            return Err(format!(
                "Level {source}: charDefs[{index}].char must be a single character"
            ));
        };
        if BUILTIN_CHARS.contains(&character) {
            return Err(format!(
                "Level {source}: charDefs[{index}].char '{character}' conflicts with built-in character"
            ));
        }
        if char_def_chars.contains(&character) {
            return Err(format!(
                "Level {source}: charDefs[{index}].char '{character}' is a duplicate"
            ));
        }
        char_def_chars.insert(character);

        let Some(solid) = def.get("solid").and_then(Value::as_bool) else {
            return Err(format!(
                "Level {source}: charDefs[{index}].solid must be a boolean"
            ));
        };
        if !solid {
            walkable_chars.insert(character);
        }

        if let Some(see_through) = def.get("seeThrough") {
            let Some(see_through) = see_through.as_bool() else {
                return Err(format!(
                    "Level {source}: charDefs[{index}].seeThrough must be a boolean"
                ));
            };
            if see_through && !solid {
                return Err(format!(
                    "Level {source}: charDefs[{index}].seeThrough requires solid to be true"
                ));
            }
        }

        validate_textures(def, &format!("charDefs[{index}]"), source)?;
    }

    Ok(char_def_chars)
}

/// Validate raw level JSON and decode it into the typed model.
/// Warnings (skipped entities, non-walkable starts) are pushed to `warnings`.
pub fn validate_level(
    data: Value,
    source: &str,
    ctx: &ValidationContext,
    warnings: &mut Vec<String>,
) -> Result<DungeonLevel, String> {
    let Value::Object(mut obj) = data else {
        return Err(format!("Level data from {source} is not an object"));
    };

    if as_string(&obj, "name").is_none() {
        return Err(format!("Level {source}: \"name\" must be a string"));
    }

    // charDefs first (level-global) so custom chars are known before grid checks.
    let mut walkable_chars = walkable_cells();
    let char_def_chars = validate_char_defs(&obj, source, &mut walkable_chars)?;
    let mut known_chars: HashSet<char> = BUILTIN_CHARS.into_iter().collect();
    known_chars.extend(char_def_chars);
    let char_sets = LevelCharSets {
        known: known_chars,
        walkable: walkable_chars,
    };

    let layers_valid =
        matches!(obj.get("layers"), Some(Value::Array(layers)) if !layers.is_empty());
    if !layers_valid {
        return Err(format!(
            "Level {source}: \"layers\" must be a non-empty array"
        ));
    }
    let Some(Value::Array(mut layers)) = obj.remove("layers") else {
        unreachable!("checked above");
    };

    // First pass: migrate entities and collect all entity IDs across all layers
    // so per-layer validation can resolve cross-layer target IDs.
    let mut global_entity_ids: HashSet<String> = HashSet::new();
    for (layer_index, layer) in layers.iter_mut().enumerate() {
        let Value::Object(layer_object) = layer else {
            return Err(format!(
                "Level {source}: layers[{layer_index}] must be an object"
            ));
        };
        let Some(Value::Array(entities)) = layer_object.get_mut("entities") else {
            return Err(format!(
                "Level {source} layers[{layer_index}]: \"entities\" must be an array"
            ));
        };
        migrate_entities(entities);
        for entity in entities.iter() {
            if let Some(id) = entity.get("id").and_then(Value::as_str)
                && !id.is_empty()
            {
                if global_entity_ids.contains(id) {
                    return Err(format!(
                        "Level {source} layers[{layer_index}]: duplicate entity id \"{id}\""
                    ));
                }
                global_entity_ids.insert(id.to_string());
            }
        }
    }

    for (layer_index, layer) in layers.iter_mut().enumerate() {
        let Value::Object(layer_object) = layer else {
            unreachable!("checked in first pass");
        };
        validate_layer_def(
            layer_object,
            layer_index,
            source,
            &global_entity_ids,
            &char_sets,
            ctx,
            warnings,
        )?;
    }

    // Top-level grid/entities mirror layer 0 for convenience access.
    let layer_zero = layers[0].as_object().expect("layer 0 validated as object");
    let grid_value = layer_zero.get("grid").expect("grid validated").clone();
    let mut top_entities: Vec<Value> = layer_zero
        .get("entities")
        .and_then(Value::as_array)
        .expect("entities validated")
        .clone();
    let grid_chars: Vec<Vec<char>> = grid_value
        .as_array()
        .expect("grid validated as array")
        .iter()
        .map(|row| row.as_str().unwrap_or("").chars().collect())
        .collect();
    let row_len = grid_chars_of(&grid_chars);

    // playerStart (optional — single-level mode; dungeon files validate their own).
    if let Some(player_start) = obj.get("playerStart") {
        let Value::Object(start) = player_start else {
            return Err(format!("Level {source}: \"playerStart\" must be an object"));
        };
        if !is_number(start, "col") || !is_number(start, "row") {
            return Err(format!(
                "Level {source}: \"playerStart\" must have numeric col and row"
            ));
        }
        if !as_string(start, "facing").is_some_and(|f| VALID_FACINGS.contains(&f)) {
            return Err(format!(
                "Level {source}: \"playerStart.facing\" must be one of {}",
                VALID_FACINGS.join(", ")
            ));
        }
        let coords = integer_coords(start);
        let in_bounds = coords.is_some_and(|(col, row)| {
            row >= 0 && (row as usize) < grid_chars.len() && col >= 0 && (col as usize) < row_len
        });
        if !in_bounds {
            return Err(format!(
                "Level {source}: playerStart ({},{}) is out of grid bounds",
                display_prop(start, "col"),
                display_prop(start, "row")
            ));
        }
        let (col, row) = coords.expect("bounds checked");
        let on_walkable =
            cell_of(&grid_chars, col, row).is_some_and(|cell| char_sets.walkable.contains(&cell));
        if !on_walkable {
            warnings.push(format!(
                "Level {source}: playerStart ({col},{row}) is not a walkable tile"
            ));
        }
    }

    // Re-run migration and validation over the top-level entity list, exactly
    // as the TS loader does (IDs are re-collected from layer 0 only).
    migrate_entities(&mut top_entities);

    let mut entity_ids: HashSet<String> = HashSet::new();
    for entity in &top_entities {
        if let Some(id) = entity.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            if entity_ids.contains(id) {
                return Err(format!("Level {source}: duplicate entity id \"{id}\""));
            }
            entity_ids.insert(id.to_string());
        }
    }

    let structural_prefix = format!("Level {source}");
    let valid_entities = validate_entity_list(
        &top_entities,
        &GridContext {
            grid_chars: &grid_chars,
            walkable_chars: &char_sets.walkable,
        },
        &entity_ids,
        &structural_prefix,
        source,
        ctx,
        warnings,
    )?;

    if let Some(environment) = obj.get("environment")
        && !environment
            .as_str()
            .is_some_and(|e| VALID_ENVIRONMENTS.contains(&e))
    {
        return Err(format!(
            "Level {source}: \"environment\" must be one of {}",
            VALID_ENVIRONMENTS.join(", ")
        ));
    }

    if let Some(skybox) = obj.get("skybox") {
        if !skybox
            .as_str()
            .is_some_and(|s| ["starry-night", "daylight", "sunset"].contains(&s))
        {
            return Err(format!(
                "Level {source}: \"skybox\" must be one of starry-night, daylight, sunset"
            ));
        }
        if obj.get("ceiling") != Some(&Value::Bool(false)) {
            warnings.push(format!(
                "Level {source}: \"skybox\" is set but \"ceiling\" is not false — skybox won't be visible"
            ));
        }
    }

    if let Some(defaults) = obj.get("defaults") {
        let Value::Object(defaults) = defaults else {
            return Err(format!("Level {source}: \"defaults\" must be an object"));
        };
        validate_textures(defaults, "defaults", source)?;
    }

    if let Some(areas) = obj.get("areas") {
        validate_areas(areas, &grid_chars, &format!("Level {source}"), source)?;
    }

    obj.insert("layers".to_string(), Value::Array(layers));
    obj.insert("grid".to_string(), grid_value);
    obj.insert("entities".to_string(), Value::Array(valid_entities));

    serde_json::from_value(Value::Object(obj))
        .map_err(|error| format!("Level {source}: failed to decode validated level: {error}"))
}

fn walkable_chars_for(level: &DungeonLevel) -> HashSet<char> {
    let mut walkable = walkable_cells();
    if let Some(char_defs) = &level.char_defs {
        for def in char_defs {
            if !def.solid {
                walkable.insert(def.character);
            }
        }
    }
    walkable
}

fn grid_cell(grid: &[String], col: i64, row: i64) -> Option<char> {
    let row_text = grid.get(usize::try_from(row).ok()?)?;
    row_text.chars().nth(usize::try_from(col).ok()?)
}

/// Validate raw dungeon JSON and decode it into the typed model.
pub fn validate_dungeon(
    data: Value,
    source: &str,
    ctx: &ValidationContext,
    warnings: &mut Vec<String>,
) -> Result<Dungeon, String> {
    let Value::Object(obj) = data else {
        return Err(format!("Dungeon data from {source} is not an object"));
    };

    let Some(name) = as_string(&obj, "name") else {
        return Err(format!("Dungeon {source}: \"name\" must be a string"));
    };
    let name = name.to_string();

    let levels_valid =
        matches!(obj.get("levels"), Some(Value::Array(levels)) if !levels.is_empty());
    if !levels_valid {
        return Err(format!(
            "Dungeon {source}: \"levels\" must be a non-empty array"
        ));
    }
    let Some(Value::Array(level_values)) = obj.get("levels") else {
        unreachable!("checked above");
    };

    let mut level_ids: HashSet<String> = HashSet::new();
    let mut levels: Vec<DungeonLevel> = Vec::new();
    for (index, level_value) in level_values.iter().enumerate() {
        let level = validate_level(
            level_value.clone(),
            &format!("{source} levels[{index}]"),
            ctx,
            warnings,
        )?;
        let Some(level_id) = level.id.as_deref().filter(|id| !id.is_empty()) else {
            return Err(format!(
                "Dungeon {source}: levels[{index}] must have a non-empty string \"id\""
            ));
        };
        if level_ids.contains(level_id) {
            return Err(format!(
                "Dungeon {source}: duplicate level id \"{level_id}\""
            ));
        }
        level_ids.insert(level_id.to_string());
        levels.push(level);
    }

    let dungeon_player_start = if let Some(player_start) = obj.get("playerStart") {
        let Value::Object(start) = player_start else {
            return Err(format!(
                "Dungeon {source}: \"playerStart\" must be an object"
            ));
        };
        let Some(level_id) = as_string(start, "levelId").filter(|id| !id.is_empty()) else {
            return Err(format!(
                "Dungeon {source}: \"playerStart.levelId\" must be a non-empty string"
            ));
        };
        if !level_ids.contains(level_id) {
            return Err(format!(
                "Dungeon {source}: \"playerStart.levelId\" \"{level_id}\" does not match any level id"
            ));
        }
        if !is_number(start, "col") || !is_number(start, "row") {
            return Err(format!(
                "Dungeon {source}: \"playerStart\" must have numeric col and row"
            ));
        }
        let Some(facing_text) = as_string(start, "facing").filter(|f| VALID_FACINGS.contains(f))
        else {
            return Err(format!(
                "Dungeon {source}: \"playerStart.facing\" must be one of {}",
                VALID_FACINGS.join(", ")
            ));
        };
        let facing = serde_json::from_value(Value::String(facing_text.to_string()))
            .expect("facing validated");

        let start_level = levels
            .iter()
            .find(|level| level.id.as_deref() == Some(level_id))
            .expect("levelId validated against level_ids");
        let start_layer_coord = start
            .get("layerIndex")
            .and_then(Value::as_i64)
            .and_then(|coord| i32::try_from(coord).ok())
            .unwrap_or(0);
        let start_layer_index = resolve_layer_coord(start_level, start_layer_coord);
        let start_grid = &start_level.layers[start_layer_index].grid;
        let start_row_len = start_grid.first().map_or(0, |row| row.chars().count());

        let coords = integer_coords(start);
        let in_bounds = coords.is_some_and(|(col, row)| {
            row >= 0
                && (row as usize) < start_grid.len()
                && col >= 0
                && (col as usize) < start_row_len
        });
        if !in_bounds {
            return Err(format!(
                "Dungeon {source}: playerStart ({},{}) is out of grid bounds on level \"{level_id}\" layer {start_layer_coord}",
                display_prop(start, "col"),
                display_prop(start, "row")
            ));
        }
        let (col, row) = coords.expect("bounds checked");

        let start_walkable = walkable_chars_for(start_level);
        let on_walkable =
            grid_cell(start_grid, col, row).is_some_and(|cell| start_walkable.contains(&cell));
        if !on_walkable {
            warnings.push(format!(
                "Dungeon {source}: playerStart ({col},{row}) is on a non-walkable tile on level \"{level_id}\" layer {start_layer_coord}"
            ));
        }

        DungeonPlayerStart {
            level_id: level_id.to_string(),
            col: i32::try_from(col)
                .map_err(|_| format!("Dungeon {source}: playerStart col out of supported range"))?,
            row: i32::try_from(row)
                .map_err(|_| format!("Dungeon {source}: playerStart row out of supported range"))?,
            facing,
            layer_index: (start_layer_coord != 0).then_some(start_layer_coord),
        }
    } else {
        // Migration: promote the first level's playerStart to the dungeon level.
        let migrate_level = levels
            .iter()
            .find(|level| level.player_start.is_some())
            .ok_or_else(|| {
                format!("Dungeon {source}: \"playerStart\" is required on the dungeon object")
            })?;
        let start = migrate_level
            .player_start
            .expect("found by player_start.is_some()");
        DungeonPlayerStart {
            level_id: migrate_level.id.clone().unwrap_or_default(),
            col: start.col,
            row: start.row,
            facing: start.facing,
            layer_index: None,
        }
    };

    // Stair validation: target must resolve to a stairs entity anywhere in the
    // dungeon, and the spawn cell one step from the target must be walkable.
    for (level_index, level) in levels.iter().enumerate() {
        for entity in get_all_level_entities(level) {
            if entity.entity_type != "stairs" {
                continue;
            }
            let stair_id = entity.id.as_deref().unwrap_or("undefined");
            let Some(target_id) = entity.prop_str("target") else {
                continue;
            };

            let mut target: Option<(&DungeonLevel, &Entity, usize)> = None;
            for other_level in &levels {
                if let Some(target_stair) = get_all_level_entities(other_level)
                    .find(|other| other.id.as_deref() == Some(target_id))
                {
                    let layer_index = find_entity_layer_index(other_level, target_id);
                    target = Some((other_level, target_stair, layer_index));
                    break;
                }
            }

            let Some((target_level, target_stair, target_layer_index)) = target else {
                warnings.push(format!(
                    "Dungeon {source}: levels[{level_index}] stairs \"{stair_id}\" target \"{target_id}\" does not match any stair entity in the dungeon — stair will not function"
                ));
                continue;
            };
            if target_stair.entity_type != "stairs" {
                warnings.push(format!(
                    "Dungeon {source}: levels[{level_index}] stairs \"{stair_id}\" target \"{target_id}\" is not a stairs entity — stair will not function"
                ));
                continue;
            }

            let target_grid = &target_level.layers[target_layer_index].grid;
            let target_level_id = target_level.id.as_deref().unwrap_or("undefined");
            let (dcol, drow) = match target_stair.prop_str("facing") {
                Some("N") => (0, -1),
                Some("S") => (0, 1),
                Some("E") => (1, 0),
                Some("W") => (-1, 0),
                _ => (0, 0),
            };
            let spawn_col = target_stair.col + dcol;
            let spawn_row = target_stair.row + drow;

            let target_row_len = target_grid.first().map_or(0, |row| row.chars().count());
            let spawn_in_bounds = spawn_row >= 0
                && (spawn_row as usize) < target_grid.len()
                && spawn_col >= 0
                && (spawn_col as usize) < target_row_len;
            if !spawn_in_bounds {
                warnings.push(format!(
                    "Dungeon {source}: levels[{level_index}] stairs \"{stair_id}\" spawn position ({spawn_col},{spawn_row}) is out of bounds on level \"{target_level_id}\" — stair will not function"
                ));
                continue;
            }

            let target_walkable = walkable_chars_for(target_level);
            let spawn_walkable = grid_cell(target_grid, spawn_col, spawn_row)
                .is_some_and(|cell| target_walkable.contains(&cell));
            if !spawn_walkable {
                warnings.push(format!(
                    "Dungeon {source}: levels[{level_index}] stairs \"{stair_id}\" spawn position ({spawn_col},{spawn_row}) is not walkable on level \"{target_level_id}\" layer {target_layer_index} — stair will not function"
                ));
            }
        }
    }

    Ok(Dungeon {
        name,
        levels,
        player_start: dungeon_player_start,
    })
}

/// Parse and validate a level JSON document.
pub fn validate_level_str(
    json: &str,
    source: &str,
    ctx: &ValidationContext,
    warnings: &mut Vec<String>,
) -> Result<DungeonLevel, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("Failed to parse level JSON from {source}: {error}"))?;
    validate_level(value, source, ctx, warnings)
}

/// Parse and validate a dungeon JSON document.
pub fn validate_dungeon_str(
    json: &str,
    source: &str,
    ctx: &ValidationContext,
    warnings: &mut Vec<String>,
) -> Result<Dungeon, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("Failed to parse dungeon JSON from {source}: {error}"))?;
    validate_dungeon(value, source, ctx, warnings)
}

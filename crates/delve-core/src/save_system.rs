//! Save/load data model, ported from the TS saveSystem.
//!
//! The TS original serializes `Map`/`Set` fields to `Record`/`Array` because
//! `JSON.stringify` drops Maps and Sets. Rust's `HashMap`/`HashSet` already
//! serialize as JSON objects/arrays, so the conversion helpers below exist
//! purely for schema-shape parity with the TS API surface (and its tests),
//! not because Rust needs them to produce valid JSON.
//!
//! `saveToSlot`/`loadFromSlot`/etc. read and write `localStorage` in the TS
//! original. There is no browser here, so those functions are ported as
//! pure functions over a [`SaveStore`] trait; a real implementation (backed
//! by files under `saves/`) arrives with the game shell. `exportSaveFile`/
//! `importSaveFile` are pure browser DOM glue (Blob, anchor click,
//! FileReader) with no reusable logic and are not ported.

use crate::entities::ItemEntity;
use crate::game_state::{
    AltarInstance, BarrelInstance, BlockInstance, BookshelfInstance, BoulderInstance,
    BoulderSpawnerInstance, BreakableWallInstance, ChestInstance, DoorInstance, FountainInstance,
    GameState, GateInstance, KeyInstance, LayerState, LevelSnapshot, LeverInstance,
    MultiLayerSnapshot, NpcInstance, PitTrapInstance, PlateInstance, PlayerStateSnapshot,
    PropInstance, RampInstance, SconceInstance, SecretWallInstance, SignInstance, SpawnerInstance,
    StairInstance, ThinWallInstance, TrapLauncherInstance, TriggerInstance, TripwireInstance,
};
use crate::grid::Facing;
use crate::signal_manager::SignalManagerState;
use crate::types::{Dungeon, EnemyInstance};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

#[must_use]
pub fn map_to_record<T: Clone>(map: &HashMap<String, T>) -> HashMap<String, T> {
    map.clone()
}

#[must_use]
pub fn record_to_map<T: Clone>(record: &HashMap<String, T>) -> HashMap<String, T> {
    record.clone()
}

#[must_use]
pub fn set_to_array(set: &HashSet<String>) -> Vec<String> {
    let mut result: Vec<String> = set.iter().cloned().collect();
    result.sort();
    result
}

#[must_use]
pub fn array_to_set(array: &[String]) -> HashSet<String> {
    array.iter().cloned().collect()
}

// ---------------------------------------------------------------------------
// Serialized forms — flat, JSON-safe mirrors of LayerState/LevelSnapshot.
// ---------------------------------------------------------------------------

/// Entity types added after the initial release fall back to an empty map
/// when absent, so saves from before that entity type existed still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerializedLevelSnapshot {
    pub doors: HashMap<String, DoorInstance>,
    pub keys: HashMap<String, KeyInstance>,
    pub levers: HashMap<String, LeverInstance>,
    pub plates: HashMap<String, PlateInstance>,
    pub triggers: HashMap<String, TriggerInstance>,
    pub tripwires: HashMap<String, TripwireInstance>,
    pub gates: HashMap<String, GateInstance>,
    pub trap_launchers: HashMap<String, TrapLauncherInstance>,
    pub sconces: HashMap<String, SconceInstance>,
    pub stairs: HashMap<String, StairInstance>,
    pub enemies: HashMap<String, EnemyInstance>,
    pub breakable_walls: HashMap<String, BreakableWallInstance>,
    pub secret_walls: HashMap<String, SecretWallInstance>,
    pub blocks: HashMap<String, BlockInstance>,
    pub chests: HashMap<String, ChestInstance>,
    pub signs: HashMap<String, SignInstance>,
    #[serde(default)]
    pub npcs: HashMap<String, NpcInstance>,
    #[serde(default)]
    pub fountains: HashMap<String, FountainInstance>,
    #[serde(default)]
    pub bookshelves: HashMap<String, BookshelfInstance>,
    #[serde(default)]
    pub altars: HashMap<String, AltarInstance>,
    #[serde(default)]
    pub barrels: HashMap<String, BarrelInstance>,
    #[serde(default)]
    pub thin_walls: HashMap<String, ThinWallInstance>,
    #[serde(default)]
    pub ramps: HashMap<String, RampInstance>,
    #[serde(default)]
    pub props: HashMap<String, PropInstance>,
    #[serde(default)]
    pub pit_traps: HashMap<String, PitTrapInstance>,
    #[serde(default)]
    pub spawners: HashMap<String, SpawnerInstance>,
    #[serde(default)]
    pub boulders: HashMap<String, BoulderInstance>,
    #[serde(default)]
    pub boulder_spawners: HashMap<String, BoulderSpawnerInstance>,
    pub destroyed_walls: Vec<String>,
    pub explored_cells: Vec<String>,
    pub registry_snapshot: Vec<ItemEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_state: Option<SignalManagerState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerializedMultiLayerSnapshot {
    pub layers: Vec<SerializedLevelSnapshot>,
    pub active_layer_index: usize,
}

/// Older saves stored a single flat [`SerializedLevelSnapshot`] per level
/// instead of the multi-layer wrapper; `deserialize_multi_layer_snapshot`
/// upgrades either shape to a one-layer [`MultiLayerSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SerializedSnapshotEntry {
    MultiLayer(SerializedMultiLayerSnapshot),
    Flat(Box<SerializedLevelSnapshot>),
}

#[must_use]
pub fn serialize_level_snapshot(snapshot: &LevelSnapshot) -> SerializedLevelSnapshot {
    let layer = &snapshot.layer;
    SerializedLevelSnapshot {
        doors: map_to_record(&layer.doors),
        keys: map_to_record(&layer.keys),
        levers: map_to_record(&layer.levers),
        plates: map_to_record(&layer.plates),
        triggers: map_to_record(&layer.triggers),
        tripwires: map_to_record(&layer.tripwires),
        gates: map_to_record(&layer.gates),
        trap_launchers: map_to_record(&layer.trap_launchers),
        sconces: map_to_record(&layer.sconces),
        stairs: map_to_record(&layer.stairs),
        enemies: map_to_record(&layer.enemies),
        breakable_walls: map_to_record(&layer.breakable_walls),
        secret_walls: map_to_record(&layer.secret_walls),
        blocks: map_to_record(&layer.blocks),
        chests: map_to_record(&layer.chests),
        signs: map_to_record(&layer.signs),
        npcs: map_to_record(&layer.npcs),
        fountains: map_to_record(&layer.fountains),
        bookshelves: map_to_record(&layer.bookshelves),
        altars: map_to_record(&layer.altars),
        barrels: map_to_record(&layer.barrels),
        thin_walls: map_to_record(&layer.thin_walls),
        ramps: map_to_record(&layer.ramps),
        props: map_to_record(&layer.props),
        pit_traps: map_to_record(&layer.pit_traps),
        spawners: map_to_record(&layer.spawners),
        boulders: map_to_record(&layer.boulders),
        boulder_spawners: map_to_record(&layer.boulder_spawners),
        destroyed_walls: set_to_array(&layer.destroyed_walls),
        explored_cells: set_to_array(&layer.explored_cells),
        registry_snapshot: snapshot.registry_snapshot.clone(),
        signal_state: snapshot.signal_state.clone(),
    }
}

#[must_use]
pub fn deserialize_level_snapshot(data: &SerializedLevelSnapshot) -> LevelSnapshot {
    LevelSnapshot {
        layer: LayerState {
            doors: record_to_map(&data.doors),
            keys: record_to_map(&data.keys),
            levers: record_to_map(&data.levers),
            plates: record_to_map(&data.plates),
            triggers: record_to_map(&data.triggers),
            tripwires: record_to_map(&data.tripwires),
            gates: record_to_map(&data.gates),
            trap_launchers: record_to_map(&data.trap_launchers),
            sconces: record_to_map(&data.sconces),
            stairs: record_to_map(&data.stairs),
            enemies: record_to_map(&data.enemies),
            breakable_walls: record_to_map(&data.breakable_walls),
            secret_walls: record_to_map(&data.secret_walls),
            blocks: record_to_map(&data.blocks),
            chests: record_to_map(&data.chests),
            signs: record_to_map(&data.signs),
            npcs: record_to_map(&data.npcs),
            fountains: record_to_map(&data.fountains),
            bookshelves: record_to_map(&data.bookshelves),
            altars: record_to_map(&data.altars),
            barrels: record_to_map(&data.barrels),
            thin_walls: record_to_map(&data.thin_walls),
            ramps: record_to_map(&data.ramps),
            props: record_to_map(&data.props),
            pit_traps: record_to_map(&data.pit_traps),
            spawners: record_to_map(&data.spawners),
            boulders: record_to_map(&data.boulders),
            boulder_spawners: record_to_map(&data.boulder_spawners),
            destroyed_walls: array_to_set(&data.destroyed_walls),
            explored_cells: array_to_set(&data.explored_cells),
        },
        registry_snapshot: data.registry_snapshot.clone(),
        signal_state: data.signal_state.clone(),
    }
}

#[must_use]
pub fn serialize_multi_layer_snapshot(
    snapshot: &MultiLayerSnapshot,
) -> SerializedMultiLayerSnapshot {
    SerializedMultiLayerSnapshot {
        layers: snapshot
            .layers
            .iter()
            .map(serialize_level_snapshot)
            .collect(),
        active_layer_index: snapshot.active_layer_index,
    }
}

#[must_use]
pub fn deserialize_multi_layer_snapshot(entry: &SerializedSnapshotEntry) -> MultiLayerSnapshot {
    match entry {
        SerializedSnapshotEntry::MultiLayer(multi) => MultiLayerSnapshot {
            layers: multi
                .layers
                .iter()
                .map(deserialize_level_snapshot)
                .collect(),
            active_layer_index: multi.active_layer_index,
        },
        SerializedSnapshotEntry::Flat(flat) => MultiLayerSnapshot {
            layers: vec![deserialize_level_snapshot(flat)],
            active_layer_index: 0,
        },
    }
}

// ---------------------------------------------------------------------------
// Save file schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPlayer {
    pub col: i64,
    pub row: i64,
    pub facing: Facing,
    #[serde(flatten)]
    pub stats: PlayerStateSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestSaveState {
    pub status: String,
    pub stage_index: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveData {
    pub version: u32,
    pub timestamp: i64,
    pub dungeon_name: String,
    pub current_level_id: String,
    pub player: SavedPlayer,
    pub keys: Vec<String>,
    pub entity_registry: Vec<ItemEntity>,
    #[serde(default)]
    pub flags: Vec<String>,
    pub level_snapshots: HashMap<String, SerializedSnapshotEntry>,
    pub level_grids: HashMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quests: Option<HashMap<String, QuestSaveState>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotMetadata {
    pub saved_at: i64,
    pub player_name: String,
    pub level_id: String,
    pub character_level: i64,
    pub dungeon_name: String,
}

// ---------------------------------------------------------------------------
// Save/load assembly
// ---------------------------------------------------------------------------

/// `timestamp` is injected rather than read from the system clock, the same
/// way `Math.random` becomes an injected closure elsewhere in this port —
/// it keeps `delve-core` free of wall-clock reads and the function testable.
pub struct BuildSaveDataParams<'a> {
    pub game_state: &'a GameState,
    pub player_col: i64,
    pub player_row: i64,
    pub player_facing: Facing,
    pub current_level_id: String,
    pub level_snapshots: &'a HashMap<String, MultiLayerSnapshot>,
    pub dungeon: &'a Dungeon,
    pub timestamp: i64,
    /// Serializable quest state; `None` until a runtime quest manager exists.
    pub quests: Option<HashMap<String, QuestSaveState>>,
}

#[must_use]
pub fn build_save_data(params: BuildSaveDataParams) -> SaveData {
    let BuildSaveDataParams {
        game_state,
        player_col,
        player_row,
        player_facing,
        current_level_id,
        level_snapshots,
        dungeon,
        timestamp,
        quests,
    } = params;

    // Flush the currently-active level into a snapshot so it's included.
    let active_snapshot = game_state.save_level_state();

    // Merge all known snapshots: previously-visited levels + the active
    // level. The active level's snapshot wins (it's freshest).
    let mut all_snapshots = level_snapshots.clone();
    all_snapshots.insert(current_level_id.clone(), active_snapshot);

    // Capture the full registry AFTER save_level_state so that the active
    // level's ground items are reflected (save_level_state updates the
    // per-level registry snapshot, but entity_registry is authoritative).
    let full_registry = game_state.entity_registry.snapshot();

    let level_snapshots = all_snapshots
        .iter()
        .map(|(id, snapshot)| {
            (
                id.clone(),
                SerializedSnapshotEntry::MultiLayer(serialize_multi_layer_snapshot(snapshot)),
            )
        })
        .collect();

    // Capture each level's current grid (may have been mutated by breakable
    // walls, etc).
    let mut level_grids = HashMap::new();
    for level in &dungeon.levels {
        let id = level.id.clone().unwrap_or_else(|| level.name.clone());
        level_grids.insert(id, level.grid.clone());
    }

    SaveData {
        version: 1,
        timestamp,
        dungeon_name: dungeon.name.clone(),
        current_level_id: current_level_id.clone(),
        player: SavedPlayer {
            col: player_col,
            row: player_row,
            facing: player_facing,
            stats: game_state.get_player_state(),
        },
        keys: game_state.player.picked_up_keys(),
        entity_registry: full_registry,
        flags: game_state.player.flags.iter().cloned().collect(),
        level_snapshots,
        level_grids,
        quests,
    }
}

pub struct ApplySaveDataResult {
    pub target_level_id: String,
    pub level_snapshots: HashMap<String, MultiLayerSnapshot>,
    pub player_col: i64,
    pub player_row: i64,
    pub player_facing: Facing,
}

/// Restores the given save into `game_state`/`dungeon`. Quest state
/// (`data.quests`) is not applied here — no runtime quest manager exists
/// yet in `delve-core`; callers can read it back off `data` once one does.
pub fn apply_save_data(
    data: &SaveData,
    game_state: &mut GameState,
    dungeon: &mut Dungeon,
) -> ApplySaveDataResult {
    // Restore mutated grids onto dungeon levels.
    for level in &mut dungeon.levels {
        let id = level.id.clone().unwrap_or_else(|| level.name.clone());
        if let Some(grid) = data.level_grids.get(&id) {
            level.grid = grid.clone();
        }
    }

    // Deserialize all level snapshots except the active one — that goes
    // through load_level_state below, which also re-initializes the signal
    // manager.
    let mut level_snapshots = HashMap::new();
    for (id, serialized) in &data.level_snapshots {
        if id != &data.current_level_id {
            level_snapshots.insert(id.clone(), deserialize_multi_layer_snapshot(serialized));
        }
    }

    // Restore the active level via the GameState API so the signal
    // machinery, entity index, and internal maps are all rebuilt
    // consistently.
    if let Some(active_serialized) = data.level_snapshots.get(&data.current_level_id) {
        let active_snapshot = deserialize_multi_layer_snapshot(active_serialized);
        game_state.current_level_id = data.current_level_id.clone();
        game_state.load_level_state(&active_snapshot);
    }

    // Restore the full entity registry AFTER load_level_state.
    // load_level_state restores entity_registry from the level's
    // registry_snapshot, which only covers ground items for that level. We
    // need backpack and equipped items too, so we overwrite with the full
    // save.
    game_state
        .entity_registry
        .restore(data.entity_registry.clone());

    game_state.restore_player_state(&data.player.stats);
    game_state.player.restore_picked_up_keys(&data.keys);

    game_state.player.flags.clear();
    game_state.player.flags.extend(data.flags.iter().cloned());

    ApplySaveDataResult {
        target_level_id: data.current_level_id.clone(),
        level_snapshots,
        player_col: data.player.col,
        player_row: data.player.row,
        player_facing: data.player.facing,
    }
}

// ---------------------------------------------------------------------------
// Slot management
// ---------------------------------------------------------------------------

pub const SAVE_SLOT_KEYS: [&str; 5] = [
    "delveward_save_1",
    "delveward_save_2",
    "delveward_save_3",
    "delveward_save_4",
    "delveward_save_5",
];

pub const AUTOSAVE_KEY: &str = "delveward_autosave";

/// Abstracts over the key-value store backing save slots — `localStorage`
/// in the TS original, files under `saves/` in the game shell. `set_item`
/// returns `Result` so a backing store can report write failures (a full
/// disk, a quota-exceeded browser store) without `delve-core` knowing what
/// kind of store it is.
pub trait SaveStore {
    fn get_item(&self, key: &str) -> Option<String>;
    fn set_item(&mut self, key: &str, value: String) -> Result<(), String>;
    fn remove_item(&mut self, key: &str);
}

pub fn save_to_slot<S: SaveStore>(store: &mut S, key: &str, data: &SaveData) -> bool {
    let Ok(json) = serde_json::to_string(data) else {
        return false;
    };
    store.set_item(key, json).is_ok()
}

#[must_use]
pub fn load_from_slot<S: SaveStore>(store: &S, key: &str) -> Option<SaveData> {
    let raw = store.get_item(key)?;
    let parsed: SaveData = serde_json::from_str(&raw).ok()?;
    if parsed.version != 1 {
        return None;
    }
    Some(parsed)
}

pub fn delete_slot<S: SaveStore>(store: &mut S, key: &str) {
    store.remove_item(key);
}

#[must_use]
pub fn get_slot_metadata<S: SaveStore>(store: &S, key: &str) -> Option<SlotMetadata> {
    let raw = store.get_item(key)?;
    let parsed: SaveData = serde_json::from_str(&raw).ok()?;
    if parsed.version != 1 {
        return None;
    }
    Some(SlotMetadata {
        saved_at: parsed.timestamp,
        player_name: parsed.player.stats.player_name,
        level_id: parsed.current_level_id,
        character_level: parsed.player.stats.level,
        dungeon_name: parsed.dungeon_name,
    })
}

#[must_use]
pub fn get_all_slot_metadata<S: SaveStore>(store: &S) -> HashMap<String, Option<SlotMetadata>> {
    SAVE_SLOT_KEYS
        .into_iter()
        .chain(std::iter::once(AUTOSAVE_KEY))
        .map(|key| (key.to_string(), get_slot_metadata(store, key)))
        .collect()
}

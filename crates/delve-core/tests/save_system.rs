//! Ported from `src/core/saveSystem.test.ts`.
//!
//! The TS suite mocks `localStorage` with a plain `{getItem, setItem,
//! removeItem}` object backed by a `Record<string, string>`; `MockStore`
//! below is the same shape implementing [`SaveStore`].

use delve_core::entities::{ItemEntity, ItemLocation};
use delve_core::game_state::{
    DoorInstance, DoorState, GameState, GameStateDeps, KeyInstance, LayerState, LevelSnapshot,
    LeverInstance, LeverState,
};
use delve_core::grid::Facing;
use delve_core::items::ItemQuality;
use delve_core::save_system::{
    AUTOSAVE_KEY, SAVE_SLOT_KEYS, SaveData, SaveStore, SavedPlayer, SerializedLevelSnapshot,
    array_to_set, deserialize_level_snapshot, get_all_slot_metadata, get_slot_metadata,
    load_from_slot, map_to_record, record_to_map, save_to_slot, serialize_level_snapshot,
    set_to_array,
};
use delve_core::signal_manager::{
    GateMode, SignalManagerState, SignalMode, SignalReceiver, SignalSource,
};
use delve_core::status_effects::{StatusEffect, StatusEffectType};
use delve_core::types::{EnemyAiState, EnemyInstance};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn minimal_level_snapshot() -> LevelSnapshot {
    let mut layer = LayerState::default();

    layer.doors.insert(
        "door_1_2".to_string(),
        DoorInstance {
            id: Some("door_1_2".to_string()),
            col: 1,
            row: 2,
            state: DoorState::Closed,
            key_id: Some("key_bronze".to_string()),
            mechanical: false,
            gate_mode: None,
        },
    );
    layer.doors.insert(
        "door_3_4".to_string(),
        DoorInstance {
            id: Some("door_3_4".to_string()),
            col: 3,
            row: 4,
            state: DoorState::Open,
            key_id: None,
            mechanical: true,
            gate_mode: None,
        },
    );
    layer.keys.insert(
        "key_1_5".to_string(),
        KeyInstance {
            id: Some("key_1_5".to_string()),
            col: 1,
            row: 5,
            key_id: "key_bronze".to_string(),
            picked_up: false,
        },
    );
    layer.enemies.insert(
        "enemy_2_3".to_string(),
        EnemyInstance {
            col: 2,
            row: 3,
            enemy_type: "rat".to_string(),
            hp: 8.0,
            max_hp: 10.0,
            atk: 3.0,
            def: 1.0,
            aggro_range: 4.0,
            move_interval: 1.0,
            blocks_movement: true,
            ai_state: EnemyAiState::Idle,
            move_timer: 0.0,
            regen_timer: None,
            regen_pause_timer: None,
            drops: None,
            status_effects: vec![StatusEffect {
                effect_type: StatusEffectType::Poison,
                remaining: 3.0,
                tick_timer: 0.0,
                tick_interval: 1.0,
                tick_damage: 2.0,
            }],
            spawner_id: None,
        },
    );
    layer.destroyed_walls = HashSet::from(["5_6".to_string(), "7_8".to_string()]);
    layer.explored_cells = HashSet::from(["0_0".to_string(), "1_0".to_string(), "0_1".to_string()]);

    LevelSnapshot {
        layer,
        registry_snapshot: vec![ItemEntity {
            instance_id: "item_1".to_string(),
            item_id: "sword_iron".to_string(),
            quality: ItemQuality::Common,
            modifiers: Vec::new(),
            location: ItemLocation::World {
                level_id: "level1".to_string(),
                col: 1,
                row: 1,
                layer_index: None,
            },
        }],
        signal_state: Some(SignalManagerState {
            sources: vec![SignalSource {
                entity_id: "src_lever_0_0".to_string(),
                targets: Vec::new(),
                signal_mode: SignalMode::Toggle,
                active: false,
                fired: false,
                duration: None,
                deactivate_at: 0.0,
                delay: None,
                delay_fire_at: 0.0,
                delay_pending: false,
            }],
            receivers: Vec::new(),
            gates: Vec::new(),
            now: 42.0,
        }),
    }
}

fn complex_level_snapshot() -> LevelSnapshot {
    let mut layer = LayerState::default();

    layer.doors.insert(
        "door_0_1".to_string(),
        DoorInstance {
            id: None,
            col: 0,
            row: 1,
            state: DoorState::Closed,
            key_id: None,
            mechanical: false,
            gate_mode: None,
        },
    );
    layer.levers.insert(
        "lever_2_2".to_string(),
        LeverInstance {
            id: Some("lever_2_2".to_string()),
            col: 2,
            row: 2,
            targets: vec!["door_0_1".to_string(), "door_5_5".to_string()],
            wall: Facing::N,
            state: LeverState::Up,
            signal_mode: Some(SignalMode::Timed),
            signal_duration: Some(1.5),
            signal_delay: None,
        },
    );
    layer.enemies.insert(
        "enemy_4_4".to_string(),
        EnemyInstance {
            col: 4,
            row: 4,
            enemy_type: "skeleton".to_string(),
            hp: 15.0,
            max_hp: 20.0,
            atk: 5.0,
            def: 2.0,
            aggro_range: 5.0,
            move_interval: 1.5,
            blocks_movement: true,
            ai_state: EnemyAiState::Chase,
            move_timer: 0.3,
            regen_timer: None,
            regen_pause_timer: None,
            drops: None,
            status_effects: vec![
                StatusEffect {
                    effect_type: StatusEffectType::Burning,
                    remaining: 5.0,
                    tick_timer: 0.0,
                    tick_interval: 0.5,
                    tick_damage: 3.0,
                },
                StatusEffect {
                    effect_type: StatusEffectType::Slow,
                    remaining: 2.0,
                    tick_timer: 0.0,
                    tick_interval: 0.0,
                    tick_damage: 0.0,
                },
            ],
            spawner_id: None,
        },
    );
    layer.explored_cells = HashSet::from(["0_0".to_string()]);

    LevelSnapshot {
        layer,
        registry_snapshot: Vec::new(),
        signal_state: Some(SignalManagerState {
            sources: vec![SignalSource {
                entity_id: "src_lever_2_2".to_string(),
                targets: vec!["door_0_1".to_string()],
                signal_mode: SignalMode::Timed,
                active: true,
                fired: false,
                duration: Some(1.5),
                deactivate_at: 90.0,
                delay: None,
                delay_fire_at: 0.0,
                delay_pending: false,
            }],
            receivers: vec![SignalReceiver {
                entity_id: "rcv_door_0_1".to_string(),
                gate_mode: GateMode::Or,
                active: true,
            }],
            gates: Vec::new(),
            now: 88.5,
        }),
    }
}

fn minimal_save_data() -> SaveData {
    SaveData {
        version: 1,
        timestamp: 1_700_000_000_000,
        dungeon_name: "Test Dungeon".to_string(),
        current_level_id: "level1".to_string(),
        player: SavedPlayer {
            col: 2,
            row: 3,
            facing: Facing::N,
            stats: delve_core::game_state::PlayerStateSnapshot {
                hp: 25.0,
                max_hp: 30.0,
                str: 10.0,
                dex: 10.0,
                vit: 10.0,
                wis: 10.0,
                xp: 100,
                level: 2,
                attribute_points: 1,
                player_name: "Hero".to_string(),
                gold: 50,
                torch_fuel: 80.0,
                max_torch_fuel: 100.0,
                hunger: 75.0,
                max_hunger: 100.0,
                status_effects: Vec::new(),
                temp_buffs: Vec::new(),
            },
        },
        keys: vec!["key_bronze".to_string()],
        entity_registry: vec![ItemEntity {
            instance_id: "item_1".to_string(),
            item_id: "sword_iron".to_string(),
            quality: ItemQuality::Common,
            modifiers: Vec::new(),
            location: ItemLocation::Backpack { slot: 0 },
        }],
        flags: Vec::new(),
        level_snapshots: HashMap::new(),
        level_grids: HashMap::new(),
        quests: None,
    }
}

// ---------------------------------------------------------------------------
// Mock store — mirrors `makeMockStorage()` in the TS suite.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockStore {
    entries: HashMap<String, String>,
    fail_next_set: bool,
}

impl SaveStore for MockStore {
    fn get_item(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    fn set_item(&mut self, key: &str, value: String) -> Result<(), String> {
        if self.fail_next_set {
            self.fail_next_set = false;
            return Err("QuotaExceededError".to_string());
        }
        self.entries.insert(key.to_string(), value);
        Ok(())
    }

    fn remove_item(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

// ---------------------------------------------------------------------------
// 1. Conversion helpers
// ---------------------------------------------------------------------------

#[test]
fn map_to_record_and_record_to_map_round_trip_multiple_entries() {
    let original = HashMap::from([
        ("a".to_string(), 1),
        ("b".to_string(), 2),
        ("c".to_string(), 3),
    ]);
    let record = map_to_record(&original);
    let restored = record_to_map(&record);
    assert_eq!(restored, original);
}

#[test]
fn map_to_record_and_record_to_map_round_trip_empty_map() {
    let original: HashMap<String, i32> = HashMap::new();
    assert_eq!(record_to_map(&map_to_record(&original)), original);
}

#[test]
fn map_to_record_produces_matching_keys() {
    let map = HashMap::from([("x".to_string(), "hello"), ("y".to_string(), "world")]);
    assert_eq!(map_to_record(&map), map);
}

#[test]
fn record_to_map_produces_matching_entries() {
    let record = HashMap::from([("p".to_string(), 10), ("q".to_string(), 20)]);
    let result = record_to_map(&record);
    assert_eq!(result.get("p"), Some(&10));
    assert_eq!(result.get("q"), Some(&20));
    assert_eq!(result.len(), 2);
}

#[test]
fn set_to_array_and_array_to_set_round_trip_multiple_entries() {
    let original = HashSet::from(["alpha".to_string(), "beta".to_string(), "gamma".to_string()]);
    let array = set_to_array(&original);
    assert_eq!(array_to_set(&array), original);
}

#[test]
fn set_to_array_and_array_to_set_round_trip_empty_set() {
    let original: HashSet<String> = HashSet::new();
    assert_eq!(array_to_set(&set_to_array(&original)), original);
}

#[test]
fn set_to_array_produces_a_plain_vec() {
    let array = set_to_array(&HashSet::from(["x".to_string(), "y".to_string()]));
    assert_eq!(array.len(), 2);
    assert!(array.contains(&"x".to_string()));
    assert!(array.contains(&"y".to_string()));
}

// ---------------------------------------------------------------------------
// 2. serialize_level_snapshot / deserialize_level_snapshot round-trip
// ---------------------------------------------------------------------------

#[test]
fn round_trips_the_minimal_snapshot_maps_have_same_entries() {
    let original = minimal_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&original));

    assert_eq!(restored.layer.doors, original.layer.doors);
    assert_eq!(restored.layer.keys, original.layer.keys);
    assert_eq!(restored.layer.enemies, original.layer.enemies);
}

#[test]
fn round_trips_destroyed_walls_and_explored_cells_as_sets() {
    let original = minimal_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&original));

    assert_eq!(
        restored.layer.destroyed_walls,
        original.layer.destroyed_walls
    );
    assert_eq!(restored.layer.explored_cells, original.layer.explored_cells);
}

#[test]
fn round_trips_registry_snapshot() {
    let original = minimal_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&original));

    assert_eq!(restored.registry_snapshot, original.registry_snapshot);
}

#[test]
fn round_trips_signal_state() {
    let original = minimal_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&original));

    assert_eq!(restored.signal_state, original.signal_state);
}

#[test]
fn round_trips_enemy_status_effects() {
    let original = minimal_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&original));

    assert_eq!(
        restored
            .layer
            .enemies
            .get("enemy_2_3")
            .map(|e| &e.status_effects),
        original
            .layer
            .enemies
            .get("enemy_2_3")
            .map(|e| &e.status_effects),
    );
}

#[test]
fn serialized_form_uses_json_arrays_for_sets_and_objects_for_maps() {
    let serialized = serialize_level_snapshot(&minimal_level_snapshot());
    let value = serde_json::to_value(&serialized).expect("serializes");

    assert!(value["destroyedWalls"].is_array());
    assert!(value["exploredCells"].is_array());
    assert!(value["doors"].is_object());
}

// ---------------------------------------------------------------------------
// 3. JSON stringify / parse round-trip
// ---------------------------------------------------------------------------

#[test]
fn survives_json_round_trip_with_no_data_loss() {
    let original = minimal_level_snapshot();
    let serialized = serialize_level_snapshot(&original);
    let json = serde_json::to_string(&serialized).expect("serializes");
    let via_parse: SerializedLevelSnapshot = serde_json::from_str(&json).expect("parses");
    let restored = deserialize_level_snapshot(&via_parse);

    assert_eq!(restored.layer.doors, original.layer.doors);
    assert_eq!(restored.layer.keys, original.layer.keys);
    assert_eq!(restored.layer.enemies, original.layer.enemies);
    assert_eq!(
        restored.layer.destroyed_walls,
        original.layer.destroyed_walls
    );
    assert_eq!(restored.layer.explored_cells, original.layer.explored_cells);
    assert_eq!(restored.registry_snapshot, original.registry_snapshot);
    assert_eq!(restored.signal_state, original.signal_state);
}

#[test]
fn preserves_enemy_status_effects_through_json_round_trip() {
    let original = minimal_level_snapshot();
    let json = serde_json::to_string(&serialize_level_snapshot(&original)).expect("serializes");
    let via_parse: SerializedLevelSnapshot = serde_json::from_str(&json).expect("parses");
    let restored = deserialize_level_snapshot(&via_parse);

    assert_eq!(
        restored
            .layer
            .enemies
            .get("enemy_2_3")
            .map(|e| &e.status_effects),
        original
            .layer
            .enemies
            .get("enemy_2_3")
            .map(|e| &e.status_effects),
    );
}

// ---------------------------------------------------------------------------
// 4. Slot management with a mocked store
// ---------------------------------------------------------------------------

#[test]
fn round_trips_a_save_data_through_a_slot() {
    let mut store = MockStore::default();
    let data = minimal_save_data();
    let key = SAVE_SLOT_KEYS[0];

    assert!(save_to_slot(&mut store, key, &data));
    let loaded = load_from_slot(&store, key);

    assert!(loaded.is_some());
    let loaded = loaded.expect("checked above");
    assert_eq!(loaded.version, 1);
    assert_eq!(loaded.player.stats.player_name, "Hero");
    assert_eq!(loaded.dungeon_name, "Test Dungeon");
    assert_eq!(loaded.keys, vec!["key_bronze".to_string()]);
}

#[test]
fn load_from_slot_returns_none_for_an_empty_slot() {
    let store = MockStore::default();
    assert!(load_from_slot(&store, "delveward_save_99").is_none());
}

#[test]
fn load_from_slot_returns_none_for_invalid_json() {
    let mut store = MockStore::default();
    store
        .set_item(
            "delveward_save_bad".to_string().as_str(),
            "{ not valid json %%".to_string(),
        )
        .expect("mock store never fails");
    assert!(load_from_slot(&store, "delveward_save_bad").is_none());
}

#[test]
fn load_from_slot_returns_none_when_version_is_not_1() {
    let mut store = MockStore::default();
    let mut bad = minimal_save_data();
    bad.version = 2;
    let json = serde_json::to_string(&bad).expect("serializes");
    store
        .set_item("delveward_save_v2", json)
        .expect("mock store never fails");
    assert!(load_from_slot(&store, "delveward_save_v2").is_none());
}

#[test]
fn save_to_slot_returns_false_when_set_item_fails() {
    let mut store = MockStore {
        fail_next_set: true,
        ..MockStore::default()
    };
    assert!(!save_to_slot(
        &mut store,
        SAVE_SLOT_KEYS[0],
        &minimal_save_data()
    ));
}

// ---------------------------------------------------------------------------
// 5. delete_slot
// ---------------------------------------------------------------------------

#[test]
fn delete_slot_removes_the_entry_so_load_from_slot_returns_none_afterwards() {
    let mut store = MockStore::default();
    let key = SAVE_SLOT_KEYS[1];
    save_to_slot(&mut store, key, &minimal_save_data());
    assert!(load_from_slot(&store, key).is_some());

    delve_core::save_system::delete_slot(&mut store, key);
    assert!(load_from_slot(&store, key).is_none());
}

#[test]
fn delete_slot_tolerates_a_slot_that_was_never_set() {
    let mut store = MockStore::default();
    delve_core::save_system::delete_slot(&mut store, "delveward_save_nonexistent");
}

// ---------------------------------------------------------------------------
// 6. get_slot_metadata
// ---------------------------------------------------------------------------

#[test]
fn get_slot_metadata_returns_correct_fields_for_a_saved_slot() {
    let mut store = MockStore::default();
    let data = minimal_save_data();
    let key = SAVE_SLOT_KEYS[2];
    save_to_slot(&mut store, key, &data);

    let meta = get_slot_metadata(&store, key).expect("slot was saved");
    assert_eq!(meta.player_name, "Hero");
    assert_eq!(meta.level_id, "level1");
    assert_eq!(meta.character_level, 2);
    assert_eq!(meta.dungeon_name, "Test Dungeon");
    assert_eq!(meta.saved_at, 1_700_000_000_000);
}

#[test]
fn get_slot_metadata_returns_none_for_an_empty_slot() {
    let store = MockStore::default();
    assert!(get_slot_metadata(&store, "delveward_save_empty").is_none());
}

#[test]
fn get_slot_metadata_returns_none_for_invalid_json() {
    let mut store = MockStore::default();
    store
        .set_item("delveward_save_bad2", "oops".to_string())
        .expect("mock store never fails");
    assert!(get_slot_metadata(&store, "delveward_save_bad2").is_none());
}

#[test]
fn get_slot_metadata_returns_none_when_version_is_not_1() {
    let mut store = MockStore::default();
    let mut bad = minimal_save_data();
    bad.version = 99;
    let json = serde_json::to_string(&bad).expect("serializes");
    store
        .set_item("delveward_save_badv", json)
        .expect("mock store never fails");
    assert!(get_slot_metadata(&store, "delveward_save_badv").is_none());
}

// ---------------------------------------------------------------------------
// 7. get_all_slot_metadata
// ---------------------------------------------------------------------------

#[test]
fn get_all_slot_metadata_returns_an_entry_for_every_manual_slot_plus_autosave() {
    let store = MockStore::default();
    let result = get_all_slot_metadata(&store);
    let expected_keys: Vec<&str> = SAVE_SLOT_KEYS
        .into_iter()
        .chain(std::iter::once(AUTOSAVE_KEY))
        .collect();

    assert_eq!(result.len(), expected_keys.len());
    for key in expected_keys {
        assert!(result.contains_key(key));
    }
}

#[test]
fn get_all_slot_metadata_returns_none_for_empty_slots_and_some_for_populated_slots() {
    let mut store = MockStore::default();
    let key = SAVE_SLOT_KEYS[3];
    save_to_slot(&mut store, key, &minimal_save_data());

    let result = get_all_slot_metadata(&store);

    assert!(result.get(key).expect("key present").is_some());
    assert_eq!(
        result
            .get(key)
            .expect("key present")
            .as_ref()
            .expect("populated")
            .player_name,
        "Hero"
    );

    for (other_key, metadata) in &result {
        if other_key != key {
            assert!(metadata.is_none());
        }
    }
}

// ---------------------------------------------------------------------------
// 8. Version validation
// ---------------------------------------------------------------------------

#[test]
fn load_from_slot_rejects_version_0() {
    let mut store = MockStore::default();
    let mut bad = minimal_save_data();
    bad.version = 0;
    let json = serde_json::to_string(&bad).expect("serializes");
    store.set_item("v0", json).expect("mock store never fails");
    assert!(load_from_slot(&store, "v0").is_none());
}

#[test]
fn load_from_slot_rejects_version_2() {
    let mut store = MockStore::default();
    let mut bad = minimal_save_data();
    bad.version = 2;
    let json = serde_json::to_string(&bad).expect("serializes");
    store.set_item("v2", json).expect("mock store never fails");
    assert!(load_from_slot(&store, "v2").is_none());
}

#[test]
fn load_from_slot_accepts_version_1() {
    let mut store = MockStore::default();
    let key = SAVE_SLOT_KEYS[0];
    save_to_slot(&mut store, key, &minimal_save_data());
    assert_eq!(load_from_slot(&store, key).map(|d| d.version), Some(1));
}

// ---------------------------------------------------------------------------
// 9. Empty LevelSnapshot round-trip
// ---------------------------------------------------------------------------

#[test]
fn empty_level_snapshot_serializes_and_deserializes_with_all_maps_empty() {
    let empty = LevelSnapshot::default();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&empty));

    assert_eq!(restored.layer.doors.len(), 0);
    assert_eq!(restored.layer.enemies.len(), 0);
    assert_eq!(restored.layer.levers.len(), 0);
    assert_eq!(restored.layer.destroyed_walls.len(), 0);
    assert_eq!(restored.layer.explored_cells.len(), 0);
    assert_eq!(restored.registry_snapshot, Vec::new());
    assert_eq!(restored.signal_state, None);
}

#[test]
fn empty_level_snapshot_survives_json_round_trip() {
    let empty = LevelSnapshot::default();
    let json = serde_json::to_string(&serialize_level_snapshot(&empty)).expect("serializes");
    let via_parse: SerializedLevelSnapshot = serde_json::from_str(&json).expect("parses");
    let restored = deserialize_level_snapshot(&via_parse);

    assert_eq!(restored.layer.doors.len(), 0);
    assert_eq!(restored.layer.destroyed_walls.len(), 0);
    assert_eq!(restored.signal_state, None);
}

// ---------------------------------------------------------------------------
// 10. Complex LevelSnapshot round-trip
// ---------------------------------------------------------------------------

#[test]
fn complex_snapshot_preserves_enemy_with_multiple_status_effects() {
    let complex = complex_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&complex));
    let enemy = restored
        .layer
        .enemies
        .get("enemy_4_4")
        .expect("enemy present");

    assert_eq!(enemy.status_effects.len(), 2);
    assert_eq!(
        enemy.status_effects[0].effect_type,
        StatusEffectType::Burning
    );
    assert_eq!(enemy.status_effects[1].effect_type, StatusEffectType::Slow);
}

#[test]
fn complex_snapshot_preserves_lever_with_multiple_targets() {
    let complex = complex_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&complex));
    let lever = restored
        .layer
        .levers
        .get("lever_2_2")
        .expect("lever present");

    assert_eq!(
        lever.targets,
        vec!["door_0_1".to_string(), "door_5_5".to_string()]
    );
    assert_eq!(lever.signal_mode, Some(SignalMode::Timed));
    assert_eq!(lever.signal_duration, Some(1.5));
}

#[test]
fn complex_snapshot_preserves_signal_state_with_sources_and_receivers() {
    let complex = complex_level_snapshot();
    let restored = deserialize_level_snapshot(&serialize_level_snapshot(&complex));
    let signal_state = restored.signal_state.expect("signal state present");

    assert_eq!(signal_state.sources.len(), 1);
    assert_eq!(signal_state.receivers.len(), 1);
    assert_eq!(signal_state.now, 88.5);
    assert_eq!(signal_state.sources[0].entity_id, "src_lever_2_2");
}

#[test]
fn complex_snapshot_survives_json_round_trip() {
    let complex = complex_level_snapshot();
    let json = serde_json::to_string(&serialize_level_snapshot(&complex)).expect("serializes");
    let via_parse: SerializedLevelSnapshot = serde_json::from_str(&json).expect("parses");
    let restored = deserialize_level_snapshot(&via_parse);

    assert_eq!(
        restored
            .layer
            .enemies
            .get("enemy_4_4")
            .expect("enemy present")
            .hp,
        15.0
    );
    assert_eq!(
        restored
            .layer
            .levers
            .get("lever_2_2")
            .expect("lever present")
            .targets,
        vec!["door_0_1".to_string(), "door_5_5".to_string()]
    );
    assert_eq!(
        restored.signal_state.expect("signal state present").now,
        88.5
    );
}

// ---------------------------------------------------------------------------
// Extra coverage: build_save_data / apply_save_data have no TS test
// counterpart (saveSystem.test.ts never imports them), but the round trip
// through a real GameState is exercised here for confidence.
// ---------------------------------------------------------------------------

use delve_core::save_system::{
    ApplySaveDataResult, BuildSaveDataParams, apply_save_data, build_save_data,
};
use delve_core::types::{Dungeon, DungeonLevel, DungeonPlayerStart};

fn empty_game_state() -> GameState {
    GameState::new(
        &[],
        None,
        "level1",
        None,
        GameStateDeps::default(),
        &mut || 0.5,
    )
}

fn single_level_dungeon() -> Dungeon {
    Dungeon {
        name: "Test Dungeon".to_string(),
        levels: vec![DungeonLevel {
            id: Some("level1".to_string()),
            name: "Level One".to_string(),
            grid: vec![
                "#####".to_string(),
                "#...#".to_string(),
                "#####".to_string(),
            ],
            player_start: None,
            entities: Vec::new(),
            environment: None,
            ceiling: None,
            skybox: None,
            dust_motes: None,
            water_drips: None,
            fireflies: None,
            defaults: None,
            char_defs: None,
            areas: None,
            layers: Vec::new(),
        }],
        player_start: DungeonPlayerStart {
            level_id: "level1".to_string(),
            col: 2,
            row: 3,
            facing: Facing::N,
            layer_index: None,
        },
    }
}

#[test]
fn build_save_data_captures_player_position_and_dungeon_name() {
    let mut game_state = empty_game_state();
    game_state.player.player_name = "Hero".to_string();
    game_state.player.gold = 42;

    let dungeon = single_level_dungeon();
    let level_snapshots = HashMap::new();

    let data = build_save_data(BuildSaveDataParams {
        game_state: &game_state,
        player_col: 2,
        player_row: 3,
        player_facing: Facing::N,
        current_level_id: "level1".to_string(),
        level_snapshots: &level_snapshots,
        dungeon: &dungeon,
        timestamp: 123,
        quests: None,
    });

    assert_eq!(data.version, 1);
    assert_eq!(data.dungeon_name, "Test Dungeon");
    assert_eq!(data.player.col, 2);
    assert_eq!(data.player.row, 3);
    assert_eq!(data.player.stats.player_name, "Hero");
    assert_eq!(data.player.stats.gold, 42);
    assert!(data.level_snapshots.contains_key("level1"));
    assert_eq!(data.level_grids.get("level1").map(Vec::len), Some(3));
}

#[test]
fn apply_save_data_restores_player_state_and_grid_mutations() {
    let game_state = empty_game_state();
    let dungeon = single_level_dungeon();
    let level_snapshots = HashMap::new();

    let mut data = build_save_data(BuildSaveDataParams {
        game_state: &game_state,
        player_col: 1,
        player_row: 1,
        player_facing: Facing::E,
        current_level_id: "level1".to_string(),
        level_snapshots: &level_snapshots,
        dungeon: &dungeon,
        timestamp: 456,
        quests: None,
    });
    data.player.stats.player_name = "Restored".to_string();
    data.player.stats.hp = 12.0;
    data.level_grids.insert(
        "level1".to_string(),
        vec![
            "#####".to_string(),
            "#.X.#".to_string(),
            "#####".to_string(),
        ],
    );

    let mut fresh_state = empty_game_state();
    let mut fresh_dungeon = single_level_dungeon();
    let ApplySaveDataResult {
        target_level_id,
        player_col,
        player_row,
        player_facing,
        ..
    } = apply_save_data(&data, &mut fresh_state, &mut fresh_dungeon);

    assert_eq!(target_level_id, "level1");
    assert_eq!((player_col, player_row, player_facing), (1, 1, Facing::E));
    assert_eq!(fresh_state.player.player_name, "Restored");
    assert_eq!(fresh_state.player.hp, 12.0);
    assert_eq!(
        fresh_dungeon.levels[0].grid,
        vec![
            "#####".to_string(),
            "#.X.#".to_string(),
            "#####".to_string()
        ]
    );
}

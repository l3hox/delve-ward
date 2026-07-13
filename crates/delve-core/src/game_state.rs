//! Central game state: per-layer world entity instances, the entity registry,
//! the signal manager, and the player character sheet.
//!
//! The TS original splits behavior across `InventoryState`, `CombatState`,
//! `WorldEntityState`, and a `GameState` facade that shares the entity
//! registry between them; in Rust the data lives in sub-structs but behavior
//! that spans them is implemented directly on [`GameState`]. TS callbacks
//! (`onDoorSignalChanged`, ...) become [`WorldEvent`]s accumulated on the
//! state and drained by the shell via [`GameState::take_events`].

use crate::entities::{EntityRegistry, EquipSlot, ItemEntity, ItemLocation};
use crate::grid::Facing;
use crate::inventory_state::{AllocatableStat, InventoryState, LEVEL_CAP};
use crate::items::{ItemDatabase, ItemDef, ItemQuality, ItemSubtype, ItemType};
use crate::loot::DropsOverride;
use crate::signal_manager::{
    GateMode, GateType, SignalEvent, SignalManager, SignalManagerState, SignalMode,
};
use crate::status_effect_state::{BuffStat, StatusEffectState, TempBuff};
use crate::status_effects::{StatusEffect, StatusEffectType, remove_effects_by_type};
use crate::types::{EnemyInstance, Entity, LayerDef};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// --- Registrars (TS module singletons become injected dependencies) ---

pub trait EnemyRegistrar: Send + Sync {
    fn has_enemy(&self, enemy_type: &str) -> bool;
    fn create_enemy(&self, col: i64, row: i64, enemy_type: &str) -> Option<EnemyInstance>;
    fn regen_pause_duration(&self, enemy_type: &str) -> Option<f64>;
}

pub trait NpcRegistrar: Send + Sync {
    fn has_npc(&self, npc_id: &str) -> bool;
}

// --- Cell keys ---

#[must_use]
pub fn door_key(col: i64, row: i64) -> String {
    format!("{col},{row}")
}

#[must_use]
pub fn parse_door_key(key: &str) -> (i64, i64) {
    let mut parts = key.split(',').map(|part| part.parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

#[must_use]
pub fn thin_wall_key(col: i64, row: i64, wall: ThinWallSide) -> String {
    format!("{col},{row}:{}", wall.as_str())
}

#[must_use]
pub fn mesh_key(layer_index: usize, col: i64, row: i64) -> String {
    format!("{layer_index}:{col},{row}")
}

#[must_use]
pub fn layer_door_key(layer_index: usize, key: &str) -> String {
    format!("{layer_index}:{key}")
}

// --- World entity instances ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DoorState {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoorInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub state: DoorState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    pub mechanical: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_mode: Option<GateMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub key_id: String,
    pub picked_up: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeverState {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeverInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub targets: Vec<String>,
    pub wall: Facing,
    pub state: LeverState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_mode: Option<SignalMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_delay: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlateInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub targets: Vec<String>,
    pub activated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_mode: Option<SignalMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_delay: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub targets: Vec<String>,
    pub signal_mode: SignalMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_delay: Option<f64>,
    pub fired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TripwireOrientation {
    EW,
    NS,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TripwireInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub targets: Vec<String>,
    pub signal_mode: SignalMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_delay: Option<f64>,
    pub visibility_threshold: f64,
    pub orientation: TripwireOrientation,
    pub triggered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub gate_type: GateType,
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StairDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StairInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub direction: StairDirection,
    pub facing: Facing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LauncherFireMode {
    Single,
    Repeat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrapLauncherInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub facing: Facing,
    pub projectile_type: String,
    pub fire_mode: LauncherFireMode,
    pub reload_time: f64,
    pub next_fire_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_range: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SconceInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub wall: Facing,
    pub lit: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakableWallInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub hp: f64,
    pub max_hp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drops: Option<DropsOverride>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretWallInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub opened: bool,
    pub persistent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChestState {
    Closed,
    Open,
    Locked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChestInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub state: ChestState,
    pub facing: Facing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_mode: Option<GateMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drops: Option<DropsOverride>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub wall: Facing,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub npc_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsableState {
    Active,
    Used,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FountainInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub state: UsableState,
    pub heal_amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookshelfInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub wall: Facing,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AltarInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub state: UsableState,
    pub buff_type: BuffStat,
    pub buff_amount: f64,
    pub buff_duration: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarrelInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub hp: f64,
    pub max_hp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drops: Option<DropsOverride>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinWallSide {
    S,
    E,
}

impl ThinWallSide {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ThinWallSide::S => "S",
            ThinWallSide::E => "E",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinWallHeight {
    Full,
    Half,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinWallInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub wall: ThinWallSide,
    pub solid: bool,
    pub height: ThinWallHeight,
    pub texture: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_back: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RampStyle {
    Ramp,
    Stairs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RampInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub facing: Facing,
    pub style: RampStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub prop_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall: Option<Facing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PitTrapState {
    Closed,
    Open,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitTrapInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub state: PitTrapState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_mode: Option<GateMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnerInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub enemy_type: String,
    pub max_active: f64,
    pub interval: f64,
    pub spawn_radius: f64,
    pub active: bool,
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_mode: Option<GateMode>,
    pub spawn_timer: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoulderState {
    Idle,
    Rolling,
    Falling,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoulderInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub direction: Facing,
    pub state: BoulderState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_mode: Option<GateMode>,
    pub roll_damage: f64,
    pub fall_damage: f64,
    pub insta_kill_enemies: bool,
    pub pushable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntervalMode {
    Fixed,
    Random,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoulderSpawnerInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub col: i64,
    pub row: i64,
    pub direction: Facing,
    pub interval_mode: IntervalMode,
    pub interval: f64,
    pub interval_min: f64,
    pub interval_max: f64,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_mode: Option<GateMode>,
    pub spawn_timer: f64,
    pub next_interval: f64,
    pub roll_damage: f64,
    pub fall_damage: f64,
    pub insta_kill_enemies: bool,
    pub pushable: bool,
}

// --- Layer state ---

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerState {
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
    pub npcs: HashMap<String, NpcInstance>,
    pub fountains: HashMap<String, FountainInstance>,
    pub bookshelves: HashMap<String, BookshelfInstance>,
    pub altars: HashMap<String, AltarInstance>,
    pub barrels: HashMap<String, BarrelInstance>,
    pub thin_walls: HashMap<String, ThinWallInstance>,
    pub ramps: HashMap<String, RampInstance>,
    pub props: HashMap<String, PropInstance>,
    pub pit_traps: HashMap<String, PitTrapInstance>,
    pub spawners: HashMap<String, SpawnerInstance>,
    pub boulders: HashMap<String, BoulderInstance>,
    pub boulder_spawners: HashMap<String, BoulderSpawnerInstance>,
    pub destroyed_walls: HashSet<String>,
    pub explored_cells: HashSet<String>,
}

/// Per-layer world snapshot; layer 0 additionally carries the item registry
/// and signal state (they are level-global).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelSnapshot {
    pub layer: LayerState,
    pub registry_snapshot: Vec<ItemEntity>,
    pub signal_state: Option<SignalManagerState>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiLayerSnapshot {
    pub layers: Vec<LevelSnapshot>,
    pub active_layer_index: usize,
}

// --- Outward events (TS callbacks) ---

#[derive(Debug, Clone, PartialEq)]
pub enum WorldEvent {
    DoorSignalChanged { col: i64, row: i64, open: bool },
    ChestSignalChanged { col: i64, row: i64, open: bool },
    PitTrapSignalChanged { col: i64, row: i64, open: bool },
    SpawnerSignalChanged { col: i64, row: i64, active: bool },
    BoulderSpawnerSignalChanged { col: i64, row: i64, active: bool },
    BoulderSignalChanged { col: i64, row: i64, active: bool },
    LeverReset { col: i64, row: i64 },
    PlateReset { col: i64, row: i64 },
    LauncherFire { col: i64, row: i64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityIndexEntry {
    pub col: i64,
    pub row: i64,
    pub entity_type: String,
    pub layer_index: usize,
}

// --- Dependencies ---

#[derive(Default)]
pub struct GameStateDeps {
    pub items: Option<Arc<ItemDatabase>>,
    pub enemy_registrar: Option<Box<dyn EnemyRegistrar>>,
    pub npc_registrar: Option<Box<dyn NpcRegistrar>>,
}

// --- Effective stats ---

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectiveStats {
    pub atk: f64,
    pub def: f64,
    pub max_hp: f64,
    pub crit_chance: f64,
    pub dodge_chance: f64,
    pub effective_str: f64,
    pub effective_dex: f64,
    pub effective_vit: f64,
    pub effective_wis: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EquipResult {
    pub success: bool,
    pub reason: Option<String>,
    pub swapped_to_slot: Option<u32>,
}

/// Full player character snapshot for save files and the HUD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateSnapshot {
    pub hp: f64,
    pub max_hp: f64,
    pub str: f64,
    pub dex: f64,
    pub vit: f64,
    pub wis: f64,
    pub xp: i64,
    pub level: i64,
    pub attribute_points: i64,
    pub player_name: String,
    pub gold: i64,
    pub torch_fuel: f64,
    pub max_torch_fuel: f64,
    #[serde(default = "default_hunger")]
    pub hunger: f64,
    #[serde(default = "default_hunger")]
    pub max_hunger: f64,
    pub status_effects: Vec<StatusEffect>,
    #[serde(default)]
    pub temp_buffs: Vec<TempBuff>,
}

fn default_hunger() -> f64 {
    100.0
}

// --- Parsing helpers ---

fn grid_char(grid: Option<&[String]>, col: i64, row: i64) -> Option<char> {
    let grid = grid?;
    let row_text = grid.get(usize::try_from(row).ok()?)?;
    row_text.chars().nth(usize::try_from(col).ok()?)
}

fn auto_detect_lever_wall(col: i64, row: i64, grid: Option<&[String]>) -> Facing {
    if grid.is_none() {
        return Facing::N;
    }
    if grid_char(grid, col, row - 1) == Some('#') {
        return Facing::N;
    }
    if grid_char(grid, col, row + 1) == Some('#') {
        return Facing::S;
    }
    if grid_char(grid, col + 1, row) == Some('#') {
        return Facing::E;
    }
    if grid_char(grid, col - 1, row) == Some('#') {
        return Facing::W;
    }
    Facing::N
}

fn auto_detect_tripwire_orientation(
    col: i64,
    row: i64,
    grid: Option<&[String]>,
) -> TripwireOrientation {
    let Some(rows) = grid else {
        return TripwireOrientation::EW;
    };
    let north_solid = row - 1 < 0 || grid_char(grid, col, row - 1) == Some('#');
    let south_solid = row + 1 >= rows.len() as i64 || grid_char(grid, col, row + 1) == Some('#');
    if north_solid && south_solid {
        TripwireOrientation::NS
    } else {
        TripwireOrientation::EW
    }
}

fn prop_facing(entity: &Entity, key: &str) -> Option<Facing> {
    match entity.prop_str(key) {
        Some("N") => Some(Facing::N),
        Some("S") => Some(Facing::S),
        Some("E") => Some(Facing::E),
        Some("W") => Some(Facing::W),
        _ => None,
    }
}

fn prop_signal_mode(entity: &Entity) -> Option<SignalMode> {
    match entity.prop_str("signalMode") {
        Some("toggle") => Some(SignalMode::Toggle),
        Some("momentary") => Some(SignalMode::Momentary),
        Some("one_shot") => Some(SignalMode::OneShot),
        Some("timed") => Some(SignalMode::Timed),
        _ => None,
    }
}

fn prop_gate_mode(entity: &Entity) -> Option<GateMode> {
    match entity.prop_str("gateMode") {
        Some("or") => Some(GateMode::Or),
        Some("and") => Some(GateMode::And),
        Some("xor") => Some(GateMode::Xor),
        _ => None,
    }
}

fn prop_targets(entity: &Entity) -> Option<Vec<String>> {
    entity.props.get("targets").and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
    })
}

/// Migrated entities carry `targets`; unmigrated test entities may still use
/// `target`/`targetDoor` (the TS parser mirrors this fallback).
fn prop_targets_with_fallback(entity: &Entity) -> Vec<String> {
    if let Some(targets) = prop_targets(entity) {
        return targets;
    }
    if let Some(target) = entity.prop_str("target")
        && !target.is_empty()
    {
        return vec![target.to_string()];
    }
    if let Some(target_door) = entity.prop_str("targetDoor")
        && !target_door.is_empty()
    {
        return vec![target_door.to_string()];
    }
    Vec::new()
}

fn prop_drops(entity: &Entity) -> Option<DropsOverride> {
    entity
        .props
        .get("drops")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn fmt_num(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn replace_grid_char(grid: &mut [String], col: i64, row: i64, replacement: char) {
    let Ok(row_index) = usize::try_from(row) else {
        return;
    };
    let Ok(col_index) = usize::try_from(col) else {
        return;
    };
    if let Some(row_text) = grid.get_mut(row_index) {
        let mut chars: Vec<char> = row_text.chars().collect();
        if col_index < chars.len() {
            chars[col_index] = replacement;
            *row_text = chars.into_iter().collect();
        }
    }
}

fn subtype_to_equip_slot(subtype: ItemSubtype, registry: &EntityRegistry) -> EquipSlot {
    match subtype {
        ItemSubtype::Sword
        | ItemSubtype::Axe
        | ItemSubtype::Dagger
        | ItemSubtype::Mace
        | ItemSubtype::Spear
        | ItemSubtype::Staff => EquipSlot::Weapon,
        ItemSubtype::Head => EquipSlot::Head,
        ItemSubtype::Chest => EquipSlot::Chest,
        ItemSubtype::Legs => EquipSlot::Legs,
        ItemSubtype::Hands => EquipSlot::Hands,
        ItemSubtype::Feet => EquipSlot::Feet,
        ItemSubtype::Shield => EquipSlot::Shield,
        ItemSubtype::Ring => {
            if registry.get_equipped(EquipSlot::Ring1).is_some() {
                EquipSlot::Ring2
            } else {
                EquipSlot::Ring1
            }
        }
        ItemSubtype::Amulet => EquipSlot::Amulet,
        _ => EquipSlot::Weapon,
    }
}

// --- GameState ---

pub struct GameState {
    pub layers: Vec<LayerState>,
    pub active_layer_index: usize,
    pub entity_by_id: HashMap<String, EntityIndexEntry>,
    pub entity_registry: EntityRegistry,
    pub signal_manager: SignalManager,
    pub current_level_id: String,
    pub status_fx: StatusEffectState,
    pub player: InventoryState,
    deps: GameStateDeps,
    pending_events: Vec<WorldEvent>,
}

impl GameState {
    pub fn new(
        entities: &[Entity],
        grid: Option<&[String]>,
        level_id: &str,
        layer_defs: Option<&[LayerDef]>,
        deps: GameStateDeps,
        random: &mut dyn FnMut() -> f64,
    ) -> Self {
        let mut state = Self {
            layers: vec![LayerState::default()],
            active_layer_index: 0,
            entity_by_id: HashMap::new(),
            entity_registry: EntityRegistry::new(),
            signal_manager: SignalManager::new(),
            current_level_id: level_id.to_string(),
            status_fx: StatusEffectState::default(),
            player: InventoryState::default(),
            deps,
            pending_events: Vec::new(),
        };

        match layer_defs {
            Some(layer_defs) if !layer_defs.is_empty() => {
                state.layers = layer_defs.iter().map(|_| LayerState::default()).collect();
                for (index, layer_def) in layer_defs.iter().enumerate() {
                    state.active_layer_index = index;
                    state.parse_entities(&layer_def.entities, Some(&layer_def.grid), random);
                }
                state.active_layer_index = 0;
            }
            _ => state.parse_entities(entities, grid, random),
        }
        state
    }

    #[must_use]
    pub fn active_layer(&self) -> &LayerState {
        &self.layers[self.active_layer_index]
    }

    pub fn active_layer_mut(&mut self) -> &mut LayerState {
        &mut self.layers[self.active_layer_index]
    }

    /// Peek a layer by index without disturbing `active_layer_index`. Needed
    /// by callers that only hold `&GameState` (the `MoveRules` closures, for
    /// ramp accessibility across layers) and so can't use TS's
    /// save-then-restore-`activeLayerIndex` trick to read another layer.
    #[must_use]
    pub fn layer(&self, index: usize) -> Option<&LayerState> {
        self.layers.get(index)
    }

    /// Mutably peek a layer by index without disturbing `active_layer_index`
    /// — the `layer` companion for callers that must write through a
    /// specific, possibly-non-active layer (e.g. a blocked-door retry that
    /// recorded the layer it was created on, since `active_layer_index` may
    /// have since changed via same-level falling).
    pub fn layer_mut(&mut self, index: usize) -> Option<&mut LayerState> {
        self.layers.get_mut(index)
    }

    /// Drain outward world events produced since the last call.
    pub fn take_events(&mut self) -> Vec<WorldEvent> {
        std::mem::take(&mut self.pending_events)
    }

    // --- Entity parsing ---

    fn parse_entities(
        &mut self,
        entities: &[Entity],
        grid: Option<&[String]>,
        random: &mut dyn FnMut() -> f64,
    ) {
        for entity in entities {
            let level_id = self.current_level_id.clone();
            let layer_index = self.active_layer_index;
            let _ = Self::parse_signal_entity(self.active_layer_mut(), entity, grid)
                || Self::parse_environment_entity(self.active_layer_mut(), entity, grid)
                || Self::parse_structure_entity(self.active_layer_mut(), entity, grid, random)
                || self.parse_npc_entity(entity, grid)
                || self.parse_item_entity(entity, &level_id, layer_index);
        }

        self.rebuild_entity_index();
        self.mark_mechanical_targets();
        self.init_signal_manager();
    }

    fn parse_signal_entity(
        layer: &mut LayerState,
        entity: &Entity,
        grid: Option<&[String]>,
    ) -> bool {
        let key = door_key(entity.col, entity.row);
        match entity.entity_type.as_str() {
            "door" => {
                let state = match entity.prop_str("state") {
                    Some("open") => DoorState::Open,
                    _ => DoorState::Closed,
                };
                layer.doors.insert(
                    key,
                    DoorInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        state,
                        key_id: entity.prop_str("keyId").map(ToString::to_string),
                        mechanical: false,
                        gate_mode: prop_gate_mode(entity),
                    },
                );
                true
            }
            "key" => {
                layer.keys.insert(
                    key,
                    KeyInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        key_id: entity.prop_str("keyId").unwrap_or_default().to_string(),
                        picked_up: false,
                    },
                );
                true
            }
            "lever" => {
                let wall = prop_facing(entity, "wall")
                    .unwrap_or_else(|| auto_detect_lever_wall(entity.col, entity.row, grid));
                layer.levers.insert(
                    key,
                    LeverInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        targets: prop_targets_with_fallback(entity),
                        wall,
                        state: LeverState::Up,
                        signal_mode: prop_signal_mode(entity),
                        signal_duration: entity.prop_f64("signalDuration"),
                        signal_delay: entity.prop_f64("signalDelay"),
                    },
                );
                true
            }
            "pressure_plate" => {
                layer.plates.insert(
                    key,
                    PlateInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        targets: prop_targets_with_fallback(entity),
                        activated: false,
                        signal_mode: prop_signal_mode(entity),
                        signal_duration: entity.prop_f64("signalDuration"),
                        signal_delay: entity.prop_f64("signalDelay"),
                    },
                );
                true
            }
            "trigger" => {
                layer.triggers.insert(
                    key,
                    TriggerInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        targets: prop_targets(entity).unwrap_or_default(),
                        signal_mode: prop_signal_mode(entity).unwrap_or(SignalMode::Momentary),
                        signal_duration: entity.prop_f64("signalDuration"),
                        signal_delay: entity.prop_f64("signalDelay"),
                        fired: false,
                    },
                );
                true
            }
            "tripwire" => {
                let orientation = match entity.prop_str("orientation") {
                    Some("EW") => TripwireOrientation::EW,
                    Some("NS") => TripwireOrientation::NS,
                    _ => auto_detect_tripwire_orientation(entity.col, entity.row, grid),
                };
                layer.tripwires.insert(
                    key,
                    TripwireInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        targets: prop_targets(entity).unwrap_or_default(),
                        signal_mode: prop_signal_mode(entity).unwrap_or(SignalMode::OneShot),
                        signal_duration: entity.prop_f64("signalDuration"),
                        signal_delay: entity.prop_f64("signalDelay"),
                        visibility_threshold: entity.prop_f64("visibilityThreshold").unwrap_or(8.0),
                        orientation,
                        triggered: false,
                    },
                );
                true
            }
            "gate" => {
                let gate_type = match entity.prop_str("gateType") {
                    Some("or") => GateType::Or,
                    Some("not") => GateType::Not,
                    Some("delay") => GateType::Delay,
                    Some("pulse_edge") => GateType::PulseEdge,
                    Some("pulse_repeat") => GateType::PulseRepeat,
                    _ => GateType::And,
                };
                layer.gates.insert(
                    key,
                    GateInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        gate_type,
                        targets: prop_targets(entity).unwrap_or_default(),
                        delay: entity.prop_f64("delay"),
                        interval: entity.prop_f64("interval"),
                    },
                );
                true
            }
            _ => false,
        }
    }

    fn parse_environment_entity(
        layer: &mut LayerState,
        entity: &Entity,
        grid: Option<&[String]>,
    ) -> bool {
        let key = door_key(entity.col, entity.row);
        match entity.entity_type.as_str() {
            "trap_launcher" => {
                let fire_mode = match entity.prop_str("fireMode") {
                    Some("single") => LauncherFireMode::Single,
                    _ => LauncherFireMode::Repeat,
                };
                layer.trap_launchers.insert(
                    key,
                    TrapLauncherInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        facing: prop_facing(entity, "facing").unwrap_or(Facing::S),
                        projectile_type: entity
                            .prop_str("projectileType")
                            .unwrap_or("dart")
                            .to_string(),
                        fire_mode,
                        reload_time: entity.prop_f64("reloadTime").unwrap_or(3.0),
                        next_fire_at: 0.0,
                        max_range: entity.prop_f64("maxRange"),
                    },
                );
                true
            }
            "torch_sconce" => {
                let wall = prop_facing(entity, "wall")
                    .unwrap_or_else(|| auto_detect_lever_wall(entity.col, entity.row, grid));
                layer.sconces.insert(
                    key,
                    SconceInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        wall,
                        lit: true,
                    },
                );
                true
            }
            "stairs" => {
                let direction = match entity.prop_str("direction") {
                    Some("up") => StairDirection::Up,
                    _ => StairDirection::Down,
                };
                layer.stairs.insert(
                    key,
                    StairInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        direction,
                        facing: prop_facing(entity, "facing").unwrap_or(Facing::N),
                    },
                );
                true
            }
            _ => false,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn parse_structure_entity(
        layer: &mut LayerState,
        entity: &Entity,
        grid: Option<&[String]>,
        random: &mut dyn FnMut() -> f64,
    ) -> bool {
        let key = door_key(entity.col, entity.row);
        match entity.entity_type.as_str() {
            "breakable_wall" => {
                let hp = entity.prop_f64("hp").unwrap_or(30.0);
                layer.breakable_walls.insert(
                    key,
                    BreakableWallInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        hp,
                        max_hp: hp,
                        drops: prop_drops(entity),
                    },
                );
                true
            }
            "secret_wall" => {
                layer.secret_walls.insert(
                    key,
                    SecretWallInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        opened: false,
                        persistent: entity.prop_bool("persistent").unwrap_or(false),
                    },
                );
                true
            }
            "block" => {
                layer.blocks.insert(
                    key,
                    BlockInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                    },
                );
                true
            }
            "chest" => {
                let state = match entity.prop_str("state") {
                    Some("open") => ChestState::Open,
                    Some("locked") => ChestState::Locked,
                    _ => ChestState::Closed,
                };
                layer.chests.insert(
                    key,
                    ChestInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        state,
                        facing: prop_facing(entity, "facing").unwrap_or(Facing::S),
                        key_id: entity.prop_str("keyId").map(ToString::to_string),
                        gate_mode: prop_gate_mode(entity),
                        targets: prop_targets(entity),
                        drops: prop_drops(entity),
                    },
                );
                true
            }
            "sign" => {
                let wall = prop_facing(entity, "wall")
                    .unwrap_or_else(|| auto_detect_lever_wall(entity.col, entity.row, grid));
                layer.signs.insert(
                    key,
                    SignInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        wall,
                        text: entity.prop_str("text").unwrap_or_default().to_string(),
                    },
                );
                true
            }
            "thin_wall" => {
                let wall = match entity.prop_str("wall") {
                    Some("E") => ThinWallSide::E,
                    _ => ThinWallSide::S,
                };
                let height = match entity.prop_str("height") {
                    Some("half") => ThinWallHeight::Half,
                    _ => ThinWallHeight::Full,
                };
                layer.thin_walls.insert(
                    thin_wall_key(entity.col, entity.row, wall),
                    ThinWallInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        wall,
                        solid: entity.prop_bool("solid").unwrap_or(true),
                        height,
                        texture: entity
                            .prop_str("texture")
                            .unwrap_or("stone_thin")
                            .to_string(),
                        texture_back: entity.prop_str("textureBack").map(ToString::to_string),
                    },
                );
                true
            }
            "ramp" => {
                let style = match entity.prop_str("style") {
                    Some("stairs") => RampStyle::Stairs,
                    _ => RampStyle::Ramp,
                };
                layer.ramps.insert(
                    key,
                    RampInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        facing: prop_facing(entity, "facing").unwrap_or(Facing::N),
                        style,
                    },
                );
                true
            }
            "prop" => {
                layer.props.insert(
                    key,
                    PropInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        prop_id: entity.prop_str("propId").unwrap_or("pillar").to_string(),
                        wall: prop_facing(entity, "wall"),
                        rotation: entity.prop_f64("rotation").map(|r| r as i64),
                    },
                );
                true
            }
            "pit_trap" => {
                let state = match entity.prop_str("state") {
                    Some("open") => PitTrapState::Open,
                    _ => PitTrapState::Closed,
                };
                layer.pit_traps.insert(
                    key,
                    PitTrapInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        state,
                        gate_mode: prop_gate_mode(entity),
                    },
                );
                true
            }
            "spawner" => {
                layer.spawners.insert(
                    key,
                    SpawnerInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        enemy_type: entity.prop_str("enemyType").unwrap_or("rat").to_string(),
                        max_active: entity.prop_f64("maxActive").unwrap_or(3.0),
                        interval: entity.prop_f64("interval").unwrap_or(10.0),
                        spawn_radius: entity.prop_f64("spawnRadius").unwrap_or(2.0),
                        active: entity.prop_bool("active") != Some(false),
                        visible: entity.prop_bool("visible") != Some(false),
                        gate_mode: prop_gate_mode(entity),
                        spawn_timer: 0.0,
                    },
                );
                true
            }
            "boulder" => {
                let state = match entity.prop_str("state") {
                    Some("rolling") => BoulderState::Rolling,
                    Some("falling") => BoulderState::Falling,
                    _ => BoulderState::Idle,
                };
                layer.boulders.insert(
                    key,
                    BoulderInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        direction: prop_facing(entity, "direction").unwrap_or(Facing::N),
                        state,
                        gate_mode: prop_gate_mode(entity),
                        roll_damage: entity.prop_f64("rollDamage").unwrap_or(30.0),
                        fall_damage: entity.prop_f64("fallDamage").unwrap_or(60.0),
                        insta_kill_enemies: entity.prop_bool("instaKillEnemies") != Some(false),
                        pushable: entity.prop_bool("pushable") == Some(true),
                    },
                );
                true
            }
            "boulder_spawner" => {
                let interval_mode = match entity.prop_str("intervalMode") {
                    Some("random") => IntervalMode::Random,
                    _ => IntervalMode::Fixed,
                };
                let interval = entity.prop_f64("interval").unwrap_or(5.0);
                let interval_min = entity.prop_f64("intervalMin").unwrap_or(3.0);
                let interval_max = entity.prop_f64("intervalMax").unwrap_or(8.0);
                let next_interval = if interval_mode == IntervalMode::Random {
                    interval_min + random() * (interval_max - interval_min).max(0.0)
                } else {
                    interval
                };
                layer.boulder_spawners.insert(
                    key,
                    BoulderSpawnerInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        direction: prop_facing(entity, "direction").unwrap_or(Facing::N),
                        interval_mode,
                        interval,
                        interval_min,
                        interval_max,
                        active: entity.prop_bool("active") != Some(false),
                        gate_mode: prop_gate_mode(entity),
                        spawn_timer: 0.0,
                        next_interval,
                        roll_damage: entity.prop_f64("rollDamage").unwrap_or(30.0),
                        fall_damage: entity.prop_f64("fallDamage").unwrap_or(60.0),
                        insta_kill_enemies: entity.prop_bool("instaKillEnemies") != Some(false),
                        pushable: entity.prop_bool("pushable") == Some(true),
                    },
                );
                true
            }
            _ => false,
        }
    }

    fn parse_npc_entity(&mut self, entity: &Entity, grid: Option<&[String]>) -> bool {
        let key = door_key(entity.col, entity.row);
        match entity.entity_type.as_str() {
            "npc" => {
                let npc_id = entity.prop_str("npcId").unwrap_or_default().to_string();
                let known = self
                    .deps
                    .npc_registrar
                    .as_ref()
                    .is_some_and(|registrar| registrar.has_npc(&npc_id));
                if known {
                    self.active_layer_mut().npcs.insert(
                        key,
                        NpcInstance {
                            id: entity.id.clone(),
                            col: entity.col,
                            row: entity.row,
                            npc_id,
                        },
                    );
                }
                true
            }
            "fountain" => {
                let state = match entity.prop_str("state") {
                    Some("used") => UsableState::Used,
                    _ => UsableState::Active,
                };
                self.active_layer_mut().fountains.insert(
                    key,
                    FountainInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        state,
                        heal_amount: entity.prop_f64("healAmount").unwrap_or(20.0),
                    },
                );
                true
            }
            "bookshelf" => {
                let wall = prop_facing(entity, "wall")
                    .unwrap_or_else(|| auto_detect_lever_wall(entity.col, entity.row, grid));
                self.active_layer_mut().bookshelves.insert(
                    key,
                    BookshelfInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        wall,
                        text: entity.prop_str("text").unwrap_or_default().to_string(),
                    },
                );
                true
            }
            "altar" => {
                let state = match entity.prop_str("state") {
                    Some("used") => UsableState::Used,
                    _ => UsableState::Active,
                };
                let buff_type = match entity.prop_str("buffType") {
                    Some("def") => BuffStat::Def,
                    Some("str") => BuffStat::Str,
                    Some("dex") => BuffStat::Dex,
                    Some("vit") => BuffStat::Vit,
                    Some("wis") => BuffStat::Wis,
                    _ => BuffStat::Atk,
                };
                self.active_layer_mut().altars.insert(
                    key,
                    AltarInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        state,
                        buff_type,
                        buff_amount: entity.prop_f64("buffAmount").unwrap_or(5.0),
                        buff_duration: entity.prop_f64("buffDuration").unwrap_or(60.0),
                    },
                );
                true
            }
            "barrel" => {
                let hp = entity.prop_f64("hp").unwrap_or(10.0);
                self.active_layer_mut().barrels.insert(
                    key,
                    BarrelInstance {
                        id: entity.id.clone(),
                        col: entity.col,
                        row: entity.row,
                        hp,
                        max_hp: hp,
                        drops: prop_drops(entity),
                    },
                );
                true
            }
            _ => false,
        }
    }

    fn parse_item_entity(&mut self, entity: &Entity, level_id: &str, layer_index: usize) -> bool {
        let key = door_key(entity.col, entity.row);
        match entity.entity_type.as_str() {
            "enemy" => {
                let enemy_type = entity.prop_str("enemyType").unwrap_or_default().to_string();
                if let Some(registrar) = self.deps.enemy_registrar.as_ref()
                    && registrar.has_enemy(&enemy_type)
                    && let Some(mut instance) =
                        registrar.create_enemy(entity.col, entity.row, &enemy_type)
                {
                    if let Some(drops) = prop_drops(entity) {
                        instance.drops = Some(drops);
                    }
                    self.active_layer_mut().enemies.insert(key, instance);
                }
                true
            }
            "equipment" | "consumable" => {
                let location = ItemLocation::World {
                    level_id: level_id.to_string(),
                    col: i32::try_from(entity.col).unwrap_or(0),
                    row: i32::try_from(entity.row).unwrap_or(0),
                    layer_index: Some(i32::try_from(layer_index).unwrap_or(0)),
                };
                self.entity_registry.create_item(
                    entity.prop_str("itemId").unwrap_or_default(),
                    ItemQuality::Common,
                    location,
                    Vec::new(),
                );
                true
            }
            _ => false,
        }
    }

    fn mark_mechanical_targets(&mut self) {
        let mut all_targets: Vec<String> = Vec::new();
        {
            let layer = self.active_layer();
            for lever in layer.levers.values() {
                all_targets.extend(lever.targets.iter().cloned());
            }
            for plate in layer.plates.values() {
                all_targets.extend(plate.targets.iter().cloned());
            }
            for trigger in layer.triggers.values() {
                all_targets.extend(trigger.targets.iter().cloned());
            }
            for tripwire in layer.tripwires.values() {
                all_targets.extend(tripwire.targets.iter().cloned());
            }
            for gate in layer.gates.values() {
                all_targets.extend(gate.targets.iter().cloned());
            }
            for chest in layer.chests.values() {
                if let Some(targets) = &chest.targets {
                    all_targets.extend(targets.iter().cloned());
                }
            }
        }
        for target in all_targets {
            let Some(position) = self.entity_by_id.get(&target).cloned() else {
                continue;
            };
            let key = door_key(position.col, position.row);
            if let Some(door) = self.active_layer_mut().doors.get_mut(&key) {
                door.mechanical = true;
            }
            if let Some(chest) = self.active_layer_mut().chests.get_mut(&key)
                && chest.gate_mode.is_none()
            {
                chest.gate_mode = Some(GateMode::Or);
            }
        }
    }

    fn init_signal_manager(&mut self) {
        self.signal_manager.clear();
        let saved_index = self.active_layer_index;

        let mut targeted_ids: HashSet<String> = HashSet::new();
        for layer in &self.layers {
            let target_lists = layer
                .levers
                .values()
                .map(|lever| &lever.targets)
                .chain(layer.plates.values().map(|plate| &plate.targets))
                .chain(layer.triggers.values().map(|trigger| &trigger.targets))
                .chain(layer.tripwires.values().map(|tripwire| &tripwire.targets))
                .chain(layer.gates.values().map(|gate| &gate.targets));
            for targets in target_lists {
                targeted_ids.extend(targets.iter().cloned());
            }
            for chest in layer.chests.values() {
                if let Some(targets) = &chest.targets {
                    targeted_ids.extend(targets.iter().cloned());
                }
            }
        }

        type Registration = Box<dyn FnOnce(&mut SignalManager)>;
        for layer_index in 0..self.layers.len() {
            let layer = &self.layers[layer_index];
            let mut registrations: Vec<Registration> = Vec::new();
            for lever in layer.levers.values() {
                if let Some(id) = lever.id.clone() {
                    let targets = lever.targets.clone();
                    let mode = lever.signal_mode.unwrap_or(SignalMode::Toggle);
                    let (duration, delay) = (lever.signal_duration, lever.signal_delay);
                    registrations.push(Box::new(move |sm| {
                        sm.register_source(&id, targets, mode, duration, delay);
                    }));
                }
            }
            for plate in layer.plates.values() {
                if let Some(id) = plate.id.clone() {
                    let targets = plate.targets.clone();
                    let mode = plate.signal_mode.unwrap_or(SignalMode::Toggle);
                    let (duration, delay) = (plate.signal_duration, plate.signal_delay);
                    registrations.push(Box::new(move |sm| {
                        sm.register_source(&id, targets, mode, duration, delay);
                    }));
                }
            }
            for trigger in layer.triggers.values() {
                if let Some(id) = trigger.id.clone() {
                    let targets = trigger.targets.clone();
                    let mode = trigger.signal_mode;
                    let (duration, delay) = (trigger.signal_duration, trigger.signal_delay);
                    registrations.push(Box::new(move |sm| {
                        sm.register_source(&id, targets, mode, duration, delay);
                    }));
                }
            }
            for tripwire in layer.tripwires.values() {
                if let Some(id) = tripwire.id.clone() {
                    let targets = tripwire.targets.clone();
                    let mode = tripwire.signal_mode;
                    let (duration, delay) = (tripwire.signal_duration, tripwire.signal_delay);
                    registrations.push(Box::new(move |sm| {
                        sm.register_source(&id, targets, mode, duration, delay);
                    }));
                }
            }
            for gate in layer.gates.values() {
                if let Some(id) = gate.id.clone() {
                    let (gate_type, targets) = (gate.gate_type, gate.targets.clone());
                    let (delay, interval) = (gate.delay, gate.interval);
                    registrations.push(Box::new(move |sm| {
                        sm.register_gate(&id, gate_type, targets, delay, interval);
                    }));
                }
            }
            for door in layer.doors.values() {
                if let Some(id) = door.id.clone()
                    && door.mechanical
                {
                    let mode = door.gate_mode.unwrap_or(GateMode::Or);
                    registrations.push(Box::new(move |sm| sm.register_receiver(&id, mode)));
                }
            }
            for launcher in layer.trap_launchers.values() {
                if let Some(id) = launcher.id.clone() {
                    registrations.push(Box::new(move |sm| sm.register_receiver(&id, GateMode::Or)));
                }
            }
            for chest in layer.chests.values() {
                if let (Some(id), Some(mode)) = (chest.id.clone(), chest.gate_mode) {
                    registrations.push(Box::new(move |sm| sm.register_receiver(&id, mode)));
                }
            }
            for pit in layer.pit_traps.values() {
                if let Some(id) = pit.id.clone() {
                    let mode = pit.gate_mode.unwrap_or(GateMode::Or);
                    registrations.push(Box::new(move |sm| sm.register_receiver(&id, mode)));
                }
            }
            for spawner in layer.spawners.values() {
                if let Some(id) = spawner.id.clone()
                    && targeted_ids.contains(&id)
                {
                    let mode = spawner.gate_mode.unwrap_or(GateMode::Or);
                    registrations.push(Box::new(move |sm| sm.register_receiver(&id, mode)));
                }
            }
            for boulder_spawner in layer.boulder_spawners.values() {
                if let Some(id) = boulder_spawner.id.clone()
                    && targeted_ids.contains(&id)
                {
                    let mode = boulder_spawner.gate_mode.unwrap_or(GateMode::Or);
                    registrations.push(Box::new(move |sm| sm.register_receiver(&id, mode)));
                }
            }
            for boulder in layer.boulders.values() {
                if let Some(id) = boulder.id.clone()
                    && targeted_ids.contains(&id)
                {
                    let mode = boulder.gate_mode.unwrap_or(GateMode::Or);
                    registrations.push(Box::new(move |sm| sm.register_receiver(&id, mode)));
                }
            }
            for chest in layer.chests.values() {
                if let Some(id) = chest.id.clone()
                    && chest.targets.as_ref().is_some_and(|t| !t.is_empty())
                {
                    let targets = chest.targets.clone().unwrap_or_default();
                    registrations.push(Box::new(move |sm| {
                        sm.register_source(&id, targets, SignalMode::Toggle, None, None);
                    }));
                }
            }
            for registration in registrations {
                registration(&mut self.signal_manager);
            }
        }

        self.active_layer_index = saved_index;

        let events = self.signal_manager.propagate();
        self.handle_signal_events(events);
        self.sync_signal_receiver_states();
    }

    /// Applies signal manager events to world state and records outward events.
    pub fn handle_signal_events(&mut self, events: Vec<SignalEvent>) {
        let mut queue: std::collections::VecDeque<SignalEvent> = events.into();
        while let Some(event) = queue.pop_front() {
            match event {
                SignalEvent::ReceiverChanged { entity_id, active } => {
                    let follow_up = self.on_receiver_changed(&entity_id, active);
                    queue.extend(follow_up);
                }
                SignalEvent::SourceDeactivated { entity_id } => {
                    self.on_source_deactivated(&entity_id);
                }
            }
        }
    }

    fn on_receiver_changed(&mut self, entity_id: &str, active: bool) -> Vec<SignalEvent> {
        let Some(entry) = self.entity_by_id.get(entity_id).cloned() else {
            return Vec::new();
        };
        let saved = self.active_layer_index;
        self.active_layer_index = entry.layer_index;
        let key = door_key(entry.col, entry.row);
        let mut follow_up = Vec::new();

        if let Some(door) = self.active_layer_mut().doors.get_mut(&key) {
            door.state = if active {
                DoorState::Open
            } else {
                DoorState::Closed
            };
            self.pending_events.push(WorldEvent::DoorSignalChanged {
                col: entry.col,
                row: entry.row,
                open: active,
            });
            self.active_layer_index = saved;
            return follow_up;
        }

        let chest_signal = {
            if let Some(chest) = self.active_layer_mut().chests.get_mut(&key) {
                if active {
                    chest.state = ChestState::Open;
                    let signal = chest
                        .id
                        .clone()
                        .filter(|_| chest.targets.as_ref().is_some_and(|t| !t.is_empty()));
                    self.pending_events.push(WorldEvent::ChestSignalChanged {
                        col: entry.col,
                        row: entry.row,
                        open: true,
                    });
                    Some(signal)
                } else {
                    chest.state = ChestState::Closed;
                    self.pending_events.push(WorldEvent::ChestSignalChanged {
                        col: entry.col,
                        row: entry.row,
                        open: false,
                    });
                    Some(None)
                }
            } else {
                None
            }
        };
        if let Some(signal) = chest_signal {
            if let Some(chest_id) = signal {
                follow_up.extend(self.signal_manager.set_source_active(&chest_id, true));
            }
            self.active_layer_index = saved;
            return follow_up;
        }

        let now = self.signal_manager.now;
        if let Some(launcher) = self.active_layer_mut().trap_launchers.get_mut(&key) {
            if active && launcher.next_fire_at == 0.0 {
                let fire_mode = launcher.fire_mode;
                let reload_time = launcher.reload_time;
                if fire_mode == LauncherFireMode::Repeat {
                    launcher.next_fire_at = now + reload_time;
                }
                self.pending_events.push(WorldEvent::LauncherFire {
                    col: entry.col,
                    row: entry.row,
                });
            } else if !active {
                launcher.next_fire_at = 0.0;
            }
        }
        if let Some(pit) = self.active_layer_mut().pit_traps.get_mut(&key) {
            pit.state = if active {
                PitTrapState::Open
            } else {
                PitTrapState::Closed
            };
            self.pending_events.push(WorldEvent::PitTrapSignalChanged {
                col: entry.col,
                row: entry.row,
                open: active,
            });
        }
        if let Some(spawner) = self.active_layer_mut().spawners.get_mut(&key) {
            spawner.active = active;
            self.pending_events.push(WorldEvent::SpawnerSignalChanged {
                col: entry.col,
                row: entry.row,
                active,
            });
        }
        if let Some(boulder_spawner) = self.active_layer_mut().boulder_spawners.get_mut(&key) {
            boulder_spawner.active = active;
            self.pending_events
                .push(WorldEvent::BoulderSpawnerSignalChanged {
                    col: entry.col,
                    row: entry.row,
                    active,
                });
        }
        if let Some(boulder) = self.active_layer_mut().boulders.get_mut(&key)
            && active
            && boulder.state == BoulderState::Idle
        {
            boulder.state = BoulderState::Rolling;
            self.pending_events.push(WorldEvent::BoulderSignalChanged {
                col: entry.col,
                row: entry.row,
                active: true,
            });
        }

        self.active_layer_index = saved;
        follow_up
    }

    fn on_source_deactivated(&mut self, entity_id: &str) {
        let Some(entry) = self.entity_by_id.get(entity_id).cloned() else {
            return;
        };
        let saved = self.active_layer_index;
        self.active_layer_index = entry.layer_index;
        let key = door_key(entry.col, entry.row);

        if let Some(lever) = self.active_layer_mut().levers.get_mut(&key)
            && lever.state == LeverState::Down
        {
            lever.state = LeverState::Up;
            self.pending_events.push(WorldEvent::LeverReset {
                col: entry.col,
                row: entry.row,
            });
        }
        if let Some(plate) = self.active_layer_mut().plates.get_mut(&key)
            && plate.activated
        {
            plate.activated = false;
            self.pending_events.push(WorldEvent::PlateReset {
                col: entry.col,
                row: entry.row,
            });
        }
        if let Some(trigger) = self.active_layer_mut().triggers.get_mut(&key)
            && trigger.fired
        {
            trigger.fired = false;
        }

        self.active_layer_index = saved;
    }

    pub fn sync_signal_receiver_states(&mut self) {
        for layer_index in 0..self.layers.len() {
            let receiver_states: Vec<(String, bool)> = {
                let sm = &self.signal_manager;
                let layer = &self.layers[layer_index];
                let ids = layer
                    .pit_traps
                    .values()
                    .filter_map(|p| p.id.clone())
                    .chain(layer.spawners.values().filter_map(|s| s.id.clone()))
                    .chain(layer.boulder_spawners.values().filter_map(|b| b.id.clone()))
                    .chain(
                        layer
                            .doors
                            .values()
                            .filter(|d| d.mechanical)
                            .filter_map(|d| d.id.clone()),
                    );
                ids.filter(|id| sm.get_receiver(id).is_some())
                    .map(|id| {
                        let active = sm.is_receiver_active(&id);
                        (id, active)
                    })
                    .collect()
            };
            let layer = &mut self.layers[layer_index];
            for (id, active) in receiver_states {
                for pit in layer.pit_traps.values_mut() {
                    if pit.id.as_deref() == Some(&id) {
                        pit.state = if active {
                            PitTrapState::Open
                        } else {
                            PitTrapState::Closed
                        };
                    }
                }
                for spawner in layer.spawners.values_mut() {
                    if spawner.id.as_deref() == Some(&id) {
                        spawner.active = active;
                    }
                }
                for boulder_spawner in layer.boulder_spawners.values_mut() {
                    if boulder_spawner.id.as_deref() == Some(&id) {
                        boulder_spawner.active = active;
                    }
                }
                for door in layer.doors.values_mut() {
                    if door.mechanical && door.id.as_deref() == Some(&id) {
                        door.state = if active {
                            DoorState::Open
                        } else {
                            DoorState::Closed
                        };
                    }
                }
            }
        }
    }

    fn rebuild_entity_index(&mut self) {
        self.entity_by_id.clear();
        for (layer_index, layer) in self.layers.iter().enumerate() {
            let mut register = |id: &Option<String>, col: i64, row: i64, entity_type: &str| {
                if let Some(id) = id {
                    self.entity_by_id.insert(
                        id.clone(),
                        EntityIndexEntry {
                            col,
                            row,
                            entity_type: entity_type.to_string(),
                            layer_index,
                        },
                    );
                }
            };
            for v in layer.doors.values() {
                register(&v.id, v.col, v.row, "door");
            }
            for v in layer.keys.values() {
                register(&v.id, v.col, v.row, "key");
            }
            for v in layer.levers.values() {
                register(&v.id, v.col, v.row, "lever");
            }
            for v in layer.plates.values() {
                register(&v.id, v.col, v.row, "pressure_plate");
            }
            for v in layer.triggers.values() {
                register(&v.id, v.col, v.row, "trigger");
            }
            for v in layer.tripwires.values() {
                register(&v.id, v.col, v.row, "tripwire");
            }
            for v in layer.gates.values() {
                register(&v.id, v.col, v.row, "gate");
            }
            for v in layer.trap_launchers.values() {
                register(&v.id, v.col, v.row, "trap_launcher");
            }
            for v in layer.sconces.values() {
                register(&v.id, v.col, v.row, "torch_sconce");
            }
            for v in layer.stairs.values() {
                register(&v.id, v.col, v.row, "stairs");
            }
            for v in layer.breakable_walls.values() {
                register(&v.id, v.col, v.row, "breakable_wall");
            }
            for v in layer.secret_walls.values() {
                register(&v.id, v.col, v.row, "secret_wall");
            }
            for v in layer.blocks.values() {
                register(&v.id, v.col, v.row, "block");
            }
            for v in layer.chests.values() {
                register(&v.id, v.col, v.row, "chest");
            }
            for v in layer.signs.values() {
                register(&v.id, v.col, v.row, "sign");
            }
            for v in layer.npcs.values() {
                register(&v.id, v.col, v.row, "npc");
            }
            for v in layer.fountains.values() {
                register(&v.id, v.col, v.row, "fountain");
            }
            for v in layer.bookshelves.values() {
                register(&v.id, v.col, v.row, "bookshelf");
            }
            for v in layer.altars.values() {
                register(&v.id, v.col, v.row, "altar");
            }
            for v in layer.barrels.values() {
                register(&v.id, v.col, v.row, "barrel");
            }
            for v in layer.thin_walls.values() {
                register(&v.id, v.col, v.row, "thin_wall");
            }
            for v in layer.ramps.values() {
                register(&v.id, v.col, v.row, "ramp");
            }
            for v in layer.props.values() {
                register(&v.id, v.col, v.row, "prop");
            }
            for v in layer.pit_traps.values() {
                register(&v.id, v.col, v.row, "pit_trap");
            }
            for v in layer.spawners.values() {
                register(&v.id, v.col, v.row, "spawner");
            }
            for v in layer.boulders.values() {
                register(&v.id, v.col, v.row, "boulder");
            }
            for v in layer.boulder_spawners.values() {
                register(&v.id, v.col, v.row, "boulder_spawner");
            }
        }
    }

    #[must_use]
    pub fn resolve_entity_position(&self, id: &str) -> Option<&EntityIndexEntry> {
        self.entity_by_id.get(id)
    }
}

// --- World entity facade (TS WorldEntityState) ---

pub struct DamageOutcome {
    pub destroyed: bool,
    pub drops: Option<DropsOverride>,
}

impl GameState {
    #[must_use]
    pub fn get_door(&self, col: i64, row: i64) -> Option<&DoorInstance> {
        self.active_layer().doors.get(&door_key(col, row))
    }

    /// Cells without a door count as open.
    #[must_use]
    pub fn is_door_open(&self, col: i64, row: i64) -> bool {
        self.get_door(col, row)
            .is_none_or(|door| door.state == DoorState::Open)
    }

    pub fn open_door(&mut self, col: i64, row: i64) -> bool {
        let has_key = |key_id: &Option<String>, inventory: &InventoryState| {
            key_id
                .as_deref()
                .is_none_or(|key_id| inventory.has_key(key_id))
        };
        let key = door_key(col, row);
        let player = &self.player;
        let Some(door) = self.layers[self.active_layer_index].doors.get_mut(&key) else {
            return false;
        };
        if door.state != DoorState::Closed || door.mechanical {
            return false;
        }
        if !has_key(&door.key_id, player) {
            return false;
        }
        door.state = DoorState::Open;
        true
    }

    pub fn close_door(&mut self, col: i64, row: i64) -> bool {
        let key = door_key(col, row);
        let blocked_by_enemy = self
            .active_layer()
            .enemies
            .get(&key)
            .is_some_and(|enemy| enemy.blocks_movement);
        let Some(door) = self.active_layer_mut().doors.get_mut(&key) else {
            return false;
        };
        if door.state != DoorState::Open || door.mechanical || blocked_by_enemy {
            return false;
        }
        door.state = DoorState::Closed;
        true
    }

    pub fn toggle_door(&mut self, col: i64, row: i64) {
        if let Some(door) = self.active_layer_mut().doors.get_mut(&door_key(col, row)) {
            door.state = match door.state {
                DoorState::Open => DoorState::Closed,
                DoorState::Closed => DoorState::Open,
            };
        }
    }

    #[must_use]
    pub fn get_stair(&self, col: i64, row: i64) -> Option<&StairInstance> {
        self.active_layer().stairs.get(&door_key(col, row))
    }

    #[must_use]
    pub fn get_lever(&self, col: i64, row: i64) -> Option<&LeverInstance> {
        self.active_layer().levers.get(&door_key(col, row))
    }

    #[must_use]
    pub fn get_sconce(&self, col: i64, row: i64) -> Option<&SconceInstance> {
        self.active_layer().sconces.get(&door_key(col, row))
    }

    #[must_use]
    pub fn get_breakable_wall(&self, col: i64, row: i64) -> Option<&BreakableWallInstance> {
        self.active_layer().breakable_walls.get(&door_key(col, row))
    }

    pub fn damage_breakable_wall(
        &mut self,
        col: i64,
        row: i64,
        damage: f64,
        grid: &mut [String],
    ) -> DamageOutcome {
        let key = door_key(col, row);
        let layer = self.active_layer_mut();
        let Some(wall) = layer.breakable_walls.get_mut(&key) else {
            return DamageOutcome {
                destroyed: false,
                drops: None,
            };
        };
        wall.hp = (wall.hp - damage).max(0.0);
        if wall.hp <= 0.0 {
            let drops = wall.drops.clone();
            layer.breakable_walls.remove(&key);
            replace_grid_char(grid, col, row, '.');
            layer.destroyed_walls.insert(key);
            return DamageOutcome {
                destroyed: true,
                drops,
            };
        }
        DamageOutcome {
            destroyed: false,
            drops: None,
        }
    }

    #[must_use]
    pub fn get_secret_wall(&self, col: i64, row: i64) -> Option<&SecretWallInstance> {
        self.active_layer().secret_walls.get(&door_key(col, row))
    }

    /// Returns `(opened, persistent)`.
    pub fn open_secret_wall(&mut self, col: i64, row: i64, grid: &mut [String]) -> (bool, bool) {
        let key = door_key(col, row);
        let layer = self.active_layer_mut();
        let Some(wall) = layer.secret_walls.get_mut(&key) else {
            return (false, false);
        };
        if wall.opened {
            return (false, false);
        }
        wall.opened = true;
        let persistent = wall.persistent;
        replace_grid_char(grid, col, row, '.');
        layer.destroyed_walls.insert(key);
        (true, persistent)
    }

    #[must_use]
    pub fn get_block(&self, col: i64, row: i64) -> Option<&BlockInstance> {
        self.active_layer().blocks.get(&door_key(col, row))
    }

    #[must_use]
    pub fn is_block_at(&self, col: i64, row: i64) -> bool {
        self.active_layer().blocks.contains_key(&door_key(col, row))
    }

    pub fn push_block(&mut self, from_col: i64, from_row: i64, to_col: i64, to_row: i64) -> bool {
        let from_key = door_key(from_col, from_row);
        let Some(mut block) = self.active_layer_mut().blocks.remove(&from_key) else {
            return false;
        };
        block.col = to_col;
        block.row = to_row;
        self.active_layer_mut()
            .blocks
            .insert(door_key(to_col, to_row), block);
        self.activate_pressure_plate(to_col, to_row);
        true
    }

    #[must_use]
    pub fn get_chest(&self, col: i64, row: i64) -> Option<&ChestInstance> {
        self.active_layer().chests.get(&door_key(col, row))
    }

    #[must_use]
    pub fn get_sign(&self, col: i64, row: i64) -> Option<&SignInstance> {
        self.active_layer().signs.get(&door_key(col, row))
    }

    #[must_use]
    pub fn get_sign_on_wall(&self, col: i64, row: i64, wall: Facing) -> Option<&SignInstance> {
        self.get_sign(col, row).filter(|sign| sign.wall == wall)
    }

    #[must_use]
    pub fn get_npc(&self, col: i64, row: i64) -> Option<&NpcInstance> {
        self.active_layer().npcs.get(&door_key(col, row))
    }

    #[must_use]
    pub fn is_npc_at(&self, col: i64, row: i64) -> bool {
        self.active_layer().npcs.contains_key(&door_key(col, row))
    }

    #[must_use]
    pub fn get_fountain(&self, col: i64, row: i64) -> Option<&FountainInstance> {
        self.active_layer().fountains.get(&door_key(col, row))
    }

    #[must_use]
    pub fn get_bookshelf_on_wall(
        &self,
        col: i64,
        row: i64,
        wall: Facing,
    ) -> Option<&BookshelfInstance> {
        self.active_layer()
            .bookshelves
            .get(&door_key(col, row))
            .filter(|shelf| shelf.wall == wall)
    }

    #[must_use]
    pub fn get_altar(&self, col: i64, row: i64) -> Option<&AltarInstance> {
        self.active_layer().altars.get(&door_key(col, row))
    }

    #[must_use]
    pub fn get_barrel(&self, col: i64, row: i64) -> Option<&BarrelInstance> {
        self.active_layer().barrels.get(&door_key(col, row))
    }

    #[must_use]
    pub fn is_barrel_at(&self, col: i64, row: i64) -> bool {
        self.active_layer()
            .barrels
            .contains_key(&door_key(col, row))
    }

    #[must_use]
    pub fn is_boulder_at(&self, col: i64, row: i64) -> bool {
        self.active_layer()
            .boulders
            .contains_key(&door_key(col, row))
    }

    pub fn damage_barrel(&mut self, col: i64, row: i64, damage: f64) -> DamageOutcome {
        let key = door_key(col, row);
        let layer = self.active_layer_mut();
        let Some(barrel) = layer.barrels.get_mut(&key) else {
            return DamageOutcome {
                destroyed: false,
                drops: None,
            };
        };
        barrel.hp = (barrel.hp - damage).max(0.0);
        if barrel.hp <= 0.0 {
            let drops = barrel.drops.clone();
            layer.barrels.remove(&key);
            return DamageOutcome {
                destroyed: true,
                drops,
            };
        }
        DamageOutcome {
            destroyed: false,
            drops: None,
        }
    }

    #[must_use]
    pub fn get_thin_wall_between(
        &self,
        from_col: i64,
        from_row: i64,
        to_col: i64,
        to_row: i64,
    ) -> Option<&ThinWallInstance> {
        let thin_walls = &self.active_layer().thin_walls;
        let (dcol, drow) = (to_col - from_col, to_row - from_row);
        match (dcol, drow) {
            (0, 1) => thin_walls.get(&thin_wall_key(from_col, from_row, ThinWallSide::S)),
            (0, -1) => thin_walls.get(&thin_wall_key(to_col, to_row, ThinWallSide::S)),
            (1, 0) => thin_walls.get(&thin_wall_key(from_col, from_row, ThinWallSide::E)),
            (-1, 0) => thin_walls.get(&thin_wall_key(to_col, to_row, ThinWallSide::E)),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_edge_blocked(&self, from_col: i64, from_row: i64, to_col: i64, to_row: i64) -> bool {
        self.get_thin_wall_between(from_col, from_row, to_col, to_row)
            .is_some()
    }

    #[must_use]
    pub fn is_solid_edge_blocked(
        &self,
        from_col: i64,
        from_row: i64,
        to_col: i64,
        to_row: i64,
    ) -> bool {
        self.get_thin_wall_between(from_col, from_row, to_col, to_row)
            .is_some_and(|wall| wall.solid)
    }

    pub fn reveal_around(&mut self, col: i64, row: i64, facing: Facing, grid: &[String]) {
        let rows = grid.len() as i64;
        let cols = grid.first().map_or(0, |row| row.chars().count()) as i64;
        let explored = &mut self.layers[self.active_layer_index].explored_cells;
        let mut mark = |c: i64, r: i64| {
            if r >= 0 && r < rows && c >= 0 && c < cols {
                explored.insert(door_key(c, r));
            }
        };
        mark(col, row);
        mark(col, row - 1);
        mark(col, row + 1);
        mark(col - 1, row);
        mark(col + 1, row);

        let (dcol, drow) = facing.delta();
        let (mut c, mut r) = (col + i64::from(dcol), row + i64::from(drow));
        while r >= 0 && r < rows && c >= 0 && c < cols {
            explored.insert(door_key(c, r));
            if grid_char(Some(grid), c, r) == Some('#') {
                break;
            }
            c += i64::from(dcol);
            r += i64::from(drow);
        }
    }

    // --- Flags ---

    #[must_use]
    pub fn has_flag(&self, flag: &str) -> bool {
        self.player.flags.contains(flag)
    }

    pub fn set_flag(&mut self, flag: &str) {
        self.player.flags.insert(flag.to_string());
    }

    pub fn remove_flag(&mut self, flag: &str) {
        self.player.flags.remove(flag);
    }

    // --- Enemies (TS CombatState) ---

    #[must_use]
    pub fn get_enemy(&self, col: i64, row: i64) -> Option<&EnemyInstance> {
        self.active_layer().enemies.get(&door_key(col, row))
    }

    #[must_use]
    pub fn is_enemy_at(&self, col: i64, row: i64) -> bool {
        self.active_layer()
            .enemies
            .contains_key(&door_key(col, row))
    }

    #[must_use]
    pub fn is_blocked_by_enemy(&self, col: i64, row: i64) -> bool {
        self.get_enemy(col, row)
            .is_some_and(|enemy| enemy.blocks_movement)
    }

    pub fn move_enemy(&mut self, from_col: i64, from_row: i64, to_col: i64, to_row: i64) {
        let from_key = door_key(from_col, from_row);
        let Some(mut enemy) = self.active_layer_mut().enemies.remove(&from_key) else {
            return;
        };
        enemy.col = to_col;
        enemy.row = to_row;
        self.active_layer_mut()
            .enemies
            .insert(door_key(to_col, to_row), enemy);
    }

    /// Returns true when the enemy was killed.
    pub fn damage_enemy(&mut self, col: i64, row: i64, amount: f64) -> bool {
        let key = door_key(col, row);
        let regen_pause = |enemy_type: &str, deps: &GameStateDeps| {
            deps.enemy_registrar
                .as_ref()
                .and_then(|registrar| registrar.regen_pause_duration(enemy_type))
                .unwrap_or(3.0)
        };
        let deps_pause = {
            let Some(enemy) = self.active_layer().enemies.get(&key) else {
                return false;
            };
            enemy
                .regen_pause_timer
                .is_some()
                .then(|| regen_pause(&enemy.enemy_type, &self.deps))
        };
        let layer = self.active_layer_mut();
        let Some(enemy) = layer.enemies.get_mut(&key) else {
            return false;
        };
        enemy.hp -= amount;
        if let Some(pause) = deps_pause {
            enemy.regen_pause_timer = Some(pause);
        }
        if enemy.hp <= 0.0 {
            layer.enemies.remove(&key);
            return true;
        }
        false
    }

    // --- Effective stats ---

    #[must_use]
    pub fn get_effective_stats(&self) -> EffectiveStats {
        let mut bonus_str = 0.0;
        let mut bonus_dex = 0.0;
        let mut bonus_vit = 0.0;
        let mut bonus_wis = 0.0;
        let mut weapon_atk = 0.0;
        let mut armor_def = 0.0;
        let mut hp_bonus = 0.0;
        let mut weapon_crit = 0.0;

        if let Some(items) = &self.deps.items {
            for (_, entity) in self.entity_registry.all_equipped() {
                let Some(item_def) = items.get_item(&entity.item_id) else {
                    continue;
                };
                bonus_str += item_def.stats.str.unwrap_or(0.0);
                bonus_dex += item_def.stats.dex.unwrap_or(0.0);
                bonus_vit += item_def.stats.vit.unwrap_or(0.0);
                bonus_wis += item_def.stats.wis.unwrap_or(0.0);
                weapon_atk += item_def.stats.atk.unwrap_or(0.0);
                armor_def += item_def.stats.def.unwrap_or(0.0);
                hp_bonus += item_def.stats.hp.unwrap_or(0.0);
                weapon_crit += item_def.stats.crit_chance.unwrap_or(0.0);
            }
        }

        let buff = |stat: BuffStat| self.status_fx.temp_buff_total(stat);
        let effective_str = self.player.str + bonus_str + buff(BuffStat::Str);
        let effective_dex = self.player.dex + bonus_dex + buff(BuffStat::Dex);
        let effective_vit = self.player.vit + bonus_vit + buff(BuffStat::Vit);
        let effective_wis = self.player.wis + bonus_wis + buff(BuffStat::Wis);

        let str_bonus = (effective_str / 2.0).floor();
        let vit_def_bonus = (effective_vit / 4.0).floor();
        let base_crit = 5.0 + (effective_dex / 3.0).floor();
        let dodge = ((effective_dex - 5.0) / 4.0).floor().clamp(0.0, 25.0);

        EffectiveStats {
            atk: weapon_atk + str_bonus + buff(BuffStat::Atk),
            def: armor_def + vit_def_bonus + buff(BuffStat::Def),
            max_hp: 40.0 + effective_vit * 5.0 + hp_bonus,
            crit_chance: base_crit + weapon_crit,
            dodge_chance: dodge,
            effective_str,
            effective_dex,
            effective_vit,
            effective_wis,
        }
    }

    #[must_use]
    pub fn get_effective_atk(&self) -> f64 {
        self.get_effective_stats().atk
    }

    #[must_use]
    pub fn get_effective_def(&self) -> f64 {
        self.get_effective_stats().def
    }

    #[must_use]
    pub fn get_equipped_weapon_def(&self) -> Option<&ItemDef> {
        let items = self.deps.items.as_ref()?;
        let weapon = self.entity_registry.get_equipped(EquipSlot::Weapon)?;
        items.get_item(&weapon.item_id)
    }

    #[must_use]
    pub fn can_equip_item(&self, item_def: &ItemDef) -> EquipResult {
        let stats = self.get_effective_stats();
        let requirement_checks = [
            (item_def.requirements.str, stats.effective_str, "STR"),
            (item_def.requirements.dex, stats.effective_dex, "DEX"),
            (item_def.requirements.vit, stats.effective_vit, "VIT"),
            (item_def.requirements.wis, stats.effective_wis, "WIS"),
        ];
        for (required, effective, label) in requirement_checks {
            if let Some(required) = required
                && required > 0.0
                && effective < required
            {
                return EquipResult {
                    success: false,
                    reason: Some(format!(
                        "Requires {} {label} (you have {})",
                        fmt_num(required),
                        fmt_num(effective)
                    )),
                    swapped_to_slot: None,
                };
            }
        }
        EquipResult {
            success: true,
            reason: None,
            swapped_to_slot: None,
        }
    }

    // --- Character sheet (TS InventoryState behavior) ---

    #[must_use]
    pub fn xp_for_level(&self, n: i64) -> i64 {
        self.player.xp_for_level(n)
    }

    /// Returns true when at least one level was gained.
    pub fn add_xp(&mut self, amount: i64) -> bool {
        if self.player.level >= LEVEL_CAP {
            return false;
        }
        self.player.xp += amount;
        let mut levelled = false;
        while self.player.level < LEVEL_CAP
            && self.player.xp >= self.player.xp_for_level(self.player.level)
        {
            self.player.level += 1;
            self.player.attribute_points += 3;
            self.player.max_hp = self.get_effective_stats().max_hp;
            levelled = true;
        }
        levelled
    }

    pub fn allocate_point(&mut self, stat: AllocatableStat) -> bool {
        if self.player.attribute_points <= 0 {
            return false;
        }
        self.player.attribute_points -= 1;
        match stat {
            AllocatableStat::Vit => {
                let was_at_max = self.player.hp == self.player.max_hp;
                self.player.vit += 1.0;
                self.player.max_hp = self.get_effective_stats().max_hp;
                if was_at_max {
                    self.player.hp = self.player.max_hp;
                }
            }
            AllocatableStat::Str => self.player.str += 1.0,
            AllocatableStat::Dex => self.player.dex += 1.0,
            AllocatableStat::Wis => self.player.wis += 1.0,
        }
        true
    }

    pub fn apply_character_setup(&mut self, str: f64, dex: f64, vit: f64, wis: f64, name: &str) {
        self.player.str = str;
        self.player.dex = dex;
        self.player.vit = vit;
        self.player.wis = wis;
        self.player.player_name = name.to_string();
        self.player.max_hp = self.get_effective_stats().max_hp;
        self.player.hp = self.player.max_hp;
    }

    pub fn add_key(&mut self, key_id: &str) {
        self.player.add_key(key_id);
    }

    #[must_use]
    pub fn has_key(&self, key_id: &str) -> bool {
        self.player.has_key(key_id)
    }

    pub fn pickup_key_at(&mut self, col: i64, row: i64) -> Option<String> {
        let key = door_key(col, row);
        let key_id = {
            let key_instance = self.layers[self.active_layer_index].keys.get_mut(&key)?;
            if key_instance.picked_up {
                return None;
            }
            key_instance.picked_up = true;
            key_instance.key_id.clone()
        };
        self.player.add_key(&key_id);
        Some(key_id)
    }

    #[must_use]
    pub fn picked_up_keys(&self) -> Vec<String> {
        self.player.picked_up_keys()
    }

    pub fn restore_picked_up_keys(&mut self, keys: &[String]) {
        self.player.restore_picked_up_keys(keys);
    }

    // --- Equipment and item flows ---

    pub fn equip_from_backpack(&mut self, backpack_index: usize) -> EquipResult {
        let failed = EquipResult::default();
        let (instance_id, item_id, backpack_slot) = {
            let backpack_items = self.entity_registry.backpack_items();
            let Some(entity) = backpack_items.get(backpack_index) else {
                return failed;
            };
            let ItemLocation::Backpack { slot } = entity.location else {
                return failed;
            };
            (entity.instance_id.clone(), entity.item_id.clone(), slot)
        };
        let Some(items) = self.deps.items.clone() else {
            return failed;
        };
        let Some(item_def) = items.get_item(&item_id) else {
            return failed;
        };

        let requirement = self.can_equip_item(item_def);
        if !requirement.success {
            return requirement;
        }

        let target_slot = subtype_to_equip_slot(item_def.subtype, &self.entity_registry);
        let existing = self
            .entity_registry
            .get_equipped(target_slot)
            .map(|entity| entity.instance_id.clone());
        if let Some(existing_id) = &existing {
            self.entity_registry.move_item(
                existing_id,
                ItemLocation::Backpack {
                    slot: backpack_slot,
                },
            );
        }
        self.entity_registry
            .move_item(&instance_id, ItemLocation::Equipped { slot: target_slot });
        self.player.max_hp = self.get_effective_stats().max_hp;

        EquipResult {
            success: true,
            reason: None,
            swapped_to_slot: existing.map(|_| backpack_slot),
        }
    }

    pub fn unequip_to_backpack(
        &mut self,
        equip_slot: EquipSlot,
        target_slot: Option<u32>,
    ) -> EquipResult {
        let Some(entity) = self.entity_registry.get_equipped(equip_slot) else {
            return EquipResult::default();
        };
        let instance_id = entity.instance_id.clone();
        let Some(slot) = target_slot.or_else(|| self.entity_registry.next_backpack_slot()) else {
            return EquipResult {
                success: false,
                reason: Some("Backpack is full".to_string()),
                swapped_to_slot: None,
            };
        };
        self.entity_registry
            .move_item(&instance_id, ItemLocation::Backpack { slot });
        self.player.max_hp = self.get_effective_stats().max_hp;
        EquipResult {
            success: true,
            reason: None,
            swapped_to_slot: None,
        }
    }

    pub fn drop_item(&mut self, instance_id: &str, col: i64, row: i64) -> bool {
        if self.entity_registry.get_item(instance_id).is_none() {
            return false;
        }
        let location = ItemLocation::World {
            level_id: self.current_level_id.clone(),
            col: i32::try_from(col).unwrap_or(0),
            row: i32::try_from(row).unwrap_or(0),
            layer_index: Some(i32::try_from(self.active_layer_index).unwrap_or(0)),
        };
        self.entity_registry.move_item(instance_id, location);
        self.player.max_hp = self.get_effective_stats().max_hp;
        true
    }

    /// Returns `Ok(item name)` on pickup, `Err(reason)` when denied, and
    /// `Err("")`-free `Ok(None)`-like `(None, None)` never — mirrors the TS
    /// `{ item?, denied? }` result.
    pub fn pickup_equipment_at(&mut self, col: i64, row: i64) -> (Option<String>, Option<String>) {
        let items = self.deps.items.clone();
        let equip_entity = {
            let ground = self.entity_registry.ground_items(
                &self.current_level_id,
                i32::try_from(col).unwrap_or(0),
                i32::try_from(row).unwrap_or(0),
            );
            ground
                .iter()
                .find(|entity| match &items {
                    None => true,
                    Some(items) => items
                        .get_item(&entity.item_id)
                        .is_some_and(|def| def.item_type != ItemType::Consumable),
                })
                .map(|entity| (entity.instance_id.clone(), entity.item_id.clone()))
        };
        let Some((instance_id, item_id)) = equip_entity else {
            return (None, None);
        };

        if let Some(items) = &items
            && let Some(item_def) = items.get_item(&item_id)
        {
            let requirement = self.can_equip_item(item_def);
            if !requirement.success {
                return (None, requirement.reason);
            }
        }

        let slot = items
            .as_ref()
            .and_then(|items| items.get_item(&item_id))
            .map_or(EquipSlot::Weapon, |def| {
                subtype_to_equip_slot(def.subtype, &self.entity_registry)
            });

        if let Some(existing) = self.entity_registry.get_equipped(slot) {
            let existing_id = existing.instance_id.clone();
            let Some(backpack_slot) = self.entity_registry.next_backpack_slot() else {
                return (None, Some("Backpack is full".to_string()));
            };
            self.entity_registry.move_item(
                &existing_id,
                ItemLocation::Backpack {
                    slot: backpack_slot,
                },
            );
        }
        self.entity_registry
            .move_item(&instance_id, ItemLocation::Equipped { slot });

        let name = items
            .as_ref()
            .and_then(|items| items.get_item(&item_id))
            .map_or_else(|| item_id.clone(), |def| def.name.clone());
        (Some(name), None)
    }

    pub fn pickup_consumable_at(&mut self, col: i64, row: i64) -> Option<String> {
        let slot = self.entity_registry.next_backpack_slot()?;
        let items = self.deps.items.clone();
        let (instance_id, item_id) = {
            let ground = self.entity_registry.ground_items(
                &self.current_level_id,
                i32::try_from(col).unwrap_or(0),
                i32::try_from(row).unwrap_or(0),
            );
            let entity = ground.iter().find(|entity| match &items {
                None => true,
                Some(items) => items
                    .get_item(&entity.item_id)
                    .is_some_and(|def| def.item_type == ItemType::Consumable),
            })?;
            (entity.instance_id.clone(), entity.item_id.clone())
        };
        self.entity_registry
            .move_item(&instance_id, ItemLocation::Backpack { slot });
        Some(
            items
                .as_ref()
                .and_then(|items| items.get_item(&item_id))
                .map_or_else(|| item_id.clone(), |def| def.name.clone()),
        )
    }

    fn apply_consumable_effects(&mut self, item_def: &ItemDef) {
        if item_def.subtype == ItemSubtype::HealthPotion {
            self.player.hp =
                (self.player.hp + item_def.stats.hp.unwrap_or(0.0)).min(self.player.max_hp);
        } else if item_def.subtype == ItemSubtype::TorchOil {
            let fuel = item_def
                .effect
                .as_ref()
                .and_then(|effect| effect.torch_fuel)
                .unwrap_or(0.0);
            self.status_fx.torch_fuel =
                (self.status_fx.torch_fuel + fuel).min(self.status_fx.max_torch_fuel);
        }
        if let Some(restore) = item_def
            .effect
            .as_ref()
            .and_then(|effect| effect.restore_hunger)
        {
            self.status_fx.restore_hunger(restore);
        }
        if item_def
            .effect
            .as_ref()
            .is_some_and(|effect| effect.cure_poison == Some(true))
        {
            self.status_fx.player_status_effects = remove_effects_by_type(
                &self.status_fx.player_status_effects,
                StatusEffectType::Poison,
            );
        }
    }

    pub fn use_consumable_from_registry(&mut self, instance_id: &str) -> bool {
        let Some(items) = self.deps.items.clone() else {
            return false;
        };
        let item_id = {
            let Some(entity) = self.entity_registry.get_item(instance_id) else {
                return false;
            };
            entity.item_id.clone()
        };
        let Some(item_def) = items.get_item(&item_id) else {
            return false;
        };
        if item_def.item_type != ItemType::Consumable {
            return false;
        }
        let item_def = item_def.clone();
        self.apply_consumable_effects(&item_def);
        self.entity_registry.remove_item(instance_id);
        true
    }

    pub fn use_consumable(&mut self, index: usize) -> bool {
        let (instance_id, item_id) = {
            let backpack_items = self.entity_registry.backpack_items();
            let Some(entity) = backpack_items.get(index) else {
                return false;
            };
            (entity.instance_id.clone(), entity.item_id.clone())
        };
        if let Some(items) = self.deps.items.clone() {
            let Some(item_def) = items.get_item(&item_id) else {
                return false;
            };
            if item_def.item_type != ItemType::Consumable {
                return false;
            }
            let item_def = item_def.clone();
            self.apply_consumable_effects(&item_def);
        }
        self.entity_registry.remove_item(&instance_id);
        true
    }

    // --- Status effect delegation ---

    pub fn drain_torch_fuel(&mut self, amount: f64) {
        self.status_fx.drain_torch_fuel(amount);
    }

    pub fn drain_hunger(&mut self, amount: f64) {
        self.status_fx.drain_hunger(amount);
    }

    pub fn restore_hunger(&mut self, amount: f64) {
        self.status_fx.restore_hunger(amount);
    }

    pub fn add_temp_buff(&mut self, stat: BuffStat, amount: f64, duration: f64) {
        self.status_fx.add_temp_buff(stat, amount, duration);
    }

    pub fn tick_temp_buffs(&mut self, delta: f64) {
        self.status_fx.tick_temp_buffs(delta);
    }

    #[must_use]
    pub fn get_temp_buff_total(&self, stat: BuffStat) -> f64 {
        self.status_fx.temp_buff_total(stat)
    }
}

// --- Activations, usables, and snapshots ---

pub struct ChestOpenResult {
    pub opened: bool,
    pub locked: bool,
    pub drops: Option<DropsOverride>,
}

impl GameState {
    /// Flip the lever and route its signal. Returns the lever's targets.
    pub fn activate_lever(&mut self, col: i64, row: i64) -> Option<Vec<String>> {
        let key = door_key(col, row);
        let (id, targets, is_down) = {
            let lever = self.layers[self.active_layer_index].levers.get_mut(&key)?;
            lever.state = match lever.state {
                LeverState::Up => LeverState::Down,
                LeverState::Down => LeverState::Up,
            };
            (
                lever.id.clone(),
                lever.targets.clone(),
                lever.state == LeverState::Down,
            )
        };
        if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
            let events = self.signal_manager.set_source_active(&id, is_down);
            self.handle_signal_events(events);
        } else {
            for target in &targets {
                if let Some(position) = self.entity_by_id.get(target).cloned() {
                    self.toggle_door(position.col, position.row);
                }
            }
        }
        Some(targets)
    }

    pub fn activate_pressure_plate(&mut self, col: i64, row: i64) -> Option<Vec<String>> {
        let key = door_key(col, row);
        let (id, targets, mode, activated) = {
            let plate = self.layers[self.active_layer_index].plates.get(&key)?;
            (
                plate.id.clone(),
                plate.targets.clone(),
                plate.signal_mode.unwrap_or(SignalMode::Toggle),
                plate.activated,
            )
        };

        if mode == SignalMode::Toggle {
            let now_activated = !activated;
            self.layers[self.active_layer_index]
                .plates
                .get_mut(&key)
                .expect("plate present")
                .activated = now_activated;
            if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
                let events = self.signal_manager.set_source_active(&id, now_activated);
                self.handle_signal_events(events);
            }
            if !now_activated {
                self.pending_events
                    .push(WorldEvent::PlateReset { col, row });
            }
            return Some(targets);
        }

        if activated {
            return None;
        }
        self.layers[self.active_layer_index]
            .plates
            .get_mut(&key)
            .expect("plate present")
            .activated = true;
        if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
            let events = self.signal_manager.set_source_active(&id, true);
            self.handle_signal_events(events);
            if mode == SignalMode::Timed
                && let Some(source) = self.signal_manager.get_source(&id)
            {
                let entity_id = source.entity_id.clone();
                self.set_source_deactivate_at(&entity_id, 0.0);
            }
        } else {
            for target in &targets {
                if let Some(position) = self.entity_by_id.get(target).cloned() {
                    let door_cell = door_key(position.col, position.row);
                    if let Some(door) = self.active_layer_mut().doors.get_mut(&door_cell)
                        && door.state == DoorState::Closed
                    {
                        door.state = DoorState::Open;
                    }
                }
            }
        }
        Some(targets)
    }

    fn set_source_deactivate_at(&mut self, entity_id: &str, deactivate_at: f64) {
        let state = self.signal_manager.save_state();
        let mut state = state;
        if let Some(source) = state
            .sources
            .iter_mut()
            .find(|source| source.entity_id == entity_id)
        {
            source.deactivate_at = deactivate_at;
        }
        self.signal_manager.load_state(state);
    }

    pub fn deactivate_pressure_plate(&mut self, col: i64, row: i64) {
        let key = door_key(col, row);
        let Some((id, mode, activated)) = self.active_layer().plates.get(&key).map(|plate| {
            (
                plate.id.clone(),
                plate.signal_mode.unwrap_or(SignalMode::Toggle),
                plate.activated,
            )
        }) else {
            return;
        };
        if !activated {
            return;
        }
        if mode == SignalMode::Momentary {
            self.layers[self.active_layer_index]
                .plates
                .get_mut(&key)
                .expect("plate present")
                .activated = false;
            if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
                let events = self.signal_manager.deactivate_source(&id);
                self.handle_signal_events(events);
            }
            self.pending_events
                .push(WorldEvent::PlateReset { col, row });
        } else if mode == SignalMode::Timed
            && let Some(id) = id
        {
            let refresh = self.signal_manager.get_source(&id).and_then(|source| {
                (source.active && source.duration.is_some())
                    .then(|| self.signal_manager.now + source.duration.expect("checked"))
            });
            if let Some(deactivate_at) = refresh {
                self.set_source_deactivate_at(&id, deactivate_at);
            }
        }
    }

    pub fn activate_trigger(&mut self, col: i64, row: i64) -> bool {
        let key = door_key(col, row);
        let Some((id, mode, fired)) = self
            .active_layer()
            .triggers
            .get(&key)
            .map(|trigger| (trigger.id.clone(), trigger.signal_mode, trigger.fired))
        else {
            return false;
        };

        if mode == SignalMode::Toggle {
            let now_fired = !fired;
            self.layers[self.active_layer_index]
                .triggers
                .get_mut(&key)
                .expect("trigger present")
                .fired = now_fired;
            if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
                let events = self.signal_manager.set_source_active(&id, now_fired);
                self.handle_signal_events(events);
            }
            return true;
        }

        if fired {
            return false;
        }
        self.layers[self.active_layer_index]
            .triggers
            .get_mut(&key)
            .expect("trigger present")
            .fired = true;
        if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
            let events = self.signal_manager.set_source_active(&id, true);
            self.handle_signal_events(events);
            if mode == SignalMode::Timed {
                self.set_source_deactivate_at(&id, 0.0);
            }
        }
        true
    }

    pub fn deactivate_trigger(&mut self, col: i64, row: i64) {
        let key = door_key(col, row);
        let Some((id, mode, fired)) = self
            .active_layer()
            .triggers
            .get(&key)
            .map(|trigger| (trigger.id.clone(), trigger.signal_mode, trigger.fired))
        else {
            return;
        };
        if !fired {
            return;
        }
        if mode == SignalMode::Momentary {
            self.layers[self.active_layer_index]
                .triggers
                .get_mut(&key)
                .expect("trigger present")
                .fired = false;
            if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
                let events = self.signal_manager.deactivate_source(&id);
                self.handle_signal_events(events);
            }
        } else if mode == SignalMode::Timed
            && let Some(id) = id
        {
            let refresh = self.signal_manager.get_source(&id).and_then(|source| {
                (source.active && source.duration.is_some())
                    .then(|| self.signal_manager.now + source.duration.expect("checked"))
            });
            if let Some(deactivate_at) = refresh {
                self.set_source_deactivate_at(&id, deactivate_at);
            }
        }
    }

    pub fn activate_tripwire(&mut self, col: i64, row: i64) -> bool {
        let key = door_key(col, row);
        let Some(tripwire) = self.layers[self.active_layer_index].tripwires.get_mut(&key) else {
            return false;
        };
        if tripwire.triggered {
            return false;
        }
        tripwire.triggered = true;
        let id = tripwire.id.clone();
        if let Some(id) = id.filter(|id| self.signal_manager.get_source(id).is_some()) {
            let events = self.signal_manager.set_source_active(&id, true);
            self.handle_signal_events(events);
        }
        true
    }

    /// Advance the signal clock and repeat-fire any active launchers.
    pub fn tick_signals(&mut self, delta: f64) {
        let events = self.signal_manager.tick(delta);
        self.handle_signal_events(events);
        self.tick_trap_launchers();
    }

    pub fn tick_trap_launchers(&mut self) {
        let now = self.signal_manager.now;
        let launcher_cells: Vec<String> = self
            .active_layer()
            .trap_launchers
            .iter()
            .filter(|(_, launcher)| {
                launcher.fire_mode == LauncherFireMode::Repeat
                    && launcher.next_fire_at > 0.0
                    && now >= launcher.next_fire_at
                    && launcher.id.is_some()
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in launcher_cells {
            let (id, col, row, reload_time) = {
                let launcher = &self.active_layer().trap_launchers[&key];
                (
                    launcher.id.clone().expect("filtered on id"),
                    launcher.col,
                    launcher.row,
                    launcher.reload_time,
                )
            };
            if self.signal_manager.is_receiver_active(&id) {
                if let Some(launcher) = self.active_layer_mut().trap_launchers.get_mut(&key) {
                    launcher.next_fire_at += reload_time;
                }
                self.pending_events
                    .push(WorldEvent::LauncherFire { col, row });
            } else if let Some(launcher) = self.active_layer_mut().trap_launchers.get_mut(&key) {
                launcher.next_fire_at = 0.0;
            }
        }
    }

    pub fn take_sconce_torch(&mut self, col: i64, row: i64) -> bool {
        let key = door_key(col, row);
        let Some(sconce) = self.layers[self.active_layer_index].sconces.get_mut(&key) else {
            return false;
        };
        if !sconce.lit {
            return false;
        }
        sconce.lit = false;
        self.status_fx.torch_fuel = self.status_fx.max_torch_fuel;
        true
    }

    pub fn open_chest(&mut self, col: i64, row: i64) -> ChestOpenResult {
        let key = door_key(col, row);
        let closed = ChestOpenResult {
            opened: false,
            locked: false,
            drops: None,
        };
        let Some((state, key_id, drops)) = self
            .active_layer()
            .chests
            .get(&key)
            .map(|chest| (chest.state, chest.key_id.clone(), chest.drops.clone()))
        else {
            return closed;
        };
        if state == ChestState::Open {
            return closed;
        }
        if state == ChestState::Locked || key_id.is_some() {
            if let Some(key_id) = key_id.filter(|key_id| self.player.has_key(key_id)) {
                self.player.inventory.remove(&key_id);
                self.layers[self.active_layer_index]
                    .chests
                    .get_mut(&key)
                    .expect("chest present")
                    .state = ChestState::Open;
                self.activate_chest_signal_at(&key);
                return ChestOpenResult {
                    opened: true,
                    locked: false,
                    drops,
                };
            }
            return ChestOpenResult {
                opened: false,
                locked: true,
                drops: None,
            };
        }
        self.layers[self.active_layer_index]
            .chests
            .get_mut(&key)
            .expect("chest present")
            .state = ChestState::Open;
        self.activate_chest_signal_at(&key);
        ChestOpenResult {
            opened: true,
            locked: false,
            drops,
        }
    }

    fn activate_chest_signal_at(&mut self, key: &str) {
        let signal_id = self.active_layer().chests.get(key).and_then(|chest| {
            chest
                .id
                .clone()
                .filter(|_| chest.targets.as_ref().is_some_and(|t| !t.is_empty()))
        });
        if let Some(id) = signal_id {
            let events = self.signal_manager.set_source_active(&id, true);
            self.handle_signal_events(events);
        }
    }

    pub fn destroy_chest(&mut self, col: i64, row: i64) -> Option<Option<DropsOverride>> {
        let key = door_key(col, row);
        let state = self.active_layer().chests.get(&key)?.state;
        if state != ChestState::Open {
            self.activate_chest_signal_at(&key);
        }
        let chest = self.active_layer_mut().chests.remove(&key)?;
        Some(chest.drops)
    }

    /// Returns `(healed, heal_amount)`.
    pub fn use_fountain(&mut self, col: i64, row: i64) -> (bool, f64) {
        let key = door_key(col, row);
        let heal_amount = {
            let Some(fountain) = self.layers[self.active_layer_index].fountains.get_mut(&key)
            else {
                return (false, 0.0);
            };
            if fountain.state == UsableState::Used {
                return (false, 0.0);
            }
            fountain.state = UsableState::Used;
            fountain.heal_amount
        };
        self.player.hp = (self.player.hp + heal_amount).min(self.player.max_hp);
        (true, heal_amount)
    }

    /// Returns `(activated, buff_type, buff_amount, buff_duration)`.
    pub fn use_altar(&mut self, col: i64, row: i64) -> (bool, BuffStat, f64, f64) {
        let key = door_key(col, row);
        let buff = {
            let Some(altar) = self.layers[self.active_layer_index].altars.get_mut(&key) else {
                return (false, BuffStat::Atk, 0.0, 0.0);
            };
            if altar.state == UsableState::Used {
                return (false, BuffStat::Atk, 0.0, 0.0);
            }
            altar.state = UsableState::Used;
            (altar.buff_type, altar.buff_amount, altar.buff_duration)
        };
        self.status_fx.add_temp_buff(buff.0, buff.1, buff.2);
        (true, buff.0, buff.1, buff.2)
    }

    // --- Player snapshot ---

    #[must_use]
    pub fn get_player_state(&self) -> PlayerStateSnapshot {
        PlayerStateSnapshot {
            hp: self.player.hp,
            max_hp: self.player.max_hp,
            str: self.player.str,
            dex: self.player.dex,
            vit: self.player.vit,
            wis: self.player.wis,
            xp: self.player.xp,
            level: self.player.level,
            attribute_points: self.player.attribute_points,
            player_name: self.player.player_name.clone(),
            gold: self.player.gold,
            torch_fuel: self.status_fx.torch_fuel,
            max_torch_fuel: self.status_fx.max_torch_fuel,
            hunger: self.status_fx.hunger,
            max_hunger: self.status_fx.max_hunger,
            status_effects: self.status_fx.player_status_effects.clone(),
            temp_buffs: self.status_fx.temp_buffs.clone(),
        }
    }

    pub fn restore_player_state(&mut self, state: &PlayerStateSnapshot) {
        self.player.hp = state.hp;
        self.player.max_hp = state.max_hp;
        self.player.str = state.str;
        self.player.dex = state.dex;
        self.player.vit = state.vit;
        self.player.wis = state.wis;
        self.player.xp = state.xp;
        self.player.level = state.level;
        self.player.attribute_points = state.attribute_points;
        self.player.player_name = state.player_name.clone();
        self.player.gold = state.gold;
        self.status_fx.torch_fuel = state.torch_fuel;
        self.status_fx.max_torch_fuel = state.max_torch_fuel;
        self.status_fx.hunger = state.hunger;
        self.status_fx.max_hunger = state.max_hunger;
        self.status_fx.player_status_effects = state.status_effects.clone();
        self.status_fx.temp_buffs = state.temp_buffs.clone();
    }

    // --- Level snapshots ---

    #[must_use]
    pub fn save_level_state(&self) -> MultiLayerSnapshot {
        let layers = self
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| LevelSnapshot {
                layer: layer.clone(),
                registry_snapshot: if index == 0 {
                    self.entity_registry.snapshot()
                } else {
                    Vec::new()
                },
                signal_state: (index == 0).then(|| self.signal_manager.save_state()),
            })
            .collect();
        MultiLayerSnapshot {
            layers,
            active_layer_index: self.active_layer_index,
        }
    }

    pub fn load_level_state(&mut self, snapshot: &MultiLayerSnapshot) {
        self.layers = snapshot
            .layers
            .iter()
            .map(|level| level.layer.clone())
            .collect();
        self.active_layer_index = snapshot.active_layer_index;
        if let Some(first) = snapshot.layers.first() {
            if !first.registry_snapshot.is_empty() {
                self.entity_registry
                    .restore(first.registry_snapshot.clone());
            }
            self.rebuild_entity_index();
            self.init_signal_manager();
            if let Some(signal_state) = &first.signal_state {
                self.signal_manager.load_state(signal_state.clone());
                self.sync_signal_receiver_states();
            }
        }
    }

    pub fn load_new_level(
        &mut self,
        layer_defs: &[LayerDef],
        level_id: Option<&str>,
        random: &mut dyn FnMut() -> f64,
    ) {
        let old_level_id = self.current_level_id.clone();
        if let Some(level_id) = level_id {
            self.current_level_id = level_id.to_string();
        }
        self.entity_registry.clear_level(&old_level_id);
        self.entity_by_id.clear();

        self.layers = layer_defs.iter().map(|_| LayerState::default()).collect();
        for (index, layer_def) in layer_defs.iter().enumerate() {
            self.active_layer_index = index;
            self.parse_entities(&layer_def.entities, Some(&layer_def.grid), random);
        }
        self.active_layer_index = 0;
    }
}

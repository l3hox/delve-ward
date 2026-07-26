//! Cross-level stair transitions, ported from the TS transition system and
//! its fade-to-black overlay: fade out, swap the level scene at the
//! midpoint, fade back in. Input systems check `Transition::is_active`.

use crate::altars::AltarHandles;
use crate::barrels::BarrelHandles;
use crate::blocks::BlockHandles;
use crate::boulders::BoulderAnimator;
use crate::chests::ChestHandles;
use crate::doors::DoorPanels;
use crate::dungeon::PitFloorHandles;
use crate::enemies::{EnemyBillboards, EnemyDb};
use crate::enemy_feedback::EnemyHealthBars;
use crate::fountains::FountainHandles;
use crate::ground_items::{GroundItemBillboards, ItemDb};
use crate::keys::KeyBillboards;
use crate::level_scene::{LevelEntity, SceneAssets, SceneContext, spawn_level_scene};
use crate::levers::LeverHandles;
use crate::npcs::{NpcBillboards, NpcDb};
use crate::plates::PlateHandles;
use crate::player::Player;
use crate::projectiles::{self, ProjectileBillboards, ProjectileManagerRes};
use crate::save_load_overlay::save_game_to_slot;
use crate::save_store::FileSaveStore;
use crate::sconces::SconceParts;
use crate::session::{DungeonRes, GameRng, LevelSnapshots, OriginalGrids, Session};
use crate::spawners::SpawnerHandles;
use crate::textures::DungeonMaterials;
use crate::tripwires::TripwireHandles;
use crate::wall_entities::WallEntityHandles;
use crate::zones;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::door_key;
use delve_core::grid::build_walkable_set;
use delve_core::level_loader::{
    find_entity_layer_index, get_all_level_entities, resolve_layer_coord,
};
use delve_core::save_system::{AUTOSAVE_KEY, SaveData, apply_save_data};
use delve_core::types::{DungeonLevel, Environment};

const FADE_SPEED: f32 = 2.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    FadeOut,
    Swap,
    FadeIn,
}

/// What `perform_level_swap`/`perform_restart`/`perform_load` should do at
/// the fade's midpoint — matches TS's three `TransitionSystemContext`
/// consumers (`triggerLevelTransition`, `restartLevel`, `loadGame`), each of
/// which independently calls `ctx.transition.startTransition(...)` with its
/// own closure; here that becomes one shared fade/phase machine with a
/// tagged pending action instead of three separate closures.
enum PendingAction {
    Stair(String),
    Restart,
    Load(Box<SaveData>),
}

#[derive(Resource, Default)]
pub struct Transition {
    phase: Phase,
    opacity: f32,
    pending: Option<PendingAction>,
}

impl Transition {
    /// Start a transition toward the stair with `stair_id` as its target.
    /// Ignored while another transition is in progress.
    pub fn begin_stair(&mut self, stair_id: String) {
        self.start(PendingAction::Stair(stair_id));
    }

    /// Start a transition back to the dungeon's starting level, resetting
    /// player vitals — ported from `restartLevel`.
    pub fn begin_restart(&mut self) {
        self.start(PendingAction::Restart);
    }

    /// Start a transition into a loaded save — ported from `loadGame`.
    pub fn begin_load(&mut self, data: SaveData) {
        self.start(PendingAction::Load(Box::new(data)));
    }

    fn start(&mut self, action: PendingAction) {
        if self.phase != Phase::Idle {
            return;
        }
        self.phase = Phase::FadeOut;
        self.pending = Some(action);
    }

    pub fn is_active(&self) -> bool {
        self.phase != Phase::Idle
    }
}

#[derive(Component)]
pub struct TransitionOverlay;

pub fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        TransitionOverlay,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(100),
    ));
}

pub fn tick_transition(
    time: Res<Time>,
    mut transition: ResMut<Transition>,
    mut overlays: Query<&mut BackgroundColor, With<TransitionOverlay>>,
) {
    match transition.phase {
        Phase::Idle | Phase::Swap => return,
        Phase::FadeOut => {
            transition.opacity = (transition.opacity + FADE_SPEED * time.delta_secs()).min(1.0);
            if transition.opacity >= 1.0 {
                transition.phase = Phase::Swap;
            }
        }
        Phase::FadeIn => {
            transition.opacity = (transition.opacity - FADE_SPEED * time.delta_secs()).max(0.0);
            if transition.opacity <= 0.0 {
                transition.phase = Phase::Idle;
            }
        }
    }
    if let Ok(mut background) = overlays.single_mut() {
        background.0 = Color::BLACK.with_alpha(transition.opacity);
    }
}

#[derive(SystemParam)]
pub struct SwapAssets<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    images: ResMut<'w, Assets<Image>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    asset_server: Res<'w, AssetServer>,
}

#[derive(SystemParam)]
pub struct SwapWorld<'w, 's> {
    session: ResMut<'w, Session>,
    // `ResMut` (not `Res`): `perform_load` restores each level's grid from
    // save data via `apply_save_data`, and `perform_restart` restores the
    // start level's grid from `OriginalGrids` — both need write access.
    // `perform_level_swap`'s read-only uses still work through `Deref`.
    dungeon: ResMut<'w, DungeonRes>,
    snapshots: ResMut<'w, LevelSnapshots>,
    dungeon_materials: Res<'w, DungeonMaterials>,
    enemy_db: Res<'w, EnemyDb>,
    items: Res<'w, ItemDb>,
    rng: ResMut<'w, GameRng>,
    clear_color: ResMut<'w, ClearColor>,
    level_entities: Query<'w, 's, Entity, With<LevelEntity>>,
    door_panels: ResMut<'w, DoorPanels>,
    enemy_billboards: ResMut<'w, EnemyBillboards>,
    ground_items: ResMut<'w, GroundItemBillboards>,
    key_billboards: ResMut<'w, KeyBillboards>,
    sconce_parts: ResMut<'w, SconceParts>,
    lever_handles: ResMut<'w, LeverHandles>,
    plate_handles: ResMut<'w, PlateHandles>,
    tripwire_handles: ResMut<'w, TripwireHandles>,
    projectiles: ResMut<'w, ProjectileManagerRes>,
    projectile_billboards: ResMut<'w, ProjectileBillboards>,
    blocked_doors: ResMut<'w, crate::session::BlockedDoors>,
    chest_handles: ResMut<'w, ChestHandles>,
    block_handles: ResMut<'w, BlockHandles>,
    wall_entity_handles: ResMut<'w, WallEntityHandles>,
    vitals: ResMut<'w, crate::status_effects::PlayerVitals>,
    health_bars: ResMut<'w, EnemyHealthBars>,
    npc_db: Res<'w, NpcDb>,
    npc_billboards: ResMut<'w, NpcBillboards>,
    quests: ResMut<'w, crate::dialog_overlay::QuestManagerRes>,
    fountain_handles: ResMut<'w, FountainHandles>,
    altar_handles: ResMut<'w, AltarHandles>,
    barrel_handles: ResMut<'w, BarrelHandles>,
    pit_floor_handles: ResMut<'w, PitFloorHandles>,
    spawner_handles: ResMut<'w, SpawnerHandles>,
    boulder_animator: ResMut<'w, BoulderAnimator>,
    zone_cameras: Query<'w, 's, Entity, With<zones::ZoneCamera>>,
}

/// Reapply recorded wall destruction to a freshly cloned grid: the clone
/// comes from the pristine dungeon definition, while the active layer's
/// `destroyed_walls` set is the durable record of every secret wall opened
/// and breakable wall destroyed (TS instead mutates one shared dungeon
/// object for the whole session).
fn replay_destroyed_walls(game: &delve_core::game_state::GameState, grid: &mut [String]) {
    for key in &game.active_layer().destroyed_walls {
        let Some((col_text, row_text)) = key.split_once(',') else {
            continue;
        };
        let (Ok(col), Ok(row)) = (col_text.parse::<usize>(), row_text.parse::<usize>()) else {
            continue;
        };
        let Some(line) = grid.get_mut(row) else {
            continue;
        };
        let mut characters: Vec<char> = line.chars().collect();
        if let Some(cell) = characters.get_mut(col) {
            *cell = '.';
            *line = characters.into_iter().collect();
        }
    }
}

/// The grid the session runs its walkability checks against: the active
/// layer's own, falling back to the level-wide copy exactly as TS's
/// `ls.layerGrids[ctx.gameState.activeLayerIndex] ?? ls.level.grid` does
/// (`transitionSystem.ts:95,99,184,188`).
///
/// `DungeonLevel::grid` is only a convenience copy of `layers[0].grid` that
/// validation fills in, so using it directly hands the session the *bottom*
/// layer's walls whenever the active layer isn't index 0 — which reads as
/// movement being dead while turning still works, since turning is the one
/// command that never consults the grid.
fn active_layer_grid(level: &DungeonLevel, active_layer_index: usize) -> Vec<String> {
    level
        .layers
        .get(active_layer_index)
        .map_or_else(|| level.grid.clone(), |layer| layer.grid.clone())
}

fn find_stair_level<'a>(dungeon: &'a DungeonRes, stair_id: &str) -> Option<&'a DungeonLevel> {
    dungeon.0.levels.iter().find(|level| {
        get_all_level_entities(level)
            .any(|entity| entity.entity_type == "stairs" && entity.id.as_deref() == Some(stair_id))
    })
}

/// The midpoint of the fade: snapshot the departing level, load the target
/// level state, rebuild the scene, and respawn the player one cell in front
/// of the target stair with their facing preserved.
pub fn perform_level_swap(
    mut commands: Commands,
    mut transition: ResMut<Transition>,
    mut assets: SwapAssets,
    mut world: SwapWorld,
    mut save_store: ResMut<FileSaveStore>,
    mut players: Query<(Entity, &Player)>,
) {
    if transition.phase != Phase::Swap {
        return;
    }
    if !matches!(transition.pending, Some(PendingAction::Stair(_))) {
        return; // a different pending action — `perform_restart`/`perform_load` handle it
    }
    let Some(PendingAction::Stair(stair_id)) = transition.pending.take() else {
        return;
    };
    transition.phase = Phase::FadeIn;
    let Ok((player_entity, player)) = players.single_mut() else {
        return;
    };
    let facing_before = player.grid_state().facing;

    let Some(target_level) = find_stair_level(&world.dungeon, &stair_id) else {
        warn!("stairs target {stair_id} not found in any level");
        return;
    };
    let target_layer_index = find_entity_layer_index(target_level, &stair_id);
    let target_level_id = target_level
        .id
        .clone()
        .unwrap_or_else(|| target_level.name.clone());

    world.blocked_doors.clear();
    let session = &mut *world.session;
    world.snapshots.0.insert(
        session.current_level_id.clone(),
        session.game.save_level_state(),
    );
    for entity in &world.level_entities {
        commands.entity(entity).despawn();
    }
    projectiles::clear_on_transition(&mut world.projectiles, &mut world.projectile_billboards);

    if let Some(snapshot) = world.snapshots.0.get(&target_level_id) {
        session.game.load_level_state(snapshot);
    } else {
        let rng = &mut world.rng.0;
        let mut random = || rng.next_f64();
        session
            .game
            .load_new_level(&target_level.layers, Some(&target_level_id), &mut random);
    }
    session.game.active_layer_index = target_layer_index;

    let Some(stair) = session
        .game
        .active_layer()
        .stairs
        .values()
        .find(|stair| stair.id.as_deref() == Some(stair_id.as_str()))
        .cloned()
    else {
        warn!("stair {stair_id} missing from loaded level {target_level_id}");
        return;
    };
    let (dcol, drow) = stair.facing.delta();
    let spawn_col = stair.col as i32 + dcol;
    let spawn_row = stair.row as i32 + drow;

    session.current_level_id = target_level_id;
    session.environment = target_level.environment.unwrap_or(Environment::Dungeon);
    session.areas = target_level.areas.clone().unwrap_or_default();
    session.grid = target_level.layers[target_layer_index].grid.clone();
    replay_destroyed_walls(&session.game, &mut session.grid);
    session.walkable = build_walkable_set(
        target_level
            .char_defs
            .iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );
    session.last_player_pose = (spawn_col, spawn_row, facing_before);

    let stairs_map = session
        .game
        .active_layer()
        .stairs
        .values()
        .map(|stair| (door_key(stair.col, stair.row), stair.direction))
        .collect();
    let mut swap_player = Player::new(
        session.grid.clone(),
        spawn_col,
        spawn_row,
        facing_before,
        session.walkable.clone(),
        stairs_map,
    );
    swap_player.snap_y_offset_to_layer(session.game.active_layer_index);
    commands.entity(player_entity).insert(swap_player);

    let mut scene_assets = SceneAssets {
        meshes: &mut assets.meshes,
        images: &mut assets.images,
        materials: &mut assets.materials,
        asset_server: &assets.asset_server,
    };
    let scene = SceneContext {
        dungeon_materials: &world.dungeon_materials,
        enemy_db: &world.enemy_db.0,
        items: &world.items.0,
        npc_db: &world.npc_db.0,
        game: &session.game,
        level: target_level,
        walkable: &session.walkable,
    };
    let handles = spawn_level_scene(&mut commands, &mut scene_assets, &scene);
    *world.door_panels = handles.door_panels;
    *world.enemy_billboards = handles.enemy_billboards;
    *world.ground_items = handles.ground_items;
    *world.key_billboards = handles.key_billboards;
    *world.sconce_parts = handles.sconce_parts;
    *world.lever_handles = handles.lever_handles;
    *world.plate_handles = handles.plate_handles;
    *world.tripwire_handles = handles.tripwire_handles;
    *world.chest_handles = handles.chest_handles;
    *world.block_handles = handles.block_handles;
    *world.wall_entity_handles = handles.wall_entity_handles;
    *world.health_bars = handles.health_bars;
    *world.npc_billboards = handles.npc_billboards;
    *world.fountain_handles = handles.fountain_handles;
    *world.pit_floor_handles = handles.pit_floor_handles;
    *world.spawner_handles = handles.spawner_handles;
    *world.boulder_animator = handles.boulder_animator;
    *world.altar_handles = handles.altar_handles;
    *world.barrel_handles = handles.barrel_handles;

    zones::spawn_player_cameras(
        &mut commands,
        player_entity,
        &world.zone_cameras,
        &mut world.clear_color,
        target_level,
        &handles.level_zones,
    );

    let Session { game, grid, .. } = session;
    // TS reveals with the target stair's facing; only the player's
    // orientation keeps the pre-transition facing.
    game.reveal_around(
        i64::from(spawn_col),
        i64::from(spawn_row),
        stair.facing,
        grid,
    );
    game.take_events(); // discard construction-time signal events

    // Autosave on arrival, matching TS's `ctx.saveGame(AUTOSAVE_KEY)` at the
    // end of `triggerLevelTransition` — restart/load do not autosave.
    // `spawn_col`/`spawn_row`/`facing_before` (not a re-fetched `Player`
    // component) are the new position: the `Commands`-driven component
    // replacement above hasn't applied yet within this system.
    if !save_game_to_slot(
        &mut save_store,
        AUTOSAVE_KEY,
        &world.session,
        spawn_col,
        spawn_row,
        facing_before,
        &world.dungeon,
        &world.snapshots,
        &world.quests.0,
    ) {
        warn!("autosave failed");
    }
}

/// The midpoint of a restart's fade: reset to the dungeon's starting level
/// and player vitals, ported from `restartLevel`. Does not autosave — TS's
/// `restartLevel` never calls `saveGame` either.
///
/// Mirrors `perform_level_swap`'s structure rather than sharing a helper
/// with it: the two flows only overlap on the scene-rebuild tail, and
/// factoring that out would obscure more than it would save given how much
/// of the middle (level selection, vitals reset, no snapshot lookup) genuinely
/// differs.
pub fn perform_restart(
    mut commands: Commands,
    mut transition: ResMut<Transition>,
    mut assets: SwapAssets,
    mut world: SwapWorld,
    original_grids: Res<OriginalGrids>,
    mut players: Query<(Entity, &Player)>,
) {
    if transition.phase != Phase::Swap {
        return;
    }
    if !matches!(transition.pending, Some(PendingAction::Restart)) {
        return; // a different pending action — the other `perform_*` systems handle it
    }
    transition.pending = None;
    transition.phase = Phase::FadeIn;
    let Ok((player_entity, _player)) = players.single_mut() else {
        return;
    };

    let start = world.dungeon.0.player_start.clone();
    let start_index = world
        .dungeon
        .0
        .levels
        .iter()
        .position(|level| level.id.as_deref() == Some(start.level_id.as_str()))
        .unwrap_or(0);
    let Some(start_level_id) = world
        .dungeon
        .0
        .levels
        .get(start_index)
        .map(|level| level.id.clone().unwrap_or_else(|| level.name.clone()))
    else {
        warn!("restart: dungeon has no levels");
        return;
    };
    // Restore the starting level's pristine grid before reading it below —
    // matches TS's `if (origGrid) startLevel.grid = [...origGrid];`. Every
    // subsequent read of the level's grid (scene rebuild, reveal) goes
    // through `session.grid`, sourced from this same restored copy, not a
    // per-layer grid — TS's own `restartLevel` reads `startLevel.grid`
    // throughout rather than `layerGrids[activeLayerIndex]`.
    if let Some(original) = original_grids.0.get(&start_level_id) {
        world.dungeon.0.levels[start_index].grid = original.clone();
    }
    let start_level = world.dungeon.0.levels[start_index].clone();

    for entity in &world.level_entities {
        commands.entity(entity).despawn();
    }
    projectiles::clear_on_transition(&mut world.projectiles, &mut world.projectile_billboards);
    world.snapshots.0.clear();
    // TS's `restartLevel` does not clear `ctx.blockedDoors` (unlike
    // `loadGame`, which does) — ported as-is rather than adding a clear
    // that TS itself omits.

    let session = &mut *world.session;
    {
        let rng = &mut world.rng.0;
        let mut random = || rng.next_f64();
        session
            .game
            .load_new_level(&start_level.layers, Some(&start_level_id), &mut random);
    }
    session.game.active_layer_index =
        resolve_layer_coord(&start_level, start.layer_index.unwrap_or(0));

    session.game.player.hp = session.game.player.max_hp;
    session.game.status_fx.torch_fuel = session.game.status_fx.max_torch_fuel;
    session.game.player.attack_cooldown = 0.0;
    session.game.player.gold = 0;
    session.game.status_fx.player_status_effects.clear();

    session.current_level_id = start_level_id;
    session.environment = start_level.environment.unwrap_or(Environment::Dungeon);
    session.areas = start_level.areas.clone().unwrap_or_default();
    session.grid = active_layer_grid(&start_level, session.game.active_layer_index);
    session.walkable = build_walkable_set(
        start_level
            .char_defs
            .iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );
    session.last_player_pose = (start.col, start.row, start.facing);

    let stairs_map = session
        .game
        .active_layer()
        .stairs
        .values()
        .map(|stair| (door_key(stair.col, stair.row), stair.direction))
        .collect();
    let mut restart_player = Player::new(
        session.grid.clone(),
        start.col,
        start.row,
        start.facing,
        session.walkable.clone(),
        stairs_map,
    );
    restart_player.snap_y_offset_to_layer(session.game.active_layer_index);
    commands.entity(player_entity).insert(restart_player);

    let mut scene_assets = SceneAssets {
        meshes: &mut assets.meshes,
        images: &mut assets.images,
        materials: &mut assets.materials,
        asset_server: &assets.asset_server,
    };
    let scene = SceneContext {
        dungeon_materials: &world.dungeon_materials,
        enemy_db: &world.enemy_db.0,
        items: &world.items.0,
        npc_db: &world.npc_db.0,
        game: &session.game,
        level: &start_level,
        walkable: &session.walkable,
    };
    let handles = spawn_level_scene(&mut commands, &mut scene_assets, &scene);
    *world.door_panels = handles.door_panels;
    *world.enemy_billboards = handles.enemy_billboards;
    *world.ground_items = handles.ground_items;
    *world.key_billboards = handles.key_billboards;
    *world.sconce_parts = handles.sconce_parts;
    *world.lever_handles = handles.lever_handles;
    *world.plate_handles = handles.plate_handles;
    *world.tripwire_handles = handles.tripwire_handles;
    *world.chest_handles = handles.chest_handles;
    *world.block_handles = handles.block_handles;
    *world.wall_entity_handles = handles.wall_entity_handles;
    *world.health_bars = handles.health_bars;
    *world.npc_billboards = handles.npc_billboards;
    *world.fountain_handles = handles.fountain_handles;
    *world.pit_floor_handles = handles.pit_floor_handles;
    *world.spawner_handles = handles.spawner_handles;
    *world.boulder_animator = handles.boulder_animator;
    *world.altar_handles = handles.altar_handles;
    *world.barrel_handles = handles.barrel_handles;

    zones::spawn_player_cameras(
        &mut commands,
        player_entity,
        &world.zone_cameras,
        &mut world.clear_color,
        &start_level,
        &handles.level_zones,
    );

    let Session { game, grid, .. } = session;
    game.reveal_around(
        i64::from(start.col),
        i64::from(start.row),
        start.facing,
        grid,
    );
    game.take_events();
}

/// The midpoint of a load's fade: apply the save's data and rebuild the
/// scene at the saved position, ported from `loadGame`. Does not autosave —
/// TS's `loadGame` never calls `saveGame` either.
pub fn perform_load(
    mut commands: Commands,
    mut transition: ResMut<Transition>,
    mut assets: SwapAssets,
    mut world: SwapWorld,
    original_grids: Res<OriginalGrids>,
    mut players: Query<(Entity, &Player)>,
) {
    if transition.phase != Phase::Swap {
        return;
    }
    if !matches!(transition.pending, Some(PendingAction::Load(_))) {
        return; // a different pending action — the other `perform_*` systems handle it
    }
    let Some(PendingAction::Load(data)) = transition.pending.take() else {
        return;
    };
    transition.phase = Phase::FadeIn;
    let Ok((player_entity, _player)) = players.single_mut() else {
        return;
    };

    for entity in &world.level_entities {
        commands.entity(entity).despawn();
    }
    projectiles::clear_on_transition(&mut world.projectiles, &mut world.projectile_billboards);
    world.blocked_doors.clear();

    // Reset every level to its pristine grid before `apply_save_data`
    // overlays the save's own `level_grids` — a defensive safety net for a
    // level the save has no entry for (matching TS's own originalGrids-first
    // restore in `loadGame`; in practice `build_save_data` always populates
    // every level, so this rarely changes anything for saves this port
    // writes itself).
    for level in &mut world.dungeon.0.levels {
        let id = level.id.clone().unwrap_or_else(|| level.name.clone());
        if let Some(original) = original_grids.0.get(&id) {
            level.grid = original.clone();
        }
    }

    let session = &mut *world.session;
    let result = apply_save_data(&data, &mut session.game, &mut world.dungeon.0);
    if let Some(quest_data) = data.quests.clone()
        && let Err(error) = world.quests.0.restore_state(quest_data)
    {
        warn!("save has invalid quest state, quests not restored: {error}");
    }

    world.snapshots.0.clear();
    for (id, snapshot) in result.level_snapshots {
        world.snapshots.0.insert(id, snapshot);
    }

    session.current_level_id = result.target_level_id.clone();
    let Some(target_level) = world
        .dungeon
        .0
        .levels
        .iter()
        .find(|level| {
            level.id.clone().unwrap_or_else(|| level.name.clone()) == session.current_level_id
        })
        .cloned()
        .or_else(|| world.dungeon.0.levels.first().cloned())
    else {
        warn!("load: dungeon has no levels");
        return;
    };

    session.game.player.attack_cooldown = 0.0;
    world.vitals.0.hunger_drain_accumulator = 0.0;
    world.vitals.0.starvation_accumulator = 0.0;

    session.environment = target_level.environment.unwrap_or(Environment::Dungeon);
    session.areas = target_level.areas.clone().unwrap_or_default();
    session.grid = active_layer_grid(&target_level, session.game.active_layer_index);
    replay_destroyed_walls(&session.game, &mut session.grid);
    session.walkable = build_walkable_set(
        target_level
            .char_defs
            .iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );
    let player_col = result.player_col as i32;
    let player_row = result.player_row as i32;
    let player_facing = result.player_facing;
    session.last_player_pose = (player_col, player_row, player_facing);

    let stairs_map = session
        .game
        .active_layer()
        .stairs
        .values()
        .map(|stair| (door_key(stair.col, stair.row), stair.direction))
        .collect();
    let mut loaded_player = Player::new(
        session.grid.clone(),
        player_col,
        player_row,
        player_facing,
        session.walkable.clone(),
        stairs_map,
    );
    loaded_player.snap_y_offset_to_layer(session.game.active_layer_index);
    commands.entity(player_entity).insert(loaded_player);

    let mut scene_assets = SceneAssets {
        meshes: &mut assets.meshes,
        images: &mut assets.images,
        materials: &mut assets.materials,
        asset_server: &assets.asset_server,
    };
    let scene = SceneContext {
        dungeon_materials: &world.dungeon_materials,
        enemy_db: &world.enemy_db.0,
        items: &world.items.0,
        npc_db: &world.npc_db.0,
        game: &session.game,
        level: &target_level,
        walkable: &session.walkable,
    };
    let handles = spawn_level_scene(&mut commands, &mut scene_assets, &scene);
    *world.door_panels = handles.door_panels;
    *world.enemy_billboards = handles.enemy_billboards;
    *world.ground_items = handles.ground_items;
    *world.key_billboards = handles.key_billboards;
    *world.sconce_parts = handles.sconce_parts;
    *world.lever_handles = handles.lever_handles;
    *world.plate_handles = handles.plate_handles;
    *world.tripwire_handles = handles.tripwire_handles;
    *world.chest_handles = handles.chest_handles;
    *world.block_handles = handles.block_handles;
    *world.wall_entity_handles = handles.wall_entity_handles;
    *world.health_bars = handles.health_bars;
    *world.npc_billboards = handles.npc_billboards;
    *world.fountain_handles = handles.fountain_handles;
    *world.pit_floor_handles = handles.pit_floor_handles;
    *world.spawner_handles = handles.spawner_handles;
    *world.boulder_animator = handles.boulder_animator;
    *world.altar_handles = handles.altar_handles;
    *world.barrel_handles = handles.barrel_handles;

    zones::spawn_player_cameras(
        &mut commands,
        player_entity,
        &world.zone_cameras,
        &mut world.clear_color,
        &target_level,
        &handles.level_zones,
    );

    let Session { game, grid, .. } = session;
    game.reveal_around(
        i64::from(player_col),
        i64::from(player_row),
        player_facing,
        grid,
    );
    game.take_events();
}

#[cfg(test)]
mod tests {
    use super::active_layer_grid;
    use delve_core::level_loader::{ValidationContext, validate_dungeon_str};
    use delve_core::types::DungeonLevel;

    fn ruins_forest() -> DungeonLevel {
        let path = crate::assets_dir().join("levels/ruins.json");
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let mut warnings = Vec::new();
        let dungeon = validate_dungeon_str(
            &json,
            "ruins.json",
            &ValidationContext::default(),
            &mut warnings,
        )
        .expect("shipped ruins.json validates");
        dungeon
            .levels
            .into_iter()
            .find(|level| level.id.as_deref() == Some("level_forest"))
            .expect("ruins.json has level_forest")
    }

    /// Ruins' forest is where this bites: the playable surface is layer index
    /// 1, sitting above a basement, so the level-wide convenience grid (a copy
    /// of layer 0) describes the wrong floor entirely.
    #[test]
    fn the_active_layers_grid_wins_over_the_level_wide_copy() {
        let level = ruins_forest();
        assert_eq!(active_layer_grid(&level, 1), level.layers[1].grid);
        assert_ne!(
            active_layer_grid(&level, 1),
            level.grid,
            "layer 1 and the level-wide copy must differ, or this proves nothing"
        );
    }

    #[test]
    fn a_missing_layer_falls_back_to_the_level_wide_grid() {
        let level = ruins_forest();
        assert_eq!(active_layer_grid(&level, 99), level.grid);
    }
}

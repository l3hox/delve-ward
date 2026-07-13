#![forbid(unsafe_code)]

mod altars;
mod attribute_panel;
mod barrels;
mod billboard;
mod blocks;
mod bookshelves;
mod char_creation;
mod chests;
mod damage_numbers;
mod dialog_overlay;
mod doors;
mod dungeon;
mod enemies;
mod enemy_feedback;
mod environment;
mod equip_layout;
mod fountains;
mod ground_items;
mod hud;
mod hud_font;
mod inventory_overlay;
mod item_tooltip;
mod keys;
mod level_scene;
mod levers;
mod mouse;
mod npcs;
mod overlay;
mod pixel_canvas;
mod plates;
mod player;
mod projectiles;
mod quest_log_overlay;
mod save_load_overlay;
mod save_store;
mod sconces;
mod session;
mod signs;
mod stairs;
mod stats_panel;
mod status_effects;
mod textures;
mod torch;
mod trading_overlay;
mod transition;
mod tripwires;
mod wall_entities;

use bevy::input::keyboard::KeyboardInput;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::window::WindowFocused;
use delve_core::enemies::EnemyDatabase;
use delve_core::game_state::{GameState, GameStateDeps, door_key};
use delve_core::grid::build_walkable_set;
use delve_core::items::ItemDatabase;
use delve_core::level_loader::{ValidationContext, resolve_layer_coord, validate_dungeon_str};
use delve_core::loot::LootTables;
use delve_core::npcs::NpcDatabase;
use delve_core::quest_manager::QuestManager;
use delve_core::quests::QuestDef;
use delve_core::random::Mulberry32;
use delve_core::types::{Dungeon, Environment};
use environment::{AMBIENT_BRIGHTNESS, environment_config};
use ground_items::{ItemDb, LootTablesRes};
use level_scene::{SceneAssets, SceneContext, spawn_level_scene};
use overlay::ActiveOverlay;
use player::Player;
use projectiles::{ProjectileBillboards, ProjectileManagerRes};
use save_store::FileSaveStore;
use session::{DungeonRes, GameRng, LevelSnapshots, OriginalGrids, Session};
use std::path::PathBuf;
use std::sync::Arc;
use textures::DungeonMaterials;

/// Same production fallback as the TS shell; override with a level name
/// argument: `delve-game dungeon1`.
const DEFAULT_DUNGEON: &str = "levels/ruins.json";

fn dungeon_path() -> String {
    match std::env::args().nth(1) {
        Some(name) => format!("levels/{name}.json"),
        None => DEFAULT_DUNGEON.to_string(),
    }
}

pub(crate) fn assets_dir() -> PathBuf {
    let local = PathBuf::from("assets");
    if local.is_dir() {
        return local;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn read_asset(relative: &str) -> String {
    let path = assets_dir().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

/// The quest defs the shipped dungeons can reference, matching TS's
/// hardcoded `questManager.loadQuest(...)` calls in `main.ts` — a
/// register-at-startup stand-in for TS's upfront `Promise.all` fetch, per
/// `PHASE4-PLAN.md` section 2.3.
const QUEST_IDS: [&str; 3] = ["fetch_amulet", "kill_spider_queen", "collect_lore"];

fn load_quest_manager() -> QuestManager {
    let mut quests = QuestManager::new();
    for quest_id in QUEST_IDS {
        let json = read_asset(&format!("data/quests/{quest_id}.json"));
        let def = QuestDef::from_json(&json)
            .unwrap_or_else(|error| panic!("failed to parse quest {quest_id}: {error}"));
        quests.register_quest_def(def);
    }
    quests
}

fn load_dungeon(relative: &str) -> Dungeon {
    let enemies =
        EnemyDatabase::from_json(&read_asset("data/enemies.json")).expect("enemies.json loads");
    let npcs = NpcDatabase::from_json(&read_asset("data/npcs.json")).expect("npcs.json loads");
    let enemy_ids = enemies.all_enemy_ids();
    let npc_ids = npcs.all_npc_ids();
    let ctx = ValidationContext {
        enemy_ids: Some(&enemy_ids),
        npc_ids: Some(&npc_ids),
    };
    let mut warnings = Vec::new();
    let dungeon = validate_dungeon_str(&read_asset(relative), relative, &ctx, &mut warnings)
        .unwrap_or_else(|error| panic!("failed to load dungeon {relative}: {error}"));
    for warning in warnings {
        warn!("{warning}");
    }
    dungeon
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let loaded = load_dungeon(&dungeon_path());
    // Captured before any gameplay mutation (breakable walls, secret walls,
    // save/load grid restores) so restart and load can reset a level back
    // to exactly what shipped on disk.
    commands.insert_resource(OriginalGrids(
        loaded
            .levels
            .iter()
            .map(|level| {
                (
                    level.id.clone().unwrap_or_else(|| level.name.clone()),
                    level.grid.clone(),
                )
            })
            .collect(),
    ));
    let start = loaded.player_start.clone();
    let level = loaded
        .levels
        .iter()
        .find(|level| level.id.as_deref() == Some(start.level_id.as_str()))
        .expect("playerStart.levelId resolves to a level")
        .clone();
    let layer_index = resolve_layer_coord(&level, start.layer_index.unwrap_or(0));
    let grid = level.layers[layer_index].grid.clone();

    let materials = DungeonMaterials::generate(&mut images, &mut standard_materials);

    let walkable = build_walkable_set(
        level
            .char_defs
            .iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );

    // Game state with the shipped databases as dependencies.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0x5EED);
    let mut rng = Mulberry32::new(seed);
    let items = Arc::new(
        ItemDatabase::from_json(&read_asset("data/items.json")).expect("items.json loads"),
    );
    let deps = GameStateDeps {
        items: Some(items.clone()),
        enemy_registrar: Some(Box::new(
            EnemyDatabase::from_json(&read_asset("data/enemies.json")).expect("enemies.json loads"),
        )),
        npc_registrar: Some(Box::new(
            NpcDatabase::from_json(&read_asset("data/npcs.json")).expect("npcs.json loads"),
        )),
    };
    let mut random = || rng.next_f64();
    let mut game = GameState::new(
        &[],
        Some(&grid),
        &start.level_id,
        Some(&level.layers),
        deps,
        &mut random,
    );
    game.active_layer_index = layer_index;
    game.take_events(); // discard construction-time signal events

    let enemy_db = Arc::new(
        EnemyDatabase::from_json(&read_asset("data/enemies.json")).expect("enemies.json loads"),
    );
    let npc_db =
        Arc::new(NpcDatabase::from_json(&read_asset("data/npcs.json")).expect("npcs.json loads"));
    let mut scene_assets = SceneAssets {
        meshes: &mut meshes,
        images: &mut images,
        materials: &mut standard_materials,
        asset_server: &asset_server,
    };
    let scene = SceneContext {
        dungeon_materials: &materials,
        enemy_db: &enemy_db,
        items: &items,
        npc_db: &npc_db,
        game: &game,
        level: &level,
        grid: &grid,
        walkable: &walkable,
    };
    let handles = spawn_level_scene(&mut commands, &mut scene_assets, &scene);
    commands.insert_resource(handles.door_panels);
    commands.insert_resource(handles.enemy_billboards);
    commands.insert_resource(handles.ground_items);
    commands.insert_resource(handles.key_billboards);
    commands.insert_resource(handles.sconce_parts);
    commands.insert_resource(handles.lever_handles);
    commands.insert_resource(handles.plate_handles);
    commands.insert_resource(handles.tripwire_handles);
    commands.insert_resource(handles.chest_handles);
    commands.insert_resource(handles.block_handles);
    commands.insert_resource(handles.wall_entity_handles);
    commands.insert_resource(handles.health_bars);
    commands.insert_resource(handles.npc_billboards);
    commands.insert_resource(handles.fountain_handles);
    commands.insert_resource(handles.altar_handles);
    commands.insert_resource(handles.barrel_handles);
    commands.insert_resource(handles.pit_floor_handles);
    commands.insert_resource(enemies::EnemyDb(enemy_db));
    commands.insert_resource(npcs::NpcDb(npc_db));
    commands.insert_resource(ItemDb(items));
    commands.insert_resource(LootTablesRes(
        LootTables::from_json(&read_asset("data/loot-tables.json"))
            .expect("loot-tables.json loads"),
    ));
    commands.insert_resource(materials);
    commands.insert_resource(dialog_overlay::QuestManagerRes(load_quest_manager()));
    commands.insert_resource(dialog_overlay::DialogTreeCache::default());
    commands.insert_resource(dialog_overlay::DialogOverlayState::default());

    let stairs_map = game
        .active_layer()
        .stairs
        .values()
        .map(|stair| (door_key(stair.col, stair.row), stair.direction))
        .collect();
    let level_id = level.id.clone().unwrap_or_else(|| level.name.clone());
    commands.insert_resource(Session::new(
        game,
        grid.clone(),
        walkable.clone(),
        level_id,
        &level,
        (start.col, start.row, start.facing),
    ));
    commands.insert_resource(GameRng(rng));
    commands.insert_resource(ProjectileManagerRes::default());
    commands.insert_resource(ProjectileBillboards::default());

    let config = environment_config(level.environment.unwrap_or(Environment::Dungeon));
    commands.insert_resource(ClearColor(config.fog_color));
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 75_f32.to_radians(),
            near: 0.1,
            far: 200.0,
            ..default()
        }),
        Transform::default(),
        DistanceFog {
            color: config.fog_color,
            falloff: FogFalloff::Linear {
                start: config.fog_near,
                end: config.fog_far,
            },
            ..default()
        },
        AmbientLight {
            color: config.ambient_color,
            brightness: AMBIENT_BRIGHTNESS,
            affects_lightmapped_meshes: true,
        },
        Player::new(
            grid,
            start.col,
            start.row,
            start.facing,
            walkable,
            stairs_map,
        ),
    ));

    torch::spawn_torch(&mut commands);
    commands.insert_resource(DungeonRes(loaded));
}

/// Logs OS-confirmed focus changes (info) and raw key events (debug, enable
/// with `RUST_LOG=delve_game=debug`) — kept while macOS focus behavior
/// settles across OS versions.
fn input_diagnostics(
    mut keyboard: MessageReader<KeyboardInput>,
    mut focus_events: MessageReader<WindowFocused>,
) {
    for event in keyboard.read() {
        debug!("keyboard event: {:?} {:?}", event.key_code, event.state);
    }
    for event in focus_events.read() {
        info!("window focus (os): {}", event.focused);
    }
}

/// On macOS the window can come up without keyboard focus when the binary is
/// launched from a terminal (observed on macOS 26); keypresses then fall
/// through to the system and beep. During a short launch grace period this
/// re-asserts focus: a rising edge on `Window::focused` makes winit activate
/// the app and make the window key. Stops as soon as the OS confirms focus,
/// so normal app-switching is unaffected.
fn claim_initial_focus(
    time: Res<Time>,
    mut windows: Query<&mut Window>,
    mut focus_events: MessageReader<WindowFocused>,
    mut settled: Local<bool>,
    mut attempts: Local<u32>,
) {
    const FOCUS_GRACE_SECONDS: f32 = 2.0;
    const ATTEMPT_INTERVAL_SECONDS: f32 = 0.5;

    if *settled {
        return;
    }
    if focus_events.read().any(|event| event.focused) {
        *settled = true;
        return;
    }
    let elapsed = time.elapsed_secs();
    if elapsed > FOCUS_GRACE_SECONDS {
        *settled = true;
        return;
    }
    let due_attempts = (elapsed / ATTEMPT_INTERVAL_SECONDS) as u32 + 1;
    if *attempts >= due_attempts {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if window.focused && *attempts > 0 {
        // The component still carries our own earlier write; drop it so the
        // next attempt is a rising edge that re-triggers the winit sync.
        window.focused = false;
        return;
    }
    *attempts += 1;
    window.focused = true;
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "DelveWard".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .set(ImagePlugin::default_nearest())
                .set(AssetPlugin {
                    // Sprites live in the repo assets dir, not next to the
                    // executable where Bevy looks by default.
                    file_path: std::fs::canonicalize(assets_dir())
                        .unwrap_or_else(|_| assets_dir())
                        .to_string_lossy()
                        .into_owned(),
                    ..Default::default()
                }),
        )
        .init_resource::<transition::Transition>()
        .init_resource::<LevelSnapshots>()
        .init_resource::<sconces::SconceFlicker>()
        .init_resource::<char_creation::CharCreation>()
        .init_resource::<session::BlockedDoors>()
        .init_resource::<status_effects::PlayerVitals>()
        .init_resource::<save_load_overlay::SaveLoadOverlay>()
        .insert_resource(FileSaveStore::new(save_store::saves_dir()))
        // The game boots straight into character creation, matching TS
        // showing that screen before the level loads — never re-entered
        // afterward, so this is an explicit initial value, not a `Default`.
        .insert_resource(ActiveOverlay::CharCreation)
        .init_resource::<mouse::MouseState>()
        .init_resource::<hud::MiniPanelState>()
        .init_resource::<inventory_overlay::InventoryOverlayState>()
        .init_resource::<attribute_panel::AttributePanelState>()
        .init_resource::<trading_overlay::TradingOverlayState>()
        .add_systems(Startup, (setup, transition::spawn_overlay, hud::setup_hud))
        .add_systems(
            Update,
            (
                // Split across two tuples (Bevy's `.chain()` tuple impl tops
                // out at 20 elements) but chained together at the outer
                // level below, so ordering is identical to one long chain.
                (
                    mouse::track_mouse,
                    char_creation::char_creation_input,
                    save_load_overlay::save_load_input,
                    dialog_overlay::dialog_input,
                    trading_overlay::trading_overlay_input,
                    quest_log_overlay::quest_log_input,
                    inventory_overlay::inventory_overlay_input,
                    attribute_panel::attribute_panel_input,
                    stats_panel::stats_panel_input,
                    hud::mini_panel_input,
                    session::player_input,
                    session::interact_input,
                    session::quick_slot_input,
                    enemies::attack_input,
                    session::player_update,
                    session::on_player_moved,
                    enemies::tick_enemies,
                    enemies::tick_attack_cooldown,
                    session::tick_game,
                    projectiles::tick_projectiles,
                )
                    .chain(),
                (
                    projectiles::position_projectile_meshes,
                    projectiles::update_fireball_explosions,
                    status_effects::tick_player_vitals,
                    save_load_overlay::check_player_death,
                    status_effects::apply_slow_multiplier,
                    status_effects::tint_enemy_status_effects,
                    billboard::face_billboards,
                    doors::animate_door_panels,
                    levers::animate_levers,
                    torch::torch_update,
                    sconces::sconce_flicker,
                )
                    .chain(),
                (
                    chests::animate_chest_lids,
                    blocks::animate_blocks,
                    enemy_feedback::tick_enemy_hit_shake,
                    damage_numbers::update_damage_numbers,
                    hud::draw_hud,
                    transition::tick_transition,
                    transition::perform_level_swap,
                    transition::perform_restart,
                    transition::perform_load,
                    input_diagnostics,
                    claim_initial_focus,
                )
                    .chain(),
            )
                .chain(),
        )
        .run();
}

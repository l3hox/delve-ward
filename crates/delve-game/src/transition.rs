//! Cross-level stair transitions, ported from the TS transition system and
//! its fade-to-black overlay: fade out, swap the level scene at the
//! midpoint, fade back in. Input systems check `Transition::is_active`.

use crate::doors::DoorPanels;
use crate::enemies::{EnemyBillboards, EnemyDb};
use crate::environment::{AMBIENT_BRIGHTNESS, environment_config};
use crate::ground_items::{GroundItemBillboards, ItemDb};
use crate::keys::KeyBillboards;
use crate::level_scene::{LevelEntity, SceneAssets, SceneContext, spawn_level_scene};
use crate::player::Player;
use crate::sconces::SconceParts;
use crate::session::{DungeonRes, GameRng, LevelSnapshots, Session};
use crate::textures::DungeonMaterials;
use bevy::ecs::system::SystemParam;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use delve_core::game_state::door_key;
use delve_core::grid::build_walkable_set;
use delve_core::level_loader::{find_entity_layer_index, get_all_level_entities};
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

#[derive(Resource, Default)]
pub struct Transition {
    phase: Phase,
    opacity: f32,
    target_stair_id: Option<String>,
}

impl Transition {
    /// Start a transition toward the stair with `stair_id` as its target.
    /// Ignored while another transition is in progress.
    pub fn begin(&mut self, stair_id: String) {
        if self.phase != Phase::Idle {
            return;
        }
        self.phase = Phase::FadeOut;
        self.target_stair_id = Some(stair_id);
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
    dungeon: Res<'w, DungeonRes>,
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
    mut players: Query<(Entity, &Player, &mut DistanceFog, &mut AmbientLight)>,
) {
    if transition.phase != Phase::Swap {
        return;
    }
    transition.phase = Phase::FadeIn;
    let Some(stair_id) = transition.target_stair_id.take() else {
        return;
    };
    let Ok((player_entity, player, mut fog, mut ambient)) = players.single_mut() else {
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

    let session = &mut *world.session;
    world.snapshots.0.insert(
        session.current_level_id.clone(),
        session.game.save_level_state(),
    );
    for entity in &world.level_entities {
        commands.entity(entity).despawn();
    }

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
    session.grid = target_level.layers[target_layer_index].grid.clone();
    session.walkable = build_walkable_set(
        target_level
            .char_defs
            .iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );
    session.last_player_pose = (spawn_col, spawn_row, facing_before);

    let config = environment_config(target_level.environment.unwrap_or(Environment::Dungeon));
    world.clear_color.0 = config.fog_color;
    fog.color = config.fog_color;
    fog.falloff = FogFalloff::Linear {
        start: config.fog_near,
        end: config.fog_far,
    };
    ambient.color = config.ambient_color;
    ambient.brightness = AMBIENT_BRIGHTNESS;

    let stairs_map = session
        .game
        .active_layer()
        .stairs
        .values()
        .map(|stair| (door_key(stair.col, stair.row), stair.direction))
        .collect();
    commands.entity(player_entity).insert(Player::new(
        session.grid.clone(),
        spawn_col,
        spawn_row,
        facing_before,
        session.walkable.clone(),
        stairs_map,
    ));

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
        game: &session.game,
        level: target_level,
        grid: &session.grid,
        walkable: &session.walkable,
    };
    let handles = spawn_level_scene(&mut commands, &mut scene_assets, &scene);
    *world.door_panels = handles.door_panels;
    *world.enemy_billboards = handles.enemy_billboards;
    *world.ground_items = handles.ground_items;
    *world.key_billboards = handles.key_billboards;
    *world.sconce_parts = handles.sconce_parts;

    let Session { game, grid, .. } = session;
    game.reveal_around(
        i64::from(spawn_col),
        i64::from(spawn_row),
        facing_before,
        grid,
    );
    game.take_events(); // discard construction-time signal events
}

#![forbid(unsafe_code)]

mod dungeon;
mod environment;
mod pixel_canvas;
mod player;
mod textures;
mod torch;

use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use delve_core::enemies::EnemyDatabase;
use delve_core::grid::build_walkable_set;
use delve_core::level_loader::{ValidationContext, resolve_layer_coord, validate_dungeon_str};
use delve_core::npcs::NpcDatabase;
use delve_core::types::{Dungeon, Environment};
use environment::{AMBIENT_BRIGHTNESS, environment_config};
use player::Player;
use std::path::PathBuf;
use textures::DungeonMaterials;

const SMOKE_DUNGEON: &str = "levels/dungeon1.json";

fn assets_dir() -> PathBuf {
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
) {
    let loaded = load_dungeon(SMOKE_DUNGEON);
    let start = &loaded.player_start;
    let level = loaded
        .levels
        .iter()
        .find(|level| level.id.as_deref() == Some(start.level_id.as_str()))
        .expect("playerStart.levelId resolves to a level");
    let layer_index = resolve_layer_coord(level, start.layer_index.unwrap_or(0));
    let grid = level.layers[layer_index].grid.clone();

    let materials = DungeonMaterials::generate(&mut images, &mut standard_materials);
    dungeon::spawn_dungeon(&mut commands, &mut meshes, &materials, level);
    commands.insert_resource(materials);

    let walkable = build_walkable_set(
        level
            .char_defs
            .iter()
            .flatten()
            .map(|def| (def.character, def.solid)),
    );

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
        Player::new(grid, start.col, start.row, start.facing, walkable),
    ));

    torch::spawn_torch(&mut commands);
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "DelveWard".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                player::player_input,
                player::player_update,
                torch::torch_update,
            )
                .chain(),
        )
        .run();
}

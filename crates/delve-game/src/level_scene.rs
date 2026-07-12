//! Building and tearing down the per-level scene: everything that is
//! despawned and rebuilt when the player takes stairs to another level.

use crate::doors::{self, DoorPanels};
use crate::dungeon;
use crate::enemies::{self, EnemyBillboards};
use crate::stairs;
use crate::textures::DungeonMaterials;
use bevy::prelude::*;
use delve_core::enemies::EnemyDatabase;
use delve_core::game_state::GameState;
use delve_core::types::DungeonLevel;
use std::collections::HashSet;

/// Marker on every root entity belonging to the current level's scene.
#[derive(Component)]
pub struct LevelEntity;

/// Mutable engine-side stores the scene spawn writes into.
pub struct SceneAssets<'a> {
    pub meshes: &'a mut Assets<Mesh>,
    pub materials: &'a mut Assets<StandardMaterial>,
    pub asset_server: &'a AssetServer,
}

/// Read-only level data the scene spawn reads from.
pub struct SceneContext<'a> {
    pub dungeon_materials: &'a DungeonMaterials,
    pub enemy_db: &'a EnemyDatabase,
    pub game: &'a GameState,
    pub level: &'a DungeonLevel,
    pub grid: &'a [String],
    pub walkable: &'a HashSet<char>,
}

pub fn spawn_level_scene(
    commands: &mut Commands,
    assets: &mut SceneAssets,
    scene: &SceneContext,
) -> (DoorPanels, EnemyBillboards) {
    dungeon::spawn_dungeon(
        commands,
        assets.meshes,
        scene.dungeon_materials,
        scene.level,
    );
    stairs::spawn_stairs(
        commands,
        assets.meshes,
        scene.dungeon_materials,
        scene.game,
        scene.level,
        scene.grid,
        scene.walkable,
    );
    let door_panels = doors::spawn_doors(
        commands,
        assets.meshes,
        scene.dungeon_materials,
        scene.game,
        scene.grid,
        scene.walkable,
    );
    let billboards = enemies::spawn_enemy_billboards(
        commands,
        assets.meshes,
        assets.materials,
        assets.asset_server,
        scene.game,
        scene.enemy_db,
    );
    (door_panels, billboards)
}

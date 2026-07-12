//! Building and tearing down the per-level scene: everything that is
//! despawned and rebuilt when the player takes stairs to another level.

use crate::doors::{self, DoorPanels};
use crate::dungeon;
use crate::enemies::{self, EnemyBillboards};
use crate::ground_items::{self, GroundItemBillboards};
use crate::keys::{self, KeyBillboards};
use crate::levers::{self, LeverHandles};
use crate::plates::{self, PlateHandles};
use crate::sconces::{self, SconceParts};
use crate::stairs;
use crate::textures::DungeonMaterials;
use crate::tripwires::{self, TripwireHandles};
use bevy::prelude::*;
use delve_core::enemies::EnemyDatabase;
use delve_core::game_state::GameState;
use delve_core::items::ItemDatabase;
use delve_core::types::DungeonLevel;
use std::collections::HashSet;

/// Marker on every root entity belonging to the current level's scene.
#[derive(Component)]
pub struct LevelEntity;

/// Mutable engine-side stores the scene spawn writes into.
pub struct SceneAssets<'a> {
    pub meshes: &'a mut Assets<Mesh>,
    pub images: &'a mut Assets<Image>,
    pub materials: &'a mut Assets<StandardMaterial>,
    pub asset_server: &'a AssetServer,
}

/// Entity lookup maps produced by the scene spawn, replaced as resources on
/// every level swap.
pub struct LevelSceneHandles {
    pub door_panels: DoorPanels,
    pub enemy_billboards: EnemyBillboards,
    pub ground_items: GroundItemBillboards,
    pub key_billboards: KeyBillboards,
    pub sconce_parts: SconceParts,
    pub lever_handles: LeverHandles,
    pub plate_handles: PlateHandles,
    pub tripwire_handles: TripwireHandles,
}

/// Read-only level data the scene spawn reads from.
pub struct SceneContext<'a> {
    pub dungeon_materials: &'a DungeonMaterials,
    pub enemy_db: &'a EnemyDatabase,
    pub items: &'a ItemDatabase,
    pub game: &'a GameState,
    pub level: &'a DungeonLevel,
    pub grid: &'a [String],
    pub walkable: &'a HashSet<char>,
}

pub fn spawn_level_scene(
    commands: &mut Commands,
    assets: &mut SceneAssets,
    scene: &SceneContext,
) -> LevelSceneHandles {
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
    let ground_items = ground_items::spawn_ground_items(
        commands,
        assets.meshes,
        assets.materials,
        assets.asset_server,
        scene.game,
        scene.items,
        i32::try_from(scene.game.active_layer_index).unwrap_or(0),
    );
    let key_billboards = keys::spawn_keys(
        commands,
        assets.meshes,
        assets.images,
        assets.materials,
        scene.game,
    );
    let sconce_parts =
        sconces::spawn_sconces(commands, assets.meshes, assets.materials, scene.game);
    let lever_handles = levers::spawn_levers(commands, assets.meshes, assets.materials, scene.game);
    let plate_handles = plates::spawn_plates(
        commands,
        assets.meshes,
        assets.images,
        assets.materials,
        scene.game,
    );
    let tripwire_handles =
        tripwires::spawn_tripwires(commands, assets.meshes, assets.materials, scene.game);
    LevelSceneHandles {
        door_panels,
        enemy_billboards: billboards,
        ground_items,
        key_billboards,
        sconce_parts,
        lever_handles,
        plate_handles,
        tripwire_handles,
    }
}

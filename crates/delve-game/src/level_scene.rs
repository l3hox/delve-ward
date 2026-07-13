//! Building and tearing down the per-level scene: everything that is
//! despawned and rebuilt when the player takes stairs to another level.

use crate::altars::{self, AltarHandles};
use crate::barrels::{self, BarrelHandles};
use crate::blocks::{self, BlockHandles};
use crate::bookshelves;
use crate::chests::{self, ChestHandles};
use crate::doors::{self, DoorPanels};
use crate::dungeon;
use crate::enemies::{self, EnemyBillboards};
use crate::enemy_feedback::{self, EnemyHealthBars};
use crate::fountains::{self, FountainHandles};
use crate::ground_items::{self, GroundItemBillboards};
use crate::keys::{self, KeyBillboards};
use crate::levers::{self, LeverHandles};
use crate::npcs::{self, NpcBillboards};
use crate::plates::{self, PlateHandles};
use crate::sconces::{self, SconceParts};
use crate::signs;
use crate::stairs;
use crate::textures::DungeonMaterials;
use crate::tripwires::{self, TripwireHandles};
use crate::wall_entities::{self, WallEntityHandles};
use bevy::prelude::*;
use delve_core::enemies::EnemyDatabase;
use delve_core::game_state::{GameState, door_key};
use delve_core::items::ItemDatabase;
use delve_core::npcs::NpcDatabase;
use delve_core::types::DungeonLevel;
use std::collections::{HashMap, HashSet};

/// Every layer's default Y offset, absent an explicit `LayerDef.y_offset`
/// override — layers stack flush, matching TS's `li * LAYER_HEIGHT`.
fn layer_y_offset(level: &DungeonLevel, layer_index: usize) -> f32 {
    level.layers[layer_index]
        .y_offset
        .unwrap_or(layer_index as f64 * f64::from(dungeon::LAYER_HEIGHT)) as f32
}

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
    pub chest_handles: ChestHandles,
    pub block_handles: BlockHandles,
    pub wall_entity_handles: WallEntityHandles,
    pub health_bars: EnemyHealthBars,
    pub npc_billboards: NpcBillboards,
    pub fountain_handles: FountainHandles,
    pub altar_handles: AltarHandles,
    pub barrel_handles: BarrelHandles,
    pub pit_floor_handles: dungeon::PitFloorHandles,
}

/// Read-only level data the scene spawn reads from.
pub struct SceneContext<'a> {
    pub dungeon_materials: &'a DungeonMaterials,
    pub enemy_db: &'a EnemyDatabase,
    pub items: &'a ItemDatabase,
    pub npc_db: &'a NpcDatabase,
    pub game: &'a GameState,
    pub level: &'a DungeonLevel,
    pub walkable: &'a HashSet<char>,
}

pub fn spawn_level_scene(
    commands: &mut Commands,
    assets: &mut SceneAssets,
    scene: &SceneContext,
) -> LevelSceneHandles {
    let mut door_panels = DoorPanels::default();
    let mut enemy_billboards = EnemyBillboards::default();
    let mut ground_items = GroundItemBillboards::default();
    let mut key_billboards = KeyBillboards::default();
    let mut sconce_parts = SconceParts::default();
    let mut lever_handles = LeverHandles::default();
    let mut plate_handles = PlateHandles::default();
    let mut tripwire_handles = TripwireHandles::default();
    let mut pit_floor_handles = dungeon::PitFloorHandles::default();
    let mut health_bars = EnemyHealthBars::default();
    let mut chest_handles = ChestHandles::default();
    let mut block_handles = BlockHandles::default();
    let mut wall_entity_handles = WallEntityHandles::default();
    let mut npc_billboards = NpcBillboards::default();
    let mut fountain_handles = FountainHandles::default();
    let mut altar_handles = AltarHandles::default();
    let mut barrel_handles = BarrelHandles::default();

    // TS builds every layer's geometry and entity meshes simultaneously at
    // load time, each Y-offset by `layer_index * LAYER_HEIGHT` (or an
    // explicit override) — not lazily for only the active layer. See
    // `PHASE5-PLAN.md` section 1.
    for layer_index in 0..scene.level.layers.len() {
        let Some(layer) = scene.game.layer(layer_index) else {
            continue;
        };
        let layer_def = &scene.level.layers[layer_index];
        let y_offset = layer_y_offset(scene.level, layer_index);
        let layer_spawn = dungeon::LayerSpawn {
            level: scene.level,
            layer_def,
            index: layer_index,
            y_offset,
        };

        let stair_cells: HashSet<String> = layer
            .stairs
            .values()
            .map(|stair| door_key(stair.col, stair.row))
            .collect();
        // Breakable/secret walls own their floor/ceiling/walls via
        // `wall_entities::spawn_wall_entities` below, on every layer —
        // excluded here so `spawn_dungeon`'s normal per-cell pass doesn't
        // double-spawn geometry underneath them.
        let wall_entity_cells: HashMap<String, (i64, i64)> = layer
            .breakable_walls
            .values()
            .map(|wall| (door_key(wall.col, wall.row), (wall.col, wall.row)))
            .chain(
                layer
                    .secret_walls
                    .values()
                    .map(|wall| (door_key(wall.col, wall.row), (wall.col, wall.row))),
            )
            .collect();
        let wall_entity_keys: HashSet<String> = wall_entity_cells.keys().cloned().collect();

        let pit_trap_cells: HashSet<String> = layer
            .pit_traps
            .values()
            .map(|pit| door_key(pit.col, pit.row))
            .collect();

        dungeon::spawn_dungeon(
            commands,
            assets.meshes,
            scene.dungeon_materials,
            &layer_spawn,
            &stair_cells,
            &wall_entity_keys,
            &pit_trap_cells,
        );
        let layer_pit_floors = dungeon::spawn_pit_floors(
            commands,
            assets.meshes,
            scene.dungeon_materials,
            &layer_spawn,
            layer,
        );
        pit_floor_handles.by_key.extend(layer_pit_floors.by_key);
        stairs::spawn_stairs(
            commands,
            assets.meshes,
            scene.dungeon_materials,
            layer,
            &layer_spawn,
            &layer_def.grid,
            scene.walkable,
        );
        let layer_door_panels = doors::spawn_doors(
            commands,
            assets.meshes,
            scene.dungeon_materials,
            layer,
            &layer_spawn,
            &layer_def.grid,
            scene.walkable,
        );
        door_panels.by_key.extend(layer_door_panels.by_key);

        let layer_billboards = enemies::spawn_enemy_billboards(
            commands,
            assets.meshes,
            assets.materials,
            assets.asset_server,
            layer,
            &layer_spawn,
            scene.enemy_db,
        );
        // Needs `layer_billboards` before it's merged into the accumulator —
        // each health bar looks up its parent billboard by the same
        // layer-prefixed key this layer's spawn just produced.
        let layer_health_bars = enemy_feedback::spawn_health_bars(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            &layer_spawn,
            &layer_billboards,
            scene.enemy_db,
        );
        health_bars.extend(layer_health_bars);
        enemy_billboards.by_key.extend(layer_billboards.by_key);

        let layer_ground_items = ground_items::spawn_ground_items(
            commands,
            assets.meshes,
            assets.materials,
            assets.asset_server,
            scene.game,
            scene.items,
            &layer_spawn,
        );
        ground_items.equipment.extend(layer_ground_items.equipment);
        ground_items
            .consumables
            .extend(layer_ground_items.consumables);

        let layer_keys = keys::spawn_keys(
            commands,
            assets.meshes,
            assets.images,
            assets.materials,
            layer,
            layer_index,
            y_offset,
        );
        key_billboards.by_key.extend(layer_keys.by_key);

        let layer_sconces = sconces::spawn_sconces(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            layer_index,
            y_offset,
        );
        sconce_parts.torches.extend(layer_sconces.torches);
        sconce_parts.lights.extend(layer_sconces.lights);

        let layer_levers = levers::spawn_levers(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            layer_index,
            y_offset,
        );
        lever_handles.by_key.extend(layer_levers.by_key);

        let layer_plates = plates::spawn_plates(
            commands,
            assets.meshes,
            assets.images,
            assets.materials,
            layer,
            layer_index,
            y_offset,
        );
        // `spawn_plates` returns default (invalid) material handles when the
        // layer has no plates — only adopt a layer's materials when it
        // actually generated them.
        if !layer_plates.by_key.is_empty() {
            plate_handles.normal_material = layer_plates.normal_material;
            plate_handles.pressed_material = layer_plates.pressed_material;
        }
        plate_handles.by_key.extend(layer_plates.by_key);

        let layer_tripwires = tripwires::spawn_tripwires(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            layer_index,
            y_offset,
        );
        tripwire_handles.by_key.extend(layer_tripwires.by_key);

        let layer_wall_entities = wall_entities::spawn_wall_entities(
            commands,
            assets.meshes,
            scene.dungeon_materials,
            &layer_spawn,
            &layer_def.grid,
            &wall_entity_cells,
        );
        wall_entity_handles.extend(layer_wall_entities);

        let layer_chests = chests::spawn_chests(
            commands,
            assets.meshes,
            assets.images,
            assets.materials,
            layer,
            &layer_spawn,
        );
        chest_handles.by_key.extend(layer_chests.by_key);

        let layer_blocks = blocks::spawn_blocks(
            commands,
            assets.meshes,
            assets.images,
            assets.materials,
            layer,
            &layer_spawn,
        );
        block_handles.by_key.extend(layer_blocks.by_key);

        signs::spawn_signs(
            commands,
            assets.meshes,
            assets.images,
            assets.materials,
            layer,
            &layer_spawn,
        );

        let layer_npcs = npcs::spawn_npc_billboards(
            commands,
            assets.meshes,
            assets.materials,
            assets.asset_server,
            layer,
            &layer_spawn,
            scene.npc_db,
        );
        npc_billboards.by_key.extend(layer_npcs.by_key);

        let layer_fountains = fountains::spawn_fountains(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            &layer_spawn,
        );
        fountain_handles.extend(layer_fountains);

        let layer_altars = altars::spawn_altars(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            &layer_spawn,
        );
        altar_handles.extend(layer_altars);

        let layer_barrels = barrels::spawn_barrels(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            &layer_spawn,
        );
        barrel_handles.extend(layer_barrels);

        bookshelves::spawn_bookshelves(
            commands,
            assets.meshes,
            assets.materials,
            layer,
            &layer_spawn,
        );
    }

    LevelSceneHandles {
        door_panels,
        enemy_billboards,
        ground_items,
        key_billboards,
        sconce_parts,
        lever_handles,
        plate_handles,
        tripwire_handles,
        chest_handles,
        block_handles,
        wall_entity_handles,
        health_bars,
        npc_billboards,
        fountain_handles,
        altar_handles,
        barrel_handles,
        pit_floor_handles,
    }
}

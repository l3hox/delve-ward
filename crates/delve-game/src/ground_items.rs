//! Ground item billboards, ported from the TS groundItemRenderer and
//! lootSpawner: equipment and consumables render as camera-facing sprites
//! with seeded spread offsets within a cell; walking over a cell picks
//! items up, and kills roll loot onto the ground.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::entities::{ItemEntity, ItemLocation};
use delve_core::game_state::{GameState, door_key, layer_door_key};
use delve_core::items::{ItemDatabase, ItemType};
use delve_core::loot::{DropsOverride, LootTables};
use delve_core::random::Mulberry32;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::Arc;

/// Equipment and consumables render at slightly different sizes for visual
/// distinction.
const EQUIPMENT_SIZE: f32 = 0.4;
const CONSUMABLE_SIZE: f32 = 0.35;
const SPREAD_RADIUS: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Equipment,
    Consumable,
}

impl ItemKind {
    pub(crate) fn of(item_type: ItemType) -> Self {
        if item_type == ItemType::Consumable {
            ItemKind::Consumable
        } else {
            ItemKind::Equipment
        }
    }

    fn size(self) -> f32 {
        match self {
            ItemKind::Equipment => EQUIPMENT_SIZE,
            ItemKind::Consumable => CONSUMABLE_SIZE,
        }
    }

    fn height(self) -> f32 {
        self.size() / 2.0 + 0.02
    }
}

/// Shared item definitions for icon and type lookups (same `Arc` as the
/// game state's dependency).
#[derive(Resource)]
pub struct ItemDb(pub Arc<ItemDatabase>);

/// Loot tables for kill drops.
#[derive(Resource)]
pub struct LootTablesRes(pub LootTables);

/// Billboard entities by cell key (`door_key` or `door_key#index` for
/// multi-item spreads), split by kind like the TS mesh maps.
#[derive(Resource, Default)]
pub struct GroundItemBillboards {
    pub equipment: HashMap<String, Entity>,
    pub consumables: HashMap<String, Entity>,
}

impl GroundItemBillboards {
    fn map_mut(&mut self, kind: ItemKind) -> &mut HashMap<String, Entity> {
        match kind {
            ItemKind::Equipment => &mut self.equipment,
            ItemKind::Consumable => &mut self.consumables,
        }
    }
}

/// Rendering-side handles for spawning and despawning item billboards
/// outside the level-scene build.
#[derive(SystemParam)]
pub struct GroundItemRender<'w, 's> {
    pub billboards: ResMut<'w, GroundItemBillboards>,
    pub commands: Commands<'w, 's>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub asset_server: Res<'w, AssetServer>,
    pub items: Res<'w, ItemDb>,
}

/// Seeded random offset for an item's index within a cell; the first item
/// sits at the center.
fn item_offset(col: i32, row: i32, index: usize) -> (f32, f32) {
    if index == 0 {
        return (0.0, 0.0);
    }
    let seed = i64::from(col) * 7919 + i64::from(row) * 6271 + index as i64 * 3037;
    let mut rng = Mulberry32::new(seed as u32);
    let angle = rng.next_f64() * PI * 2.0;
    let dist = SPREAD_RADIUS * (0.4 + rng.next_f64() * 0.6);
    ((angle.cos() * dist) as f32, (angle.sin() * dist) as f32)
}

fn billboard_material(
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    icon: &str,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color_texture: Some(asset_server.load(format!("sprites/items/{icon}.png"))),
        perceptual_roughness: 1.0,
        metallic: 0.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Mask(0.5),
        double_sided: true,
        cull_mode: None,
        ..default()
    })
}

struct ItemPlacement<'a> {
    kind: ItemKind,
    icon: &'a str,
    col: i32,
    row: i32,
    index: usize,
    layer_y_offset: f32,
}

fn spawn_item_mesh(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    placement: &ItemPlacement,
) -> Entity {
    let size = placement.kind.size();
    let (dx, dz) = item_offset(placement.col, placement.row, placement.index);
    let center_x = placement.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
    let center_z = placement.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
    commands
        .spawn((
            LevelEntity,
            crate::billboard::FacesCamera,
            Mesh3d(meshes.add(Rectangle::new(size, size))),
            MeshMaterial3d(billboard_material(materials, asset_server, placement.icon)),
            Transform::from_xyz(
                center_x + dx,
                placement.kind.height() + placement.layer_y_offset,
                center_z + dz,
            ),
        ))
        .id()
}

fn world_cell(entity: &ItemEntity) -> Option<(i32, i32)> {
    match &entity.location {
        ItemLocation::World { col, row, .. } => Some((*col, *row)),
        _ => None,
    }
}

/// Build billboards for every ground item on the active level, grouped by
/// cell for spread offsets. Part of the level-scene build.
pub fn spawn_ground_items(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    game: &GameState,
    items: &ItemDatabase,
    layer_spawn: &crate::dungeon::LayerSpawn,
) -> GroundItemBillboards {
    let mut billboards = GroundItemBillboards::default();
    let layer_index = i32::try_from(layer_spawn.index).unwrap_or(0);
    let layer_y_offset = layer_spawn.y_offset;

    for kind in [ItemKind::Equipment, ItemKind::Consumable] {
        let mut by_cell: HashMap<String, Vec<&ItemEntity>> = HashMap::new();
        for entity in game
            .entity_registry
            .all_ground_items_for_level(&game.current_level_id, Some(layer_index))
        {
            let Some(def) = items.get_item(&entity.item_id) else {
                continue;
            };
            if ItemKind::of(def.item_type) != kind {
                continue;
            }
            let Some((col, row)) = world_cell(entity) else {
                continue;
            };
            by_cell
                .entry(door_key(i64::from(col), i64::from(row)))
                .or_default()
                .push(entity);
        }

        for (key, entities) in by_cell {
            let render_key = layer_door_key(layer_spawn.index, &key);
            for (index, entity) in entities.iter().enumerate() {
                let Some(def) = items.get_item(&entity.item_id) else {
                    continue;
                };
                let Some((col, row)) = world_cell(entity) else {
                    continue;
                };
                let mesh = spawn_item_mesh(
                    commands,
                    meshes,
                    materials,
                    asset_server,
                    &ItemPlacement {
                        kind,
                        icon: &def.icon,
                        col,
                        row,
                        index,
                        layer_y_offset,
                    },
                );
                let store_key = if index == 0 {
                    render_key.clone()
                } else {
                    format!("{render_key}#{index}")
                };
                billboards.map_mut(kind).insert(store_key, mesh);
            }
        }
    }

    billboards
}

/// Remove the primary mesh and any multi-item spread entries at a cell.
fn hide_item_meshes(map: &mut HashMap<String, Entity>, commands: &mut Commands, key: &str) {
    let spread_prefix = format!("{key}#");
    map.retain(|store_key, entity| {
        if store_key == key || store_key.starts_with(&spread_prefix) {
            commands.entity(*entity).despawn();
            false
        } else {
            true
        }
    });
}

/// Add one billboard for an item, picking the next free spread slot at its
/// cell (mirrors the TS `addSingleGroundItemMesh`). `pub(crate)` so
/// `session::apply_inventory_action` can respawn a dropped item's world
/// billboard after `InventoryAction::Drop` succeeds.
///
/// `active_layer_index` positions the billboard at that layer's default
/// `index * LAYER_HEIGHT` — gameplay-triggered spawns (pickup-drop, loot)
/// only ever touch the player's own active layer and have no `LayerDef` in
/// scope to honor a custom `yOffset` override, unlike the level-scene build
/// in `level_scene.rs`. No current level content overrides layer `yOffset`,
/// so this is a disclosed simplification, not an observed bug.
pub(crate) fn add_single_item_mesh(
    render: &mut GroundItemRender,
    kind: ItemKind,
    entity: &ItemEntity,
    active_layer_index: usize,
) {
    let Some(def) = render.items.0.get_item(&entity.item_id) else {
        return;
    };
    let Some((col, row)) = world_cell(entity) else {
        return;
    };
    let render_key = layer_door_key(
        active_layer_index,
        &door_key(i64::from(col), i64::from(row)),
    );
    let mut index = 0;
    let mut store_key = render_key.clone();
    while render.billboards.map_mut(kind).contains_key(&store_key) {
        index += 1;
        store_key = format!("{render_key}#{index}");
    }
    let icon = def.icon.clone();
    let layer_y_offset = active_layer_index as f32 * crate::dungeon::LAYER_HEIGHT;
    let mesh = spawn_item_mesh(
        &mut render.commands,
        &mut render.meshes,
        &mut render.materials,
        &render.asset_server,
        &ItemPlacement {
            kind,
            icon: &icon,
            col,
            row,
            index,
            layer_y_offset,
        },
    );
    render.billboards.map_mut(kind).insert(store_key, mesh);
}

fn first_remaining_of_kind(
    game: &GameState,
    items: &ItemDatabase,
    kind: ItemKind,
    col: i64,
    row: i64,
) -> Option<ItemEntity> {
    game.entity_registry
        .ground_items(
            &game.current_level_id,
            i32::try_from(col).unwrap_or(0),
            i32::try_from(row).unwrap_or(0),
        )
        .into_iter()
        .find(|entity| {
            items
                .get_item(&entity.item_id)
                .is_some_and(|def| ItemKind::of(def.item_type) == kind)
        })
        .cloned()
}

/// Equipment and consumable walk-over pickups for the cell the player just
/// entered, with billboard bookkeeping (hide all at the cell, re-show one
/// remaining), ported from the TS move handler.
pub fn handle_pickups(
    game: &mut GameState,
    render: &mut GroundItemRender,
    hud: &mut crate::hud::HudState,
    col: i64,
    row: i64,
) {
    let render_key = layer_door_key(game.active_layer_index, &door_key(col, row));
    let items = render.items.0.clone();

    let (equipped, denied) = game.pickup_equipment_at(col, row);
    if let Some(denied) = denied {
        info!("{denied}");
        hud.show_message(&denied);
    } else if let Some(name) = equipped {
        let message = format!("Equipped: {name}");
        info!("{message}");
        hud.show_message(&message);
        hide_item_meshes(
            &mut render.billboards.equipment,
            &mut render.commands,
            &render_key,
        );
        if let Some(remaining) =
            first_remaining_of_kind(game, &items, ItemKind::Equipment, col, row)
        {
            add_single_item_mesh(
                render,
                ItemKind::Equipment,
                &remaining,
                game.active_layer_index,
            );
        }
    }

    if let Some(name) = game.pickup_consumable_at(col, row) {
        info!("Picked up: {name}");
        hide_item_meshes(
            &mut render.billboards.consumables,
            &mut render.commands,
            &render_key,
        );
        if let Some(remaining) =
            first_remaining_of_kind(game, &items, ItemKind::Consumable, col, row)
        {
            add_single_item_mesh(
                render,
                ItemKind::Consumable,
                &remaining,
                game.active_layer_index,
            );
        }
    }
}

/// Roll loot for a kill (or other source) and drop the results at a cell,
/// ported from the TS lootSpawner.
pub fn spawn_loot(
    game: &mut GameState,
    render: &mut GroundItemRender,
    loot_tables: &LootTables,
    enemy_type: &str,
    drops: Option<&DropsOverride>,
    (col, row, layer_index): (i64, i64, usize),
    random: &mut dyn FnMut() -> f64,
) {
    let result = loot_tables.roll_loot(enemy_type, drops, random);
    game.player.gold += result.gold;
    if result.gold > 0 {
        info!("Looted {} gold", result.gold);
    }

    let items = render.items.0.clone();
    for drop in result.items {
        let location = ItemLocation::World {
            level_id: game.current_level_id.clone(),
            col: i32::try_from(col).unwrap_or(0),
            row: i32::try_from(row).unwrap_or(0),
            layer_index: Some(i32::try_from(layer_index).unwrap_or(0)),
        };
        let entity =
            game.entity_registry
                .create_item(&drop.item_id, drop.quality, location, drop.modifiers);
        let Some(def) = items.get_item(&drop.item_id) else {
            continue;
        };
        add_single_item_mesh(render, ItemKind::of(def.item_type), &entity, layer_index);
    }
}

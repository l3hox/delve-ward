//! Enemy billboards and AI ticking: sprites face the camera's view plane,
//! move with the core AI, and melee the player when adjacent.

use crate::dungeon::CELL_SIZE;
use crate::level_scene::LevelEntity;
use crate::player::Player;
use crate::session::{GameRng, Session};
use crate::transition::Transition;
use bevy::prelude::*;
use delve_core::combat::{CombatResultType, enemy_attack_player, player_attack};
use delve_core::enemies::{DEFAULT_SPRITE_SIZE, EnemyDatabase};
use delve_core::enemy_ai::{EnemyActionType, EnemyUpdateContext, update_enemies};
use delve_core::game_state::{DoorState, door_key};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Shared enemy definitions for the AI context and sprite lookups.
#[derive(Resource)]
pub struct EnemyDb(pub Arc<EnemyDatabase>);

#[derive(Component)]
pub struct EnemyBillboard;

/// Billboard entities by enemy cell key, re-keyed as enemies move.
#[derive(Resource, Default)]
pub struct EnemyBillboards {
    pub by_key: HashMap<String, Entity>,
}

/// Rendering-side handles the enemy tick needs to apply AI actions.
#[derive(bevy::ecs::system::SystemParam)]
pub struct EnemyRenderState<'w, 's> {
    billboards: ResMut<'w, EnemyBillboards>,
    commands: Commands<'w, 's>,
    transforms: Query<'w, 's, &'static mut Transform, With<EnemyBillboard>>,
}

fn sprite_asset_path(sprite_path: &str) -> String {
    sprite_path.trim_start_matches('/').to_string()
}

pub fn spawn_enemy_billboards(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
    game: &delve_core::game_state::GameState,
    database: &EnemyDatabase,
) -> EnemyBillboards {
    let mut billboards = EnemyBillboards::default();
    for (key, enemy) in &game.active_layer().enemies {
        let def = database.get_enemy(&enemy.enemy_type);
        let size = def
            .and_then(|def| def.sprite.size)
            .unwrap_or(DEFAULT_SPRITE_SIZE) as f32;
        let y_offset = def.and_then(|def| def.sprite.y_offset).unwrap_or(0.0) as f32;
        let texture_path = def
            .map(|def| sprite_asset_path(&def.sprite.path))
            .unwrap_or_else(|| "sprites/skeleton.png".to_string());

        let material = materials.add(StandardMaterial {
            base_color_texture: Some(asset_server.load(texture_path)),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            reflectance: 0.0,
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            ..default()
        });
        let center_x = enemy.col as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let center_z = enemy.row as f32 * CELL_SIZE + CELL_SIZE / 2.0;
        let entity = commands
            .spawn((
                LevelEntity,
                EnemyBillboard,
                Mesh3d(meshes.add(Rectangle::new(size, size))),
                MeshMaterial3d(material),
                Transform::from_xyz(center_x, size * 0.5 + y_offset, center_z),
            ))
            .id();
        billboards.by_key.insert(key.clone(), entity);
    }
    billboards
}

/// All sprites face the camera's view plane (its yaw, not the camera point).
pub fn face_billboards_to_camera(
    cameras: Query<&Transform, (With<Player>, Without<EnemyBillboard>)>,
    mut billboards: Query<&mut Transform, With<EnemyBillboard>>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    let (yaw, _, _) = camera.rotation.to_euler(EulerRot::YXZ);
    for mut transform in &mut billboards {
        transform.rotation = Quat::from_rotation_y(yaw);
    }
}

fn cell_center(col: i64, row: i64) -> (f32, f32) {
    (
        col as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        row as f32 * CELL_SIZE + CELL_SIZE / 2.0,
    )
}

pub fn tick_enemies(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut rng: ResMut<GameRng>,
    database: Res<EnemyDb>,
    players: Query<&Player>,
    mut render: EnemyRenderState,
) {
    let Ok(player) = players.single() else {
        return;
    };
    let player_state = player.grid_state();
    let (player_col, player_row) = (i64::from(player_state.col), i64::from(player_state.row));

    // Snapshot door state so the AI's callbacks don't borrow the game state.
    let closed_doors: HashSet<String> = session
        .game
        .active_layer()
        .doors
        .values()
        .filter(|door| door.state == DoorState::Closed)
        .map(|door| door_key(door.col, door.row))
        .collect();
    let is_door_open = |col: i64, row: i64| !closed_doors.contains(&door_key(col, row));

    let Session {
        game,
        grid,
        walkable,
        ..
    } = &mut *session;
    let context = EnemyUpdateContext {
        player_col,
        player_row,
        grid,
        walkable,
        is_door_open: &is_door_open,
        is_hole: None,
        is_edge_blocked: None,
        enemies: &database.0,
    };
    let rng = &mut rng.0;
    let mut random = || rng.next_f64();
    let actions = update_enemies(game, &context, f64::from(time.delta_secs()), &mut random);

    for action in actions {
        match action.action_type {
            EnemyActionType::Move => {
                let (Some(to_col), Some(to_row)) = (action.to_col, action.to_row) else {
                    continue;
                };
                let old_key = door_key(action.from_col, action.from_row);
                if let Some(entity) = render.billboards.by_key.remove(&old_key) {
                    render
                        .billboards
                        .by_key
                        .insert(door_key(to_col, to_row), entity);
                    if let Ok(mut transform) = render.transforms.get_mut(entity) {
                        let (center_x, center_z) = cell_center(to_col, to_row);
                        transform.translation.x = center_x;
                        transform.translation.z = center_z;
                    }
                }
            }
            EnemyActionType::Attack => {
                let Some(enemy_atk) = game
                    .get_enemy(action.from_col, action.from_row)
                    .map(|enemy| enemy.atk)
                else {
                    continue;
                };
                let result = enemy_attack_player(game, enemy_atk, &mut random);
                info!(
                    "Enemy hits you for {} — HP {}/{}",
                    result.damage, game.player.hp, game.player.max_hp
                );
                if game.player.hp <= 0.0 {
                    info!("You died.");
                }
            }
            EnemyActionType::StatusKill => {
                let key = door_key(action.from_col, action.from_row);
                game.active_layer_mut().enemies.remove(&key);
                if let Some(entity) = render.billboards.by_key.remove(&key) {
                    render.commands.entity(entity).despawn();
                }
            }
            _ => {}
        }
    }
}

pub fn attack_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    transition: Res<Transition>,
    mut rng: ResMut<GameRng>,
    mut billboards: ResMut<EnemyBillboards>,
    players: Query<&Player>,
    mut commands: Commands,
) {
    if transition.is_active() || !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let rng = &mut rng.0;
    let mut random = || rng.next_f64();
    let results = player_attack(player.grid_state(), &mut session.game, &mut random);
    for result in results {
        match result.result_type {
            CombatResultType::Hit => {
                info!(
                    "You hit the {} for {}",
                    result.enemy_type.as_deref().unwrap_or("enemy"),
                    result.damage.unwrap_or(0.0)
                );
            }
            CombatResultType::Kill => {
                info!(
                    "You slay the {}!",
                    result.enemy_type.as_deref().unwrap_or("enemy")
                );
                if let (Some(col), Some(row)) = (result.target_col, result.target_row) {
                    let key = door_key(col, row);
                    if let Some(entity) = billboards.by_key.remove(&key) {
                        commands.entity(entity).despawn();
                    }
                }
            }
            CombatResultType::NoTarget => info!("You swing at nothing."),
            CombatResultType::Cooldown => {}
            other => debug!("attack result: {other:?}"),
        }
    }
}

/// Wind down the swing cooldown.
pub fn tick_attack_cooldown(time: Res<Time>, mut session: ResMut<Session>) {
    if session.game.player.attack_cooldown > 0.0 {
        session.game.player.attack_cooldown =
            (session.game.player.attack_cooldown - f64::from(time.delta_secs())).max(0.0);
    }
}

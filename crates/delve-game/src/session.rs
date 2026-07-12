//! The running game session: `GameState` and the active grid as Bevy
//! resources, plus the input/update systems that connect the player
//! controller, interaction, and world events to rendering.

use crate::doors::{DoorPanel, DoorPanels};
use crate::player::Player;
use bevy::prelude::*;
use delve_core::game_state::{GameState, WorldEvent};
use delve_core::grid::{Facing, MoveRules};
use delve_core::interaction::{InteractionType, interact};
use delve_core::random::Mulberry32;
use std::collections::HashSet;

#[derive(Resource)]
pub struct Session {
    pub game: GameState,
    pub grid: Vec<String>,
    pub walkable: HashSet<char>,
    last_player_pose: (i32, i32, Facing),
}

impl Session {
    pub fn new(
        game: GameState,
        grid: Vec<String>,
        walkable: HashSet<char>,
        start: (i32, i32, Facing),
    ) -> Self {
        Self {
            game,
            grid,
            walkable,
            last_player_pose: start,
        }
    }
}

/// Seeded gameplay randomness (loot rolls, erratic movement, spawners).
#[derive(Resource)]
pub struct GameRng(pub Mulberry32);

/// Build the TS movement callbacks from the game state and hand them to `f`.
fn with_move_rules<R>(game: &GameState, f: impl FnOnce(&MoveRules) -> R) -> R {
    let is_door_open = |col: i32, row: i32| game.is_door_open(i64::from(col), i64::from(row));
    let is_blocked = |col: i32, row: i32| {
        let (col, row) = (i64::from(col), i64::from(row));
        game.is_blocked_by_enemy(col, row)
            || game.is_block_at(col, row)
            || game.is_npc_at(col, row)
            || game.is_barrel_at(col, row)
            || game.is_boulder_at(col, row)
    };
    let is_edge_blocked = |from_col: i32, from_row: i32, to_col: i32, to_row: i32| {
        game.is_edge_blocked(
            i64::from(from_col),
            i64::from(from_row),
            i64::from(to_col),
            i64::from(to_row),
        )
    };
    f(&MoveRules {
        is_door_open: Some(&is_door_open),
        is_blocked: Some(&is_blocked),
        is_edge_blocked: Some(&is_edge_blocked),
        is_ramp_accessible: None,
    })
}

pub fn player_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<Session>,
    mut players: Query<&mut Player>,
) {
    let Ok(mut player) = players.single_mut() else {
        return;
    };
    with_move_rules(&session.game, |rules| {
        if keys.just_pressed(KeyCode::KeyW) || keys.just_pressed(KeyCode::ArrowUp) {
            player.move_forward(rules);
        }
        if keys.just_pressed(KeyCode::KeyS) || keys.just_pressed(KeyCode::ArrowDown) {
            player.move_back(rules);
        }
        if keys.just_pressed(KeyCode::KeyA) {
            player.strafe_left(rules);
        }
        if keys.just_pressed(KeyCode::KeyD) {
            player.strafe_right(rules);
        }
        if keys.just_pressed(KeyCode::KeyQ) || keys.just_pressed(KeyCode::ArrowLeft) {
            player.turn_left(rules);
        }
        if keys.just_pressed(KeyCode::KeyE) || keys.just_pressed(KeyCode::ArrowRight) {
            player.turn_right(rules);
        }
    });
}

pub fn player_update(
    time: Res<Time>,
    session: Res<Session>,
    mut players: Query<(&mut Player, &mut Transform)>,
) {
    let Ok((mut player, mut transform)) = players.single_mut() else {
        return;
    };
    with_move_rules(&session.game, |rules| {
        player.update(time.delta_secs(), &mut transform, rules);
    });
}

/// Reveal explored cells and pick up keys whenever the player's logical cell
/// or facing changes.
pub fn on_player_moved(mut session: ResMut<Session>, players: Query<&Player>) {
    let Ok(player) = players.single() else {
        return;
    };
    let state = player.grid_state();
    let pose = (state.col, state.row, state.facing);
    if pose == session.last_player_pose {
        return;
    }
    session.last_player_pose = pose;

    let Session { game, grid, .. } = &mut *session;
    let (col, row) = (i64::from(pose.0), i64::from(pose.1));
    game.reveal_around(col, row, pose.2, grid);
    if let Some(key_id) = game.pickup_key_at(col, row) {
        info!("Picked up key: {key_id}");
    }
}

fn set_panel_open(
    panels: &DoorPanels,
    panel_query: &mut Query<&mut DoorPanel>,
    key: &str,
    open: bool,
) {
    if let Some(&entity) = panels.by_key.get(key)
        && let Ok(mut panel) = panel_query.get_mut(entity)
    {
        panel.set_open(open);
    }
}

/// Apply drained world events to rendering (door panels for now; pit traps,
/// spawners, and launchers land with their phases).
pub fn apply_world_events(
    events: Vec<WorldEvent>,
    panels: &DoorPanels,
    panel_query: &mut Query<&mut DoorPanel>,
) {
    for event in events {
        match event {
            WorldEvent::DoorSignalChanged { col, row, open } => {
                set_panel_open(
                    panels,
                    panel_query,
                    &delve_core::game_state::door_key(col, row),
                    open,
                );
            }
            other => debug!("unhandled world event: {other:?}"),
        }
    }
}

pub fn interact_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    players: Query<&Player>,
    panels: Res<DoorPanels>,
    mut panel_query: Query<&mut DoorPanel>,
) {
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let player_state = player.grid_state();
    let facing_cell = delve_core::grid::get_facing_cell(player_state);
    let facing_key =
        delve_core::game_state::door_key(i64::from(facing_cell.0), i64::from(facing_cell.1));

    let result = {
        let Session { game, grid, .. } = &mut *session;
        interact(player_state, grid, game)
    };

    match result.result_type {
        InteractionType::DoorOpened => {
            set_panel_open(&panels, &mut panel_query, &facing_key, true);
        }
        InteractionType::DoorClosed => {
            set_panel_open(&panels, &mut panel_query, &facing_key, false);
        }
        InteractionType::DoorBlocked => {
            if let Some(&entity) = panels.by_key.get(&facing_key)
                && let Ok(mut panel) = panel_query.get_mut(entity)
            {
                panel.bounce();
            }
        }
        InteractionType::LeverActivated => {
            for target in result.targets.iter().flatten() {
                let Some(position) = session.game.resolve_entity_position(target) else {
                    continue;
                };
                let (col, row) = (position.col, position.row);
                let open = session.game.is_door_open(col, row);
                set_panel_open(
                    &panels,
                    &mut panel_query,
                    &delve_core::game_state::door_key(col, row),
                    open,
                );
            }
        }
        _ => {}
    }
    if let Some(message) = &result.message {
        info!("{message}");
    }

    let events = session.game.take_events();
    apply_world_events(events, &panels, &mut panel_query);
}

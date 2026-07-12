//! The running game session: `GameState` and the active grid as Bevy
//! resources, plus the input/update systems that connect the player
//! controller, interaction, and world events to rendering.

use crate::doors::{DoorPanel, DoorPanels};
use crate::ground_items::{self, GroundItemRender};
use crate::keys::{self, KeyBillboards};
use crate::levers::{self, LeverRender};
use crate::plates::{self, PlateRender};
use crate::player::Player;
use crate::sconces::{self, SconceRender};
use crate::transition::Transition;
use crate::tripwires::{self, TripwireHandles};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::game_state::{DoorState, GameState, LeverState, MultiLayerSnapshot, WorldEvent};
use delve_core::grid::{Facing, MoveRules};
use delve_core::interaction::{InteractionType, interact};
use delve_core::random::Mulberry32;
use delve_core::types::{Dungeon, Environment, TextureArea};
use std::collections::{HashMap, HashSet};

#[derive(Resource)]
pub struct Session {
    pub game: GameState,
    pub grid: Vec<String>,
    pub walkable: HashSet<char>,
    pub current_level_id: String,
    /// Level default environment plus per-area overrides, for cell-local
    /// checks like torch drain.
    pub environment: Environment,
    pub areas: Vec<TextureArea>,
    pub(crate) last_player_pose: (i32, i32, Facing),
}

impl Session {
    pub fn new(
        game: GameState,
        grid: Vec<String>,
        walkable: HashSet<char>,
        current_level_id: String,
        level: &delve_core::types::DungeonLevel,
        start: (i32, i32, Facing),
    ) -> Self {
        Self {
            game,
            grid,
            walkable,
            current_level_id,
            environment: level.environment.unwrap_or(Environment::Dungeon),
            areas: level.areas.clone().unwrap_or_default(),
            last_player_pose: start,
        }
    }
}

/// Seeded gameplay randomness (loot rolls, erratic movement, spawners).
#[derive(Resource)]
pub struct GameRng(pub Mulberry32);

/// The full loaded dungeon, kept for cross-level transitions.
#[derive(Resource)]
pub struct DungeonRes(pub Dungeon);

/// Departed levels' state, restored when the player returns.
#[derive(Resource, Default)]
pub struct LevelSnapshots(pub HashMap<String, MultiLayerSnapshot>);

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
    gate: crate::char_creation::InputGate,
    mut players: Query<&mut Player>,
) {
    if gate.blocked() {
        return;
    }
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

/// Reveal explored cells, pick up keys and items, activate signal entities,
/// and start stair transitions whenever the player's logical cell or facing
/// changes.
pub fn on_player_moved(
    mut session: ResMut<Session>,
    players: Query<&Player>,
    mut transition: ResMut<Transition>,
    mut item_render: GroundItemRender,
    mut key_billboards: ResMut<KeyBillboards>,
    mut hud: ResMut<crate::hud::HudState>,
    mut signal: SignalRenderState,
) {
    let Ok(player) = players.single() else {
        return;
    };
    let state = player.grid_state();
    let pose = (state.col, state.row, state.facing);
    if pose == session.last_player_pose {
        return;
    }
    let (prev_col, prev_row, _) = session.last_player_pose;
    session.last_player_pose = pose;
    // TS gates its momentary-source deactivate/activate calls on an actual
    // cell change (`col !== prevCol || row !== prevRow`) because its onMove
    // callback only fires on real moves; this port's on_player_moved also
    // runs for pure turns (its pose includes facing), so the same guard is
    // needed here to avoid re-toggling on every turn-in-place.
    let moved = pose.0 != prev_col || pose.1 != prev_row;

    let Session {
        game,
        grid,
        environment,
        areas,
        ..
    } = &mut *session;
    let (col, row) = (i64::from(pose.0), i64::from(pose.1));

    if moved {
        game.deactivate_pressure_plate(i64::from(prev_col), i64::from(prev_row));
        game.deactivate_trigger(i64::from(prev_col), i64::from(prev_row));

        // Safety: if the player ended up on a closed door cell, force it
        // open and hand it to the blocked-door retry cycle.
        let key = delve_core::game_state::door_key(col, row);
        let closed_underfoot = game
            .active_layer()
            .doors
            .get(&key)
            .is_some_and(|door| door.state == DoorState::Closed);
        if closed_underfoot {
            set_door_state(game, &key, DoorState::Open);
            set_panel_open(&signal.door_panels, &mut signal.panel_query, &key, true);
            signal.blocked_doors.by_key.insert(
                key,
                BlockedDoor {
                    col,
                    row,
                    timer: DOOR_RETRY_INTERVAL,
                },
            );
        }
    }

    game.reveal_around(col, row, pose.2, grid);
    if let Some(key_id) = game.pickup_key_at(col, row) {
        info!("Picked up key: {key_id}");
        keys::hide_key_mesh(
            &mut key_billboards,
            &mut item_render.commands,
            &delve_core::game_state::door_key(col, row),
        );
    }
    ground_items::handle_pickups(game, &mut item_render, &mut hud, col, row);

    if moved {
        let key = delve_core::game_state::door_key(col, row);
        game.activate_trigger(col, row);
        if game.activate_tripwire(col, row) {
            tripwires::hide_tripwire_mesh(&signal.tripwires, &mut item_render.commands, &key);
            hud.show_message("Oops! A tripwire!");
        }
        let pressed = game.activate_pressure_plate(col, row).is_some()
            && game
                .active_layer()
                .plates
                .get(&key)
                .is_some_and(|plate| plate.activated);
        if pressed {
            plates::press_plate(&mut signal.plate, &key);
        }

        // Torch fuel drains one unit per step, except in environments with
        // their own light (open sky, luminous mist).
        let cell_environment =
            crate::environment::resolve_environment_at_cell(col, row, *environment, areas);
        if delve_core::player_controller::should_drain_torch(cell_environment) {
            game.drain_torch_fuel(1.0);
        }
    }

    let events = game.take_events();
    apply_world_events(events, game, (col, row), &mut signal);

    if let Some(stair) = game.get_stair(col, row)
        && let Some(stair_id) = &stair.id
    {
        transition.begin(stair_id.clone());
    }
}

/// Bundled rendering handles for door, lever, plate, and tripwire visuals,
/// grouped into one `SystemParam` so systems that touch signal-entity
/// rendering stay under the argument-count lint.
#[derive(SystemParam)]
pub struct SignalRenderState<'w, 's> {
    pub door_panels: Res<'w, DoorPanels>,
    pub panel_query: Query<'w, 's, &'static mut DoorPanel>,
    pub lever: LeverRender<'w, 's>,
    pub plate: PlateRender<'w, 's>,
    pub tripwires: Res<'w, TripwireHandles>,
    pub blocked_doors: ResMut<'w, BlockedDoors>,
}

const DOOR_RETRY_INTERVAL: f32 = 1.5;

/// Doors a signal tried to close while the cell was occupied. The close is
/// held off and retried on a timer (with a panel bounce) until the cell
/// clears — a signal-driven close can never land on a standing player or
/// enemy, ported from the TS `blockedDoors` map.
#[derive(Resource, Default)]
pub struct BlockedDoors {
    by_key: HashMap<String, BlockedDoor>,
}

impl BlockedDoors {
    pub fn clear(&mut self) {
        self.by_key.clear();
    }
}

struct BlockedDoor {
    col: i64,
    row: i64,
    timer: f32,
}

fn is_door_cell_occupied(game: &GameState, player_cell: (i64, i64), col: i64, row: i64) -> bool {
    player_cell == (col, row) || game.get_enemy(col, row).is_some()
}

fn bounce_panel(panels: &DoorPanels, panel_query: &mut Query<&mut DoorPanel>, key: &str) {
    if let Some(&entity) = panels.by_key.get(key)
        && let Ok(mut panel) = panel_query.get_mut(entity)
    {
        panel.bounce();
    }
}

fn set_door_state(game: &mut GameState, key: &str, state: DoorState) {
    if let Some(door) = game.active_layer_mut().doors.get_mut(key) {
        door.state = state;
    }
}

/// Retry pending blocked-door closes: once the cell clears, the door
/// actually closes; while it stays occupied, the panel bounces and the
/// timer re-arms.
fn tick_blocked_doors(
    game: &mut GameState,
    player_cell: (i64, i64),
    signal: &mut SignalRenderState,
    delta: f32,
) {
    let mut close_now = Vec::new();
    let mut bounce_now = Vec::new();
    for (key, entry) in &mut signal.blocked_doors.by_key {
        entry.timer -= delta;
        if entry.timer > 0.0 {
            continue;
        }
        if is_door_cell_occupied(game, player_cell, entry.col, entry.row) {
            entry.timer = DOOR_RETRY_INTERVAL;
            bounce_now.push(key.clone());
        } else {
            close_now.push(key.clone());
        }
    }
    for key in close_now {
        signal.blocked_doors.by_key.remove(&key);
        set_door_state(game, &key, DoorState::Closed);
        set_panel_open(&signal.door_panels, &mut signal.panel_query, &key, false);
    }
    for key in bounce_now {
        bounce_panel(&signal.door_panels, &mut signal.panel_query, &key);
    }
}

/// Advance the game state's timed signals each frame and apply the
/// resulting world events (timed doors, levers, and plates). Paused while
/// character creation or a level transition is active — the TS loop pauses
/// its tick while overlays are open, and pausing across the swap keeps
/// timed state out of the mid-transition window.
pub fn tick_game(
    time: Res<Time>,
    mut session: ResMut<Session>,
    gate: crate::char_creation::InputGate,
    mut signal: SignalRenderState,
) {
    if gate.blocked() {
        return;
    }
    let delta = time.delta_secs();
    let player_cell = (
        i64::from(session.last_player_pose.0),
        i64::from(session.last_player_pose.1),
    );
    session.game.tick_signals(f64::from(delta));
    let events = session.game.take_events();
    let Session { game, .. } = &mut *session;
    apply_world_events(events, game, player_cell, &mut signal);
    tick_blocked_doors(game, player_cell, &mut signal, delta);
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

/// Apply drained world events to rendering: door panels, lever resets, and
/// plate resets. Pit traps and spawners land with their phases. Signal
/// closes on occupied door cells are held open and retried via
/// [`BlockedDoors`].
pub fn apply_world_events(
    events: Vec<WorldEvent>,
    game: &mut GameState,
    player_cell: (i64, i64),
    signal: &mut SignalRenderState,
) {
    for event in events {
        match event {
            WorldEvent::DoorSignalChanged { col, row, open } => {
                let key = delve_core::game_state::door_key(col, row);
                if open {
                    signal.blocked_doors.by_key.remove(&key);
                    set_panel_open(&signal.door_panels, &mut signal.panel_query, &key, true);
                } else if is_door_cell_occupied(game, player_cell, col, row) {
                    set_door_state(game, &key, DoorState::Open);
                    signal.blocked_doors.by_key.insert(
                        key.clone(),
                        BlockedDoor {
                            col,
                            row,
                            timer: DOOR_RETRY_INTERVAL,
                        },
                    );
                    bounce_panel(&signal.door_panels, &mut signal.panel_query, &key);
                } else {
                    signal.blocked_doors.by_key.remove(&key);
                    set_panel_open(&signal.door_panels, &mut signal.panel_query, &key, false);
                }
            }
            WorldEvent::LeverReset { col, row } => {
                levers::set_lever_target(
                    &mut signal.lever,
                    &delve_core::game_state::door_key(col, row),
                    LeverState::Up,
                );
            }
            WorldEvent::PlateReset { col, row } => {
                plates::release_plate(
                    &mut signal.plate,
                    &delve_core::game_state::door_key(col, row),
                );
            }
            other => debug!("unhandled world event: {other:?}"),
        }
    }
}

pub fn interact_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    gate: crate::char_creation::InputGate,
    players: Query<&Player>,
    mut signal: SignalRenderState,
    mut sconce_render: SconceRender,
) {
    if gate.blocked() || !keys.just_pressed(KeyCode::Space) {
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
            set_panel_open(
                &signal.door_panels,
                &mut signal.panel_query,
                &facing_key,
                true,
            );
        }
        InteractionType::DoorClosed => {
            set_panel_open(
                &signal.door_panels,
                &mut signal.panel_query,
                &facing_key,
                false,
            );
        }
        InteractionType::DoorBlocked => {
            if let Some(&entity) = signal.door_panels.by_key.get(&facing_key)
                && let Ok(mut panel) = signal.panel_query.get_mut(entity)
            {
                panel.bounce();
            }
        }
        InteractionType::SconceTaken => {
            sconces::extinguish_sconce(
                &mut sconce_render,
                &delve_core::game_state::door_key(
                    i64::from(player_state.col),
                    i64::from(player_state.row),
                ),
            );
        }
        InteractionType::LeverActivated => {
            for target in result.targets.iter().flatten() {
                let Some(position) = session.game.resolve_entity_position(target) else {
                    continue;
                };
                let (col, row) = (position.col, position.row);
                let open = session.game.is_door_open(col, row);
                set_panel_open(
                    &signal.door_panels,
                    &mut signal.panel_query,
                    &delve_core::game_state::door_key(col, row),
                    open,
                );
            }
            let (lever_col, lever_row) = (i64::from(player_state.col), i64::from(player_state.row));
            if let Some(state) = session
                .game
                .get_lever(lever_col, lever_row)
                .map(|lever| lever.state)
            {
                levers::set_lever_target(
                    &mut signal.lever,
                    &delve_core::game_state::door_key(lever_col, lever_row),
                    state,
                );
            }
        }
        _ => {}
    }
    if let Some(message) = &result.message {
        info!("{message}");
    }

    let player_cell = (i64::from(player_state.col), i64::from(player_state.row));
    let events = session.game.take_events();
    apply_world_events(events, &mut session.game, player_cell, &mut signal);
}

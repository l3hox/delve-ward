//! First-person grid movement with tweened camera, ported from the TS
//! rendering `Player`. `no_clip` (threaded through `move_forward`/
//! `move_back`/`strafe_left`/`strafe_right`) is `debug.rs`'s debug
//! fullbright flag — TS ties `debugNoClip` to `debugFullbright` 1:1 rather
//! than exposing a separate toggle.

use crate::dungeon::{CELL_SIZE, EYE_HEIGHT, LAYER_HEIGHT};
use bevy::prelude::*;
use delve_core::game_state::{StairDirection, door_key};
use delve_core::grid::{Facing, MoveRules, PlayerState};
use std::collections::{HashMap, HashSet, VecDeque};

const TWEEN_SPEED: f32 = 20.0;
const ANIM_THRESHOLD: f32 = 0.05;
/// Pull the camera back from the cell center to see tile edges.
const CAMERA_BACK_OFFSET: f32 = 0.95;
const MAX_QUEUED_COMMANDS: usize = 3;
/// Camera dips/rises when stepping onto stairs.
const STAIR_Y_OFFSET: f32 = 0.35;
/// Camera tilts down/up on stairs (radians, ~8.5°).
const STAIR_PITCH: f32 = 0.15;

/// Terminal fall speed, units/sec — `player.ts`'s `FALL_TERMINAL_VELOCITY`.
const FALL_TERMINAL_VELOCITY: f32 = 20.0;
/// Distance over which the fall accelerates before reaching terminal
/// velocity — `player.ts`'s `FALL_ACCEL_DISTANCE` (2 layers' worth).
const FALL_ACCEL_DISTANCE: f32 = 2.0 * LAYER_HEIGHT;
/// `FALL_TERMINAL_VELOCITY^2 / (2 * FALL_ACCEL_DISTANCE)` — `player.ts`'s
/// `FALL_ACCEL`, derived the same way (40 u/s² at the current constants).
const FALL_ACCEL: f32 =
    (FALL_TERMINAL_VELOCITY * FALL_TERMINAL_VELOCITY) / (2.0 * FALL_ACCEL_DISTANCE);
/// Camera pitch while falling, radians (looking down) — `FALL_CAMERA_PITCH`.
const FALL_CAMERA_PITCH: f32 = -0.4;
/// Fraction of a walk tween's progress at which a pending fall activates —
/// `FALL_TRIGGER_PROGRESS`, lets the player visually step into the pit
/// before dropping.
const FALL_TRIGGER_PROGRESS: f32 = 0.667;

#[derive(Clone, Copy)]
enum Command {
    Forward,
    Back,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
}

/// A fall queued by a hole-detection trigger but not yet active — becomes
/// real kinematic motion once the in-progress walk tween crosses
/// `FALL_TRIGGER_PROGRESS`. TS also stores a `totalDistance` alongside
/// `landingLayer` (`player.ts:52`), but nothing in the TS codebase ever
/// reads it back out (confirmed by search) — dropped here rather than
/// carried as dead data, per this port's dead-variable convention.
#[derive(Clone, Copy)]
struct PendingFall {
    landing_layer: usize,
}

#[derive(Component)]
pub struct Player {
    state: PlayerState,
    grid: Vec<String>,
    stairs: HashMap<String, StairDirection>,
    current_pos: Vec3,
    target_pos: Vec3,
    // Continuous angle accumulation avoids wrap-around on repeated turns.
    current_angle: f32,
    target_angle: f32,
    current_pitch: f32,
    target_pitch: f32,
    command_queue: VecDeque<Command>,
    pub slow_multiplier: f32,
    /// Additive Y offset layered on top of `current_pos`'s own Y — layer
    /// positioning, ported from `player.ts`'s `yOffset`/`targetYOffset`.
    /// Outside a fall, this lerps toward `target_y_offset` the same way
    /// `current_pos` lerps toward `target_pos`; a future ramp-crossing slice
    /// shares this same channel rather than adding a second one.
    y_offset: f32,
    target_y_offset: f32,
    is_falling: bool,
    fall_velocity: f32,
    fall_distance: f32,
    fall_target_y_offset: f32,
    fall_landing_layer: usize,
    pending_fall: Option<PendingFall>,
    move_start_pos: Option<Vec3>,
    move_start_dist: f32,
}

fn grid_to_world(col: i32, row: i32, stairs: &HashMap<String, StairDirection>) -> Vec3 {
    let mut eye_y = EYE_HEIGHT;
    match stairs.get(&door_key(i64::from(col), i64::from(row))) {
        Some(StairDirection::Down) => eye_y -= STAIR_Y_OFFSET,
        Some(StairDirection::Up) => eye_y += STAIR_Y_OFFSET,
        None => {}
    }
    Vec3::new(
        col as f32 * CELL_SIZE + CELL_SIZE / 2.0,
        eye_y,
        row as f32 * CELL_SIZE + CELL_SIZE / 2.0,
    )
}

fn pitch_for_cell(col: i32, row: i32, stairs: &HashMap<String, StairDirection>) -> f32 {
    match stairs.get(&door_key(i64::from(col), i64::from(row))) {
        Some(StairDirection::Down) => -STAIR_PITCH,
        Some(StairDirection::Up) => STAIR_PITCH,
        None => 0.0,
    }
}

fn facing_angle(facing: Facing) -> f32 {
    facing.angle() as f32
}

impl Player {
    pub fn new(
        grid: Vec<String>,
        start_col: i32,
        start_row: i32,
        facing: Facing,
        walkable: HashSet<char>,
        stairs: HashMap<String, StairDirection>,
    ) -> Self {
        let position = grid_to_world(start_col, start_row, &stairs);
        let pitch = pitch_for_cell(start_col, start_row, &stairs);
        Self {
            state: PlayerState::with_walkable(start_col, start_row, facing, walkable),
            grid,
            stairs,
            current_pos: position,
            target_pos: position,
            current_angle: facing_angle(facing),
            target_angle: facing_angle(facing),
            current_pitch: pitch,
            target_pitch: pitch,
            command_queue: VecDeque::new(),
            slow_multiplier: 1.0,
            y_offset: 0.0,
            target_y_offset: 0.0,
            is_falling: false,
            fall_velocity: 0.0,
            fall_distance: 0.0,
            fall_target_y_offset: 0.0,
            fall_landing_layer: 0,
            pending_fall: None,
            move_start_pos: None,
            move_start_dist: 0.0,
        }
    }

    fn is_animating(&self) -> bool {
        self.is_falling
            || self.pending_fall.is_some()
            || self.current_pos.distance(self.target_pos) > ANIM_THRESHOLD
            || (self.current_angle - self.target_angle).abs() > ANIM_THRESHOLD
    }

    /// Whether a fall is queued or in progress — ported from TS's `get
    /// falling()`.
    pub fn falling(&self) -> bool {
        self.is_falling || self.pending_fall.is_some()
    }

    /// Queues a fall to `landing_layer`; it activates once the in-progress
    /// walk tween crosses `FALL_TRIGGER_PROGRESS` — ported from
    /// `setPendingFall`.
    pub fn set_pending_fall(&mut self, landing_layer: usize) {
        self.pending_fall = Some(PendingFall { landing_layer });
    }

    /// Swaps the grid/walkable-set/stairs a move is checked against —
    /// ported from `switchGrid`, used when the player's active layer
    /// changes (falling, and now ramp crossings).
    pub fn switch_grid(
        &mut self,
        grid: Vec<String>,
        walkable: HashSet<char>,
        stairs: HashMap<String, StairDirection>,
    ) {
        self.grid = grid;
        self.stairs = stairs;
        self.state.set_walkable(walkable);
    }

    /// Sets the ordinary-lerp Y-offset target directly — ramp crossings use
    /// this (`ls.player.targetYOffset = destLayer * LAYER_HEIGHT` in
    /// `main.ts`'s ramp-detection block), climbing smoothly via the same
    /// lerp `update` already applies outside a fall, rather than the fall's
    /// kinematic integration.
    pub fn set_target_y_offset(&mut self, value: f32) {
        self.target_y_offset = value;
    }

    fn enqueue(&mut self, command: Command) {
        if self.command_queue.len() < MAX_QUEUED_COMMANDS {
            self.command_queue.push_back(command);
        }
    }

    fn run(&mut self, command: Command, rules: &MoveRules, no_clip: bool) {
        if self.is_animating() {
            self.enqueue(command);
            return;
        }
        match command {
            Command::Forward => {
                let moved = if no_clip {
                    self.debug_move(self.state.facing.delta())
                } else {
                    self.state.move_forward(&self.grid, rules)
                };
                if moved {
                    self.arrive();
                }
            }
            Command::Back => {
                let moved = if no_clip {
                    let (dc, dr) = self.state.facing.delta();
                    self.debug_move((-dc, -dr))
                } else {
                    self.state.move_back(&self.grid, rules)
                };
                if moved {
                    self.arrive();
                }
            }
            Command::StrafeLeft => {
                let moved = if no_clip {
                    self.debug_move(self.state.facing.turned_left().delta())
                } else {
                    self.state.strafe_left(&self.grid, rules)
                };
                if moved {
                    self.arrive();
                }
            }
            Command::StrafeRight => {
                let moved = if no_clip {
                    self.debug_move(self.state.facing.turned_right().delta())
                } else {
                    self.state.strafe_right(&self.grid, rules)
                };
                if moved {
                    self.arrive();
                }
            }
            Command::TurnLeft => {
                self.state.turn_left();
                self.target_angle += std::f32::consts::FRAC_PI_2;
            }
            Command::TurnRight => {
                self.state.turn_right();
                self.target_angle -= std::f32::consts::FRAC_PI_2;
            }
        }
    }

    /// TS's private `debugMove` (`rendering/player.ts:164-171`): bounds-only
    /// movement bypassing every walkability/door/edge/entity check.
    fn debug_move(&mut self, delta: (i32, i32)) -> bool {
        let next_col = self.state.col + delta.0;
        let next_row = self.state.row + delta.1;
        let in_bounds = next_row >= 0
            && next_col >= 0
            && (next_row as usize) < self.grid.len()
            && self
                .grid
                .first()
                .is_some_and(|line| (next_col as usize) < line.chars().count());
        if !in_bounds {
            return false;
        }
        self.state.col = next_col;
        self.state.row = next_row;
        true
    }

    fn arrive(&mut self) {
        self.target_pos = grid_to_world(self.state.col, self.state.row, &self.stairs);
        self.target_pitch = pitch_for_cell(self.state.col, self.state.row, &self.stairs);
        // Captured here (not in `update`) so `move_start_dist` reflects this
        // tween's true total distance, matching `trackMoveStart`'s call
        // site right after `targetPos` is set in every TS move method.
        self.move_start_pos = Some(self.current_pos);
        self.move_start_dist = self.current_pos.distance(self.target_pos);
    }

    pub fn grid_state(&self) -> &PlayerState {
        &self.state
    }

    pub fn move_forward(&mut self, rules: &MoveRules, no_clip: bool) {
        self.run(Command::Forward, rules, no_clip);
    }

    pub fn move_back(&mut self, rules: &MoveRules, no_clip: bool) {
        self.run(Command::Back, rules, no_clip);
    }

    pub fn strafe_left(&mut self, rules: &MoveRules, no_clip: bool) {
        self.run(Command::StrafeLeft, rules, no_clip);
    }

    pub fn strafe_right(&mut self, rules: &MoveRules, no_clip: bool) {
        self.run(Command::StrafeRight, rules, no_clip);
    }

    pub fn turn_left(&mut self, rules: &MoveRules) {
        self.run(Command::TurnLeft, rules, false);
    }

    pub fn turn_right(&mut self, rules: &MoveRules) {
        self.run(Command::TurnRight, rules, false);
    }

    /// Advances the tween, the fall state machine, and the camera transform
    /// one frame — ported from `Player.update`. Returns the landing layer
    /// the instant a fall completes, so the caller (which owns `Session`/
    /// `GameState`) can apply the TS `onFallLand` callback's effects
    /// (`activeLayerIndex` switch, grid swap, `revealAround`) — this
    /// function only owns the player's own tween/camera state, not the
    /// wider game session, matching how every other core-vs-shell boundary
    /// in this port returns events instead of reaching across the boundary
    /// itself.
    pub fn update(
        &mut self,
        delta: f32,
        transform: &mut Transform,
        rules: &MoveRules,
        no_clip: bool,
    ) -> Option<usize> {
        let alpha = ((TWEEN_SPEED / self.slow_multiplier) * delta).min(1.0);

        self.current_pos = self.current_pos.lerp(self.target_pos, alpha);
        if self.current_pos.distance(self.target_pos) < 0.005 {
            self.current_pos = self.target_pos;
        }

        self.current_angle += (self.target_angle - self.current_angle) * alpha;
        if (self.current_angle - self.target_angle).abs() < 0.005 {
            self.current_angle = self.target_angle;
        }

        self.current_pitch += (self.target_pitch - self.current_pitch) * alpha;
        if (self.current_pitch - self.target_pitch).abs() < 0.005 {
            self.current_pitch = self.target_pitch;
        }

        // A queued fall activates once the walk tween that triggered it is
        // mostly done, so the player visibly steps into the pit first.
        if let Some(pending) = self.pending_fall
            && self.move_start_pos.is_some()
            && self.move_start_dist > 0.0
        {
            let remaining = self.current_pos.distance(self.target_pos);
            let progress = 1.0 - remaining / self.move_start_dist;
            if progress >= FALL_TRIGGER_PROGRESS {
                self.is_falling = true;
                self.fall_velocity = 0.0;
                self.fall_distance = 0.0;
                self.fall_target_y_offset = pending.landing_layer as f32 * LAYER_HEIGHT;
                self.fall_landing_layer = pending.landing_layer;
                self.target_pitch = FALL_CAMERA_PITCH;
                self.command_queue.clear();
                self.pending_fall = None;
                self.move_start_pos = None;
            }
        }

        // Y offset: kinematic fall integration while falling, the ordinary
        // lerp otherwise — the two paths are mutually exclusive by
        // construction (`is_falling` gates which branch runs).
        let mut landed = None;
        if self.is_falling {
            if self.fall_distance < FALL_ACCEL_DISTANCE {
                self.fall_velocity =
                    (self.fall_velocity + FALL_ACCEL * delta).min(FALL_TERMINAL_VELOCITY);
            }
            let dy = self.fall_velocity * delta;
            self.y_offset -= dy;
            self.fall_distance += dy;

            if self.y_offset <= self.fall_target_y_offset {
                self.y_offset = self.fall_target_y_offset;
                self.target_y_offset = self.fall_target_y_offset;
                self.is_falling = false;
                self.fall_velocity = 0.0;
                self.fall_distance = 0.0;
                self.target_pitch = 0.0;
                landed = Some(self.fall_landing_layer);
            }
        } else {
            self.y_offset += (self.target_y_offset - self.y_offset) * alpha;
            if (self.y_offset - self.target_y_offset).abs() < 0.005 {
                self.y_offset = self.target_y_offset;
            }
        }

        transform.translation = self.current_pos;
        transform.translation.y += self.y_offset;
        transform.translation.x += self.current_angle.sin() * CAMERA_BACK_OFFSET;
        transform.translation.z += self.current_angle.cos() * CAMERA_BACK_OFFSET;
        transform.rotation =
            Quat::from_euler(EulerRot::YXZ, self.current_angle, self.current_pitch, 0.0);

        // Drain one queued command per frame once the animation completes
        // (never mid-fall — matches TS's explicit `!isFalling` check, kept
        // even though `is_animating()` alone would already cover it).
        if !self.is_falling
            && !self.is_animating()
            && let Some(next) = self.command_queue.pop_front()
        {
            self.run(next, rules, no_clip);
        }

        landed
    }
}

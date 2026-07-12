//! First-person grid movement with tweened camera, ported from the TS
//! rendering `Player`. Falling and noclip arrive with their phases.

use crate::dungeon::{CELL_SIZE, EYE_HEIGHT};
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

#[derive(Clone, Copy)]
enum Command {
    Forward,
    Back,
    StrafeLeft,
    StrafeRight,
    TurnLeft,
    TurnRight,
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
        }
    }

    fn is_animating(&self) -> bool {
        self.current_pos.distance(self.target_pos) > ANIM_THRESHOLD
            || (self.current_angle - self.target_angle).abs() > ANIM_THRESHOLD
    }

    fn enqueue(&mut self, command: Command) {
        if self.command_queue.len() < MAX_QUEUED_COMMANDS {
            self.command_queue.push_back(command);
        }
    }

    fn run(&mut self, command: Command, rules: &MoveRules) {
        if self.is_animating() {
            self.enqueue(command);
            return;
        }
        match command {
            Command::Forward => {
                if self.state.move_forward(&self.grid, rules) {
                    self.arrive();
                }
            }
            Command::Back => {
                if self.state.move_back(&self.grid, rules) {
                    self.arrive();
                }
            }
            Command::StrafeLeft => {
                if self.state.strafe_left(&self.grid, rules) {
                    self.arrive();
                }
            }
            Command::StrafeRight => {
                if self.state.strafe_right(&self.grid, rules) {
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

    fn arrive(&mut self) {
        self.target_pos = grid_to_world(self.state.col, self.state.row, &self.stairs);
        self.target_pitch = pitch_for_cell(self.state.col, self.state.row, &self.stairs);
    }

    pub fn grid_state(&self) -> &PlayerState {
        &self.state
    }

    pub fn move_forward(&mut self, rules: &MoveRules) {
        self.run(Command::Forward, rules);
    }

    pub fn move_back(&mut self, rules: &MoveRules) {
        self.run(Command::Back, rules);
    }

    pub fn strafe_left(&mut self, rules: &MoveRules) {
        self.run(Command::StrafeLeft, rules);
    }

    pub fn strafe_right(&mut self, rules: &MoveRules) {
        self.run(Command::StrafeRight, rules);
    }

    pub fn turn_left(&mut self, rules: &MoveRules) {
        self.run(Command::TurnLeft, rules);
    }

    pub fn turn_right(&mut self, rules: &MoveRules) {
        self.run(Command::TurnRight, rules);
    }

    pub fn update(&mut self, delta: f32, transform: &mut Transform, rules: &MoveRules) {
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

        transform.translation = self.current_pos;
        transform.translation.x += self.current_angle.sin() * CAMERA_BACK_OFFSET;
        transform.translation.z += self.current_angle.cos() * CAMERA_BACK_OFFSET;
        transform.rotation =
            Quat::from_euler(EulerRot::YXZ, self.current_angle, self.current_pitch, 0.0);

        // Drain one queued command per frame once the animation completes.
        if !self.is_animating()
            && let Some(next) = self.command_queue.pop_front()
        {
            self.run(next, rules);
        }
    }
}

//! Pure grid logic: facing directions, walkability, and grid-step player state.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Facing {
    N,
    E,
    S,
    W,
}

impl Facing {
    /// Camera Y rotation for each facing (camera faces -Z by default = North).
    #[must_use]
    pub fn angle(self) -> f64 {
        match self {
            Facing::N => 0.0,
            Facing::E => -PI / 2.0,
            Facing::S => PI,
            Facing::W => PI / 2.0,
        }
    }

    /// `(dcol, drow)` for one step in this facing.
    #[must_use]
    pub fn delta(self) -> (i32, i32) {
        match self {
            Facing::N => (0, -1),
            Facing::E => (1, 0),
            Facing::S => (0, 1),
            Facing::W => (-1, 0),
        }
    }

    #[must_use]
    pub fn turned_left(self) -> Facing {
        match self {
            Facing::N => Facing::W,
            Facing::W => Facing::S,
            Facing::S => Facing::E,
            Facing::E => Facing::N,
        }
    }

    #[must_use]
    pub fn turned_right(self) -> Facing {
        match self {
            Facing::N => Facing::E,
            Facing::E => Facing::S,
            Facing::S => Facing::W,
            Facing::W => Facing::N,
        }
    }
}

#[must_use]
pub fn walkable_cells() -> HashSet<char> {
    HashSet::from(['.'])
}

/// Extend the built-in walkable set with non-solid custom chars.
#[must_use]
pub fn build_walkable_set<I: IntoIterator<Item = (char, bool)>>(char_defs: I) -> HashSet<char> {
    let mut set = walkable_cells();
    for (character, solid) in char_defs {
        if !solid {
            set.insert(character);
        }
    }
    set
}

#[must_use]
pub fn cell_at(grid: &[String], col: i32, row: i32) -> Option<char> {
    if row < 0 || row as usize >= grid.len() {
        return None;
    }
    if col < 0 {
        return None;
    }
    grid[row as usize].chars().nth(col as usize)
}

type CellPredicate<'a> = &'a dyn Fn(i32, i32) -> bool;
type EdgePredicate<'a> = &'a dyn Fn(i32, i32, i32, i32) -> bool;

/// Optional movement constraints injected by the game shell.
#[derive(Default, Clone, Copy)]
pub struct MoveRules<'a> {
    pub is_door_open: Option<CellPredicate<'a>>,
    pub is_blocked: Option<CellPredicate<'a>>,
    pub is_edge_blocked: Option<EdgePredicate<'a>>,
    pub is_ramp_accessible: Option<EdgePredicate<'a>>,
}

#[must_use]
pub fn is_walkable(
    grid: &[String],
    col: i32,
    row: i32,
    walkable: &HashSet<char>,
    is_door_open: Option<CellPredicate>,
    is_blocked: Option<CellPredicate>,
) -> bool {
    if row < 0 || row as usize >= grid.len() {
        return false;
    }
    let row_len = grid[0].chars().count();
    if col < 0 || col as usize >= row_len {
        return false;
    }
    let Some(cell) = cell_at(grid, col, row) else {
        return false;
    };
    if !walkable.contains(&cell) {
        return false;
    }
    if let Some(door_open) = is_door_open
        && !door_open(col, row)
    {
        return false;
    }
    if let Some(blocked) = is_blocked
        && blocked(col, row)
    {
        return false;
    }
    true
}

pub struct PlayerState {
    pub col: i32,
    pub row: i32,
    pub facing: Facing,
    walkable: HashSet<char>,
}

impl PlayerState {
    #[must_use]
    pub fn new(col: i32, row: i32, facing: Facing) -> Self {
        Self::with_walkable(col, row, facing, walkable_cells())
    }

    #[must_use]
    pub fn with_walkable(col: i32, row: i32, facing: Facing, walkable: HashSet<char>) -> Self {
        Self {
            col,
            row,
            facing,
            walkable,
        }
    }

    pub fn set_walkable(&mut self, walkable: HashSet<char>) {
        self.walkable = walkable;
    }

    fn can_move_to(&self, grid: &[String], new_col: i32, new_row: i32, rules: &MoveRules) -> bool {
        let walkable = is_walkable(
            grid,
            new_col,
            new_row,
            &self.walkable,
            rules.is_door_open,
            rules.is_blocked,
        );
        let ramp_accessible = rules
            .is_ramp_accessible
            .is_some_and(|accessible| accessible(self.col, self.row, new_col, new_row));
        if !walkable && !ramp_accessible {
            return false;
        }
        let edge_blocked = rules
            .is_edge_blocked
            .is_some_and(|blocked| blocked(self.col, self.row, new_col, new_row));
        if walkable && edge_blocked {
            return false;
        }
        true
    }

    fn step(&mut self, grid: &[String], dcol: i32, drow: i32, rules: &MoveRules) -> bool {
        let new_col = self.col + dcol;
        let new_row = self.row + drow;
        if !self.can_move_to(grid, new_col, new_row, rules) {
            return false;
        }
        self.col = new_col;
        self.row = new_row;
        true
    }

    pub fn move_forward(&mut self, grid: &[String], rules: &MoveRules) -> bool {
        let (dcol, drow) = self.facing.delta();
        self.step(grid, dcol, drow, rules)
    }

    pub fn move_back(&mut self, grid: &[String], rules: &MoveRules) -> bool {
        let (dcol, drow) = self.facing.delta();
        self.step(grid, -dcol, -drow, rules)
    }

    pub fn strafe_left(&mut self, grid: &[String], rules: &MoveRules) -> bool {
        let (dcol, drow) = self.facing.turned_left().delta();
        self.step(grid, dcol, drow, rules)
    }

    pub fn strafe_right(&mut self, grid: &[String], rules: &MoveRules) -> bool {
        let (dcol, drow) = self.facing.turned_right().delta();
        self.step(grid, dcol, drow, rules)
    }

    pub fn turn_left(&mut self) {
        self.facing = self.facing.turned_left();
    }

    pub fn turn_right(&mut self) {
        self.facing = self.facing.turned_right();
    }
}

/// The cell one step ahead of the player.
#[must_use]
pub fn get_facing_cell(state: &PlayerState) -> (i32, i32) {
    let (dcol, drow) = state.facing.delta();
    (state.col + dcol, state.row + drow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> Vec<String> {
        rows.iter().map(ToString::to_string).collect()
    }

    // 5x5 grid: walls around the edge, floor inside.
    fn boxed_grid() -> Vec<String> {
        grid(&["#####", "#...#", "#...#", "#...#", "#####"])
    }

    fn walkable(grid: &[String], col: i32, row: i32) -> bool {
        is_walkable(grid, col, row, &walkable_cells(), None, None)
    }

    #[test]
    fn is_walkable_true_for_floor_cells() {
        let grid = boxed_grid();
        assert!(walkable(&grid, 1, 1));
        assert!(walkable(&grid, 2, 2));
        assert!(walkable(&grid, 3, 3));
    }

    #[test]
    fn is_walkable_false_for_wall_cells() {
        let grid = boxed_grid();
        assert!(!walkable(&grid, 0, 0));
        assert!(!walkable(&grid, 4, 4));
        assert!(!walkable(&grid, 2, 0));
    }

    #[test]
    fn is_walkable_false_out_of_bounds() {
        let grid = boxed_grid();
        assert!(!walkable(&grid, -1, 2));
        assert!(!walkable(&grid, 2, -1));
        assert!(!walkable(&grid, 5, 2));
        assert!(!walkable(&grid, 2, 5));
    }

    #[test]
    fn is_walkable_recognizes_floor_only() {
        let grid = grid(&["#.#"]);
        assert!(walkable(&grid, 1, 0));
        assert!(!walkable(&grid, 0, 0));
    }

    #[test]
    fn is_walkable_false_for_void_cells() {
        let grid = grid(&["# #", "#.#"]);
        assert!(!walkable(&grid, 1, 0));
        assert!(walkable(&grid, 1, 1));
    }

    #[test]
    fn walkable_cells_contains_exactly_floor() {
        let cells = walkable_cells();
        assert_eq!(cells, HashSet::from(['.']));
        assert!(!cells.contains(&'#'));
        assert!(!cells.contains(&' '));
    }

    #[test]
    fn full_left_rotation_cycles_back() {
        let mut facing = Facing::N;
        facing = facing.turned_left();
        assert_eq!(facing, Facing::W);
        facing = facing.turned_left();
        assert_eq!(facing, Facing::S);
        facing = facing.turned_left();
        assert_eq!(facing, Facing::E);
        facing = facing.turned_left();
        assert_eq!(facing, Facing::N);
    }

    #[test]
    fn full_right_rotation_cycles_back() {
        let mut facing = Facing::N;
        facing = facing.turned_right();
        assert_eq!(facing, Facing::E);
        facing = facing.turned_right();
        assert_eq!(facing, Facing::S);
        facing = facing.turned_right();
        assert_eq!(facing, Facing::W);
        facing = facing.turned_right();
        assert_eq!(facing, Facing::N);
    }

    #[test]
    fn left_then_right_is_identity() {
        for facing in [Facing::N, Facing::E, Facing::S, Facing::W] {
            assert_eq!(facing.turned_left().turned_right(), facing);
        }
    }

    #[test]
    fn facing_deltas() {
        assert_eq!(Facing::N.delta(), (0, -1));
        assert_eq!(Facing::S.delta(), (0, 1));
        assert_eq!(Facing::E.delta(), (1, 0));
        assert_eq!(Facing::W.delta(), (-1, 0));
    }

    #[test]
    fn player_initializes_at_position_and_facing() {
        let player = PlayerState::new(2, 3, Facing::S);
        assert_eq!(player.col, 2);
        assert_eq!(player.row, 3);
        assert_eq!(player.facing, Facing::S);
    }

    #[test]
    fn move_forward_into_open_cell_succeeds() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(2, 2, Facing::N);
        assert!(player.move_forward(&grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (2, 1));
    }

    #[test]
    fn move_forward_into_wall_fails() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(1, 1, Facing::N);
        assert!(!player.move_forward(&grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (1, 1));
    }

    #[test]
    fn move_back_into_open_cell_succeeds() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(2, 2, Facing::N);
        assert!(player.move_back(&grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (2, 3));
    }

    #[test]
    fn move_back_into_wall_fails() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(1, 3, Facing::N);
        assert!(!player.move_back(&grid, &MoveRules::default()));
        assert_eq!(player.row, 3);
    }

    #[test]
    fn strafe_left_moves_perpendicular() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(2, 2, Facing::N);
        assert!(player.strafe_left(&grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (1, 2));
    }

    #[test]
    fn strafe_right_moves_perpendicular() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(2, 2, Facing::N);
        assert!(player.strafe_right(&grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (3, 2));
    }

    #[test]
    fn strafe_into_wall_fails() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(1, 1, Facing::S);
        assert!(!player.strafe_right(&grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (1, 1));
    }

    #[test]
    fn turn_left_changes_facing_without_moving() {
        let mut player = PlayerState::new(2, 2, Facing::N);
        player.turn_left();
        assert_eq!(player.facing, Facing::W);
        assert_eq!((player.col, player.row), (2, 2));
    }

    #[test]
    fn turn_right_changes_facing_without_moving() {
        let mut player = PlayerState::new(2, 2, Facing::N);
        player.turn_right();
        assert_eq!(player.facing, Facing::E);
        assert_eq!((player.col, player.row), (2, 2));
    }

    #[test]
    fn walk_a_path_forward_turn_right_forward() {
        let grid = boxed_grid();
        let mut player = PlayerState::new(1, 3, Facing::N);
        assert!(player.move_forward(&grid, &MoveRules::default()));
        player.turn_right();
        assert!(player.move_forward(&grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (2, 2));
        assert_eq!(player.facing, Facing::E);
    }

    #[test]
    fn move_forward_from_edge_out_of_bounds_fails() {
        let tiny_grid = grid(&["."]);
        let mut player = PlayerState::new(0, 0, Facing::N);
        assert!(!player.move_forward(&tiny_grid, &MoveRules::default()));
        assert_eq!((player.col, player.row), (0, 0));
    }

    #[test]
    fn moves_onto_custom_walkable_chars() {
        let custom_grid = grid(&["#####", "#.b.#", "#####"]);
        let walkable = build_walkable_set([('b', false)]);
        let mut player = PlayerState::with_walkable(1, 1, Facing::E, walkable);
        assert!(player.move_forward(&custom_grid, &MoveRules::default()));
        assert_eq!(player.col, 2);
    }

    #[test]
    fn cannot_move_onto_solid_char_def_chars() {
        let custom_grid = grid(&["#####", "#.@.#", "#####"]);
        let walkable = build_walkable_set([('@', true)]);
        let mut player = PlayerState::with_walkable(1, 1, Facing::E, walkable);
        assert!(!player.move_forward(&custom_grid, &MoveRules::default()));
        assert_eq!(player.col, 1);
    }

    #[test]
    fn build_walkable_set_defaults_without_char_defs() {
        assert_eq!(build_walkable_set([]), walkable_cells());
    }

    #[test]
    fn build_walkable_set_adds_walkable_chars() {
        let set = build_walkable_set([('b', false), ('m', false)]);
        assert!(set.contains(&'b'));
        assert!(set.contains(&'m'));
        assert!(set.contains(&'.'));
    }

    #[test]
    fn build_walkable_set_skips_solid_chars() {
        let set = build_walkable_set([('@', true), ('b', false)]);
        assert!(!set.contains(&'@'));
        assert!(set.contains(&'b'));
    }

    #[test]
    fn is_walkable_uses_custom_set() {
        let custom_grid = grid(&["#b#"]);
        let walkable = HashSet::from(['.', 'b']);
        assert!(is_walkable(&custom_grid, 1, 0, &walkable, None, None));
        assert!(!is_walkable(&custom_grid, 0, 0, &walkable, None, None));
    }

    fn door_grid() -> Vec<String> {
        grid(&["#####", "#...#", "#...#", "#####"])
    }

    #[test]
    fn walkable_cell_with_open_door_callback() {
        let grid = door_grid();
        assert!(is_walkable(
            &grid,
            2,
            1,
            &walkable_cells(),
            Some(&|_, _| true),
            None
        ));
    }

    #[test]
    fn walkable_cell_with_closed_door_callback() {
        let grid = door_grid();
        assert!(!is_walkable(
            &grid,
            2,
            1,
            &walkable_cells(),
            Some(&|_, _| false),
            None
        ));
    }

    #[test]
    fn walkable_cell_with_no_callback() {
        let grid = door_grid();
        assert!(walkable(&grid, 2, 1));
    }

    #[test]
    fn door_callback_is_called_for_walkable_cells() {
        let grid = door_grid();
        let called = std::cell::Cell::new(false);
        let callback = |_: i32, _: i32| {
            called.set(true);
            true
        };
        assert!(is_walkable(
            &grid,
            1,
            1,
            &walkable_cells(),
            Some(&callback),
            None
        ));
        assert!(called.get());
    }

    #[test]
    fn player_can_walk_through_open_door() {
        let grid = door_grid();
        let rules = MoveRules {
            is_door_open: Some(&|_, _| true),
            ..MoveRules::default()
        };
        let mut player = PlayerState::new(1, 1, Facing::E);
        assert!(player.move_forward(&grid, &rules));
        assert_eq!(player.col, 2);
    }

    #[test]
    fn player_cannot_walk_through_closed_door() {
        let grid = door_grid();
        let rules = MoveRules {
            is_door_open: Some(&|_, _| false),
            ..MoveRules::default()
        };
        let mut player = PlayerState::new(1, 1, Facing::E);
        assert!(!player.move_forward(&grid, &rules));
        assert_eq!(player.col, 1);
    }

    #[test]
    fn get_facing_cell_all_directions() {
        assert_eq!(get_facing_cell(&PlayerState::new(3, 3, Facing::N)), (3, 2));
        assert_eq!(get_facing_cell(&PlayerState::new(3, 3, Facing::E)), (4, 3));
        assert_eq!(get_facing_cell(&PlayerState::new(3, 3, Facing::S)), (3, 4));
        assert_eq!(get_facing_cell(&PlayerState::new(3, 3, Facing::W)), (2, 3));
    }
}

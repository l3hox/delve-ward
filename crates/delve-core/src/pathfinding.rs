//! BFS grid pathfinding — pure logic.

use std::collections::{HashMap, HashSet, VecDeque};

#[must_use]
pub fn manhattan_distance(col1: i64, row1: i64, col2: i64, row2: i64) -> i64 {
    (col1 - col2).abs() + (row1 - row2).abs()
}

const DIRS: [(i64, i64); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

pub type PathCell = (i64, i64);

/// BFS shortest path from `(from_col, from_row)` to `(to_col, to_row)`.
/// Returns the cells from first step to destination, or `None` if unreachable.
/// `is_passable` checks walkability + closed doors + other blockers; the goal
/// cell is always considered reachable (enemies path TO the player, not onto
/// them).
pub fn find_path(
    grid: &[String],
    from_col: i64,
    from_row: i64,
    to_col: i64,
    to_row: i64,
    is_passable: &dyn Fn(i64, i64) -> bool,
    is_edge_blocked: Option<&dyn Fn(i64, i64, i64, i64) -> bool>,
) -> Option<Vec<PathCell>> {
    if from_col == to_col && from_row == to_row {
        return Some(Vec::new());
    }

    let rows = grid.len() as i64;
    let cols = grid.first().map_or(0, |row| row.chars().count()) as i64;
    let mut visited: HashSet<PathCell> = HashSet::new();
    let mut parent: HashMap<PathCell, PathCell> = HashMap::new();

    let start = (from_col, from_row);
    let goal = (to_col, to_row);

    visited.insert(start);
    let mut queue: VecDeque<PathCell> = VecDeque::from([start]);

    while let Some((col, row)) = queue.pop_front() {
        for (dcol, drow) in DIRS {
            let next = (col + dcol, row + drow);
            if next.0 < 0 || next.0 >= cols || next.1 < 0 || next.1 >= rows {
                continue;
            }
            if is_edge_blocked.is_some_and(|blocked| blocked(col, row, next.0, next.1)) {
                continue;
            }
            if visited.contains(&next) {
                continue;
            }
            visited.insert(next);

            let passable = next == goal || is_passable(next.0, next.1);
            if !passable {
                continue;
            }

            parent.insert(next, (col, row));

            if next == goal {
                let mut path = Vec::new();
                let mut current = goal;
                while current != start {
                    path.push(current);
                    current = parent[&current];
                }
                path.reverse();
                return Some(path);
            }

            queue.push_back(next);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> Vec<String> {
        rows.iter().map(ToString::to_string).collect()
    }

    fn passable_in(rows: &'static [&'static str]) -> impl Fn(i64, i64) -> bool {
        move |col, row| {
            if row < 0 || row >= rows.len() as i64 {
                return false;
            }
            let line = rows[row as usize];
            if col < 0 || col >= line.len() as i64 {
                return false;
            }
            line.as_bytes()[col as usize] == b'.'
        }
    }

    #[test]
    fn manhattan_distance_cases() {
        assert_eq!(manhattan_distance(3, 4, 3, 4), 0);
        assert_eq!(manhattan_distance(1, 0, 4, 0), 3);
        assert_eq!(manhattan_distance(0, 1, 0, 5), 4);
        assert_eq!(manhattan_distance(1, 1, 4, 5), 7);
    }

    const GRID: [&str; 5] = ["#####", "#...#", "#.#.#", "#...#", "#####"];

    #[test]
    fn returns_empty_path_when_start_equals_end() {
        let path = find_path(&grid(&GRID), 1, 1, 1, 1, &passable_in(&GRID), None);
        assert_eq!(path, Some(Vec::new()));
    }

    #[test]
    fn finds_straight_line_path_in_open_corridor() {
        const CORRIDOR: [&str; 3] = ["#####", "#...#", "#####"];
        let path = find_path(&grid(&CORRIDOR), 1, 1, 3, 1, &passable_in(&CORRIDOR), None)
            .expect("path exists");
        assert_eq!(path, vec![(2, 1), (3, 1)]);
    }

    #[test]
    fn finds_path_around_a_wall() {
        let path =
            find_path(&grid(&GRID), 1, 1, 3, 1, &passable_in(&GRID), None).expect("path exists");
        assert_eq!(path.len(), 2);
        assert_eq!(path.last(), Some(&(3, 1)));
    }

    #[test]
    fn returns_none_when_target_unreachable() {
        const ISOLATED: [&str; 3] = ["#####", "#.#.#", "#####"];
        let path = find_path(&grid(&ISOLATED), 1, 1, 3, 1, &passable_in(&ISOLATED), None);
        assert!(path.is_none());
    }

    #[test]
    fn respects_passable_callback() {
        let block_middle = |col: i64, row: i64| {
            if row == 2 {
                return false;
            }
            passable_in(&GRID)(col, row)
        };
        let path = find_path(&grid(&GRID), 1, 1, 1, 3, &block_middle, None);
        assert!(path.is_none());
    }

    #[test]
    fn finds_shortest_path() {
        const OPEN: [&str; 5] = ["#######", "#.....#", "#.....#", "#.....#", "#######"];
        let path =
            find_path(&grid(&OPEN), 1, 1, 5, 1, &passable_in(&OPEN), None).expect("path exists");
        assert_eq!(path.len(), 4);
    }

    #[test]
    fn routes_around_a_thin_wall_edge() {
        const CORRIDOR: [&str; 5] = ["#####", "#...#", "#...#", "#...#", "#####"];
        let edge_blocked = |from_col: i64, from_row: i64, to_col: i64, to_row: i64| {
            (from_col == 1 && from_row == 1 && to_col == 2 && to_row == 1)
                || (from_col == 2 && from_row == 1 && to_col == 1 && to_row == 1)
        };
        let path = find_path(
            &grid(&CORRIDOR),
            1,
            1,
            3,
            1,
            &passable_in(&CORRIDOR),
            Some(&edge_blocked),
        )
        .expect("path exists");
        assert_ne!(path[0], (2, 1));
        assert_eq!(path.last(), Some(&(3, 1)));
    }
}

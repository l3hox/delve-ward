//! Ported from `src/core/projectileManager.test.ts`. Every `it` block there
//! has a matching `#[test]` here (the `it.each` facing-direction block
//! becomes four separate tests, one per direction).

use delve_core::grid::Facing;
use delve_core::projectiles::{
    HitType, ProjectileManager, ProjectileUpdateContext, SpawnOptions, projectile_stats,
};

// Simple 5x5 grid: walls on edges, open floor inside (cols 1-3, rows 1-3).
fn is_walkable_bounded(col: i64, row: i64) -> bool {
    (1..=3).contains(&col) && (1..=3).contains(&row)
}

// No doors by default — every cell is open.
fn is_door_open_always(_col: i64, _row: i64) -> bool {
    true
}

// No enemies by default.
fn no_enemies(_col: i64, _row: i64) -> bool {
    false
}

// Large open space so walls/doors never interfere.
fn open_field(_col: i64, _row: i64) -> bool {
    true
}

// Player parked far away so it doesn't interfere unless a test moves it.
const FAR: i64 = 99;

fn bounded_context() -> ProjectileUpdateContext<'static> {
    ProjectileUpdateContext {
        is_walkable: &is_walkable_bounded,
        is_door_open: &is_door_open_always,
        player_col: FAR,
        player_row: FAR,
        is_enemy_at: Some(&no_enemies),
        is_block_at: None,
        is_solid_edge_blocked: None,
        layer_filter: None,
    }
}

fn open_context() -> ProjectileUpdateContext<'static> {
    ProjectileUpdateContext {
        is_walkable: &open_field,
        is_door_open: &is_door_open_always,
        player_col: FAR,
        player_row: FAR,
        is_enemy_at: Some(&no_enemies),
        is_block_at: None,
        is_solid_edge_blocked: None,
        layer_filter: None,
    }
}

fn dart_options(col: i64, row: i64, direction: Facing) -> SpawnOptions<'static> {
    SpawnOptions {
        col,
        row,
        direction,
        projectile_type: "dart",
        source: None,
        max_range: None,
        layer_index: None,
    }
}

// --- spawn() ---

#[test]
fn creates_a_projectile_with_stats_from_projectile_stats() {
    let mut pm = ProjectileManager::new();
    let p = pm
        .spawn(dart_options(2, 2, Facing::N))
        .expect("dart spawns");

    let dart = projectile_stats("dart").expect("dart stats exist");
    assert_eq!(p.projectile_type, "dart");
    assert_eq!(p.speed, dart.speed);
    assert_eq!(p.damage, dart.damage);
    assert_eq!(p.damage_type, dart.damage_type);
    assert_eq!(p.max_range, dart.max_range);
}

#[test]
fn spawns_at_wall_edge_offset_toward_launcher() {
    let mut pm = ProjectileManager::new();
    let e = pm
        .spawn(SpawnOptions {
            col: 2,
            row: 3,
            direction: Facing::E,
            projectile_type: "arrow",
            source: None,
            max_range: None,
            layer_index: None,
        })
        .expect("arrow spawns");
    assert!((e.col - 2.05).abs() < 1e-5); // offset toward west wall (launcher side)
    assert!((e.row - 3.5).abs() < 1e-5);

    let mut pm2 = ProjectileManager::new();
    let n = pm2
        .spawn(dart_options(5, 5, Facing::N))
        .expect("dart spawns");
    assert!((n.col - 5.5).abs() < 1e-5);
    assert!((n.row - 5.95).abs() < 1e-5); // offset toward south wall
}

#[test]
fn assigns_a_unique_id_per_projectile() {
    let mut pm = ProjectileManager::new();
    let a = pm.spawn(dart_options(1, 1, Facing::N)).expect("spawns");
    let b = pm.spawn(dart_options(1, 1, Facing::S)).expect("spawns");
    assert_ne!(a.id, b.id);
}

#[test]
fn initializes_traveled_to_zero() {
    let mut pm = ProjectileManager::new();
    let p = pm.spawn(dart_options(1, 1, Facing::N)).expect("spawns");
    assert_eq!(p.traveled, 0.0);
}

#[test]
fn applies_max_range_override_when_provided() {
    let mut pm = ProjectileManager::new();
    let p = pm
        .spawn(SpawnOptions {
            max_range: Some(5.0),
            ..dart_options(1, 1, Facing::N)
        })
        .expect("spawns");
    assert_eq!(p.max_range, 5.0);
}

#[test]
fn carries_status_effect_from_projectile_stats_fireball() {
    let mut pm = ProjectileManager::new();
    let p = pm
        .spawn(SpawnOptions {
            projectile_type: "fireball",
            ..dart_options(1, 1, Facing::E)
        })
        .expect("fireball spawns");
    assert_eq!(p.status_effect.as_deref(), Some("burning"));
}

#[test]
fn errors_for_an_unknown_projectile_type() {
    let mut pm = ProjectileManager::new();
    let error = pm
        .spawn(SpawnOptions {
            projectile_type: "laser",
            ..dart_options(1, 1, Facing::N)
        })
        .expect_err("unknown type fails");
    assert!(error.contains("laser"));
}

// --- movement ---

#[test]
fn dart_moving_n_travels_one_cell_in_one_over_speed_seconds() {
    let mut pm = ProjectileManager::new();
    let p = pm.spawn(dart_options(2, 2, Facing::N)).expect("spawns");
    let start_row = p.row; // 2.95

    pm.update(0.125, &bounded_context());

    let restored = &pm.get_all()[0];
    assert!((restored.row - (start_row - 1.0)).abs() < 1e-5);
    assert!((restored.col - 2.5).abs() < 1e-5);
}

#[test]
fn advances_traveled_by_speed_times_delta() {
    let mut pm = ProjectileManager::new();
    pm.spawn(dart_options(2, 2, Facing::E)).expect("spawns");

    pm.update(0.1, &bounded_context());

    let p = &pm.get_all()[0];
    let dart = projectile_stats("dart").expect("dart stats exist");
    assert!((p.traveled - dart.speed * 0.1).abs() < 1e-5);
}

// --- wall collision ---

#[test]
fn removes_projectile_and_fires_wall_event_when_entering_a_wall_cell() {
    let mut pm = ProjectileManager::new();
    // Start at col=1, row=2 facing W. After moving left it will hit col=0 (wall).
    pm.spawn(dart_options(1, 2, Facing::W)).expect("spawns");

    // Advance enough to cross into col=0.
    let events = pm.update(0.2, &bounded_context());

    assert!(pm.get_all().is_empty());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].hit_type, HitType::Wall);
    assert_eq!(events[0].col, 0);
    assert_eq!(events[0].projectile.projectile_type, "dart");
}

// --- door collision ---

#[test]
fn removes_projectile_and_fires_door_event_for_closed_door() {
    fn closed_door_at_col_2(col: i64, _row: i64) -> bool {
        col != 2
    }

    let mut pm = ProjectileManager::new();
    pm.spawn(dart_options(1, 2, Facing::E)).expect("spawns");

    let context = ProjectileUpdateContext {
        is_door_open: &closed_door_at_col_2,
        ..bounded_context()
    };
    let events = pm.update(0.2, &context);

    assert!(pm.get_all().is_empty());
    assert_eq!(events[0].hit_type, HitType::Door);
}

// --- range expiry ---

#[test]
fn removes_projectile_after_traveling_max_range_with_no_event() {
    let mut pm = ProjectileManager::new();
    // Override maxRange to 1 so it expires quickly.
    pm.spawn(SpawnOptions {
        max_range: Some(1.0),
        ..dart_options(2, 2, Facing::N)
    })
    .expect("spawns");

    // Advance 0.2s — dart at speed 8 travels 1.6 cells, past maxRange=1.
    let events = pm.update(0.2, &open_context());

    assert!(pm.get_all().is_empty());
    assert!(events.is_empty());
}

// --- player collision ---

#[test]
fn fires_player_event_when_projectile_enters_the_player_cell() {
    let mut pm = ProjectileManager::new();
    // Dart at col=1,row=2 facing E. Player is at col=2,row=2.
    pm.spawn(dart_options(1, 2, Facing::E)).expect("spawns");

    let context = ProjectileUpdateContext {
        player_col: 2,
        player_row: 2,
        ..open_context()
    };
    let events = pm.update(0.2, &context);

    assert!(pm.get_all().is_empty());
    assert_eq!(events[0].hit_type, HitType::Player);
}

// --- facing directions ---
// 0.125s at speed 8 = exactly 1 cell.

#[test]
fn facing_n_moves_row_decreases() {
    let mut pm = ProjectileManager::new();
    pm.spawn(SpawnOptions {
        max_range: Some(99.0),
        ..dart_options(5, 5, Facing::N)
    })
    .expect("spawns");
    pm.update(0.125, &open_context());
    let p = &pm.get_all()[0];
    assert_eq!(p.col.floor() as i64, 5);
    assert_eq!(p.row.floor() as i64, 4);
}

#[test]
fn facing_s_moves_row_increases() {
    let mut pm = ProjectileManager::new();
    pm.spawn(SpawnOptions {
        max_range: Some(99.0),
        ..dart_options(5, 5, Facing::S)
    })
    .expect("spawns");
    pm.update(0.125, &open_context());
    let p = &pm.get_all()[0];
    assert_eq!(p.col.floor() as i64, 5);
    assert_eq!(p.row.floor() as i64, 6);
}

#[test]
fn facing_e_moves_col_increases() {
    let mut pm = ProjectileManager::new();
    pm.spawn(SpawnOptions {
        max_range: Some(99.0),
        ..dart_options(5, 5, Facing::E)
    })
    .expect("spawns");
    pm.update(0.125, &open_context());
    let p = &pm.get_all()[0];
    assert_eq!(p.col.floor() as i64, 6);
    assert_eq!(p.row.floor() as i64, 5);
}

#[test]
fn facing_w_moves_col_decreases() {
    let mut pm = ProjectileManager::new();
    pm.spawn(SpawnOptions {
        max_range: Some(99.0),
        ..dart_options(5, 5, Facing::W)
    })
    .expect("spawns");
    pm.update(0.125, &open_context());
    let p = &pm.get_all()[0];
    assert_eq!(p.col.floor() as i64, 4);
    assert_eq!(p.row.floor() as i64, 5);
}

// --- clear() ---

#[test]
fn clear_removes_all_projectiles() {
    let mut pm = ProjectileManager::new();
    pm.spawn(dart_options(1, 1, Facing::N)).expect("spawns");
    pm.spawn(SpawnOptions {
        projectile_type: "arrow",
        ..dart_options(2, 2, Facing::S)
    })
    .expect("spawns");
    assert_eq!(pm.get_all().len(), 2);

    pm.clear();

    assert!(pm.get_all().is_empty());
}

// --- saveState() / loadState() ---

#[test]
fn roundtrip_preserves_all_projectile_fields() {
    let mut pm = ProjectileManager::new();
    let p = pm
        .spawn(SpawnOptions {
            projectile_type: "fireball",
            ..dart_options(2, 1, Facing::S)
        })
        .expect("spawns");

    let snapshot = pm.save_state();
    let mut pm2 = ProjectileManager::new();
    pm2.load_state(snapshot);

    let restored = &pm2.get_all()[0];
    assert_eq!(restored.id, p.id);
    assert_eq!(restored.col, p.col);
    assert_eq!(restored.row, p.row);
    assert_eq!(restored.direction, p.direction);
    assert_eq!(restored.speed, p.speed);
    assert_eq!(restored.damage, p.damage);
    assert_eq!(restored.damage_type, p.damage_type);
    assert_eq!(restored.status_effect, p.status_effect);
    assert_eq!(restored.projectile_type, p.projectile_type);
    assert_eq!(restored.traveled, p.traveled);
    assert_eq!(restored.max_range, p.max_range);
}

#[test]
fn load_state_replaces_existing_state() {
    let mut pm = ProjectileManager::new();
    pm.spawn(dart_options(1, 1, Facing::N)).expect("spawns");
    let snapshot = pm.save_state();

    let mut pm2 = ProjectileManager::new();
    pm2.spawn(SpawnOptions {
        projectile_type: "arrow",
        ..dart_options(3, 3, Facing::W)
    })
    .expect("spawns");
    pm2.spawn(SpawnOptions {
        projectile_type: "arrow",
        ..dart_options(2, 2, Facing::E)
    })
    .expect("spawns");
    assert_eq!(pm2.get_all().len(), 2);

    pm2.load_state(snapshot);
    assert_eq!(pm2.get_all().len(), 1);
}

#[test]
fn save_state_snapshot_is_independent_of_the_manager() {
    let mut pm = ProjectileManager::new();
    pm.spawn(dart_options(1, 1, Facing::N)).expect("spawns");
    let mut snapshot = pm.save_state();

    snapshot[0].damage = 9999.0;

    let dart = projectile_stats("dart").expect("dart stats exist");
    let original = &pm.get_all()[0];
    assert_eq!(original.damage, dart.damage);
}

// --- hit events ---

#[test]
fn hit_event_carries_correct_hit_type_cell_coords_and_projectile() {
    let mut pm = ProjectileManager::new();
    // Dart facing north from (2,1) — next cell is (2,0) which is a wall.
    pm.spawn(dart_options(2, 1, Facing::N)).expect("spawns");

    let events = pm.update(0.2, &bounded_context());

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].hit_type, HitType::Wall);
    assert_eq!(events[0].col, 2);
    assert_eq!(events[0].row, 0);
    assert_eq!(events[0].projectile.projectile_type, "dart");
}

#[test]
fn enemy_hit_fires_event_with_enemy_type() {
    fn enemy_at_3_5(col: i64, row: i64) -> bool {
        col == 3 && row == 5
    }

    let mut pm = ProjectileManager::new();
    // Enemy at (3,5). Dart at (2,5) facing E.
    pm.spawn(dart_options(2, 5, Facing::E)).expect("spawns");

    let context = ProjectileUpdateContext {
        is_enemy_at: Some(&enemy_at_3_5),
        ..open_context()
    };
    let events = pm.update(0.2, &context);

    assert!(pm.get_all().is_empty());
    assert_eq!(events[0].hit_type, HitType::Enemy);
}

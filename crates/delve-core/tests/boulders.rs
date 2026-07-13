//! `boulderSystem.ts` has no dedicated vitest suite (it's a `game/`
//! orchestrator). This is a from-scratch behavioral spec for
//! `delve_core::boulders`, covering rolling, falling, ramp descent,
//! chain-pushing, hole-plugging, chest crashes, and enemy/player damage.

use delve_core::boulders::{
    BoulderContext, BoulderEvent, BoulderTransitionKind, tick_boulder_spawners, tick_boulders,
};
use delve_core::game_state::{
    BoulderInstance, BoulderSpawnerInstance, BoulderState, ChestInstance, ChestState, GameState,
    GameStateDeps, IntervalMode, RampInstance, RampStyle, door_key,
};
use delve_core::grid::{Facing, walkable_cells};
use delve_core::player_controller::PlayerTickState;
use delve_core::types::{EnemyAiState, EnemyInstance, LayerDef};

fn rows(rows: &[&str]) -> Vec<String> {
    rows.iter().map(ToString::to_string).collect()
}

fn open_support_grid() -> Vec<String> {
    rows(&["#####", "#####", "#####", "#####", "#####"])
}

fn open_room_grid() -> Vec<String> {
    rows(&["#####", "#...#", "#...#", "#...#", "#####"])
}

/// A support grid with a single open cell at (2, 2) — a hole for the room
/// layer above it — everywhere else solid.
fn one_hole_support_grid() -> Vec<String> {
    rows(&["#####", "#####", "##.##", "#####", "#####"])
}

fn layer_def(grid: Vec<String>) -> LayerDef {
    LayerDef {
        id: None,
        y_offset: None,
        grid,
        entities: Vec::new(),
        ceiling: None,
        defaults: None,
        areas: None,
    }
}

/// Layer 0 is the support/landing floor, layer 1 is the room under test.
/// `GameState::new` resets `active_layer_index` to 0 after parsing every
/// layer's (empty) entities — point it at layer 1 to match every test's
/// `active_layer_mut()`/`active_layer()` calls.
fn game_with_layers(below: Vec<String>, above: Vec<String>) -> GameState {
    let layers = [layer_def(below), layer_def(above)];
    let mut game = GameState::new(
        &[],
        None,
        "test",
        Some(&layers),
        GameStateDeps::default(),
        &mut || 0.5,
    );
    game.active_layer_index = 1;
    game
}

fn make_boulder(col: i64, row: i64, direction: Facing, state: BoulderState) -> BoulderInstance {
    BoulderInstance {
        id: None,
        col,
        row,
        direction,
        state,
        gate_mode: None,
        roll_damage: 5.0,
        fall_damage: 10.0,
        insta_kill_enemies: false,
        pushable: true,
    }
}

fn make_enemy(col: i64, row: i64, hp: f64) -> EnemyInstance {
    EnemyInstance {
        col,
        row,
        enemy_type: "rat_test".to_string(),
        hp,
        max_hp: hp,
        atk: 2.0,
        def: 0.0,
        aggro_range: 3.0,
        move_interval: 0.6,
        blocks_movement: true,
        ai_state: EnemyAiState::Idle,
        move_timer: 0.0,
        regen_timer: None,
        regen_pause_timer: None,
        drops: None,
        status_effects: Vec::new(),
        spawner_id: None,
    }
}

fn always_resting(_key: &str) -> bool {
    true
}

fn never_resting(_key: &str) -> bool {
    false
}

fn context<'a>(
    layer_defs: &'a [LayerDef],
    walkable: &'a std::collections::HashSet<char>,
    is_resting: &'a dyn Fn(&str) -> bool,
) -> BoulderContext<'a> {
    BoulderContext {
        layer_defs,
        level_areas: &[],
        char_defs: &[],
        walkable,
        player_layer: 1,
        player_col: -99,
        player_row: -99,
        debug_fullbright: false,
        is_resting,
    }
}

// ---------------------------------------------------------------------------
// Rolling
// ---------------------------------------------------------------------------

#[test]
fn rolling_into_open_cell_moves_and_emits_a_rolled_event_with_no_trigger_activation() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert_eq!(events.len(), 1);
    let BoulderEvent::Moved(moved) = &events[0] else {
        panic!("expected a Moved event, got {:?}", events[0]);
    };
    assert_eq!(moved.kind, BoulderTransitionKind::Rolled);
    assert_eq!((moved.col, moved.row), (2, 2));
    assert_eq!(moved.new_layer_index, 1);
    assert!(!moved.tripwire_activated);
    assert!(!moved.plate_activated);
    assert!(!game.active_layer().boulders.contains_key(&door_key(1, 2)));
    let moved_boulder = game
        .active_layer()
        .boulders
        .get(&door_key(2, 2))
        .expect("boulder moved");
    assert_eq!(moved_boulder.state, BoulderState::Rolling);
}

#[test]
fn rolling_into_a_wall_with_both_sides_open_goes_idle() {
    // At (2,1) facing N: forward is the north wall; both W (0,1) and E
    // (4,1) neighbors are open interior cells.
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(2, 1),
        make_boulder(2, 1, Facing::N, BoulderState::Rolling),
    );

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert!(events.is_empty());
    let boulder = game.active_layer().boulders.get(&door_key(2, 1)).unwrap();
    assert_eq!(boulder.state, BoulderState::Idle);
}

#[test]
fn is_resting_false_leaves_the_boulder_completely_untouched() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &never_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert!(events.is_empty());
    let boulder = game
        .active_layer()
        .boulders
        .get(&door_key(1, 2))
        .expect("boulder untouched");
    assert_eq!(boulder.state, BoulderState::Rolling);
    assert_eq!((boulder.col, boulder.row), (1, 2));
}

// ---------------------------------------------------------------------------
// Falling
// ---------------------------------------------------------------------------

#[test]
fn idle_boulder_over_a_fresh_hole_starts_falling() {
    let mut game = game_with_layers(one_hole_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::E, BoulderState::Idle),
    );

    let layer_defs = [
        layer_def(one_hole_support_grid()),
        layer_def(open_room_grid()),
    ];
    let walkable = walkable_cells();
    // Realistic shells report the boulder's fresh post-fall key as "not
    // resting" (the fall tween just started) — that's what stops pass 2
    // from immediately advancing it again in the same tick. Only the
    // boulder's original (pre-fall) key is resting here, isolating pass
    // 1's transition from pass 2's landing-and-resume logic.
    let original_key = delve_core::game_state::layer_door_key(1, &door_key(2, 2));
    let is_resting = move |key: &str| key == original_key;
    let ctx = context(&layer_defs, &walkable, &is_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    let fell = events.iter().find_map(|event| match event {
        BoulderEvent::Moved(moved) if moved.kind == BoulderTransitionKind::Fell => Some(moved),
        _ => None,
    });
    let fell = fell.expect("expected a Fell event");
    assert_eq!(fell.new_layer_index, 0);
    assert_eq!((fell.col, fell.row), (2, 2));

    game.active_layer_index = 0;
    let landed = game
        .active_layer()
        .boulders
        .get(&door_key(2, 2))
        .expect("boulder landed on layer 0");
    assert_eq!(landed.state, BoulderState::Falling);
    assert!(game.layers[1].boulders.is_empty());
}

#[test]
fn hole_plugged_by_another_boulder_below_is_not_treated_as_a_hole() {
    let mut game = game_with_layers(one_hole_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::E, BoulderState::Idle),
    );
    // A second boulder plugs the hole from below, on layer 0.
    game.layers[0].boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::N, BoulderState::Idle),
    );

    let layer_defs = [
        layer_def(one_hole_support_grid()),
        layer_def(open_room_grid()),
    ];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert!(!events.iter().any(|event| matches!(
        event,
        BoulderEvent::Moved(moved) if moved.kind == BoulderTransitionKind::Fell
    )));
    let boulder = game
        .active_layer()
        .boulders
        .get(&door_key(2, 2))
        .expect("boulder stayed put");
    assert_eq!(boulder.state, BoulderState::Idle);
}

#[test]
fn falling_boulder_deals_fall_damage_to_a_colocated_enemy_and_resumes_rolling() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::N, BoulderState::Falling),
    );
    game.active_layer_mut()
        .enemies
        .insert(door_key(2, 2), make_enemy(2, 2, 100.0));

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    let damaged = events.iter().find_map(|event| match event {
        BoulderEvent::EnemyDamaged {
            damage,
            killed,
            layer_index,
            ..
        } => Some((*damage, *killed, *layer_index)),
        _ => None,
    });
    assert_eq!(damaged, Some((10.0, false, 1)));
    let enemy = game
        .active_layer()
        .enemies
        .get(&door_key(2, 2))
        .expect("enemy survived");
    assert_eq!(enemy.hp, 90.0);
}

#[test]
fn falling_boulder_deals_fall_damage_to_the_player_unless_debug_fullbright() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::N, BoulderState::Falling),
    );
    game.player.hp = 50.0;

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let mut ctx = context(&layer_defs, &walkable, &always_resting);
    ctx.player_col = 2;
    ctx.player_row = 2;
    let mut tick_state = PlayerTickState::default();

    tick_boulders(&mut game, &ctx, &mut tick_state);
    assert_eq!(game.player.hp, 40.0);
    assert_eq!(
        tick_state.player_damage_flash_timer,
        delve_core::player_controller::PLAYER_DAMAGE_FLASH_DURATION
    );

    // Rebuild identically but with debug_fullbright on — no damage this time.
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::N, BoulderState::Falling),
    );
    game.player.hp = 50.0;
    ctx.debug_fullbright = true;
    let mut tick_state = PlayerTickState::default();
    tick_boulders(&mut game, &ctx, &mut tick_state);
    assert_eq!(game.player.hp, 50.0);
}

// ---------------------------------------------------------------------------
// Ramps
// ---------------------------------------------------------------------------

#[test]
fn ramp_descent_moves_the_boulder_one_layer_down_to_the_ramps_bottom_cell() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::S, BoulderState::Rolling),
    );
    // Ramp on layer 0: top cell = (2,2) + N-delta(0,-1) = (2,1)... instead
    // place the ramp so its top cell is exactly the boulder's cell (2,2),
    // facing N (so its "up" direction, (0,-1), is the reverse of the
    // boulder's S direction, (0,1)).
    game.layers[0].ramps.insert(
        door_key(2, 3),
        RampInstance {
            id: None,
            col: 2,
            row: 3,
            facing: Facing::N,
            style: RampStyle::Ramp,
        },
    );

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert_eq!(events.len(), 1);
    let BoulderEvent::Moved(moved) = &events[0] else {
        panic!("expected a Moved event, got {:?}", events[0]);
    };
    assert_eq!(moved.kind, BoulderTransitionKind::Descended);
    assert_eq!(moved.new_layer_index, 0);
    assert_eq!((moved.col, moved.row), (2, 3));
    assert!(game.layers[1].boulders.is_empty());
    let landed = game.layers[0]
        .boulders
        .get(&door_key(2, 3))
        .expect("boulder descended");
    assert_eq!(landed.state, BoulderState::Rolling);
}

// ---------------------------------------------------------------------------
// Enemy / player collisions while rolling
// ---------------------------------------------------------------------------

#[test]
fn rolling_into_an_enemy_damages_but_does_not_kill_it() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );
    game.active_layer_mut()
        .enemies
        .insert(door_key(2, 2), make_enemy(2, 2, 100.0));

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert!(events.iter().any(|event| matches!(
        event,
        BoulderEvent::EnemyDamaged { damage, killed: false, .. } if *damage == 5.0
    )));
    let enemy = game
        .active_layer()
        .enemies
        .get(&door_key(2, 2))
        .expect("enemy survived");
    assert_eq!(enemy.hp, 95.0);
    // A non-fatal `damage_enemy` result still lets the boulder advance into
    // the enemy's cell — only a `blocked` result stops it. Only `blocked`
    // (wall, closed door, another block/boulder) stops the roll.
    assert!(!game.active_layer().boulders.contains_key(&door_key(1, 2)));
    assert!(game.active_layer().boulders.contains_key(&door_key(2, 2)));
}

#[test]
fn rolling_into_an_enemy_kills_it_when_damage_meets_or_exceeds_hp() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );
    game.active_layer_mut()
        .enemies
        .insert(door_key(2, 2), make_enemy(2, 2, 3.0));

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert!(
        events
            .iter()
            .any(|event| matches!(event, BoulderEvent::EnemyDamaged { killed: true, .. }))
    );
    assert!(!game.active_layer().enemies.contains_key(&door_key(2, 2)));
}

#[test]
fn insta_kill_enemies_removes_the_enemy_with_no_damage_number() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    let mut boulder = make_boulder(1, 2, Facing::E, BoulderState::Rolling);
    boulder.insta_kill_enemies = true;
    game.active_layer_mut()
        .boulders
        .insert(door_key(1, 2), boulder);
    game.active_layer_mut()
        .enemies
        .insert(door_key(2, 2), make_enemy(2, 2, 1000.0));

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    let insta_killed_layer = events.iter().find_map(|event| match event {
        BoulderEvent::EnemyInstaKilled { layer_index, .. } => Some(*layer_index),
        _ => None,
    });
    assert_eq!(insta_killed_layer, Some(1));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, BoulderEvent::EnemyDamaged { .. }))
    );
    assert!(!game.active_layer().enemies.contains_key(&door_key(2, 2)));
}

/// A three-layer setup with the boulder/enemy on layer 2 (not layer 0 or the
/// two-layer fixtures' layer 1) — pins `EnemyDamaged`/`EnemyInstaKilled`'s
/// `layer_index` field to the actual layer the collision happened on,
/// rather than a value that would coincidentally match a hardcoded 0 or 1.
#[test]
fn rolling_into_an_enemy_on_a_non_ground_layer_tags_events_with_that_layer() {
    let layer_defs = [
        layer_def(open_support_grid()),
        layer_def(open_support_grid()),
        layer_def(open_room_grid()),
    ];
    let mut game = GameState::new(
        &[],
        None,
        "test",
        Some(&layer_defs),
        GameStateDeps::default(),
        &mut || 0.5,
    );
    game.active_layer_index = 2;
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );
    game.active_layer_mut()
        .enemies
        .insert(door_key(2, 2), make_enemy(2, 2, 3.0));

    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    let damaged_layer = events.iter().find_map(|event| match event {
        BoulderEvent::EnemyDamaged { layer_index, .. } => Some(*layer_index),
        _ => None,
    });
    assert_eq!(damaged_layer, Some(2));
}

#[test]
fn rolling_into_the_player_deals_roll_damage_and_sets_the_flash_timer() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );
    game.player.hp = 20.0;

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let mut ctx = context(&layer_defs, &walkable, &always_resting);
    ctx.player_col = 2;
    ctx.player_row = 2;
    let mut tick_state = PlayerTickState::default();

    tick_boulders(&mut game, &ctx, &mut tick_state);

    assert_eq!(game.player.hp, 15.0);
    assert_eq!(
        tick_state.player_damage_flash_timer,
        delve_core::player_controller::PLAYER_DAMAGE_FLASH_DURATION
    );
}

// ---------------------------------------------------------------------------
// Chests and chain-pushing
// ---------------------------------------------------------------------------

#[test]
fn rolling_onto_a_chest_crashes_it_and_reports_its_drops() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );
    game.active_layer_mut().chests.insert(
        door_key(2, 2),
        ChestInstance {
            id: None,
            col: 2,
            row: 2,
            state: ChestState::Closed,
            facing: Facing::N,
            key_id: None,
            gate_mode: None,
            targets: None,
            drops: None,
        },
    );

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    let events = tick_boulders(&mut game, &ctx, &mut tick_state);

    assert!(events.iter().any(|event| matches!(
        event,
        BoulderEvent::ChestCrashed {
            col: 2,
            row: 2,
            drops: None,
            ..
        }
    )));
    assert!(!game.active_layer().chests.contains_key(&door_key(2, 2)));
}

#[test]
fn chain_push_flips_the_blocked_boulder_to_rolling_and_the_pusher_to_idle() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut().boulders.insert(
        door_key(1, 2),
        make_boulder(1, 2, Facing::E, BoulderState::Rolling),
    );
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::N, BoulderState::Idle),
    );

    let layer_defs = [layer_def(open_support_grid()), layer_def(open_room_grid())];
    let walkable = walkable_cells();
    let ctx = context(&layer_defs, &walkable, &always_resting);
    let mut tick_state = PlayerTickState::default();

    // HashMap iteration order is unspecified, so same-tick propagation from
    // the pusher to the pushed boulder isn't guaranteed within one call
    // (TS's insertion-ordered Map makes this deterministic there — this is
    // an accepted, pre-existing divergence from the underlying HashMap
    // storage, not something this module can fix). Running two ticks
    // reaches the same steady state regardless of ordering.
    tick_boulders(&mut game, &ctx, &mut tick_state);
    tick_boulders(&mut game, &ctx, &mut tick_state);

    let pusher = game
        .active_layer()
        .boulders
        .get(&door_key(1, 2))
        .expect("pusher never moves");
    assert_eq!(pusher.state, BoulderState::Idle);
    assert!(!game.active_layer().boulders.contains_key(&door_key(2, 2)));
    let pushed = game
        .active_layer()
        .boulders
        .get(&door_key(3, 2))
        .expect("pushed boulder advanced");
    assert_eq!(pushed.direction, Facing::E);
}

// ---------------------------------------------------------------------------
// Boulder spawners
// ---------------------------------------------------------------------------

fn make_boulder_spawner(col: i64, row: i64) -> BoulderSpawnerInstance {
    BoulderSpawnerInstance {
        id: None,
        col,
        row,
        direction: Facing::S,
        interval_mode: IntervalMode::Fixed,
        interval: 1.0,
        interval_min: 1.0,
        interval_max: 3.0,
        active: true,
        gate_mode: None,
        spawn_timer: 0.0,
        next_interval: 1.0,
        roll_damage: 5.0,
        fall_damage: 10.0,
        insta_kill_enemies: false,
        pushable: true,
    }
}

#[test]
fn boulder_spawner_spawns_a_rolling_boulder_when_its_cell_is_free() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut()
        .boulder_spawners
        .insert(door_key(2, 2), make_boulder_spawner(2, 2));

    let events = tick_boulder_spawners(&mut game, 2.0, &mut || 0.5);

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        BoulderEvent::Spawned { col: 2, row: 2, .. }
    ));
    let boulder = game
        .active_layer()
        .boulders
        .get(&door_key(2, 2))
        .expect("boulder spawned");
    assert_eq!(boulder.state, BoulderState::Rolling);
    assert_eq!(boulder.direction, Facing::S);
}

#[test]
fn boulder_spawner_skips_when_its_cell_already_holds_a_boulder() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    game.active_layer_mut()
        .boulder_spawners
        .insert(door_key(2, 2), make_boulder_spawner(2, 2));
    game.active_layer_mut().boulders.insert(
        door_key(2, 2),
        make_boulder(2, 2, Facing::S, BoulderState::Idle),
    );

    let events = tick_boulder_spawners(&mut game, 2.0, &mut || 0.5);

    assert!(events.is_empty());
}

#[test]
fn boulder_spawner_random_interval_mode_draws_within_min_max_bounds() {
    let mut game = game_with_layers(open_support_grid(), open_room_grid());
    let mut spawner = make_boulder_spawner(2, 2);
    spawner.interval_mode = IntervalMode::Random;
    spawner.interval_min = 2.0;
    spawner.interval_max = 6.0;
    spawner.next_interval = 1.0;
    game.active_layer_mut()
        .boulder_spawners
        .insert(door_key(2, 2), spawner);

    tick_boulder_spawners(&mut game, 1.0, &mut || 0.5);

    let spawner = game
        .active_layer()
        .boulder_spawners
        .get(&door_key(2, 2))
        .unwrap();
    assert!(spawner.next_interval >= 2.0 && spawner.next_interval <= 6.0);
}

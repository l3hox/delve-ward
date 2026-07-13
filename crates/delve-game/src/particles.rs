//! Particle systems and light-distance culling, ported from
//! `rendering/particles.ts` (dust motes, sconce embers, water drips,
//! fireflies) and the culling loop in `main.ts:1419-1431`. See
//! `planning/PHASE5-PLAN.md` section 7 for the research behind this
//! module.
//!
//! ## Rendering approach
//! TS renders dust motes, embers, and fireflies as `THREE.Points`
//! (`particles.ts:84,185,711`) and water drips/splashes as individual
//! `THREE.Sprite`s (`particles.ts:556,569`). This port uses one billboard
//! quad entity per particle slot instead — `Mesh3d(Rectangle)` plus
//! `crate::billboard::FacesCamera`, the same convention already used for
//! enemy/ground-item/key sprites — rather than a custom GPU point-sprite
//! or shader pipeline. Per-system entity counts here (dust 40, embers up
//! to `sconce_count * 4`, drips up to 8 concurrent plus up to 32 splash
//! rings, fireflies 12) are small enough that per-entity
//! `Transform`/`Visibility` updates are cheap; matching Three.js's exact
//! point-sprite/shader technique isn't needed to match the visual
//! character, consistent with D10's "visual character, not exact output"
//! convention.
//!
//! Each system spawns one root entity (tagged [`LevelEntity`] for the
//! existing level-cleanup convention) whose own `Visibility` is the
//! per-level enabled/disabled toggle, with pooled particle entities as
//! its children at `Visibility::Inherited` (shown) or `Visibility::Hidden`
//! (dormant slot) — mirroring `THREE.Group`/`THREE.Points`' own
//! container-visibility semantics, where hiding the parent hides every
//! child regardless of the child's own state.
//!
//! ## Confirmed dead TS code, not ported
//! `DustMotes` and `SconceEmbers` compute a per-particle `opacity` value
//! from age/lifetime/distance every frame and write it into a
//! `BufferAttribute` named `'opacity'` (`particles.ts:150-153,269-278`).
//! Both use a stock `THREE.PointsMaterial` (`particles.ts:73-82,174-183`),
//! which has no shader hook for an arbitrary named vertex attribute —
//! unlike `Fireflies`, which uses a `THREE.ShaderMaterial` that explicitly
//! declares and reads `attribute float opacity` (`particles.ts:680,701`).
//! For dust motes and embers the per-particle fade is therefore dead code:
//! every particle always renders at the flat material-level opacity
//! (`DUST_OPACITY` = 0.25, embers' hardcoded 0.8; `particles.ts:15,178`)
//! regardless of age or distance. This port sets that flat alpha once at
//! spawn and does not reproduce the unread per-particle fade math — ported
//! faithfully as "no fade," not silently improved into a working one.
//!
//! ## Ember source snapshot (one-shot, matching TS)
//! `SconceEmbers.setSources()` (`particles.ts:192-215`) is called once per
//! level load in TS, not every frame, so a sconce extinguished at runtime
//! keeps emitting embers from its last-known position until the next
//! level load (`extinguishSconce`, `sconceRenderer.ts:100-117`, never
//! re-calls `setSources`). [`collect_ember_sources`] is a one-shot
//! snapshot for the same reason — call it once after
//! `sconces::spawn_sconces`, not per frame.
//!
//! ## Known limitation: dust motes ignore the player's layer Y-offset
//! `DustMotes.createParticle` takes a `cy` parameter but never reads it
//! (`particles.ts:101,106`) — dust always spawns at a fixed
//! `WALL_HEIGHT`-relative height band, not the player's actual Y. Ported
//! faithfully: [`spawn_dust_motes`]/[`update_dust_motes`] never read the
//! player's Y (including any layer Y-offset) for the same reason. A
//! future multi-layer-aware pass would need to add the player's current
//! layer Y-offset here; out of scope for this slice.
//!
//! ## D16 seeding
//! Every RNG draw in `particles.ts` is unseeded `Math.random()`. Per D16
//! (`DECISIONS.md`), this port seeds one [`Mulberry32`] per system with a
//! fixed constant instead, matching `sconces.rs`'s `SconceFlicker` and
//! `torch.rs`'s `TorchFlicker` — visual character matches, exact frames
//! don't need to.
//!
//! ## Registration (not wired by this module)
//! Nothing here is called from `main.rs`/`level_scene.rs` yet — that
//! wiring is a later integration pass. What it needs, once it exists:
//!
//! - **At level-scene spawn** (once per level load, mirroring
//!   `props::spawn_props`/`sconces::spawn_sconces`'s call sites):
//!   - [`spawn_dust_motes`] — pass `level.dust_motes != Some(false)` (TS
//!     default: visible unless `dustMotes === false`, `main.ts:1122`).
//!   - [`spawn_fireflies`] — pass `level.fireflies == Some(true)` (TS
//!     default: off, `main.ts:1125`). Insert the returned [`FireflyPool`]
//!     as a resource.
//!   - [`spawn_water_drips`] — pass layer 0's grid/char_defs (TS predates
//!     multi-layer for this system; see the module doc above) and
//!     `level.water_drips == Some(true)` (TS default: off,
//!     `main.ts:1124`). Insert the returned [`WaterDripPool`] as a
//!     resource.
//!   - After `sconces::spawn_sconces` returns its `SconceParts`, call
//!     [`collect_ember_sources`] then [`spawn_embers`]; insert the
//!     returned [`EmberPool`] as a resource. Re-run both once per level
//!     load only — never per frame (see the module doc above).
//! - **In the `Update` schedule**, gated the same way `sconce_flicker` is
//!   (`InputGate::paused()`, `overlay.rs`) — TS runs all four `.update()`
//!   calls inside the same `if (!anyOverlayOpen)` block as sconce flicker
//!   (`main.ts:1268,1287-1290`):
//!   - [`update_dust_motes`], [`update_embers`], [`update_fireflies`],
//!     [`update_water_drips`], [`update_splash_rings`].
//! - **In the `Update` schedule, ungated** — TS's culling loop
//!   (`main.ts:1419-1431`) runs outside the `anyOverlayOpen` block, every
//!   frame regardless of overlay state:
//!   - [`cull_distant_lights`].
//! - Run order between the systems above is free: the gated ones only
//!   translate/scale/reactivate particles, `cull_distant_lights` only
//!   touches light `Visibility`, and none read each other's output.
//!
//! Until that integration pass lands, nothing in this module is reachable
//! from `main`, so every item below would otherwise trigger `dead_code` —
//! see `skybox.rs`'s `follow_skybox_camera` for the same situation on a
//! single function; here it applies file-wide.
#![allow(dead_code)]

use bevy::prelude::*;
use delve_core::grid::build_walkable_set;
use delve_core::random::Mulberry32;
use delve_core::types::CharDef;
use std::collections::HashMap;

use crate::billboard::FacesCamera;
use crate::dungeon::{CELL_SIZE, WALL_HEIGHT};
use crate::level_scene::LevelEntity;
use crate::overlay::InputGate;
use crate::player::Player;
use crate::sconces::SconceParts;
use crate::torch::{TorchFill, TorchMain};

// --- Dust motes (particles.ts:7-16) ---

const DUST_COUNT: usize = 40;
const DUST_SPAWN_RADIUS: f32 = 3.5;
const DUST_MIN_LIFETIME: f32 = 3.0;
const DUST_MAX_LIFETIME: f32 = 6.0;
/// Approximate world-space quad size. TS's `DUST_SIZE` (0.035) is a
/// `THREE.PointsMaterial` screen-perspective point size, a different unit
/// space than a world-space quad — not a literal conversion, matching D10.
const DUST_QUAD_SIZE: f32 = 0.08;
const DUST_DRIFT_SPEED: f32 = 0.15;
const DUST_OPACITY: f32 = 0.25;
const DUST_COLOR: Color = Color::srgb_u8(0xff, 0xdd, 0xaa);
const DUST_SEED: u32 = 0xA53C_1DE5;

// --- Sconce embers (particles.ts:18-27) ---

const EMBER_COUNT_PER_SCONCE: usize = 4;
const EMBER_MIN_LIFETIME: f32 = 0.6;
const EMBER_MAX_LIFETIME: f32 = 1.4;
const EMBER_QUAD_SIZE: f32 = 0.05;
const EMBER_RISE_SPEED: f32 = 0.8;
const EMBER_DRIFT: f32 = 0.3;
const EMBER_SPAWN_INTERVAL: f32 = 0.15;
const EMBER_OPACITY: f32 = 0.8;
const EMBER_COLOR: Color = Color::srgb_u8(0xff, 0x66, 0x22);
const EMBER_SEED: u32 = 0xE3B2_8F41;

// --- Water drips (particles.ts:289-300) ---
//
// TS declares `DRIP_FALL_SPEED` (`particles.ts:292`) but never reads it —
// `spawnDrip` sets `vy: 0` and the falling phase integrates purely from
// `DRIP_GRAVITY`. Dropped here rather than ported as an unread constant.

const DRIP_FORM_TIME: f32 = 1.5;
const DRIP_GRAVITY: f32 = 8.0;
const SPLASH_LIFETIME: f32 = 0.35;
const SPLASH_RING_COUNT: usize = 4;
const DRIP_MIN_INTERVAL: f32 = 10.0;
const DRIP_MAX_INTERVAL: f32 = 30.0;
const DRIP_COLOR: Color = Color::srgb_u8(0x66, 0x99, 0xcc);
const DRIP_MAX_SOURCES: usize = 8;
const DRIP_SPAWN_RADIUS: f32 = 5.0;
const DRIP_SEED: u32 = 0x7D91_2A06;

// --- Fireflies (particles.ts:605-616) ---

const FLY_COUNT: usize = 12;
const FLY_FADE_DURATION: f32 = 1.0;
const FLY_SPAWN_RADIUS: f32 = 5.0;
const FLY_MIN_LIFETIME: f32 = 8.0;
const FLY_MAX_LIFETIME: f32 = 20.0;
const FLY_QUAD_SIZE: f32 = 0.09;
const FLY_DRIFT_SPEED: f32 = 0.2;
const FLY_MAX_HEIGHT: f32 = 0.9;
const FLY_MIN_HEIGHT: f32 = 0.05;
const FLY_OPACITY: f32 = 0.8;
const FLY_COLOR: Color = Color::srgb_u8(0xcc, 0xff, 0x44);
const FLY_SEED: u32 = 0xF17E_1177;

// --- Light distance culling (main.ts:93-95,1419-1431) ---

const LIGHT_CULL_DISTANCE: f32 = 14.0;

fn random_signed(rng: &mut Mulberry32) -> f32 {
    rng.next_f64() as f32 - 0.5
}

fn quad_material(color: Color, opacity: f32, alpha_mode: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color: color.with_alpha(opacity),
        unlit: true,
        fog_enabled: true,
        alpha_mode,
        ..default()
    }
}

// ==================== Dust motes ====================

#[derive(Component)]
pub(crate) struct DustMote {
    velocity: Vec3,
    age: f32,
    lifetime: f32,
}

#[derive(Component)]
struct DustMotesRoot;

fn dust_particle(rng: &mut Mulberry32) -> (Vec3, DustMote) {
    let angle = rng.next_f64() as f32 * std::f32::consts::TAU;
    let dist = rng.next_f64() as f32 * DUST_SPAWN_RADIUS;
    let position = Vec3::new(
        angle.cos() * dist,
        WALL_HEIGHT * 0.5 + rng.next_f64() as f32 * WALL_HEIGHT * 0.5,
        angle.sin() * dist,
    );
    let lifetime =
        DUST_MIN_LIFETIME + rng.next_f64() as f32 * (DUST_MAX_LIFETIME - DUST_MIN_LIFETIME);
    let mote = DustMote {
        velocity: Vec3::new(
            random_signed(rng) * DUST_DRIFT_SPEED,
            (rng.next_f64() as f32 - 0.3) * DUST_DRIFT_SPEED * 0.5,
            random_signed(rng) * DUST_DRIFT_SPEED,
        ),
        // Staggered initial ages, matching `particles.ts:111`.
        age: rng.next_f64() as f32 * DUST_MAX_LIFETIME,
        lifetime,
    };
    (position, mote)
}

/// Spawns the dust-mote pool. `enabled` is `level.dust_motes != Some(false)`
/// (TS default true, `main.ts:1122`). Pre-fills all particles centered on
/// the world origin, matching TS's own constructor-time
/// `createParticle(0, 0, 0)` pre-fill (`particles.ts:88-90`) rather than
/// the player's actual start position — the same quirk TS ships with; the
/// pool self-corrects within one `DUST_MAX_LIFETIME` as particles expire
/// and respawn near the player.
pub fn spawn_dust_motes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    enabled: bool,
) {
    let mut rng = Mulberry32::new(DUST_SEED);
    let mesh = meshes.add(Rectangle::new(DUST_QUAD_SIZE, DUST_QUAD_SIZE));
    let material = materials.add(quad_material(DUST_COLOR, DUST_OPACITY, AlphaMode::Add));

    let root = commands
        .spawn((
            DustMotesRoot,
            LevelEntity,
            Transform::default(),
            if enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ))
        .id();

    let children: Vec<Entity> = (0..DUST_COUNT)
        .map(|_| {
            let (position, mote) = dust_particle(&mut rng);
            commands
                .spawn((
                    mote,
                    FacesCamera,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(position),
                ))
                .id()
        })
        .collect();
    commands.entity(root).add_children(&children);
}

/// Gated the same way TS's `.update()` call is — see the module doc's
/// "Registration" section.
pub fn update_dust_motes(
    time: Res<Time>,
    gate: InputGate,
    player: Query<&Transform, (With<Player>, Without<DustMote>)>,
    mut rng: Local<Option<Mulberry32>>,
    mut motes: Query<(&mut Transform, &mut DustMote), Without<Player>>,
) {
    if gate.paused() {
        return;
    }
    let Ok(player_transform) = player.single() else {
        return;
    };
    let rng = rng.get_or_insert_with(|| Mulberry32::new(DUST_SEED));
    let delta = time.delta_secs();
    let player_pos = player_transform.translation;

    for (mut transform, mut mote) in &mut motes {
        mote.age += delta;
        if mote.age >= mote.lifetime {
            let (offset, fresh) = dust_particle(rng);
            transform.translation = player_pos + Vec3::new(offset.x, 0.0, offset.z);
            transform.translation.y = offset.y;
            *mote = DustMote {
                velocity: fresh.velocity,
                age: 0.0,
                lifetime: fresh.lifetime,
            };
            continue;
        }
        transform.translation += mote.velocity * delta;
    }
}

// ==================== Sconce embers ====================

#[derive(Component)]
pub(crate) struct Ember {
    active: bool,
    velocity: Vec3,
    age: f32,
    lifetime: f32,
}

#[derive(Component)]
struct EmbersRoot;

#[derive(Resource)]
pub struct EmberPool {
    sources: Vec<Vec3>,
    spawn_timer: f32,
    rng: Mulberry32,
}

/// One-shot snapshot of every lit sconce's flame-head world position,
/// matching `SconceEmbers.setSources` (`particles.ts:192-215`) reading
/// `sconceGroup.children[3]`'s world position. `sconce_parts.torches`
/// stores `[handle, head]` per sconce (`sconces.rs:175`) — index `1` is
/// the head. Call once per level load, after `sconces::spawn_sconces`,
/// not per frame (see the module doc's "Ember source snapshot" section).
pub fn collect_ember_sources(
    sconce_parts: &SconceParts,
    transforms: &Query<&GlobalTransform>,
) -> Vec<Vec3> {
    sconce_parts
        .lights
        .keys()
        .filter_map(|key| sconce_parts.torches.get(key))
        .filter_map(|[_handle, head]| transforms.get(*head).ok())
        .map(GlobalTransform::translation)
        .collect()
}

/// Spawns the ember pool sized to `sources.len() * EMBER_COUNT_PER_SCONCE`
/// (`particles.ts:209`) — zero sources spawns zero entities, matching
/// `spawnEmber`'s no-op when `this.sources.length === 0`
/// (`particles.ts:218`).
pub fn spawn_embers(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    sources: Vec<Vec3>,
) -> EmberPool {
    let max_particles = sources.len() * EMBER_COUNT_PER_SCONCE;
    let mesh = meshes.add(Rectangle::new(EMBER_QUAD_SIZE, EMBER_QUAD_SIZE));
    let material = materials.add(quad_material(EMBER_COLOR, EMBER_OPACITY, AlphaMode::Add));

    let root = commands
        .spawn((
            EmbersRoot,
            LevelEntity,
            Transform::default(),
            Visibility::Inherited,
        ))
        .id();

    let children: Vec<Entity> = (0..max_particles)
        .map(|_| {
            commands
                .spawn((
                    Ember {
                        active: false,
                        velocity: Vec3::ZERO,
                        age: 0.0,
                        lifetime: 0.0,
                    },
                    FacesCamera,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
    commands.entity(root).add_children(&children);

    EmberPool {
        sources,
        spawn_timer: 0.0,
        rng: Mulberry32::new(EMBER_SEED),
    }
}

/// Gated the same way TS's `.update()` call is — see the module doc's
/// "Registration" section.
pub fn update_embers(
    time: Res<Time>,
    gate: InputGate,
    mut pool: ResMut<EmberPool>,
    mut embers: Query<(&mut Transform, &mut Ember, &mut Visibility)>,
) {
    if gate.paused() {
        return;
    }
    let delta = time.delta_secs();

    pool.spawn_timer -= delta;
    if pool.spawn_timer <= 0.0 && !pool.sources.is_empty() {
        pool.spawn_timer = EMBER_SPAWN_INTERVAL;
        let source_index = (pool.rng.next_f64() as f32 * pool.sources.len() as f32) as usize;
        let source_index = source_index.min(pool.sources.len().saturating_sub(1));
        let source = pool.sources[source_index];
        if let Some((mut transform, mut ember, mut visibility)) =
            embers.iter_mut().find(|(_, ember, _)| !ember.active)
        {
            transform.translation = source
                + Vec3::new(
                    random_signed(&mut pool.rng) * 0.06,
                    0.0,
                    random_signed(&mut pool.rng) * 0.06,
                );
            ember.active = true;
            ember.age = 0.0;
            ember.lifetime = EMBER_MIN_LIFETIME
                + pool.rng.next_f64() as f32 * (EMBER_MAX_LIFETIME - EMBER_MIN_LIFETIME);
            ember.velocity = Vec3::new(
                random_signed(&mut pool.rng) * EMBER_DRIFT,
                EMBER_RISE_SPEED + pool.rng.next_f64() as f32 * 0.3,
                random_signed(&mut pool.rng) * EMBER_DRIFT,
            );
            *visibility = Visibility::Inherited;
        }
    }

    for (mut transform, mut ember, mut visibility) in &mut embers {
        if !ember.active {
            continue;
        }
        ember.age += delta;
        if ember.age >= ember.lifetime {
            ember.active = false;
            *visibility = Visibility::Hidden;
            continue;
        }
        transform.translation += ember.velocity * delta;
        // Slow down horizontal drift, matching `particles.ts:257-258`.
        ember.velocity.x *= 0.98;
        ember.velocity.z *= 0.98;
    }
}

// ==================== Water drips ====================

#[derive(Clone, Copy, PartialEq, Eq)]
enum DripPhase {
    Forming,
    Falling,
    Splash,
}

#[derive(Component)]
pub(crate) struct WaterDrip {
    active: bool,
    phase: DripPhase,
    age: f32,
    form_duration: f32,
    velocity_y: f32,
}

#[derive(Component)]
pub(crate) struct SplashRing {
    ring_delay: f32,
    age: f32,
}

#[derive(Component)]
struct WaterDripsRoot;

#[derive(Resource)]
pub struct WaterDripPool {
    walkable_cells: Vec<(i32, i32, f32, f32)>,
    source_timers: HashMap<(i32, i32), f32>,
    rng: Mulberry32,
    splash_mesh: Handle<Mesh>,
    splash_material: Handle<StandardMaterial>,
}

/// Builds the drop-sprite pool (capped at `DRIP_MAX_SOURCES`, matching
/// TS's own cap) and the walkable-cell scan `WaterDrips.setLevel` performs
/// (`particles.ts:403-421`). TS predates multi-layer levels for this
/// system (`activeGrid()`-only); pass layer 0's grid/char_defs until a
/// multi-layer-aware pass extends this (see the module doc above).
/// `enabled` is `level.water_drips == Some(true)` (TS default off,
/// `main.ts:1124`).
pub fn spawn_water_drips(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    grid: &[String],
    char_defs: &[CharDef],
    enabled: bool,
) -> WaterDripPool {
    let walkable = build_walkable_set(char_defs.iter().map(|def| (def.character, def.solid)));
    let walkable_cells: Vec<(i32, i32, f32, f32)> = grid
        .iter()
        .enumerate()
        .flat_map(|(row, line)| {
            let walkable = &walkable;
            line.chars().enumerate().filter_map(move |(col, ch)| {
                let eligible = walkable.contains(&ch) && ch != 'S' && ch != 'U' && ch != 'D';
                eligible.then(|| {
                    (
                        col as i32,
                        row as i32,
                        col as f32 * CELL_SIZE + CELL_SIZE / 2.0,
                        row as f32 * CELL_SIZE + CELL_SIZE / 2.0,
                    )
                })
            })
        })
        .collect();

    let drop_mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let drop_material = materials.add(quad_material(DRIP_COLOR, 0.8, AlphaMode::Blend));
    let splash_mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let splash_material = materials.add(quad_material(DRIP_COLOR, 0.6, AlphaMode::Blend));

    let root = commands
        .spawn((
            WaterDripsRoot,
            LevelEntity,
            Transform::default(),
            if enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ))
        .id();

    let children: Vec<Entity> = (0..DRIP_MAX_SOURCES)
        .map(|_| {
            commands
                .spawn((
                    WaterDrip {
                        active: false,
                        phase: DripPhase::Forming,
                        age: 0.0,
                        form_duration: DRIP_FORM_TIME,
                        velocity_y: 0.0,
                    },
                    Mesh3d(drop_mesh.clone()),
                    MeshMaterial3d(drop_material.clone()),
                    Transform::from_scale(Vec3::new(0.02, 0.04, 1.0)),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
    commands.entity(root).add_children(&children);

    WaterDripPool {
        walkable_cells,
        source_timers: HashMap::new(),
        rng: Mulberry32::new(DRIP_SEED),
        splash_mesh,
        splash_material,
    }
}

fn drip_interval(rng: &mut Mulberry32) -> f32 {
    DRIP_MIN_INTERVAL + rng.next_f64() as f32 * (DRIP_MAX_INTERVAL - DRIP_MIN_INTERVAL)
}

/// Falling-phase stretch factor from downward speed, matching
/// `particles.ts:473` (`Math.min(3, 1 + speed * 0.15)`).
fn drip_fall_stretch(speed: f32) -> f32 {
    (1.0 + speed * 0.15).min(3.0)
}

/// Forming-phase opacity ramp, matching `particles.ts:454`.
fn drip_form_opacity(t: f32) -> f32 {
    0.3 + t * 0.5
}

/// Splash-ring progress for `ring_index` at `age`, matching
/// `particles.ts:499-500` (`ringDelay = r * 0.06`,
/// `rt = max(0, (age - ringDelay) / (SPLASH_LIFETIME - ringDelay))`).
fn splash_ring_progress(age: f32, ring_delay: f32) -> f32 {
    let span = SPLASH_LIFETIME - ring_delay;
    if span <= 0.0 {
        return 0.0;
    }
    ((age - ring_delay) / span).max(0.0)
}

/// Gated the same way TS's `.update()` call is — see the module doc's
/// "Registration" section. Spawns [`SplashRing`] entities via `commands`
/// when a drip enters its splash phase; they age and despawn
/// independently in [`update_splash_rings`].
#[allow(clippy::too_many_arguments)]
pub fn update_water_drips(
    time: Res<Time>,
    gate: InputGate,
    mut commands: Commands,
    player: Query<&Transform, (With<Player>, Without<WaterDrip>)>,
    mut pool: ResMut<WaterDripPool>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drips: Query<(
        &mut Transform,
        &mut WaterDrip,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    if gate.paused() {
        return;
    }
    let Ok(player_transform) = player.single() else {
        return;
    };
    let delta = time.delta_secs();
    let player_pos = player_transform.translation;

    try_spawn_drips(&mut pool, &mut drips, player_pos, delta);

    for (mut transform, mut drip, mut visibility, material) in &mut drips {
        if !drip.active {
            continue;
        }
        drip.age += delta;
        match drip.phase {
            DripPhase::Forming => {
                let t = (drip.age / drip.form_duration).min(1.0);
                let scale = 0.02 + t * 0.06;
                transform.scale = Vec3::new(scale, scale * 2.0, 1.0);
                transform.translation.y = WALL_HEIGHT - 0.02 - t * 0.04;
                if let Some(mut handle) = materials.get_mut(&material.0) {
                    handle.base_color = DRIP_COLOR.with_alpha(drip_form_opacity(t));
                }
                if drip.age >= drip.form_duration {
                    drip.phase = DripPhase::Falling;
                    drip.age = 0.0;
                    drip.velocity_y = 0.0;
                }
            }
            DripPhase::Falling => {
                drip.velocity_y += DRIP_GRAVITY * delta;
                transform.translation.y -= drip.velocity_y * delta;
                let stretch = drip_fall_stretch(drip.velocity_y);
                transform.scale = Vec3::new(0.06, 0.06 * stretch, 1.0);
                if let Some(mut handle) = materials.get_mut(&material.0) {
                    handle.base_color = DRIP_COLOR.with_alpha(0.8);
                }
                if transform.translation.y <= 0.01 {
                    drip.phase = DripPhase::Splash;
                    drip.age = 0.0;
                    *visibility = Visibility::Hidden;
                    spawn_splash_rings(
                        &mut commands,
                        pool.splash_mesh.clone(),
                        pool.splash_material.clone(),
                        transform.translation,
                    );
                }
            }
            DripPhase::Splash => {
                if drip.age / SPLASH_LIFETIME >= 1.0 {
                    drip.active = false;
                }
            }
        }
    }
}

fn try_spawn_drips(
    pool: &mut WaterDripPool,
    drips: &mut Query<(
        &mut Transform,
        &mut WaterDrip,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    player_pos: Vec3,
    delta: f32,
) {
    let radius_sq = DRIP_SPAWN_RADIUS * DRIP_SPAWN_RADIUS;
    let nearby: Vec<(i32, i32, f32, f32)> = pool
        .walkable_cells
        .iter()
        .copied()
        .filter(|&(_, _, x, z)| {
            let dx = x - player_pos.x;
            let dz = z - player_pos.z;
            dx * dx + dz * dz <= radius_sq
        })
        .collect();
    if nearby.is_empty() {
        return;
    }

    let active_count = drips.iter().filter(|(_, drip, _, _)| drip.active).count();
    let mut free_slot = active_count < DRIP_MAX_SOURCES;

    for (col, row, x, z) in nearby {
        let key = (col, row);
        let timer = pool
            .source_timers
            .entry(key)
            .or_insert_with(|| drip_interval(&mut pool.rng));
        *timer -= delta;
        if *timer <= 0.0 && free_slot {
            if let Some((mut transform, mut drip, mut visibility, _)) =
                drips.iter_mut().find(|(_, drip, _, _)| !drip.active)
            {
                let ox = random_signed(&mut pool.rng) * CELL_SIZE * 0.6;
                let oz = random_signed(&mut pool.rng) * CELL_SIZE * 0.6;
                transform.translation = Vec3::new(x + ox, WALL_HEIGHT - 0.02, z + oz);
                transform.scale = Vec3::new(0.02, 0.04, 1.0);
                *drip = WaterDrip {
                    active: true,
                    phase: DripPhase::Forming,
                    age: 0.0,
                    form_duration: DRIP_FORM_TIME * (0.7 + pool.rng.next_f64() as f32 * 0.6),
                    velocity_y: 0.0,
                };
                *visibility = Visibility::Inherited;
                free_slot = false;
            }
            *timer = drip_interval(&mut pool.rng);
        }
    }
}

fn spawn_splash_rings(
    commands: &mut Commands,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    at: Vec3,
) {
    for ring in 0..SPLASH_RING_COUNT {
        commands.spawn((
            SplashRing {
                ring_delay: ring as f32 * 0.06,
                age: 0.0,
            },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(Vec3::new(at.x, 0.02, at.z))
                .with_scale(Vec3::new(0.03, 0.009, 1.0)),
            Visibility::Inherited,
        ));
    }
}

/// Ages and despawns splash rings independently of their parent drip —
/// see the module doc's water-drip design note. Gated the same way TS's
/// `.update()` call is.
pub fn update_splash_rings(
    time: Res<Time>,
    gate: InputGate,
    mut commands: Commands,
    mut rings: Query<(Entity, &mut Transform, &mut SplashRing)>,
) {
    if gate.paused() {
        return;
    }
    let delta = time.delta_secs();
    for (entity, mut transform, mut ring) in &mut rings {
        ring.age += delta;
        let rt = splash_ring_progress(ring.age, ring.ring_delay);
        if rt >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let scale = 0.03 + rt * 0.15;
        transform.scale = Vec3::new(scale, scale * 0.3, 1.0);
    }
}

// ==================== Fireflies ====================

#[derive(Component)]
pub(crate) struct Firefly {
    alive: bool,
    respawn_timer: f32,
    velocity: Vec3,
    age: f32,
    lifetime: f32,
    blinks: bool,
    blink_phase: f32,
    blink_speed: f32,
}

#[derive(Component)]
struct FirefliesRoot;

#[derive(Resource)]
pub struct FireflyPool {
    enabled: bool,
    rng: Mulberry32,
}

fn firefly_particle(rng: &mut Mulberry32) -> Firefly {
    Firefly {
        alive: true,
        respawn_timer: 0.0,
        velocity: Vec3::new(
            random_signed(rng) * FLY_DRIFT_SPEED,
            random_signed(rng) * FLY_DRIFT_SPEED * 0.3,
            random_signed(rng) * FLY_DRIFT_SPEED,
        ),
        age: 0.0,
        lifetime: FLY_MIN_LIFETIME + rng.next_f64() as f32 * (FLY_MAX_LIFETIME - FLY_MIN_LIFETIME),
        blinks: rng.next_f64() < 0.33,
        blink_phase: rng.next_f64() as f32 * std::f32::consts::TAU,
        blink_speed: 1.5 + rng.next_f64() as f32 * 2.5,
    }
}

fn firefly_position(rng: &mut Mulberry32, center: Vec2) -> Vec3 {
    let angle = rng.next_f64() as f32 * std::f32::consts::TAU;
    let dist = rng.next_f64() as f32 * FLY_SPAWN_RADIUS;
    Vec3::new(
        center.x + angle.cos() * dist,
        FLY_MIN_HEIGHT + rng.next_f64() as f32 * (FLY_MAX_HEIGHT - FLY_MIN_HEIGHT),
        center.y + angle.sin() * dist,
    )
}

/// Blink multiplier for a firefly at `blink_phase`, matching
/// `particles.ts:800` (`0.5 + 0.5 * Math.sin(phase)` for blinking flies,
/// `1.0` for non-blinking ones — only 1 in 3 fireflies blink).
fn firefly_blink(blinks: bool, blink_phase: f32) -> f32 {
    if blinks {
        0.5 + 0.5 * blink_phase.sin()
    } else {
        1.0
    }
}

/// Fade-in/fade-out envelope over `FLY_FADE_DURATION` at both ends of
/// life, matching `particles.ts:803-812`.
fn firefly_life_fade(age: f32, lifetime: f32) -> f32 {
    let fade_out_start = lifetime - FLY_FADE_DURATION;
    if age < FLY_FADE_DURATION {
        age / FLY_FADE_DURATION
    } else if age > fade_out_start {
        (lifetime - age) / FLY_FADE_DURATION
    } else {
        1.0
    }
}

/// Distance fade from the player, matching `particles.ts:817-818`
/// (`max(0, 1 - distSq / radius^2)`, XZ-plane only).
fn distance_fade(dx: f32, dz: f32, radius: f32) -> f32 {
    (1.0 - (dx * dx + dz * dz) / (radius * radius)).max(0.0)
}

/// Spawns the firefly pool, all initially dormant with a zeroed respawn
/// timer — matching TS's constructor, which pushes `null` slots with
/// `respawnTimers[i] = 0` (`particles.ts:714-717`), so every slot spawns
/// on the first eligible frame once enabled. `enabled` is
/// `level.fireflies == Some(true)` (TS default off, `main.ts:1125`).
pub fn spawn_fireflies(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    enabled: bool,
) -> FireflyPool {
    let mesh = meshes.add(Rectangle::new(FLY_QUAD_SIZE, FLY_QUAD_SIZE));
    let material = materials.add(quad_material(FLY_COLOR, FLY_OPACITY, AlphaMode::Add));

    let root = commands
        .spawn((
            FirefliesRoot,
            LevelEntity,
            Transform::default(),
            Visibility::Inherited,
        ))
        .id();

    let children: Vec<Entity> = (0..FLY_COUNT)
        .map(|_| {
            commands
                .spawn((
                    Firefly {
                        alive: false,
                        respawn_timer: 0.0,
                        velocity: Vec3::ZERO,
                        age: 0.0,
                        lifetime: 0.0,
                        blinks: false,
                        blink_phase: 0.0,
                        blink_speed: 0.0,
                    },
                    FacesCamera,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
    commands.entity(root).add_children(&children);

    FireflyPool {
        enabled,
        rng: Mulberry32::new(FLY_SEED),
    }
}

/// Gated the same way TS's `.update()` call is — see the module doc's
/// "Registration" section.
pub fn update_fireflies(
    time: Res<Time>,
    gate: InputGate,
    player: Query<&Transform, (With<Player>, Without<Firefly>)>,
    mut pool: ResMut<FireflyPool>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut flies: Query<(
        &mut Transform,
        &mut Firefly,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    if gate.paused() {
        return;
    }
    let Ok(player_transform) = player.single() else {
        return;
    };
    let delta = time.delta_secs();
    let center = Vec2::new(
        player_transform.translation.x,
        player_transform.translation.z,
    );

    for (mut transform, mut firefly, mut visibility, material) in &mut flies {
        if !firefly.alive {
            if pool.enabled {
                firefly.respawn_timer -= delta;
                if firefly.respawn_timer <= 0.0 {
                    let fresh = firefly_particle(&mut pool.rng);
                    transform.translation = firefly_position(&mut pool.rng, center);
                    *firefly = fresh;
                    *visibility = Visibility::Inherited;
                }
            }
            continue;
        }

        firefly.age += delta;
        firefly.blink_phase += firefly.blink_speed * delta;
        if firefly.age >= firefly.lifetime {
            firefly.alive = false;
            firefly.respawn_timer = 0.5 + pool.rng.next_f64() as f32 * 1.5;
            *visibility = Visibility::Hidden;
            continue;
        }

        transform.translation += firefly.velocity * delta;
        if transform.translation.y < FLY_MIN_HEIGHT {
            transform.translation.y = FLY_MIN_HEIGHT;
            firefly.velocity.y = firefly.velocity.y.abs();
        }
        if transform.translation.y > FLY_MAX_HEIGHT {
            transform.translation.y = FLY_MAX_HEIGHT;
            firefly.velocity.y = -firefly.velocity.y.abs();
        }

        firefly.velocity.x += random_signed(&mut pool.rng) * 0.3 * delta;
        firefly.velocity.z += random_signed(&mut pool.rng) * 0.3 * delta;
        let speed = (firefly.velocity.x * firefly.velocity.x
            + firefly.velocity.z * firefly.velocity.z)
            .sqrt();
        if speed > FLY_DRIFT_SPEED {
            firefly.velocity.x *= FLY_DRIFT_SPEED / speed;
            firefly.velocity.z *= FLY_DRIFT_SPEED / speed;
        }

        let blink = firefly_blink(firefly.blinks, firefly.blink_phase);
        let life_fade = firefly_life_fade(firefly.age, firefly.lifetime);
        let dx = transform.translation.x - center.x;
        let dz = transform.translation.z - center.y;
        let fade = distance_fade(dx, dz, FLY_SPAWN_RADIUS);
        let opacity = blink * life_fade * fade * FLY_OPACITY;
        if let Some(mut handle) = materials.get_mut(&material.0) {
            handle.base_color = FLY_COLOR.with_alpha(opacity);
        }
    }
}

// ==================== Light distance culling ====================

/// Whether a light at squared distance `distance_sq` from the camera
/// should be hidden, matching `main.ts:1422,1429`
/// (`(dx*dx+dy*dy+dz*dz) < cullDistSq` drives `.visible`, so the light is
/// hidden when the distance is at or beyond the threshold).
fn light_culled(distance_sq: f32) -> bool {
    distance_sq >= LIGHT_CULL_DISTANCE * LIGHT_CULL_DISTANCE
}

/// Ungated — TS's culling loop (`main.ts:1419-1431`) runs every frame
/// regardless of overlay state, unlike the four particle `.update()`
/// calls above. Skips lights already at `intensity == 0.0`, matching
/// `sconce_flicker`'s existing convention (`sconces.rs:245`) so this
/// system never fights `extinguish_sconce` over which is the source of
/// truth for an extinguished light's state (see `planning/PHASE5-PLAN.md`
/// section 7's risk note).
#[allow(clippy::type_complexity)]
pub fn cull_distant_lights(
    player: Query<&Transform, (With<Player>, Without<PointLight>)>,
    mut lights: Query<
        (&GlobalTransform, &PointLight, &mut Visibility),
        (Without<TorchMain>, Without<TorchFill>),
    >,
) {
    let Ok(player_transform) = player.single() else {
        return;
    };
    let camera_pos = player_transform.translation;

    for (light_transform, light, mut visibility) in &mut lights {
        if light.intensity == 0.0 {
            continue;
        }
        let distance_sq = camera_pos.distance_squared(light_transform.translation());
        *visibility = if light_culled(distance_sq) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FLY_FADE_DURATION, LIGHT_CULL_DISTANCE, SPLASH_LIFETIME, distance_fade, drip_fall_stretch,
        drip_form_opacity, firefly_blink, firefly_life_fade, light_culled, splash_ring_progress,
    };
    use std::f32::consts::PI;

    #[test]
    fn light_culled_matches_ts_threshold() {
        let just_inside = LIGHT_CULL_DISTANCE - 0.01;
        let just_outside = LIGHT_CULL_DISTANCE + 0.01;
        assert!(!light_culled(just_inside * just_inside));
        assert!(light_culled(just_outside * just_outside));
        assert!(light_culled(LIGHT_CULL_DISTANCE * LIGHT_CULL_DISTANCE));
    }

    #[test]
    fn drip_fall_stretch_caps_at_three() {
        assert!((drip_fall_stretch(0.0) - 1.0).abs() < 1e-6);
        assert!((drip_fall_stretch(1.0 / 0.15) - 2.0).abs() < 1e-5);
        assert!((drip_fall_stretch(100.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn drip_form_opacity_ramps_from_point_three_to_point_eight() {
        assert!((drip_form_opacity(0.0) - 0.3).abs() < 1e-6);
        assert!((drip_form_opacity(1.0) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn splash_ring_progress_delays_by_ring_index() {
        let ring0 = 0.0_f32;
        let ring3 = 3.0 * 0.06;
        assert!((splash_ring_progress(0.0, ring0) - 0.0).abs() < 1e-6);
        assert_eq!(splash_ring_progress(0.0, ring3), 0.0);
        let progressed = splash_ring_progress(SPLASH_LIFETIME, ring0);
        assert!((progressed - 1.0).abs() < 1e-6);
    }

    #[test]
    fn firefly_blink_is_flat_for_non_blinking_flies() {
        assert!((firefly_blink(false, 0.0) - 1.0).abs() < 1e-6);
        assert!((firefly_blink(false, 42.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn firefly_blink_oscillates_between_zero_and_one() {
        assert!((firefly_blink(true, 0.0) - 0.5).abs() < 1e-6);
        assert!((firefly_blink(true, PI / 2.0) - 1.0).abs() < 1e-5);
        assert!((firefly_blink(true, 3.0 * PI / 2.0) - 0.0).abs() < 1e-5);
    }

    #[test]
    fn firefly_life_fade_ramps_in_and_out() {
        let lifetime = 10.0;
        assert!((firefly_life_fade(0.0, lifetime) - 0.0).abs() < 1e-6);
        assert!((firefly_life_fade(FLY_FADE_DURATION, lifetime) - 1.0).abs() < 1e-6);
        assert!((firefly_life_fade(lifetime / 2.0, lifetime) - 1.0).abs() < 1e-6);
        assert!((firefly_life_fade(lifetime, lifetime) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn distance_fade_is_one_at_center_and_zero_beyond_radius() {
        assert!((distance_fade(0.0, 0.0, 5.0) - 1.0).abs() < 1e-6);
        assert!((distance_fade(5.0, 0.0, 5.0) - 0.0).abs() < 1e-6);
        assert_eq!(distance_fade(10.0, 0.0, 5.0), 0.0);
    }
}

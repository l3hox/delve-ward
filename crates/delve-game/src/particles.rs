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
//! ## Ember source snapshot (re-collected on pickup, matching TS)
//! `SconceEmbers.setSources()` (`particles.ts:192-215`) is called once at
//! level load (`main.ts:1121`) *and* again whenever a torch is taken —
//! `inputSystem.ts:170-173` calls `extinguishSconce(...)` then
//! `ctx.sconceEmbers.setSources(...)` as two sequential steps in the
//! `sconce_taken` result handler, so the just-extinguished sconce (now
//! `light.intensity === 0`) is excluded from the very next collection.
//! `setSources` itself resets `this.particles = []`
//! (`particles.ts:214`), discarding every in-flight ember, not just
//! future spawns — the refresh is a full pool rebuild, not a filter.
//! `extinguishSconce` alone never refreshes; a sconce extinguished by any
//! future path that doesn't *also* call `setSources` right after (the
//! same latent risk TS itself has, since the refresh is the caller's
//! responsibility, not `extinguishSconce`'s) would keep emitting from a
//! stale position — currently moot since `extinguish_sconce` has exactly
//! one call site ([`crate::session::interact_input`]'s `SconceTaken` arm),
//! which re-triggers collection right after by re-inserting
//! [`EmbersPending`]. [`collect_ember_sources`] itself stays a pure
//! snapshot function — [`init_embers`] is what makes it "once at load,
//! once per pickup" rather than truly one-shot.
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
//! ## Registration
//! - **At level-scene spawn** (`level_scene.rs`, once per level load):
//!   - [`spawn_dust_motes`] — `level.dust_motes != Some(false)` (TS
//!     default: visible unless `dustMotes === false`, `main.ts:1122`).
//!   - [`spawn_fireflies`] — `level.fireflies == Some(true)` (TS
//!     default: off, `main.ts:1125`); the returned [`FireflyPool`] is a
//!     resource.
//!   - [`spawn_water_drips`] — layer 0's grid/char_defs (TS predates
//!     multi-layer for this system; see the module doc above) and
//!     `level.water_drips == Some(true)` (TS default: off,
//!     `main.ts:1124`); the returned [`WaterDripPool`] is a resource.
//!   - Embers can't spawn there: [`collect_ember_sources`] reads the
//!     sconce heads' `GlobalTransform`s, and the scene spawn's own
//!     commands haven't applied yet, let alone propagated. The scene
//!     spawn inserts the [`EmbersPending`] marker instead, and
//!     [`init_embers`] (registered in `PostUpdate` after
//!     `TransformSystems::Propagate`) collects sources and builds the
//!     [`EmberPool`] on the first frame the propagated positions exist.
//!   - [`crate::session::interact_input`]'s `SconceTaken` arm re-inserts
//!     [`EmbersPending`] right after `extinguish_sconce`, so
//!     [`init_embers`] rebuilds the pool again on torch pickup, matching
//!     TS's load-once-plus-pickup-refresh `setSources` pattern (see the
//!     module doc's "Ember source snapshot" section above).
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

/// Converts a `THREE.PointsMaterial` size into the world-space quad size
/// that covers the same pixels, since this port draws billboard quads where
/// TS draws point sprites (see the rendering-approach note above).
///
/// THREE sizes a perspective point sprite as `gl_PointSize = size * (scale /
/// -mvPosition.z)` with `scale = viewport_height / 2`
/// (`points.glsl.js:34-40`, `WebGLMaterials.js:297`), so a point covers
/// `size * height / (2 * distance)` pixels. A world-space quad of height `S`
/// covers `S * height / (2 * distance * tan(fov / 2))`. Equal pixel coverage
/// therefore means `S = size * tan(fov / 2)` — distance cancels, so one
/// constant holds at every depth.
///
/// [`point_quad_size`] is what the per-system constants below are derived
/// with; `POINT_SIZE_TO_QUAD` is `tan(CAMERA_FOV_DEGREES / 2)` spelled out
/// because `f32::tan` isn't const. `point_size_conversion_matches_the_camera
/// _fov` pins it to the camera so the two can't drift apart.
const POINT_SIZE_TO_QUAD: f32 = 0.767_327;

pub(crate) const fn point_quad_size(point_size: f32) -> f32 {
    point_size * POINT_SIZE_TO_QUAD
}

// --- Dust motes (particles.ts:7-16) ---

const DUST_COUNT: usize = 40;
const DUST_SPAWN_RADIUS: f32 = 3.5;
const DUST_MIN_LIFETIME: f32 = 3.0;
const DUST_MAX_LIFETIME: f32 = 6.0;
/// TS's `DUST_SIZE` (0.035) as a quad — see [`point_quad_size`].
const DUST_QUAD_SIZE: f32 = point_quad_size(0.035);
const DUST_DRIFT_SPEED: f32 = 0.15;
const DUST_OPACITY: f32 = 0.25;
const DUST_COLOR: Color = Color::srgb_u8(0xff, 0xdd, 0xaa);
const DUST_SEED: u32 = 0xA53C_1DE5;

// --- Sconce embers (particles.ts:18-27) ---

const EMBER_COUNT_PER_SCONCE: usize = 4;
const EMBER_MIN_LIFETIME: f32 = 0.6;
const EMBER_MAX_LIFETIME: f32 = 1.4;
/// TS's `EMBER_SIZE` (0.05) as a quad — see [`point_quad_size`].
const EMBER_QUAD_SIZE: f32 = point_quad_size(0.05);
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
/// TS's `FLY_SIZE` (0.06) as a quad — see [`point_quad_size`].
const FLY_QUAD_SIZE: f32 = point_quad_size(0.06);
const FLY_DRIFT_SPEED: f32 = 0.2;
const FLY_MAX_HEIGHT: f32 = 0.9;
const FLY_MIN_HEIGHT: f32 = 0.05;
const FLY_OPACITY: f32 = 0.8;
const FLY_COLOR: Color = Color::srgb_u8(0xcc, 0xff, 0x44);
const FLY_SEED: u32 = 0xF17E_1177;

// --- Light distance culling (main.ts:93-95,1419-1431) ---

/// How far from the camera a light may sit before it is switched off.
///
/// TS culls at 14 (`main.ts:93-95,1419-1431`), which this port matched until
/// it was measured. Two problems with that number. It is tighter than the view
/// itself — dungeon fog reaches 26 — so lights died at barely half the distance
/// the player can see, and a large room lost its far lights while still showing
/// its far wall. And it caps how grand a scene can get: a cavern lit by
/// hundreds of sconces would have kept about twenty of them.
///
/// The cull now sits where things genuinely stop being visible: the farthest
/// fog distance, plus the reach of a light itself, since a light just past the
/// horizon still illuminates geometry this side of it. Measured on
/// `stress_lights` (300 lit sconces over six galleries, release, vsync off):
/// culled 4.57ms, unculled 4.63ms, unculled with every light's range raised to
/// 40 so they all overlap 4.62ms — one frame's noise apart. The cull is not
/// what pays for the frame, so buying back the view costs nothing.
///
/// A deliberate break from TS parity, recorded as D20.
const LIGHT_CULL_DISTANCE: f32 = crate::environment::MAX_FOG_FAR + MAX_LIGHT_RANGE;

/// The longest range any light in this game is given — the player's torch
/// (`torch.rs`'s `TORCH_RANGE`). Lights this far beyond the fog horizon can
/// still light something inside it.
const MAX_LIGHT_RANGE: f32 = 12.0;

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
pub(crate) struct EmbersRoot;

#[derive(Resource)]
pub struct EmberPool {
    sources: Vec<Vec3>,
    spawn_timer: f32,
    rng: Mulberry32,
}

/// Snapshot of every lit sconce's flame-head world position, matching
/// `SconceEmbers.setSources` (`particles.ts:192-215`) reading
/// `sconceGroup.children[3]`'s world position from sconces whose
/// `light.intensity !== 0` — an extinguished or torch-taken sconce emits
/// nothing. `sconce_parts.torches` stores `[handle, head]` per sconce
/// (`sconces.rs:175`) — index `1` is the head. A pure function, not
/// itself gated to run once — [`init_embers`] is what calls it only at
/// level load and on torch pickup, not per frame (see the module doc's
/// "Ember source snapshot" section).
pub fn collect_ember_sources(
    sconce_parts: &SconceParts,
    lights: &Query<&PointLight>,
    transforms: &Query<&GlobalTransform>,
) -> Vec<Vec3> {
    sconce_parts
        .lights
        .iter()
        .filter(|(_, light)| {
            lights
                .get(**light)
                .is_ok_and(|light| light.intensity != 0.0)
        })
        .filter_map(|(key, _)| sconce_parts.torches.get(key))
        .filter_map(|[_handle, head]| transforms.get(*head).ok())
        .map(GlobalTransform::translation)
        .collect()
}

/// Inserted in place of an [`EmberPool`] whenever the ember source list
/// needs recollecting: by the level-scene spawn (initial load — ember
/// sources need the sconce heads' propagated `GlobalTransform`s, which
/// don't exist until the frame the scene spawn's commands apply and
/// propagate) and by [`crate::session::interact_input`]'s `SconceTaken`
/// arm (torch pickup — see the module doc's "Ember source snapshot"
/// section). [`init_embers`] consumes this and rebuilds the pool either
/// way.
#[derive(Resource)]
pub struct EmbersPending;

/// Ember (re)initialization, deferred via [`EmbersPending`] from whichever
/// caller needs a fresh source collection. Registered in `PostUpdate`
/// after `TransformSystems::Propagate`, so it always sees up-to-date
/// world positions — on initial load this is what makes the first run
/// after a scene spawn see real positions instead of pre-propagation
/// defaults; on a pickup-triggered refresh the positions haven't moved,
/// only the light-intensity filter in [`collect_ember_sources`] has.
///
/// Despawns the previous [`EmbersRoot`] (and its pooled children, via
/// Bevy's recursive despawn) before spawning the replacement — TS's
/// `setSources` clears `this.particles = []` on every call
/// (`particles.ts:214`), discarding in-flight embers too, so a full
/// rebuild is the faithful behavior, not just a leak-avoidance measure.
#[allow(clippy::too_many_arguments)]
pub fn init_embers(
    mut commands: Commands,
    pending: Option<Res<EmbersPending>>,
    sconce_parts: Option<Res<SconceParts>>,
    existing_roots: Query<Entity, With<EmbersRoot>>,
    lights: Query<&PointLight>,
    transforms: Query<&GlobalTransform>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if pending.is_none() {
        return;
    }
    let Some(sconce_parts) = sconce_parts else {
        return;
    };
    // A head entity the transform query can't see yet means this frame's
    // commands haven't applied — retry next frame rather than snapshot a
    // partial set.
    let heads_ready = sconce_parts
        .torches
        .values()
        .all(|[_handle, head]| transforms.contains(*head));
    if !heads_ready {
        return;
    }
    for root in &existing_roots {
        commands.entity(root).despawn();
    }
    let sources = collect_ember_sources(&sconce_parts, &lights, &transforms);
    let pool = spawn_embers(&mut commands, &mut meshes, &mut materials, sources);
    commands.insert_resource(pool);
    commands.remove_resource::<EmbersPending>();
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
/// "Registration" section. The pool is optional because [`init_embers`]
/// builds it a frame after the scene spawn (see [`EmbersPending`]) — until
/// then there is nothing to update.
pub fn update_embers(
    time: Res<Time>,
    gate: InputGate,
    pool: Option<ResMut<EmberPool>>,
    mut embers: Query<(&mut Transform, &mut Ember, &mut Visibility)>,
) {
    let Some(mut pool) = pool else {
        return;
    };
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
        FLY_FADE_DURATION, LIGHT_CULL_DISTANCE, POINT_SIZE_TO_QUAD, SPLASH_LIFETIME, distance_fade,
        drip_fall_stretch, drip_form_opacity, firefly_blink, firefly_life_fade, light_culled,
        point_quad_size, splash_ring_progress,
    };
    use std::f32::consts::PI;

    /// `POINT_SIZE_TO_QUAD` is a spelled-out `tan(fov / 2)` because `f32::tan`
    /// isn't const; if the camera's field of view ever changes, this catches
    /// the stale constant instead of letting every particle silently take on
    /// the wrong size.
    #[test]
    fn point_size_conversion_matches_the_camera_fov() {
        let expected = (crate::zones::CAMERA_FOV_DEGREES.to_radians() / 2.0).tan();
        assert!(
            (POINT_SIZE_TO_QUAD - expected).abs() < 1e-6,
            "{POINT_SIZE_TO_QUAD} vs {expected}"
        );
    }

    /// A point sprite and its replacement quad have to cover the same pixels:
    /// at any distance `d`, THREE's point spans `size * h / (2 * d)` pixels
    /// and a quad of height `S` spans `S * h / (2 * d * tan(fov / 2))`.
    #[test]
    fn a_converted_quad_covers_the_same_pixels_as_the_ts_point_sprite() {
        let viewport_height = 1080.0_f32;
        let half_fov_tangent = (crate::zones::CAMERA_FOV_DEGREES.to_radians() / 2.0).tan();
        for point_size in [0.035_f32, 0.05, 0.06, 0.12] {
            let quad = point_quad_size(point_size);
            for distance in [1.0_f32, 4.0, 12.5] {
                let point_pixels = point_size * viewport_height / (2.0 * distance);
                let quad_pixels = quad * viewport_height / (2.0 * distance * half_fov_tangent);
                assert!(
                    (point_pixels - quad_pixels).abs() < 1e-3,
                    "{point_size} at {distance}: {point_pixels} vs {quad_pixels}"
                );
            }
        }
    }

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

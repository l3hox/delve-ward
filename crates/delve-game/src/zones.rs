//! Environment-zone multi-camera rendering, ported from TS's single-camera
//! N-pass render loop (`main.ts`'s `!ls.multiZone` branch vs its per-zone
//! `camera.layers.enable(zoneLayer)` loop). This port uses N camera
//! *entities* instead of N passes through one camera: each zone camera is
//! tagged `RenderLayers::from_layers(&[0, zone])`, so it draws both its own
//! zone's per-cell content and everything left on the shared layer (0).
//!
//! Every cell-positioned entity type — dungeon floor/ceiling/walls, doors,
//! keys, plates, tripwires, levers, sconce brackets/arms/torches, enemies,
//! NPCs, ground items, blocks, chests, signs, fountains, bookshelves,
//! altars, barrels, ramps, props, spawners, boulders, thin walls, and wall
//! entities — is tagged with its own cell's zone at spawn time, either via
//! [`tag_cell`] (col/row already in scope in the spawning function) or
//! [`tag_by_key`] (a layer-prefixed handle map, tagged centrally from
//! `level_scene.rs` after the spawn call returns). `RenderLayers` does not
//! propagate from a parent entity to its children (verified against the
//! vendored visibility source: culling reads `Option<&RenderLayers>`
//! directly off the entity being tested, no ancestor walk) — every one of
//! these tagging calls targets the visible mesh entities themselves, never
//! a group root.
//!
//! What's left genuinely shared (tagged with nothing, relying on Bevy's
//! default `RenderLayers` being layer 0, which every zone camera's `[0,
//! zone]` set already includes) is TS's own `enableAll()` list: stairs,
//! trap launcher meshes, sconce lights (the brackets/arms/torches around
//! them ARE tagged; only the light itself stays shared, matching TS's own
//! `light.layers.enableAll()` call), damage numbers, fireball explosion
//! particles, health bars, and projectiles.
//!
//! Single-zone levels never touch any of this: [`spawn_player_cameras`]
//! keeps today's one-entity camera (Camera3d + Projection + DistanceFog +
//! AmbientLight bundled directly on the player) untouched, matching TS's
//! `!multiZone` fast path exactly, and every `tag_cell`/`tag_by_key`/
//! `tag_forest` call is a no-op when the level isn't multi-zone.

use crate::environment::{
    AMBIENT_BRIGHTNESS, EnvironmentConfig, environment_config, resolve_environment_at_cell,
};
use crate::player::Player;
use crate::session::Session;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Camera3dDepthLoadOp, SubCameraView};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use delve_core::env_zones::{build_env_zone_map, build_env_zone_map_with_existing_zones};
use delve_core::game_state::door_key;
use delve_core::types::{DungeonLevel, Environment, TextureArea};
use std::collections::HashMap;

/// Per-layer zone assignment for a level, computed once when its scene is
/// built. Ported from `buildLevelScene`'s split between the active layer's
/// own `buildEnvZoneMap` call (which decides the level's zone list and
/// multi-zone-ness) and every other layer's `buildEnvZoneMapWithExistingZones`
/// call (which reuses that same zone indexing, so a zone number means the
/// same environment on every layer of the level).
#[derive(Resource, Clone, Default)]
pub struct LevelZones {
    pub zones: Vec<Environment>,
    pub multi_zone: bool,
    by_layer: Vec<HashMap<String, usize>>,
}

impl LevelZones {
    #[must_use]
    pub fn zone_at(&self, layer_index: usize, col: i64, row: i64) -> Option<usize> {
        self.by_layer
            .get(layer_index)?
            .get(&door_key(col, row))
            .copied()
    }
}

fn layer_areas(level: &DungeonLevel, layer_index: usize) -> &[TextureArea] {
    level.layers[layer_index]
        .areas
        .as_deref()
        .or(level.areas.as_deref())
        .unwrap_or(&[])
}

/// Ported from the `buildEnvZoneMap`/`buildEnvZoneMapWithExistingZones` split
/// in `buildLevelScene`: `active_layer_index` is `gameState.activeLayerIndex`
/// at scene-build time, captured once and unaffected by later same-scene
/// layer switches (falling, ramps) since those never rebuild the scene.
#[must_use]
pub fn compute_level_zones(level: &DungeonLevel, active_layer_index: usize) -> LevelZones {
    let level_environment = level.environment.unwrap_or(Environment::Dungeon);
    let active_grid = &level.layers[active_layer_index].grid;
    let active_areas = layer_areas(level, active_layer_index);
    let primary = build_env_zone_map(active_grid, level_environment, active_areas);

    let mut by_layer = vec![HashMap::new(); level.layers.len()];
    if primary.multi_zone {
        by_layer[active_layer_index] = primary.zone_by_cell;
        for (layer_index, layer_def) in level.layers.iter().enumerate() {
            if layer_index == active_layer_index {
                continue;
            }
            let areas = layer_areas(level, layer_index);
            by_layer[layer_index] = build_env_zone_map_with_existing_zones(
                &layer_def.grid,
                level_environment,
                areas,
                &primary.zones,
            );
        }
    }

    LevelZones {
        zones: primary.zones,
        multi_zone: primary.multi_zone,
        by_layer,
    }
}

/// Tags a single cell-positioned entity with its own cell's zone. A no-op
/// when the level isn't multi-zone, matching TS's `if (ldZoneMap)` gate —
/// under which the whole tagging pass is skipped and every mesh keeps
/// three.js's default (layer-0-only) mask.
pub fn tag_cell(
    commands: &mut Commands,
    zones: &LevelZones,
    layer_index: usize,
    entity: Entity,
    col: i64,
    row: i64,
) {
    if let Some(zone) = zones.zone_at(layer_index, col, row) {
        commands
            .entity(entity)
            .insert(RenderLayers::from_layers(&[zone]));
    }
}

/// Ported from `buildLevelScene`'s `zones.indexOf(levelEnv) + 1 || 1`: the
/// zone index the level's own default environment landed at when `zones`
/// (the active layer's first-encountered-order environment list) was built,
/// falling back to zone 1 if that environment isn't present in `zones` at
/// all (`indexOf` returns -1, so `-1 + 1 = 0`, and JS's `0 || 1` coerces the
/// falsy zero to 1). Pulled out of [`tag_forest`] as a pure function so the
/// index math is unit-testable without spinning up `Commands`.
#[must_use]
fn forest_zone_index(zones: &[Environment], level_environment: Environment) -> usize {
    zones
        .iter()
        .position(|&environment| environment == level_environment)
        .map_or(1, |index| index + 1)
}

/// Tags a layer's whole forest batch to ONE zone (see [`forest_zone_index`])
/// — forest trees don't carry a per-cell zone the way other cell-positioned
/// content does; TS tags the entire layer's `ForestMeshes` group as a single
/// unit. A no-op when the level isn't multi-zone, matching every other
/// tagging call's `if (ldZoneMap)` gate.
pub fn tag_forest(
    commands: &mut Commands,
    zones: &LevelZones,
    level_environment: Environment,
    entities: impl IntoIterator<Item = Entity>,
) {
    if !zones.multi_zone {
        return;
    }
    let zone = forest_zone_index(&zones.zones, level_environment);
    for entity in entities {
        commands
            .entity(entity)
            .insert(RenderLayers::from_layers(&[zone]));
    }
}

/// Tags every entity in a layer-prefixed handle map (`layer_door_key`
/// format, e.g. `"0:12,7"`) with its cell's zone — the central counterpart
/// to TS's per-builder `tagByKey` closure in `buildLevelScene`. Callers pass
/// the layer-local map (before it's merged into the level-wide accumulator)
/// so the `"{layer_index}:"` prefix strip below is unambiguous.
pub fn tag_by_key<'a>(
    commands: &mut Commands,
    zones: &LevelZones,
    layer_index: usize,
    entities: impl IntoIterator<Item = (&'a String, &'a Entity)>,
) {
    if !zones.multi_zone {
        return;
    }
    let Some(zone_by_cell) = zones.by_layer.get(layer_index) else {
        return;
    };
    let prefix = format!("{layer_index}:");
    for (key, entity) in entities {
        let Some(stripped) = key.strip_prefix(prefix.as_str()) else {
            continue;
        };
        // Multi-item cell spreads suffix the cell key with `#index`
        // (`ground_items.rs`'s `store_key`) — the zone map only knows plain
        // `"col,row"` keys, so look up the cell portion before that suffix.
        let cell_key = stripped.split('#').next().unwrap_or(stripped);
        if let Some(&zone) = zone_by_cell.get(cell_key) {
            commands
                .entity(*entity)
                .insert(RenderLayers::from_layers(&[zone]));
        }
    }
}

/// Marks a camera entity spawned as one of a multi-zone level's per-zone
/// passes, so the next level swap can find and despawn every one of them
/// before respawning for the new level's own zone count. Carries the zone's
/// own environment so a caller iterating every camera entity (`debug.rs`'s
/// fullbright toggle, restoring each one's fog/ambient) can tell a per-zone
/// camera from the single-zone fast path's combined entity, and which
/// environment to restore a per-zone one to, without re-deriving it from
/// `RenderLayers` or threading `LevelZones` through every call site.
#[derive(Component)]
pub struct ZoneCamera(pub Environment);

fn default_projection() -> Projection {
    Projection::Perspective(PerspectiveProjection {
        fov: 75_f32.to_radians(),
        near: 0.1,
        far: 200.0,
        ..default()
    })
}

pub(crate) fn fog_for(environment: Environment) -> DistanceFog {
    let config = environment_config(environment);
    DistanceFog {
        color: config.fog_color,
        falloff: FogFalloff::Linear {
            start: config.fog_near,
            end: config.fog_far,
        },
        ..default()
    }
}

pub(crate) fn ambient_for(environment: Environment) -> AmbientLight {
    let config = environment_config(environment);
    AmbientLight {
        color: config.ambient_color,
        brightness: AMBIENT_BRIGHTNESS,
        affects_lightmapped_meshes: true,
    }
}

/// Removes every zone camera left from a prior level (if any) plus any
/// single-camera components left on `player` (if the prior level was
/// single-zone), then attaches the camera setup this level's zone count
/// needs. Called once per level swap, right after `spawn_level_scene` — safe
/// to call unconditionally regardless of what the *previous* level's zone
/// architecture was, since it always tears down before rebuilding.
///
/// Single-zone levels (`!zones.multi_zone`) get TS's `!multiZone` fast path:
/// one camera, bundled directly on `player`, byte-identical to the
/// pre-multi-zone setup — `level` here (not `zones.zones`) is the same
/// `level.environment` source of truth `setup`/the transition swaps already
/// used, so this fast path is not just equivalent but literally unchanged.
///
/// Multi-zone levels get one child camera per zone: `RenderLayers` including
/// shared layer 0 so untagged content still renders, `Camera.order` equal to
/// the zone index (matching TS's pass order 1..=zones.len()), and
/// `ClearColorConfig::Custom` on only the first zone — TS's
/// `scene.background = i === 0 ? new THREE.Color(cfg.fogColor) : null`.
pub fn spawn_player_cameras(
    commands: &mut Commands,
    player: Entity,
    zone_cameras: &Query<Entity, With<ZoneCamera>>,
    clear_color: &mut ClearColor,
    level: &DungeonLevel,
    zones: &LevelZones,
) {
    for entity in zone_cameras {
        commands.entity(entity).despawn();
    }
    commands.entity(player).remove::<(
        Camera3d,
        Projection,
        DistanceFog,
        AmbientLight,
        RenderLayers,
    )>();

    if !zones.multi_zone {
        let environment = level.environment.unwrap_or(Environment::Dungeon);
        clear_color.0 = environment_config(environment).fog_color;
        commands.entity(player).insert((
            Camera3d::default(),
            default_projection(),
            fog_for(environment),
            ambient_for(environment),
        ));
        return;
    }

    commands.entity(player).with_children(|parent| {
        for (index, &environment) in zones.zones.iter().enumerate() {
            let zone = index + 1;
            parent.spawn((
                ZoneCamera(environment),
                Camera3d {
                    // TS disables autoClear and clears once up front, so all
                    // N passes share one persistent depth buffer — cameras
                    // targeting the same window share one depth texture too
                    // (keyed by (target, msaa)), so only the first zone
                    // clears it; every later zone must load what the first
                    // one wrote or it paints over nearer geometry from
                    // earlier passes regardless of actual depth.
                    depth_load_op: if index == 0 {
                        Camera3dDepthLoadOp::Clear(0.0)
                    } else {
                        Camera3dDepthLoadOp::Load
                    },
                    ..default()
                },
                default_projection(),
                Camera {
                    order: zone as isize,
                    clear_color: if index == 0 {
                        ClearColorConfig::Custom(environment_config(environment).fog_color)
                    } else {
                        ClearColorConfig::None
                    },
                    ..default()
                },
                RenderLayers::from_layers(&[0, zone]),
                Transform::IDENTITY,
                fog_for(environment),
                ambient_for(environment),
            ));
        }
    });
}

/// TS's `main.ts:82-84` (comments verbatim): cut 15% from the top of the
/// frustum for a claustrophobic feel, expand 20% beyond the bottom to
/// reveal more floor. `CAMERA_CROP_SIDE` is the average of the two,
/// applied symmetrically left/right so the resulting frustum is uniformly
/// scaled rather than stretched — undoing the aspect-ratio distortion the
/// asymmetric vertical crop would otherwise introduce.
const CAMERA_CROP_TOP: f32 = 0.15;
const CAMERA_CROP_BOTTOM: f32 = -0.2;
const CAMERA_CROP_SIDE: f32 = (CAMERA_CROP_TOP + CAMERA_CROP_BOTTOM) / 2.0;

/// Ported from TS's `applyCameraViewCrop` (`main.ts:118-125`):
/// `camera.setViewOffset(w, h, cropX, cropTop, w - cropX*2, h - cropTop -
/// cropBottom)`. Bevy's `SubCameraView` is the same mechanism — a camera
/// pretends it's one sub-rectangle of a larger `full_size` frame, which
/// asymmetrically shifts and scales the frustum exactly like `setViewOffset`
/// does (verified term-for-term against both `PerspectiveCamera
/// .updateProjectionMatrix` in `node_modules/three/src/cameras/
/// PerspectiveCamera.js` and Bevy's `PerspectiveProjection
/// ::get_clip_from_view_for_sub` in the vendored `bevy_camera-0.19.0/src/
/// projection.rs` — both produce `top' = top - CROP_TOP*height` and
/// `bottom' = bottom + CROP_BOTTOM*height` from the same inputs).
/// `SubCameraView::offset` is top-down (screen-space) pixels, matching
/// `setViewOffset`'s `x`/`y` params directly — `get_clip_from_view_for_sub`
/// flips it to bottom-up internally, so no sign adjustment is needed here.
///
/// Pixel math mirrors TS's floor-once-then-integer-arithmetic order (floor
/// each crop amount individually, then combine with plain subtraction)
/// rather than flooring only the final result, so rounding lands on the
/// same pixel TS would pick.
#[must_use]
fn camera_view_crop(width: f32, height: f32) -> SubCameraView {
    let full_width = width.floor() as i32;
    let full_height = height.floor() as i32;
    let crop_top = (height * CAMERA_CROP_TOP).floor() as i32;
    let crop_bottom = (height * CAMERA_CROP_BOTTOM).floor() as i32;
    let crop_side = (width * CAMERA_CROP_SIDE).floor() as i32;
    SubCameraView {
        full_size: UVec2::new(full_width.max(0) as u32, full_height.max(0) as u32),
        offset: Vec2::new(crop_side as f32, crop_top as f32),
        size: UVec2::new(
            (full_width - crop_side * 2).max(0) as u32,
            (full_height - crop_top - crop_bottom).max(0) as u32,
        ),
    }
}

/// Applies the view crop to every camera this module owns — the single
/// combined entity in single-zone levels and every `ZoneCamera` child in
/// multi-zone ones both carry `Camera3d`, so one query reaches both
/// architectures identically without needing to know which is active.
///
/// TS recomputes the crop on every window resize (`main.ts:1202-1206`,
/// alongside `camera.aspect` and `renderer.setSize`) in addition to once at
/// startup. Rather than mirror that split (a resize-event listener plus a
/// separate startup call), this runs the same cheap recomputation every
/// frame — the window query and crop math are a handful of float ops, and
/// writing only happens when the computed value actually changed
/// (`SubCameraView` is `PartialEq`), so an unchanged window costs one query
/// and one comparison per camera per frame with no extra writes to trigger
/// Bevy's change detection or the projection-matrix recompute it gates.
pub fn apply_camera_view_crop(
    windows: Query<&Window>,
    mut cameras: Query<&mut Camera, With<Camera3d>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    if window.width() <= 0.0 || window.height() <= 0.0 {
        return;
    }
    let crop = camera_view_crop(window.width(), window.height());
    for mut camera in &mut cameras {
        if camera.sub_camera_view != Some(crop) {
            camera.sub_camera_view = Some(crop);
        }
    }
}

/// TS's `delta * 2` rate in `lerpEnvironment(ctx.scene, ctx.ambient,
/// targetCfg, delta * 2)` (`game/statusEffectSystem.ts:27`).
const ENVIRONMENT_LERP_RATE: f32 = 2.0;

/// Component-wise channel lerp matching THREE.Color's own `Color.lerp` in
/// its actual working space: linear. `ColorManagement.enabled` defaults to
/// true with `LinearSRGBColorSpace` as the working space
/// (`three/src/math/ColorManagement.js:6-8`), and `Color.setHex` converts
/// every hex through `toWorkingColorSpace` before `lerp` ever runs — so the
/// endpoints match either way, but intermediate frames must ease linear
/// channels or they read brighter than TS's.
fn lerp_color(from: Color, to: Color, t: f32) -> Color {
    let from = from.to_linear();
    let to = to.to_linear();
    Color::linear_rgba(
        from.red + (to.red - from.red) * t,
        from.green + (to.green - from.green) * t,
        from.blue + (to.blue - from.blue) * t,
        from.alpha + (to.alpha - from.alpha) * t,
    )
}

/// Ported from `lerpEnvironment` (`rendering/environment.ts:62-79`): eases
/// fog near/far/color, the clear color (TS's `scene.background`), and
/// ambient color toward `target` at rate `t` — TS's own
/// `value += (target - value) * t` on every channel, unclamped (a huge
/// frame hitch can overshoot past `target`; TS never clamps `t` either).
fn lerp_environment(
    fog: &mut DistanceFog,
    ambient: &mut AmbientLight,
    clear_color: &mut ClearColor,
    target: &EnvironmentConfig,
    t: f32,
) {
    if let FogFalloff::Linear { start, end } = &mut fog.falloff {
        *start += (target.fog_near - *start) * t;
        *end += (target.fog_far - *end) * t;
    }
    fog.color = lerp_color(fog.color, target.fog_color, t);
    clear_color.0 = lerp_color(clear_color.0, target.fog_color, t);
    ambient.color = lerp_color(ambient.color, target.ambient_color, t);
}

/// Ported from `tickStatusEffects`'s `!ctx.ls.multiZone` branch
/// (`game/statusEffectSystem.ts:20-29`), itself only reached while
/// `!anyOverlayOpen` (`main.ts:1299-1302`): every unpaused frame, re-resolve
/// the player's current cell against the level's area overrides and ease
/// the shared single-zone camera toward whatever environment that resolves
/// to. Deliberately uses `session.areas` (the level-wide list, matching
/// TS's own `ctx.ls.level.areas` argument) rather than the active layer's
/// own override list — TS's call site never consults the per-layer list
/// here even though scene-building does elsewhere, and this port matches
/// that exactly rather than "fixing" it.
///
/// Gated implicitly on the combined `Player`+`Camera3d` entity actually
/// carrying `DistanceFog`/`AmbientLight`: true only under the single-zone
/// fast path (`spawn_player_cameras`), and false whenever
/// `debug::toggle_fullbright` has stripped `DistanceFog` for the fullbright
/// light — so this system silently no-ops under fullbright with no explicit
/// check needed, matching TS by construction: fullbright sets
/// `scene.fog = null` (`inputSystem.ts:345`) and `lerpEnvironment` early
/// returns on a null fog (`environment.ts:68-69`). Never fights a multi-zone level's
/// per-`ZoneCamera` fog either, since those live on child entities and
/// `Player` itself carries neither component while multi-zone.
pub fn lerp_zone_environment(
    time: Res<Time>,
    session: Res<Session>,
    gate: crate::overlay::InputGate,
    mut clear_color: ResMut<ClearColor>,
    mut cameras: Query<(&Player, &mut DistanceFog, &mut AmbientLight)>,
) {
    if gate.paused() {
        return;
    }
    let Ok((player, mut fog, mut ambient)) = cameras.single_mut() else {
        return;
    };
    let state = player.grid_state();
    let target = environment_config(resolve_environment_at_cell(
        i64::from(state.col),
        i64::from(state.row),
        session.environment,
        &session.areas,
    ));
    let t = time.delta_secs() * ENVIRONMENT_LERP_RATE;
    lerp_environment(&mut fog, &mut ambient, &mut clear_color, &target, t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use delve_core::level_loader::{ValidationContext, validate_dungeon_str};

    fn layered_level() -> DungeonLevel {
        let path = crate::assets_dir().join("levels/dungeon_m1-layered.json");
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let mut warnings = Vec::new();
        let dungeon = validate_dungeon_str(
            &json,
            "dungeon_m1-layered.json",
            &ValidationContext::default(),
            &mut warnings,
        )
        .expect("shipped dungeon_m1-layered.json validates");
        dungeon
            .levels
            .into_iter()
            .find(|level| level.id.as_deref() == Some("level_1"))
            .expect("dungeon_m1-layered.json has level_1")
    }

    #[test]
    fn compute_level_zones_finds_the_dungeon_area_override_over_outdoor_default() {
        let level = layered_level();
        // Layer 1's area override only covers rows 4-16 of its 17-row grid
        // (layer 0's covers every row, making layer 0 alone single-zone) —
        // rows 0-3 stay the level's outdoor default, so layer 1 is the
        // layer whose own grid genuinely spans two zones.
        let zones = compute_level_zones(&level, 1);
        assert!(zones.multi_zone);
        assert_eq!(zones.zones.len(), 2);
        assert!(zones.zones.contains(&Environment::Outdoor));
        assert!(zones.zones.contains(&Environment::Dungeon));
    }

    #[test]
    fn compute_level_zones_reuses_the_active_layers_zone_index_on_other_layers() {
        let level = layered_level();
        let zones = compute_level_zones(&level, 1);
        // Layer 3 has no area override, so (0,0) is the level's outdoor
        // default on every layer — its zone index must match whichever
        // index the active layer (1) assigned to `Environment::Outdoor`,
        // not a fresh first-encountered index of its own.
        let outdoor_zone = zones
            .zones
            .iter()
            .position(|&environment| environment == Environment::Outdoor)
            .map(|index| index + 1)
            .expect("outdoor is one of the level's zones");
        assert_eq!(zones.zone_at(3, 0, 0), Some(outdoor_zone));
    }

    #[test]
    fn compute_level_zones_is_not_multi_zone_for_a_single_environment_level() {
        let level = layered_level();
        // level_2/level_3 in the same dungeon are single-environment; reuse
        // this level's own layer 3 (no area override) in isolation by
        // treating it as the active layer — every cell resolves to the same
        // outdoor default, so there is exactly one zone.
        let zones = compute_level_zones(&level, 3);
        assert!(!zones.multi_zone);
        assert_eq!(zones.zones, vec![Environment::Outdoor]);
    }

    #[test]
    fn forest_zone_index_finds_the_levels_default_environment() {
        let zones = [Environment::Dungeon, Environment::Outdoor];
        assert_eq!(forest_zone_index(&zones, Environment::Outdoor), 2);
    }

    #[test]
    fn forest_zone_index_falls_back_to_one_when_the_default_environment_is_absent() {
        let zones = [Environment::Dungeon, Environment::Mist];
        assert_eq!(forest_zone_index(&zones, Environment::Outdoor), 1);
    }

    /// Hand-computed from TS's own formula: crop_top = floor(1080*0.15) =
    /// 162, crop_bottom = floor(1080*-0.2) = -216, crop_side =
    /// floor(1920*-0.025) = -48 — every intermediate value already a whole
    /// number, so this case can't hide a flooring mistake on its own (see
    /// the fractional case below for that).
    #[test]
    fn camera_view_crop_matches_ts_set_view_offset_for_a_1080p_window() {
        let crop = camera_view_crop(1920.0, 1080.0);
        assert_eq!(crop.full_size, UVec2::new(1920, 1080));
        assert_eq!(crop.offset, Vec2::new(-48.0, 162.0));
        assert_eq!(crop.size, UVec2::new(2016, 1134));
    }

    /// A window size TS's `Math.floor(h * CAMERA_CROP_TOP)` etc. would
    /// genuinely round: crop_top = floor(333*0.15) = floor(49.95) = 49,
    /// crop_bottom = floor(333*-0.2) = floor(-66.6) = -67 (floors toward
    /// negative infinity, not toward zero), crop_side =
    /// floor(777*-0.025) = floor(-19.425) = -20.
    #[test]
    fn camera_view_crop_floors_toward_negative_infinity_like_ts_math_floor() {
        let crop = camera_view_crop(777.0, 333.0);
        assert_eq!(crop.full_size, UVec2::new(777, 333));
        assert_eq!(crop.offset, Vec2::new(-20.0, 49.0));
        assert_eq!(crop.size, UVec2::new(817, 351));
    }

    /// The midpoint is 0.5 in LINEAR channels — the space THREE's
    /// color-managed `Color.lerp` actually eases in. A gamma-space lerp
    /// would land here too for black/white endpoints, so the next test's
    /// asymmetric endpoints are the real space pin; this one covers alpha
    /// and the trivial case.
    #[test]
    fn lerp_color_eases_halfway_between_black_and_white() {
        let mixed = lerp_color(Color::BLACK, Color::WHITE, 0.5).to_linear();
        assert!((mixed.red - 0.5).abs() < 1e-6);
        assert!((mixed.green - 0.5).abs() < 1e-6);
        assert!((mixed.blue - 0.5).abs() < 1e-6);
        assert!((mixed.alpha - 1.0).abs() < 1e-6);
    }

    /// Pins the lerp SPACE with endpoints where gamma and linear midpoints
    /// genuinely differ: black → sRGB 0x88aacc. The linear midpoint is half
    /// the target's linear channels; easing sRGB channels instead would
    /// read brighter (sRGB 0.266667 vs the correct ~0.226 for red).
    #[test]
    fn lerp_color_eases_in_linear_space_not_gamma() {
        let target = Color::srgb_u8(0x88, 0xaa, 0xcc);
        let mixed = lerp_color(Color::BLACK, target, 0.5).to_linear();
        let expected = target.to_linear();
        assert!((mixed.red - expected.red / 2.0).abs() < 1e-6);
        assert!((mixed.green - expected.green / 2.0).abs() < 1e-6);
        assert!((mixed.blue - expected.blue / 2.0).abs() < 1e-6);
    }

    #[test]
    fn lerp_color_at_t_zero_stays_at_the_start_color() {
        let start = Color::srgb_u8(10, 20, 30);
        let mixed = lerp_color(start, Color::srgb_u8(200, 200, 200), 0.0).to_srgba();
        let expected = start.to_srgba();
        assert!((mixed.red - expected.red).abs() < 1e-6);
        assert!((mixed.green - expected.green).abs() < 1e-6);
        assert!((mixed.blue - expected.blue).abs() < 1e-6);
    }

    /// Fog distances lerp in plain float space (fog_near 6.0 → 20.0 and
    /// fog_far 26.0 → 80.0 land at 13.0 and 53.0 halfway); colors lerp in
    /// linear channels, asserted against the same to_linear conversion the
    /// implementation uses so the pinned fact is the midpoint formula and
    /// the space, not a magic constant.
    #[test]
    fn lerp_environment_moves_fog_distances_and_colors_toward_the_target_by_t() {
        let mut fog = fog_for(Environment::Dungeon);
        let mut ambient = ambient_for(Environment::Dungeon);
        let mut clear_color = ClearColor(environment_config(Environment::Dungeon).fog_color);
        let dungeon = environment_config(Environment::Dungeon);
        let target = environment_config(Environment::Outdoor);

        lerp_environment(&mut fog, &mut ambient, &mut clear_color, &target, 0.5);

        let FogFalloff::Linear { start, end } = fog.falloff else {
            panic!("fog_for always builds FogFalloff::Linear");
        };
        assert!((start - 13.0).abs() < 1e-4);
        assert!((end - 53.0).abs() < 1e-4);

        let fog_linear = fog.color.to_linear();
        let fog_expected = lerp_color(dungeon.fog_color, target.fog_color, 0.5).to_linear();
        assert!((fog_linear.red - fog_expected.red).abs() < 1e-6);
        assert!((fog_linear.green - fog_expected.green).abs() < 1e-6);
        assert!((fog_linear.blue - fog_expected.blue).abs() < 1e-6);

        let ambient_linear = ambient.color.to_linear();
        let ambient_expected =
            lerp_color(dungeon.ambient_color, target.ambient_color, 0.5).to_linear();
        assert!((ambient_linear.red - ambient_expected.red).abs() < 1e-6);
        assert!((ambient_linear.green - ambient_expected.green).abs() < 1e-6);
        assert!((ambient_linear.blue - ambient_expected.blue).abs() < 1e-6);

        // Clear color follows the same `target.fog_color`/`t` as the fog
        // color itself — same formula, same inputs.
        assert_eq!(clear_color.0.to_linear(), fog_linear);
    }
}

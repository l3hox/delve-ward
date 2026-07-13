//! Debug tooling: `KeyM` toggles fullbright (bright ambient light, no fog,
//! noclip movement, invincibility, guaranteed melee kills), `KeyY`/`KeyH`
//! fly between layers while fullbright is on — ported from
//! `game/inputSystem.ts`'s `KeyM`/`KeyY`/`KeyH` cases (`main.ts`'s
//! `debugFullbright`/`debugLayerIndex` state, `debugNoClip` on
//! `rendering/player.ts`).
//!
//! [`DebugFlags::fullbright`] is consulted at every TS `!debugFullbright`
//! site this port has:
//! - `session::player_input` (this crate) — `debugNoClip` (TS ties it 1:1
//!   to `debugFullbright`, so there's no separate noclip flag here either).
//! - `enemies::tick_enemies`'s attack arm — TS's `!debugFullbright` half of
//!   `li === savedLayer && !debugFullbright` (`main.ts:1339`).
//! - `enemies::attack_input` — the `KeyF` "auto-kill" (`main.ts:242-247`):
//!   sets the facing enemy's hp to 1 before the swing resolves.
//! - `status_effects::tick_player_vitals` → `delve_core::player_controller
//!   ::tick_player_controller`'s `debug_fullbright` param (status-effect
//!   and starvation damage), already wired for a hardcoded `false`.
//! - `boulders::tick_boulders_system` → `delve_core::boulders::
//!   BoulderContext::debug_fullbright` (fall/roll damage), already wired
//!   for a hardcoded `false`.
//! - `projectiles::apply_projectile_hit`'s `HitType::Player` arm —
//!   `main.ts:976`'s `hitType === 'player' && !debugFullbright`; this port
//!   had no gate here at all before this module.
//!
//! **Multi-zone fog/ambient**: TS's `main.ts:1443` skips recomputing each
//! zone's fog/background/ambient on every one of its N per-frame render
//! passes while `debugFullbright` is on, so the one shared `scene.fog`/
//! `ambient` the debug light set at toggle time survives every pass. This
//! port has no per-frame reapply to skip in the first place — multi-zone
//! levels use N *camera entities* (`zones::spawn_player_cameras`), each
//! carrying its own `DistanceFog`/`AmbientLight` set once at scene-build
//! time, not touched again after. So `toggle_fullbright` gets the
//! equivalent observable result by writing every camera entity's fog/
//! ambient directly when the flag flips, single-zone (the combined
//! `Player`+`Camera3d` entity) and multi-zone (every `zones::ZoneCamera`
//! child) alike, and restoring each one to *its own* environment — the
//! zone's, not necessarily the session's — on toggle-off. `zones::ZoneCamera`
//! carries its zone's `Environment` for exactly this: telling a per-zone
//! camera apart from the single-zone fast path's entity, and which
//! environment to restore it to, in one query.
//!
//! **Not reproduced**: TS also re-syncs `debugLayerIndex` to
//! `activeLayerIndex` on every ramp crossing and fall landing
//! (`main.ts:618,638,705`), even while fullbright is off. Since `KeyM`'s
//! own ON-toggle always resets `layer_index` fresh from
//! `active_layer_index` (`main.ts:346`), that background sync has no
//! observable effect on anything TS itself reads — `debugLayerIndex` is
//! only ever consulted while fullbright is already on. Skipped rather than
//! threading a debug resource through the ramp/fall path for a no-op.

use crate::dungeon::LAYER_HEIGHT;
use crate::environment::AMBIENT_BRIGHTNESS;
use crate::player::Player;
use crate::session::{self, DungeonRes, Session};
use crate::zones::{self, ZoneCamera};
use bevy::prelude::*;
use delve_core::level_loader::resolve_layer_coord;

/// Scales TS's `debugLight`'s `THREE.AmbientLight(0xffffff, 2)` (intensity
/// 2) the same way `AMBIENT_BRIGHTNESS` scales intensity 1.
const DEBUG_LIGHT_BRIGHTNESS: f32 = AMBIENT_BRIGHTNESS * 2.0;

#[derive(Resource, Default)]
pub struct DebugFlags {
    pub fullbright: bool,
    /// The layer `KeyY`/`KeyH` fly between — independent of
    /// `GameState::active_layer_index` only in name; this module keeps
    /// them equal at every point either one changes, matching TS's
    /// `debugLayerIndex` shadowing `gameState.activeLayerIndex`.
    pub layer_index: usize,
}

/// `main.ts:340-358`'s `KeyM` case. `players`/`cameras` are separate
/// queries (rather than one query on the combined single-zone entity) since
/// a multi-zone level's `Player` carries neither `Camera3d` nor
/// `AmbientLight` at all — both moved to its `ZoneCamera` children (see the
/// module doc comment) — so a query requiring both on the same entity
/// matches nothing there, silently no-opping the whole toggle.
#[derive(bevy::ecs::system::SystemParam)]
pub struct FullbrightTargets<'w, 's> {
    players: Query<'w, 's, &'static mut Player>,
    cameras: Query<
        'w,
        's,
        (
            Entity,
            &'static mut AmbientLight,
            Option<&'static ZoneCamera>,
        ),
        With<Camera3d>,
    >,
    commands: Commands<'w, 's>,
}

pub fn toggle_fullbright(
    keys: Res<ButtonInput<KeyCode>>,
    gate: crate::overlay::InputGate,
    mut flags: ResMut<DebugFlags>,
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut targets: FullbrightTargets,
) {
    if gate.blocked() || !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    let Ok(mut player) = targets.players.single_mut() else {
        return;
    };

    flags.fullbright = !flags.fullbright;
    if flags.fullbright {
        for (camera, mut ambient, _zone) in &mut targets.cameras {
            ambient.color = Color::WHITE;
            ambient.brightness = DEBUG_LIGHT_BRIGHTNESS;
            targets.commands.entity(camera).remove::<DistanceFog>();
        }
        flags.layer_index = session.game.active_layer_index;
    } else {
        for (camera, mut ambient, zone) in &mut targets.cameras {
            let environment = zone.map_or(session.environment, |zone_camera| zone_camera.0);
            *ambient = zones::ambient_for(environment);
            targets
                .commands
                .entity(camera)
                .insert(zones::fog_for(environment));
        }

        let home_layer_id = dungeon.0.player_start.layer_index.unwrap_or(0);
        let home_layer = session::find_level_by_id(&dungeon, &session.current_level_id)
            .map_or(0, |level| resolve_layer_coord(level, home_layer_id));
        flags.layer_index = home_layer;
        let Session {
            game,
            grid,
            walkable,
            current_level_id,
            ..
        } = &mut *session;
        session::switch_active_layer(
            game,
            grid,
            walkable,
            &mut player,
            &dungeon,
            current_level_id,
            home_layer,
        );
        player.set_target_y_offset(home_layer as f32 * LAYER_HEIGHT);
    }
    info!(
        "Debug fullbright: {}",
        if flags.fullbright { "ON" } else { "OFF" }
    );
}

/// `main.ts:359-381`'s `KeyY`/`KeyH` cases. `players` has no `With<Camera3d>`
/// for the same reason `toggle_fullbright`'s doesn't — see its doc comment.
pub fn layer_fly(
    keys: Res<ButtonInput<KeyCode>>,
    gate: crate::overlay::InputGate,
    mut flags: ResMut<DebugFlags>,
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut players: Query<&mut Player>,
) {
    if gate.blocked() || !flags.fullbright {
        return;
    }
    let fly_up = keys.just_pressed(KeyCode::KeyY);
    let fly_down = keys.just_pressed(KeyCode::KeyH);
    if !fly_up && !fly_down {
        return;
    }
    let layer_count = session::find_level_by_id(&dungeon, &session.current_level_id)
        .map_or(0, |level| level.layers.len());
    if layer_count <= 1 {
        return;
    }
    let Some(next_layer) = (if fly_up {
        let next = flags.layer_index + 1;
        (next < layer_count).then_some(next)
    } else {
        flags.layer_index.checked_sub(1)
    }) else {
        return;
    };
    let Ok(mut player) = players.single_mut() else {
        return;
    };

    flags.layer_index = next_layer;
    let Session {
        game,
        grid,
        walkable,
        current_level_id,
        ..
    } = &mut *session;
    session::switch_active_layer(
        game,
        grid,
        walkable,
        &mut player,
        &dungeon,
        current_level_id,
        next_layer,
    );
    player.set_target_y_offset(next_layer as f32 * LAYER_HEIGHT);
    info!("Debug fly: layer {next_layer}");
}

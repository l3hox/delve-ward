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
//! **Not implemented, report-first**: `main.ts:1443`'s multi-zone render
//! loop skips recomputing each zone's fog/background/ambient every frame
//! while `debugFullbright` is on (so the debug light/no-fog state isn't
//! immediately overwritten by the next zone's environment). The Rust
//! equivalent lives in `zones.rs`, owned by another agent for this slice —
//! flagged in the completion report rather than touched. Until that lands,
//! fullbright's lighting/fog changes may be visibly overridden on the next
//! frame in a multi-zone level.
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
use crate::environment::{AMBIENT_BRIGHTNESS, environment_config};
use crate::player::Player;
use crate::session::{self, DungeonRes, Session};
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

/// `main.ts:340-358`'s `KeyM` case.
pub fn toggle_fullbright(
    keys: Res<ButtonInput<KeyCode>>,
    gate: crate::overlay::InputGate,
    mut flags: ResMut<DebugFlags>,
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut players: Query<(Entity, &mut Player, &mut AmbientLight), With<Camera3d>>,
    mut commands: Commands,
) {
    if gate.blocked() || !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    let Ok((camera, mut player, mut ambient)) = players.single_mut() else {
        return;
    };

    flags.fullbright = !flags.fullbright;
    if flags.fullbright {
        ambient.color = Color::WHITE;
        ambient.brightness = DEBUG_LIGHT_BRIGHTNESS;
        commands.entity(camera).remove::<DistanceFog>();
        flags.layer_index = session.game.active_layer_index;
    } else {
        let config = environment_config(session.environment);
        ambient.color = config.ambient_color;
        ambient.brightness = AMBIENT_BRIGHTNESS;
        commands.entity(camera).insert(DistanceFog {
            color: config.fog_color,
            falloff: FogFalloff::Linear {
                start: config.fog_near,
                end: config.fog_far,
            },
            ..default()
        });

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

/// `main.ts:359-381`'s `KeyY`/`KeyH` cases.
pub fn layer_fly(
    keys: Res<ButtonInput<KeyCode>>,
    gate: crate::overlay::InputGate,
    mut flags: ResMut<DebugFlags>,
    mut session: ResMut<Session>,
    dungeon: Res<DungeonRes>,
    mut players: Query<&mut Player, With<Camera3d>>,
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

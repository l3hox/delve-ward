//! Player vitals and enemy status-effect visuals: the per-frame player
//! controller tick (status damage-over-time, temp buffs, hunger drain,
//! starvation), the damage flash timer, the slow-effect movement
//! multiplier, and the enemy billboard tint. Enemy damage-over-time itself
//! is already ticked inside `delve_core::enemy_ai::update_enemies` and
//! handled by `enemies::tick_enemies`; this module only adds the ambient
//! tint that reads the result.

use crate::char_creation::InputGate;
use crate::enemies::{EnemyBillboard, EnemyBillboards};
use crate::player::Player;
use crate::session::Session;
use bevy::prelude::*;
use delve_core::player_controller::{
    PLAYER_DAMAGE_FLASH_DURATION, PlayerTickState, tick_player_controller,
};
use delve_core::status_effects::{StatusEffectType, get_slow_multiplier, has_effect};

const BURNING_TINT: Color = Color::srgb_u8(0xFF, 0x88, 0x44);
const POISON_TINT: Color = Color::srgb_u8(0x66, 0xFF, 0x66);

/// The player's per-frame accumulator state (damage flash timer, hunger and
/// starvation accumulators), shared with the HUD's flash overlay.
#[derive(Resource, Default)]
pub struct PlayerVitals(pub PlayerTickState);

impl PlayerVitals {
    /// 0..1 red-overlay strength for the HUD, mirroring the TS
    /// `damageFlashAlpha` computation.
    pub fn damage_flash_alpha(&self) -> f32 {
        (self.0.player_damage_flash_timer / PLAYER_DAMAGE_FLASH_DURATION).clamp(0.0, 1.0) as f32
    }

    pub fn flash(&mut self) {
        self.0.player_damage_flash_timer = PLAYER_DAMAGE_FLASH_DURATION;
    }
}

/// The per-frame player controller tick: status effect damage, temp buff
/// decay, hunger drain, and starvation. Death detection (and the resulting
/// save/restart flow) is centralized in `save_load::check_player_death`,
/// which runs later in the same gated chain.
pub fn tick_player_vitals(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut vitals: ResMut<PlayerVitals>,
    gate: InputGate,
) {
    if gate.blocked() {
        return;
    }
    let delta = f64::from(time.delta_secs());
    // Decrement before the tick so damage taken this frame flashes for the
    // full duration.
    vitals.0.player_damage_flash_timer = (vitals.0.player_damage_flash_timer - delta).max(0.0);

    let game = &mut session.game;
    tick_player_controller(game, &mut vitals.0, delta, false);
}

/// `ls.player.slowMultiplier = getSlowMultiplier(gameState.playerStatusEffects);`
/// — read every frame regardless of the character-creation gate, matching
/// where TS sets it (outside the `anyOverlayOpen` guard).
pub fn apply_slow_multiplier(session: Res<Session>, mut players: Query<&mut Player>) {
    let Ok(mut player) = players.single_mut() else {
        return;
    };
    player.slow_multiplier =
        get_slow_multiplier(&session.game.status_fx.player_status_effects) as f32;
}

/// Tints each enemy billboard's material by its active status effect —
/// burning orange, poison green, otherwise back to white (no tint). Runs
/// unconditionally every frame, matching the TS comment on this block:
/// "Status effect tint on enemies (always — static visual)".
pub fn tint_enemy_status_effects(
    session: Res<Session>,
    billboards: Res<EnemyBillboards>,
    billboard_materials: Query<&MeshMaterial3d<StandardMaterial>, With<EnemyBillboard>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (key, enemy) in &session.game.active_layer().enemies {
        let Some(&entity) = billboards.by_key.get(key) else {
            continue;
        };
        let Ok(material_handle) = billboard_materials.get(entity) else {
            continue;
        };
        let Some(mut material) = materials.get_mut(&material_handle.0) else {
            continue;
        };
        material.base_color = if has_effect(&enemy.status_effects, StatusEffectType::Burning) {
            BURNING_TINT
        } else if has_effect(&enemy.status_effects, StatusEffectType::Poison) {
            POISON_TINT
        } else {
            Color::WHITE
        };
    }
}

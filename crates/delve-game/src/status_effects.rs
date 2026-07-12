//! Player and enemy status-effect ticking: player damage-over-time, the
//! slow-effect movement multiplier, and the enemy billboard tint — ported
//! from the effect portion of TS's `tickPlayerController` (called by
//! `statusEffectSystem.tickStatusEffects`) and the always-on enemy tint
//! block in `main.ts`'s per-frame loop. Enemy damage-over-time itself is
//! already ticked inside `delve_core::enemy_ai::update_enemies` and handled
//! by `enemies::tick_enemies` (`StatusDamage`/`StatusKill` actions); this
//! module only adds the ambient tint that reads the result.
//!
//! Hunger, starvation, and temp buffs aren't ported anywhere in delve-game
//! yet, so `tickPlayerController`'s hunger/starvation portion stays out of
//! scope here too.

use crate::char_creation::InputGate;
use crate::enemies::{EnemyBillboard, EnemyBillboards};
use crate::player::Player;
use crate::session::Session;
use bevy::prelude::*;
use delve_core::status_effects::{StatusEffectType, get_slow_multiplier, has_effect, tick_effects};

const BURNING_TINT: Color = Color::srgb_u8(0xFF, 0x88, 0x44);
const POISON_TINT: Color = Color::srgb_u8(0x66, 0xFF, 0x66);

/// Player damage-over-time, ported from TS's `tickEffects(gameState.playerStatusEffects, delta)`
/// call in `tickPlayerController`. The death log only fires on the tick HP
/// actually crosses to zero — TS transitions into a save/restart overlay at
/// that point (a later slice here), so without that guard this would log
/// every frame while the player stays dead.
pub fn tick_player_status_effects(time: Res<Time>, mut session: ResMut<Session>, gate: InputGate) {
    if gate.blocked() {
        return;
    }
    let delta = f64::from(time.delta_secs());
    let game = &mut session.game;
    let was_alive = game.player.hp > 0.0;

    let result = tick_effects(&mut game.status_fx.player_status_effects, delta);
    if result.damage > 0.0 {
        game.player.hp = (game.player.hp - result.damage).max(0.0);
    }
    game.status_fx
        .player_status_effects
        .retain(|effect| effect.remaining > 0.0);

    if was_alive && game.player.hp <= 0.0 {
        info!("You died.");
    }
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

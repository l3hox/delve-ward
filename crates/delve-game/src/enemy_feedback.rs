//! Enemy combat feedback: damage flash tint, hit-shake, and world-space
//! health bar billboards. Ported from the TS `main.ts`'s `enemyDamageFlash`
//! closure, `rendering/enemyAnimator.ts`'s hit-shake fields (lunge and the
//! unread `hitPhase` bookkeeping are not ported — nothing in this slice
//! triggers a lunge, and `hitPhase` is written but never read in the TS
//! source), and `rendering/enemyHealthBar.ts`'s `EnemyHealthBarManager`.
//!
//! The health bar is a child of the enemy billboard rather than TS's
//! independently-tracked sprite: a pure-Y local offset on a yaw-rotated
//! parent stays vertical (rotating around Y never moves the Y axis), so
//! position sync and camera-facing both come for free from Bevy's transform
//! hierarchy — TS needs its own `updatePositions`/`updateBillboards` passes
//! only because its THREE.js sprites aren't parented to the enemy mesh. The
//! bar itself is two flat-colored quads (background + fill) instead of TS's
//! redrawn canvas texture — this engine has no cheap per-instance texture
//! update path, and a scaled quad is visually equivalent to a filled rect
//! at this size. The 1px canvas border TS draws is skipped as imperceptible
//! at world scale.

use crate::enemies::EnemyBillboards;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use delve_core::enemies::EnemyDatabase;
use delve_core::game_state::{LayerState, layer_door_key};
use std::collections::HashMap;

/// Matches TS's `ENEMY_DAMAGE_FLASH_DURATION` (`main.ts`).
pub const ENEMY_DAMAGE_FLASH_DURATION: f32 = 0.12;
/// The color `tint_enemy_status_effects` applies while a flash timer is
/// still counting down.
pub const ENEMY_DAMAGE_FLASH_COLOR: Color = Color::srgb(1.0, 0.0, 0.0);

const HIT_SHAKE_DURATION: f32 = 0.3;
const HIT_SHAKE_FREQUENCY: f32 = 40.0;
const HIT_SHAKE_AMPLITUDE: f32 = 0.25;

const BAR_FULL_WIDTH: f32 = 0.6;
const BAR_HEIGHT: f32 = 0.1;
const BAR_Y_OFFSET: f32 = 0.12;
const BAR_BG_COLOR: Color = Color::srgb_u8(0x22, 0x22, 0x22);

/// Per-enemy-billboard damage flash timer. Always present (spawned at
/// zero) so `status_effects::tint_enemy_status_effects` can consult and
/// decay it every frame as the single writer of the billboard's
/// `base_color`, rather than racing a second system that also writes it.
#[derive(Component, Default)]
pub struct EnemyDamageFlash {
    pub timer: f32,
}

/// Positional hit-shake offset, ported from `EnemyAnimator`'s `hitTimer`
/// field. `prev_offset` mirrors TS's subtract-then-recompute-then-add
/// technique against `pos.x` directly, since nothing else besides a move
/// snap (which overwrites `Transform.translation.x` outright) touches it.
#[derive(Component, Default)]
pub struct EnemyHitShake {
    timer: f32,
    prev_offset: f32,
}

/// Marks a health bar's fill quad, so its `Transform`/material query can be
/// proven disjoint from other `Transform` queries in the same system — the
/// enemy billboard's own `Query<&mut Transform, With<EnemyBillboard>>` most
/// notably.
#[derive(Component)]
pub struct HealthBarFill;

/// `Without<EnemyBillboard>` isn't redundant with `With<HealthBarFill>`:
/// Bevy only proves two same-component queries disjoint via an explicit
/// With/Without pair on the SAME marker on both sides, not just two
/// different `With<T>` filters — without it, this conflicts with any
/// caller's own `Query<&mut Transform, With<EnemyBillboard>>` (see
/// `enemies::EnemyRenderState::transforms`) and panics at schedule build.
type BarFillQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static MeshMaterial3d<StandardMaterial>,
    ),
    (With<HealthBarFill>, Without<crate::enemies::EnemyBillboard>),
>;

struct HealthBarHandles {
    anchor: Entity,
    fill: Entity,
    last_hp: f64,
    last_max_hp: f64,
}

/// Enemy health bar entities by cell key, re-keyed as enemies move and
/// dropped on kill — mirrors `EnemyBillboards`. The bar's own position and
/// camera-facing come for free from the parent-child hierarchy (see the
/// module doc comment), so this only needs to track redraw state.
#[derive(Resource, Default)]
pub struct EnemyHealthBars {
    by_key: HashMap<String, HealthBarHandles>,
}

impl EnemyHealthBars {
    /// No-op for an unknown key, matching TS's `EnemyHealthBarManager.rekey`.
    pub fn rekey(&mut self, old_key: &str, new_key: &str) {
        if let Some(entry) = self.by_key.remove(old_key) {
            self.by_key.insert(new_key.to_string(), entry);
        }
    }

    /// No-op for an unknown key, matching TS's `EnemyHealthBarManager.remove`.
    /// Despawning the bar's own entities isn't needed here — they're
    /// children of the enemy billboard, which the caller already despawns.
    pub fn remove(&mut self, key: &str) {
        self.by_key.remove(key);
    }

    /// Merges another layer's spawn result in, keyed by its own
    /// layer-prefixed keys.
    pub(crate) fn extend(&mut self, other: Self) {
        self.by_key.extend(other.by_key);
    }
}

/// 0..1, floored at zero — matches TS's `Math.max(0, hp / maxHp)`.
fn health_bar_ratio(hp: f64, max_hp: f64) -> f64 {
    if max_hp > 0.0 {
        (hp / max_hp).max(0.0)
    } else {
        0.0
    }
}

/// Matches TS's `entry.sprite.visible = hp < maxHp;`.
fn health_bar_visible(hp: f64, max_hp: f64) -> bool {
    hp < max_hp
}

/// Matches TS's `if (hp === entry.lastHp && maxHp === entry.lastMaxHp) return;`.
fn should_redraw(hp: f64, max_hp: f64, last_hp: f64, last_max_hp: f64) -> bool {
    (hp - last_hp).abs() > f64::EPSILON || (max_hp - last_max_hp).abs() > f64::EPSILON
}

/// Matches TS's `getFillColor`.
fn fill_color_for_ratio(ratio: f64) -> Color {
    if ratio > 0.7 {
        Color::srgb_u8(0x33, 0xcc, 0x33)
    } else if ratio > 0.3 {
        Color::srgb_u8(0xcc, 0xcc, 0x33)
    } else {
        Color::srgb_u8(0xcc, 0x33, 0x33)
    }
}

/// The fill quad's local X so it grows/shrinks from a fixed left edge (the
/// quad mesh spans `full_width` at `scale.x == 1.0`, centered on its own
/// origin) — a scale-and-reposition equivalent of TS's
/// `ctx.fillRect(1, 1, fillWidth, h)` filling from the bar's left border.
fn fill_translation_x(ratio: f64, full_width: f32) -> f32 {
    -full_width * (1.0 - ratio as f32) / 2.0
}

/// Local Y offset from the enemy billboard's own origin to the bar anchor —
/// matches TS's tested `updatePositions` formula
/// (`worldPos.y + spriteHeight * 0.5 + BAR_Y_OFFSET`), applied here as a
/// pure local-space offset since the parent transform already supplies
/// `worldPos.y`.
fn bar_anchor_y_offset(sprite_height: f32) -> f32 {
    sprite_height * 0.5 + BAR_Y_OFFSET
}

/// Spawns a hidden health bar (background + fill quad) as a child of every
/// enemy billboard, matching `EnemyHealthBarManager::create`'s "start
/// hidden — full HP" behaviour. Must run after
/// `enemies::spawn_enemy_billboards` in the same scene build, since it
/// looks up each enemy's already-spawned billboard entity as the parent.
pub fn spawn_health_bars(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    layer_state: &LayerState,
    layer_spawn: &crate::dungeon::LayerSpawn,
    billboards: &EnemyBillboards,
    database: &EnemyDatabase,
) -> EnemyHealthBars {
    let mut health_bars = EnemyHealthBars::default();
    let bg_mesh = meshes.add(Rectangle::new(BAR_FULL_WIDTH, BAR_HEIGHT));
    let bg_material = materials.add(StandardMaterial {
        base_color: BAR_BG_COLOR,
        unlit: true,
        cull_mode: None,
        ..default()
    });
    let fill_mesh = meshes.add(Rectangle::new(BAR_FULL_WIDTH, BAR_HEIGHT * 0.7));

    for (key, enemy) in &layer_state.enemies {
        let render_key = layer_door_key(layer_spawn.index, key);
        let Some(&parent) = billboards.by_key.get(&render_key) else {
            continue;
        };
        let def = database.get_enemy(&enemy.enemy_type);
        let (sprite_height, _) = crate::enemies::sprite_dimensions(def);

        let anchor = commands
            .spawn((
                Transform::from_xyz(0.0, bar_anchor_y_offset(sprite_height), 0.001),
                Visibility::Hidden,
            ))
            .id();
        commands.entity(parent).add_child(anchor);

        let bg = commands
            .spawn((
                Mesh3d(bg_mesh.clone()),
                MeshMaterial3d(bg_material.clone()),
                Transform::IDENTITY,
            ))
            .id();
        commands.entity(anchor).add_child(bg);

        let fill_material = materials.add(StandardMaterial {
            base_color: fill_color_for_ratio(1.0),
            unlit: true,
            cull_mode: None,
            ..default()
        });
        let fill = commands
            .spawn((
                Mesh3d(fill_mesh.clone()),
                MeshMaterial3d(fill_material),
                Transform::from_xyz(0.0, 0.0, 0.001),
                HealthBarFill,
            ))
            .id();
        commands.entity(anchor).add_child(fill);

        health_bars.by_key.insert(
            render_key,
            HealthBarHandles {
                anchor,
                fill,
                last_hp: enemy.hp,
                last_max_hp: enemy.max_hp,
            },
        );
    }

    health_bars
}

/// Enemy hit-flash and hit-shake queries, plus the health bar tracking
/// resource — bundled so the combat-feedback call sites (melee, projectile,
/// and the AI tick's status-effect ticks) stay under the argument-count
/// lint. `visibility` and `materials` are deliberately NOT fields here:
/// each caller already owns a `Query<&mut Visibility>`/
/// `Assets<StandardMaterial>` of its own (wall reveals, ground item
/// materials), and a second field of either type here would conflict with
/// it under Bevy's per-system access check — see `update_health_bar`.
#[derive(SystemParam)]
pub struct CombatFeedback<'w, 's> {
    flashes: Query<'w, 's, &'static mut EnemyDamageFlash>,
    shakes: Query<'w, 's, &'static mut EnemyHitShake>,
    pub health_bars: ResMut<'w, EnemyHealthBars>,
    bar_fills: BarFillQuery<'w, 's>,
}

impl CombatFeedback<'_, '_> {
    /// Ported from the TS `enemyDamageFlash` closure's trigger half — the
    /// decay and material write live in
    /// `status_effects::tint_enemy_status_effects`.
    pub fn flash(&mut self, entity: Entity) {
        if let Ok(mut flash) = self.flashes.get_mut(entity) {
            flash.timer = ENEMY_DAMAGE_FLASH_DURATION;
        }
    }

    /// Ported from TS's `EnemyAnimator.triggerHit`.
    pub fn trigger_hit_shake(&mut self, entity: Entity) {
        if let Ok(mut shake) = self.shakes.get_mut(entity) {
            shake.timer = HIT_SHAKE_DURATION;
        }
    }

    /// Ported from TS's `EnemyHealthBarManager.update`. No-op for an
    /// unknown key (an enemy on a background layer with no spawned bar —
    /// see `projectiles.rs`'s module doc comment on the single-active-layer
    /// constraint).
    pub fn update_health_bar(
        &mut self,
        visibility: &mut Query<&mut Visibility>,
        materials: &mut Assets<StandardMaterial>,
        key: &str,
        hp: f64,
        max_hp: f64,
    ) {
        let Some(entry) = self.health_bars.by_key.get_mut(key) else {
            return;
        };
        if !should_redraw(hp, max_hp, entry.last_hp, entry.last_max_hp) {
            return;
        }
        entry.last_hp = hp;
        entry.last_max_hp = max_hp;

        if let Ok(mut visible) = visibility.get_mut(entry.anchor) {
            *visible = if health_bar_visible(hp, max_hp) {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
        let ratio = health_bar_ratio(hp, max_hp);
        if let Ok((mut transform, material_handle)) = self.bar_fills.get_mut(entry.fill) {
            transform.scale.x = (ratio as f32).max(0.001);
            transform.translation.x = fill_translation_x(ratio, BAR_FULL_WIDTH);
            if let Some(mut material) = materials.get_mut(&material_handle.0) {
                material.base_color = fill_color_for_ratio(ratio);
            }
        }
    }
}

/// Undoes last frame's shake offset, computes this frame's, and reapplies —
/// the same subtract/recompute/add shape as TS's `EnemyAnimator.update`,
/// applied directly to `Transform.translation.x`. Ungated, like every other
/// small per-frame animation timer in this crate (damage numbers, chest
/// lids) — TS's own `enemyAnimator.update(delta)` call sits in the same
/// always-runs block as those.
pub fn tick_enemy_hit_shake(
    time: Res<Time>,
    mut shakes: Query<(&mut EnemyHitShake, &mut Transform)>,
) {
    let delta = time.delta_secs();
    for (mut shake, mut transform) in &mut shakes {
        transform.translation.x -= shake.prev_offset;
        if shake.timer <= 0.0 {
            shake.prev_offset = 0.0;
            continue;
        }
        let elapsed = HIT_SHAKE_DURATION - shake.timer;
        let offset = (elapsed * HIT_SHAKE_FREQUENCY).sin()
            * HIT_SHAKE_AMPLITUDE
            * (shake.timer / HIT_SHAKE_DURATION);
        transform.translation.x += offset;
        shake.prev_offset = offset;
        shake.timer = (shake.timer - delta).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handles(hp: f64, max_hp: f64) -> HealthBarHandles {
        HealthBarHandles {
            anchor: Entity::PLACEHOLDER,
            fill: Entity::PLACEHOLDER,
            last_hp: hp,
            last_max_hp: max_hp,
        }
    }

    #[test]
    fn ratio_floors_at_zero_and_handles_zero_max_hp() {
        assert!((health_bar_ratio(7.0, 10.0) - 0.7).abs() < 1e-9);
        assert_eq!(health_bar_ratio(-3.0, 10.0), 0.0);
        assert_eq!(health_bar_ratio(5.0, 0.0), 0.0);
    }

    #[test]
    fn visible_below_max_hidden_at_max() {
        assert!(health_bar_visible(7.0, 10.0));
        assert!(!health_bar_visible(10.0, 10.0));
    }

    #[test]
    fn redraw_skipped_only_when_both_values_unchanged() {
        assert!(!should_redraw(7.0, 10.0, 7.0, 10.0));
        assert!(should_redraw(4.0, 10.0, 7.0, 10.0));
        assert!(should_redraw(7.0, 12.0, 7.0, 10.0));
    }

    #[test]
    fn fill_color_thresholds() {
        assert_eq!(fill_color_for_ratio(0.9), Color::srgb_u8(0x33, 0xcc, 0x33));
        assert_eq!(fill_color_for_ratio(0.5), Color::srgb_u8(0xcc, 0xcc, 0x33));
        assert_eq!(fill_color_for_ratio(0.2), Color::srgb_u8(0xcc, 0x33, 0x33));
    }

    #[test]
    fn fill_translation_keeps_left_edge_fixed() {
        assert_eq!(fill_translation_x(1.0, 0.6), 0.0);
        assert!((fill_translation_x(0.5, 0.6) - (-0.15)).abs() < 1e-6);
        assert!((fill_translation_x(0.0, 0.6) - (-0.3)).abs() < 1e-6);
    }

    #[test]
    fn anchor_y_offset_matches_ts_update_positions_formula() {
        // worldPos.y(1) + spriteHeight(2.0)*0.5 + BAR_Y_OFFSET(0.12) == 2.12,
        // ported here as a pure local offset (see the function doc comment).
        assert!((bar_anchor_y_offset(2.0) - 1.12).abs() < 1e-6);
    }

    #[test]
    fn rekey_moves_entry_and_is_noop_for_unknown_key() {
        let mut bars = EnemyHealthBars::default();
        bars.by_key.insert("1,1".to_string(), handles(10.0, 10.0));
        bars.rekey("1,1", "2,1");
        assert!(!bars.by_key.contains_key("1,1"));
        assert!(bars.by_key.contains_key("2,1"));
        bars.rekey("99,99", "1,1");
        assert!(!bars.by_key.contains_key("99,99"));
    }

    #[test]
    fn remove_deletes_entry_and_is_noop_for_unknown_key() {
        let mut bars = EnemyHealthBars::default();
        bars.by_key.insert("5,5".to_string(), handles(8.0, 8.0));
        bars.remove("5,5");
        assert!(!bars.by_key.contains_key("5,5"));
        bars.remove("99,99"); // must not panic
    }
}

//! Status effect system: poison, slow, and burning on the player and enemies.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusEffectType {
    Poison,
    Slow,
    Burning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEffect {
    #[serde(rename = "type")]
    pub effect_type: StatusEffectType,
    /// Seconds left.
    pub remaining: f64,
    /// Accumulator for periodic damage.
    pub tick_timer: f64,
    /// Seconds between damage ticks.
    pub tick_interval: f64,
    /// Damage per tick (0 for slow).
    pub tick_damage: f64,
}

fn effect_defaults(effect_type: StatusEffectType) -> StatusEffect {
    let (tick_interval, tick_damage) = match effect_type {
        StatusEffectType::Poison => (1.0, 2.0),
        StatusEffectType::Slow => (0.0, 0.0),
        StatusEffectType::Burning => (1.0, 3.0),
    };
    StatusEffect {
        effect_type,
        remaining: 0.0,
        tick_timer: 0.0,
        tick_interval,
        tick_damage,
    }
}

#[derive(Debug, PartialEq)]
pub struct TickResult {
    pub damage: f64,
    pub expired_types: Vec<StatusEffectType>,
}

/// Add or refresh a status effect. Same-type refreshes to
/// `max(remaining, duration)`; no damage stacking.
pub fn apply_effect(
    effects: &mut Vec<StatusEffect>,
    effect_type: StatusEffectType,
    duration: f64,
) {
    if let Some(existing) = effects
        .iter_mut()
        .find(|effect| effect.effect_type == effect_type)
    {
        existing.remaining = existing.remaining.max(duration);
    } else {
        let mut effect = effect_defaults(effect_type);
        effect.remaining = duration;
        effects.push(effect);
    }
}

/// Tick all effects by `delta` seconds. Returns accumulated damage and the
/// newly-expired types.
pub fn tick_effects(effects: &mut [StatusEffect], delta: f64) -> TickResult {
    let mut damage = 0.0;
    let mut expired_types = Vec::new();

    for effect in effects.iter_mut() {
        effect.remaining -= delta;

        if effect.tick_interval > 0.0 && effect.tick_damage > 0.0 {
            effect.tick_timer += delta;
            while effect.tick_timer >= effect.tick_interval {
                effect.tick_timer -= effect.tick_interval;
                damage += effect.tick_damage;
            }
        }

        if effect.remaining <= 0.0 {
            expired_types.push(effect.effect_type);
        }
    }

    TickResult {
        damage,
        expired_types,
    }
}

/// Remove all effects of a given type.
#[must_use]
pub fn remove_effects_by_type(
    effects: &[StatusEffect],
    effect_type: StatusEffectType,
) -> Vec<StatusEffect> {
    effects
        .iter()
        .filter(|effect| effect.effect_type != effect_type)
        .cloned()
        .collect()
}

#[must_use]
pub fn has_effect(effects: &[StatusEffect], effect_type: StatusEffectType) -> bool {
    effects
        .iter()
        .any(|effect| effect.effect_type == effect_type)
}

/// Returns 2.0 if slow is active, 1.0 otherwise.
#[must_use]
pub fn get_slow_multiplier(effects: &[StatusEffect]) -> f64 {
    if has_effect(effects, StatusEffectType::Slow) {
        2.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use StatusEffectType::{Burning, Poison, Slow};

    #[test]
    fn apply_adds_a_new_effect() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].effect_type, Poison);
        assert_eq!(effects[0].remaining, 5.0);
        assert_eq!(effects[0].tick_damage, 2.0);
        assert_eq!(effects[0].tick_interval, 1.0);
    }

    #[test]
    fn apply_refreshes_duration_without_stacking() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);
        apply_effect(&mut effects, Poison, 3.0);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].remaining, 5.0);

        apply_effect(&mut effects, Poison, 8.0);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].remaining, 8.0);
    }

    #[test]
    fn apply_allows_different_types_simultaneously() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);
        apply_effect(&mut effects, Burning, 3.0);
        apply_effect(&mut effects, Slow, 4.0);
        assert_eq!(effects.len(), 3);
        let mut types: Vec<StatusEffectType> =
            effects.iter().map(|effect| effect.effect_type).collect();
        types.sort_by_key(|effect_type| format!("{effect_type:?}"));
        assert_eq!(types, vec![Burning, Poison, Slow]);
    }

    #[test]
    fn tick_applies_damage_at_intervals() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);

        assert_eq!(tick_effects(&mut effects, 0.5).damage, 0.0);
        assert_eq!(tick_effects(&mut effects, 0.5).damage, 2.0);
        assert_eq!(tick_effects(&mut effects, 1.0).damage, 2.0);
    }

    #[test]
    fn tick_returns_expired_types() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 1.5);

        assert!(tick_effects(&mut effects, 1.0).expired_types.is_empty());
        assert_eq!(tick_effects(&mut effects, 0.6).expired_types, vec![Poison]);
    }

    #[test]
    fn tick_handles_multiple_simultaneous_effects() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);
        apply_effect(&mut effects, Burning, 3.0);
        assert_eq!(tick_effects(&mut effects, 1.0).damage, 5.0);
    }

    #[test]
    fn slow_does_not_deal_damage() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Slow, 5.0);
        assert_eq!(tick_effects(&mut effects, 2.0).damage, 0.0);
    }

    #[test]
    fn tick_handles_large_delta_with_multiple_ticks() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 10.0);
        assert_eq!(tick_effects(&mut effects, 3.0).damage, 6.0);
    }

    #[test]
    fn slow_multiplier_reflects_active_slow() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Slow, 5.0);
        assert_eq!(get_slow_multiplier(&effects), 2.0);

        let mut without_slow = Vec::new();
        assert_eq!(get_slow_multiplier(&without_slow), 1.0);
        apply_effect(&mut without_slow, Poison, 5.0);
        assert_eq!(get_slow_multiplier(&without_slow), 1.0);
    }

    #[test]
    fn remove_clears_specific_type() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);
        apply_effect(&mut effects, Burning, 3.0);

        let result = remove_effects_by_type(&effects, Poison);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].effect_type, Burning);
    }

    #[test]
    fn remove_leaves_others_untouched() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);
        apply_effect(&mut effects, Slow, 4.0);
        apply_effect(&mut effects, Burning, 3.0);

        let result = remove_effects_by_type(&effects, Slow);
        assert_eq!(result.len(), 2);
        assert!(has_effect(&result, Poison));
        assert!(has_effect(&result, Burning));
    }

    #[test]
    fn remove_returns_empty_when_removing_only_effect() {
        let mut effects = Vec::new();
        apply_effect(&mut effects, Poison, 5.0);
        assert!(remove_effects_by_type(&effects, Poison).is_empty());
    }

    #[test]
    fn has_effect_reports_presence() {
        let mut effects = Vec::new();
        assert!(!has_effect(&effects, Burning));
        apply_effect(&mut effects, Burning, 3.0);
        assert!(has_effect(&effects, Burning));
        assert!(!has_effect(&effects, Poison));
    }
}

//! Pure combat logic: weapon behavior, damage math, and the player attack
//! flow. Randomness is injected (`random` yields floats in [0, 1)); the TS
//! original calls `Math.random` directly.

use crate::game_state::GameState;
use crate::grid::{PlayerState, get_facing_cell};
use crate::items::ItemSubtype;
use crate::loot::DropsOverride;

/// Default fallback when no weapon (or an unknown subtype) is equipped.
pub const PLAYER_ATTACK_COOLDOWN: f64 = 0.8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponBehavior {
    pub cooldown: f64,
    pub damage_multiplier: f64,
}

#[must_use]
pub fn weapon_behavior(subtype: ItemSubtype) -> Option<WeaponBehavior> {
    let (cooldown, damage_multiplier) = match subtype {
        ItemSubtype::Sword => (0.8, 1.0),
        ItemSubtype::Axe => (1.2, 1.5),
        ItemSubtype::Dagger => (0.5, 0.7),
        ItemSubtype::Mace => (1.1, 1.3),
        ItemSubtype::Spear => (0.9, 1.1),
        ItemSubtype::Staff => (1.0, 0.8),
        _ => return None,
    };
    Some(WeaponBehavior {
        cooldown,
        damage_multiplier,
    })
}

/// Attack cooldown for the currently equipped weapon.
#[must_use]
pub fn get_weapon_cooldown(game_state: &GameState) -> f64 {
    game_state
        .get_equipped_weapon_def()
        .and_then(|def| weapon_behavior(def.subtype))
        .map_or(PLAYER_ATTACK_COOLDOWN, |behavior| behavior.cooldown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatResultType {
    Miss,
    Hit,
    Kill,
    NoTarget,
    Cooldown,
    WallHit,
    WallDestroy,
    BarrelHit,
    BarrelDestroy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CombatResult {
    pub result_type: CombatResultType,
    pub damage: Option<f64>,
    pub target_col: Option<i64>,
    pub target_row: Option<i64>,
    pub enemy_type: Option<String>,
    pub drops_override: Option<DropsOverride>,
}

impl CombatResult {
    fn of(result_type: CombatResultType) -> Self {
        Self {
            result_type,
            damage: None,
            target_col: None,
            target_row: None,
            enemy_type: None,
            drops_override: None,
        }
    }
}

/// Calculate damage: `max(1, atk - def + random(-1..=+1))`.
pub fn calculate_damage(atk: f64, def: f64, random: &mut dyn FnMut() -> f64) -> f64 {
    let roll = (random() * 3.0).floor() - 1.0;
    (atk - def + roll).max(1.0)
}

/// Resolve weapon effect: applies weapon subtype multiplier and specials.
/// Axe ignores 1 DEF; dagger overrides crit chance to a flat 10%; mace gains
/// +2 damage against armored enemies.
pub fn resolve_weapon_effect(
    subtype: Option<ItemSubtype>,
    atk: f64,
    enemy_def: f64,
    crit_chance: f64,
    random: &mut dyn FnMut() -> f64,
) -> (f64, bool) {
    let multiplier = subtype
        .and_then(weapon_behavior)
        .map_or(1.0, |behavior| behavior.damage_multiplier);

    let mut effective_def = enemy_def;
    let mut bonus_damage = 0.0;
    let mut effective_crit_chance = crit_chance;

    match subtype {
        Some(ItemSubtype::Axe) => effective_def = (enemy_def - 1.0).max(0.0),
        Some(ItemSubtype::Dagger) => effective_crit_chance = 10.0,
        Some(ItemSubtype::Mace) => {
            if enemy_def > 0.0 {
                bonus_damage = 2.0;
            }
        }
        _ => {}
    }

    let is_crit = random() * 100.0 < effective_crit_chance;
    let base_damage = calculate_damage(atk, effective_def, random);
    let mut final_damage = (base_damage * multiplier).floor() + bonus_damage;
    if is_crit {
        final_damage = (final_damage * 1.5).floor();
    }
    (final_damage.max(1.0), is_crit)
}

/// Player attacks the cell they're facing. Handles weapon subtype specials
/// including the spear's 2-cell range. Returns results for all cells hit.
pub fn player_attack(
    player_state: &PlayerState,
    game_state: &mut GameState,
    random: &mut dyn FnMut() -> f64,
) -> Vec<CombatResult> {
    if game_state.player.attack_cooldown > 0.0 {
        return vec![CombatResult::of(CombatResultType::Cooldown)];
    }

    let cooldown = get_weapon_cooldown(game_state);
    let subtype = game_state.get_equipped_weapon_def().map(|def| def.subtype);
    let stats = game_state.get_effective_stats();

    let (front_col, front_row) = get_facing_cell(player_state);
    let (front_col, front_row) = (i64::from(front_col), i64::from(front_row));

    let mut cells = vec![(front_col, front_row)];
    if subtype == Some(ItemSubtype::Spear) {
        let (dcol, drow) = player_state.facing.delta();
        cells.push((front_col + i64::from(dcol), front_row + i64::from(drow)));
    }

    let mut results = Vec::new();
    let mut hit_anything = false;

    for (col, row) in cells {
        let Some((enemy_def, enemy_type, enemy_drops)) = game_state
            .get_enemy(col, row)
            .map(|enemy| (enemy.def, enemy.enemy_type.clone(), enemy.drops.clone()))
        else {
            continue;
        };

        hit_anything = true;
        let (damage, _is_crit) =
            resolve_weapon_effect(subtype, stats.atk, enemy_def, stats.crit_chance, random);
        let killed = game_state.damage_enemy(col, row, damage);
        results.push(CombatResult {
            result_type: if killed {
                CombatResultType::Kill
            } else {
                CombatResultType::Hit
            },
            damage: Some(damage),
            target_col: Some(col),
            target_row: Some(row),
            enemy_type: Some(enemy_type),
            drops_override: if killed { enemy_drops } else { None },
        });
    }

    game_state.player.attack_cooldown = cooldown;

    if !hit_anything {
        if let Some(wall_drops) = game_state
            .get_breakable_wall(front_col, front_row)
            .map(|wall| wall.drops.clone())
        {
            let (damage, _) =
                resolve_weapon_effect(subtype, stats.atk, 0.0, stats.crit_chance, random);
            return vec![CombatResult {
                result_type: CombatResultType::WallHit,
                damage: Some(damage),
                target_col: Some(front_col),
                target_row: Some(front_row),
                enemy_type: None,
                drops_override: wall_drops,
            }];
        }
        if game_state.get_barrel(front_col, front_row).is_some() {
            let (damage, _) =
                resolve_weapon_effect(subtype, stats.atk, 0.0, stats.crit_chance, random);
            let outcome = game_state.damage_barrel(front_col, front_row, damage);
            return vec![CombatResult {
                result_type: if outcome.destroyed {
                    CombatResultType::BarrelDestroy
                } else {
                    CombatResultType::BarrelHit
                },
                damage: Some(damage),
                target_col: Some(front_col),
                target_row: Some(front_row),
                enemy_type: None,
                drops_override: outcome.drops,
            }];
        }
        return vec![CombatResult::of(CombatResultType::NoTarget)];
    }

    results
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnemyAttackResult {
    pub damage: f64,
}

/// Enemy attacks the player. Called when enemy AI emits an attack action.
pub fn enemy_attack_player(
    game_state: &mut GameState,
    enemy_atk: f64,
    random: &mut dyn FnMut() -> f64,
) -> EnemyAttackResult {
    let damage = calculate_damage(enemy_atk, game_state.get_effective_def(), random);
    game_state.player.hp = (game_state.player.hp - damage).max(0.0);
    EnemyAttackResult { damage }
}

//! Player character sheet: stats, XP/levels, gold, flags, and picked-up keys.
//! Equip/pickup flows that touch the entity registry and the item database
//! live on `GameState`, which owns all three.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const LEVEL_CAP: i64 = 15;

#[derive(Debug, Clone, PartialEq)]
pub struct InventoryState {
    /// Picked-up key ids.
    pub inventory: HashSet<String>,
    pub flags: HashSet<String>,
    pub hp: f64,
    pub max_hp: f64,
    pub atk: f64,
    pub def: f64,
    pub attack_cooldown: f64,
    pub str: f64,
    pub dex: f64,
    pub vit: f64,
    pub wis: f64,
    pub xp: i64,
    pub level: i64,
    pub attribute_points: i64,
    pub player_name: String,
    pub gold: i64,
}

impl Default for InventoryState {
    fn default() -> Self {
        let vit = 5.0;
        let max_hp = 40.0 + vit * 5.0;
        Self {
            inventory: HashSet::new(),
            flags: HashSet::new(),
            str: 5.0,
            dex: 5.0,
            vit,
            wis: 5.0,
            xp: 0,
            level: 1,
            attribute_points: 0,
            player_name: "Adventurer".to_string(),
            gold: 0,
            atk: 3.0,
            def: 1.0,
            attack_cooldown: 0.0,
            max_hp,
            hp: max_hp,
        }
    }
}

/// Stat a player can allocate attribute points into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AllocatableStat {
    Str,
    Dex,
    Vit,
    Wis,
}

impl InventoryState {
    pub fn add_key(&mut self, key_id: &str) {
        self.inventory.insert(key_id.to_string());
    }

    #[must_use]
    pub fn has_key(&self, key_id: &str) -> bool {
        self.inventory.contains(key_id)
    }

    #[must_use]
    pub fn picked_up_keys(&self) -> Vec<String> {
        self.inventory.iter().cloned().collect()
    }

    pub fn restore_picked_up_keys(&mut self, keys: &[String]) {
        self.inventory = keys.iter().cloned().collect();
    }

    /// Total XP required to reach level `n + 1` from the start.
    #[must_use]
    pub fn xp_for_level(&self, n: i64) -> i64 {
        100 * n * (n + 1) / 2
    }
}

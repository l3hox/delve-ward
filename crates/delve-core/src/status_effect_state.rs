//! Player-scoped status effect state: active effects, temp buffs, torch fuel,
//! and hunger.

use crate::status_effects::StatusEffect;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuffStat {
    Atk,
    Def,
    Str,
    Dex,
    Vit,
    Wis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TempBuff {
    pub stat: BuffStat,
    pub amount: f64,
    pub remaining: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusEffectState {
    pub player_status_effects: Vec<StatusEffect>,
    pub temp_buffs: Vec<TempBuff>,
    pub torch_fuel: f64,
    pub max_torch_fuel: f64,
    pub hunger: f64,
    pub max_hunger: f64,
}

impl Default for StatusEffectState {
    fn default() -> Self {
        Self {
            player_status_effects: Vec::new(),
            temp_buffs: Vec::new(),
            torch_fuel: 200.0,
            max_torch_fuel: 200.0,
            hunger: 100.0,
            max_hunger: 100.0,
        }
    }
}

impl StatusEffectState {
    pub fn drain_torch_fuel(&mut self, amount: f64) {
        self.torch_fuel = (self.torch_fuel - amount).max(0.0);
    }

    pub fn drain_hunger(&mut self, amount: f64) {
        self.hunger = (self.hunger - amount).max(0.0);
    }

    pub fn restore_hunger(&mut self, amount: f64) {
        self.hunger = (self.hunger + amount).min(self.max_hunger);
    }

    /// Same-stat buffs replace instead of stacking.
    pub fn add_temp_buff(&mut self, stat: BuffStat, amount: f64, duration: f64) {
        self.temp_buffs.retain(|buff| buff.stat != stat);
        self.temp_buffs.push(TempBuff {
            stat,
            amount,
            remaining: duration,
        });
    }

    pub fn tick_temp_buffs(&mut self, delta: f64) {
        for buff in &mut self.temp_buffs {
            buff.remaining -= delta;
        }
        self.temp_buffs.retain(|buff| buff.remaining > 0.0);
    }

    #[must_use]
    pub fn temp_buff_total(&self, stat: BuffStat) -> f64 {
        self.temp_buffs
            .iter()
            .filter(|buff| buff.stat == stat)
            .map(|buff| buff.amount)
            .sum()
    }
}

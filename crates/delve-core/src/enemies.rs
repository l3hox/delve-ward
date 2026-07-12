//! Enemy database — typed model of `data/enemies.json` and query methods.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_SPRITE_SIZE: f64 = 1.2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnemySpriteData {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_offset: Option<f64>,
}

/// Behavior parameters are behavior-specific and consumed by the AI systems.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnemyBehavior {
    #[serde(rename = "type")]
    pub behavior_type: String,
    pub params: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnemyDef {
    pub id: String,
    pub name: String,
    pub max_hp: f64,
    pub atk: f64,
    pub def: f64,
    pub aggro_range: f64,
    pub move_interval: f64,
    pub blocks_movement: bool,
    /// Can move over cells with no floor (holes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fly: Option<bool>,
    pub xp: f64,
    pub sprite: EnemySpriteData,
    pub behaviors: Vec<EnemyBehavior>,
}

#[derive(Deserialize)]
struct EnemiesJsonPayload {
    #[allow(dead_code)]
    version: String,
    enemies: Vec<EnemyDef>,
}

#[derive(Debug)]
pub struct EnemyDatabase {
    enemies: Vec<EnemyDef>,
    index: HashMap<String, usize>,
}

impl EnemyDatabase {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let payload: EnemiesJsonPayload = serde_json::from_str(json)
            .map_err(|error| format!("Failed to load enemy database: {error}"))?;
        let mut enemies: Vec<EnemyDef> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for enemy in payload.enemies {
            if let Some(&position) = index.get(&enemy.id) {
                enemies[position] = enemy;
            } else {
                index.insert(enemy.id.clone(), enemies.len());
                enemies.push(enemy);
            }
        }
        Ok(Self { enemies, index })
    }

    #[must_use]
    pub fn get_enemy(&self, id: &str) -> Option<&EnemyDef> {
        self.index.get(id).map(|&position| &self.enemies[position])
    }

    #[must_use]
    pub fn all_enemies(&self) -> &[EnemyDef] {
        &self.enemies
    }

    #[must_use]
    pub fn all_enemy_ids(&self) -> HashSet<String> {
        self.enemies.iter().map(|enemy| enemy.id.clone()).collect()
    }

    #[must_use]
    pub fn has_behavior(&self, id: &str, behavior_type: &str) -> bool {
        self.get_behavior(id, behavior_type).is_some()
    }

    #[must_use]
    pub fn get_behavior(&self, id: &str, behavior_type: &str) -> Option<&EnemyBehavior> {
        self.get_enemy(id)?
            .behaviors
            .iter()
            .find(|behavior| behavior.behavior_type == behavior_type)
    }

    /// Instantiate a fresh enemy at a cell from its definition.
    pub fn create_enemy_instance(
        &self,
        col: i64,
        row: i64,
        enemy_type: &str,
    ) -> Result<crate::types::EnemyInstance, String> {
        let def = self
            .get_enemy(enemy_type)
            .ok_or_else(|| format!("Unknown enemy type: {enemy_type}"))?;
        let has_regen = self.has_behavior(enemy_type, "regen");
        Ok(crate::types::EnemyInstance {
            col,
            row,
            enemy_type: def.id.clone(),
            hp: def.max_hp,
            max_hp: def.max_hp,
            atk: def.atk,
            def: def.def,
            aggro_range: def.aggro_range,
            move_interval: def.move_interval,
            blocks_movement: def.blocks_movement,
            ai_state: crate::types::EnemyAiState::Idle,
            move_timer: 0.0,
            regen_timer: has_regen.then_some(0.0),
            regen_pause_timer: has_regen.then_some(0.0),
            drops: None,
            status_effects: Vec::new(),
            spawner_id: None,
        })
    }
}

impl crate::game_state::EnemyRegistrar for EnemyDatabase {
    fn has_enemy(&self, enemy_type: &str) -> bool {
        self.get_enemy(enemy_type).is_some()
    }

    fn create_enemy(
        &self,
        col: i64,
        row: i64,
        enemy_type: &str,
    ) -> Option<crate::types::EnemyInstance> {
        self.create_enemy_instance(col, row, enemy_type).ok()
    }

    fn regen_pause_duration(&self, enemy_type: &str) -> Option<f64> {
        self.get_behavior(enemy_type, "regen")
            .and_then(|behavior| behavior.params.get("pauseOnDamage"))
            .and_then(serde_json::Value::as_f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENEMIES_JSON: &str = include_str!("../../../assets/data/enemies.json");

    fn database() -> EnemyDatabase {
        EnemyDatabase::from_json(ENEMIES_JSON).expect("shipped enemies.json parses")
    }

    #[test]
    fn rat_has_expected_stats() {
        let db = database();
        let rat = db.get_enemy("rat").expect("rat exists");
        assert_eq!(rat.name, "Rat");
        assert_eq!(rat.max_hp, 8.0);
        assert_eq!(rat.atk, 2.0);
        assert_eq!(rat.def, 0.0);
        assert_eq!(rat.aggro_range, 3.0);
        assert_eq!(rat.move_interval, 0.6);
        assert!(rat.blocks_movement);
        assert_eq!(rat.xp, 10.0);
        assert_eq!(rat.sprite.path, "/sprites/rat.png");
    }

    #[test]
    fn unknown_enemy_returns_none() {
        assert!(database().get_enemy("dragon_emperor").is_none());
    }

    #[test]
    fn behavior_queries_find_declared_behaviors() {
        let db = database();
        let with_behavior = db
            .all_enemies()
            .iter()
            .find(|enemy| !enemy.behaviors.is_empty())
            .expect("at least one enemy declares behaviors");
        let behavior_type = with_behavior.behaviors[0].behavior_type.clone();
        assert!(db.has_behavior(&with_behavior.id, &behavior_type));
        assert!(db.get_behavior(&with_behavior.id, &behavior_type).is_some());
        assert!(!db.has_behavior(&with_behavior.id, "no_such_behavior"));
        assert!(!db.has_behavior("no_such_enemy", &behavior_type));
    }

    #[test]
    fn load_failure_reports_error() {
        let error = EnemyDatabase::from_json("[]").expect_err("wrong shape fails");
        assert!(error.contains("Failed to load enemy database"));
    }
}

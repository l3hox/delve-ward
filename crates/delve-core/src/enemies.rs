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

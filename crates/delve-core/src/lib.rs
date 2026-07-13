#![forbid(unsafe_code)]

//! Pure game logic for DelveWard: level model, grid, game state, signals,
//! combat, quests, save data. Must never depend on Bevy or any rendering
//! or windowing crate.

pub mod boulders;
pub mod combat;
pub mod dialog_manager;
pub mod dialogs;
pub mod enemies;
pub mod enemy_ai;
pub mod entities;
pub mod env_zones;
pub mod forest_placement;
pub mod game_state;
pub mod grid;
pub mod interaction;
pub mod inventory_state;
pub mod items;
pub mod level_loader;
pub mod loot;
pub mod npcs;
pub mod pathfinding;
pub mod player_controller;
pub mod projectiles;
pub mod quest_manager;
pub mod quests;
pub mod random;
pub mod save_system;
pub mod signal_manager;
pub mod spawners;
pub mod status_effect_state;
pub mod status_effects;
pub mod texture_names;
pub mod texture_resolver;
pub mod types;

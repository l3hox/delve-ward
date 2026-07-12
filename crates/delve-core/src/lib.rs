#![forbid(unsafe_code)]

//! Pure game logic for DelveWard: level model, grid, game state, signals,
//! combat, quests, save data. Must never depend on Bevy or any rendering
//! or windowing crate.

pub mod dialogs;
pub mod enemies;
pub mod entities;
pub mod grid;
pub mod items;
pub mod level_loader;
pub mod loot;
pub mod npcs;
pub mod quests;
pub mod random;
pub mod status_effects;
pub mod texture_names;
pub mod texture_resolver;
pub mod types;

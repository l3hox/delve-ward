//! Decorative prop rendering (pillars, rubble, stalactites, statues,
//! crates, banners) — static per-layer geometry with no runtime visual
//! state, ported from `rendering/propRenderer.ts`. TS's `meshMap` has no
//! consumer outside scene building, so no handle map leaves this module.

use crate::dungeon::LayerSpawn;
use bevy::prelude::*;
use delve_core::game_state::LayerState;

pub fn spawn_props(
    _commands: &mut Commands,
    _meshes: &mut Assets<Mesh>,
    _materials: &mut Assets<StandardMaterial>,
    _layer_state: &LayerState,
    _layer_spawn: &LayerSpawn,
) {
}

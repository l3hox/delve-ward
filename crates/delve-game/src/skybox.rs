//! Skybox rendering: an inverted sphere (radius 180, inside the camera's
//! `far: 200.0`) wrapping a procedurally-drawn 2D texture, ported from
//! `rendering/skybox.ts` — deliberately NOT Bevy's cubemap `Skybox`
//! component (see PHASE5-PLAN.md §2). Texture drawing is seeded with
//! `mulberry32` keyed by variant name per decision D10, replacing TS's
//! unseeded `Math.random()`.

use bevy::prelude::*;
use delve_core::types::DungeonLevel;

pub fn spawn_skybox(
    _commands: &mut Commands,
    _meshes: &mut Assets<Mesh>,
    _images: &mut Assets<Image>,
    _materials: &mut Assets<StandardMaterial>,
    _level: &DungeonLevel,
) {
}

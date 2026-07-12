//! Environment presets — fog, background, and ambient light per level.
//! Fog distances match the TS presets; light intensities are visual
//! approximations of the Three.js values in Bevy's physical units, to be
//! re-tuned in the phase 6 side-by-side parity audit.

use bevy::prelude::*;
use delve_core::types::Environment;

/// Scales the TS ambient colors (used at Three.js intensity 1) into cd/m².
pub const AMBIENT_BRIGHTNESS: f32 = 900.0;

pub struct EnvironmentConfig {
    pub fog_color: Color,
    pub fog_near: f32,
    pub fog_far: f32,
    pub ambient_color: Color,
}

#[must_use]
pub fn environment_config(environment: Environment) -> EnvironmentConfig {
    match environment {
        Environment::Dungeon => EnvironmentConfig {
            fog_color: Color::srgb_u8(0x00, 0x00, 0x00),
            fog_near: 6.0,
            fog_far: 26.0,
            ambient_color: Color::srgb_u8(0x1a, 0x1a, 0x22),
        },
        Environment::Mist => EnvironmentConfig {
            fog_color: Color::srgb_u8(0x7a, 0x8a, 0x8f),
            fog_near: 2.0,
            fog_far: 14.0,
            ambient_color: Color::srgb_u8(0x88, 0x99, 0xaa),
        },
        Environment::Forest => EnvironmentConfig {
            fog_color: Color::srgb_u8(0x1a, 0x2e, 0x1a),
            fog_near: 4.0,
            fog_far: 20.0,
            ambient_color: Color::srgb_u8(0x3a, 0x55, 0x30),
        },
        Environment::Outdoor => EnvironmentConfig {
            fog_color: Color::srgb_u8(0x88, 0xaa, 0xcc),
            fog_near: 20.0,
            fog_far: 80.0,
            ambient_color: Color::srgb_u8(0xbb, 0xcc, 0xee),
        },
    }
}

//! Stock Bevy window with no game code, for isolating the macOS activation
//! bug (planning/ISSUE-macos-focus.md). If this window also cannot be
//! foregrounded, the bug is in Bevy/winit/macOS, not in delve-game.
//!
//! Run: cargo run -p delve-game --example window_baseline
//! Then click the window and press any key; watch the log lines.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use bevy::window::WindowFocused;

fn log_input(
    mut keyboard: MessageReader<KeyboardInput>,
    mut focus_events: MessageReader<WindowFocused>,
) {
    for event in keyboard.read() {
        info!("key: {:?} {:?}", event.key_code, event.state);
    }
    for event in focus_events.read() {
        info!("window focus (os): {}", event.focused);
    }
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Update, log_input)
        .run();
}

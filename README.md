# DelveWard-rust

A Rust port of [DelveWard](../DelveWard), a grid-based first-person dungeon crawler in the Legend of Grimrock style: step movement, 90-degree turns, pixelart textures, torchlit corridors.

The original is TypeScript + Three.js. This port rebuilds the game runtime on [Bevy](https://bevy.org) for native desktop, reading the same level, item, enemy, dialog, and quest JSON as the original. The dungeon editor is not ported; content is authored in the original's web editor.

## Status

Seed stage. The workspace compiles and the port plan lives in `planning/PORT-PLAN.md`.

## Running

On macOS, build and launch the app bundle (macOS 26 refuses to activate bare
terminal binaries — the window would render but never accept keyboard focus):

```sh
scripts/bundle-macos.sh
open target/DelveWard.app
```

Elsewhere:

```sh
cargo run
```

## Layout

- `crates/delve-core`: pure game logic, no engine dependency
- `crates/delve-game`: Bevy application (rendering, input, HUD)
- `assets/`: sprites, levels, and data snapshotted from the original
- `planning/`: port plan, decisions, progress state

## License

EUPL-1.2, same as the original. See `LICENSE`.

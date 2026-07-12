# Install DelveWard-rust

> Build and verify the DelveWard-rust workspace from a fresh checkout.

This guide gets the Rust port building and its tests passing. It assumes nothing beyond a POSIX shell and network access.

**OBJECTIVE**: A compiled workspace with all tests green and a runnable game binary.

**DONE WHEN**: `cargo test --workspace` reports all tests passing and `cargo run` opens a window titled "DelveWard".

## TODO

- [ ] Install the Rust toolchain via rustup
- [ ] Build the workspace
- [ ] Run the test suite
- [ ] Launch the game window

## Steps

1. Install rustup if missing (the pinned toolchain in `rust-toolchain.toml` installs automatically on first cargo use):

    ```sh
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    ```

2. Build the workspace (the first Bevy build takes several minutes):

    ```sh
    cargo build --workspace
    ```

3. Run the tests:

    ```sh
    cargo test --workspace
    ```

4. Launch the game:

    ```sh
    cargo run
    ```

EXECUTE NOW: run the TODO steps in order and confirm the DONE WHEN condition.

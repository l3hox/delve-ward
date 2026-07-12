# ISSUE: macOS keyboard focus / window activation

Status: RESOLVED 2026-07-12. macOS 26 (Tahoe) denies activation to bare
binaries launched from a terminal; wrapping the binary in a minimal `.app`
bundle and launching via `open` fixes it. Supported macOS launch path:

```sh
scripts/bundle-macos.sh
open target/DelveWard.app
```

Confirmed on the user's machine: window activates, keys work, movement fine.
`cargo run` still shows the window but cannot be focused on macOS 26; use it
only for launch/init smoke tests. The launch-grace focus re-assert
(`claim_initial_focus` in `main.rs`) stays — it is harmless, self-settling,
and covers terminal launches on older macOS versions.

The original diagnosis follows, kept for reference.

## Symptom (user machine, macOS 26.5 Tahoe, Darwin 25.5)

- `cargo run` shows the window and graphics render correctly.
- Every keypress produces the macOS alert sound ("blip"); no control reacts.
- The window CANNOT be brought to the foreground at all — clicking it does
  not activate the app. This is the key clue: the process is never becoming
  the active application, so its window can never become key and all key
  events fall through the responder chain (hence the beeps).

## What has been tried

1. `claim_initial_focus` system in `crates/delve-game/src/main.rs`: during a
   2s launch grace period, re-asserts `Window::focused = true` on a rising
   edge, which makes bevy_winit call winit `focus_window()` →
   `NSApplication.activateIgnoringOtherApps(true)` + `makeKeyAndOrderFront`.
   Verified (via log) that the retry loop fires on schedule. Did NOT fix the
   user's symptom — consistent with activation being denied at the OS level,
   not merely missed at launch.
2. Verified versions: winit 0.30.13 (latest 0.30.x as of 2026-03), bevy_winit
   0.19.0. winit defaults on macOS are `ActivationPolicy::Regular` +
   `activate_ignoring_other_apps = true`, applied in
   `applicationDidFinishLaunching` (checked in the vendored winit source).
3. Diagnostics in place: `input_diagnostics` logs `window focus (os): <bool>`
   at info level (OS-confirmed, from `WindowFocused` events) and every raw
   key event at `RUST_LOG=delve_game=debug`.

## What could not be verified here

Agent/sandbox processes cannot acquire focus themselves and synthetic
keystrokes are TCC-blocked, so interactive confirmation must come from the
user's terminal.

## Ready-to-run tests (need the user's interactive terminal)

Both are prepared in the repo; run them in a normal terminal, not through
an agent sandbox.

1. Bundle test (most likely durable fix):

   ```sh
   scripts/bundle-macos.sh
   open target/DelveWard.app
   ```

   If the window activates and keys work → bundling is the fix; document
   `open target/DelveWard.app` as the supported macOS launch path.

2. Baseline test (isolates app code vs Bevy/winit/OS):

   ```sh
   cargo run -p delve-game --example window_baseline
   ```

   A stock Bevy window with no game code, logging key and focus events.
   If it has the same cannot-foreground symptom → upstream Tahoe issue;
   search/file at rust-windowing/winit and bevyengine/bevy ("Tahoe
   activation", "cannot focus window macOS 26").

## Next steps after the tests

1. Check the environment for known Tahoe focus-bug triggers reported in
   Apple Community threads: Logitech G Hub, window managers (Rectangle,
   AeroSpace, AltTab), MDM profiles. Toggling G Hub reportedly resolves the
   system-wide "phantom focus" bug on macOS 26.
2. If unbundled activation is fundamentally broken on Tahoe, file a
   `bevy_winit` issue with the `window_baseline` repro.

## Related Apple Community / upstream references

- macOS "Phantom Focus" bug: https://discussions.apple.com/thread/256232766
- Windows constantly lose focus on Tahoe: https://discussions.apple.com/thread/256140830
- Window and App issues with Tahoe: https://discussions.apple.com/thread/256162304
- NSBeep responder-chain mechanics: https://christiantietze.de/posts/2016/11/nsresponder-finding-beep/

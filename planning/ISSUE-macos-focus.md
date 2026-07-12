# ISSUE: macOS keyboard focus / window activation

Status: OPEN. Read this before touching input or window code.

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

## Next steps, in order

1. Baseline: does a stock winit/Bevy example have the same problem on this
   machine? `cargo new` + winit window example, or any Bevy example. If YES →
   upstream Tahoe issue; search/file at rust-windowing/winit and
   bevyengine/bevy ("Tahoe activation", "cannot focus window macOS 26").
2. Check the user's environment for known Tahoe focus-bug triggers reported
   in Apple Community threads: Logitech G Hub, window managers (Rectangle,
   AeroSpace, AltTab), MDM profiles. Toggling G Hub reportedly resolves the
   system-wide "phantom focus" bug on macOS 26.
3. App bundle test: macOS treats bundled apps more reliably for activation.
   Create a minimal `DelveWard.app/Contents/{Info.plist,MacOS/delve-game}`
   wrapper around the built binary and launch via `open DelveWard.app`. If
   bundling fixes activation, add a `make bundle` target / small script and
   document `open` as the supported launch path on macOS.
4. If unbundled activation is fundamentally broken on Tahoe, consider a
   `bevy_winit` issue report with the minimal repro from step 1.

## Related Apple Community / upstream references

- macOS "Phantom Focus" bug: https://discussions.apple.com/thread/256232766
- Windows constantly lose focus on Tahoe: https://discussions.apple.com/thread/256140830
- Window and App issues with Tahoe: https://discussions.apple.com/thread/256162304
- NSBeep responder-chain mechanics: https://christiantietze.de/posts/2016/11/nsresponder-finding-beep/

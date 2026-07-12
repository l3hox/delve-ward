#!/bin/sh
# Assemble a minimal macOS .app bundle around the delve-game binary.
# Diagnostic for planning/ISSUE-macos-focus.md: macOS activates bundled
# apps launched via `open` more reliably than bare terminal binaries.
#
# Usage: scripts/bundle-macos.sh [--release]
# Then:  open target/DelveWard.app
#
# Assets resolve through the compile-time repo path baked into the binary,
# so the bundle only works on the machine that built it.
set -eu

cd "$(dirname "$0")/.."

profile=debug
cargo_flags=""
if [ "${1:-}" = "--release" ]; then
    profile=release
    cargo_flags="--release"
fi

cargo build -p delve-game $cargo_flags

app=target/DelveWard.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp packaging/macos/Info.plist "$app/Contents/Info.plist"
cp "target/$profile/delve-game" "$app/Contents/MacOS/delve-game"

if ! codesign --force --sign - "$app" 2>/dev/null; then
    echo "warning: ad-hoc codesign failed; continuing unsigned" >&2
fi

echo "built $app"
echo "launch with: open $app"

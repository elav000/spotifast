#!/usr/bin/env bash
# Build, install, and launch the regular local macOS app.
# Run from anywhere in the checkout with: packaging/macos/install-local.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
app="/Applications/Spotifast.app"
build=true
launch=true

for arg in "$@"; do
  case "$arg" in
    --no-build) build=false ;;
    --no-launch) launch=false ;;
    *) echo "Usage: $0 [--no-build] [--no-launch]" >&2; exit 2 ;;
  esac
done

if [[ "$(uname -s)" != Darwin ]]; then
  echo "This script installs a macOS app bundle and must run on macOS." >&2
  exit 1
fi

cd "$root"
if pgrep -x Spotifast >/dev/null; then
  echo "Spotifast is running. Quit it, then run this script again to replace the installed app." >&2
  exit 1
fi

if $build; then
  cargo build --locked --release
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
if [[ -z "$version" ]]; then
  echo "Could not read the app version from Cargo.toml." >&2
  exit 1
fi

packaging/macos/bundle.sh target/release/spotifast "$app" "$version"
if $launch; then
  open -a "$app"
fi

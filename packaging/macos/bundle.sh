#!/bin/bash
# Build the product app bundle from a GUI binary, on a macOS machine.
#
#   packaging/macos/bundle.sh <binary> <output.app> <version>
#
# Set CODESIGN_IDENTITY to sign with a Developer ID; the default is an ad-hoc
# signature, which arm64 requires before the app will launch at all.
#
# The .icns is generated here from the committed 1024px PNG, because iconutil
# only exists on macOS. The Info.plist template lives next to this script.
set -euo pipefail

binary="$1"
app="$2"
version="$3"
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
product_name="$(python3 "$root/packaging/brand.py" name)"
bundle_id="$(python3 "$root/packaging/brand.py" macos_bundle_id)"
slug="$(python3 "$root/packaging/brand.py" slug)"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

executable="$product_name"
identifier="$bundle_id"
cp "$binary" "$app/Contents/MacOS/$executable"
chmod 755 "$app/Contents/MacOS/$executable"
# The build number has to be numbers: a release candidate's -rc1 comes off.
build="${version%%-*}"
sed -e "s/__VERSION__/$version/g" -e "s/__BUILD__/$build/g" \
    -e "s/__PRODUCT_NAME__/$product_name/g" -e "s/__ICON_NAME__/$slug/g" \
    -e "s/__EXECUTABLE__/$executable/g" -e "s/__IDENTIFIER__/$identifier/g" "$here/Info.plist" \
    > "$app/Contents/Info.plist"
# Starting in the tray at login: a login item cannot pass --start-hidden, so
# SMAppService registers this agent from the bundle instead (#122).
mkdir -p "$app/Contents/Library/LaunchAgents"
cp "$here/autostart.plist" "$app/Contents/Library/LaunchAgents/$identifier.autostart.plist"
sed -i '' -e "s/__IDENTIFIER__/$identifier/g" "$app/Contents/Library/LaunchAgents/$identifier.autostart.plist"

iconset="$(mktemp -d)/$slug.iconset"
mkdir -p "$iconset"
# iconutil reads only these base sizes, each with an optional @2x. It ignores
# an icon_64x64 without saying so, so generating one is two wasted sips calls.
for size in 16 32 128 256 512; do
    sips -z $size $size "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/$slug.icns"

# arm64 refuses to launch an unsigned bundle, so sign one way or another.
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --options runtime \
        --sign "$CODESIGN_IDENTITY" "$app"
else
    codesign --force --sign - "$app"
fi
codesign --verify --strict "$app"

echo "$app"

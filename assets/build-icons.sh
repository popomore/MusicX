#!/bin/sh
# Rebuild the checked-in macOS icon resources from their vector originals.
set -eu
cd "$(dirname "$0")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/musicx.iconset"
for size in 16 32 128 256 512; do
    sips -s format png -z "$size" "$size" musicx.svg \
        --out "$work/musicx.iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -s format png -z "$double" "$double" musicx.svg \
        --out "$work/musicx.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns "$work/musicx.iconset" --output musicx.icns
sed '/id="playing-indicator"/d' musicx-mark.svg > "$work/musicx-mark-idle.svg"
sips -s format png "$work/musicx-mark-idle.svg" --out musicx-menu.png >/dev/null
sips -s format png musicx-mark.svg --out musicx-menu-playing.png >/dev/null
sips -s format png -z 256 256 musicx.svg --out musicx-now-playing.png >/dev/null

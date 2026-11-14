#!/bin/sh
# Regenerates Solilog.icns from ../icon.svg. Needs rsvg-convert (brew install librsvg).
# The .icns is committed, so building the app doesn't need this.
set -eu
cd "$(dirname "$0")"
iconset="$(mktemp -d)/Solilog.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" ../icon.svg -o "$iconset/icon_${size}x${size}.png"
    rsvg-convert -w $((size * 2)) -h $((size * 2)) ../icon.svg -o "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o Solilog.icns
rm -r "$(dirname "$iconset")"
echo "wrote packaging/macos/Solilog.icns"

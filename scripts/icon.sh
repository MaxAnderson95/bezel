#!/bin/sh
# Renders assets/bezel.svg into assets/Bezel.icns, which scripts/bundle.sh puts in Bezel.app.
# Run after changing the SVG; needs ImageMagick (`brew install imagemagick`) and macOS's iconutil.
set -eu
cd "$(dirname "$0")/.."
work=$(mktemp -d "${TMPDIR:-/tmp}/bezel-icon.XXXXXX")
trap 'rm -rf "$work"' EXIT
set=$work/Bezel.iconset
mkdir "$set"
for size in 16 32 128 256 512; do
  magick -background none -density 72 assets/bezel.svg -resize "${size}x${size}" "PNG32:$set/icon_${size}x${size}.png"
  double=$((size * 2))
  magick -background none -density 72 assets/bezel.svg -resize "${double}x${double}" "PNG32:$set/icon_${size}x${size}@2x.png"
done
iconutil --convert icns --output assets/Bezel.icns "$set"
echo assets/Bezel.icns

#!/usr/bin/env bash
# Renders every mockup screen in both themes to renders/<screen>-<theme>.png at 2x.
set -euo pipefail
cd "$(dirname "$0")"
chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
mkdir -p renders
screens=("${@:-a b b2 c d dk creds cert}")
for s in ${screens[@]}; do
  for t in dark light; do
    "$chrome" --headless=new --disable-gpu --hide-scrollbars --allow-file-access-from-files \
      --force-device-scale-factor=2 --window-size=1280,820 --virtual-time-budget=3000 \
      --screenshot="$PWD/renders/$s-$t.png" "file://$PWD/bezel.html?v=$s&t=$t&shot=1" 2>/dev/null
  done
done

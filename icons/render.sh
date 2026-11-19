#!/bin/sh
# Renders the launcher icon into res/ for every density; rerun after editing the SVGs.
set -e
cd "$(dirname "$0")"
for d in mdpi:1 hdpi:1.5 xhdpi:2 xxhdpi:3 xxxhdpi:4; do
    dir=../res/mipmap-${d%:*}
    s=${d#*:}
    mkdir -p "$dir"
    rsvg-convert -w "$(echo "48 * $s / 1" | bc)" app_legacy.svg -o "$dir/ic_launcher.png"
    rsvg-convert -w "$(echo "108 * $s / 1" | bc)" app_foreground.svg -o "$dir/ic_launcher_foreground.png"
done

#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# The README and site screenshots (assets/*.png here, public/img/*.png on the site), made from the
# shell goldens (crates/shell/tests/golden) so they always show the current brand and UI:
#   settings.png  settings-appearance-wide-light    app-store.png  store-discover-wide-light
#   setup.png     setup-welcome-1280-light          login.png      login-light
#   lock.png      lock-light                        (copied as they are)
#   desktop.png         the dusk wallpaper at 900x600 + menubar-desktop-light + dock-light
#   control-center.png  desktop.png + controlcenter-desktop-light at 524,32
#   notification.png    desktop.png + banner-action-light at 472,32
#   search.png          desktop.png + searchbar-files-light at 86,56
# Composed pictures: the wallpaper is drawn center/cover as the golden harness draws it; each golden
# window shows the wallpaper behind it at its own size, and that ground is made transparent by
# differencing against the same wallpaper drawn at the golden's size, so only the window is laid on.
# usage: tools/site-shots.sh <out dir>   (from the repository root; needs rsvg-convert and ImageMagick)
set -eu
OUT=${1:?usage: tools/site-shots.sh <out dir>}
G=crates/shell/tests/golden
W=design/wallpapers/wonderland-dusk.svg
mkdir -p "$OUT"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
rsvg-convert -w 1920 -h 1080 "$W" -o "$T/wall.png"

cover() { # <w> <h> <out>: the wallpaper drawn center/cover into w x h
  convert "$T/wall.png" -resize "$1x$2^" -gravity center -extent "$1x$2" "$3"
}
cut() { # <golden> <out>: the golden with its wallpaper ground made transparent
  cover "$(identify -format %w "$1")" "$(identify -format %h "$1")" "$T/ground.png"
  convert "$1" "$T/ground.png" -compose difference -composite -colorspace gray -level 0,6% "$T/mask.png"
  convert "$1" -alpha off "$T/mask.png" -compose copy-opacity -composite "$2"
}
png() { # <in> <out>: 8-bit RGB, no metadata (no date chunks, so a rerun gives the same bytes)
  convert "$1" -strip -define png:exclude-chunks=date,time -define png:color-type=2 "$2"
}

cover 900 600 "$T/base.png"
cut "$G/menubar-desktop-light.png" "$T/bar.png"
cut "$G/dock-light.png" "$T/dock.png"
convert "$T/base.png" "$T/bar.png" -geometry +0+0 -composite "$T/dock.png" -geometry -30+480 -composite "$T/desktop.png"
png "$T/desktop.png" "$OUT/desktop.png"

for shot in control-center:controlcenter-desktop:524:32 notification:banner-action:472:32 search:searchbar-files:86:56; do
  IFS=: read -r name golden x y <<END
$shot
END
  cut "$G/$golden-light.png" "$T/win.png"
  convert "$T/desktop.png" "$T/win.png" -geometry "+$x+$y" -composite "$T/shot.png"
  png "$T/shot.png" "$OUT/$name.png"
done

cp "$G/settings-appearance-wide-light.png" "$OUT/settings.png"
cp "$G/store-discover-wide-light.png" "$OUT/app-store.png"
cp "$G/setup-welcome-1280-light.png" "$OUT/setup.png"
cp "$G/login-light.png" "$OUT/login.png"
cp "$G/lock-light.png" "$OUT/lock.png"

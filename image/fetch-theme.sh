#!/bin/sh
# Install Inter (type system) and adw-gtk3 (GTK3 theme) into STAGE. Neither is in FreeBSD pkg (15.1 quarterly).
# Usage: fetch-theme.sh STAGE   Env: WOLOS_WORK (download cache, /work). Pinned by sha256.
set -eu
STAGE=$1; C=${WOLOS_WORK:-/work}/dl; mkdir -p "$C"
get() { # url sha256
  f=$C/${1##*/}
  [ -f "$f" ] || fetch -o "$f" "$1"
  [ "$(sha256 -q "$f")" = "$2" ] || { echo "fetch-theme.sh: checksum mismatch $f" >&2; rm -f "$f"; exit 1; }
  echo "$f"
}
inter=$(get https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip 9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e)
adw=$(get https://github.com/lassekongo83/adw-gtk3/releases/download/v6.5/adw-gtk3v6.5.tar.xz a81780fadfc432be0fc3d89c4ebb41aa28e4f032d42c36f9789c57dd10cfa41c)
F=$STAGE/usr/local/share/fonts/Inter; T=$STAGE/usr/local/share/themes
mkdir -p "$F" "$T"
tar -xf "$inter" -C "$F" InterVariable.ttf InterVariable-Italic.ttf
tar -xOf "$inter" LICENSE.txt > "$F/LICENSE.txt"
rm -rf "$T/adw-gtk3" "$T/adw-gtk3-dark"; tar -xJf "$adw" -C "$T"
[ "$STAGE" != / ] || { fc-cache -f "$F" >/dev/null 2>&1 || :; }
echo "theme: Inter 4.1 -> $F, adw-gtk3 6.5 -> $T"

#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Install the system web apps (ADR 0048) into DEST: the kit and every app listed in webapps/SHIPPED
# under DEST/usr/share/wolos/webapps/, without the development files (mock-bridge.js, fixtures.js,
# an app's fixtures/ and test/ folders), and each app's launcher entry (webapps/launchers/, generated
# by crates/webengine/tests/launchers.rs; not under files/, which wolos-system installs whole) under
# DEST/usr/share/applications/.
# Usage: tools/install-webapps.sh DEST   (run by the wolos-webapps PKGBUILD; needs only POSIX tools)
set -eu
DEST=${1:?DEST}
REPO=$(cd "$(dirname "$0")/.." && pwd)
SYS=$DEST/usr/share/wolos/webapps
APPS=$DEST/usr/share/applications
install -d "$SYS" "$APPS"

LIST=$(mktemp)
trap 'rm -f "$LIST" "$LIST.raw"' EXIT

# copy_folder SRC DST: every file of SRC but the development ones, keeping the folder layout. The
# list goes through a file, not a pipe: a failing `find` or `install` stops the script.
copy_folder() {
  [ -d "$1" ] || { echo "install-webapps.sh: $1 is not a folder" >&2; exit 1; }
  (cd "$1" && find . -type f ! -name mock-bridge.js ! -name fixtures.js ! -path './fixtures/*' ! -path './test/*') >"$LIST.raw" ||
    { echo "install-webapps.sh: cannot list $1" >&2; exit 1; }
  sort "$LIST.raw" >"$LIST" || exit 1
  [ -s "$LIST" ] || { echo "install-webapps.sh: $1 has no file to install" >&2; exit 1; }
  install -d "$2"
  while IFS= read -r f; do
    install -Dm644 "$1/$f" "$2/$f"
  done <"$LIST"
}

[ -d "$REPO/webapps/kit" ] || { echo "install-webapps.sh: $REPO/webapps/kit is missing" >&2; exit 1; }
copy_folder "$REPO/webapps/kit" "$SYS/kit"
n=0
while IFS= read -r slug; do
  case $slug in '' | '#'*) continue ;; esac
  [ -f "$REPO/webapps/$slug/app.toml" ] || { echo "install-webapps.sh: webapps/$slug has no app.toml" >&2; exit 1; }
  entry=$REPO/webapps/launchers/org.wolos.webapp.$slug.desktop
  [ -f "$entry" ] || { echo "install-webapps.sh: no launcher entry $entry (cargo test -p webengine --test launchers, WOL_REGEN=1)" >&2; exit 1; }
  copy_folder "$REPO/webapps/$slug" "$SYS/$slug"
  install -Dm644 "$entry" "$APPS/org.wolos.webapp.$slug.desktop"
  n=$((n + 1))
done < "$REPO/webapps/SHIPPED"
[ "$n" -gt 0 ] || { echo "install-webapps.sh: webapps/SHIPPED lists no app" >&2; exit 1; }

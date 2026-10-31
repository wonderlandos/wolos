#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Installs the repo guard as pre-commit and pre-push. Usage: tools/hooks/install.sh
set -eu
H=$(cd "$(dirname "$0")" && pwd); C=$(git -C "$H" rev-parse --git-common-dir)
case $C in /*) ;; *) C=$H/$C ;; esac
for n in pre-commit pre-push; do cp "$H/public-guard" "$C/hooks/$n"; chmod +x "$C/hooks/$n"; done
grep -qx '/strategy/' "$C/info/exclude" 2>/dev/null || echo '/strategy/' >> "$C/info/exclude"
echo "hooks: installed ($C/hooks)"

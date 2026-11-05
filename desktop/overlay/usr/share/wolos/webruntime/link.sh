#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Web-app runtime profiles (ADR 0042): point the flag files of the shared-engine apps, the code
# editor and the browser at the user's profile. Called by wol-session: `sh link.sh <config home>
# <share dir> <soft|gpu>`. Profile from <config home>/wolos/web-runtime (balanced, light, full;
# anything else is balanced). Only files that are missing or a link into <share dir> are (re)linked:
# a file of your own, or your own link elsewhere, is never touched.
set -eu
cfg=$1 share=$2 gpu=$3
case $gpu in soft | gpu) ;; *) gpu=soft ;; esac
profile=
if [ -r "$cfg/wolos/web-runtime" ]; then
	read -r profile <"$cfg/wolos/web-runtime" || true # a last line without a newline still counts
fi
case $profile in balanced | light | full) ;; *) profile=balanced ;; esac
rt="$share/webruntime/$profile-$gpu"
# The previous browser's flags file: our link to it points at a file that is gone. Only our own
# link is removed; a file of your own stays.
old="$cfg/helium-flags.conf"
if [ -L "$old" ]; then
	case $(readlink "$old") in "$share"/*) rm -f "$old" ;; esac
fi
for f in electron-flags.conf code-flags.conf atlas-flags.conf; do
	[ -e "$rt/$f" ] || continue
	if [ -L "$cfg/$f" ]; then
		case $(readlink "$cfg/$f") in "$share"/*) ;; *) continue ;; esac
	elif [ -e "$cfg/$f" ]; then
		continue
	fi
	mkdir -p "$cfg" && ln -sfn "$rt/$f" "$cfg/$f"
done

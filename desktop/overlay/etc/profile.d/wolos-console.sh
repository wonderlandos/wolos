# SPDX-License-Identifier: GPL-2.0-or-later
# WolOS console (ADR 0032 "Console"): read by every login shell (/etc/profile), so the whole desktop
# session inherits it (wol-login starts the session through `su -l`). nushell reads no system
# config: its vendor autoload directory is pointed at the WolOS setup. A login bash also reads the
# WolOS bash setup (an interactive non-login bash reads it from ~/.bashrc, see the file's header).
# shellcheck shell=sh
export NU_VENDOR_AUTOLOAD_DIR="${NU_VENDOR_AUTOLOAD_DIR:-/usr/share/wolos/nu/autoload}"
if [ -n "${BASH_VERSION:-}" ] && [ -r /usr/share/wolos/bash/bashrc ]; then
  # shellcheck source=/dev/null
  . /usr/share/wolos/bash/bashrc
fi

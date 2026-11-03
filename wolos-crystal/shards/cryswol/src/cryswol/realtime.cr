# SPDX-License-Identifier: BSD-2-Clause
# Opt-in: the compositor's realtime helpers (GPL-2.0-or-later, a static library inside the
# compositor plugin, no shared object). For code linked into the compositor only; the bindings
# carry no link annotation unless built with `-Dwol_realtime_link`.
require "../wol/realtime/*"

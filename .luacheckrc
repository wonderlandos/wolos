-- SPDX-License-Identifier: GPL-2.0-or-later
-- Lua shell plugins and widgets run inside crates/wol-lua, which hands them one global: `wol`
-- (crates/wol-lua/src/lib.rs, plugin.rs). Nothing else is injected; the sandbox keeps the safe stdlib.
std = "min"
read_globals = { "wol" }
exclude_files = { "target/**", "third_party/**" }

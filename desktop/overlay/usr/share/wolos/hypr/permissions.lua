-- SPDX-License-Identifier: GPL-2.0-or-later
-- WolOS privacy (docs/modules/security.md): who may copy the screen, capture input
-- or load compositor plugins. The Lua form of permissions.conf (wol_core::security::hypr_permissions).
hl.config({
    ecosystem = {
        enforce_permissions = true,
        no_update_news = true,
        no_donation_nag = true,
    },
})
hl.permission({ binary = "^/usr/lib/xdg-desktop-portal-hyprland$", type = "screencopy", mode = "allow" })
hl.permission({ binary = "^/usr/lib/xdg-desktop-portal-hyprland$", type = "cursorpos", mode = "allow" })
hl.permission({ binary = "^/usr/bin/grim$", type = "screencopy", mode = "allow" })
hl.permission({ binary = "^/usr/bin/grim$", type = "cursorpos", mode = "allow" })
hl.permission({ binary = "^/usr/bin/wf-recorder$", type = "screencopy", mode = "allow" })
hl.permission({ binary = "^/usr/bin/wf-recorder$", type = "cursorpos", mode = "allow" })
hl.permission({ binary = "^/usr/bin/wol-window-shot$", type = "screencopy", mode = "allow" })
hl.permission({ binary = "^/usr/bin/wol-window-shot$", type = "cursorpos", mode = "allow" })
hl.permission({ binary = "^/usr/bin/wol-shell$", type = "screencopy", mode = "allow" })
hl.permission({ binary = "^/usr/bin/wol-shell$", type = "cursorpos", mode = "allow" })
hl.permission({ binary = ".*", type = "screencopy", mode = "deny" })
hl.permission({ binary = ".*", type = "cursorpos", mode = "deny" })
hl.permission({ binary = ".*", type = "input-capture", mode = "deny" })
hl.permission({ binary = [[^/usr/lib/hyprland/plugins/[^/]+\.so$]], type = "plugin", mode = "allow" })
hl.permission({ binary = ".*", type = "plugin", mode = "deny" })

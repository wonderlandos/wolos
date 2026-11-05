-- SPDX-License-Identifier: GPL-2.0-or-later
-- Login window compositor (wol-greeter-session). Exiting wol-loginwindow restarts the greeter.
-- Scale 1 like the desktop session (hyprland.lua): without a rule the compositor picks a scale from
-- the display's size, and a VM display at 1.5 left the welcome card taller than the screen.
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        -- No compositor nags: guiutils is not shipped; wol-login restarts this compositor (no start-hyprland).
        disable_hyprland_guiutils_check = true,
        disable_watchdog_warning = true,
        -- Black like the loader splash and the console (ADR 0012).
        background_color = "rgb(000000)",
    },
})
-- No pointer over the black first frames; the window shows it after its own first frame (kit::Reveal).
-- The pointer the desktop uses (adwaita-cursors, a dependency of adwaita-icon-theme).
hl.env("XCURSOR_THEME", "Adwaita")
hl.env("XCURSOR_SIZE", "24")
hl.config({ cursor = { invisible = true } })
-- The first account's keyboard layout (wol-login writes it with the account, QA-ACC-10): a
-- password with ß, ç or @ is typed as at the desktop. No file (no account yet): the US layout.
local keyboard_lua = "/etc/wolos/login-keyboard.lua"
local keyboard_file = io.open(keyboard_lua, "r")
if keyboard_file then
    keyboard_file:close()
    require(keyboard_lua)
end
-- The layout of the account that last used this computer (wol-login writes it when a session
-- starts and ends, QA-A7-16): a layout changed in Settings is what the login window types on.
-- It wins over the install-time file above; two lines, the layouts and the variants.
local last_layout = io.open("/var/lib/wolos/last-layout", "r")
if last_layout then
    local layout, variant = last_layout:read("*l"), last_layout:read("*l")
    last_layout:close()
    if layout and layout:match("^[%w_,]+$") and variant and variant:match("^[%w_,]*$") then
        -- pcall: a value the compositor refuses leaves the layout as it was, not the login window dead.
        local applied = pcall(hl.config, {input={kb_layout=layout, kb_variant=variant}})
    end
end
hl.config({
    decoration = { rounding = 12 },
    animations = { enabled = false },
})
-- Compositor and UI run as _wollogin (wol-greeter-session).
-- VM GPUs (vmwgfx): no pointer on the cursor plane, draw it.
hl.exec_cmd([[sh -c 'systemd-detect-virt -q --vm && hyprctl eval "hl.config({cursor = {no_hardware_cursors = 1}})"']])
hl.on("hyprland.start", function()
    hl.exec_cmd("sh -c 'exec hypridle -c /usr/share/wolos/hypr/login-idle.conf'") -- the display turns off after 10 idle minutes (QA-FILES-10)
    hl.exec_cmd("wol-vbox-guest") -- VirtualBox: resize with the VM window, clipboard, drag and drop (only under VirtualBox)
    hl.exec_cmd([[sh -c '/usr/bin/wol-loginwindow; pkill -x -u "$(id -u)" hypridle; hyprctl dispatch "hl.dsp.dpms({ action = \"off\" })"; hyprctl dispatch "hl.dsp.exit()"']]) -- hypridle first: it aborts when its display goes (QA-LIVE-32); output off: no stale card in the last scanout (QA-SHEET-3)
end)

-- Whole-frame redraws: with damage tracking a page fade's last frame was never repainted (VM).
hl.config({ debug = { damage_tracking = 0 } })

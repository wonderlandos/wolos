-- First screen of the live image (installer-session, ADR 0023): Install WolOS alone, no shell.
-- Scale 1 like the desktop session (hyprland.lua): without a rule the compositor picks a scale from
-- the display's size, and a VM display at 1.5 left the welcome card taller than the screen.
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        disable_hyprland_guiutils_check = true,
        disable_watchdog_warning = true,
        background_color = "rgb(000000)",
    },
})
-- The pointer the desktop uses (adwaita-cursors, a dependency of adwaita-icon-theme).
hl.env("XCURSOR_THEME", "Adwaita")
hl.env("XCURSOR_SIZE", "24")
-- Pointer shown from the start: Install WolOS opens over the live desktop (no kit::Reveal from
-- black), so nothing would ever unhide a `cursor.invisible` here (VirtualBox: no pointer at all).
hl.config({
    decoration = { rounding = 12 },
    animations = { enabled = false },
})
-- VM GPUs (vmwgfx): no pointer on the cursor plane, draw it.
hl.exec_cmd([[sh -c 'systemd-detect-virt -q --vm && hyprctl eval "hl.config({cursor = {no_hardware_cursors = 1}})"']])
hl.on("hyprland.start", function()
    hl.exec_cmd("wol-vbox-guest") -- VirtualBox: resize with the VM window, clipboard, drag and drop (only under VirtualBox)
    hl.exec_cmd([[sh -c '/usr/bin/wol-installer --boot; hyprctl dispatch "hl.dsp.exit()"']])
end)

hl.config({ debug = { damage_tracking = 0 } })

-- Setup assistant compositor (wol-greeter-session setup, ADR 0004). Exiting wol-setup restarts the greeter.
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
hl.config({
    decoration = { rounding = 12 },
    animations = { enabled = false },
})
-- Compositor and UI run as _wollogin (wol-greeter-session).
-- VM GPUs (vmwgfx): no pointer on the cursor plane, draw it.
hl.exec_cmd([[sh -c 'systemd-detect-virt -q --vm && hyprctl eval "hl.config({cursor = {no_hardware_cursors = 1}})"']])
hl.on("hyprland.start", function()
    hl.exec_cmd("wol-vbox-guest") -- VirtualBox: resize with the VM window, clipboard, drag and drop (only under VirtualBox)
    hl.exec_cmd([[sh -c '/usr/bin/wol-setup; hyprctl dispatch "hl.dsp.exit()"']])
end)

-- Whole-frame redraws: with damage tracking a page fade's last frame was never repainted (VM).
hl.config({ debug = { damage_tracking = 0 } })

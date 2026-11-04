-- SPDX-License-Identifier: GPL-2.0-or-later
-- WolOS system compositor defaults (the WolOS desktop feel). Loaded by ~/.config/hypr/hyprland.lua.
-- Override anything by adding calls after the `require` in the user file.
-- Lua configuration of the compositor 0.57 series (hyprland.conf is the 0.56 one, kept for v0.12.x);
-- checked with `Hyprland --verify-config -c` on 0.57.1.
local mod = "SUPER"

-- Compositor performance defaults (vfr, damage tracking, cached blur, HW cursor).
require("/usr/share/wolos/hypr/wolos-perf.lua")
-- Privacy: who may copy the screen, capture input or load plugins (wol_core::security).
require("/usr/share/wolos/hypr/permissions.lua")

-- Plugin: hyprbars (hyprland-plugins). Path set by the image build.
hl.plugin.load("/usr/lib/hyprland/plugins/libhyprbars.so")
-- wol-hypr: placement + foreground QoS (plugins/wol-hypr).
hl.plugin.load("/usr/lib/hyprland/plugins/libwol-hypr.so")

-- Top-level hl.exec_cmd runs on every load (start and reload); exec-once lines run on hl.on("hyprland.start").
local function once(cmd)
    hl.on("hyprland.start", function()
        hl.exec_cmd(cmd)
    end)
end

-- VM GPUs (vmwgfx): no pointer on the cursor plane, draw it (again after a reload).
hl.exec_cmd([[sh -c 'systemd-detect-virt -q --vm && hyprctl eval "hl.config({cursor = {no_hardware_cursors = 1}})"']])
-- The same as a setting (wol-session writes it each start; reload keeps it, no frame with the plane).
-- A start without wol-session (a TTY, another launcher) has no such file: nothing to load then.
local vm_lua = (os.getenv("HOME") or "") .. "/.cache/wolos/hypr-vm.lua"
local vm_file = io.open(vm_lua, "r")
if vm_file then
    vm_file:close()
    require(vm_lua)
end
once("wol-vbox-guest") -- VirtualBox: resize with the VM window, clipboard, drag and drop (only under VirtualBox)
-- --systemd: the systemd user manager gets DISPLAY/WAYLAND_DISPLAY too, whichever D-Bus daemon runs.
once([[sh -c 'dbus-update-activation-environment --systemd --all; systemctl --user restart wolos-session.target']])
-- The portal and the accessibility bus start now, in parallel with the shell: GTK waits for the portal
-- registry at display open (2.8 s warm, 10 s at first boot, QA-100 H17-03).
once("systemctl --user start --no-block xdg-desktop-portal.service xdg-desktop-portal-gtk.service xdg-desktop-portal-hyprland.service at-spi-dbus-bus.service")
-- X11 apps follow the monitor scale: Xft.dpi, Xcursor.size, GDK_SCALE, Steam UI (again after a reload).
hl.exec_cmd("wol-x11-env")
-- Xwayland has no auth cookie: only this account's X11 clients may connect (other users, root).
once([[sh -c 'xhost +si:localuser:"$(id -un)" && xhost -local: && xhost -']])
once("gnome-keyring-daemon --start --components=secrets") -- finishes the login keyring wol-login unlocked
-- Input method (wolos-module-ime): nothing in this session runs /etc/xdg/autostart, so start it here;
-- XMODIFIERS/QT_IM_MODULE come from /etc/profile.d/wolos-ime.sh.
once([[sh -c '[ -x /usr/bin/fcitx5 ] && exec fcitx5 -d']])
once([[sh -c 'exec wol-shell --supervised 2>>"$XDG_RUNTIME_DIR/wol-shell.log"']])
once([[wol theme apply]]) -- GTK/Qt/foot files + gsettings from the theme in use; saves nothing (QA-SET-63)
-- Live: the account gets Flathub (QA-FDAY-11). /var/lib/flatpak is not there, so flatpak in a terminal
-- saw no remote; the file ships the key, nothing is downloaded (the Store adds the same remote).
once([[sh -c '[ -e /etc/wolos-live ] && exec flatpak remote-add --user --if-not-exists flathub /etc/flatpak/remotes.d/flathub.flatpakrepo']])

hl.env("XDG_CURRENT_DESKTOP", "Hyprland")
hl.env("XDG_SESSION_TYPE", "wayland")
-- Qt on Wayland; an app without the Wayland plugin (old Qt 5, AppImages) falls back to X11.
hl.env("QT_QPA_PLATFORM", "wayland;xcb")
hl.env("QT_QPA_PLATFORMTHEME", "qt6ct")
hl.env("MOZ_ENABLE_WAYLAND", "1")
-- Java/AWT (JetBrains, Android Studio, Swing) under Xwayland: the compositor does not reparent windows.
hl.env("_JAVA_AWT_WM_NONREPARENTING", "1")
hl.env("XCURSOR_THEME", "Adwaita")
hl.env("XCURSOR_SIZE", "24")
-- The compositor's own pointer (hyprcursor) follows the same theme and size; a theme without a hyprcursor
-- build falls back to the Xcursor one. Settings > Accessibility writes the same four values.
hl.env("HYPRCURSOR_THEME", "Adwaita")
hl.env("HYPRCURSOR_SIZE", "24")

hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })

hl.config({
    input = {
        kb_layout = "us",
        follow_mouse = 2,
        natural_scroll = true,
        touchpad = {
            natural_scroll = true,
            tap_to_click = true,
            clickfinger_behavior = true,
            disable_while_typing = true,
        },
    },
})

-- 3-finger horizontal swipe switches space.
hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })
-- 3-finger swipe up/down: Overview (wol-shell overview surface, ADR 0005).
hl.gesture({ fingers = 3, direction = "up", action = function() hl.exec_cmd("wol shell show overview") end })
hl.gesture({ fingers = 3, direction = "down", action = function() hl.exec_cmd("wol shell hide overview") end })

hl.config({
    general = {
        gaps_in = 6,
        gaps_out = 12,
        border_size = 0,
        layout = "dwindle",
        resize_on_border = true,
    },
})

-- Zoom (maximise, fullscreen state 1) fills the band between the menu bar and the Dock exactly:
-- no gaps, square corners, like a zoomed window. f[1] = a workspace whose fullscreen window
-- is maximised.
hl.workspace_rule({ workspace = "f[1]", gaps_out = 0, gaps_in = 0, no_rounding = true })

hl.config({
    decoration = {
        rounding = 12,
        blur = {
            enabled = true,
            size = 8,
            passes = 3,
            vibrancy = 0.2,
            new_optimizations = true,
        },
        shadow = {
            enabled = true,
            range = 30,
            render_power = 3,
            offset = "0 8",
            -- color, color_inactive: the theme (hypr/wolos/theme.lua below).
        },
    },
})

hl.config({ animations = { enabled = true } })
-- Generated from crates/realtime/src/anim.rs (the shell's curves and durations, HIG section 7).
-- Speed in deciseconds; a spring ignores it (the value is its settle time).
hl.curve("ease", { type = "bezier", points = { { 0.2, 0.8 }, { 0.2, 1 } } })
hl.curve("in", { type = "bezier", points = { { 0.4, 0 }, { 1, 1 } } })
hl.curve("out", { type = "bezier", points = { { 0, 0 }, { 0.2, 1 } } })
hl.curve("in-out", { type = "bezier", points = { { 0.4, 0 }, { 0.2, 1 } } })
hl.curve("glass", { type = "bezier", points = { { 0.34, 1.56 }, { 0.64, 1 } } })
hl.curve("overshoot", { type = "bezier", points = { { 0.22, 1.2 }, { 0.36, 1 } } })
hl.curve("soft", { type = "bezier", points = { { 0.25, 0.1 }, { 0.25, 1 } } })
hl.curve("window-open", { type = "spring", stiffness = 385.5, damping = 31.4, mass = 1 })
hl.curve("window-close", { type = "spring", stiffness = 322.3, damping = 30.5, mass = 1 })
hl.animation({ leaf = "windowsIn", enabled = true, speed = 1.9, spring = "window-open", style = "popin 90%" })
hl.animation({ leaf = "windowsOut", enabled = true, speed = 2.3, spring = "window-close", style = "popin 90%" })
hl.animation({ leaf = "windowsMove", enabled = true, speed = 2.5, bezier = "overshoot" })
hl.animation({ leaf = "fade", enabled = true, speed = 2.5, bezier = "soft" })
hl.animation({ leaf = "fadeIn", enabled = true, speed = 2.5, bezier = "ease" })
hl.animation({ leaf = "fadeOut", enabled = true, speed = 1.5, bezier = "in" })
hl.animation({ leaf = "layers", enabled = true, speed = 2.5, bezier = "ease", style = "fade" })
hl.animation({ leaf = "layersOut", enabled = true, speed = 1.5, bezier = "in", style = "fade" })
hl.animation({ leaf = "fadeLayersIn", enabled = true, speed = 2.5, bezier = "ease" })
hl.animation({ leaf = "fadeLayersOut", enabled = true, speed = 1.5, bezier = "in" })
hl.animation({ leaf = "workspaces", enabled = true, speed = 4.0, bezier = "soft", style = "slide" })
hl.animation({ leaf = "specialWorkspace", enabled = true, speed = 4.0, bezier = "soft", style = "slidevert" })

hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        -- X11 _NET_ACTIVE_WINDOW carries no activation token: a Steam or Wine pop-up took the keyboard
        -- mid-typing (ADR 0027). A window asking for focus is marked urgent; the Dock shows it.
        focus_on_activate = false,
        -- hyprland-guiutils is not shipped: no "missing" banner (compositor nags are never shown to users).
        disable_hyprland_guiutils_check = true,
        -- One picture from the loader splash on (ADR 0012): black like the splash and the console.
        background_color = "rgb(000000)",
        -- wol-lock's supervisor replaces a crashed lock screen; the session stays locked meanwhile.
        allow_session_lock_restore = true,
        lockdead_screen_delay = 5000,
    },
})

-- Floating by default. Size and position come from wol-hypr (realtime placement.rs): a window keeps
-- the size it asked for, cut to the usable area; dialogs centre on their parent. A `size` rule here
-- hit dialogs and fixed-size windows too, which flashed at the wrong size.
-- Named so System Settings > Desktop & Dock > Tile windows can turn it off live
-- (the rule's `enabled`); wol-float-apps keeps chosen apps floating while tiling.
hl.window_rule({ name = "wol-float", match = { class = ".*" }, float = true })
hl.window_rule({ name = "wol-float-apps", enabled = false, match = { class = "^$" }, float = true })
-- The terminal is opaque (QA-LIVE-12): foot's own `alpha` (the theme's 0.9 glass look) lets the windows behind
-- it bleed through the text where the compositor has no blur. `opaque` drops the buffer's alpha.
hl.window_rule({ name = "wol-terminal-opaque", match = { class = "^(foot|footclient)$" }, opaque = true })
-- The polkit password dialog (pkexec, Files, disks) stays on top and keeps the keyboard until it is
-- answered: once another window was focused nothing brought it back, and the password typed next
-- landed in the terminal and its history (QA-LIVE-38; interim until a WolOS agent).
hl.window_rule({
    name = "wol-polkit-agent",
    match = { class = "^(lxqt-policykit-agent)$" },
    float = true,
    center = true,
    pin = true,
    stay_focused = true,
    dim_around = true,
})
-- Placement: wol-hypr centres new floating windows slightly high (realtime placement.rs).

-- Blur and fade for shell surfaces.
-- Namespaces are regexes; ignore_alpha > 0 keeps blur off the transparent margins and shadows.
-- Search (wol-panel-searchbar) and Control Center (wol-panel-controlcenter) are resident wol-shell panels.
hl.layer_rule({ match = { namespace = "^(wol-menubar|wol-dock|wol-panel-.*)$" }, blur = true, ignore_alpha = 0.3 })
-- The screenshot bar and thumbnail vanish at once: a fade-out would land in the capture.
hl.layer_rule({ match = { namespace = "^(wol-panel-capture|wol-panel-capture-thumb)$" }, no_anim = true })
-- A toplevel whose app id ends in .glass (a Wolkit window whose root computes backdrop: glass) blurs
-- what is really behind it; the session blur is xray, the wallpaper only. realtime glass.rs.
hl.window_rule({ name = "wol-glass", match = { class = "^.+\\.glass$" }, no_blur = false, xray = false })

-- Title bars: window buttons right (close at the edge, then zoom, minimise), title centered.
-- The strip's keys and rule effects exist once its plugin is loaded: the compositor loads the
-- plugins after the first pass of this file and runs it again, so the first pass skips them.
if hl.plugin.hyprbars then
    hl.config({
        plugin = {
            hyprbars = {
                bar_height = 28,
                -- Colours (bar, title, the inactive look) come from the theme: hypr/wolos/theme.lua below.
                bar_text_font = "Inter",
                bar_text_size = 11,
                bar_text_align = "center",
                bar_part_of_window = true,
                bar_precedence_over_border = true,
                bar_buttons_alignment = "right",
                bar_padding = 17,
                bar_button_padding = 10,
                on_double_click = "zoom",
                -- Pointer over any button: glyphs on all three.
                icon_on_hover = true,
            },
        },
    })
    -- One set of window buttons per window (realtime chrome.rs): wol-hypr tags windows that draw their
    -- own (no strip) and fixed-size dialogs (close only).
    hl.window_rule({ name = "wol-lights-none", match = { tag = "wol-lights-none" }, ["hyprbars:no_bar"] = true })
    hl.window_rule({ name = "wol-lights-close", match = { tag = "wol-lights-close" }, ["hyprbars:buttons"] = 1 })
end
-- The window buttons (Kit close/minimise/zoom), loaded once: each call adds a button.
require("/usr/share/wolos/hypr/wolos/lights.lua")
-- Every colour is the theme's (wol_core::theme::hypr): this is the default theme's render;
-- `wol theme set` writes ~/.config/hypr/wolos/theme.lua (user file glob, read after this file)
-- and sets the same keys live.
require("/usr/share/wolos/hypr/wolos/theme.lua")

-- Super is Command (⌘). Plain ⌘+letter belongs to the app in front; system shortcuts are the WolOS ones.
local function bind(keys, cmd, opts)
    hl.bind(keys, hl.dsp.exec_cmd(cmd), opts)
end
local REPEAT = { repeating = true }
local LOCKED = { locked = true }
local LOCKED_REPEAT = { locked = true, repeating = true }

-- Apps / shell
bind(mod .. " + Space", "wol-search")
bind(mod .. " + Return", "foot")
bind(mod .. " + comma", "wol-settings")
bind(mod .. " + Escape", "wol shell toggle session")
bind(mod .. " + ALT + Escape", "wol shell toggle force-quit")
-- Every shortcut on one sheet (the ⌘? help key).
bind(mod .. " + SHIFT + slash", "wol shell toggle shortcuts")
-- Keyboard into the menu bar and the Dock (⌃F2, ⌃F3); Esc gives it back.
bind("CTRL + F2", "wol shell show menubar")
bind("CTRL + F3", "wol shell show dock")
-- The two panels without a pointer (QA-ACC-7): ⌃⌘C Control Center, ⌃⌘A Notification Center (Alerts).
bind("CTRL + " .. mod .. " + C", "wol shell toggle control-center")
bind("CTRL + " .. mod .. " + A", "wol shell toggle notifications")
-- Esc closes menus and panels and dismisses notification banners, and releasing Super finishes ⌘Tab:
-- no binds. The wol-hypr plugin forwards Esc (`hide transient`) and the Super release (`hide
-- switcher`) to the shell socket only while a shell layer is on screen, without a process; the app
-- still gets its keys (audit3 R2A-10: a `wol` process per Esc in every app, games included).
-- Overview: ⌃↑, ⌘↑ too.
bind("CTRL + Up", "wol shell toggle overview")
bind(mod .. " + Up", "wol shell toggle overview")
-- App switcher: hold Super, tap Tab (Shift reverses); releasing Super focuses the selection (wol-hypr).
bind(mod .. " + Tab", "wol shell show switcher")
bind(mod .. " + SHIFT + Tab", "wol shell show switcher-prev")
bind("CTRL + " .. mod .. " + Q", "wol-lock")

-- App commands: wol-app taps the app's own key (Ctrl+C; a terminal's Ctrl+Shift+C), appmenu::send.
bind(mod .. " + C", "wol-app copy")
bind(mod .. " + V", "wol-app paste", REPEAT)
bind(mod .. " + X", "wol-app cut")
bind(mod .. " + Z", "wol-app undo", REPEAT)
bind(mod .. " + SHIFT + Z", "wol-app redo", REPEAT)
bind(mod .. " + A", "wol-app select-all")
bind(mod .. " + S", "wol-app save")
bind(mod .. " + O", "wol-app open")
bind(mod .. " + P", "wol-app print")
bind(mod .. " + F", "wol-app find")
bind(mod .. " + G", "wol-app find-next", REPEAT)
bind(mod .. " + T", "wol-app new-tab")
bind(mod .. " + W", "wol-app close")
bind(mod .. " + N", "wol-app new")
bind(mod .. " + R", "wol-app reload")
bind(mod .. " + L", "wol-app location")
bind(mod .. " + equal", "wol-app zoom-in", REPEAT)
bind(mod .. " + minus", "wol-app zoom-out", REPEAT)
bind(mod .. " + 0", "wol-app actual-size")
bind("CTRL + " .. mod .. " + Space", "wol-app emoji")
-- Clipboard history (⇧⌘V, the common clipboard-manager key).
bind(mod .. " + SHIFT + V", "wol shell toggle clipboard")
-- Theme picker (ADR 0031 themes v2): every theme as a preview card.
bind("CTRL + " .. mod .. " + T", "wol shell toggle themes")

-- Windows (Cmd semantics)
-- App actions shared with the menu bar app/Window menus (wol-app).
bind(mod .. " + Q", "wol-app quit")
bind(mod .. " + H", "wol-app hide")
bind(mod .. " + ALT + H", "wol-app hide-others")
bind(mod .. " + M", "wol-app minimise")
-- (no ⇧⌘H: it showed special:hidden, every minimised window at once, as an overlay; minimised
-- windows come back through the Dock, the switcher or app activation: fork patch 0008.)
hl.bind("CTRL + " .. mod .. " + F", hl.dsp.window.fullscreen({ mode = "fullscreen" }))
hl.bind(mod .. " + mouse:272", hl.dsp.window.drag(), { mouse = true })
hl.bind(mod .. " + mouse:273", hl.dsp.window.resize(), { mouse = true })

-- Screenshots: 3 full, 4 region, 5 region to clipboard
bind(mod .. " + SHIFT + 3", "wol-screenshot full")
bind(mod .. " + SHIFT + 4", "wol-screenshot region")
bind(mod .. " + SHIFT + 5", "wol shell show screenshot")
bind("Print", "wol-screenshot full")
-- Copy to the clipboard instead of saving (Control with the same keys).
bind("CTRL + " .. mod .. " + SHIFT + 3", "wol-screenshot full --copy")
bind("CTRL + " .. mod .. " + SHIFT + 4", "wol-screenshot clip")

-- Accessibility: zoom around the pointer (⌥⌘8, ⌥⌘=, ⌥⌘-).
bind(mod .. " + ALT + 8", "wol-zoom toggle")
bind(mod .. " + ALT + equal", "wol-zoom in", REPEAT)
bind(mod .. " + ALT + minus", "wol-zoom out", REPEAT)

-- Spaces: ⌃⌥←/→ and ⌃⌥1..4 (the GNOME keys). Not plain ⌃: apps own Ctrl+arrows (word-wise cursor)
-- and Ctrl+1..9 (browser tabs), and Super is ⌘ here, so ⌃ alone is the app's key. Not ⌃⌘: tiling.lua
-- resizes with ⌃⌘arrows. Known overlap (audit3 R2A-6): JetBrains IDEs (Navigate Back/Forward) and
-- VS Code (move editor between groups) use ⌃⌥←/→ too and lose them here; Settings > Keyboard rebinds.
hl.bind("CTRL + ALT + Left", hl.dsp.focus({ workspace = "e-1" }))
hl.bind("CTRL + ALT + Right", hl.dsp.focus({ workspace = "e+1" }))
hl.bind("CTRL + ALT + 1", hl.dsp.focus({ workspace = 1 }))
hl.bind("CTRL + ALT + 2", hl.dsp.focus({ workspace = 2 }))
hl.bind("CTRL + ALT + 3", hl.dsp.focus({ workspace = 3 }))
hl.bind("CTRL + ALT + 4", hl.dsp.focus({ workspace = 4 }))

-- Moving a window (QA-PWR-11): ⌃⌥⇧←/→ and ⌃⌥⇧1..4 send the focused window to another space (the
-- same keys as switching, plus ⇧); ⌃⌥⌘←/→ send it to the display on that side. hyprctl does the
-- move so the .conf and .lua forms are one text (units.rs hypr_conf_and_lua_bind_sets_equal).
bind("CTRL + ALT + SHIFT + Left", "hyprctl dispatch movetoworkspace e-1")
bind("CTRL + ALT + SHIFT + Right", "hyprctl dispatch movetoworkspace e+1")
bind("CTRL + ALT + SHIFT + 1", "hyprctl dispatch movetoworkspace 1")
bind("CTRL + ALT + SHIFT + 2", "hyprctl dispatch movetoworkspace 2")
bind("CTRL + ALT + SHIFT + 3", "hyprctl dispatch movetoworkspace 3")
bind("CTRL + ALT + SHIFT + 4", "hyprctl dispatch movetoworkspace 4")
bind("CTRL + ALT + " .. mod .. " + Left", "hyprctl dispatch movewindow mon:l")
bind("CTRL + ALT + " .. mod .. " + Right", "hyprctl dispatch movewindow mon:r")

-- Toggles (wol toggle, ADR 0031 W5): each key flips the state.
bind("CTRL + " .. mod .. " + N", "wol toggle nightlight")
bind("CTRL + " .. mod .. " + D", "wol toggle dnd")
bind("CTRL + " .. mod .. " + I", "wol toggle stay-awake")

-- Media
bind("XF86AudioRaiseVolume", "wol-volume up", LOCKED_REPEAT)
bind("XF86AudioLowerVolume", "wol-volume down", LOCKED_REPEAT)
bind("XF86AudioMute", "wol-volume mute", LOCKED)
bind("XF86AudioMicMute", "wol-volume micmute", LOCKED)
bind("XF86MonBrightnessUp", "wol-backlight up", LOCKED_REPEAT)
bind("XF86MonBrightnessDown", "wol-backlight down", LOCKED_REPEAT)
bind("XF86AudioPlay", "playerctl play-pause", LOCKED)
bind("XF86AudioNext", "playerctl next", LOCKED)
bind("XF86AudioPrev", "playerctl previous", LOCKED)

-- System Settings > Keyboard > Shortcuts records a new shortcut in this submap: WolOS shortcuts pause so
-- the keys reach Settings. Esc (non-consuming) always returns.
hl.define_submap("wol-record", function()
    hl.bind("escape", hl.dsp.submap("reset"), { non_consuming = true })
end)

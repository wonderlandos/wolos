// SPDX-License-Identifier: GPL-2.0-or-later
//! The desktop hyprland.conf drives hyprbars and window animations; it must agree with the
//! realtime geometry/curves and the kit (12 px buttons, close centre 22.5 px in from the right
//! edge, 22 px apart).
//! Live 0.56.2: buttons were 20 px apart, CSD apps got a second bar, and the
//! open animation used a 400 ms overshooting spring instead of our ease-out.
use realtime::anim::{Animation, Bezier};
use realtime::titlebar::{self, Align, Button};
const CONF: &str = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
/// The window buttons, sourced once by hyprland.conf (colours live outside it: themes v2).
const LIGHTS: &str =
    include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos/lights.conf");
/// The default theme's compositor colours (`wol_core::theme::hypr`), sourced by hyprland.conf.
const THEME: &str = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos/theme.conf");

fn value(key: &str) -> String {
    CONF.lines()
        .map(str::trim)
        .find_map(|l| {
            l.strip_prefix(key)?
                .trim()
                .strip_prefix('=')
                .map(|v| v.trim().to_owned())
        })
        .unwrap_or_else(|| panic!("{key} missing"))
}

fn rgb(s: &str) -> (i32, i32, i32) {
    let h = s
        .trim()
        .strip_prefix("rgb(")
        .unwrap()
        .strip_suffix(')')
        .unwrap();
    let c = |i: usize| i32::from_str_radix(&h[i..i + 2], 16).unwrap();
    (c(0), c(2), c(4))
}

#[test]
fn hyprbars_buttons_match_kit_order_colour_and_spacing() {
    assert!(CONF.contains("\nsource = /usr/share/wolos/hypr/wolos/lights.conf\n"));
    let buttons: Vec<Vec<String>> = LIGHTS
        .lines()
        .filter_map(|l| l.trim().strip_prefix("hyprbars-button"))
        .map(|l| {
            l.trim_start_matches([' ', '='])
                .split(',')
                .map(|s| s.trim().to_owned())
                .collect()
        })
        .collect();
    // WolOS window buttons: one quiet neutral for all three (no red, amber and green), the ink
    // glyph on it readable (>= 3:1), and the circle visible on both bars (the default light and
    // dark surfaces, >= 3:1).
    assert_eq!(buttons.len(), 3);
    let lum = |c: (i32, i32, i32)| {
        let f = |v: i32| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
    };
    let contrast = |a: (i32, i32, i32), b: (i32, i32, i32)| {
        let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
        (hi + 0.05) / (lo + 0.05)
    };
    let circle = rgb(&buttons[0][0]);
    // Right-aligned: hyprbars puts the first button at the right edge, so the file lists
    // realtime's right row (close, zoom, minimise); the patch maps slot i to the i-th button.
    let action = |b| match b {
        Button::Close => "close",
        Button::Minimize => "minimize",
        Button::Zoom => "zoom",
    };
    let order = Align::Right.order().map(action);
    assert_eq!(order, ["close", "zoom", "minimize"]);
    for (b, action) in buttons.iter().zip(order) {
        // The patched hyprbars runs these on the bar's own window (desktop/hyprland/hyprbars).
        assert_eq!(b[3], action, "order: {b:?} should be {action}");
        assert_eq!(b[1], titlebar::BUTTON.to_string());
        assert_eq!(rgb(&b[0]), circle, "one colour for all three: {b:?}");
        let ink = (0x22, 0x1b, 0x36);
        assert!(contrast(ink, circle) >= 3.0, "glyph on the circle: {b:?}");
    }
    for (name, bar) in [("light", (255, 255, 255)), ("dark", (0x2c, 0x2c, 0x2e))] {
        assert!(contrast(circle, bar) >= 3.0, "circle on the {name} bar");
    }
    assert_eq!(value("bar_buttons_alignment"), "right");
    assert_eq!(value("bar_padding"), titlebar::INSET.to_string());
    assert_eq!(value("bar_button_padding"), titlebar::GAP.to_string());
    assert_eq!(value("bar_height"), titlebar::HEIGHT.to_string());
    // Close centre 22.5 px in from the right edge, the next ones 22 px apart leftward, at any width.
    for w in [300, 641, 1280] {
        let centre =
            |b| titlebar::origin(Align::Right, w, b).0 as f32 + titlebar::BUTTON as f32 / 2.0;
        assert!(
            (w as f32 - centre(Button::Close) - 22.5).abs() <= 1.0,
            "{w}"
        );
        assert!((centre(Button::Close) - centre(Button::Zoom) - 22.0).abs() <= 1.0);
        assert!((centre(Button::Zoom) - centre(Button::Minimize) - 22.0).abs() <= 1.0);
    }
    assert_eq!(value("on_double_click"), "zoom");
    // Hovering any button shows the glyphs on all three (titlebar::Hover).
    assert_eq!(value("icon_on_hover"), "true");
    for (b, glyph) in buttons.iter().zip(["×", "□", "−"]) {
        assert_eq!(b[2], glyph, "{b:?}");
        assert!(
            b.len() == 5 && b[4].starts_with("rgba("),
            "glyph colour: {b:?}"
        );
    }
}

/// The server-side buttons look like the client-side ones (`wol_core::theme::apps` window
/// controls): the button under the pointer takes the theme's accent, deeper while pressed, its glyph
/// in the text on the accent. The patched hyprbars places and hits them with realtime's
/// right-aligned row, against the bar's own width.
#[test]
fn hyprbars_buttons_are_right_aligned_and_take_the_accent_on_hover() {
    for key in [
        "button_hover_color",
        "button_pressed_color",
        "col.button_hover_text",
    ] {
        let line = format!("plugin:hyprbars:{key} = rgba(");
        assert!(THEME.lines().any(|l| l.starts_with(&line)), "{key}");
    }
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../desktop/hyprland/hyprbars"
    );
    let patches: String = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    let added = |s: &str| patches.lines().any(|l| l.starts_with('+') && l.contains(s));
    for s in [
        "wol_lights_at_w((int)assignedBoxGlobal().w",
        "wol_lights_over_w((int)assignedBoxGlobal().w",
        "wol_lights_slot_w(slot)",
        "\"plugin:hyprbars:button_hover_color\"",
        "\"plugin:hyprbars:button_pressed_color\"",
        "\"plugin:hyprbars:col.button_hover_text\"",
        "addConfigValueV2(PHANDLE, g_pGlobalState->config.buttonHoverColor)",
    ] {
        assert!(added(s), "{s}");
    }
    // The header carries the additive calls next to the left-aligned ones.
    let header = include_str!("../include/wol_realtime.h");
    for f in [
        "int32_t wol_lights_at(int32_t count",
        "int32_t wol_lights_over(int32_t count",
        "int32_t wol_lights_at_w(int32_t width, int32_t count",
        "int32_t wol_lights_over_w(int32_t width, int32_t count",
        "int32_t wol_lights_slot_w(int32_t slot)",
    ] {
        assert!(header.contains(f), "{f}");
    }
}

/// Exactly one set of lights per window (realtime::chrome); wol-hypr tags what gets no strip.
#[test]
fn lights_exceptions_follow_the_chrome_policy() {
    let rules: Vec<&str> = CONF.lines().map(str::trim).collect();
    assert!(rules.contains(&"windowrule = match:tag wol-lights-none, hyprbars:no_bar on"));
    assert!(rules.contains(&"windowrule = match:tag wol-lights-close, hyprbars:buttons 1"));
    assert!(!CONF.contains("wol-csd"));
    let plugin = include_str!("../../../plugins/wol-hypr/main.cpp");
    assert!(plugin.contains("wol_window_lights(") && plugin.contains("wol-lights-none"));
    assert!(plugin.contains("wol-lights-close"));
}

/// Live: un-zooming put a window back where the layout first saw it, not where wol-hypr placed it:
/// 26 px off on Wayland, and at 0,0 on Xwayland (which maps there) with the title bar and its
/// buttons above the screen. Placement has to go through the layout so it remembers the box.
#[test]
fn placement_goes_through_the_layout() {
    let plugin = include_str!("../../../plugins/wol-hypr/main.cpp");
    assert!(plugin.contains("g_layoutManager->setTargetGeom("));
    assert!(!plugin.contains("setPositionGlobal"));
}

/// Live: dragging a window by its title bar and letting go over the menu bar or the Dock left
/// the drag running: the window stuck to the pointer and clicks went nowhere. The bar that
/// started a press or drag must see its release wherever it happens.
#[test]
fn hyprbars_is_patched_and_the_patch_ends_drags_anywhere() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../desktop/hyprland/hyprbars"
    );
    let patches: String = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    let added = |s: &str| patches.lines().any(|l| l.starts_with('+') && l.contains(s));
    assert!(added(
        "m_bDragPending || m_bDraggingThis || m_bCancelledDown || m_pressedButton >= 0"
    ));
    assert!(added("\"hyprbars:buttons\""));
    // The buttons act on the bar's own window in-process (0.57: Config::Actions, no dispatcher table).
    assert!(added("Config::Actions::closeWindow(PWINDOW)"));
    // Live: hover changed only while rendering, so a pointer that stopped after one motion over a
    // light got no glyphs (a hardware cursor makes no frame). Input decides, rendering reads.
    assert!(added("void CHyprBar::sampleHover()") && added("wol_lights_hover(m_hover"));
    let removed = |s: &str| patches.lines().any(|l| l.starts_with('-') && l.contains(s));
    assert!(removed("m_iButtonHoverState ^= (1 << i);"));
    assert!(!added("m_iButtonHoverState") && !added("m_bButtonHovered"));
    let build = include_str!("../../../desktop/hyprland/build-plugins.sh");
    assert!(
        build.contains("\"$HERE\"/hyprbars/*.patch"),
        "build-plugins.sh applies the patches"
    );
}

/// Position and size of a new floating window come from wol-hypr (realtime placement.rs): the
/// conf's `size` rule hit dialogs and fixed-size windows, which flashed at the wrong size; a
/// dialog centres on its parent (`wol_centre`), a window keeps its own size cut to fit (`wol_fit`),
/// the next window of an app cascades (previous window passed to `wol_place`).
#[test]
fn plugin_owns_floating_placement() {
    assert!(
        !CONF
            .lines()
            .any(|l| l.trim().starts_with("windowrule") && l.contains(", move "))
    );
    let float = &CONF[CONF.find("name = wol-float\n").unwrap()..];
    let float = &float[..float.find('}').unwrap()];
    assert!(!float.contains("size ="), "conf sizes windows: {float}");
    let plugin = include_str!("../../../plugins/wol-hypr/main.cpp");
    for call in [
        "wol_fit(",
        "wol_centre(",
        "wol_place(",
        "isTransient(w) ? parentOf(w)",
        "previousOfClass(w)",
    ] {
        assert!(plugin.contains(call), "{call}");
    }
}

/// Zoom (`fullscreen 1`) fills the band between the menu bar and the Dock exactly: the Dock
/// reserves the shelf only (shell ui::dock::exclusive_zone), and a maximised workspace has no gaps
/// and no rounding. Live: a 12 px gap and a 36 px band above the shelf.
#[test]
fn zoomed_windows_fill_the_band_between_menu_bar_and_dock() {
    assert!(
        CONF.lines()
            .any(|l| l.trim() == "workspace = f[1], gapsout:0, gapsin:0, rounding:false"),
        "maximised workspace rule"
    );
}

/// Inactive windows look inactive: grey lights, a flatter bar and a dimmer title on the window
/// without focus; the hyprbars patch adds the two colours and tracks focus for all three.
#[test]
fn inactive_windows_look_inactive() {
    for key in [
        "inactive_button_color",
        "bar_color_inactive",
        "col.text_inactive",
    ] {
        let line = format!("plugin:hyprbars:{key} = rgba(");
        assert!(THEME.lines().any(|l| l.starts_with(&line)), "{key}");
    }
    assert!(CONF.contains("\nsource = /usr/share/wolos/hypr/wolos/theme.conf\n"));
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../desktop/hyprland/hyprbars"
    );
    let patches: String = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    let added = |s: &str| patches.lines().any(|l| l.starts_with('+') && l.contains(s));
    assert!(
        added("plugin:hyprbars:bar_color_inactive") && added("plugin:hyprbars:col.text_inactive")
    );
    assert!(
        added("m_bTitleColorChanged = true;"),
        "a focus change re-renders the title"
    );
}

/// The usable area changes live (VBoxClient resizes the guest display with the VM window, the
/// Dock hides or returns): open floating windows are refit and zoomed windows recalculated.
#[test]
fn display_or_dock_changes_refit_open_windows() {
    let plugin = include_str!("../../../plugins/wol-hypr/main.cpp");
    assert!(plugin.contains("m_events.render.pre.listen("));
    assert!(plugin.contains("wol_refit(") && plugin.contains("recalculateMonitor("));
    let header = include_str!("../include/wol_realtime.h");
    for f in ["wol_fit(", "wol_centre(", "wol_refit("] {
        assert!(header.contains(f), "{f}");
    }
}

#[test]
fn window_animations_use_realtime_curves() {
    let bez = |name: &str| -> Vec<f32> {
        let l = CONF
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("bezier") && l.contains(name))
            .unwrap();
        l.split(',')
            .skip(1)
            .map(|v| v.trim().parse().unwrap())
            .collect()
    };
    let anim = |name: &str| -> Vec<String> {
        let l = CONF
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(&format!("animation = {name},")))
            .unwrap();
        l.split(',').skip(1).map(|v| v.trim().to_owned()).collect()
    };
    for (name, curve, want, ms) in [
        (
            "windowsIn",
            "wolOut",
            Bezier::EASE_OUT,
            Animation::open(0).duration_ms,
        ),
        (
            "windowsOut",
            "wolIn",
            Bezier::EASE_IN,
            Animation::close(0).duration_ms,
        ),
    ] {
        let a = anim(name);
        assert_eq!(a[2], curve);
        // The compositor's speed is in deciseconds.
        let speed: f32 = a[1].parse().unwrap();
        assert!(
            (speed * 100.0 - ms as f32).abs() < 1.0,
            "{name}: {speed}ds vs {ms}ms"
        );
        assert_eq!(bez(curve), vec![want.0, want.1, want.2, want.3]);
    }
}

#[test]
fn every_animation_names_a_defined_bezier() {
    // Live: dropping `spring`/`ease` left five animations with "no such bezier".
    let defined: Vec<&str> = CONF
        .lines()
        .filter_map(|l| l.trim().strip_prefix("bezier = "))
        .map(|l| l.split(',').next().unwrap().trim())
        .collect();
    for l in CONF
        .lines()
        .filter_map(|l| l.trim().strip_prefix("animation = "))
    {
        let f: Vec<&str> = l.split(',').map(str::trim).collect();
        if f.len() > 3 && f[1] == "1" {
            assert!(
                f[3] == "default" || defined.contains(&f[3]),
                "{l}: no such bezier"
            );
        }
    }
}

#[test]
fn every_wol_hypr_build_allows_private_access() {
    // main.cpp reads the private xdg-decoration map; a build without the flag fails to compile.
    for build in [
        include_str!("../../../plugins/wol-hypr/Makefile"),
        include_str!("../../../desktop/hyprland/build-plugins.sh"),
    ] {
        assert!(build.contains("-fno-access-control"));
    }
}

/// Live (iGPU passthrough): the setup and login screens run the compositor directly (wol-login restarts
/// them) and showed the guiutils and "launched without start-hyprland" banners over Setup.
#[test]
fn greeter_compositors_show_no_nag_banners() {
    for (name, conf) in [
        (
            "login.conf",
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/login.conf"),
        ),
        (
            "setup.conf",
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/setup.conf"),
        ),
    ] {
        for key in [
            "disable_hyprland_guiutils_check",
            "disable_watchdog_warning",
        ] {
            assert!(
                conf.lines()
                    .map(|l| l.split_whitespace().collect::<String>())
                    .any(|l| l == format!("{key}=true")),
                "{name} lacks {key} = true"
            );
        }
    }
}

/// A focus the user asked for (click, keybind, focuswindow, switcher) raises the floating window;
/// focus that merely follows the desktop (pointer, workspace switch, the fallback after a close)
/// leaves the order alone.
#[test]
fn activation_raises_the_window() {
    let plugin = include_str!("../../../plugins/wol-hypr/main.cpp");
    let hook = &plugin[plugin.find("static void onActive").unwrap()..];
    let hook = &hook[..hook.find("\n}").unwrap()];
    assert!(hook.contains("raiseLater(w)"), "{hook}");
    assert!(hook.contains("raisesOnFocus(reason)"), "{hook}");
    // Audit4 S5-6: raised in-process after the focus change, no hyprctl alterzorder (it simulated
    // pointer motion that handed a tiled-mode focus back to the window under the pointer).
    let raise = &plugin[plugin.find("static void raiseLater").unwrap()..];
    let raise = &raise[..raise.find("\n}").unwrap()];
    for call in ["doLater(", "windowState()->raise(w)", "bringTargetToTop("] {
        assert!(raise.contains(call), "{call}: {raise}");
    }
    assert!(!plugin.contains("alterzorder"));
    let filter = &plugin[plugin.find("static bool raisesOnFocus").unwrap()..];
    let filter = &filter[..filter.find("\n}").unwrap()];
    for yes in [
        "FOCUS_REASON_CLICK",
        "FOCUS_REASON_DISPATCH_FOCUSWINDOW",
        "FOCUS_REASON_KEYBIND",
    ] {
        assert!(filter.contains(yes), "{yes}");
    }
    for no in [
        "FOCUS_REASON_FFM",
        "FOCUS_REASON_WORKSPACE_CHANGE",
        "FOCUS_REASON_UNMAP_WINDOW_FLOATING",
    ] {
        assert!(!filter.contains(no), "{no} must not raise");
    }
}

/// ⇧⌘H showed special:hidden (every minimised window at once) as an overlay; minimised windows
/// come back through the Dock, the switcher or activation (fork patch 0008).
#[test]
fn no_shortcut_shows_the_hidden_workspace() {
    assert!(!CONF.lines().any(|l| {
        let l = l.split('#').next().unwrap();
        l.contains("togglespecialworkspace") && l.contains("hidden")
    }));
}

/// Live perf run: the perf defaults (vfr, damage tracking, cached blur, HW cursor) were installed
/// but never sourced, so the session ran without them. They ship with the desktop conf.
#[test]
fn desktop_conf_sources_the_perf_defaults() {
    assert!(
        CONF.lines()
            .any(|l| l.trim() == "source = /usr/share/wolos/hypr/wolos-perf.conf")
    );
    let perf = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos-perf.conf");
    assert!(perf.contains("vfr = true") && perf.contains("damage_tracking = 2"));
}

/// Live (iGPU passthrough): GTK 4's default renderer never mapped the shell, Setup or login windows;
/// cairo did. Session GTK apps use cairo until a GPU renderer is proven on hardware.
#[test]
fn session_gtk_uses_a_proven_renderer() {
    let session = include_str!("../../../desktop/overlay/usr/bin/wol-session");
    assert!(
        session
            .lines()
            .any(|l| l.trim() == "export GSK_RENDERER=${GSK_RENDERER:-cairo}")
    );
}

/// Live: fontconfig resolved "monospace" to Font Awesome (foot drew icon-font capitals).
#[test]
fn monospace_resolves_to_jetbrains_mono() {
    let fc = include_str!("../../../desktop/overlay/etc/fonts/conf.d/50-wolos-fonts.conf");
    let compact: String = fc.split_whitespace().collect();
    assert!(compact.contains("<family>monospace</family><prefer><family>JetBrainsMono</family>"));
}

/// Live (iGPU passthrough): Setup never mapped its window with GTK's default renderer and did with
/// cairo. Login window and Setup are small static screens: cairo, no GPU dependency before login.
#[test]
fn greeter_gtk_uses_cairo() {
    let g = include_str!("../../../desktop/overlay/usr/bin/wol-greeter-session");
    assert!(
        g.lines()
            .any(|l| l.trim() == "export GSK_RENDERER=${GSK_RENDERER:-cairo}")
    );
}

/// Live: the shell never appeared and left no trace. Its stderr goes to the session runtime dir.
#[test]
fn shell_output_is_logged() {
    assert!(
        CONF.lines().any(|l| l.trim()
            == r#"exec-once = sh -c 'exec wol-shell 2>>"$XDG_RUNTIME_DIR/wol-shell.log"'"#)
    );
}

/// Window open, close and resize settle in about 0.2 s (2 in the compositor's 100 ms units): longer
/// reads as lag when dragging a window edge.
#[test]
fn window_animations_take_about_a_fifth_of_a_second() {
    let conf = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../desktop/overlay/usr/share/wolos/hypr/hyprland.conf"),
    )
    .unwrap();
    for name in ["windowsIn", "windowsOut", "windowsMove"] {
        let l = conf
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(&format!("animation = {name},")))
            .unwrap_or_else(|| panic!("{name}"));
        let speed: f32 = l.split(',').nth(2).unwrap().trim().parse().unwrap();
        assert!((1.8..=2.2).contains(&speed), "{l}");
    }
}

/// Live (VM, virtio-gpu): with damage tracking on, the compositor never repainted the last frame of a
/// Setup page fade, so the previous page stayed half-visible behind the next one until Setup
/// closed. Setup and the login screen redraw whole frames; they are short-lived and mostly still.
#[test]
fn setup_and_login_redraw_whole_frames() {
    for f in ["setup.conf", "login.conf"] {
        let conf = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../desktop/overlay/usr/share/wolos/hypr")
                .join(f),
        )
        .unwrap();
        assert!(
            conf.lines().any(|l| l.trim() == "damage_tracking = 0"),
            "{f}: damage_tracking = 0"
        );
    }
}

/// The fork throttles covered windows, bounds the output-reconfigure wait and schedules against the
/// predicted present time; the shipped conf states the tuned values explicitly.
#[test]
fn compositor_frame_policy_is_configured() {
    let perf = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos-perf.conf");
    let misc = &perf[perf.find("misc {").unwrap()..];
    let misc = &misc[..misc.find("\n}").unwrap()];
    let get = |k: &str| -> f64 {
        misc.lines()
            .find_map(|l| {
                let l = l.split('#').next().unwrap().trim();
                l.strip_prefix(k)?
                    .trim()
                    .strip_prefix('=')?
                    .trim()
                    .replace("true", "1")
                    .parse()
                    .ok()
            })
            .unwrap_or_else(|| panic!("misc:{k} missing"))
    };
    assert_eq!(get("throttle_occluded"), 1.0);
    assert_eq!(get("throttle_window_count"), 10.0);
    assert_eq!(get("throttle_occlusion_cutoff"), 0.2);
    assert_eq!(get("throttle_freeze_ms"), 33.3);
    assert_eq!(get("throttle_freeze_updates"), 2.0);
    assert_eq!(get("reconfigure_ack_timeout_ms"), 250.0);
    assert_eq!(get("frame_deadline"), 0.95);
}

/// Audit-x11 X1-1 / audit4 S5-2: X11 override-redirect and borderless popups are never placed,
/// cascaded from, refitted or tagged; the rule is realtime's (wm_gates.rs via wol_realtime.h).
#[test]
fn x11_popups_are_left_where_they_open() {
    let plugin = include_str!("../../../plugins/wol-hypr/main.cpp");
    let body = |name: &str| {
        let f = &plugin[plugin.find(name).unwrap_or_else(|| panic!("{name}"))..];
        f[..f.find("\n}").unwrap()].to_owned()
    };
    assert!(body("static bool x11OverrideRedirect").contains("isX11OverrideRedirect()"));
    assert!(body("static int32_t placementOf").contains("wol_x11_placement("));
    assert!(body("static bool managedWindow").contains("wol_x11_managed("));
    let late = body("static void onOpenLate");
    assert!(late.contains("placementOf(w)") && late.contains("WOL_PLACE_SKIP"));
    assert!(late.contains("WOL_PLACE_CLAMP_ONLY"));
    assert!(body("static void onOpen(").contains("!managedWindow(w)"));
    assert!(body("static PHLWINDOW previousOfClass").contains("!managedWindow(o)"));
    assert!(body("static void refitMonitor").contains("!managedWindow(w)"));
    let header = include_str!("../include/wol_realtime.h");
    for f in [
        "wol_x11_placement(",
        "wol_x11_managed(",
        "wol_keys_step(",
        "wol_placement_limits(",
    ] {
        assert!(header.contains(f), "{f}");
    }
}

// ---- The Lua configuration of the compositor 0.57 series (WolOS 0.13). While both forms ship, the
// Lua files carry the same settings as the `.conf` files the tests above check, so those checks hold
// for both.

const SHARE: &str = "../../../desktop/overlay/usr/share/wolos/hypr";
const LUA: &str = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/hyprland.lua");
const LIGHTS_LUA: &str =
    include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos/lights.lua");

/// `key = value` leaves of a `.conf` (block or `category:key` lines) or of a Lua table, values
/// unquoted and numbers normalised, comments dropped; top-level calls and keywords skipped.
fn leaves(text: &str, lua: bool) -> Vec<(String, String)> {
    let comment = if lua { "--" } else { "#" };
    let num = |v: &str| v.parse::<f64>().map_or(v.to_string(), |n| format!("{n}"));
    text.lines()
        .filter_map(|l| l.split(comment).next())
        .map(str::trim)
        .flat_map(|l| {
            // A one-line `hl.window_rule({ name = …, match = { k = v }, effect = v })`, a one-line
            // `hl.config({ a = { b = v } })` or an inline `a = { b = v },`: their fields.
            let inline = l.split_once(" = {").is_some_and(|(_, r)| r.contains(" = "));
            if let Some(r) = l
                .strip_prefix("hl.window_rule(")
                .or_else(|| l.strip_prefix("hl.config("))
                .or(inline.then_some(l))
            {
                return r
                    .split([',', '{', '}'])
                    .map(str::to_string)
                    .collect::<Vec<_>>();
            }
            if l.starts_with("hl.") || l.starts_with("local ") || l.starts_with("if ") {
                return Vec::new();
            }
            vec![l.to_string()]
        })
        .filter_map(|l| {
            l.split_once(" = ")
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .map(|(k, v)| {
            let v = v.trim().trim_end_matches(',').trim();
            let v = v
                .trim_matches('"')
                .trim_start_matches("[[")
                .trim_end_matches("]]");
            let v = match v {
                "on" => "true",
                "off" => "false",
                v => v,
            };
            (k.trim().rsplit(':').next().unwrap().to_string(), num(v))
        })
        .filter(|(k, v)| {
            !v.is_empty()
                && v != "{"
                && !k.is_empty()
                && k.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
                && !matches!(
                    k.as_str(),
                    "name"
                        | "enabled"
                        | "source"
                        | "plugin"
                        | "monitor"
                        | "env"
                        | "exec"
                        | "exec-once"
                )
        })
        .map(|(k, v)| (k.replace('-', "_"), v))
        .collect()
}

#[test]
fn lua_buttons_are_the_conf_buttons() {
    let conf: Vec<Vec<String>> = LIGHTS
        .lines()
        .filter_map(|l| l.trim().strip_prefix("hyprbars-button = "))
        .map(|l| l.split(',').map(|f| f.trim().to_string()).collect())
        .collect();
    let field = |l: &str, k: &str| -> String {
        let at = l.find(&format!("{k} = ")).unwrap() + k.len() + 3;
        let rest = &l[at..];
        rest[..rest.find([',', '}']).unwrap()]
            .trim()
            .trim_matches('"')
            .to_string()
    };
    let lua: Vec<Vec<String>> = LIGHTS_LUA
        .lines()
        .filter(|l| l.contains(".add_button("))
        .map(|l| {
            ["bg_color", "size", "icon", "action", "fg_color"]
                .map(|k| field(l, k))
                .to_vec()
        })
        .collect();
    assert_eq!(conf.len(), 3);
    assert_eq!(
        lua, conf,
        "lights.lua adds the buttons of lights.conf, in order"
    );
}

#[test]
fn lua_strip_settings_are_the_conf_ones() {
    let block = |t: &str, open: &str, close: &str| -> String {
        let a = t.find(open).unwrap_or_else(|| panic!("{open}"));
        t[a..a + t[a..].find(close).unwrap()].to_string()
    };
    let conf = leaves(&block(CONF, "hyprbars {", "}"), false);
    let lua = leaves(&block(LUA, "hyprbars = {", "}"), true);
    assert!(conf.len() >= 10, "{conf:?}");
    assert_eq!(lua, conf);
    // The theme keys are the same keywords (wol_core::theme::hypr), for both files.
    let theme_lua = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos/theme.lua");
    for l in THEME.lines().filter(|l| !l.starts_with('#')) {
        let (k, v) = l.split_once(" = ").unwrap();
        let want = format!("[\"{}\"] = \"{v}\"", k.replace(':', "."));
        assert!(theme_lua.contains(&want), "theme.lua lacks {want}");
    }
}

#[test]
fn lua_curves_and_animations_are_the_conf_ones() {
    let conf_bez: Vec<(String, Vec<f32>)> = CONF
        .lines()
        .filter_map(|l| l.trim().strip_prefix("bezier = "))
        .map(|l| {
            let mut f = l.split(',').map(str::trim);
            let n = f.next().unwrap().to_string();
            (n, f.map(|v| v.parse().unwrap()).collect())
        })
        .collect();
    let lua_bez: Vec<(String, Vec<f32>)> = LUA
        .lines()
        .filter_map(|l| l.trim().strip_prefix("hl.curve(\""))
        .map(|l| {
            let (n, rest) = l.split_once('"').unwrap();
            let pts: Vec<f32> = rest
                .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .filter(|v| !v.is_empty())
                .map(|v| v.parse().unwrap())
                .collect();
            (n.to_string(), pts)
        })
        .collect();
    assert_eq!(lua_bez, conf_bez);
    let conf_anim: Vec<Vec<String>> = CONF
        .lines()
        .filter_map(|l| l.trim().strip_prefix("animation = "))
        .map(|l| l.split(',').map(|f| f.trim().to_string()).collect())
        .collect();
    let field = |l: &str, k: &str| -> Option<String> {
        let at = l.find(&format!("{k} = "))? + k.len() + 3;
        let rest = &l[at..];
        Some(
            rest[..rest.find([',', '}']).unwrap()]
                .trim()
                .trim_matches('"')
                .to_string(),
        )
    };
    let lua_anim: Vec<Vec<String>> = LUA
        .lines()
        .filter(|l| l.trim().starts_with("hl.animation("))
        .map(|l| {
            let mut v = vec![
                field(l, "leaf").unwrap(),
                if field(l, "enabled").as_deref() == Some("true") {
                    "1".into()
                } else {
                    "0".into()
                },
                field(l, "speed").unwrap(),
                field(l, "bezier").unwrap(),
            ];
            v.extend(field(l, "style"));
            v
        })
        .collect();
    assert_eq!(lua_anim, conf_anim);
}

#[test]
fn lua_perf_defaults_and_greeters_are_the_conf_ones() {
    assert!(
        LUA.lines()
            .any(|l| l.trim() == "require(\"/usr/share/wolos/hypr/wolos-perf.lua\")")
    );
    assert!(
        LUA.lines()
            .any(|l| l.trim() == "require(\"/usr/share/wolos/hypr/permissions.lua\")")
    );
    for (conf, lua) in [
        (
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos-perf.conf"),
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/wolos-perf.lua"),
        ),
        (
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/login.conf"),
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/login.lua"),
        ),
        (
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/setup.conf"),
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/setup.lua"),
        ),
        (
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/installer.conf"),
            include_str!("../../../desktop/overlay/usr/share/wolos/hypr/installer.lua"),
        ),
    ] {
        let mut c = leaves(conf, false);
        let mut l = leaves(lua, true);
        c.sort();
        l.sort();
        assert_eq!(l, c, "Lua settings differ from the conf ({SHARE})");
    }
}

/// The session starts the same programs: every `exec-once` of hyprland.conf is a `once(…)` of
/// hyprland.lua and every `exec` an `hl.exec_cmd(…)`, but for the VM pointer line, which sets the
/// option through `hyprctl eval` (0.57 has no `keyword` command).
#[test]
fn lua_session_starts_the_conf_programs() {
    let cmd = |l: &str| l.split("  #").next().unwrap().trim().to_string();
    let lua_cmds: Vec<String> = LUA
        .lines()
        .map(|l| l.split(" -- ").next().unwrap().trim())
        .filter_map(|l| {
            let inner = l
                .strip_prefix("once(")
                .or_else(|| l.strip_prefix("hl.exec_cmd("))?
                .strip_suffix(')')?;
            let inner = inner
                .strip_prefix("[[")
                .and_then(|i| i.strip_suffix("]]"))
                .or_else(|| inner.strip_prefix('"').and_then(|i| i.strip_suffix('"')))?;
            Some(format!(
                "{}{inner}",
                if l.starts_with("once(") {
                    "once "
                } else {
                    "exec "
                }
            ))
        })
        .collect();
    let mut n = 0;
    for l in CONF.lines().map(str::trim) {
        let (kind, c) = if let Some(c) = l.strip_prefix("exec-once = ") {
            ("once ", cmd(c))
        } else if let Some(c) = l.strip_prefix("exec = ") {
            ("exec ", cmd(c))
        } else {
            continue;
        };
        let want = format!("{kind}{c}").replace(
            "hyprctl keyword cursor:no_hardware_cursors 1",
            "hyprctl eval \"hl.config({cursor = {no_hardware_cursors = 1}})\"",
        );
        assert!(
            lua_cmds.contains(&want),
            "hyprland.lua lacks `{want}`: {lua_cmds:#?}"
        );
        n += 1;
    }
    assert_eq!(n, lua_cmds.len(), "{lua_cmds:#?}");
}

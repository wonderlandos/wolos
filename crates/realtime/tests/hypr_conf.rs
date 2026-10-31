//! The desktop hyprland.conf drives hyprbars and window animations; it must agree with the
//! realtime geometry/curves and the kit (12 px lights, close centre 22.5 px in, 22 px apart).
//! Live 0.56.2: lights were green-yellow-red, 20 px apart, CSD apps got a second bar, and the
//! open animation used a 400 ms overshooting spring instead of our ease-out.
use realtime::anim::{Animation, Bezier};
use realtime::titlebar::{self, Button};

const CONF: &str = include_str!("../../../desktop/overlay/usr/share/wolos/hypr/hyprland.conf");

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
fn hyprbars_lights_match_kit_order_colour_and_spacing() {
    let buttons: Vec<Vec<String>> = CONF
        .lines()
        .filter_map(|l| l.trim().strip_prefix("hyprbars-button"))
        .map(|l| {
            l.trim_start_matches([' ', '='])
                .split(',')
                .map(|s| s.trim().to_owned())
                .collect()
        })
        .collect();
    // Kit light/dark colours; ±12 per channel (measure.py) must hold for both themes.
    let kit = [
        ("close", [(255, 59, 48), (255, 69, 58)]),
        ("minimize", [(255, 149, 0), (255, 159, 10)]),
        ("zoom", [(52, 199, 89), (48, 209, 88)]),
    ];
    assert_eq!(buttons.len(), 3);
    for (b, (action, themes)) in buttons.iter().zip(kit) {
        // The patched hyprbars runs these on the bar's own window (image/hyprland/hyprbars).
        assert_eq!(b[3], action, "order: {b:?} should be {action}");
        assert_eq!(b[1], titlebar::BUTTON.to_string());
        let c = rgb(&b[0]);
        for t in themes {
            assert!(
                (c.0 - t.0).abs() <= 12 && (c.1 - t.1).abs() <= 12 && (c.2 - t.2).abs() <= 12,
                "{c:?} vs {t:?}"
            );
        }
    }
    assert_eq!(value("bar_buttons_alignment"), "left");
    assert_eq!(value("bar_padding"), titlebar::INSET.to_string());
    assert_eq!(value("bar_button_padding"), titlebar::GAP.to_string());
    assert_eq!(value("bar_height"), titlebar::HEIGHT.to_string());
    let centre = |b| titlebar::button_origin(b).0 as f32 + titlebar::BUTTON as f32 / 2.0;
    assert!((centre(Button::Close) - 22.5).abs() <= 1.0);
    assert!((centre(Button::Minimize) - centre(Button::Close) - 22.0).abs() <= 1.0);
    assert_eq!(value("on_double_click"), "zoom");
    // macOS: hovering any light shows the glyphs on all three (titlebar::Hover).
    assert_eq!(value("icon_on_hover"), "true");
    for (b, glyph) in buttons.iter().zip(["×", "−", "+"]) {
        assert_eq!(b[2], glyph, "{b:?}");
        assert!(
            b.len() == 5 && b[4].starts_with("rgba("),
            "glyph colour: {b:?}"
        );
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
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../image/hyprland/hyprbars");
    let patches: String = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    let added = |s: &str| patches.lines().any(|l| l.starts_with('+') && l.contains(s));
    assert!(added(
        "m_bDragPending || m_bDraggingThis || m_bCancelledDown || m_pressedButton >= 0"
    ));
    assert!(added("\"hyprbars:buttons\""));
    assert!(added("d[\"closewindow\"](ADDR)"));
    // Live: hover changed only while rendering, so a pointer that stopped after one motion over a
    // light got no glyphs (a hardware cursor makes no frame). Input decides, rendering reads.
    assert!(added("void CHyprBar::sampleHover()") && added("wol_lights_hover(m_hover"));
    let removed = |s: &str| patches.lines().any(|l| l.starts_with('-') && l.contains(s));
    assert!(removed("m_iButtonHoverState ^= (1 << i);"));
    assert!(!added("m_iButtonHoverState") && !added("m_bButtonHovered"));
    let build = include_str!("../../../image/hyprland/build-plugins.sh");
    assert!(
        build.contains("hyprland/hyprbars/*.patch"),
        "build-plugins.sh applies the patches"
    );
}

#[test]
fn plugin_owns_floating_placement() {
    assert!(
        !CONF
            .lines()
            .any(|l| l.trim().starts_with("windowrule") && l.contains(", move "))
    );
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
        // Hyprland speed is in deciseconds.
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
        include_str!("../../../image/hyprland/build-plugins.sh"),
    ] {
        assert!(build.contains("-fno-access-control"));
    }
}

/// Live: two Hyprland banners on a fresh session ("started without start-hyprland",
/// ".conf ... removed in 0.57"). Customers must never see compositor nags.
#[test]
fn session_starts_hyprland_without_nag_banners() {
    let session = include_str!("../../../desktop/overlay/usr/bin/wol-session");
    let launch = session
        .lines()
        .rev()
        .find(|l| l.starts_with("exec "))
        .unwrap();
    // On the user bus inside a logind session, else a private bus: always through start-hyprland.
    assert_eq!(launch, "exec ${bus:+$bus --} start-hyprland");
    // hyprland-guiutils is not in the image: its "missing" banner must be off.
    assert_eq!(value("disable_hyprland_guiutils_check"), "true");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../image/hyprland/patches");
    let patches: String = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert!(
        patches.lines().any(|l| l.starts_with('-') && l.contains("TXT_KEY_NOTIF_OUTDATED_CONFIG), CHyprColor")),
        "fork patch must drop the .conf deprecation notification (0.56 pin keeps .conf)"
    );
}

/// Live (iGPU passthrough): the setup and login screens run Hyprland directly (wol-login restarts
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

/// Live: libadwaita drew Text Editor's minimise light disabled; Hyprland 0.56 never advertised
/// the minimise capability and ignored set_minimized.
#[test]
fn fork_makes_client_minimise_real() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../image/hyprland/patches");
    let patches: String = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    let added = |s: &str| patches.lines().any(|l| l.starts_with('+') && l.contains(s));
    assert!(added("XDG_TOPLEVEL_WM_CAPABILITIES_MINIMIZE"));
    assert!(added("requestsMinimize") && added("special:hidden"));
}

/// Live: `focuswindow` (switcher, menu actions) focused a floating window but left it behind
/// another; wol-hypr raises the window that becomes active.
#[test]
fn activation_raises_the_window() {
    let plugin = include_str!("../../../plugins/wol-hypr/main.cpp");
    let hook = &plugin[plugin.find("static void onActive").unwrap()..];
    let hook = &hook[..hook.find("\n}").unwrap()];
    assert!(hook.contains("alterzorder top"), "{hook}");
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

/// Window open, close and resize settle in about 0.2 s (2 in Hyprland's 100 ms units): longer
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

/// Live (VM, virtio-gpu): with damage tracking on, Hyprland never repainted the last frame of a
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

/// The keys must exist in the fork, or Hyprland reports a config error at login.
#[test]
fn fork_registers_the_frame_policy_keys() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../image/hyprland/patches");
    let patches: String = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    for k in [
        "misc:throttle_occluded",
        "misc:throttle_window_count",
        "misc:throttle_occlusion_cutoff",
        "misc:throttle_freeze_ms",
        "misc:throttle_freeze_updates",
        "misc:reconfigure_ack_timeout_ms",
        "misc:frame_deadline",
    ] {
        assert!(
            patches.contains("+        MS<") && patches.contains(&format!("\"{k}\"")),
            "{k}"
        );
    }
}

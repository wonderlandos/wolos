// SPDX-License-Identifier: GPL-2.0-or-later
//! Overlay files that only the running system reads, and that nothing parsed (QA-SHADOW-28, 29):
//! wold's admin job, the Inter fontconfig rule, the Plugin Maker's menu entry and the GTK 3 dark
//! stylesheet. A typo in any of them failed on a person's desktop, never in a build.
#![allow(dead_code)] // siblings.rs helpers this file does not call
#[path = "support/siblings.rs"]
mod siblings;

use std::fs;

fn read(p: &str) -> String {
    fs::read_to_string(siblings::root().join(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// QA-SHADOW-28: the admin job wold starts is a valid job spec for the program the image ships,
/// kept alive, with a QoS class that exists. (Its sibling `org.wol.login.json` is in units.rs.)
#[test]
fn the_admin_job_is_a_valid_spec_for_the_shipped_daemon() {
    let spec = read("desktop/overlay/etc/wolos/jobs/org.wol.admin.json");
    let job = shared::jobs::JobSpec::parse(&spec).unwrap();
    assert_eq!(job.label, "org.wol.admin", "the label is the file's name");
    assert_eq!(job.program, ["/usr/bin/wol-admind"]);
    assert!(job.keep_alive, "wol-admind is restarted when it dies");
    assert_eq!(job.user, None, "admind keeps wold's root identity");
    let qos = job.qos.as_deref().expect("a QoS class");
    assert!(
        shared::qos::QosClass::ALL.iter().any(|c| c.name() == qos),
        "{qos} is not a QoS class"
    );
    // The program is built and installed by the image.
    assert!(read("tools/build-rust.sh").contains("wol-admind"));
}

/// QA-SHADOW-29: the Inter rule is well-formed fontconfig and sends Inter, sans-serif and
/// system-ui to the variable font the image ships.
#[test]
fn the_inter_font_rule_is_wellformed_and_prefers_the_variable_font() {
    let x = read("desktop/overlay/etc/fonts/conf.d/60-wolos-inter.conf");
    assert!(x.starts_with("<?xml version=\"1.0\"?>"));
    assert!(x.contains("<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">"));
    for tag in ["fontconfig", "alias", "prefer", "family"] {
        let open = x.matches(&format!("<{tag}")).count();
        let close = x.matches(&format!("</{tag}>")).count();
        assert!(
            open > 0 && open == close,
            "<{tag}> opens {open}, closes {close}"
        );
    }
    for family in ["Inter", "sans-serif", "system-ui"] {
        let rule = x
            .lines()
            .find(|l| l.contains(&format!("<family>{family}</family>")))
            .unwrap_or_else(|| panic!("no rule for {family}"));
        assert!(rule.contains("<family>Inter Variable</family>"), "{rule}");
    }
}

/// QA-SHADOW-29: the Plugin Maker's menu entry starts a program the image installs, with its icon
/// in the design set, and is a valid entry for a launcher (Type, Name, Exec, Categories).
#[test]
fn the_plugin_maker_entry_starts_what_the_image_ships() {
    let d = read("desktop/overlay/usr/share/applications/org.wolos.plugin-maker.desktop");
    assert_eq!(d.lines().next(), Some("[Desktop Entry]"));
    let key = |k: &str| {
        d.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("no {k}="))
            .to_string()
    };
    assert_eq!(key("Type"), "Application");
    assert!(!key("Name").is_empty() && !key("Comment").is_empty());
    let exec = key("Exec");
    let program = exec.split_whitespace().next().unwrap();
    assert_eq!(program, "wol-plugin-maker");
    assert!(
        read("tools/build-rust.sh").contains(program),
        "{program} is installed"
    );
    assert!(key("Categories").ends_with(';'));
    let icon = key("Icon");
    assert!(
        siblings::root()
            .join(format!("design/icons/apps/{icon}.svg"))
            .exists(),
        "{icon} is in the design set"
    );
}

/// QA-SHADOW-29: libhandy apps in dark mode read the GTK 3 theme's `gtk-dark.css`; it is the same
/// stylesheet as `gtk.css` (the theme's colours are one set, `wol theme` writes both alike).
#[test]
fn the_gtk3_dark_stylesheet_is_the_themes_own() {
    let dir = "desktop/overlay/usr/share/wolos/gtk3-theme/gtk-3.0";
    let (light, dark) = (
        read(&format!("{dir}/gtk.css")),
        read(&format!("{dir}/gtk-dark.css")),
    );
    assert_eq!(light, dark);
    assert!(dark.contains("@define-color accent_bg_color"), "{dark}");
    assert!(dark.starts_with("@import"), "it builds on adw-gtk3");
}

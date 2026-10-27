//! One CssProvider per process. Loaded once; reloaded only when the active theme file or dark preference changes.
use crate::perf::Changed;
use crate::theme;
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;
use wol_core::theme::accent;

/// Installs the theme + `extra` CSS and watches `$XDG_CONFIG_HOME/wol/theme` (inotify/kqueue via GIO, no polling).
pub fn install(extra: &'static str) {
    let display = gdk::Display::default().expect("display");
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );
    icons(&display);
    let path = theme::active_path();
    let last = Rc::new(RefCell::new(Changed::<String>::new()));
    let load = {
        let (provider, path, last) = (provider.clone(), path.clone(), last.clone());
        move || {
            let id = std::fs::read_to_string(&path).ok();
            let id = id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from);
            // Auto: light by day, dark at night (wol_core::theme::accent).
            let dark = if accent::is_auto(id.as_deref()) {
                accent::auto_is_dark(super::now().hour)
            } else {
                gtk::Settings::default()
                    .map(|s| s.is_gtk_application_prefer_dark_theme())
                    .unwrap_or(false)
            };
            let t = theme::resolve(id.as_deref(), dark);
            let contrast = contrast_on();
            if last.borrow_mut().set(format!(
                "{}{}",
                t.id,
                if contrast { "+contrast" } else { "" }
            )) {
                let mut css = theme::stylesheet(&t, extra);
                if contrast {
                    css.push_str(CONTRAST_CSS);
                }
                provider.load_from_string(&css);
                if let Some(s) = gtk::Settings::default() {
                    s.set_gtk_application_prefer_dark_theme(t.variant == theme::Variant::Dark);
                }
            }
        }
    };
    load();
    auto_timer(Rc::new(load.clone()));
    for p in [path.clone(), contrast_path()] {
        if let Ok(m) = gio::File::for_path(&p)
            .monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let load = load.clone();
            m.connect_changed(move |_, _, _, _| load());
            // Keep the monitor alive for the process lifetime.
            std::mem::forget(m);
        }
    }
    let _ = glib::MainContext::default();
}

/// Re-evaluates the theme at each light/dark switch time (only changes anything in Auto).
fn auto_timer(load: Rc<dyn Fn()>) {
    let t = super::now();
    let secs = accent::secs_to_next_switch(t.hour, t.minute, t.second).max(1);
    glib::timeout_add_seconds_local_once(secs, move || {
        load();
        auto_timer(load);
    });
}

/// Writes the active theme id; every shell process reloads through its file monitor.
/// Third-party apps (GTK, Qt, foot) follow on a worker thread: files + gsettings (live dark mode).
pub fn set_active(id: &str) -> std::io::Result<()> {
    theme::save(id)?;
    let id = id.to_string();
    std::thread::spawn(move || {
        let home = wol_core::theme::config_home();
        if let Err(e) = wol_core::theme::apps::apply(&id, &home) {
            eprintln!("wol: app themes: {e}");
        }
    });
    Ok(())
}

/// Increase Contrast flag, relative to `$XDG_CONFIG_HOME`; every shell process reloads on change.
pub const CONTRAST_FILE: &str = "wol/contrast";
/// Token overrides appended to the kit stylesheet while the flag exists.
pub const CONTRAST_CSS: &str = include_str!("contrast.css");

fn contrast_path() -> std::path::PathBuf {
    std::path::Path::new(&super::config_home()).join(CONTRAST_FILE)
}

pub fn contrast_on() -> bool {
    contrast_path().exists()
}

pub fn set_contrast(on: bool) -> std::io::Result<()> {
    let p = contrast_path();
    if on {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, "")
    } else {
        match std::fs::remove_file(p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

pub fn active_id() -> Option<String> {
    theme::saved()
}

const PRODUCT_ICONS: &str = "WolOS";

/// The shell always draws the product icon theme (Adwaita in dev), never the host user's
/// (`~/.config/gtk-4.0/settings.ini`): same look everywhere and a large theme such as
/// breeze costs ~100 ms of startup.
fn icon_theme(dirs: &[&str], exists: impl Fn(&str) -> bool) -> &'static str {
    if dirs.iter().any(|d| exists(&format!("{d}/{PRODUCT_ICONS}"))) {
        PRODUCT_ICONS
    } else {
        "Adwaita"
    }
}

/// WolOS icons: installed share dir, else the repo's design/ tree (dev).
fn icons(display: &gdk::Display) {
    let theme = gtk::IconTheme::for_display(display);
    let dirs: Vec<String> = theme
        .search_path()
        .iter()
        .filter_map(|p| p.to_str().map(str::to_string))
        .collect();
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let name = icon_theme(&dirs, |p| {
        std::path::Path::new(p).join("index.theme").is_file()
    });
    gtk::Settings::for_display(display).set_gtk_icon_theme_name(Some(name));
    for p in [
        "/usr/local/share/icons/WolOS/scalable/apps",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/icons/apps"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/icons/symbolic"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/brand"),
    ] {
        if std::path::Path::new(p).is_dir() {
            theme.add_search_path(p);
        }
    }
}

thread_local! { static FIXED: std::cell::OnceCell<gtk::CssProvider> = const { std::cell::OnceCell::new() }; }

/// Loads one theme, no file watch (golden renders).
pub fn apply(t: &theme::Theme, extra: &str) {
    let display = gdk::Display::default().expect("display");
    FIXED.with(|c| {
        let p = c.get_or_init(|| {
            let p = gtk::CssProvider::new();
            gtk::style_context_add_provider_for_display(
                &display,
                &p,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
            icons(&display);
            p
        });
        p.load_from_string(&theme::stylesheet(t, extra));
    });
    if let Some(s) = gtk::Settings::default() {
        s.set_gtk_application_prefer_dark_theme(t.variant == theme::Variant::Dark);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_icon_theme_is_the_product_theme() {
        let dirs = |have: &'static [&'static str]| move |d: &str| have.contains(&d);
        assert_eq!(icon_theme(&["/a", "/b"], dirs(&["/b/WolOS"])), "WolOS");
        assert_eq!(icon_theme(&["/a"], dirs(&[])), "Adwaita");
    }
}

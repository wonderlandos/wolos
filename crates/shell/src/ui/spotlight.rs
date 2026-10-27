//! Spotlight: installed apps, Settings pages and file names under `$HOME`, ranked per keystroke
//! from prebuilt lowercase indexes.
use crate::files;
use crate::kit::{
    self,
    surface::{Keyboard, Spec},
    widgets as w,
};
use crate::search::{self, Item};
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Kind shown on a Settings result.
pub const SETTINGS: &str = "System Settings";
/// Results shown at most; apps and Settings keep at most [`APPS`] of them when files match.
pub const ROWS: usize = 8;
pub const APPS: usize = 5;

pub fn index(apps: Vec<crate::desktop::Entry>) -> Vec<Item> {
    // Pinned apps come from PINS below (WolOS name, icon, launcher), never twice.
    let mut v: Vec<Item> = apps
        .into_iter()
        .filter(|e| crate::apps::pin_for_entry(e, crate::apps::PINS).is_none())
        .map(|e| Item::new(&e.name, "Application", &e.icon, e.exec))
        .collect();
    for p in crate::apps::PINS {
        if !v.iter().any(|i| i.title == p.name) {
            v.push(Item::new(
                p.name,
                "Application",
                p.icon,
                p.exec.iter().map(|s| s.to_string()).collect(),
            ));
        }
    }
    // Every Settings page under the name Settings opens; a pane's search words find it too.
    let open = |t: &str| vec!["wol-settings".into(), t.into()];
    for p in crate::ui::settings::SIDEBAR.iter().flat_map(|g| g.iter()) {
        v.push(Item::new(p.title, SETTINGS, "wolos-settings", open(p.title)).with_words(p.words));
    }
    for t in crate::ui::settings::SUBPAGES {
        v.push(Item::new(t, SETTINGS, "wolos-settings", open(t)));
    }
    v
}

/// Seconds between two listings of `$HOME` by the shell (Spotlight also lists it when stale).
const RELIST_SECS: u32 = 3600;

/// wol-shell: lists `$HOME` for Spotlight at login and every hour, off the main thread.
pub fn keep_file_index() {
    let list = || {
        glib::spawn_future_local(async {
            let _ = gtk::gio::spawn_blocking(|| files::rebuild(&glib::home_dir())).await;
        });
    };
    list();
    glib::timeout_add_seconds_local(RELIST_SECS, move || {
        list();
        glib::ControlFlow::Continue
    });
}

/// Apps and Settings first, then file names, [`ROWS`] at most.
pub fn results(items: &[Item], files: &files::Index, query: &str) -> Vec<Item> {
    let mut top: Vec<Item> = search::search(items, query, ROWS)
        .into_iter()
        .cloned()
        .collect();
    let found = files.search(query, ROWS - top.len().min(APPS));
    top.truncate(ROWS - found.len());
    top.extend(found);
    top
}

pub fn no_results(query: &str) -> String {
    format!("No results for \u{201c}{query}\u{201d}")
}

/// The row under [`no_results`]: the browser searches for the query.
pub fn web_search(query: &str) -> Item {
    let browser = crate::apps::PINS.iter().find(|p| p.name == "Browser");
    let (icon, exec) = browser.map_or(("wolos-browser", "firefox"), |p| (p.icon, p.exec[0]));
    Item::new(
        &format!("Search the web for \u{201c}{query}\u{201d}"),
        "Browser",
        icon,
        vec![exec.into(), "--search".into(), query.into()],
    )
}

pub fn run() -> glib::ExitCode {
    super::run_panel_app(
        "org.wolos.spotlight",
        "spotlight",
        Spec::panel(
            "wol-panel-spotlight",
            [true, false, false, false],
            Keyboard::Exclusive,
        ),
        |win, _| {
            let items: Rc<RefCell<Vec<Item>>> = Default::default();
            let names: Rc<RefCell<files::Index>> = Default::default();
            let (c, render) = content(
                items.clone(),
                names.clone(),
                &std::env::var("WOL_SPOTLIGHT_QUERY").unwrap_or_default(),
            );
            let _ = win;
            // Index off the main thread (parallel .desktop parse); typing before it lands re-renders when ready.
            {
                let render = render.clone();
                glib::spawn_future_local(async move {
                    if let Ok(v) =
                        gtk::gio::spawn_blocking(|| index(crate::desktop::load_all())).await
                    {
                        *items.borrow_mut() = v;
                        render();
                    }
                });
            }
            // File names: the saved list at once, a fresh one after when it is old.
            glib::spawn_future_local(async move {
                let home = glib::home_dir();
                let h = home.clone();
                let Ok((idx, stale)) = gtk::gio::spawn_blocking(move || files::load(&h)).await
                else {
                    return;
                };
                *names.borrow_mut() = idx;
                render();
                if stale
                    && let Ok(idx) = gtk::gio::spawn_blocking(move || files::rebuild(&home)).await
                {
                    *names.borrow_mut() = idx;
                    render();
                }
            });
            c
        },
    )
}

/// Search field + ranked hits. Returns the widget and a re-render hook (index arrived).
/// A shown result: its row and what it launches.
type Hit = (gtk::Button, Vec<String>);

pub fn content(
    items: Rc<RefCell<Vec<Item>>>,
    names: Rc<RefCell<files::Index>>,
    query: &str,
) -> (gtk::Widget, Rc<dyn Fn()>) {
    let panel = w::glass_panel(0);
    panel.add_css_class("spotlight-panel");
    panel.set_width_request(w::SPOTLIGHT_WIDTH);
    panel.set_valign(gtk::Align::Start);
    panel.set_margin_top(w::PANEL_TOP);
    let bar = w::hbox(w::S3);
    bar.add_css_class("spotlight-bar");
    bar.append(&w::icon("system-search-symbolic", w::ICON_M));
    let entry = gtk::Text::builder()
        .placeholder_text("Spotlight Search")
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    bar.append(&entry);
    panel.append(&bar);
    let hits = w::vbox(w::S1);
    hits.add_css_class("spotlight-hits");
    hits.set_visible(false);
    panel.append(&hits);

    let shown: Rc<RefCell<Vec<Hit>>> = Default::default();
    let sel = Rc::new(Cell::new(0usize));
    let render: Rc<dyn Fn()> = {
        let (shown, hits, sel, entry) = (shown.clone(), hits.clone(), sel.clone(), entry.clone());
        Rc::new(move || {
            let t0 = std::time::Instant::now();
            while let Some(c) = hits.first_child() {
                hits.remove(&c);
            }
            let mut s = shown.borrow_mut();
            s.clear();
            sel.set(0);
            let q = entry.text();
            let mut found = results(&items.borrow(), &names.borrow(), &q);
            if found.is_empty() && !q.trim().is_empty() {
                let none = w::centered(&no_results(q.trim()), "secondary");
                none.add_css_class("spotlight-none");
                none.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                hits.append(&none);
                found.push(web_search(q.trim()));
            }
            for (i, it) in found.into_iter().enumerate() {
                let row = w::hbox(w::S3);
                row.append(&w::app_icon(&it.icon, w::ICON_M));
                let t = w::label(&it.title, "");
                t.set_hexpand(true);
                t.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                row.append(&t);
                row.append(&w::label(it.kind, "secondary"));
                let b = gtk::Button::builder().child(&row).build();
                b.add_css_class("hit");
                if i == 0 {
                    b.add_css_class("selected");
                }
                let exec = it.exec.clone();
                b.connect_clicked(move |b| {
                    kit::spawn(&exec);
                    if let Some(w) = b.root().and_downcast::<gtk::Window>() {
                        w.close();
                    }
                });
                hits.append(&b);
                s.push((b, it.exec));
            }
            hits.set_visible(!s.is_empty());
            // WOL_SHELL_TIMING: per-keystroke search + row build time (target < 16 ms).
            if std::env::var_os("WOL_SHELL_TIMING").is_some() {
                eprintln!("wol-spotlight: keystroke {:?}", t0.elapsed());
            }
        })
    };
    {
        let r = render.clone();
        entry.connect_changed(move |_| r());
    }
    {
        let (shown, sel) = (shown.clone(), sel.clone());
        entry.connect_activate(move |e| {
            if let Some((_, exec)) = shown.borrow().get(sel.get()) {
                kit::spawn(exec);
                if let Some(w) = e.root().and_downcast::<gtk::Window>() {
                    w.close();
                }
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    {
        let (shown, sel) = (shown.clone(), sel.clone());
        keys.connect_key_pressed(move |_, k, _, _| {
            let s = shown.borrow();
            if s.is_empty() {
                return glib::Propagation::Proceed;
            }
            let next = match k {
                gdk::Key::Down => (sel.get() + 1).min(s.len() - 1),
                gdk::Key::Up => sel.get().saturating_sub(1),
                _ => return glib::Propagation::Proceed,
            };
            s[sel.get()].0.remove_css_class("selected");
            s[next].0.add_css_class("selected");
            sel.set(next);
            glib::Propagation::Stop
        });
    }
    entry.add_controller(keys);
    entry.set_text(query);
    (panel.upcast(), render)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_has_settings_pages_and_pins() {
        let idx = index(vec![]);
        assert!(
            idx.iter().any(
                |i| i.title == "Users & Groups" && i.exec == ["wol-settings", "Users & Groups"]
            )
        );
        assert_eq!(search::search(&idx, "term", 1)[0].title, "Terminal");
    }

    /// "Menu Bar & Dock" and "Hardware" opened Appearance: every Settings result is a page
    /// Settings has, under the name it shows, and the pane's search words find it.
    #[test]
    fn every_settings_result_opens_its_pane() {
        use crate::ui::settings::{PAGES, SUBPAGES, canonical};
        let idx = index(vec![]);
        let rows: Vec<&Item> = idx.iter().filter(|i| i.kind == SETTINGS).collect();
        assert_eq!(rows.len(), PAGES.len() + SUBPAGES.len());
        for r in &rows {
            assert_eq!(r.exec, ["wol-settings", r.title.as_str()]);
            assert_eq!(canonical(&r.title), r.title, "an old name");
            assert!(PAGES.contains(&r.title.as_str()) || SUBPAGES.contains(&r.title.as_str()));
        }
        for (q, want) in [
            ("hardware", "Hardware"),
            ("menu bar", "Desktop & Dock"),
            ("dark", "Appearance"),
            ("wifi", "Wi-Fi"),
            ("storage", "Storage"),
        ] {
            assert_eq!(search::search(&idx, q, 1)[0].title, want, "{q}");
        }
    }

    /// Apps and Settings first, files fill the list; never more than [`ROWS`].
    #[test]
    fn files_fill_the_results_after_apps() {
        let idx = index(vec![]);
        let files = files::Index::parse(
            "Documents/Terminal notes.txt\nterm paper.pdf\n",
            std::path::Path::new("/home/alice"),
        );
        let got = results(&idx, &files, "term");
        assert_eq!(got[0].title, "Terminal");
        assert!(got.iter().any(|i| i.title == "term paper.pdf"));
        let many = files::Index::parse(
            &(0..20)
                .map(|i| format!("text {i}.txt\n"))
                .collect::<String>(),
            std::path::Path::new("/home/alice"),
        );
        let got = results(&idx, &many, "te");
        assert_eq!(got.len(), ROWS);
        assert!(got.iter().filter(|i| i.kind == "Text").count() >= ROWS - APPS);
        assert!(results(&idx, &files::Index::default(), "").is_empty());
    }

    /// Nothing found: say so, and offer the web.
    #[test]
    fn no_results_offers_the_web() {
        assert_eq!(no_results("qwzx"), "No results for \u{201c}qwzx\u{201d}");
        let web = web_search("tax forms");
        assert_eq!(web.exec, ["firefox", "--search", "tax forms"]);
        assert_eq!(web.title, "Search the web for \u{201c}tax forms\u{201d}");
    }

    #[test]
    fn pinned_apps_appear_once_under_their_wolos_name() {
        // Launchpad listed "Geary" (its .desktop) and "Mail" (the pin) for the same app.
        let e = |id: &str, name: &str, exec: &str| crate::desktop::Entry {
            id: id.into(),
            name: name.into(),
            exec: vec![exec.into()],
            icon: id.trim_end_matches(".desktop").into(),
        };
        let idx = index(vec![
            e("org.gnome.Geary.desktop", "Geary", "geary"),
            e("firefox.desktop", "Firefox", "/usr/local/bin/firefox"),
            e(
                "org.gnome.Calculator.desktop",
                "Calculator",
                "gnome-calculator",
            ),
        ]);
        let titles: Vec<&str> = idx.iter().map(|i| i.title.as_str()).collect();
        for (name, n) in [
            ("Mail", 1),
            ("Geary", 0),
            ("Browser", 1),
            ("Firefox", 0),
            ("Calculator", 1),
        ] {
            assert_eq!(titles.iter().filter(|t| **t == name).count(), n, "{name}");
        }
        let mail = idx.iter().find(|i| i.title == "Mail").unwrap();
        assert_eq!(
            (mail.icon.as_str(), mail.exec.as_slice()),
            ("wolos-mail", &["geary".to_string()][..])
        );
        let web = idx.iter().find(|i| i.title == "Browser").unwrap();
        assert_eq!(web.exec, ["firefox"]);
    }
}

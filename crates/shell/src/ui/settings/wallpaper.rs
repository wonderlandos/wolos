//! Wallpaper: Dynamic (day, dusk and night pictures following the clock), the shipped pictures and
//! your own photo. The choice is one journaled line (`wolos/wallpaper`, same file as
//! `wol wallpaper set`); the desktop follows it.
use super::*;
use kit::pickers::{DYNAMIC, DYNAMIC_CHOICE, shipped_wallpaper, wallpaper_choice};

/// Shipped pictures: (saved choice, file, name).
fn shipped() -> Vec<(String, PathBuf, String)> {
    DYNAMIC
        .iter()
        .filter_map(|n| {
            let f = shipped_wallpaper(n)?;
            let name = n
                .split('-')
                .map(|w| {
                    let mut c = w.chars();
                    c.next()
                        .map(|f| f.to_uppercase().chain(c).collect::<String>())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ");
            Some((f.display().to_string(), f, name))
        })
        .collect()
}

/// Saves `choice` (a path or `dynamic`) as one journaled change and tells the desktop.
fn choose(choice: &str, status: &gtk::Label) {
    let r = save_journaled(
        "wallpaper",
        shared::theme::WALLPAPER_FILE,
        Some(&format!("{choice}\n")),
    );
    say(status, if r.is_ok() { "" } else { SAVE_FAILED });
    reload_shell();
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Wallpaper");
    let status = status_label();
    let saved = wallpaper_choice();
    let pics = shipped();
    let tiles = w::wrap_row(w::S3, 4);
    let mut first: Option<gtk::ToggleButton> = None;
    let mut add = |t: &gtk::ToggleButton, choice: String| {
        if let Some(f) = &first {
            t.set_group(Some(f));
        } else {
            first = Some(t.clone());
        }
        let st = status.clone();
        t.connect_toggled(move |t| {
            if t.is_active() {
                choose(&choice, &st);
            }
        });
        tiles.append(t);
    };
    let strips: Vec<&std::path::Path> = pics.iter().map(|x| x.1.as_path()).collect();
    let dynamic =
        form::wallpaper_strips_tile(&strips, "Dynamic", saved.as_deref() == Some(DYNAMIC_CHOICE));
    add(&dynamic, DYNAMIC_CHOICE.into());
    let default = shipped_wallpaper(DYNAMIC[1]).map(|f| f.display().to_string());
    for (choice, file, name) in &pics {
        let on = match saved.as_deref() {
            Some(s) => s == choice,
            None => default.as_deref() == Some(choice),
        };
        add(&form::wallpaper_tile(Some(file), name, on), choice.clone());
    }
    let own = saved
        .as_deref()
        .filter(|s| *s != DYNAMIC_CHOICE && !pics.iter().any(|x| x.0 == *s))
        .map(PathBuf::from)
        .filter(|f| f.is_file());
    if let Some(f) = own {
        add(
            &form::wallpaper_tile(Some(&f), "Your Photo", true),
            f.display().to_string(),
        );
    }
    p.append(&w::section("", &[tiles.clone().upcast()]));
    if let Some(card) = tiles.parent() {
        card.add_css_class("preview-card");
    }
    p.append(&w::footnote(
        "Dynamic shows the day picture in daytime, dusk in the evening and night after dark.",
    ));
    let (st, page, env) = (status.clone(), p.downgrade(), Rc::downgrade(e));
    p.append(&w::section(
        "",
        &[form::button_row(
            "Your own photo",
            "Any picture on this computer.",
            "Add Photo…",
            move || {
                let (Some(page), st, env) = (page.upgrade(), st.clone(), env.clone()) else {
                    return;
                };
                kit::pickers::choose_picture(&page, move |f| {
                    choose(&f.display().to_string(), &st);
                    // The new photo shows as the chosen tile.
                    if let Some(e) = env.upgrade() {
                        e.refresh();
                    }
                });
            },
        )],
    ));
    p.append(&status);
    p
}

//! Appearance: Light, Dark or Auto with an accent colour (the WolOS themes), or any theme of the
//! catalogue in one click. Every pick is journaled (`wol undo`, Settings > Restore).
use super::*;
use wol_core::theme::ThemeCatalog;

/// The two WolOS themes: the Light / Dark / Auto tiles, not gallery cards.
const WOLOS_THEMES: [&str; 2] = ["wol-light", "wol-dark"];

/// Catalogue themes for the gallery (built-ins and installed ones), WolOS themes left out.
pub(crate) fn gallery(cat: &ThemeCatalog) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = cat
        .list()
        .into_iter()
        .filter(|s| !WOLOS_THEMES.contains(&s.id.as_str()) && !s.id.starts_with("wol-"))
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    v.sort_by_key(|a| a.1.to_lowercase());
    v
}

/// Saves `id` (None = follow light/dark) as one journaled change, then restyles every app.
fn choose(id: Option<&str>) {
    let _ = save_journaled("theme", shared::theme::ACTIVE_FILE, id);
    let _ = match id {
        Some(id) => kit::style::set_active(id),
        None => crate::theme::clear(),
    };
}

pub(super) fn page(_: &Rc<Env>) -> gtk::Box {
    let p = w::page("Appearance");
    let saved = kit::style::active_id();
    let (mode, acc) = accent::split(saved.as_deref());
    let custom = saved
        .as_deref()
        .filter(|id| !id.starts_with("wol-"))
        .map(str::to_string);
    let state = Rc::new(RefCell::new((mode, acc.to_string())));
    let light = crate::theme::resolve(Some("wol-light"), false);
    let dark = crate::theme::resolve(Some("wol-dark"), true);
    let tiles = w::wrap_row(w::S3, 3);
    let choices: [(Option<Variant>, &str, Vec<w::Palette>); 3] = [
        (Some(Variant::Light), "Light", vec![w::Palette::of(&light)]),
        (Some(Variant::Dark), "Dark", vec![w::Palette::of(&dark)]),
        (
            None,
            "Auto",
            vec![w::Palette::of(&light), w::Palette::of(&dark)],
        ),
    ];
    let accent_row = w::vbox(0);
    accent_row.set_visible(custom.is_none());
    let apply = |s: &(Option<Variant>, String)| choose(accent::theme_id(s.0, &s.1).as_deref());
    // One group for the tiles and the gallery: exactly one look is checked.
    let group: Rc<RefCell<Option<gtk::ToggleButton>>> = Default::default();
    let join = {
        let group = group.clone();
        move |t: &gtk::ToggleButton| {
            if let Some(f) = &*group.borrow() {
                t.set_group(Some(f));
            }
            if group.borrow().is_none() {
                *group.borrow_mut() = Some(t.clone());
            }
        }
    };
    for (v, title, slices) in choices {
        let t = w::theme_preview(slices, title, custom.is_none() && v == mode);
        join(&t);
        let (st, ar) = (state.clone(), accent_row.clone());
        t.connect_toggled(move |t| {
            if t.is_active() {
                st.borrow_mut().0 = v;
                ar.set_visible(true);
                apply(&st.borrow());
            }
        });
        tiles.append(&t);
    }
    p.append(&w::section("", &[tiles.clone().upcast()]));
    if let Some(card) = tiles.parent() {
        card.add_css_class("preview-card");
    }

    let swatches = w::wrap_row(w::S2, 8);
    let variant = if mode == Some(Variant::Dark) {
        Variant::Dark
    } else {
        Variant::Light
    };
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, name, ..) in accent::ACCENTS {
        let hex = accent::color(variant, id).unwrap_or_default();
        let c = Color::parse(hex).unwrap_or(Color::WHITE);
        let dot = Color::parse(accent::text_on(hex)).unwrap_or(Color::WHITE);
        let s = w::accent_swatch(c, dot, name, id == acc);
        if let Some(f) = &first {
            s.set_group(Some(f));
        } else {
            first = Some(s.clone());
        }
        let st = state.clone();
        s.connect_toggled(move |s| {
            if s.is_active() {
                st.borrow_mut().1 = id.to_string();
                apply(&st.borrow());
            }
        });
        swatches.append(&s);
    }
    accent_row.append(&w::section("", &[row("Accent color", &swatches)]));
    p.append(&accent_row);

    let cards = w::wrap_row(w::S3, 4);
    let cat = ThemeCatalog::load(&wol_core::theme::search_paths(
        &wol_core::theme::config_home(),
    ));
    for (id, name) in gallery(&cat) {
        let Ok(t) = cat.resolve(&id) else { continue };
        let card = w::theme_preview(
            vec![w::Palette::of(&t)],
            &name,
            custom.as_deref() == Some(id.as_str()),
        );
        card.set_tooltip_text(Some(&name));
        join(&card);
        let ar = accent_row.clone();
        card.connect_toggled(move |c| {
            if c.is_active() {
                ar.set_visible(false);
                choose(Some(&id));
            }
        });
        cards.append(&card);
    }
    p.append(&w::section("Themes", &[cards.clone().upcast()]));
    if let Some(card) = cards.parent() {
        card.add_css_class("preview-card");
    }
    p.append(&w::footnote(
        "A theme restyles the whole desktop and your apps at once. Accent colors apply to the WolOS Light and Dark looks.",
    ));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_lists_every_other_builtin_theme_by_name() {
        let g = gallery(&ThemeCatalog::builtin());
        assert_eq!(g.len(), 21, "23 built-ins minus WolOS Light and Dark");
        assert!(g.iter().all(|(id, _)| !id.starts_with("wol-")));
        let names: Vec<&str> = g.iter().map(|x| x.1.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted, "alphabetical");
    }
}

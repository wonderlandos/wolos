//! General > Language & Region: the language of menus and formats (desktop prefs `language`, the
//! session's `LANG` from the next login) and the keyboard's input sources live in Keyboard.
use super::*;
use wol_core::region::LANGUAGES;

/// Index in [`LANGUAGES`] for a saved locale (None or unknown = English (US)).
pub(crate) fn language_index(saved: Option<&str>) -> usize {
    saved
        .and_then(|l| LANGUAGES.iter().position(|x| x.0 == l))
        .unwrap_or(0)
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = general::subpage(e, "Language & Region");
    let status = status_label();
    let d = e.desk.get();
    let names: Vec<&str> = LANGUAGES.iter().map(|l| l.1).collect();
    let note = w::footnote("Apps use the new language the next time you log in.");
    note.set_visible(false);
    let (dk, st, n) = (e.desk.clone(), status.clone(), note.clone());
    p.append(&w::section(
        "",
        &[form::search_picker_row(
            "Preferred language",
            "Menus, dates and numbers.",
            &names,
            language_index(d.language.as_deref()),
            move |i| {
                dk.set(&st, |p| p.language = Some(LANGUAGES[i].0.to_string()));
                n.set_visible(true);
            },
        )],
    ));
    p.append(&note);
    let keyboard = form::nav_row(
        "input-keyboard-symbolic",
        "Input Sources",
        "Keyboard layouts you type with",
    );
    let open = e.opener("Keyboard");
    keyboard.connect_clicked(move |_| open());
    p.append(&w::section("", &[keyboard.upcast()]));
    p.append(&status);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_language_selects_its_row() {
        assert_eq!(language_index(None), 0);
        assert_eq!(language_index(Some("pt_BR.UTF-8")), 2);
        assert_eq!(language_index(Some("xx_XX.UTF-8")), 0);
    }
}

//! Widgets: what is on the desktop (widgets.json, the desktop surface follows the file), adding a
//! built-in widget at the first free spot, removing one, and Edit Widgets for arranging by hand.
use super::*;
use crate::ui::widgets_gallery::{Entry, builtin_icon};
use shared::widgets::grid::Grid;
use shared::widgets::{Builtin, Desktop, Kind, Options, Placed};

fn path() -> PathBuf {
    wol_core::widgets::desktop_path(&PathBuf::from(kit::config_home()))
}

/// Saves the layout as one journaled change (`wol undo`, Settings > Restore).
fn save(d: &Desktop) -> bool {
    save_journaled("widgets", shared::widgets::DESKTOP_FILE, Some(&d.to_json())).is_ok()
}

/// `kind` added at the first free cell of the default desktop grid, in its first size.
pub(crate) fn place(d: &mut Desktop, b: Builtin) -> Option<String> {
    let kind = Kind::Builtin(b);
    let size = *Entry::Builtin(b).sizes().first()?;
    let (col, row) =
        Grid::headless(kit::data::card_metrics()).free_spot(&d.widgets, None, size, (0, 0))?;
    let id = d.new_id(&kind);
    d.widgets.push(Placed {
        id: id.clone(),
        kind,
        size,
        col,
        row,
        options: Options::default(),
    });
    Some(id)
}

fn name_of(k: &Kind) -> String {
    match k {
        Kind::Builtin(b) => b.name().into(),
        Kind::Custom(id) | Kind::Unknown(id) => id.clone(),
    }
}

fn icon_of(k: &Kind) -> &'static str {
    match k {
        Kind::Builtin(b) => builtin_icon(*b),
        _ => "wolos-widgets-symbolic",
    }
}

fn fill(holder: &gtk::Box, status: &gtk::Label) {
    clear(holder);
    let (d, problem) = wol_core::widgets::load_desktop(&path());
    if problem.is_some() {
        say(
            status,
            "The widget layout could not be read. Adding a widget starts a new one.",
        );
    }
    let mut placed: Vec<gtk::Widget> = d
        .widgets
        .iter()
        .map(|p| {
            let (h, st, id) = (holder.clone(), status.clone(), p.id.clone());
            let remove = w::icon_button("list-remove-symbolic", "Remove", move || {
                let (mut d, _) = wol_core::widgets::load_desktop(&path());
                d.remove(&id);
                say(&st, if save(&d) { "" } else { SAVE_FAILED });
                fill(&h, &st);
            });
            w::list_row(
                icon_of(&p.kind),
                &name_of(&p.kind),
                p.size.label(),
                Some(remove.upcast_ref()),
            )
            .upcast()
        })
        .collect();
    if placed.is_empty() {
        placed.push(
            w::list_row(
                "wolos-widgets-symbolic",
                "No widgets on the desktop",
                "Add one below, or right-click the desktop.",
                None,
            )
            .upcast(),
        );
    }
    holder.append(&w::section("On the Desktop", &placed));
    let add: Vec<gtk::Widget> = Builtin::ALL
        .into_iter()
        .map(|b| {
            let (h, st) = (holder.clone(), status.clone());
            let button = w::button("Add", w::Kind::Default, move || {
                let (mut d, _) = wol_core::widgets::load_desktop(&path());
                match place(&mut d, b) {
                    Some(_) => say(&st, if save(&d) { "" } else { SAVE_FAILED }),
                    None => say(&st, "The desktop is full. Remove a widget first."),
                }
                fill(&h, &st);
            });
            w::list_row(
                builtin_icon(b),
                b.name(),
                b.description(),
                Some(button.upcast_ref()),
            )
            .upcast()
        })
        .collect();
    holder.append(&w::section("Add Widgets", &add));
}

pub(super) fn page(_: &Rc<Env>) -> gtk::Box {
    let p = w::page("Widgets");
    let status = status_label();
    p.append(&w::section(
        "",
        &[form::button_row(
            "Arrange widgets",
            "Move and resize them, or find more.",
            "Edit Widgets…",
            || kit::spawn(&argv(&["wol", "shell", "show", "widgets"])),
        )],
    ));
    let holder = w::vbox(w::S6);
    fill(&holder, &status);
    p.append(&holder);
    p.append(&status);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_widgets_take_free_cells() {
        let mut d = Desktop::default();
        let a = place(&mut d, Builtin::Clock).unwrap();
        let b = place(&mut d, Builtin::Weather).unwrap();
        assert_ne!(a, b);
        let (pa, pb) = (d.get(&a).unwrap(), d.get(&b).unwrap());
        assert_ne!(
            (pa.col, pa.row),
            (pb.col, pb.row),
            "never on top of each other"
        );
    }
}

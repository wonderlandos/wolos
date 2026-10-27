//! General > Login Items: apps that open when you log in (desktop prefs `login_items`, written to
//! the managed Hyprland include as `exec-once`; journaled).
use super::*;
use wol_core::hyprprefs::is_valid_command;

/// The command a desktop entry starts, when it is safe to run at login.
fn command(e: &crate::desktop::Entry) -> Option<String> {
    let c = e.exec.join(" ");
    is_valid_command(&c).then_some(c)
}

/// (icon, name) for a login command: the app whose command it is, else the command itself.
pub(crate) fn describe(entries: &[crate::desktop::Entry], cmd: &str) -> (String, String) {
    entries
        .iter()
        .find(|e| command(e).as_deref() == Some(cmd))
        .map(|e| (e.icon.clone(), e.name.clone()))
        .unwrap_or_else(|| (String::new(), cmd.to_string()))
}

fn fill(holder: &gtk::Box, e: &Rc<Env>, status: &gtk::Label) {
    clear(holder);
    let items = e.desk.get().login_items;
    let entries = crate::desktop::load_all();
    let mut rows: Vec<gtk::Widget> = items
        .iter()
        .map(|cmd| {
            let (icon, name) = describe(&entries, cmd);
            let (h, env, st, c) = (holder.clone(), e.clone(), status.clone(), cmd.clone());
            let remove = w::icon_button("list-remove-symbolic", "Remove", move || {
                env.desk.set(&st, |p| p.login_items.retain(|x| *x != c));
                fill(&h, &env, &st);
            });
            let sub = if name == *cmd { "" } else { cmd.as_str() };
            let r = w::list_row("", &name, sub, Some(remove.upcast_ref()));
            r.prepend(&w::app_icon(
                if icon.is_empty() {
                    w::PACKAGE_ICON
                } else {
                    &icon
                },
                form::ICON_ROW,
            ));
            r.upcast()
        })
        .collect();
    if rows.is_empty() {
        rows.push(w::list_row("", "No apps open at login", "", None).upcast());
    }
    let (h, env, st) = (holder.clone(), e.clone(), status.clone());
    let add = w::button("Add App…", w::Kind::Default, || {});
    add.connect_clicked(move |b| {
        let taken = env.desk.get().login_items;
        let mut apps: Vec<(String, String)> = crate::desktop::load_all()
            .iter()
            .filter_map(|x| Some((x.name.clone(), command(x)?)))
            .filter(|(_, c)| !taken.contains(c))
            .collect();
        apps.sort_by_key(|a| a.0.to_lowercase());
        apps.dedup_by(|a, b| a.1 == b.1);
        let items: Vec<w::MenuItem> = apps
            .iter()
            .map(|(name, c)| {
                let (h, env, st, c) = (h.clone(), env.clone(), st.clone(), c.clone());
                let act: Box<dyn Fn()> = Box::new(move || {
                    env.desk.set(&st, |p| p.login_items.push(c.clone()));
                    fill(&h, &env, &st);
                });
                (name.as_str(), act)
            })
            .collect();
        let m = w::menu(items);
        m.set_parent(b);
        m.popup();
    });
    rows.push(form::row("", "", &add));
    holder.append(&w::section("Open at Login", &rows));
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = general::subpage(e, "Login Items");
    let status = status_label();
    let holder = w::vbox(0);
    fill(&holder, e, &status);
    p.append(&holder);
    p.append(&w::footnote(
        "These apps open each time you log in. Removing one here does not uninstall it.",
    ));
    p.append(&status);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_commands_show_as_apps() {
        let entries = vec![crate::desktop::Entry {
            id: "org.wolos.Notes.desktop".into(),
            name: "Notes".into(),
            exec: vec!["wol-notes".into()],
            icon: "wolos-notes".into(),
        }];
        assert_eq!(
            describe(&entries, "wol-notes"),
            ("wolos-notes".into(), "Notes".into())
        );
        assert_eq!(
            describe(&entries, "foot --server"),
            (String::new(), "foot --server".into())
        );
    }
}

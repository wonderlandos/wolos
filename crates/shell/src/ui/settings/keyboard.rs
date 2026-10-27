//! Keyboard: key repeat, input sources (desktop prefs, applied live) and the way into Keyboard Shortcuts.
use super::*;

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let desk = &e.desk;
    let p = w::page("Keyboard");
    let status = status_label();
    let d = desk.get();
    let (d1, s1, d2, s2) = (desk.clone(), status.clone(), desk.clone(), status.clone());
    let shortcuts = form::nav_row(
        "",
        "Keyboard Shortcuts",
        "Change or add shortcuts for WolOS",
    );
    let env = e.clone();
    shortcuts.connect_clicked(move |_| {
        env.shortcut_group.set(None);
        env.open(shortcuts::TITLE);
    });
    p.append(&w::section(
        "",
        &[
            form::slider_row(
                "Key repeat rate",
                (5.0, 60.0, 5.0),
                d.repeat_rate.into(),
                ("Slow", "Fast"),
                move |v| d1.set(&s1, |p| p.repeat_rate = v as u32),
            ),
            // Negated so a shorter delay sits to the right, as on the Mac.
            form::slider_row(
                "Delay until repeat",
                (-1000.0, -150.0, 50.0),
                -f64::from(d.repeat_delay),
                ("Long", "Short"),
                move |v| d2.set(&s2, |p| p.repeat_delay = (-v) as u32),
            ),
            shortcuts.upcast(),
        ],
    ));
    p.append(&sources(desk, &status));
    let field = w::text_field("Type here to test");
    field.set_hexpand(true);
    p.append(&w::section("", &[form::row("Try it", "", &field)]));
    p.append(&status);
    p
}

fn layout_name(code: &str) -> &str {
    prefs::LAYOUTS
        .iter()
        .find(|l| l.0 == code)
        .map_or(code, |l| l.1)
}

/// Input Sources: one row per layout (first = default), Add lists the others.
fn sources(desk: &Rc<Desk>, status: &gtk::Label) -> gtk::Widget {
    let holder = w::vbox(w::S2);
    fn fill(holder: &gtk::Box, desk: &Rc<Desk>, status: &gtk::Label) {
        while let Some(c) = holder.first_child() {
            holder.remove(&c);
        }
        let layouts = desk.get().layouts;
        let mut rows: Vec<gtk::Widget> = layouts
            .iter()
            .enumerate()
            .map(|(i, code)| {
                let (h, dk, st, c) = (holder.clone(), desk.clone(), status.clone(), code.clone());
                let remove = w::icon_button("list-remove-symbolic", "Remove", move || {
                    dk.set(&st, |p| p.layouts.retain(|x| *x != c));
                    fill(&h, &dk, &st);
                });
                remove.set_sensitive(layouts.len() > 1);
                form::row(
                    layout_name(code),
                    if i == 0 { "Default" } else { "" },
                    &remove,
                )
            })
            .collect();
        let others: Vec<(&str, &str)> = prefs::LAYOUTS
            .iter()
            .filter(|l| !layouts.iter().any(|x| x == l.0))
            .copied()
            .collect();
        if !others.is_empty() {
            let (h, dk, st) = (holder.clone(), desk.clone(), status.clone());
            let add = w::button("Add Input Source…", w::Kind::Default, || {});
            add.connect_clicked(move |b| {
                let items: Vec<w::MenuItem> = others
                    .iter()
                    .map(|(code, name)| {
                        let (h, dk, st, code) = (h.clone(), dk.clone(), st.clone(), *code);
                        let act: Box<dyn Fn()> = Box::new(move || {
                            dk.set(&st, |p| p.layouts.push(code.to_string()));
                            fill(&h, &dk, &st);
                        });
                        (*name, act)
                    })
                    .collect();
                let m = w::menu(items);
                m.set_parent(b);
                m.popup();
            });
            rows.push(form::row("", "", &add));
        }
        holder.append(&w::section("Input Sources", &rows));
        if layouts.len() > 1 {
            holder.append(&w::footnote(
                "Press ⌃Space to switch to the next input source.",
            ));
        }
    }
    fill(&holder, desk, status);
    holder.upcast()
}

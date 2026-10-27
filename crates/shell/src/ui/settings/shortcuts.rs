//! Keyboard > Keyboard Shortcuts: every WolOS shortcut, grouped, read from the shipped hyprland.conf.
//! Click a shortcut to record new keys; a combo already in use offers a swap; each changed shortcut
//! resets on its own, Restore Defaults resets all. Custom shortcuts open an app or run a command.
use super::*;
use wol_core::shortcuts::{Combo, Custom, Group, Refused, Shortcut};

pub const TITLE: &str = "Keyboard Shortcuts";

/// Recorders by shortcut id, re-synced after every change (a swap moves two at once).
type Recorders = Rc<RefCell<Vec<(String, form::ShortcutRecorder)>>>;

/// What each modifier symbol means on PC keyboards (Super acts as Command), shown once on top.
pub const KEY_LEGEND: [(&str, &str); 4] = [
    ("⌘", "Command: the Windows or Super key"),
    ("⌥", "Option: Alt"),
    ("⌃", "Control: Ctrl"),
    ("⇧", "Shift"),
];

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let (ctx, keys) = (&e.ctx, &e.keys);
    let p = form::subpage(TITLE, e.opener("Keyboard"));
    p.append(&w::section("", &[kit::pickers::key_legend(&KEY_LEGEND)]));
    let status = status_label();
    let recorders: Recorders = Default::default();
    let groups = w::vbox(w::S6);
    let custom = w::vbox(0);
    let sync: Rc<dyn Fn()> = {
        let (keys, recorders) = (keys.clone(), recorders.clone());
        Rc::new(move || {
            let list = keys.list();
            for (id, r) in recorders.borrow().iter() {
                r.set(
                    list.iter()
                        .find(|s| &s.id == id)
                        .and_then(|s| s.combo.as_ref()),
                );
                r.set_conflict(false);
            }
        })
    };
    let list = keys.list();
    let mut sections: Vec<(Group, gtk::Box)> = Vec::new();
    for g in Group::ALL.into_iter().filter(|g| *g != Group::Custom) {
        let members: Vec<&Shortcut> = list.iter().filter(|s| s.group == g).collect();
        if members.is_empty() {
            continue;
        }
        let rows: Vec<gtk::Widget> = members
            .into_iter()
            .map(|s| shortcut_row(keys, s, &recorders, &sync, &status))
            .collect();
        let sec = w::section(g.title(), &rows);
        if g == Group::Tiling {
            sec.append(&w::footnote(
                "Active while Desktop & Dock > Tile windows is on.",
            ));
        }
        groups.append(&sec);
        sections.push((g, sec));
    }
    // Opened for one group (Desktop & Dock > Tiling shortcuts): only that group, and a way to all.
    let all = w::button("Show All Shortcuts", w::Kind::Plain, || {});
    all.set_halign(gtk::Align::Start);
    let filter: Rc<dyn Fn(Option<Group>)> = {
        let (sections, custom, all) = (sections.clone(), custom.clone(), all.clone());
        Rc::new(move |only: Option<Group>| {
            for (g, sec) in &sections {
                sec.set_visible(only.is_none_or(|o| o == *g));
            }
            custom.set_visible(only.is_none());
            all.set_visible(only.is_some());
        })
    };
    {
        let f = filter.clone();
        all.connect_clicked(move |_| f(None));
    }
    {
        let (env, f) = (e.clone(), filter.clone());
        p.connect_map(move |_| f(env.shortcut_group.take()));
    }
    filter(e.shortcut_group.get());
    p.append(&all);
    p.append(&groups);
    fill_custom(ctx, keys, &custom, &recorders, &sync, &status);
    p.append(&custom);
    let (k, st, sy, c, cu, rec) = (
        keys.clone(),
        status.clone(),
        sync.clone(),
        ctx.clone(),
        custom.clone(),
        recorders.clone(),
    );
    let restore = w::button("Restore Defaults", w::Kind::Default, move || {
        if k.edit(&st, |o| {
            o.restore_defaults();
            Ok(())
        })
        .is_ok()
        {
            sy();
            fill_custom(&c, &k, &cu, &rec, &sy, &st);
        }
    });
    restore.set_halign(gtk::Align::End);
    p.append(&restore);
    p.append(&status);
    p
}

/// One shortcut: name, recorder, a reset button while changed, and a swap offer on conflict.
fn shortcut_row(
    keys: &Rc<Keys>,
    s: &Shortcut,
    recorders: &Recorders,
    sync: &Rc<dyn Fn()>,
    status: &gtk::Label,
) -> gtk::Widget {
    let b = w::vbox(0);
    let id = s.id.clone();
    let notice_slot = w::vbox(0);
    let reset = w::icon_button("view-refresh-symbolic", "Reset to the default", || {});
    reset.set_visible(changed(s));
    let pause = {
        let k = keys.clone();
        move |on: bool| k.capture(on)
    };
    let record = {
        let (k, id, st, slot, sy, rs) = (
            keys.clone(),
            id.clone(),
            status.clone(),
            notice_slot.clone(),
            sync.clone(),
            reset.downgrade(),
        );
        move |combo: Option<Combo>| {
            clear(&slot);
            match k.edit(&st, |o| o.assign(&k.defaults, &id, combo.clone(), false)) {
                Ok(()) => {
                    sy();
                    if let Some(r) = rs.upgrade() {
                        r.set_visible(k.list().iter().any(|s| s.id == id && changed(s)));
                    }
                }
                Err(Refused::Conflict(_, other)) => {
                    let label = combo.as_ref().map(Combo::label).unwrap_or_default();
                    let (k2, id2, st2, slot2, sy2, c2, rs2) = (
                        k.clone(),
                        id.clone(),
                        st.clone(),
                        slot.clone(),
                        sy.clone(),
                        combo.clone(),
                        rs.clone(),
                    );
                    let swap = w::button("Swap", w::Kind::Default, move || {
                        clear(&slot2);
                        let _ = k2.edit(&st2, |o| o.assign(&k2.defaults, &id2, c2.clone(), true));
                        sy2();
                        if let Some(r) = rs2.upgrade() {
                            r.set_visible(k2.list().iter().any(|s| s.id == id2 && changed(s)));
                        }
                    });
                    let (slot3, sy3) = (slot.clone(), sy.clone());
                    let cancel = w::button("Cancel", w::Kind::Plain, move || {
                        clear(&slot3);
                        sy3();
                    });
                    slot.append(&form::notice(
                        &format!("{label} is used by “{other}”. Swap their shortcuts?"),
                        &[swap, cancel],
                    ));
                }
                Err(_) => sy(),
            }
        }
    };
    let r = form::ShortcutRecorder::new(s.combo.as_ref(), pause, record);
    r.set_editable(s.editable);
    {
        let (k, id, st, sy, slot) = (
            keys.clone(),
            id.clone(),
            status.clone(),
            sync.clone(),
            notice_slot.clone(),
        );
        reset.connect_clicked(move |b| {
            clear(&slot);
            if k.edit(&st, |o| {
                o.reset(&k.defaults, &id);
                Ok(())
            })
            .is_ok()
            {
                b.set_visible(false);
                sy();
            }
        });
    }
    let trailing = w::hbox(w::S1);
    trailing.append(&reset);
    trailing.append(&r.button);
    b.append(&form::row(&s.name, "", &trailing));
    b.append(&notice_slot);
    recorders.borrow_mut().push((id, r));
    b.upcast()
}

fn changed(s: &Shortcut) -> bool {
    match (&s.combo, &s.default) {
        (Some(a), Some(b)) => !a.same(b),
        (None, None) => false,
        _ => true,
    }
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// Custom shortcuts: name, command, recorder, remove; Add opens a sheet-free inline form.
fn fill_custom(
    ctx: &Rc<Ctx>,
    keys: &Rc<Keys>,
    holder: &gtk::Box,
    recorders: &Recorders,
    sync: &Rc<dyn Fn()>,
    status: &gtk::Label,
) {
    clear(holder);
    recorders
        .borrow_mut()
        .retain(|(id, _)| !id.starts_with("custom-"));
    let list = keys.list();
    let mut rows: Vec<gtk::Widget> = Vec::new();
    for s in list.iter().filter(|s| s.group == Group::Custom) {
        let row = w::vbox(0);
        let line = shortcut_row(keys, s, recorders, sync, status);
        row.append(&line);
        let (c, k, h, rec, sy, st, id) = (
            ctx.clone(),
            keys.clone(),
            holder.clone(),
            recorders.clone(),
            sync.clone(),
            status.clone(),
            s.id.clone(),
        );
        let remove = w::button("Remove", w::Kind::Plain, move || {
            if k.edit(&st, |o| {
                o.remove_custom(&id);
                Ok(())
            })
            .is_ok()
            {
                fill_custom(&c, &k, &h, &rec, &sy, &st);
            }
        });
        rows.push(row.upcast());
        rows.push(form::row("", &s.bind.args, &remove));
    }
    // Add: name + command, then the keys.
    let name = w::text_field("Name, e.g. Open Browser");
    let command = w::text_field("Command, e.g. firefox");
    name.set_hexpand(true);
    command.set_hexpand(true);
    let problem = status_label();
    let add = {
        let (c, k, h, rec, sy, st, name, command, problem) = (
            ctx.clone(),
            keys.clone(),
            holder.clone(),
            recorders.clone(),
            sync.clone(),
            status.clone(),
            name.clone(),
            command.clone(),
            problem.clone(),
        );
        move |combo: Option<Combo>| {
            let Some(combo) = combo else { return };
            let custom = Custom {
                name: name.text().trim().to_string(),
                combo,
                command: command.text().trim().to_string(),
            };
            match k.edit(&st, |o| o.add_custom(&k.defaults, custom)) {
                Ok(()) => fill_custom(&c, &k, &h, &rec, &sy, &st),
                Err(Refused::Conflict(_, other)) => say(
                    &problem,
                    &format!("Those keys are used by “{other}”. Choose other keys."),
                ),
                Err(_) => say(&problem, "Give the shortcut a name and a command to run."),
            }
        }
    };
    let k = keys.clone();
    let rec = form::ShortcutRecorder::new(None, move |on| k.capture(on), add);
    rows.push(form::row("Name", "", &name));
    rows.push(form::row("Command", "", &command));
    rows.push(form::row(
        "Keys",
        "Click, then press the new shortcut",
        &rec.button,
    ));
    holder.append(&w::section("Custom Shortcuts", &rows));
    holder.append(&problem);
}

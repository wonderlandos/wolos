//! Restore: recent settings changes with Undo (`wol_core::journal`, same as `wol undo`), and restore
//! points (ZFS boot environments) to start the system from an earlier state.
use super::*;
use wol_core::journal::{Entry, is_undone};

/// What a change was about, in words.
pub(crate) fn topic(what: &str) -> &str {
    match what {
        "theme" => "Appearance",
        "wallpaper" => "Wallpaper",
        "focus" => "Focus",
        "shell" => "Desktop & Dock",
        "desktop" => "Desktop, windows and effects",
        "keyboard" | "shortcuts" => "Keyboard shortcuts",
        "displays" => "Displays",
        "idle" => "Lock Screen",
        "helpers" => "Assistants & Automation",
        "files" => "Tidied Downloads",
        "widgets" => "Widgets",
        other => other,
    }
}

/// Who made it: "System Settings", "Control Center", "Terminal", "a script", "an assistant".
pub(crate) fn actor(by: &str) -> String {
    match by {
        "settings" => "System Settings".into(),
        "control-center" => "Control Center".into(),
        "cli" => "Terminal".into(),
        b if b.starts_with("script:") => format!("Script “{}”", &b[7..]),
        b if b.starts_with("mcp:") => format!("Assistant “{}”", &b[4..]),
        b => b.into(),
    }
}

/// "Just now", "5 min ago", "3 h ago", "2 days ago".
pub(crate) fn ago(now: u64, t: u64) -> String {
    match now.saturating_sub(t) {
        s if s < 60 => "Just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 2 * 86_400 => "Yesterday".into(),
        s => format!("{} days ago", s / 86_400),
    }
}

/// Changes worth listing: not undo entries themselves, not already undone; newest first, at most `n`.
pub(crate) fn listed(all: &[Entry], n: usize) -> Vec<&Entry> {
    all.iter()
        .filter(|e| e.undoes.is_none() && !is_undone(all, e.id))
        .take(n)
        .collect()
}

/// A restore point's name in words: the running system, one made before an update, or its own name.
pub(crate) fn point_name(b: &wol_core::update::be::BootEnv) -> String {
    if b.active {
        "Current system".into()
    } else if b.name.starts_with("wol-before-update-") {
        "Before an update".into()
    } else {
        b.name.clone()
    }
}

/// "20 October 2026, 18:40 · 412 MB" from `bectl`'s "2026-10-20 18:40" and "412M".
pub(crate) fn point_line(created: &str, space: &str) -> String {
    let when = (|| {
        let (date, time) = created.split_once(' ')?;
        let mut d = date.split('-');
        let (y, m, day) = (d.next()?, d.next()?.parse::<usize>().ok()?, d.next()?);
        let month = super::datetime::MONTHS.get(m.checked_sub(1)?)?;
        Some(format!(
            "{} {month} {y}, {time}",
            day.trim_start_matches('0')
        ))
    })()
    .unwrap_or_else(|| created.to_string());
    let size = match space.char_indices().last() {
        Some((i, u @ ('K' | 'M' | 'G' | 'T'))) => format!("{} {u}B", &space[..i]),
        _ => space.to_string(),
    };
    if size.is_empty() {
        when
    } else {
        format!("{when} · {size}")
    }
}

fn history(e: &Rc<Env>, into: &gtk::Box, status: &gtk::Label) {
    clear(into);
    let all = e.sys().history();
    let now = wol_core::journal::now();
    let rows: Vec<gtk::Widget> = listed(&all, 8)
        .into_iter()
        .map(|x| {
            let (env, h, st, id) = (e.clone(), into.clone(), status.clone(), x.id);
            let undo = w::button("Undo", w::Kind::Default, move || {
                match env.sys().undo(Some(id)) {
                    Ok(_) => say(&st, ""),
                    Err(m) => say(&st, &m),
                }
                history(&env, &h, &st);
            });
            w::list_row(
                "",
                topic(&x.what),
                &format!("{} · {}", actor(&x.by), ago(now, x.t)),
                Some(undo.upcast_ref()),
            )
            .upcast()
        })
        .collect();
    if rows.is_empty() {
        into.append(&w::section(
            "Recent Changes",
            &[w::list_row(
                "wolos-restore-symbolic",
                "No changes to undo",
                "Changes to settings show here, so you can take any of them back.",
                None,
            )
            .upcast()],
        ));
    } else {
        into.append(&w::section("Recent Changes", &rows));
    }
}

fn points(e: &Rc<Env>, into: &gtk::Box, status: &gtk::Label) {
    clear(into);
    let mut bes = e.sys().boot_environments();
    if bes.is_empty() {
        return;
    }
    bes.sort_by(|a, b| b.created.cmp(&a.created));
    let rows: Vec<gtk::Widget> = bes
        .iter()
        .map(|b| {
            let sub = point_line(&b.created, &b.space);
            let trailing: gtk::Widget = if b.active {
                w::label("Running now", "secondary").upcast()
            } else if b.next_boot {
                w::label("Starts next time", "secondary").upcast()
            } else {
                let (env, h, st, name) = (e.clone(), into.clone(), status.clone(), b.name.clone());
                w::button("Start From This", w::Kind::Default, move || {
                    let (env2, h2, st2) = (env.clone(), h.clone(), st.clone());
                    env.ctx.act(
                        Action::BootEnvironment(name.clone()),
                        Rc::new(move |r| match r {
                            Ok(()) => {
                                say(&st2, "Restart to use this restore point.");
                                points(&env2, &h2, &st2);
                            }
                            Err(x) => say(&st2, x.message()),
                        }),
                    );
                })
                .upcast()
            };
            w::list_row(
                "drive-harddisk-symbolic",
                &point_name(b),
                &sub,
                Some(&trailing),
            )
            .upcast()
        })
        .collect();
    into.append(&w::section("Restore Points", &rows));
    into.append(&w::footnote(
        "A restore point is made before every update. Starting from one keeps your files and brings back the system as it was.",
    ));
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Restore");
    let status = status_label();
    let changes = w::vbox(0);
    history(e, &changes, &status);
    p.append(&changes);
    let bes = w::vbox(w::S6);
    points(e, &bes, &status);
    p.append(&bes);
    p.append(&status);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_read_as_sentences() {
        assert_eq!(topic("theme"), "Appearance");
        assert_eq!(actor("mcp:claude"), "Assistant “claude”");
        assert_eq!(actor("script:tidy"), "Script “tidy”");
        assert_eq!(ago(1000, 990), "Just now");
        assert_eq!(ago(10_000, 10_000 - 300), "5 min ago");
        assert_eq!(ago(100_000, 100_000 - 7200), "2 h ago");
        assert_eq!(ago(1_000_000, 1_000_000 - 3 * 86_400), "3 days ago");
        let e = |id, undoes| Entry {
            id,
            t: 0,
            by: "settings".into(),
            what: "theme".into(),
            changes: vec![],
            undoes,
        };
        let all = vec![e(3, Some(1)), e(2, None), e(1, None)];
        let ids: Vec<u64> = listed(&all, 8).iter().map(|x| x.id).collect();
        assert_eq!(
            ids,
            [2],
            "undo entries and undone changes are not offered again"
        );
    }

    #[test]
    fn restore_points_read_as_words() {
        let be = |name: &str, active| wol_core::update::be::BootEnv {
            name: name.into(),
            active,
            next_boot: active,
            mountpoint: None,
            space: "412M".into(),
            created: "2026-10-02 18:40".into(),
        };
        assert_eq!(point_name(&be("default", true)), "Current system");
        assert_eq!(
            point_name(&be("wol-before-update-1792", false)),
            "Before an update"
        );
        assert_eq!(point_name(&be("my-test", false)), "my-test");
        assert_eq!(
            point_line("2026-10-02 18:40", "412M"),
            "2 October 2026, 18:40 · 412 MB"
        );
        assert_eq!(point_line("odd", "9.8G"), "odd · 9.8 GB");
        assert_eq!(point_line("odd", ""), "odd");
    }
}

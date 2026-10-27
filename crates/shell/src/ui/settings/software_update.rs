//! Software Update: what the daily check found (FreeBSD's fixes, package updates, the ones that
//! fix security problems), Check Now, and Update Now into a new boot environment.
use super::*;
use wol_core::update::check::Updates;

// ---- Software Update ----

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Vec<String>),
    Installing,
    Installed,
    Failed(String),
}

/// `wol update check --json` stdout -> state.
pub fn parse_check(out: Option<&str>) -> UpdateState {
    match out.and_then(|o| serde_json::from_str::<Vec<String>>(o.trim()).ok()) {
        Some(v) if v.is_empty() => UpdateState::UpToDate,
        Some(v) => UpdateState::Available(v),
        None => UpdateState::Failed(crate::copy::UPDATE_CHECK.into()),
    }
}

/// The daily check's findings -> the state the page opens in: never checked asks for a check.
pub fn from_snapshot(u: Option<&Updates>) -> UpdateState {
    match u {
        None => UpdateState::Idle,
        Some(u) if u.count() == 0 => UpdateState::UpToDate,
        Some(u) => UpdateState::Available(
            u.base
                .iter()
                .map(|b| format!("FreeBSD {b}"))
                .chain(u.packages.iter().cloned())
                .collect(),
        ),
    }
}

/// "FreeBSD 15.1-RELEASE-p3, firefox and curl", at most `max` names then "and 3 more" (the title
/// already says how many).
pub fn names_line(names: &[String], max: usize) -> String {
    let shown: Vec<&str> = names.iter().take(max).map(String::as_str).collect();
    match (names.len().saturating_sub(max), shown.as_slice()) {
        (0, [one]) => (*one).to_string(),
        (0, [rest @ .., last]) => format!("{} and {last}", rest.join(", ")),
        (more, _) => format!("{} and {more} more", shown.join(", ")),
    }
}

/// "1 update fixes a security problem. Last checked 3 h ago."
pub fn snapshot_line(u: &Updates, now: u64) -> String {
    let fixes = match u.security.len() {
        0 => String::new(),
        1 => "1 update fixes a security problem. ".into(),
        n => format!("{n} updates fix security problems. "),
    };
    let when = super::restore::ago(now, u.checked).to_lowercase();
    format!("{fixes}Last checked {when}.")
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let ctx = &e.ctx;
    let p = general::subpage(e, "Software Update");
    let icon = w::icon("software-update-available-symbolic", w::ICON_M);
    icon.add_css_class("accent-icon");
    let title = w::label("", "title");
    title.set_wrap(true);
    let detail = w::label("", "secondary");
    detail.set_wrap(true);
    let text = w::vbox(0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&title);
    text.append(&detail);
    let spin = w::spinner();
    spin.set_spinning(false);
    spin.set_vexpand(false);
    let head = w::hbox(w::S3);
    head.add_css_class("list-row");
    head.add_css_class("hero-row");
    head.append(&icon);
    head.append(&text);
    head.append(&spin);
    let bar = w::progress();
    let progress_row = w::hbox(0);
    progress_row.add_css_class("list-row");
    progress_row.append(&bar);
    let names = w::label("", "secondary");
    names.set_wrap(true);
    names.set_wrap_mode(gtk::pango::WrapMode::Word);
    names.set_hexpand(true);
    let names_row = w::hbox(0);
    names_row.add_css_class("list-row");
    names_row.append(&names);
    p.append(&w::section(
        "",
        &[
            head.upcast(),
            progress_row.clone().upcast(),
            names_row.clone().upcast(),
        ],
    ));
    let buttons = w::hbox(w::S2);
    buttons.append(&w::spacer());
    let check = w::button("Check Now", w::Kind::Default, || {});
    let install = w::button("Update Now", w::Kind::Primary, || {});
    buttons.append(&check);
    buttons.append(&install);
    p.append(&buttons);

    // What the daily check adds under the title (security fixes, when it ran); a fresh Check Now
    // clears it.
    let note: Rc<RefCell<Option<String>>> = Default::default();
    let show: Rc<dyn Fn(&UpdateState)> = {
        let (title, detail, spin, progress_row, names_row, names, check, install, bar) = (
            title.clone(),
            detail.clone(),
            spin.clone(),
            progress_row.clone(),
            names_row.clone(),
            names.clone(),
            check.clone(),
            install.clone(),
            bar.clone(),
        );
        let pulse: Rc<RefCell<Option<glib::SourceId>>> = Default::default();
        let note = note.clone();
        Rc::new(move |s: &UpdateState| {
            let busy = matches!(s, UpdateState::Checking | UpdateState::Installing);
            spin.set_spinning(busy);
            spin.set_visible(busy);
            progress_row.set_visible(*s == UpdateState::Installing);
            if let Some(id) = pulse.borrow_mut().take() {
                id.remove();
            }
            if *s == UpdateState::Installing {
                let b = bar.clone();
                *pulse.borrow_mut() = Some(glib::timeout_add_local(
                    std::time::Duration::from_millis(120),
                    move || {
                        b.pulse();
                        glib::ControlFlow::Continue
                    },
                ));
            }
            check.set_sensitive(!busy);
            install.set_visible(matches!(s, UpdateState::Available(_)));
            names_row.set_visible(matches!(s, UpdateState::Available(_)));
            let (t, d) = match s {
                UpdateState::Idle => (
                    "Software Update".to_string(),
                    "Check for new WolOS packages.".to_string(),
                ),
                UpdateState::Checking => ("Checking for updates…".into(), String::new()),
                UpdateState::UpToDate => (
                    "WolOS is up to date".into(),
                    match note.borrow().as_deref() {
                        Some(n) => format!("All packages are current. {n}"),
                        None => "All packages are current.".into(),
                    },
                ),
                UpdateState::Available(v) => {
                    names.set_label(&names_line(v, 6));
                    (
                        format!(
                            "{} update{} available",
                            v.len(),
                            if v.len() == 1 { "" } else { "s" }
                        ),
                        match note.borrow().as_deref() {
                            Some(n) => format!("{n} WolOS saves a restore point first."),
                            None => "WolOS saves a restore point before updating.".into(),
                        },
                    )
                }
                UpdateState::Installing => (
                    "Installing updates…".into(),
                    "You can keep working while this runs.".into(),
                ),
                UpdateState::Installed => (
                    "Updates installed".into(),
                    "Restart to finish updating.".into(),
                ),
                UpdateState::Failed(e) => ("Software Update".into(), e.clone()),
            };
            title.set_label(&t);
            detail.set_label(&d);
            detail.set_visible(!d.is_empty());
        })
    };
    show(&UpdateState::Idle);
    {
        let (show, note, m) = (show.clone(), note.clone(), e.machine());
        off_thread(
            move || m.updates(),
            move |u| {
                *note.borrow_mut() = u
                    .as_ref()
                    .map(|u| snapshot_line(u, wol_core::journal::now()));
                show(&from_snapshot(u.as_ref()));
            },
        );
    }
    {
        let (show, note) = (show.clone(), note.clone());
        check.connect_clicked(move |_| {
            *note.borrow_mut() = None;
            show(&UpdateState::Checking);
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let out = wol_core::system::run_with_timeout(
                    &["wol", "update", "check", "--json"],
                    std::time::Duration::from_secs(120),
                );
                let _ = tx.send_blocking(out);
            });
            let show = show.clone();
            glib::spawn_future_local(async move {
                if let Ok(out) = rx.recv().await {
                    show(&parse_check(out.as_deref()));
                }
            });
        });
    }
    {
        let (show, ctx) = (show.clone(), ctx.clone());
        install.connect_clicked(move |_| {
            show(&UpdateState::Installing);
            let show = show.clone();
            ctx.act(
                Action::PkgUpgrade,
                Rc::new(move |r| {
                    show(&match r {
                        Ok(()) => UpdateState::Installed,
                        Err(e) => UpdateState::Failed(crate::copy::update(e).into()),
                    })
                }),
            );
        });
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_check_states() {
        assert_eq!(parse_check(Some("[]\n")), UpdateState::UpToDate);
        assert_eq!(
            parse_check(Some(r#"["curl","git"]"#)),
            UpdateState::Available(vec!["curl".into(), "git".into()])
        );
        assert!(matches!(parse_check(None), UpdateState::Failed(_)));
    }

    #[test]
    fn update_names_read_as_a_list() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(names_line(&n(&["curl"]), 6), "curl");
        assert_eq!(names_line(&n(&["a", "b", "c"]), 6), "a, b and c");
        assert_eq!(names_line(&n(&["a", "b", "c", "d"]), 2), "a, b and 2 more");
    }

    #[test]
    fn the_daily_check_opens_the_page() {
        assert_eq!(from_snapshot(None), UpdateState::Idle);
        let mut u = Updates {
            checked: 10_000,
            ..Default::default()
        };
        assert_eq!(from_snapshot(Some(&u)), UpdateState::UpToDate);
        assert_eq!(
            snapshot_line(&u, 10_000 + 3 * 3600),
            "Last checked 3 h ago."
        );
        u.packages = vec!["curl".into(), "git".into()];
        u.security = vec!["curl".into()];
        u.base = Some("15.1-RELEASE-p3".into());
        assert_eq!(
            from_snapshot(Some(&u)),
            UpdateState::Available(vec![
                "FreeBSD 15.1-RELEASE-p3".into(),
                "curl".into(),
                "git".into()
            ])
        );
        assert_eq!(
            snapshot_line(&u, 10_300),
            "1 update fixes a security problem. Last checked 5 min ago."
        );
    }
}

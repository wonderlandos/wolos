//! Users & Groups: the people who log in (name, full name, login shell) and the Add User sheet.
use super::*;

/// "Alice Liddell" -> "alice".
pub fn account_name(full: &str) -> String {
    let first = full.split_whitespace().next().unwrap_or("");
    let s: String = first
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .take(32)
        .collect();
    s.trim_start_matches(|c: char| c.is_ascii_digit())
        .to_string()
}

/// One login user from passwd: (login, full name, shell name).
pub(crate) fn people(passwd: &str, logins: &[String]) -> Vec<(String, String, String)> {
    logins
        .iter()
        .map(|u| {
            let f: Vec<&str> = passwd
                .lines()
                .find(|l| l.split(':').next() == Some(u.as_str()))
                .map(|l| l.split(':').collect())
                .unwrap_or_default();
            let full = f.get(4).map_or("", |g| g.split(',').next().unwrap_or(""));
            let shell = f.last().and_then(|s| s.rsplit('/').next()).unwrap_or("");
            (u.clone(), full.replace('&', u), shell.to_string())
        })
        .collect()
}

fn person_row(login: &str, full: &str, shell: &str) -> gtk::Widget {
    let sub = match (full.is_empty(), shell.is_empty()) {
        (false, false) => format!("{login} · {shell}"),
        (false, true) => login.to_string(),
        (true, false) => shell.to_string(),
        (true, true) => String::new(),
    };
    let r = w::list_row("", if full.is_empty() { login } else { full }, &sub, None);
    r.prepend(&w::avatar(login, w::ICON_M));
    r.upcast()
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let (ctx, host, sheet) = (&e.ctx, &e.host, &e.sheets.user);
    let p = w::page("Users & Groups");
    let status = status_label();
    // The Users card: one row per person, rebuilt when someone is added.
    let list = w::vbox(0);
    type People = Rc<RefCell<Vec<(String, String, String)>>>;
    let known: People = Default::default();
    let refill: Rc<dyn Fn()> = {
        let (list, known) = (list.clone(), known.clone());
        Rc::new(move || {
            clear(&list);
            let rows: Vec<gtk::Widget> = known
                .borrow()
                .iter()
                .map(|(u, f, sh)| person_row(u, f, sh))
                .collect();
            if rows.is_empty() {
                list.append(&w::section(
                    "Users",
                    &[
                        w::list_row("system-users-symbolic", "No other accounts", "", None)
                            .upcast(),
                    ],
                ));
            } else {
                list.append(&w::section("Users", &rows));
            }
        })
    };
    refill();
    p.append(&list);
    {
        let (m, users, known, refill) = (
            e.machine(),
            host.users.clone(),
            known.clone(),
            refill.clone(),
        );
        off_thread(
            move || people(&m.passwd(), &users),
            move |v| {
                *known.borrow_mut() = v;
                refill();
            },
        );
    }

    let full = w::text_field("Full name");
    let short = w::text_field("Account name");
    let pass = w::password_field("Password");
    let verify = w::password_field("Verify");
    let admin = w::toggle(false, |_| {});
    {
        let s = short.clone();
        full.connect_changed(move |f| s.set_text(&account_name(&f.text())));
    }
    let body = w::vbox(w::S3);
    body.append(&w::label("New account", "title"));
    body.append(&full);
    body.append(&short);
    body.append(&pass);
    body.append(&verify);
    body.append(&w::list_row(
        "",
        "Allow this user to administer this computer",
        "",
        Some(admin.upcast_ref()),
    ));
    let buttons = w::hbox(w::S2);
    buttons.append(&w::spacer());
    {
        let r = sheet.clone();
        buttons.append(&w::button("Cancel", w::Kind::Default, move || {
            r.set_reveal_child(false)
        }));
    }
    {
        let (pass, verify) = (pass.clone(), verify.clone());
        let (ctx, r, full, short, admin, st, known, refill) = (
            ctx.clone(),
            sheet.clone(),
            full.clone(),
            short.clone(),
            admin.clone(),
            status.clone(),
            known.clone(),
            refill.clone(),
        );
        buttons.append(&w::button("Create User", w::Kind::Primary, move || {
            let (name, full_name) = (short.text().to_string(), full.text().to_string());
            if !wol_core::settings::users::is_valid_name(&name) {
                return say(&st, "Account names use lowercase letters, digits, - and _.");
            }
            let password = pass.text().to_string();
            if let Some(e) = password_problem(&password, &verify.text()) {
                return say(&st, e);
            }
            pass.set_text("");
            verify.set_text("");
            r.set_reveal_child(false);
            say(&st, &format!("Creating “{name}”…"));
            let (st, known, refill, n2, f2) = (
                st.clone(),
                known.clone(),
                refill.clone(),
                name.clone(),
                full_name.clone(),
            );
            ctx.act(
                Action::UserAdd {
                    name,
                    full_name,
                    admin: admin.is_active(),
                    password,
                },
                Rc::new(move |res| match res {
                    Ok(()) => {
                        say(&st, "");
                        known
                            .borrow_mut()
                            .push((n2.clone(), f2.clone(), "zsh".into()));
                        refill();
                    }
                    Err(e) => say(&st, e.message()),
                }),
            );
        }));
    }
    body.append(&buttons);
    sheet_body(sheet, &body);
    let bar = w::hbox(w::S2);
    bar.append(&w::spacer());
    {
        let (r, full) = (sheet.clone(), full.clone());
        bar.append(&w::button("Add User…", w::Kind::Default, move || {
            r.set_reveal_child(true);
            full.grab_focus();
        }));
    }
    p.append(&bar);
    p.append(&w::footnote(
        "New accounts get a private home folder and use zsh in Terminal.",
    ));
    p.append(&status);
    p
}

/// Inline error for the Add User password fields, None when acceptable.
pub fn password_problem(password: &str, verify: &str) -> Option<&'static str> {
    if !wol_core::settings::users::is_valid_password(password) {
        Some("Use a password of at least 8 characters.")
    } else if password != verify {
        Some("The passwords do not match.")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_names_from_full_names() {
        for (full, want) in [
            ("Alice Liddell", "alice"),
            ("Mad Hatter", "mad"),
            ("  Dinah ", "dinah"),
            ("José", "jos"),
            ("42 Wonder", ""),
        ] {
            assert_eq!(account_name(full), want, "{full}");
        }
    }

    #[test]
    fn people_from_passwd() {
        let passwd = "alice:*:1001:1001:Alice Liddell,,,:/home/alice:/usr/local/bin/zsh\n\
                      bill:*:1002:1002:& the Lizard:/home/bill:/bin/sh\n";
        assert_eq!(
            people(passwd, &["alice".into(), "bill".into(), "ghost".into()]),
            [
                ("alice".into(), "Alice Liddell".into(), "zsh".into()),
                ("bill".into(), "bill the Lizard".into(), "sh".into()),
                ("ghost".into(), String::new(), String::new()),
            ]
        );
    }

    #[test]
    fn add_user_password_rules() {
        assert_eq!(
            password_problem("", ""),
            Some("Use a password of at least 8 characters.")
        );
        assert_eq!(
            password_problem("longenough", "longenouhg"),
            Some("The passwords do not match.")
        );
        assert_eq!(password_problem("longenough", "longenough"), None);
    }
}

//! General > Sharing: the computer's name and Remote Login (wol-admind `hostname`, `remote-login`;
//! the Authenticate sheet asks first). Controls whose verb this build lacks are not shown.
use super::*;
use wol_core::sharing::is_valid_hostname;

fn render(e: &Rc<Env>, into: &gtk::Box, s: Sharing, status: &gtk::Label) {
    let host = e.host.hostname.clone();
    let mut rows = Vec::new();
    if admin("hostname", &[]).is_some() {
        let (ctx, st) = (e.ctx.clone(), status.clone());
        rows.push(form::entry_row(
            "Computer name",
            &host,
            "wonderland",
            move |t| {
                let name = t.trim().to_lowercase();
                if !is_valid_hostname(&name) {
                    return say(
                        &st,
                        "Use lowercase letters, digits and hyphens, up to 63 characters.",
                    );
                }
                if let Some(a) = admin("hostname", &[&name]) {
                    ctx.act_say(a, &st)
                }
            },
        ));
    } else {
        rows.push(info("Computer name", &host));
    }
    into.append(&w::section("", &rows));
    into.append(&w::footnote(
        "Other computers on your network see this name, and Bluetooth devices show it when pairing.",
    ));
    if let (Some(on), Some(_)) = (s.remote_login, admin("remote-login", &[])) {
        let (ctx, st) = (e.ctx.clone(), status.clone());
        let user = e
            .host
            .users
            .first()
            .cloned()
            .unwrap_or_else(|| "you".into());
        into.append(&w::section(
            "",
            &[form::toggle_row(
                "Remote Login",
                &format!("Log in from another computer: ssh {user}@{host}.local"),
                on,
                move |v| {
                    if let Some(a) = admin("remote-login", &[if v { "on" } else { "off" }]) {
                        ctx.act_say(a, &st)
                    }
                },
            )],
        ));
    }
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = general::subpage(e, "Sharing");
    let body = w::vbox(w::S6);
    p.append(&body);
    let status = status_label();
    p.append(&status);
    let (m, e2) = (e.machine(), e.clone());
    later(
        &body,
        move || m.sharing(),
        move |into, s| render(&e2, into, s, &status),
    );
    p
}

//! Privacy & Security: which apps may record the screen, control input, read the clipboard history
//! or see the window list (compositor consent through wol-admind `consent`) and how each choice is
//! enforced, the firewall (pf, `firewall`), the kernel protections WolOS ships, disk encryption,
//! and the way to Assistants & Automation (`wol_core::security`).
use super::*;
use wol_core::security::{self, Check, Enforcement};

/// (capability, title, what it allows): `wol_core::consent::CAPABILITIES` in words.
pub(crate) const KINDS: [(&str, &str, &str); 4] = [
    (
        "screen-capture",
        "Screen Recording",
        "Can see and record everything on your screen.",
    ),
    (
        "input-injection",
        "Remote Control",
        "Can type and click as if it were you.",
    ),
    (
        "clipboard-history",
        "Clipboard History",
        "Can read everything you copy.",
    ),
    (
        "window-list",
        "Window List",
        "Can see which windows are open.",
    ),
];

/// "A", "A and B", "A, B and C".
fn listed(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What WolOS can promise for these permissions (`security::PERMISSIONS`): the compositor refuses
/// some outright; for the rest it passes the choice on and cannot stop an app that ignores it.
pub(crate) fn enforcement_note(caps: &[&str]) -> String {
    let titles = |want: Enforcement| -> Vec<&str> {
        KINDS
            .iter()
            .filter(|k| caps.contains(&k.0) && security::enforcement(k.0) == Some(want))
            .map(|k| k.1)
            .collect()
    };
    let (hard, soft) = (
        titles(Enforcement::Compositor),
        titles(Enforcement::Advisory),
    );
    let mut note = Vec::new();
    if !hard.is_empty() {
        let verb = if hard.len() == 1 { "is" } else { "are" };
        note.push(format!("{} {verb} enforced by WolOS.", listed(&hard)));
    }
    if !soft.is_empty() {
        note.push(format!(
            "For {}, WolOS passes on your choice but cannot stop an app that ignores it.",
            listed(&soft)
        ));
    }
    note.join(" ")
}

/// "On", "4 of 6 on"; None when the kernel could not be asked (not FreeBSD).
pub(crate) fn protections_line(checks: &[Check]) -> Option<String> {
    let known: Vec<&Check> = checks.iter().filter(|c| c.now.is_some()).collect();
    if known.is_empty() {
        return None;
    }
    let on = known.iter().filter(|c| c.ok()).count();
    Some(if on == checks.len() {
        "On".to_string()
    } else {
        format!("{on} of {} on", checks.len())
    })
}

/// App name and icon for an executable path: its desktop entry, else the file name.
fn app_of(entries: &[crate::desktop::Entry], path: &str) -> (String, String) {
    let base = path.rsplit('/').next().unwrap_or(path);
    entries
        .iter()
        .find(|e| {
            e.exec
                .first()
                .is_some_and(|x| x == path || x.rsplit('/').next() == Some(base))
        })
        .map(|e| (e.name.clone(), e.icon.clone()))
        .unwrap_or_else(|| (base.to_string(), w::PACKAGE_ICON.to_string()))
}

struct Facts {
    grants: Vec<(String, String)>,
    security: Option<security::Status>,
    encrypted: Option<bool>,
}

fn render(e: &Rc<Env>, into: &gtk::Box, f: Facts, status: &gtk::Label) {
    let entries = crate::desktop::load_all();
    let can_change = admin("consent", &[]).is_some();
    for (cap, title, what) in KINDS {
        let apps: Vec<&String> = f
            .grants
            .iter()
            .filter(|g| g.1 == cap)
            .map(|g| &g.0)
            .collect();
        if apps.is_empty() {
            continue;
        }
        let rows: Vec<gtk::Widget> = apps
            .into_iter()
            .map(|path| {
                let (name, icon) = app_of(&entries, path);
                let (ctx, st, path) = (e.ctx.clone(), status.clone(), path.clone());
                let r = form::permission_row(&icon, &name, what, true, move |on| {
                    let verdict = if on { "allow" } else { "deny" };
                    if let Some(a) = admin("consent", &[&path, cap, verdict]) {
                        ctx.act_say(a, &st)
                    }
                });
                r.set_sensitive(can_change);
                r
            })
            .collect();
        into.append(&w::section(title, &rows));
    }
    if f.grants.is_empty() {
        into.append(&w::section(
            "Privacy",
            &[w::list_row(
                "security-high-symbolic",
                "No app has extra access",
                "Apps you allow to record the screen or control input are listed here.",
                None,
            )
            .upcast()],
        ));
    }
    let shown: Vec<&str> = KINDS
        .iter()
        .map(|k| k.0)
        .filter(|c| f.grants.is_empty() || f.grants.iter().any(|g| g.1 == *c))
        .collect();
    into.append(&w::footnote(&enforcement_note(&shown)));
    let mut rows = Vec::new();
    let sec = f.security.as_ref();
    if let (Some(on), Some(_)) = (sec.and_then(|s| s.firewall), admin("firewall", &[])) {
        let (ctx, st) = (e.ctx.clone(), status.clone());
        let stopped = on && sec.and_then(|s| s.firewall_running) == Some(false);
        rows.push(form::toggle_row(
            "Firewall",
            if stopped {
                "On at startup, but not running now."
            } else {
                "Blocks connections from other computers that you did not start."
            },
            on,
            move |v| {
                if let Some(a) = admin("firewall", &[if v { "on" } else { "off" }]) {
                    ctx.act_say(a, &st)
                }
            },
        ));
    }
    if let Some(line) = sec.and_then(|s| protections_line(&s.hardening)) {
        let value = w::label(&line, "secondary");
        rows.push(
            form::status_row(
                "",
                "System protections",
                "Accounts cannot see each other's programs or link to each other's files.",
                &value,
            )
            .0,
        );
    }
    if let Some(enc) = f.encrypted {
        rows.push(info("Disk encryption", if enc { "On" } else { "Off" }));
    }
    if !rows.is_empty() {
        into.append(&w::section("Security", &rows));
    }
    let helpers = form::nav_row(
        "wolos-assistant-symbolic",
        "Assistants & Automation",
        "What helpers and scripts may change",
    );
    let open = e.opener("Assistants & Automation");
    helpers.connect_clicked(move |_| open());
    into.append(&w::section("", &[helpers.upcast()]));
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Privacy & Security");
    let body = w::vbox(w::S6);
    p.append(&body);
    let status = status_label();
    p.append(&status);
    let (m, e2) = (e.machine(), e.clone());
    later(
        &body,
        move || Facts {
            grants: m.consent(),
            security: m.security(),
            encrypted: m.storage().and_then(|s| s.encrypted),
        },
        move |into, f| render(&e2, into, f, &status),
    );
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_grantable_capability_is_described() {
        for c in wol_core::consent::CAPABILITIES {
            assert!(KINDS.iter().any(|k| k.0 == c), "{c}");
        }
        assert_eq!(
            app_of(&[], "/usr/local/bin/obs"),
            ("obs".into(), w::PACKAGE_ICON.into())
        );
    }

    #[test]
    fn the_note_says_which_choices_are_enforced() {
        let all: Vec<&str> = KINDS.iter().map(|k| k.0).collect();
        assert_eq!(
            enforcement_note(&all),
            "Screen Recording is enforced by WolOS. For Remote Control, Clipboard History and \
             Window List, WolOS passes on your choice but cannot stop an app that ignores it."
        );
        assert_eq!(
            enforcement_note(&["screen-capture"]),
            "Screen Recording is enforced by WolOS."
        );
        assert!(!enforcement_note(&["window-list"]).contains("enforced"));
    }

    #[test]
    fn protections_count_what_the_kernel_reports() {
        let c = |now: Option<&str>| Check {
            key: "kern.randompid".into(),
            want: "1".into(),
            now: now.map(Into::into),
        };
        assert_eq!(protections_line(&[c(None), c(None)]), None);
        assert_eq!(
            protections_line(&[c(Some("1")), c(Some("1"))]).unwrap(),
            "On"
        );
        assert_eq!(
            protections_line(&[c(Some("1")), c(Some("0")), c(None)]).unwrap(),
            "1 of 3 on"
        );
    }
}

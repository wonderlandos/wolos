//! Assistants & Automation: whether AI helpers and scripts may use WolOS at all, what each kind of
//! change needs, the helpers seen so far (audit log) and how to connect one
//! (archicture/modules/automation.md). Nothing reaches the system while the switch is off.
use super::*;
use wol_core::automation::Status;

/// "Seen 3 times · 1 refused".
pub(crate) fn seen_line(actions: u32, refused: u32) -> String {
    let times = match actions {
        1 => "Used once".to_string(),
        n => format!("Used {n} times"),
    };
    match refused {
        0 => times,
        1 => format!("{times} · 1 request refused"),
        r => format!("{times} · {r} requests refused"),
    }
}

fn render(e: &Rc<Env>, into: &gtk::Box, s: &Status, status: &gtk::Label) {
    clear(into);
    // Shown while off too, dimmed: what turning it on would allow, and how to connect a helper.
    let more = w::vbox(w::S6);
    more.set_sensitive(s.enabled);
    let (env, st, m) = (e.clone(), status.clone(), more.clone());
    let toggle = w::toggle(s.enabled, move |on| {
        match env.sys().set_helpers_enabled(on) {
            Ok(()) => {
                m.set_sensitive(on);
                say(&st, "");
            }
            Err(x) => say(&st, x.message()),
        }
    });
    into.append(&w::section(
        "",
        &[w::list_row(
            "wolos-assistant-symbolic",
            "Allow assistants and scripts",
            "AI helpers and scripts, only with your consent.",
            Some(toggle.upcast_ref()),
        )
        .upcast()],
    ));
    let caps: Vec<gtk::Widget> = s
        .capabilities
        .iter()
        .map(|c| {
            let (env, st, id) = (e.clone(), status.clone(), c.id.clone());
            form::toggle_row(&c.label, "", c.allowed, move |on| {
                if let Err(x) = env.sys().set_helper_capability(&id, on) {
                    say(&st, x.message())
                }
            })
        })
        .collect();
    if !caps.is_empty() {
        more.append(&w::section("What Helpers May Do", &caps));
        more.append(&w::footnote(
            "Changes that need an administrator always ask for your password, whoever requests them. Every change can be undone in Restore.",
        ));
    }
    let seen: Vec<gtk::Widget> = s
        .helpers
        .iter()
        .map(|h| {
            let sub = format!("{} · {}", h.via, seen_line(h.actions, h.refused));
            w::list_row("wolos-assistant-symbolic", &h.name, &sub, None).upcast()
        })
        .collect();
    if !seen.is_empty() {
        more.append(&w::section("Helpers Seen", &seen));
    }
    if !s.always.is_empty() {
        let rows: Vec<gtk::Widget> = s.always.iter().map(|g| info(&g.helper, &g.label)).collect();
        more.append(&w::section("Always Allowed", &rows));
    }
    more.append(&w::section(
        "Connect a Helper",
        &[
            info("AI assistants", "wol mcp"),
            info("Scripts and Terminal", "wol help"),
        ],
    ));
    more.append(&w::footnote(
        "Point an assistant that speaks MCP at the command “wol mcp”. Scripts call the same verbs as “wol”. Both are optional: WolOS never needs AI.",
    ));
    into.append(&more);
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Assistants & Automation");
    let body = w::vbox(w::S6);
    p.append(&body);
    let status = status_label();
    p.append(&status);
    render(e, &body, &e.sys().helpers(), &status);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_lines() {
        assert_eq!(seen_line(1, 0), "Used once");
        assert_eq!(seen_line(12, 1), "Used 12 times · 1 request refused");
        assert_eq!(seen_line(3, 2), "Used 3 times · 2 requests refused");
    }
}

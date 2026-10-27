//! Notifications: Focus (Do Not Disturb, for a while or until turned off) and Notification Center.
//! Same file as the Control Center Focus tile and `wol focus` (`crate::focus`).
use super::*;
use crate::focus::{self, Focus};

/// "Turn off" choices while Focus is on.
const UNTIL: [&str; 3] = ["When I turn it off", "In one hour", "Tomorrow morning"];

fn until_index(f: Focus, now: u64) -> usize {
    match f {
        Focus::Until(u) if u.saturating_sub(now) <= focus::HOUR => 1,
        Focus::Until(_) => 2,
        _ => 0,
    }
}

fn focus_for(i: usize) -> Focus {
    let now = focus::unix_now();
    match i {
        1 => Focus::Until(now + focus::HOUR),
        2 => {
            let t = kit::now();
            Focus::Until(now + focus::secs_to_morning(t.hour, t.minute, t.second))
        }
        _ => Focus::On,
    }
}

pub(super) fn page(_: &Rc<Env>) -> gtk::Box {
    let p = w::page("Notifications");
    let status = status_label();
    let cur = focus::read();
    let now = focus::unix_now();
    let until = form::picker_row("Turn off", "", &UNTIL, until_index(cur, now), {
        let st = status.clone();
        move |i| {
            let r = focus::write(focus_for(i));
            say(&st, if r.is_ok() { "" } else { SAVE_FAILED });
        }
    });
    until.set_visible(cur.active());
    let t = kit::now();
    let sub: Rc<RefCell<Option<gtk::Label>>> = Default::default();
    let toggle = {
        let (st, until, sub) = (status.clone(), until.clone(), sub.clone());
        w::toggle(cur.active(), move |on| {
            let f = if on { Focus::On } else { Focus::Off };
            let r = focus::write(f);
            until.set_visible(on);
            let t = kit::now();
            if let Some(l) = &*sub.borrow() {
                l.set_label(&state_line(f, focus::unix_now(), t.hour, t.minute));
            }
            say(&st, if r.is_ok() { "" } else { SAVE_FAILED });
        })
    };
    let (dnd, line) = form::status_row(
        "weather-clear-night-symbolic",
        "Do Not Disturb",
        &state_line(cur, now, t.hour, t.minute),
        &toggle,
    );
    *sub.borrow_mut() = Some(line);
    p.append(&w::section("Focus", &[dnd, until]));
    p.append(&w::footnote(
        "While Focus is on, banners stay quiet and wait in Notification Center. Critical alerts, such as a low battery, still show.",
    ));
    p.append(&w::section(
        "",
        &[form::button_row(
            "Notification Center",
            "Every notification you missed, newest first.",
            "Show",
            || kit::spawn(&argv(&["wol", "shell", "show", "notifications"])),
        )],
    ));
    p.append(&status);
    p
}

/// "Off", "On until you turn it off", "On until 15:05".
fn state_line(f: Focus, now: u64, h: u8, m: u8) -> String {
    match f {
        Focus::Off => "Off".into(),
        Focus::On => "On until you turn it off".into(),
        Focus::Until(_) => format!("On {}", focus::label(f, now, h, m).to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_lines_and_choices() {
        assert_eq!(state_line(Focus::Off, 0, 9, 0), "Off");
        assert_eq!(state_line(Focus::On, 0, 9, 0), "On until you turn it off");
        assert_eq!(state_line(Focus::Until(3600), 0, 9, 0), "On until 10:00");
        assert_eq!(until_index(Focus::On, 0), 0);
        assert_eq!(until_index(Focus::Until(1800), 0), 1);
        assert_eq!(until_index(Focus::Until(30_000), 0), 2);
        assert_eq!(focus_for(0), Focus::On);
        assert!(matches!(focus_for(1), Focus::Until(_)));
    }
}

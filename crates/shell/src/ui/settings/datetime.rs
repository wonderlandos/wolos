//! General > Date & Time: network time and the time zone (wol-admind `ntp`, `timezone`; the
//! Authenticate sheet asks first). Controls whose verb this build lacks are not shown.
use super::*;
use wol_core::region::zone_label;

struct Facts {
    zones: Vec<String>,
    zone: String,
    ntp: Option<bool>,
}

pub(super) const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// "Tuesday 27 October, 14:05".
pub(crate) fn clock_line(t: &crate::clock::Stamp) -> String {
    const DAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    format!(
        "{} {} {}, {:02}:{:02}",
        DAYS[usize::from(t.weekday.clamp(1, 7) - 1)],
        t.day,
        MONTHS[usize::from(t.month.clamp(1, 12) - 1)],
        t.hour,
        t.minute
    )
}

fn render(e: &Rc<Env>, into: &gtk::Box, f: Facts, status: &gtk::Label) {
    let mut rows = vec![info("Date and time", &clock_line(&kit::now()))];
    if let (Some(on), Some(_)) = (f.ntp, admin("ntp", &[])) {
        let (ctx, st) = (e.ctx.clone(), status.clone());
        rows.push(form::toggle_row(
            "Set time and date automatically",
            "From time servers on the internet.",
            on,
            move |v| {
                if let Some(a) = admin("ntp", &[if v { "on" } else { "off" }]) {
                    ctx.act_say(a, &st)
                }
            },
        ));
    }
    into.append(&w::section("", &rows));
    if f.zones.is_empty() || admin("timezone", &[]).is_none() {
        into.append(&w::section("", &[info("Time zone", &zone_label(&f.zone))]));
        return;
    }
    let labels: Vec<String> = f.zones.iter().map(|z| zone_label(z)).collect();
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    let at = f.zones.iter().position(|z| *z == f.zone).unwrap_or(0);
    let (ctx, st, zones) = (e.ctx.clone(), status.clone(), f.zones.clone());
    into.append(&w::section(
        "",
        &[form::search_picker_row(
            "Time zone",
            "Type a city to find it.",
            &names,
            at,
            move |i| {
                if let Some(a) = zones.get(i).and_then(|z| admin("timezone", &[z])) {
                    ctx.act_say(a, &st)
                }
            },
        )],
    ));
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = general::subpage(e, "Date & Time");
    let body = w::vbox(w::S6);
    p.append(&body);
    let status = status_label();
    p.append(&status);
    let (m, e2) = (e.machine(), e.clone());
    later(
        &body,
        move || Facts {
            zones: m.zones(),
            zone: m.zone(),
            ntp: m.ntp(),
        },
        move |into, f| render(&e2, into, f, &status),
    );
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_line_reads_like_a_sentence() {
        let t = crate::clock::Stamp {
            weekday: 2,
            day: 27,
            month: 10,
            hour: 9,
            minute: 5,
            ..kit::FAKE_NOW
        };
        assert_eq!(clock_line(&t), "Tuesday 27 October, 09:05");
    }
}

//! Calendar widget data: the month grid (weeks start Monday) and today's events parsed from the
//! iCalendar text evolution-data-server returns (`eds.rs`). No calendar service, no events shown.

pub const MONTHS: [&str; 12] = [
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
pub const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Weeks of `y`-`m`, Monday first; None pads the first and last week.
pub fn weeks(y: i32, m: u32) -> Vec<[Option<u8>; 7]> {
    let first = super::weather::weekday(y, m, 1) as usize;
    let n = days_in_month(y, m) as usize;
    let mut out = Vec::new();
    let mut week = [None; 7];
    for i in 0..first + n {
        if i >= first {
            week[i % 7] = Some((i - first + 1) as u8);
        }
        if i % 7 == 6 {
            out.push(week);
            week = [None; 7];
        }
    }
    if week.iter().any(Option::is_some) {
        out.push(week);
    }
    out
}

/// Local date and time, as the widget shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTime {
    pub y: i32,
    pub m: u32,
    pub d: u32,
    pub hh: u32,
    pub mm: u32,
}

/// DTSTART/DTEND as written: the provider converts zoned and UTC times to local.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    Date { y: i32, m: u32, d: u32 },
    Utc(DateTime),
    Zoned { tzid: String, at: DateTime },
    Floating(DateTime),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RawEvent {
    pub summary: String,
    pub location: String,
    pub start: When,
    pub end: Option<When>,
    pub recurring: bool,
}

/// One event as shown today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub title: String,
    pub location: String,
    /// None = all day.
    pub start: Option<(u32, u32)>,
    pub end: Option<(u32, u32)>,
}

impl Event {
    /// "15:00" or "All day".
    pub fn time_label(&self) -> String {
        match self.start {
            Some((h, m)) => format!("{h:02}:{m:02}"),
            None => "All day".into(),
        }
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n' | 'N') => out.push(' '),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_when(params: &str, value: &str) -> Option<When> {
    let v = value.trim();
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<u32>().ok();
    let (y, m, d) = (num(0, 4)? as i32, num(4, 6)?, num(6, 8)?);
    if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") || v.len() == 8 {
        return Some(When::Date { y, m, d });
    }
    if v.get(8..9) != Some("T") {
        return None;
    }
    let at = DateTime {
        y,
        m,
        d,
        hh: num(9, 11)?,
        mm: num(11, 13)?,
    };
    if v.ends_with('Z') {
        return Some(When::Utc(at));
    }
    match params
        .split(';')
        .find_map(|p| p.strip_prefix("TZID="))
        .map(|t| t.trim_matches('"').to_string())
    {
        Some(tzid) => Some(When::Zoned { tzid, at }),
        None => Some(When::Floating(at)),
    }
}

/// Every VEVENT in `ics` (one component or a whole VCALENDAR); cancelled ones are left out.
pub fn parse_ics(ics: &str) -> Vec<RawEvent> {
    // Unfold continuation lines (RFC 5545 3.1).
    let mut lines: Vec<String> = Vec::new();
    for l in ics.split('\n').map(|l| l.trim_end_matches('\r')) {
        match (l.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(cont), Some(last)) => last.push_str(cont),
            _ => lines.push(l.to_string()),
        }
    }
    let mut out = Vec::new();
    let mut cur: Option<(RawEvent, bool)> = None;
    let mut depth = 0;
    for l in &lines {
        let (head, value) = l.split_once(':').unwrap_or((l, ""));
        let (name, params) = head.split_once(';').unwrap_or((head, ""));
        match (name.to_ascii_uppercase().as_str(), value) {
            ("BEGIN", "VEVENT") if cur.is_none() => {
                cur = Some((
                    RawEvent {
                        summary: String::new(),
                        location: String::new(),
                        start: When::Date { y: 0, m: 0, d: 0 },
                        end: None,
                        recurring: false,
                    },
                    false,
                ));
                depth = 0;
            }
            ("BEGIN", _) if cur.is_some() => depth += 1, // VALARM inside the event
            ("END", "VEVENT") if depth == 0 => {
                if let Some((e, cancelled)) = cur.take()
                    && !cancelled
                    && !matches!(e.start, When::Date { y: 0, .. })
                {
                    out.push(e);
                }
            }
            ("END", _) if cur.is_some() => depth -= 1,
            (prop, v) if depth == 0 => {
                let Some((e, cancelled)) = cur.as_mut() else {
                    continue;
                };
                match prop {
                    "SUMMARY" => e.summary = unescape(v),
                    "LOCATION" => e.location = unescape(v),
                    "DTSTART" => {
                        if let Some(w) = parse_when(params, v) {
                            e.start = w
                        }
                    }
                    "DTEND" => e.end = parse_when(params, v),
                    "RRULE" | "RDATE" => e.recurring = true,
                    "STATUS" => *cancelled = v.eq_ignore_ascii_case("CANCELLED"),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// Today's events still to come (or in progress) at `now`, all-day first, then by start.
/// `local` converts zoned/UTC times to local; recurring events keep their time of day.
pub fn upcoming(
    events: &[RawEvent],
    now: DateTime,
    local: impl Fn(&When) -> Option<DateTime>,
) -> Vec<Event> {
    let today = (now.y, now.m, now.d);
    let mut out: Vec<Event> = events
        .iter()
        .filter_map(|e| {
            let title = if e.summary.trim().is_empty() {
                "Untitled event".to_string()
            } else {
                e.summary.trim().to_string()
            };
            if let When::Date { y, m, d } = e.start {
                return ((y, m, d) == today || e.recurring).then(|| Event {
                    title,
                    location: e.location.clone(),
                    start: None,
                    end: None,
                });
            }
            let s = local(&e.start)?;
            if (s.y, s.m, s.d) != today && !e.recurring {
                return None;
            }
            let end = e
                .end
                .as_ref()
                .and_then(&local)
                .map(|x| (x.hh, x.mm))
                .filter(|_| !e.recurring)
                .unwrap_or(((s.hh + 1).min(23), s.mm));
            (end > (now.hh, now.mm)).then(|| Event {
                title,
                location: e.location.clone(),
                start: Some((s.hh, s.mm)),
                end: Some(end),
            })
        })
        .collect();
    out.sort_by_key(|e| e.start);
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn october_2026_starts_on_thursday() {
        let w = weeks(2026, 10);
        assert_eq!(w.len(), 5);
        assert_eq!(w[0], [None, None, None, Some(1), Some(2), Some(3), Some(4)]);
        assert_eq!(
            w[4],
            [
                Some(26),
                Some(27),
                Some(28),
                Some(29),
                Some(30),
                Some(31),
                None
            ]
        );
        assert_eq!(weeks(2021, 2).len(), 4, "Feb 2021 fills exactly four weeks");
        assert_eq!(weeks(2026, 3).len(), 6);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
    }

    const ICS: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Croquet with the Q\r\n ueen\r\nDTSTART;TZID=Europe/Lisbon:20261026T150000\r\nDTEND;TZID=Europe/Lisbon:20261026T160000\r\nLOCATION:Garden\\, east lawn\r\nBEGIN:VALARM\r\nSUMMARY:alarm text\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Tea party\r\nDTSTART:20261026T090000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Unbirthday\r\nDTSTART;VALUE=DATE:20261026\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Cancelled\r\nSTATUS:CANCELLED\r\nDTSTART:20261026T100000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Standup\r\nDTSTART:20250105T170000\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parses_events_with_zones_folding_and_alarms() {
        let e = parse_ics(ICS);
        assert_eq!(e.len(), 4, "{e:#?}");
        assert_eq!(e[0].summary, "Croquet with the Queen");
        assert_eq!(e[0].location, "Garden, east lawn");
        assert!(
            matches!(&e[0].start, When::Zoned { tzid, at } if tzid == "Europe/Lisbon" && at.hh == 15)
        );
        assert!(matches!(e[1].start, When::Utc(DateTime { hh: 9, .. })));
        assert_eq!(
            e[2].start,
            When::Date {
                y: 2026,
                m: 10,
                d: 26
            }
        );
        assert!(e[3].recurring);
        assert!(parse_ics("garbage").is_empty());
    }

    #[test]
    fn upcoming_lists_what_is_left_of_today() {
        let now = DateTime {
            y: 2026,
            m: 10,
            d: 26,
            hh: 14,
            mm: 5,
        };
        // Test zone: UTC+1 for every zoned or UTC time.
        let local = |w: &When| match w {
            When::Utc(t) | When::Zoned { at: t, .. } => Some(DateTime { hh: t.hh + 1, ..*t }),
            When::Floating(t) => Some(*t),
            When::Date { .. } => None,
        };
        let got: Vec<(String, String)> = upcoming(&parse_ics(ICS), now, local)
            .into_iter()
            .map(|e| (e.time_label(), e.title))
            .collect();
        assert_eq!(
            got,
            [
                ("All day".to_string(), "Unbirthday".to_string()),
                ("16:00".into(), "Croquet with the Queen".into()),
                ("17:00".into(), "Standup".into()),
            ],
            "the 10:00 tea party is over"
        );
    }
}

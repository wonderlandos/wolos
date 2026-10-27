//! Today's events from evolution-data-server (session bus), when it is installed. Bus names carry
//! a version (`...Calendar8`, `...Sources5`), found by prefix. Calendar objects only live as long
//! as the connection that opened them, so one connection does the whole read. Blocking: call off
//! the UI thread. None = no calendar service (the widget then shows the month only).
use super::calendar::RawEvent;
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SOURCES: &str = "org.gnome.evolution.dataserver.Sources";
const CALENDAR: &str = "org.gnome.evolution.dataserver.Calendar";

/// Highest-versioned bus name with `prefix` + digits.
pub fn versioned(names: &[String], prefix: &str) -> Option<String> {
    names
        .iter()
        .filter_map(|n| {
            let v = n.strip_prefix(prefix)?;
            (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                .then(|| (v.parse::<u32>().unwrap_or(0), n.clone()))
        })
        .max()
        .map(|(_, n)| n)
}

/// A source's key file: calendar, and not disabled.
pub fn is_enabled_calendar(data: &str) -> bool {
    data.lines().any(|l| l.trim() == "[Calendar]")
        && !data
            .lines()
            .any(|l| l.trim().replace(' ', "") == "Enabled=false")
}

/// `(occur-in-time-range? ...)` for [start, end) in UTC `YYYYMMDDTHHMMSSZ`.
pub fn range_query(start_utc: &str, end_utc: &str) -> String {
    format!("(occur-in-time-range? (make-time \"{start_utc}\") (make-time \"{end_utc}\"))")
}

/// Events in the UTC range from every enabled calendar.
pub fn read(start_utc: &str, end_utc: &str) -> Option<Vec<RawEvent>> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
    let mut names: Vec<String> = dbus
        .list_activatable_names()
        .ok()?
        .into_iter()
        .map(|n| n.to_string())
        .collect();
    names.extend(dbus.list_names().ok()?.into_iter().map(|n| n.to_string()));
    let sources = versioned(&names, SOURCES)?;
    let factory = versioned(&names, CALENDAR)?;
    type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    let objects: Objects = conn
        .call_method(
            Some(sources.as_str()),
            "/org/gnome/evolution/dataserver/SourceManager",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let query = range_query(start_utc, end_utc);
    let mut out = Vec::new();
    for ifaces in objects.values() {
        let Some(src) = ifaces.get("org.gnome.evolution.dataserver.Source") else {
            continue;
        };
        let s = |k: &str| {
            src.get(k)
                .and_then(|v| v.try_clone().ok())
                .and_then(|v| String::try_from(v).ok())
                .unwrap_or_default()
        };
        if !is_enabled_calendar(&s("Data")) {
            continue;
        }
        let uid = s("UID");
        let Ok(reply) = conn.call_method(
            Some(factory.as_str()),
            "/org/gnome/evolution/dataserver/CalendarFactory",
            Some("org.gnome.evolution.dataserver.CalendarFactory"),
            "OpenCalendar",
            &(uid.as_str(),),
        ) else {
            continue;
        };
        let Ok((path, bus)) = reply.body().deserialize::<(String, String)>() else {
            continue;
        };
        let iface = "org.gnome.evolution.dataserver.Calendar";
        if conn
            .call_method(Some(bus.as_str()), path.as_str(), Some(iface), "Open", &())
            .is_err()
        {
            continue;
        }
        if let Ok(r) = conn.call_method(
            Some(bus.as_str()),
            path.as_str(),
            Some(iface),
            "GetObjectList",
            &(query.as_str(),),
        ) && let Ok(objs) = r.body().deserialize::<Vec<String>>()
        {
            for o in objs {
                out.extend(super::calendar::parse_ics(&o));
            }
        }
        let _ = conn.call_method(Some(bus.as_str()), path.as_str(), Some(iface), "Close", &());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_versioned_bus_names() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let names = n(&[
            "org.gnome.evolution.dataserver.Calendar7",
            "org.gnome.evolution.dataserver.Calendar8",
            "org.gnome.evolution.dataserver.CalendarFactory",
            "org.gnome.evolution.dataserver.Sources5",
        ]);
        assert_eq!(
            versioned(&names, CALENDAR).as_deref(),
            Some("org.gnome.evolution.dataserver.Calendar8")
        );
        assert_eq!(
            versioned(&names, SOURCES).as_deref(),
            Some("org.gnome.evolution.dataserver.Sources5")
        );
        assert_eq!(versioned(&n(&["org.freedesktop.DBus"]), SOURCES), None);
    }

    #[test]
    fn only_enabled_calendars_are_read() {
        assert!(is_enabled_calendar(
            "[Data Source]\nDisplayName=Personal\n[Calendar]\nBackendName=local\n"
        ));
        assert!(!is_enabled_calendar(
            "[Data Source]\nEnabled=false\n[Calendar]\n"
        ));
        assert!(!is_enabled_calendar("[Data Source]\n[Address Book]\n"));
        assert_eq!(
            range_query("20261026T000000Z", "20261027T000000Z"),
            "(occur-in-time-range? (make-time \"20261026T000000Z\") (make-time \"20261027T000000Z\"))"
        );
    }

    /// Manual: `cargo test -p shell --lib live_eds -- --ignored --nocapture` lists this year's events.
    #[test]
    #[ignore]
    fn live_eds() {
        let e = read("20260101T000000Z", "20270101T000000Z");
        println!("{e:#?}");
        assert!(e.is_some(), "evolution-data-server not reachable");
    }
}

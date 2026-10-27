//! Menu bar clock "Mon 26 Oct 14:05", lock clock "14:05" / "Monday, 26 October". No allocation: callers reuse a buffer.
use std::fmt::Write;

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTHS: [&str; 12] = [
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

/// Local time fields. `weekday` 1 = Monday .. 7 = Sunday, `month` 1..12.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

pub fn menubar(buf: &mut String, t: Stamp) {
    buf.clear();
    let _ = write!(
        buf,
        "{} {} {} {:02}:{:02}",
        &DAYS[(t.weekday - 1) as usize][..3],
        t.day,
        &MONTHS[(t.month - 1) as usize][..3],
        t.hour,
        t.minute
    );
}

/// Menu bar clock without the date (Settings > Menu Bar > Show date off): "14:05".
pub fn menubar_time(buf: &mut String, t: Stamp) {
    buf.clear();
    let _ = write!(buf, "{:02}:{:02}", t.hour, t.minute);
}

pub fn lock_time(buf: &mut String, t: Stamp) {
    buf.clear();
    let _ = write!(buf, "{}:{:02}", t.hour, t.minute);
}

pub fn lock_date(buf: &mut String, t: Stamp) {
    buf.clear();
    let _ = write!(
        buf,
        "{}, {} {}",
        DAYS[(t.weekday - 1) as usize],
        t.day,
        MONTHS[(t.month - 1) as usize]
    );
}

/// Seconds until the next minute boundary: the clock ticks once a minute, not every second.
pub fn secs_to_next_minute(second: u8) -> u32 {
    60 - second.min(59) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: Stamp = Stamp {
        weekday: 1,
        day: 26,
        month: 10,
        hour: 14,
        minute: 5,
        second: 30,
    };

    #[test]
    fn formats() {
        let mut b = String::new();
        menubar(&mut b, T);
        assert_eq!(b, "Mon 26 Oct 14:05");
        lock_time(&mut b, T);
        assert_eq!(b, "14:05");
        lock_date(&mut b, T);
        assert_eq!(b, "Monday, 26 October");
        menubar(
            &mut b,
            Stamp {
                weekday: 7,
                day: 1,
                month: 1,
                hour: 0,
                minute: 0,
                second: 0,
            },
        );
        assert_eq!(b, "Sun 1 Jan 00:00");
    }

    #[test]
    fn reuses_buffer_without_growing() {
        let mut b = String::with_capacity(32);
        let cap = b.capacity();
        for _ in 0..100 {
            menubar(&mut b, T);
        }
        assert_eq!(b.capacity(), cap);
    }

    #[test]
    fn ticks_on_minute_boundary() {
        for (s, w) in [(0, 60), (30, 30), (59, 1), (60, 1)] {
            assert_eq!(secs_to_next_minute(s), w);
        }
    }
}

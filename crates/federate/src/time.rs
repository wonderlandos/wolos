// SPDX-License-Identifier: BSD-2-Clause
//! `expires_at` of TLD and domain records: RFC 3339 (`2026-07-07T03:01:37.015283418+00:00`, `...Z`).
//! A record past it stops resolving even though its signature still holds, and a value that does
//! not read as a time counts as past (the protocol fails closed).

/// Seconds since 1970 of an RFC 3339 date-time, or `None`.
pub fn unix_seconds(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        let t = s.get(a..z)?;
        t.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| t.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    let offset = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let (oh, om) = (num(i + 1, i + 3)?, num(i + 4, i + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let o = oh * 3600 + om * 60;
            if *sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    // Days from the civil date (proleptic Gregorian), H. Hinnant's algorithm.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// Whether a record with this `expires_at` has expired at `now` (seconds since 1970).
pub fn expired(expires_at: Option<&str>, now: i64) -> bool {
    match expires_at {
        None => false,
        Some(t) => unix_seconds(t).is_none_or(|t| t <= now),
    }
}

/// Now, in seconds since 1970.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_reads_as_unix_seconds() {
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            unix_seconds("2026-07-07T03:01:37.015283418+00:00"),
            Some(1_783_393_297)
        );
        assert_eq!(
            unix_seconds("2026-07-07T00:01:37-03:00"),
            Some(1_783_393_297)
        );
        assert_eq!(unix_seconds("2000-02-29T12:00:00Z"), Some(951_825_600));
        for bad in [
            "",
            "2026-07-07",
            "2026-13-01T00:00:00Z",
            "2026-07-07T03:01:37",
            "2026-07-07T03:01:37.Z",
            "2026-07-07T03:01:37+0000",
            "x026-07-07T03:01:37Z",
        ] {
            assert_eq!(unix_seconds(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_unreadable_expiry_is_an_expired_one() {
        assert!(!expired(None, 10));
        assert!(!expired(Some("1970-01-01T00:01:00Z"), 59));
        assert!(expired(Some("1970-01-01T00:01:00Z"), 60));
        assert!(expired(Some("soon"), 0));
    }
}

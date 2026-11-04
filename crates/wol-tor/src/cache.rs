// SPDX-License-Identifier: GPL-2.0-or-later
//! The bootstrap cache (ADR 0053 D4): what makes a Tor group start warm.
//!
//! `wol-tor-cache.service` runs tor now and then and keeps the directory documents in
//! `$XDG_STATE_HOME/wol-tor/cache/`. A group copies them into its tmpfs DataDirectory before its
//! tor starts, so tor only has to build circuits. The cache holds no browsing data: the consensus,
//! the microdescriptors and the authority certificates are public documents, and `state` holds
//! the guard and the learned circuit-build timeout (one guard per machine, as in Tor Browser).
//! A group's tor never writes back: only the cache unit does ([`collect`]).
//! A consensus is used only while it is valid (`valid-until` in its header); a stale one is not
//! copied, so tor fetches a new one instead of trying with old relays.
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Directory documents copied when the consensus is valid.
pub const DOCUMENTS: [&str; 4] = [
    "cached-certs",
    "cached-microdesc-consensus",
    "cached-microdescs",
    "cached-microdescs.new",
];
/// tor's state file (guards, circuit-build timeout).
pub const STATE: &str = "state";
/// Largest file copied (the microdescriptors are about 10 MB today).
pub const MAX_FILE: u64 = 64 << 20;
/// Bytes of the consensus read to find `valid-until`.
pub const HEAD: u64 = 4096;

/// A cache failure.
#[derive(Debug)]
pub enum CacheError {
    /// A file could not be read or written.
    Io(String, io::Error),
    /// A file is larger than [`MAX_FILE`].
    TooLarge(String),
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "the Tor cache file {p} could not be copied ({e})"),
            Self::TooLarge(p) => write!(
                f,
                "the Tor cache file {p} is larger than {MAX_FILE} bytes and was skipped"
            ),
        }
    }
}

impl std::error::Error for CacheError {}

/// What [`seed`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct Seeded {
    /// Files copied.
    pub copied: Vec<&'static str>,
    /// The consensus was valid and copied.
    pub warm: bool,
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm), proleptic Gregorian.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `valid-until YYYY-MM-DD HH:MM:SS` (UTC) from the head of a consensus.
pub fn valid_until(head: &str) -> Option<SystemTime> {
    stamp(head, "valid-until ")
}

/// `valid-after YYYY-MM-DD HH:MM:SS` (UTC) from the head of a consensus.
pub fn valid_after(head: &str) -> Option<SystemTime> {
    stamp(head, "valid-after ")
}

/// The valid-after and valid-until of the consensus kept in `dir` (a tor DataDirectory or the
/// cache); `None` when there is none or it cannot be read.
pub fn consensus_window(dir: &Path) -> Option<(SystemTime, SystemTime)> {
    let head = head_of(&dir.join("cached-microdesc-consensus")).ok()?;
    Some((valid_after(&head)?, valid_until(&head)?))
}

fn stamp(head: &str, key: &str) -> Option<SystemTime> {
    let line = head.lines().find_map(|l| l.strip_prefix(key))?;
    let (date, time) = line.trim().split_once(' ')?;
    let d: Vec<i64> = date
        .split('-')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let t: Vec<i64> = time
        .split(':')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let ([y, mo, da], [h, mi, s]) = (d.as_slice(), t.as_slice()) else {
        return None;
    };
    let in_range = (1..=12).contains(mo)
        && (1..=31).contains(da)
        && (0..24).contains(h)
        && (0..60).contains(mi)
        && (0..=60).contains(s);
    if !in_range {
        return None;
    }
    let secs = days_from_civil(*y, *mo, *da) * 86_400 + h * 3600 + mi * 60 + s;
    Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?))
}

fn head_of(p: &Path) -> io::Result<String> {
    let mut s = String::new();
    fs::File::open(p)?.take(HEAD).read_to_string(&mut s)?;
    Ok(s)
}

/// Whether the cache's consensus is still valid at `now`.
pub fn consensus_valid(cache: &Path, now: SystemTime) -> bool {
    head_of(&cache.join("cached-microdesc-consensus"))
        .ok()
        .and_then(|h| valid_until(&h))
        .is_some_and(|v| v > now)
}

fn copy_one(from: &Path, to: &Path, name: &str) -> Result<bool, CacheError> {
    let src = from.join(name);
    let meta = match fs::metadata(&src) {
        Ok(m) if m.is_file() => m,
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(CacheError::Io(name.into(), e)),
    };
    if meta.len() > MAX_FILE {
        return Err(CacheError::TooLarge(name.into()));
    }
    let tmp = to.join(format!(".{name}.wol-tor"));
    let io_err = |e| CacheError::Io(name.into(), e);
    fs::copy(&src, &tmp).map_err(io_err)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(io_err)?;
    fs::rename(&tmp, to.join(name)).map_err(io_err)?;
    Ok(true)
}

/// Seeds a group's DataDirectory `data` from `cache`: `state` always (the guard), the directory
/// documents only when the consensus is valid at `now`. A missing cache is not an error (the
/// first start is cold).
pub fn seed(cache: &Path, data: &Path, now: SystemTime) -> Result<Seeded, CacheError> {
    let mut copied = Vec::new();
    if copy_one(cache, data, STATE)? {
        copied.push(STATE);
    }
    let warm = consensus_valid(cache, now);
    if warm {
        for d in DOCUMENTS {
            if copy_one(cache, data, d)? {
                copied.push(d);
            }
        }
    }
    Ok(Seeded { copied, warm })
}

/// The cache unit, after its tor bootstrapped: keeps the documents and the state for next time.
pub fn collect(data: &Path, cache: &Path) -> Result<Vec<&'static str>, CacheError> {
    let mut out = Vec::new();
    for d in DOCUMENTS.iter().copied().chain([STATE]) {
        if copy_one(data, cache, d)? {
            out.push(d);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> SystemTime {
        let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
        UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).unwrap())
    }

    #[test]
    fn civil_dates_match_known_epochs() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(
            at(2026, 11, 4, 21, 0, 0)
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            1_793_826_000
        );
    }

    #[test]
    fn valid_until_is_read_from_the_header() {
        let head = concat!(
            "network-status-version 3 microdesc\nvote-status consensus\n",
            "valid-after 2026-11-04 20:00:00\nfresh-until 2026-11-04 21:00:00\n",
            "valid-until 2026-11-04 23:00:00\n"
        );
        assert_eq!(valid_until(head), Some(at(2026, 11, 4, 23, 0, 0)));
        assert_eq!(valid_after(head), Some(at(2026, 11, 4, 20, 0, 0)));
        assert_eq!(valid_until("valid-until 2026-13-04 23:00:00\n"), None);
        assert_eq!(valid_until("valid-until soon\n"), None);
        assert_eq!(valid_until("nothing here"), None);
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wol-tor-cache-{tag}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn clean(d: &Path) {
        for f in DOCUMENTS.iter().chain([&STATE]) {
            let _ = fs::remove_file(d.join(f));
        }
        let _ = fs::remove_dir(d);
    }

    #[test]
    fn seed_copies_guard_always_and_documents_only_when_fresh() {
        let (cache, data) = (dir("c1"), dir("d1"));
        fs::write(cache.join(STATE), "Guard in=default rsa_id=AAAA\n").unwrap();
        fs::write(
            cache.join("cached-microdesc-consensus"),
            "valid-until 2026-11-04 23:00:00\n",
        )
        .unwrap();
        fs::write(cache.join("cached-microdescs"), "m").unwrap();
        let fresh = seed(&cache, &data, at(2026, 11, 4, 22, 0, 0)).unwrap();
        assert!(fresh.warm);
        assert_eq!(
            fresh.copied,
            vec![STATE, "cached-microdesc-consensus", "cached-microdescs"]
        );
        let mode = fs::metadata(data.join(STATE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        clean(&data);
        let data = dir("d1");
        let stale = seed(&cache, &data, at(2026, 11, 5, 1, 0, 0)).unwrap();
        assert!(!stale.warm);
        assert_eq!(stale.copied, vec![STATE]);
        assert!(!data.join("cached-microdesc-consensus").exists());
        clean(&data);
        clean(&cache);
    }

    #[test]
    fn missing_cache_is_a_cold_start_not_an_error() {
        let (cache, data) = (dir("c2"), dir("d2"));
        assert_eq!(
            seed(&cache, &data, SystemTime::now()).unwrap(),
            Seeded {
                copied: vec![],
                warm: false
            }
        );
        fs::write(data.join("cached-certs"), "c").unwrap();
        assert_eq!(collect(&data, &cache).unwrap(), vec!["cached-certs"]);
        clean(&data);
        clean(&cache);
    }
}

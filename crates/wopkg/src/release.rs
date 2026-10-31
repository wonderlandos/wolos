//! Releases and channels (ADR 0022): versions `MAJOR.MINOR.PATCH[-pre]`, signed channel feeds
//! (`stable`, `beta`, `nightly`), staged rollout by a stable per-machine bucket, pause, and the
//! plan `wopkg update` follows. Never plans a downgrade: going back is a rollback.
use crate::ffi::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let (core, pre) = match s.trim().split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_owned())),
            Some(_) => return None,
            None => (s.trim(), None),
        };
        let mut it = core.split('.').map(|x| x.parse::<u32>().ok());
        let v = Version {
            major: it.next()??,
            minor: it.next().unwrap_or(Some(0))?,
            patch: it.next().unwrap_or(Some(0))?,
            pre,
        };
        it.next().is_none().then_some(v)
    }
    /// `--to 1.1` matches every 1.1.x; `--to 1.1.2` only that one.
    pub fn matches(&self, want: &str) -> bool {
        let parts: Vec<&str> = want.split('.').collect();
        match parts.len() {
            1 => parts[0].parse() == Ok(self.major),
            2 => parts[0].parse() == Ok(self.major) && parts[1].parse() == Ok(self.minor),
            _ => Version::parse(want).as_ref() == Some(self),
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(o.major, o.minor, o.patch))
            .then_with(|| match (&self.pre, &o.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => pre_cmp(a, b),
            })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// semver pre-release order: dot parts, numbers numerically and below words.
fn pre_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    _ => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

pub const CHANNELS: [&str; 3] = shared::packages::CHANNELS;

/// One release in a channel feed. `manifest` is the signed `release.json` text wold installs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub manifest: String,
    /// Ed25519 signature of `manifest`, lowercase hex.
    pub signature: String,
    /// Percent of machines offered it now (staged rollout), 0..=100.
    #[serde(default = "full")]
    pub rollout: u8,
    #[serde(default)]
    pub notes: String,
    /// A security release skips the pause and the rollout.
    #[serde(default)]
    pub security: bool,
}

fn full() -> u8 {
    100
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feed {
    pub format: u32,
    pub channel: String,
    pub releases: Vec<Release>,
}

impl Feed {
    /// Only a feed signed by a trusted key counts.
    pub fn verify(text: &[u8], sig: &[u8], keys: &[[u8; 32]]) -> Result<Feed> {
        match wol_core::package::verify_index(text, Some(sig), keys) {
            Ok(wol_core::package::Trust::Signed(_)) => {}
            _ => return Err(Error::untrusted("The release list is not signed by WolOS.")),
        }
        let f: Feed = serde_json::from_slice(text)
            .map_err(|_| Error::untrusted("The release list is damaged."))?;
        if f.format != 1 || !CHANNELS.contains(&f.channel.as_str()) {
            return Err(Error::untrusted(
                "The release list is in an unknown format.",
            ));
        }
        Ok(f)
    }
}

/// Stable 0..100 bucket for staged rollout: the same machine gets the same answer per release.
pub fn bucket(machine_id: &str, version: &str) -> u8 {
    let h = Sha256::digest(format!("{machine_id}:{version}").as_bytes());
    (u16::from_be_bytes([h[0], h[1]]) % 100) as u8
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    UpToDate,
    Available,
    /// Newer release exists but this machine is not in its rollout yet.
    Staged,
    /// A new major version: only with `--to`.
    MajorNeedsChoice,
    Paused,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub current: String,
    pub channel: String,
    pub reason: Reason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Release>,
    /// The newest version seen, even when not offered (for "1.2 is coming").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest: Option<String>,
}

pub struct Inputs<'a> {
    pub current: &'a str,
    pub feed: &'a Feed,
    pub to: Option<&'a str>,
    pub machine_id: &'a str,
    pub paused: bool,
}

pub fn plan(i: &Inputs) -> Result<Plan> {
    let cur = Version::parse(i.current)
        .ok_or_else(|| Error::invalid(format!("“{}” is not a WolOS version.", i.current)))?;
    let mut rel: Vec<(Version, &Release)> = i
        .feed
        .releases
        .iter()
        .filter_map(|r| Some((Version::parse(&r.version)?, r)))
        .collect();
    rel.sort_by(|a, b| b.0.cmp(&a.0));
    let newest = rel.first().map(|(v, _)| v.to_string());
    let mk = |reason, target: Option<&Release>| Plan {
        current: cur.to_string(),
        channel: i.feed.channel.clone(),
        reason,
        target: target.cloned(),
        newest: newest.clone(),
    };
    if let Some(to) = i.to {
        let Some((v, r)) = rel.iter().find(|(v, _)| v.matches(to)) else {
            return Err(Error::new(
                crate::ffi::Status::NotFound,
                format!("WolOS {to} is not in the {} channel.", i.feed.channel),
            ));
        };
        return match v.cmp(&cur) {
            Ordering::Greater => Ok(mk(Reason::Available, Some(r))),
            Ordering::Equal => Ok(mk(Reason::UpToDate, None)),
            Ordering::Less => Err(Error::invalid(format!(
                "WolOS {v} is older than this system ({cur}). To go back, use `wopkg rollback system`."
            ))),
        };
    }
    let newer: Vec<&(Version, &Release)> = rel.iter().filter(|(v, _)| *v > cur).collect();
    if newer.is_empty() {
        return Ok(mk(Reason::UpToDate, None));
    }
    let same_major: Vec<&&(Version, &Release)> =
        newer.iter().filter(|(v, _)| v.major == cur.major).collect();
    if let Some(s) = same_major.iter().find(|(_, r)| r.security) {
        return Ok(mk(Reason::Available, Some(s.1)));
    }
    if i.paused {
        return Ok(mk(Reason::Paused, None));
    }
    for (v, r) in same_major {
        if bucket(i.machine_id, &v.to_string()) < r.rollout {
            return Ok(mk(Reason::Available, Some(r)));
        }
    }
    if newer.iter().any(|(v, _)| v.major == cur.major) {
        return Ok(mk(Reason::Staged, None));
    }
    Ok(mk(Reason::MajorNeedsChoice, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(v: &str, rollout: u8, security: bool) -> Release {
        Release {
            version: v.into(),
            manifest: "{}".into(),
            signature: String::new(),
            rollout,
            notes: String::new(),
            security,
        }
    }
    fn feed(rs: Vec<Release>) -> Feed {
        Feed {
            format: 1,
            channel: "stable".into(),
            releases: rs,
        }
    }

    #[test]
    fn version_order() {
        let order = [
            "0.11.2",
            "1.0.0-beta.1",
            "1.0.0-beta.2",
            "1.0.0-beta.10",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1",
            "2.0.0",
        ];
        let vs: Vec<Version> = order.iter().map(|s| Version::parse(s).unwrap()).collect();
        for w in vs.windows(2) {
            assert!(w[0] < w[1], "{} < {}", w[0], w[1]);
        }
        assert_eq!(Version::parse("1.1").unwrap().to_string(), "1.1.0");
        for bad in ["", "x", "1.2.3.4", "1.-", "-1"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(
            Version::parse("1.1.4").unwrap().matches("1.1")
                && !Version::parse("1.2.0").unwrap().matches("1.1")
        );
    }

    #[test]
    fn plan_table() {
        let f = feed(vec![
            r("1.0.1", 100, false),
            r("1.1.0", 100, false),
            r("2.0.0", 100, false),
        ]);
        let p = |cur, to, paused| {
            plan(&Inputs {
                current: cur,
                feed: &f,
                to,
                machine_id: "m",
                paused,
            })
        };
        assert_eq!(
            p("1.0.0", None, false).unwrap().target.unwrap().version,
            "1.1.0",
            "newest in the same major"
        );
        assert_eq!(
            p("1.1.0", None, false).unwrap().reason,
            Reason::MajorNeedsChoice,
            "a new major is a choice"
        );
        assert_eq!(
            p("1.1.0", Some("2.0"), false)
                .unwrap()
                .target
                .unwrap()
                .version,
            "2.0.0"
        );
        assert_eq!(p("2.0.0", None, false).unwrap().reason, Reason::UpToDate);
        assert!(
            p("2.0.0", Some("1.1"), false).is_err(),
            "no downgrade, use rollback"
        );
        assert!(p("1.0.0", Some("3"), false).is_err());
        assert_eq!(p("1.0.0", None, true).unwrap().reason, Reason::Paused);
        assert_eq!(
            p("1.0.0", Some("1.1"), true).unwrap().reason,
            Reason::Available,
            "an explicit choice ignores the pause"
        );
    }

    #[test]
    fn rollout_and_security() {
        let f = feed(vec![r("1.1.0", 0, false)]);
        let p = plan(&Inputs {
            current: "1.0.0",
            feed: &f,
            to: None,
            machine_id: "m",
            paused: false,
        })
        .unwrap();
        assert_eq!(
            (p.reason, p.newest.as_deref()),
            (Reason::Staged, Some("1.1.0"))
        );
        let f = feed(vec![r("1.0.2", 0, true)]);
        let p = plan(&Inputs {
            current: "1.0.0",
            feed: &f,
            to: None,
            machine_id: "m",
            paused: true,
        })
        .unwrap();
        assert_eq!(
            p.reason,
            Reason::Available,
            "security fixes skip pause and rollout"
        );
        let ids: Vec<u8> = (0..1000)
            .map(|i| bucket(&format!("machine-{i}"), "1.1.0"))
            .collect();
        let half = ids.iter().filter(|b| **b < 50).count();
        assert!((400..600).contains(&half), "{half}");
        assert_eq!(bucket("a", "1.1.0"), bucket("a", "1.1.0"));
    }

    #[test]
    fn feeds_must_be_signed() {
        let seed = [3u8; 32];
        let text = serde_json::to_vec(&feed(vec![r("1.1.0", 100, false)])).unwrap();
        let keys = [wol_core::package::public_of(&seed)];
        assert!(Feed::verify(&text, &wol_core::package::sign(&seed, &text), &keys).is_ok());
        assert!(Feed::verify(&text, &wol_core::package::sign(&[4; 32], &text), &keys).is_err());
        assert!(Feed::verify(&text, b"", &keys).is_err());
    }
}

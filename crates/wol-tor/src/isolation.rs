// SPDX-License-Identifier: GPL-2.0-or-later
//! The tor credential of each stream (ADR 0053 D3): different sites never share a circuit, and
//! "New circuit for this site" moves one site without touching the others.
//!
//! tor's `IsolateSOCKSAuth` puts streams with different username/password on different circuits.
//! The relay always sends a credential, built here as username `<group>/<key>` and password
//! `<epoch>[.<browser epoch>]`:
//! - with engine patch 0007 the browser offers username = top-frame site (from the request's
//!   `NetworkAnonymizationKey`, real public-suffix rules) and that is the key;
//! - without it (the repackaged binary) the key is the destination's site, approximated without a
//!   public-suffix list ([`site_of`]): coarser than first-party isolation (an embedded tracker
//!   shares its own circuit across sites) but never two sites on one circuit.
//!
//! Epochs only grow and live in memory: the group's tor is new at each start anyway.
use crate::group::Group;
use crate::socks::{Host, Target};
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// Longest key kept whole (username must fit SOCKS5's 255 bytes with the group prefix).
pub const MAX_KEY: usize = 200;

/// The site of a host as the relay sees it: IP literals stay whole; for names, the last two
/// labels, or three when the last is a two-letter country code under a short second level
/// (`example.co.uk`, `example.com.br`). Approximate on purpose (no public-suffix list in the
/// relay); the patched engine sends the exact site.
pub fn site_of(host: &Host) -> String {
    match host {
        Host::V4(a) => a.to_string(),
        Host::V6(a) => a.to_string(),
        Host::Domain(d) => site_of_name(d),
    }
}

fn site_of_name(name: &str) -> String {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = name.split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    let take = if n >= 3
        && labels[n - 1].len() == 2
        && labels[n - 2].len() <= 3
        && labels[n - 1] != "io"
    {
        3
    } else {
        2
    };
    let site = labels[n.saturating_sub(take)..].join(".");
    clip(&site)
}

fn clip(s: &str) -> String {
    if s.len() <= MAX_KEY {
        return s.to_string();
    }
    let mut start = s.len() - MAX_KEY;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

/// A key the browser offered is used only if it is plain text a site name can be.
fn clean_offered(user: &str) -> Option<String> {
    let ok = !user.is_empty()
        && user.len() <= MAX_KEY
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_:[]".contains(&b));
    ok.then(|| user.to_ascii_lowercase())
}

fn clean_epoch(pass: &str) -> Option<&str> {
    (!pass.is_empty() && pass.len() <= 32 && pass.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then_some(pass)
}

/// The credentials of one group.
#[derive(Debug)]
pub struct Isolator {
    group: Group,
    epochs: Mutex<HashMap<String, u64>>,
    /// Keys the browser has offered (exact registrable domains with private registries).
    offered: Mutex<BTreeSet<String>>,
}

/// Most offered keys remembered (a Tor tab session visits far fewer sites).
const MAX_OFFERED: usize = 4096;

impl Isolator {
    /// A new group starts every site at epoch 0.
    pub fn new(group: Group) -> Self {
        Self {
            group,
            epochs: Mutex::new(HashMap::new()),
            offered: Mutex::new(BTreeSet::new()),
        }
    }

    /// The key of a stream: the browser's site when it offered a usable one (remembered, so
    /// [`Self::key_of_site`] agrees with it), else the target's.
    fn key(&self, offered: Option<&(String, String)>, target: &Target) -> String {
        match offered.and_then(|(u, _)| clean_offered(u)) {
            Some(k) => {
                let mut seen = self.offered.lock().unwrap_or_else(|p| p.into_inner());
                if seen.len() < MAX_OFFERED || seen.contains(&k) {
                    seen.insert(k.clone());
                }
                k
            }
            None => site_of(&target.host),
        }
    }

    /// The key a host or site name belongs to: the longest key the browser offered that is the
    /// name or a parent of it (so `www.example.github.io` is `example.github.io`, never
    /// `github.io`), else the destination rule of [`site_of`]. This is the one mapping used by
    /// "New circuit for this site"; streams use the same keys.
    pub fn key_of_site(&self, name: &str) -> String {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let seen = self.offered.lock().unwrap_or_else(|p| p.into_inner());
        seen.iter()
            .filter(|k| name == **k || name.ends_with(&format!(".{k}")))
            .max_by_key(|k| k.len())
            .cloned()
            .unwrap_or_else(|| site_of_name(&name))
    }

    /// The tor username and password for a stream.
    pub fn credential(
        &self,
        offered: Option<&(String, String)>,
        target: &Target,
    ) -> (String, String) {
        let key = self.key(offered, target);
        let epoch = self
            .epochs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .copied()
            .unwrap_or(0);
        let pass = match offered.and_then(|(_, p)| clean_epoch(p)) {
            Some(b) => format!("{epoch}.{b}"),
            None => epoch.to_string(),
        };
        (format!("{}/{key}", self.group), pass)
    }

    /// "New circuit for this site": the next stream to the site's key gets a fresh circuit.
    /// `site` is a host or a site; it is reduced by [`Self::key_of_site`]. Returns the epoch.
    pub fn new_circuit(&self, site: &str) -> u64 {
        let key = self.key_of_site(site);
        let mut m = self.epochs.lock().unwrap_or_else(|p| p.into_inner());
        let e = m.entry(key).or_insert(0);
        *e += 1;
        *e
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn t(h: &str) -> Target {
        Target {
            host: Host::Domain(h.into()),
            port: 443,
        }
    }

    #[test]
    fn site_keys_follow_the_registrable_domain() {
        assert_eq!(site_of_name("www.example.org"), "example.org");
        assert_eq!(site_of_name("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(site_of_name("shop.example.com.br"), "example.com.br");
        assert_eq!(site_of_name("cdn.thing.io"), "thing.io");
        assert_eq!(site_of_name("example.org."), "example.org");
        assert_eq!(site_of_name("localhost"), "localhost");
        assert_eq!(
            site_of_name("abc.duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion"),
            "duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion"
        );
        assert_eq!(site_of(&Host::V4(Ipv4Addr::new(192, 0, 2, 7))), "192.0.2.7");
    }

    #[test]
    fn different_sites_get_different_credentials() {
        let iso = Isolator::new(Group::new("g").unwrap());
        let a = iso.credential(None, &t("www.example.org"));
        let b = iso.credential(None, &t("static.example.org"));
        let c = iso.credential(None, &t("tracker.example.net"));
        assert_eq!(a, b, "one site, one circuit");
        assert_ne!(a, c);
        assert_eq!(a, ("g/example.org".into(), "0".into()));
    }

    #[test]
    fn groups_never_share_a_credential() {
        let one = Isolator::new(Group::new("one").unwrap()).credential(None, &t("example.org"));
        let two = Isolator::new(Group::new("two").unwrap()).credential(None, &t("example.org"));
        assert_ne!(one, two);
    }

    #[test]
    fn browser_offered_site_wins_over_destination() {
        let iso = Isolator::new(Group::new("g").unwrap());
        let offered = ("news.example".to_string(), "17".to_string());
        let (u, p) = iso.credential(Some(&offered), &t("tracker.example.net"));
        assert_eq!(u, "g/news.example");
        assert_eq!(p, "0.17");
        let junk = ("a/b\u{1}".to_string(), "x y".to_string());
        assert_eq!(
            iso.credential(Some(&junk), &t("tracker.example.net")),
            ("g/example.net".into(), "0".into())
        );
    }

    #[test]
    fn new_circuit_moves_only_that_site() {
        let iso = Isolator::new(Group::new("g").unwrap());
        let before_a = iso.credential(None, &t("example.org"));
        let before_b = iso.credential(None, &t("example.net"));
        assert_eq!(iso.new_circuit("www.example.org"), 1);
        assert_ne!(iso.credential(None, &t("example.org")), before_a);
        assert_eq!(iso.credential(None, &t("example.net")), before_b);
        assert_eq!(iso.new_circuit("example.org"), 2);
    }

    #[test]
    fn new_circuit_bumps_the_key_the_browser_offered() {
        // Patch 0007 sends the registrable domain with private registries: for a page on
        // www.example.github.io that is example.github.io, not github.io.
        let iso = Isolator::new(Group::new("g").unwrap());
        let offered = ("example.github.io".to_string(), "0".to_string());
        let other = ("other.github.io".to_string(), "0".to_string());
        let before = iso.credential(Some(&offered), &t("cdn.thing.net"));
        let before_other = iso.credential(Some(&other), &t("cdn.thing.net"));
        assert_eq!(before.0, "g/example.github.io");
        assert_eq!(iso.new_circuit("www.example.github.io"), 1);
        assert_ne!(iso.credential(Some(&offered), &t("cdn.thing.net")), before);
        assert_eq!(
            iso.credential(Some(&other), &t("cdn.thing.net")),
            before_other,
            "a sibling site on the same public suffix keeps its circuit"
        );
        assert_eq!(iso.new_circuit("example.github.io"), 2);
    }

    #[test]
    fn one_function_maps_a_site_name_to_its_key() {
        let iso = Isolator::new(Group::new("g").unwrap());
        let offered = ("example.github.io".to_string(), "0".to_string());
        let _ = iso.credential(Some(&offered), &t("x.net"));
        for name in [
            "example.github.io",
            "WWW.Example.GitHub.io.",
            "a.b.example.github.io",
        ] {
            assert_eq!(iso.key_of_site(name), "example.github.io", "{name}");
        }
        assert_eq!(
            iso.key_of_site("www.example.org"),
            "example.org",
            "no offer: Tier 0 rule"
        );
    }

    #[test]
    fn long_names_fit_the_socks_username() {
        let long = format!("{}.example.org", "a".repeat(250));
        let iso = Isolator::new(Group::new(&"g".repeat(32)).unwrap());
        let (u, _) = iso.credential(None, &t(&format!("x.{long}")));
        assert!(u.len() <= 255);
        assert!(clip(&"é".repeat(150)).len() <= MAX_KEY);
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
//! The `plugins` source (ADR 0031 W4, docs/modules/plugins.md "Distribution"): the signed plugin
//! index (`index.json` + `index.json.sig`, Ed25519 by the key in `/usr/share/wolos/keys/`) and the
//! check of one downloaded `.wolplug` against its index entry. Plugins install for the person
//! only, so nothing here asks wold for anything. Order of trust: the index signature before the
//! index is parsed; the entry's size and SHA-256 before the download is decompressed; the
//! `.wolplug`'s own signature before its files are read (`wol_core::package::verify_wolplug`).
//!
//! `index.json`, format 1:
//! `{"format":1,"plugins":[{"id","name","version","summary","author","license","kinds":[..],
//! "permissions":[..],"min_wolos","url","sha256","size"}]}`, and from the hub (ADR 0047) the optional
//! "description", "icon" and "screenshots" (pictures by CDN file name `<sha256>.<png|webp>`). An entry that breaks a rule is
//! dropped on its own (the rest of a signed index stays usable); a second entry with the same id
//! is dropped too.
use crate::ffi::{Error, Result};
use serde::{Deserialize, Serialize};
use shared::plugins::{Kind, Permission, is_plugin_id, is_wolos_version};
use std::io::Read;
use wol_core::package::{
    Trust, VerifiedPlugin, WOLPLUG_MAX_FILE, WOLPLUG_MAX_TAR, require_signed, verify_index,
    verify_wolplug,
};

/// Largest `index.json` read.
pub const MAX_INDEX: usize = 4 << 20;
/// Entries kept from one index.
pub const MAX_ENTRIES: usize = 5000;
/// Where the index lives: the CDN, which the release tooling publishes it to (signed; an empty
/// list until plugins are reviewed). e2e D5: the old host, plugins.wolos.org, never resolved.
pub const DEFAULT_URL: &str = "https://cdn.wolos.org/plugins/index.json";
const MAX_URL: usize = 512;
const MAX_TEXT: usize = 200;

/// One plugin as the index lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub author: String,
    pub license: String,
    /// `shared::plugins::Kind` ids: what it adds (menubar, widget, theme, ...).
    pub kinds: Vec<String>,
    /// `shared::plugins::Permission` ids, shown before Install.
    pub permissions: Vec<String>,
    /// Lowest WolOS version it runs on; empty = any.
    pub min_wolos: String,
    /// `https://` URL of the `.wolplug`.
    pub url: String,
    /// SHA-256 of the `.wolplug` file, lowercase hex.
    pub sha256: String,
    /// Bytes of the `.wolplug` file.
    pub size: u64,
    /// The package's own description (hub index; empty = none): plain text, at most
    /// `shared::package::MAX_DESCRIPTION` characters.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The listing's icon (hub index; empty = none): the CDN file name `<sha256>.<png|webp>`
    /// (`shared::package::picture_url`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// Up to `shared::package::MAX_SCREENSHOTS` screenshots, file names like `icon`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PluginIndex {
    pub plugins: Vec<Entry>,
}

/// The signed freshness of one list (audit C6-2), kept apart from what callers list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Freshness {
    /// Increasing publication number; 0 in lists made before it existed.
    serial: u64,
    /// End of validity, Unix seconds; 0 = none.
    expires: u64,
}

#[derive(Deserialize)]
struct Raw {
    format: u32,
    #[serde(default)]
    serial: u64,
    #[serde(default)]
    expires: u64,
    #[serde(default)]
    plugins: Vec<serde_json::Value>,
}

fn text_ok(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control)
}

fn hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn url_ok(s: &str) -> bool {
    s.len() <= MAX_URL
        && s.strip_prefix("https://").is_some_and(|r| {
            r.split('/').next().is_some_and(|h| !h.is_empty())
                && r.ends_with(".wolplug")
                && r.bytes().all(|b| b.is_ascii_graphic())
        })
}

impl Entry {
    /// Every rule an entry must keep to be listed.
    pub fn is_valid(&self) -> bool {
        is_plugin_id(&self.id)
            && !self.id.starts_with(shared::plugins::RESERVED_PREFIX)
            && !self.name.trim().is_empty()
            && text_ok(&self.name, shared::plugins::MAX_NAME)
            && shared::package::is_version(&self.version)
            && text_ok(&self.summary, MAX_TEXT)
            && text_ok(&self.author, MAX_TEXT)
            && text_ok(&self.license, 64)
            && !self.kinds.is_empty()
            && self.kinds.len() <= 16
            && self.kinds.iter().all(|k| Kind::parse(k).is_some())
            && self.permissions.len() <= 16
            && self
                .permissions
                .iter()
                .all(|p| Permission::parse(p).is_some())
            && (self.min_wolos.is_empty() || is_wolos_version(&self.min_wolos))
            && url_ok(&self.url)
            && hex64(&self.sha256)
            && self.size > 0
            && self.size <= WOLPLUG_MAX_FILE
            && shared::package::is_description(&self.description)
            && (self.icon.is_empty() || shared::package::is_picture_file(&self.icon))
            && self.screenshots.len() <= shared::package::MAX_SCREENSHOTS
            && self
                .screenshots
                .iter()
                .all(|s| shared::package::is_picture_file(s))
    }

    /// Permission labels for the install sheet, in index order.
    pub fn permission_labels(&self) -> Vec<&'static str> {
        self.permissions
            .iter()
            .filter_map(|p| Permission::parse(p))
            .map(Permission::label)
            .collect()
    }
}

/// Pure parse of `index.json` bytes (fuzzed; never panics). Unknown format is an error; bad
/// entries are dropped one by one.
pub fn parse(text: &[u8]) -> Result<PluginIndex> {
    parse_full(text).map(|(i, _)| i)
}

fn parse_full(text: &[u8]) -> Result<(PluginIndex, Freshness)> {
    if text.len() > MAX_INDEX {
        return Err(Error::untrusted("The plugin list is too large."));
    }
    let raw: Raw = serde_json::from_slice(text)
        .map_err(|_| Error::untrusted("The plugin list is damaged."))?;
    if raw.format != 1 {
        return Err(Error::untrusted("The plugin list is in an unknown format."));
    }
    let mut plugins: Vec<Entry> = Vec::new();
    for v in raw.plugins {
        if plugins.len() >= MAX_ENTRIES {
            break;
        }
        let Ok(e) = serde_json::from_value::<Entry>(v) else {
            continue;
        };
        if e.is_valid() && !plugins.iter().any(|p| p.id == e.id) {
            plugins.push(e);
        }
    }
    Ok((
        PluginIndex { plugins },
        Freshness {
            serial: raw.serial,
            expires: raw.expires,
        },
    ))
}

/// The index only counts when signed by a trusted key: the signature is checked first.
pub fn verify(text: &[u8], sig: Option<&[u8]>, keys: &[[u8; 32]]) -> Result<PluginIndex> {
    verify_full(text, sig, keys).map(|(i, _)| i)
}

fn verify_full(
    text: &[u8],
    sig: Option<&[u8]>,
    keys: &[[u8; 32]],
) -> Result<(PluginIndex, Freshness)> {
    match verify_index(text, sig, keys) {
        Ok(Trust::Signed(_)) => parse_full(text),
        _ => Err(Error::untrusted(
            "The plugin list is not signed by the WolOS plugin index.",
        )),
    }
}

impl PluginIndex {
    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.plugins.iter().find(|e| e.id == id)
    }

    /// Case-insensitive match on id, name, summary and author; name hits first.
    pub fn search(&self, term: &str) -> Vec<&Entry> {
        let t = term.trim().to_lowercase();
        if t.is_empty() {
            return self.plugins.iter().collect();
        }
        let mut hits: Vec<(u8, &Entry)> = self
            .plugins
            .iter()
            .filter_map(|e| {
                let rank = if e.name.to_lowercase().contains(&t) {
                    0
                } else if e.id.contains(&t) {
                    1
                } else if e.summary.to_lowercase().contains(&t)
                    || e.author.to_lowercase().contains(&t)
                {
                    2
                } else {
                    return None;
                };
                Some((rank, e))
            })
            .collect();
        hits.sort_by_key(|h| h.0);
        hits.into_iter().map(|h| h.1).collect()
    }
}

/// zstd frame -> tar, stopping one byte past [`WOLPLUG_MAX_TAR`] so a bomb is cut short.
pub fn unpack(wolplug: &[u8]) -> Result<Vec<u8>> {
    let damaged = || Error::untrusted("The plugin file is damaged.");
    if wolplug.len() as u64 > WOLPLUG_MAX_FILE {
        return Err(Error::untrusted("The plugin file is too large."));
    }
    let dec = ruzstd::decoding::StreamingDecoder::new(wolplug).map_err(|_| damaged())?;
    let mut out = Vec::new();
    dec.take(WOLPLUG_MAX_TAR + 1)
        .read_to_end(&mut out)
        .map_err(|_| damaged())?;
    if out.len() as u64 > WOLPLUG_MAX_TAR {
        return Err(Error::untrusted("The plugin file is too large."));
    }
    Ok(out)
}

/// A downloaded `.wolplug` for `entry`: size and SHA-256 as listed, then unpacked with a bound,
/// then its own signature, hashes and manifest; it must be the id and version listed.
pub fn check_download(entry: &Entry, file: &[u8], keys: &[[u8; 32]]) -> Result<VerifiedPlugin> {
    if file.len() as u64 != entry.size || wol_core::package::sha256_hex(file) != entry.sha256 {
        return Err(Error::untrusted(
            "The plugin download does not match the plugin list.",
        ));
    }
    let tar = unpack(file)?;
    let v = verify_wolplug(&tar, keys)
        .and_then(require_signed)
        .map_err(Error::untrusted)?;
    if v.manifest.id != entry.id || v.manifest.version != entry.version {
        return Err(Error::untrusted(
            "The plugin download is not the plugin the list names.",
        ));
    }
    // The list's permissions are the sentence a person consents to ("Asks for nothing"), and Turn
    // On grants what the package's own manifest declares: the two must be the same set, or a
    // listing could understate what the plugin gets (audit6 ST6-5).
    if !same_permissions(&entry.permissions, &v.manifest.permissions) {
        return Err(Error::untrusted(
            "The plugin download asks for other permissions than the plugin list says.",
        ));
    }
    Ok(v)
}

/// The index entry's permission ids and the manifest's permissions name the same set (order and
/// repeats do not matter; an id that is no permission never matches).
fn same_permissions(listed: &[String], declared: &[Permission]) -> bool {
    let mut a: Vec<Permission> = Vec::new();
    for p in listed {
        match Permission::parse(p) {
            Some(p) => a.push(p),
            None => return false,
        }
    }
    let mut b = declared.to_vec();
    for v in [&mut a, &mut b] {
        v.sort_by_key(|p| p.id());
        v.dedup();
    }
    a == b
}

// ---- Network and cache (blocking: callers run these off the UI thread) ----

/// Under `$XDG_CACHE_HOME`: `index.json` and `index.json.sig`, kept only once verified.
pub const CACHE_DIR: &str = "wolos/plugin-index";
const INDEX_FILE: &str = "index.json";
const SIG_FILE: &str = "index.json.sig";

/// GET `url` into memory, at most `max` bytes (a longer body is refused, not cut).
fn get_bounded(net: &crate::fetch::Net, url: &str, max: u64) -> Result<Vec<u8>> {
    net.check_url(url)?;
    let resp = net
        .agent
        .get(url)
        .call()
        .map_err(|_| Error::unavailable("The plugin list could not be reached."))?;
    if resp.status().as_u16() != 200 {
        return Err(Error::unavailable("The plugin list could not be reached."));
    }
    let mut out = Vec::new();
    resp.into_body()
        .into_reader()
        .take(max + 1)
        .read_to_end(&mut out)
        .map_err(|_| Error::unavailable("The download stopped."))?;
    if out.len() as u64 > max {
        return Err(Error::untrusted(
            "The download is larger than it should be.",
        ));
    }
    Ok(out)
}

/// The cached list once its signature checks, with its freshness; expiry is not looked at here.
fn cached_full(dir: &std::path::Path, keys: &[[u8; 32]]) -> Result<(PluginIndex, Freshness)> {
    let text = shared::bounded::read_file(&dir.join(INDEX_FILE), MAX_INDEX as u64)
        .ok_or_else(|| Error::unavailable("The plugin list has not been downloaded yet."))?;
    let sig = shared::bounded::read_file(&dir.join(SIG_FILE), 64);
    verify_full(&text, sig.as_deref(), keys)
}

/// The cached index in `dir`, checked again on every read (a changed cache is refused), and
/// refused once past its signed `expires` (audit C10-3: a blocked index URL must not freeze the
/// list silently).
pub fn cached(dir: &std::path::Path, keys: &[[u8; 32]]) -> Result<PluginIndex> {
    let (index, f) = cached_full(dir, keys)?;
    crate::release::check_fresh(
        "plugin list",
        f.serial,
        f.expires,
        0,
        crate::release::unix_now(),
    )?;
    Ok(index)
}

/// A cached list older than this is downloaded again when the Store opens (lists without an
/// `expires` too).
pub const MAX_AGE: u64 = 24 * 3600;

/// Whether a cached list written at `fetched` should be downloaded again at `now`: once half of
/// the time it had left at download is gone, or after [`MAX_AGE`], whichever comes first.
fn is_stale(fetched: u64, expires: u64, now: u64) -> bool {
    let half = if expires == 0 {
        MAX_AGE
    } else {
        (expires.saturating_sub(fetched) / 2).min(MAX_AGE)
    };
    now.saturating_sub(fetched) >= half
}

/// The list for the Store: a download when `fresh` (the cached copy when that fails); else the
/// cached copy, downloaded again when it is missing, expired or stale ([`is_stale`]), keeping the
/// still-valid cached copy when that download fails.
pub fn load(
    net: &crate::fetch::Net,
    url: &str,
    dir: &std::path::Path,
    keys: &[[u8; 32]],
    fresh: bool,
) -> Result<PluginIndex> {
    if fresh {
        return refresh(net, url, dir, keys).or_else(|e| cached(dir, keys).map_err(|_| e));
    }
    let now = crate::release::unix_now();
    let fetched = std::fs::metadata(dir.join(INDEX_FILE))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    match cached_full(dir, keys) {
        Ok((index, f))
            if crate::release::check_fresh("plugin list", f.serial, f.expires, 0, now).is_ok() =>
        {
            if is_stale(fetched, f.expires, now) {
                Ok(refresh(net, url, dir, keys).unwrap_or(index))
            } else {
                Ok(index)
            }
        }
        _ => refresh(net, url, dir, keys),
    }
}

/// Downloads `url` and `url.sig`, verifies them, and only then replaces the cache in `dir`
/// (unique temp names, then rename). A list that does not verify leaves the old cache alone.
pub fn refresh(
    net: &crate::fetch::Net,
    url: &str,
    dir: &std::path::Path,
    keys: &[[u8; 32]],
) -> Result<PluginIndex> {
    let text = get_bounded(net, url, MAX_INDEX as u64)?;
    let sig = get_bounded(net, &format!("{url}.sig"), 64)?;
    let (index, f) = verify_full(&text, Some(&sig), keys)?;
    // Replay guard (audit C6-2): never older than the cached list (expired or not), never past
    // its own expiry.
    let last = cached_full(dir, keys).map_or(0, |(_, c)| c.serial);
    crate::release::check_fresh(
        "plugin list",
        f.serial,
        f.expires,
        last,
        crate::release::unix_now(),
    )?;
    let cant = || Error::unavailable("The plugin list cannot be saved.");
    std::fs::create_dir_all(dir).map_err(|_| cant())?;
    for (name, bytes) in [(SIG_FILE, &sig), (INDEX_FILE, &text)] {
        shared::atomic::atomic_write(&dir.join(name), 0o644, bytes).map_err(|_| cant())?;
    }
    Ok(index)
}

/// Downloads `entry`'s `.wolplug` (at most its listed size) and checks it ([`check_download`]).
pub fn download(
    net: &crate::fetch::Net,
    entry: &Entry,
    keys: &[[u8; 32]],
) -> Result<VerifiedPlugin> {
    let file = get_bounded(net, &entry.url, entry.size)?;
    check_download(entry, &file, keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [3; 32];

    fn entry(id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": "Weather", "version": "1.2", "summary": "Forecast in the menu bar",
            "author": "Alice", "license": "MIT", "kinds": ["menubar"], "permissions": ["http"],
            "min_wolos": "0.12", "url": format!("https://plugins.wolos.org/{id}-1.2.wolplug"),
            "sha256": "ab".repeat(32), "size": 4096
        })
    }

    fn index(plugins: Vec<serde_json::Value>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"format": 1, "plugins": plugins})).unwrap()
    }

    /// The Store reads the list from the CDN over TLS; a signed empty list is a valid list (what
    /// is published until plugins are reviewed), an unsigned one is not.
    #[test]
    fn the_list_comes_from_the_cdn_and_may_be_empty() {
        assert_eq!(DEFAULT_URL, "https://cdn.wolos.org/plugins/index.json");
        let text = index(vec![]);
        let keys = [wol_core::package::public_of(&SEED)];
        let sig = wol_core::package::sign(&SEED, &text);
        assert!(verify(&text, Some(&sig), &keys).unwrap().plugins.is_empty());
        assert!(verify(&text, None, &keys).is_err());
    }

    #[test]
    fn a_signed_index_lists_its_valid_entries() {
        let text = index(vec![entry("alice.weather"), entry("bob.clock")]);
        let sig = wol_core::package::sign(&SEED, &text);
        let keys = [wol_core::package::public_of(&SEED)];
        let i = verify(&text, Some(&sig), &keys).unwrap();
        assert_eq!(i.plugins.len(), 2);
        let w = i.get("alice.weather").unwrap();
        assert_eq!(w.version, "1.2");
        assert_eq!(w.permission_labels().len(), 1);
        assert!(verify(&text, None, &keys).is_err(), "unsigned");
        assert!(
            verify(&text, Some(&[0; 64]), &keys).is_err(),
            "bad signature"
        );
        let mut changed = text.clone();
        changed[20] ^= 1;
        assert!(
            verify(&changed, Some(&sig), &keys).is_err(),
            "changed after signing"
        );
    }

    /// The hub index (hub-pipeline schema/index.schema.json) adds the package's description and
    /// its pictures by CDN file name; an index without them reads as before.
    #[test]
    fn the_listing_description_and_pictures_are_read() {
        let sha = "95d648d3d175b7df65af3f36caacaacc448897768f22945b5637685a8d5242f6";
        let (icon, shot) = (format!("{sha}.png"), format!("{sha}.webp"));
        let mut e = entry("alice.weather");
        e["description"] = serde_json::json!("Forecast.\nIn the menu bar.");
        e["icon"] = serde_json::json!(icon);
        e["screenshots"] = serde_json::json!([shot, icon]);
        let i = parse(&index(vec![e, entry("bob.clock")])).unwrap();
        let w = i.get("alice.weather").unwrap();
        assert_eq!(w.description, "Forecast.\nIn the menu bar.");
        assert_eq!(w.icon, icon);
        assert_eq!(w.screenshots, [shot.clone(), icon.clone()]);
        assert_eq!(
            shared::package::picture_url(&w.icon).as_deref(),
            Some(format!("https://cdn.wolos.org/plugins/pictures/{icon}").as_str())
        );
        let old = i.get("bob.clock").unwrap();
        assert!(old.description.is_empty() && old.icon.is_empty() && old.screenshots.is_empty());
        let text = serde_json::to_string(old).unwrap();
        assert!(
            !text.contains("description")
                && !text.contains("icon")
                && !text.contains("screenshots")
        );
        // Out of bounds: the entry is dropped, the rest of the list stays.
        for (k, v) in [
            ("description", serde_json::json!("x".repeat(4001))),
            ("description", serde_json::json!("a\u{1b}b")),
            ("icon", serde_json::json!(sha)),
            (
                "icon",
                serde_json::json!("https://cdn.wolos.org/plugins/pictures/x.png"),
            ),
            ("screenshots", serde_json::json!(vec![shot.clone(); 9])),
            ("screenshots", serde_json::json!([format!("{sha}.gif")])),
        ] {
            let mut e = entry("alice.weather");
            e[k] = v.clone();
            let i = parse(&index(vec![e, entry("bob.clock")])).unwrap();
            assert!(i.get("alice.weather").is_none(), "{k}={v}");
            assert!(i.get("bob.clock").is_some());
        }
    }

    #[test]
    fn bad_entries_are_dropped_one_by_one() {
        let mut bad = vec![];
        for (k, v) in [
            ("id", serde_json::json!("NoDots")),
            ("id", serde_json::json!("org.wolos.fake")),
            (
                "url",
                serde_json::json!("http://plugins.wolos.org/x.wolplug"),
            ),
            ("url", serde_json::json!("https://plugins.wolos.org/x.tar")),
            ("sha256", serde_json::json!("xyz")),
            ("size", serde_json::json!(0)),
            ("size", serde_json::json!(WOLPLUG_MAX_FILE + 1)),
            ("kinds", serde_json::json!(["daemon"])),
            ("kinds", serde_json::json!([])),
            ("permissions", serde_json::json!(["root"])),
            ("min_wolos", serde_json::json!("soon")),
            ("name", serde_json::json!("A\u{7}bell")),
            ("version", serde_json::json!("1 2")),
            ("size", serde_json::json!("big")),
        ] {
            let mut e = entry("alice.bad");
            e[k] = v;
            bad.push(e);
        }
        bad.push(serde_json::json!("not an object"));
        bad.push(entry("alice.good"));
        bad.push(entry("alice.good"));
        let i = parse(&index(bad)).unwrap();
        let ids: Vec<&str> = i.plugins.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["alice.good"]);
    }

    #[test]
    fn parse_is_bounded_and_total() {
        assert!(parse(b"").is_err());
        assert!(parse(b"{\"format\":2,\"plugins\":[]}").is_err());
        assert!(parse(&vec![b' '; MAX_INDEX + 1]).is_err());
        assert_eq!(parse(b"{\"format\":1}").unwrap(), PluginIndex::default());
        // Cut and flipped inputs: errors or smaller lists, never a panic.
        let text = index(vec![entry("alice.weather")]);
        for cut in 0..text.len() {
            let _ = parse(&text[..cut]);
        }
        let mut x: u32 = 7;
        for _ in 0..200 {
            let mut t = text.clone();
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let i = (x as usize) % t.len();
            t[i] = (x >> 16) as u8;
            let _ = parse(&t);
        }
    }

    #[test]
    fn search_ranks_names_first() {
        let mut a = entry("alice.weather");
        a["summary"] = serde_json::json!("Shows the clock too");
        let mut b = entry("bob.clock");
        b["name"] = serde_json::json!("Clock");
        let i = parse(&index(vec![a, b])).unwrap();
        let hits: Vec<&str> = i.search("CLOCK").iter().map(|e| e.id.as_str()).collect();
        assert_eq!(hits, ["bob.clock", "alice.weather"]);
        assert_eq!(i.search(" ").len(), 2);
        assert!(i.search("nothing").is_empty());
    }

    #[test]
    fn a_download_must_match_its_entry_before_it_is_unpacked() {
        let e: Entry = serde_json::from_value(entry("alice.weather")).unwrap();
        let keys = [wol_core::package::public_of(&SEED)];
        let err = check_download(&e, b"short", &keys).unwrap_err();
        assert!(err.message.contains("does not match"), "{err}");
        // Right size and hash, but not a zstd frame: damaged, never parsed.
        let junk = vec![7u8; 64];
        let e2 = Entry {
            size: 64,
            sha256: wol_core::package::sha256_hex(&junk),
            ..e.clone()
        };
        assert!(check_download(&e2, &junk, &keys).is_err());
        assert!(unpack(&vec![0; WOLPLUG_MAX_FILE as usize + 1]).is_err());
    }

    /// `.wolplug` bytes (zstd of the ustar) for a theme plugin `id`, signed by `seed`.
    fn wolplug(id: &str, seed: Option<&[u8; 32]>) -> Vec<u8> {
        wolplug_of(id, seed, &[Kind::Theme])
    }

    /// The same for a plugin of `kinds` (a Dock plugin asks to run, a Toggle one for toggles).
    fn wolplug_of(id: &str, seed: Option<&[u8; 32]>, kinds: &[Kind]) -> Vec<u8> {
        use wol_core::package::{manifest, sign, ustar};
        let files: Vec<(String, Vec<u8>)> = wol_core::plugins::scaffold(id, "Demo", kinds)
            .unwrap()
            .into_iter()
            .map(|(r, t)| (format!("{id}/{r}"), t.into_bytes()))
            .collect();
        let mut all: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let sha = manifest(&all);
        let sig = seed.map(|s| sign(s, sha.as_bytes()));
        all.push(("SHA256", sha.as_bytes()));
        if let Some(s) = &sig {
            all.push(("SHA256.sig", s.as_slice()));
        }
        let tar = ustar(&all);
        ruzstd::encoding::compress_to_vec(&tar[..], ruzstd::encoding::CompressionLevel::Fastest)
    }

    fn entry_for(id: &str, file: &[u8]) -> Entry {
        Entry {
            version: "0.1".into(),
            // The theme plugin the tests build asks for nothing.
            permissions: vec![],
            size: file.len() as u64,
            sha256: wol_core::package::sha256_hex(file),
            ..serde_json::from_value(entry(id)).unwrap()
        }
    }

    #[test]
    fn a_signed_download_that_matches_its_entry_is_ready_to_install() {
        let keys = [wol_core::package::public_of(&SEED)];
        let f = wolplug("alice.demo", Some(&SEED));
        let v = check_download(&entry_for("alice.demo", &f), &f, &keys).unwrap();
        assert_eq!(v.manifest.id, "alice.demo");
        // Unsigned, or listed under another id or version: refused.
        let u = wolplug("alice.demo", None);
        let err = check_download(&entry_for("alice.demo", &u), &u, &keys).unwrap_err();
        assert!(err.message.contains("not signed"), "{err}");
        let err = check_download(&entry_for("alice.other", &f), &f, &keys).unwrap_err();
        assert!(err.message.contains("not the plugin"), "{err}");
        let e = Entry {
            version: "9.9".into(),
            ..entry_for("alice.demo", &f)
        };
        assert!(check_download(&e, &f, &keys).is_err());
    }

    /// ST6-5: the list says what a person agrees to; Turn On grants the package manifest's own
    /// permissions. A listing that says less (or more, or other) than the signed package is refused.
    #[test]
    fn a_download_must_ask_for_what_the_list_says() {
        let keys = [wol_core::package::public_of(&SEED)];
        let f = wolplug_of("alice.demo", Some(&SEED), &[Kind::Dock, Kind::Toggle]);
        let with = |perms: &[&str]| Entry {
            permissions: perms.iter().map(|p| p.to_string()).collect(),
            ..entry_for("alice.demo", &f)
        };
        // Same set, any order, repeats allowed.
        assert!(check_download(&with(&["toggles", "run"]), &f, &keys).is_ok());
        assert!(check_download(&with(&["run", "toggles", "run"]), &f, &keys).is_ok());
        // Understated, overstated, other, or none at all: untrusted.
        for bad in [
            &["run"][..],
            &[],
            &["run", "toggles", "http"],
            &["run", "http"],
        ] {
            let err = check_download(&with(bad), &f, &keys).unwrap_err();
            assert!(err.message.contains("other permissions"), "{bad:?}: {err}");
            assert_eq!(err.status, crate::ffi::Status::Untrusted, "{bad:?}");
        }
        // The plugin that asks for nothing, listed as asking for something.
        let t = wolplug("alice.theme", Some(&SEED));
        let listed = Entry {
            permissions: vec!["run".into()],
            ..entry_for("alice.theme", &t)
        };
        assert!(check_download(&listed, &t, &keys).is_err());
    }

    #[test]
    fn refresh_keeps_only_a_verified_list_and_downloads_are_checked() {
        use crate::testutil::{Served, serve, temp};
        let keys = [wol_core::package::public_of(&SEED)];
        let text = index(vec![entry("alice.weather")]);
        let sig = wol_core::package::sign(&SEED, &text).to_vec();
        let f = wolplug("alice.demo", Some(&SEED));
        let s = serve(vec![
            ("/index.json", Served::bytes(text.clone())),
            ("/index.json.sig", Served::bytes(sig)),
            ("/bad/index.json", Served::bytes(text.clone())),
            ("/bad/index.json.sig", Served::bytes(vec![0; 64])),
            ("/alice.demo-0.1.wolplug", Served::bytes(f.clone())),
        ]);
        let net = crate::fetch::Net::new(true);
        let dir = temp("plugin-index");
        assert!(cached(&dir, &keys).is_err(), "nothing cached yet");
        let i = refresh(&net, &s.url("/index.json"), &dir, &keys).unwrap();
        assert_eq!(i.plugins.len(), 1);
        assert_eq!(cached(&dir, &keys).unwrap(), i);
        // A list that does not verify is refused and the cache stays as it was.
        assert!(refresh(&net, &s.url("/bad/index.json"), &dir, &keys).is_err());
        assert_eq!(cached(&dir, &keys).unwrap(), i);
        // C6-2: an older (lower serial) or expired list, validly signed, is refused and the cache
        // stays as it was.
        let newer = serde_json::to_vec(&serde_json::json!({
            "format": 1, "serial": 5, "plugins": [entry("alice.weather")]
        }))
        .unwrap();
        let older = serde_json::to_vec(&serde_json::json!({
            "format": 1, "serial": 4, "plugins": []
        }))
        .unwrap();
        let expired = serde_json::to_vec(&serde_json::json!({
            "format": 1, "serial": 9, "expires": 1, "plugins": []
        }))
        .unwrap();
        let signed = |t: &[u8]| Served::bytes(wol_core::package::sign(&SEED, t).to_vec());
        let s2 = serve(vec![
            ("/n/index.json", Served::bytes(newer.clone())),
            ("/n/index.json.sig", signed(&newer)),
            ("/o/index.json", Served::bytes(older.clone())),
            ("/o/index.json.sig", signed(&older)),
            ("/e/index.json", Served::bytes(expired.clone())),
            ("/e/index.json.sig", signed(&expired)),
        ]);
        let n = refresh(&net, &s2.url("/n/index.json"), &dir, &keys).unwrap();
        assert_eq!(n.plugins.len(), 1);
        let serial = |d: &std::path::Path| cached_full(d, &keys).unwrap().1.serial;
        assert_eq!(serial(&dir), 5);
        assert!(refresh(&net, &s2.url("/o/index.json"), &dir, &keys).is_err());
        assert!(refresh(&net, &s2.url("/e/index.json"), &dir, &keys).is_err());
        assert_eq!(serial(&dir), 5);
        // The same serial again (a re-download) is fine.
        assert!(refresh(&net, &s2.url("/n/index.json"), &dir, &keys).is_ok());
        // A changed cache is refused on read.
        std::fs::write(dir.join("index.json"), b"{\"format\":1,\"plugins\":[]}").unwrap();
        assert!(cached(&dir, &keys).is_err());
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 2, "no temp files left: {left:?}");
        // The download is read up to its listed size and checked.
        let e = Entry {
            url: s.url("/alice.demo-0.1.wolplug"),
            ..entry_for("alice.demo", &f)
        };
        assert_eq!(download(&net, &e, &keys).unwrap().manifest.id, "alice.demo");
        let short = Entry {
            size: e.size - 1,
            ..e.clone()
        };
        assert!(download(&net, &short, &keys).is_err());
        // Plain HTTP is refused outside loopback tests.
        assert!(download(&crate::fetch::Net::new(false), &e, &keys).is_err());
    }

    #[test]
    fn a_stale_list_is_due_at_half_its_lifetime_or_after_a_day() {
        assert!(!is_stale(1000, 0, 1000 + MAX_AGE - 1));
        assert!(is_stale(1000, 0, 1000 + MAX_AGE));
        assert!(!is_stale(1000, 1200, 1099));
        assert!(is_stale(1000, 1200, 1100));
        assert!(is_stale(1000, 1000 + 10 * MAX_AGE, 1000 + MAX_AGE));
        // A clock set back never makes a list stale on its own; expiry still applies.
        assert!(!is_stale(1000, 1200, 10));
    }

    /// C10-3: an expired cached list is refused, and `load(false)` downloads a missing, stale or
    /// expired cache again; a still-valid cache survives a failed download.
    #[test]
    fn an_expired_cache_is_refused_and_a_stale_one_refreshed() {
        use crate::testutil::{Served, serve, temp};
        let keys = [wol_core::package::public_of(&SEED)];
        let now = crate::release::unix_now();
        let list = |serial: u64, expires: u64, ids: &[&str]| {
            serde_json::to_vec(&serde_json::json!({
                "format": 1, "serial": serial, "expires": expires,
                "plugins": ids.iter().map(|i| entry(i)).collect::<Vec<_>>()
            }))
            .unwrap()
        };
        let put = |dir: &std::path::Path, text: &[u8], age: u64| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(SIG_FILE), wol_core::package::sign(&SEED, text)).unwrap();
            std::fs::write(dir.join(INDEX_FILE), text).unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
            std::fs::File::options()
                .write(true)
                .open(dir.join(INDEX_FILE))
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        let next = list(6, now + 7 * 24 * 3600, &["alice.weather", "bob.clock"]);
        let s = serve(vec![
            ("/index.json", Served::bytes(next.clone())),
            (
                "/index.json.sig",
                Served::bytes(wol_core::package::sign(&SEED, &next).to_vec()),
            ),
        ]);
        let net = crate::fetch::Net::new(true);
        let url = s.url("/index.json");
        let gone = s.url("/missing/index.json");
        // Expired: `cached` refuses it, and load(false) downloads the list again.
        let dir = temp("plugin-expired");
        put(&dir, &list(5, now - 1, &["alice.weather"]), 0);
        let err = cached(&dir, &keys).unwrap_err();
        assert!(err.message.contains("expired"), "{err}");
        assert!(
            load(&net, &gone, &dir, &keys, false).is_err(),
            "no stale fallback"
        );
        assert_eq!(
            load(&net, &url, &dir, &keys, false).unwrap().plugins.len(),
            2
        );
        assert_eq!(cached(&dir, &keys).unwrap().plugins.len(), 2);
        // Fresh and valid: used as is, no download needed.
        let dir = temp("plugin-fresh");
        put(&dir, &list(5, now + 1000, &["alice.weather"]), 0);
        assert_eq!(
            load(&net, &gone, &dir, &keys, false).unwrap().plugins.len(),
            1
        );
        // Past half its lifetime: downloaded again; when that fails the valid cache is kept.
        let dir = temp("plugin-stale");
        put(&dir, &list(5, now + 1000, &["alice.weather"]), 1500);
        assert_eq!(
            load(&net, &gone, &dir, &keys, false).unwrap().plugins.len(),
            1
        );
        assert_eq!(
            load(&net, &url, &dir, &keys, false).unwrap().plugins.len(),
            2
        );
        // `fresh` always downloads, and falls back to a valid cache only.
        let dir = temp("plugin-load-fresh");
        put(&dir, &list(5, now + 1000, &["alice.weather"]), 0);
        assert_eq!(
            load(&net, &gone, &dir, &keys, true).unwrap().plugins.len(),
            1
        );
        assert_eq!(
            load(&net, &url, &dir, &keys, true).unwrap().plugins.len(),
            2
        );
        let dir = temp("plugin-load-expired");
        put(&dir, &list(5, now - 1, &["alice.weather"]), 0);
        assert!(load(&net, &gone, &dir, &keys, true).is_err());
        // An expired cache still sets the floor: an older signed list is refused.
        let dir = temp("plugin-floor");
        put(&dir, &list(9, now - 1, &[]), 0);
        assert!(refresh(&net, &url, &dir, &keys).is_err());
    }

    #[test]
    fn a_decompression_bomb_is_cut_short() {
        let zeros = vec![0u8; WOLPLUG_MAX_TAR as usize + 4096];
        let bomb = ruzstd::encoding::compress_to_vec(
            &zeros[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        assert!((bomb.len() as u64) < WOLPLUG_MAX_FILE);
        let err = unpack(&bomb).unwrap_err();
        assert!(err.message.contains("too large"), "{err}");
    }
}

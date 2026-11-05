// SPDX-License-Identifier: BSD-2-Clause
//! One resolution: address -> verified bytes, or what stopped it. The root zone is held for a few
//! minutes and never replaced by an older one, nor by another zone of the same version (rollback
//! protection, signatures.md "Replay protection"); the newest version seen is kept on disk so a
//! restart cannot go back either, and no version below [`verify::MIN_ROOT_VERSION`] is taken.
//! Manifests are held by address (they cannot change under it), within a byte budget; blocks are
//! fetched each time and hash-checked each time.
use crate::source::{self, Source};
use crate::uri::FedUrl;
use crate::verify::{self, Failure, Layer, Zone, text};
use crate::{page, time};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a verified root zone is used before it is asked for again.
pub const ZONE_FRESH: Duration = Duration::from_secs(300);
/// Manifests held (by address), and the bytes they may take together.
const MANIFESTS: usize = 64;
pub const MANIFEST_BUDGET: usize = 8 << 20;

/// The network's TLDs (the signed root zone of 2026-11-04, none of them an IANA TLD on that
/// date): the only hosts an `http(s)://` URL is taken from the web for. A TLD must also be in the
/// held zone's table when one is held; a TLD the network adds later is reachable as `fed://` but
/// is not taken from the web until it is listed here (ADR 0052, the mapping's trust).
pub const KNOWN_TLDS: &[&str] = &[
    "amarelo", "azul", "blau", "branco", "busca", "cara", "comu", "fed", "foto", "mosca", "oi",
    "pages", "pagina", "pic", "preto", "records", "rosa", "sound", "tipos", "types", "verde",
    "vid", "weblog",
];

/// How one load ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Verified bytes.
    Content {
        mime: String,
        bytes: Vec<u8>,
        /// The block's address (a strong ETag).
        address: String,
    },
    /// A check failed: nothing was served.
    Refused(Failure),
    /// The zone has no such TLD or name, or the manifest no such file.
    NotFound(String),
    /// The TLD or name exists but does not resolve (status or expiry).
    Unavailable(String),
    /// What this build does not resolve yet (delegated registries, non-manifest targets).
    Unsupported(String),
    /// The network could not be reached and nothing verified is held.
    Network(String),
    /// Not a Federate address.
    BadAddress(String),
}

/// What a load is for: a document (page or frame) may be larger than anything it loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Load {
    Document,
    Subresource,
}

impl Load {
    pub fn max_block(self) -> u64 {
        match self {
            Load::Document => source::MAX_BLOCK,
            Load::Subresource => source::MAX_SUBRESOURCE,
        }
    }
}

struct Held {
    zone: Arc<Zone>,
    at: Instant,
}

pub struct Resolver {
    source: Box<dyn Source>,
    root_key: String,
    fresh: Duration,
    zone: Mutex<Option<Held>>,
    manifests: Mutex<Manifests>,
    /// The newest zone verified: version and digest (also on disk at `state`).
    newest: Mutex<(u64, Option<String>)>,
    state: Option<PathBuf>,
}

#[derive(Default)]
struct Manifests {
    by_address: HashMap<String, (Arc<Value>, usize)>,
    bytes: usize,
}

/// `$XDG_STATE_HOME/wolos/federate/root-version` (`$HOME/.local/state/...` without it), with the
/// environment read through `env` (the host's `main` passes it).
pub fn default_state(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    let base = match set("XDG_STATE_HOME") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(set("HOME")?).join(".local/state"),
    };
    Some(base.join("wolos/federate/root-version"))
}

/// `<version> <digest>` of the sealed state file ([`crate::state`]); a version below the minimum,
/// or a file that does not verify, counts as none.
fn read_state(path: &std::path::Path) -> Option<(u64, Option<String>)> {
    let t = crate::state::load(path)?;
    let mut it = t.split_whitespace();
    let v: u64 = it.next()?.parse().ok()?;
    let d = it
        .next()
        .filter(|d| verify::is_address(d))
        .map(str::to_owned);
    (v >= verify::MIN_ROOT_VERSION).then_some((v, d))
}

/// The process's resolver: the pinned root key and the network's HTTPS door, with no state file
/// (a host that has one installs its own resolver; `wol-webhost` does).
pub fn shared() -> &'static Resolver {
    static R: OnceLock<Resolver> = OnceLock::new();
    R.get_or_init(|| {
        Resolver::new(
            Box::new(source::Https::new(crate::BOOTSTRAP)),
            crate::ROOT_KEY,
        )
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The MIME type of a file name (the protocol serves by the manifest's file name).
pub fn mime_of(file: &str) -> &'static str {
    let ext = file
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "webmanifest" => "application/json",
        "md" | "markdown" => "text/markdown; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

impl Resolver {
    pub fn new(source: Box<dyn Source>, root_key: &str) -> Resolver {
        Resolver {
            source,
            root_key: root_key.to_owned(),
            fresh: ZONE_FRESH,
            zone: Mutex::new(None),
            manifests: Mutex::new(Manifests::default()),
            newest: Mutex::new((verify::MIN_ROOT_VERSION, None)),
            state: None,
        }
    }

    /// Keeps the newest zone version seen in `path` (read now, written when it advances). A
    /// lower or unreadable value there is ignored.
    pub fn with_state(mut self, path: Option<PathBuf>) -> Resolver {
        if let Some(seen) = path.as_deref().and_then(read_state) {
            *lock(&self.newest) = seen;
        }
        self.state = path;
        self
    }

    /// The newest zone version verified (or the minimum).
    pub fn newest_version(&self) -> u64 {
        lock(&self.newest).0
    }

    /// The bytes the held manifests take.
    pub fn held_manifest_bytes(&self) -> usize {
        lock(&self.manifests).bytes
    }

    /// Whether `zone` may be used: newer than every zone verified so far, or that same zone.
    /// Advances the newest seen (and its file) when it is newer.
    fn admit(&self, zone: &Zone) -> Result<(), Failure> {
        let mut newest = lock(&self.newest);
        let (v, d) = newest.clone();
        let same = d.as_deref() == Some(zone.digest.as_str());
        if zone.version < v || (zone.version == v && d.is_some() && !same) {
            return Err(Failure {
                layer: Layer::Root,
                subject: "the root zone".into(),
                reason: format!(
                    "version {} is not newer than the newest zone already verified ({v}): a replay or a fork",
                    zone.version
                ),
            });
        }
        if !same {
            *newest = (zone.version, Some(zone.digest.clone()));
            if let Some(p) = &self.state {
                // Best effort: a state that cannot be written only loses the memory across a restart.
                let record = format!("{} {}", zone.version, zone.digest);
                let _ = crate::state::store(p, &record, &crate::state::system_fill);
            }
        }
        Ok(())
    }

    /// How long a verified zone is used before the source is asked again ([`ZONE_FRESH`]).
    pub fn with_fresh(mut self, fresh: Duration) -> Resolver {
        self.fresh = fresh;
        self
    }

    /// The verified root zone: the one held while fresh, else the network's (refused if it does
    /// not verify, or is older than the one held), else the held one when the network is down.
    pub fn zone(&self) -> Result<Arc<Zone>, Outcome> {
        let held = lock(&self.zone).as_ref().map(|h| (h.zone.clone(), h.at));
        if let Some((z, at)) = &held
            && at.elapsed() < self.fresh
        {
            return Ok(z.clone());
        }
        let fetched = match self.source.get("/v1/root", source::MAX_ROOT) {
            Ok(bytes) => bytes,
            Err(e) => {
                return held.map(|(z, _)| z).ok_or(Outcome::Network(e));
            }
        };
        let zone = match Zone::verify(&fetched, &self.root_key) {
            Ok(z) => Arc::new(z),
            // A distributor sent a zone that does not verify: never used. The held one, which
            // did, stays in use while there is one.
            Err(f) => return held.map(|(z, _)| z).ok_or(Outcome::Refused(f)),
        };
        // Older than, or another zone of the same version as, one already verified: refused. The
        // held one goes on serving (its freshness restarts, so the source is not asked every load).
        let keep = match self.admit(&zone) {
            Ok(()) => zone,
            Err(f) => match held {
                Some((z, _)) => z,
                None => return Err(Outcome::Refused(f)),
            },
        };
        *lock(&self.zone) = Some(Held {
            zone: keep.clone(),
            at: Instant::now(),
        });
        Ok(keep)
    }

    /// Whether `tld` is a Federate TLD for the web's purposes: on the built-in [`KNOWN_TLDS`] list,
    /// whatever the held zone says. A name under it is only ever `fed://` on WolOS, never http(s)
    /// (ADR 0052, exclusive). The list is deliberately not widened by the zone: whoever holds the
    /// root key cannot take a TLD of the ordinary web by adding it; a TLD the zone adds is reachable
    /// as `fed://` at once and under http(s) spelling after a WolOS update lists it.
    pub fn knows_tld(&self, tld: &str) -> bool {
        KNOWN_TLDS.contains(&tld)
    }

    /// Whether `host` ends in a Federate TLD, however many labels it has, read in the one spelling
    /// [`crate::uri::web_host`] gives (a trailing dot and the case do not make it another name).
    pub fn is_fed_host(&self, host: &str) -> bool {
        crate::uri::web_host(host)
            .rsplit_once('.')
            .is_some_and(|(_, tld)| self.knows_tld(tld))
    }

    /// See [`crate::map_http`]: the `fed://` spelling of every `http(s)` URL whose host ends in a
    /// Federate TLD. A host that is not `label.tld` (`www.home.fed`, a bad label) is mapped as it
    /// stands, so the `fed:` handler answers with a page that says why; it is never left to the
    /// web. `None` only for a URL that is not http(s) or whose host is not under a Federate TLD.
    pub fn map_http(&self, url: &str) -> Option<String> {
        let host = FedUrl::http_host(url)?;
        if !self.is_fed_host(&host) {
            return None;
        }
        FedUrl::from_http(url)
            .map(|u| u.to_url())
            .or_else(|| FedUrl::raw_fed(url))
    }

    fn manifest(&self, address: &str, fqdn: &str, owner: &str) -> Result<Arc<Value>, Outcome> {
        // Held only after it verified for this address; the name and owner are checked again.
        if let Some((m, _)) = lock(&self.manifests).by_address.get(address).cloned()
            && m.as_object()
                .is_some_and(|o| text(o, "domain") == fqdn && text(o, "owner_public_key") == owner)
        {
            return Ok(m);
        }
        let bytes = self
            .source
            .get(&format!("/v1/manifest/{address}"), source::MAX_MANIFEST)
            .map_err(Outcome::Network)?;
        let m = Arc::new(verify::manifest(&bytes, address, fqdn, owner).map_err(Outcome::Refused)?);
        let mut held = lock(&self.manifests);
        let n = bytes.len();
        if held.by_address.len() >= MANIFESTS || held.bytes + n > MANIFEST_BUDGET {
            *held = Manifests::default();
        }
        if n <= MANIFEST_BUDGET
            && held
                .by_address
                .insert(address.to_owned(), (m.clone(), n))
                .is_none()
        {
            held.bytes += n;
        }
        Ok(m)
    }

    /// Resolves one address through the whole chain, as a page (the larger block cap).
    pub fn resolve(&self, u: &FedUrl) -> Outcome {
        self.resolve_at(u, time::now())
    }

    pub fn resolve_at(&self, u: &FedUrl, now: i64) -> Outcome {
        self.resolve_load(u, now, Load::Document)
    }

    /// Resolves one address for a load of `kind` at `now` (seconds since 1970).
    pub fn resolve_load(&self, u: &FedUrl, now: i64, kind: Load) -> Outcome {
        match self.chain(u, now, kind) {
            Ok(o) | Err(o) => o,
        }
    }

    fn chain(&self, u: &FedUrl, now: i64, kind: Load) -> Result<Outcome, Outcome> {
        let fqdn = u.domain();
        let zone = self.zone()?;
        // Never judge a record earlier than the zone was made: a clock set back cannot bring an
        // expired TLD or name back.
        let now = zone.generated_at.map_or(now, |g| now.max(g));
        let Some(tld) = zone.tld(&u.tld) else {
            return Err(Outcome::NotFound(format!(
                "The Federate root zone has no TLD .{}.",
                u.tld
            )));
        };
        let status = text(tld, "status");
        if !matches!(status, "official" | "delegated") {
            return Err(Outcome::Unavailable(format!(
                "The TLD .{} is {status}.",
                u.tld
            )));
        }
        if time::expired(tld.get("expires_at").and_then(Value::as_str), now) {
            return Err(Outcome::Unavailable(format!(
                "The TLD .{} has expired.",
                u.tld
            )));
        }
        let registry = text(tld, "registry_type");
        if registry != "root_managed" {
            return Err(Outcome::Unsupported(format!(
                "Names under .{} live in a delegated registry ({registry}), which WolOS does not read yet.",
                u.tld
            )));
        }
        let Some(record) = zone.domain(&fqdn) else {
            return Err(Outcome::NotFound(format!(
                "{fqdn} is not registered on the Federate Network."
            )));
        };
        verify::domain(record, &fqdn, &u.tld, text(tld, "operator_public_key"))
            .map_err(Outcome::Refused)?;
        let rec = record
            .as_object()
            .ok_or_else(|| Outcome::NotFound(fqdn.clone()))?;
        let status = text(rec, "status");
        if status != "active" {
            return Err(Outcome::Unavailable(format!("{fqdn} is {status}.")));
        }
        if time::expired(rec.get("expires_at").and_then(Value::as_str), now) {
            return Err(Outcome::Unavailable(format!("{fqdn} has expired.")));
        }
        let target = text(rec, "target_type");
        if target != "manifest" {
            return Err(Outcome::Unsupported(format!(
                "{fqdn} points at a {target}, which WolOS does not open yet."
            )));
        }
        let address = text(rec, "manifest_hash");
        if !verify::is_address(address) {
            return Err(Outcome::Refused(Failure {
                layer: Layer::Domain,
                subject: fqdn,
                reason: "the manifest address is not a BLAKE3 address".into(),
            }));
        }
        let manifest = self.manifest(address, &fqdn, text(rec, "owner_public_key"))?;
        let Some((file, block)) = verify::file_for(&manifest, &u.path) else {
            return Err(Outcome::NotFound(format!(
                "{fqdn} has no page at {}.",
                u.path
            )));
        };
        if !verify::is_address(&block) {
            return Err(Outcome::Refused(Failure {
                layer: Layer::Manifest,
                subject: fqdn,
                reason: format!("the address of {file} is not a BLAKE3 address"),
            }));
        }
        let bytes = self
            .source
            .get(&format!("/v1/block/{block}"), kind.max_block())
            .map_err(Outcome::Network)?;
        verify::check_address(Layer::Content, &bytes, &block).map_err(|f| {
            Outcome::Refused(Failure {
                subject: format!("{fqdn}/{file}"),
                ..f
            })
        })?;
        Ok(Outcome::Content {
            mime: mime_of(&file).to_owned(),
            bytes,
            address: block,
        })
    }

    /// The reply for one URL (`fed://`, or a mapped `http(s)://`) loaded as a page.
    pub fn serve(&self, url: &str) -> page::Reply {
        self.serve_load(url, Load::Document)
    }

    /// The reply for one load of `kind`: verified content, or a page that says what happened.
    pub fn serve_load(&self, url: &str, kind: Load) -> page::Reply {
        // An http(s) URL of a Federate TLD host is read as the address it spells; an http(s) URL of
        // any other host is not an address at all.
        let parsed = FedUrl::parse(url).or_else(|e| match self.map_http(url) {
            Some(fed) => FedUrl::parse(&fed),
            None => Err(e),
        });
        match parsed {
            Ok(u) => page::reply(&u.to_url(), self.resolve_load(&u, time::now(), kind)),
            Err(e) => page::reply(url, Outcome::BadAddress(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    /// QA-A10-CS-2: a host spelled with its root label (`home.fed.`) or in another case is the same
    /// name to every reader, so the fed-only rule cannot be stepped around by spelling.
    #[test]
    fn a_fed_host_is_one_whatever_its_trailing_dot_or_case() {
        let r = super::shared();
        for fed in [
            "home.fed",
            "home.fed.",
            "HOME.Fed",
            "www.home.fed.",
            "home.fed..",
        ] {
            assert!(r.is_fed_host(fed), "{fed}");
        }
        for web in ["example.com", "example.com.", "fed", "fed.example.com", ""] {
            assert!(!r.is_fed_host(web), "{web}");
        }
    }
}

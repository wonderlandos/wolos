// SPDX-License-Identifier: BSD-2-Clause
//! The whole chain against the network's own data (captured 2026-11-04 from
//! https://federate.network/v1/root, /v1/manifest/41220e29..., /v1/block/ddfe588a...), and
//! against a small network signed here, for the layers the live data cannot break one at a time.
use ed25519_dalek::{Signer, SigningKey};
use federate::resolve::{KNOWN_TLDS, Outcome};
use federate::{FedUrl, Layer, Resolver, Source, canon, verify};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

const ROOT: &[u8] = include_bytes!("vectors/root-zone.json");
const MANIFEST: &[u8] = include_bytes!("vectors/manifest-home.fed.json");
const INDEX: &[u8] = include_bytes!("vectors/block-home.fed-index.html");
const HOME_MANIFEST: &str = "41220e29510791e1fbbc13dbf2ba610dc9598f28a0f102f488f20610780c4c6f";
const HOME_INDEX: &str = "ddfe588ac4fe64292a561ae36aee15d32250b1bbc2b62fd8f84e09b96e254066";
const NOW: i64 = 1_793_000_000; // 2026-10-27
const V: u64 = verify::MIN_ROOT_VERSION;

/// A distributor that answers from a table (and can be changed between loads).
#[derive(Default)]
struct Table(Mutex<HashMap<String, Vec<u8>>>);

impl Table {
    fn put(&self, path: &str, bytes: &[u8]) {
        self.0
            .lock()
            .unwrap()
            .insert(path.to_owned(), bytes.to_vec());
    }
}

impl Source for &'static Table {
    fn get(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self
            .0
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or(format!("no {path}"))?;
        if b.len() as u64 > max {
            return Err("too large".into());
        }
        Ok(b)
    }
}

fn table() -> &'static Table {
    Box::leak(Box::default())
}

fn live() -> &'static Table {
    let t = table();
    t.put("/v1/root", ROOT);
    t.put(&format!("/v1/manifest/{HOME_MANIFEST}"), MANIFEST);
    t.put(&format!("/v1/block/{HOME_INDEX}"), INDEX);
    t
}

fn resolve(r: &Resolver, url: &str) -> Outcome {
    r.resolve_at(&FedUrl::parse(url).unwrap(), NOW)
}

fn refused_at(o: &Outcome) -> Option<Layer> {
    match o {
        Outcome::Refused(f) => Some(f.layer),
        _ => None,
    }
}

#[test]
fn the_networks_home_page_verifies_end_to_end() {
    let r = Resolver::new(Box::new(live()), federate::ROOT_KEY);
    match resolve(&r, "fed://home.fed") {
        Outcome::Content {
            mime,
            bytes,
            address,
        } => {
            assert_eq!(mime, "text/html; charset=utf-8");
            assert_eq!(bytes, INDEX);
            assert_eq!(address, HOME_INDEX);
        }
        other => panic!("{other:?}"),
    }
    // The zone's TLD table is the network's list.
    let mut names = r.zone().unwrap().tld_names();
    names.sort();
    assert_eq!(names, KNOWN_TLDS);
    // The reply the engine gets, for both spellings.
    for url in [
        "fed://home.fed/",
        "http://home.fed/",
        "https://home.fed/index.html",
    ] {
        let reply = r.serve(url);
        assert_eq!((reply.status, reply.body.as_slice()), (200, INDEX), "{url}");
    }
}

#[test]
fn a_changed_byte_anywhere_in_the_live_chain_is_refused() {
    // Content: one byte of the block.
    let t = live();
    let mut bad = INDEX.to_vec();
    bad[100] ^= 1;
    t.put(&format!("/v1/block/{HOME_INDEX}"), &bad);
    let r = Resolver::new(Box::new(t), federate::ROOT_KEY);
    let o = resolve(&r, "fed://home.fed");
    assert_eq!(refused_at(&o), Some(Layer::Content), "{o:?}");
    let page = r.serve("fed://home.fed");
    assert_eq!(page.status, 502);
    assert!(
        !page.body.windows(bad.len()).any(|w| w == bad.as_slice()),
        "nothing of it shown"
    );

    // Manifest: a file address swapped (the bytes no longer hash to the domain record's address).
    let t = live();
    let m = String::from_utf8(MANIFEST.to_vec())
        .unwrap()
        .replace(HOME_INDEX, &"0".repeat(64));
    t.put(&format!("/v1/manifest/{HOME_MANIFEST}"), m.as_bytes());
    let r = Resolver::new(Box::new(t), federate::ROOT_KEY);
    assert_eq!(
        refused_at(&resolve(&r, "fed://home.fed")),
        Some(Layer::Manifest)
    );

    // Root: the zone's signature, or the zone's data under it.
    let t = live();
    let z = String::from_utf8(ROOT.to_vec())
        .unwrap()
        .replace(HOME_MANIFEST, HOME_INDEX);
    t.put("/v1/root", z.as_bytes());
    let r = Resolver::new(Box::new(t), federate::ROOT_KEY);
    assert_eq!(
        refused_at(&resolve(&r, "fed://home.fed")),
        Some(Layer::Root)
    );

    // A root key other than the pinned one.
    let other = hex(&SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes());
    let r = Resolver::new(Box::new(live()), &other);
    assert_eq!(
        refused_at(&resolve(&r, "fed://home.fed")),
        Some(Layer::Root)
    );
}

#[test]
fn names_and_paths_the_zone_does_not_have_are_not_found() {
    let r = Resolver::new(Box::new(live()), federate::ROOT_KEY);
    assert!(matches!(
        resolve(&r, "fed://nobody.fed"),
        Outcome::NotFound(_)
    ));
    assert!(matches!(
        resolve(&r, "fed://home.nope"),
        Outcome::NotFound(_)
    ));
    assert!(matches!(
        resolve(&r, "fed://home.fed/missing"),
        Outcome::NotFound(_)
    ));
    assert_eq!(r.serve("fed://nobody.fed").status, 404);
    assert_eq!(r.serve("fed://home.fed:80/").status, 400);
}

#[test]
fn no_network_and_nothing_held_says_so() {
    let r = Resolver::new(Box::new(table()), federate::ROOT_KEY);
    assert!(matches!(resolve(&r, "fed://home.fed"), Outcome::Network(_)));
    assert_eq!(r.serve("fed://home.fed").status, 504);
}

#[test]
fn only_the_networks_tlds_are_taken_from_the_web() {
    let r = Resolver::new(Box::new(table()), federate::ROOT_KEY);
    assert_eq!(
        r.map_http("http://home.fed/a?b=1").as_deref(),
        Some("fed://home.fed/a?b=1")
    );
    assert_eq!(
        r.map_http("https://lobo.pagina").as_deref(),
        Some("fed://lobo.pagina")
    );
    assert_eq!(r.map_http("https://example.com/"), None);
    // Every host under a Federate TLD is the network's, valid name or not: never the web.
    assert_eq!(
        r.map_http("https://www.home.fed/").as_deref(),
        Some("fed://www.home.fed/")
    );
    assert_eq!(r.serve("https://www.home.fed/").status, 400);
    assert!(r.is_fed_host("a.b.pagina"));
    assert!(!r.is_fed_host("example.com"));
    assert_eq!(
        r.serve("https://example.com/").status,
        400,
        "never fetched as Federate"
    );
}

// --- A network signed here --------------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public(k: &SigningKey) -> String {
    hex(&k.verifying_key().to_bytes())
}

fn sign(mut v: Value, k: &SigningKey) -> Value {
    let sig = k.sign(&canon::signable(&v).unwrap());
    v["signature"] = Value::String(hex(&sig.to_bytes()));
    v
}

struct Net {
    root: SigningKey,
    op: SigningKey,
    owner: SigningKey,
}

impl Net {
    fn new() -> Net {
        Net {
            root: key(1),
            op: key(2),
            owner: key(3),
        }
    }

    fn tld(&self, extra: Value) -> Value {
        let mut t = json!({"tld": "fed", "status": "official", "mode": "official",
            "owner_public_key": public(&self.root), "operator_public_key": public(&self.op),
            "operator_name": "test", "registry_type": "root_managed",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "signature_algorithm": "ed25519", "signature": null});
        merge(&mut t, extra);
        sign(t, &self.root)
    }

    fn record(&self, manifest: &str, extra: Value, by: &SigningKey) -> Value {
        let mut d = json!({"domain": "site.fed", "tld": "fed", "label": "site",
            "owner_public_key": public(&self.owner), "target_type": "manifest",
            "manifest_hash": manifest, "status": "active",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "signature_algorithm": "ed25519", "signature": null});
        merge(&mut d, extra);
        sign(d, by)
    }

    fn zone(&self, version: u64, tld: Value, record: Value) -> Vec<u8> {
        self.zone_with(
            version,
            "2026-01-01T00:00:00Z",
            json!({"fed": tld}),
            json!({"site.fed": record}),
        )
    }

    fn zone_with(&self, version: u64, generated: &str, tlds: Value, domains: Value) -> Vec<u8> {
        let z = json!({"network": "federate", "root_version": version,
            "generated_at": generated, "root_public_key": public(&self.root),
            "tlds": tlds, "domains": domains, "audit": [],
            "signature_algorithm": "ed25519", "signature": null});
        serde_json::to_vec(&sign(z, &self.root)).unwrap()
    }

    fn manifest(&self, block: &str, by: &SigningKey) -> Vec<u8> {
        let m = json!({"domain": "site.fed", "version": 1, "entry": "index.html",
            "files": {"index.html": block}, "owner_public_key": public(&self.owner),
            "created_at": "2026-01-01T00:00:00Z", "signature_algorithm": "ed25519", "signature": null});
        serde_json::to_vec_pretty(&sign(m, by)).unwrap()
    }

    /// A table serving the whole network, with the knobs the tests turn.
    fn serve(
        &self,
        tld: Value,
        rec_extra: Value,
        rec_by: &SigningKey,
        man_by: &SigningKey,
    ) -> &'static Table {
        let page = b"<h1>site</h1>";
        let block = verify::address(page);
        let m = self.manifest(&block, man_by);
        let ma = verify::address(&m);
        let t = table();
        t.put(
            "/v1/root",
            &self.zone(V + 5, self.tld(tld), self.record(&ma, rec_extra, rec_by)),
        );
        t.put(&format!("/v1/manifest/{ma}"), &m);
        t.put(&format!("/v1/block/{block}"), page);
        t
    }
}

fn merge(into: &mut Value, extra: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), extra) {
        a.extend(b);
    }
}

fn resolver(n: &Net, t: &'static Table) -> Resolver {
    Resolver::new(Box::new(t), &public(&n.root))
}

#[test]
fn a_network_signed_here_resolves() {
    let n = Net::new();
    let t = n.serve(json!({}), json!({}), &n.op, &n.owner);
    let o = resolve(&resolver(&n, t), "fed://site.fed/");
    assert!(
        matches!(&o, Outcome::Content { bytes, .. } if bytes == b"<h1>site</h1>"),
        "{o:?}"
    );
}

#[test]
fn each_layer_refuses_a_record_signed_by_the_wrong_key() {
    let n = Net::new();
    let stranger = key(7);
    let t = n.serve(json!({}), json!({}), &stranger, &n.owner);
    assert_eq!(
        refused_at(&resolve(&resolver(&n, t), "fed://site.fed")),
        Some(Layer::Domain)
    );
    let t = n.serve(json!({}), json!({}), &n.op, &stranger);
    assert_eq!(
        refused_at(&resolve(&resolver(&n, t), "fed://site.fed")),
        Some(Layer::Manifest)
    );
    // A TLD record not signed by the root key, inside a zone that is.
    let mut tld = n.tld(json!({}));
    tld = sign(tld, &stranger);
    let page = b"x";
    let t = table();
    t.put(
        "/v1/root",
        &n.zone(
            V + 5,
            tld,
            n.record(&verify::address(page), json!({}), &n.op),
        ),
    );
    assert_eq!(
        refused_at(&resolve(&resolver(&n, t), "fed://site.fed")),
        Some(Layer::Tld)
    );
}

#[test]
fn status_expiry_and_delegation_stop_the_load_honestly() {
    let n = Net::new();
    let cases = [
        (json!({}), json!({"status": "suspended"}), "unavailable"),
        (
            json!({}),
            json!({"expires_at": "2026-01-02T00:00:00Z"}),
            "unavailable",
        ),
        (json!({}), json!({"expires_at": "whenever"}), "unavailable"),
        (json!({"status": "disabled"}), json!({}), "unavailable"),
        (
            json!({"expires_at": "2026-01-02T00:00:00Z"}),
            json!({}),
            "unavailable",
        ),
        (
            json!({"registry_type": "delegated_native", "registry_providers": ["x:4077"]}),
            json!({}),
            "unsupported",
        ),
        (json!({}), json!({"target_type": "service"}), "unsupported"),
    ];
    for (tld, rec, want) in cases {
        let t = n.serve(tld.clone(), rec.clone(), &n.op, &n.owner);
        let o = resolve(&resolver(&n, t), "fed://site.fed");
        let got = match o {
            Outcome::Unavailable(_) => "unavailable",
            Outcome::Unsupported(_) => "unsupported",
            _ => "other",
        };
        assert_eq!(got, want, "{tld} {rec}: {o:?}");
    }
}

#[test]
fn an_older_zone_never_replaces_a_newer_one() {
    let n = Net::new();
    let t = n.serve(json!({}), json!({}), &n.op, &n.owner);
    let r = resolver(&n, t).with_fresh(Duration::ZERO);
    assert_eq!(r.zone().unwrap().version, V + 5);
    // A correctly signed zone of version 4 that drops the name: refused as a rollback.
    let gone = json!({"network": "federate", "root_version": V + 4, "generated_at": "x",
        "root_public_key": public(&n.root), "tlds": {"fed": n.tld(json!({}))}, "domains": {},
        "audit": [], "signature_algorithm": "ed25519", "signature": null});
    t.put(
        "/v1/root",
        &serde_json::to_vec(&sign(gone, &n.root)).unwrap(),
    );
    assert_eq!(r.zone().unwrap().version, V + 5);
    assert!(matches!(
        resolve(&r, "fed://site.fed"),
        Outcome::Content { .. }
    ));
    // A zone that does not verify is not used; the held one goes on serving.
    t.put("/v1/root", b"{\"network\":\"federate\"}");
    assert_eq!(r.zone().unwrap().version, V + 5);
}

#[test]
fn a_zone_below_the_built_in_version_is_refused() {
    let n = Net::new();
    let t = table();
    t.put(
        "/v1/root",
        &n.zone(
            V - 1,
            n.tld(json!({})),
            n.record(&"0".repeat(64), json!({}), &n.op),
        ),
    );
    assert_eq!(
        refused_at(&resolve(&resolver(&n, t), "fed://site.fed")),
        Some(Layer::Root)
    );
    // Refused by the zone check itself, not only by the resolver's newest-seen memory.
    let old = n.zone(
        V - 1,
        n.tld(json!({})),
        n.record(&"0".repeat(64), json!({}), &n.op),
    );
    let e = verify::Zone::verify(&old, &public(&n.root)).unwrap_err();
    assert!(
        e.reason
            .contains("older than the zone this WolOS was built with"),
        "{e}"
    );
}

#[test]
fn another_zone_of_the_same_version_is_refused() {
    let n = Net::new();
    let t = n.serve(json!({}), json!({}), &n.op, &n.owner);
    let r = resolver(&n, t).with_fresh(Duration::ZERO);
    let first = r.zone().unwrap().digest.clone();
    // Same version, correctly signed, the name dropped: a fork, not an update.
    t.put(
        "/v1/root",
        &n.zone_with(
            V + 5,
            "2026-01-01T00:00:00Z",
            json!({"fed": n.tld(json!({}))}),
            json!({}),
        ),
    );
    assert_eq!(r.zone().unwrap().digest, first);
    assert!(matches!(
        resolve(&r, "fed://site.fed"),
        Outcome::Content { .. }
    ));
}

fn state_file(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("fed-state-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.join("wolos/federate/root-version")
}

#[test]
fn a_restart_cannot_go_back_to_an_older_zone() {
    let n = Net::new();
    let path = state_file("restart");
    let t = n.serve(json!({}), json!({}), &n.op, &n.owner);
    let r = resolver(&n, t).with_state(Some(path.clone()));
    assert!(matches!(
        resolve(&r, "fed://site.fed"),
        Outcome::Content { .. }
    ));
    // The file is sealed: the record is not in clear, and opens only through the state module.
    let raw = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains(&(V + 5).to_string()));
    assert!(
        federate::state::load(&path)
            .unwrap()
            .starts_with(&format!("{} ", V + 5))
    );
    // A new process, and a distributor that serves the older (correctly signed) zone.
    let old = table();
    old.put(
        "/v1/root",
        &n.zone_with(
            V + 4,
            "2026-01-01T00:00:00Z",
            json!({"fed": n.tld(json!({}))}),
            json!({}),
        ),
    );
    let again = resolver(&n, old).with_state(Some(path.clone()));
    assert_eq!(again.newest_version(), V + 5);
    assert_eq!(
        refused_at(&resolve(&again, "fed://site.fed")),
        Some(Layer::Root)
    );
    // A plain (unsealed) record, even a higher one, is ignored: the floor stays the built-in one.
    std::fs::write(&path, format!("{} x\n", V + 99)).unwrap();
    assert_eq!(
        resolver(&n, table())
            .with_state(Some(path.clone()))
            .newest_version(),
        V
    );
    // So is a sealed record with a byte changed, and a deleted one falls back to the built-in floor.
    let mut sealed = raw.clone();
    sealed[raw.len() / 2] ^= 1;
    std::fs::write(&path, &sealed).unwrap();
    assert_eq!(
        resolver(&n, table())
            .with_state(Some(path.clone()))
            .newest_version(),
        V
    );
    std::fs::write(&path, &raw).unwrap();
    assert_eq!(
        resolver(&n, table())
            .with_state(Some(path.clone()))
            .newest_version(),
        V + 5
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        resolver(&n, table())
            .with_state(Some(path.clone()))
            .newest_version(),
        V
    );
    // A state file below the built-in version is ignored.
    std::fs::write(&path, "5 x\n").unwrap();
    assert_eq!(
        resolver(&n, table())
            .with_state(Some(path))
            .newest_version(),
        V
    );
}

#[test]
fn a_clock_set_back_does_not_bring_an_expired_name_back() {
    let n = Net::new();
    let page = b"<h1>site</h1>";
    let m = n.manifest(&verify::address(page), &n.owner);
    let ma = verify::address(&m);
    let rec = n.record(&ma, json!({"expires_at": "2026-03-01T00:00:00Z"}), &n.op);
    let t = table();
    t.put(
        "/v1/root",
        &n.zone_with(
            V + 5,
            "2026-06-01T00:00:00Z",
            json!({"fed": n.tld(json!({}))}),
            json!({"site.fed": rec}),
        ),
    );
    t.put(&format!("/v1/manifest/{ma}"), &m);
    t.put(&format!("/v1/block/{}", verify::address(page)), page);
    let r = resolver(&n, t);
    let january = 1_768_435_200; // 2026-01-15, before the expiry, after which the zone was made
    let o = r.resolve_at(&FedUrl::parse("fed://site.fed").unwrap(), january);
    assert!(matches!(o, Outcome::Unavailable(_)), "{o:?}");
}

#[test]
fn every_built_in_tld_is_taken_from_the_web_whatever_the_zone_holds() {
    let n = Net::new();
    let mut zzz = n.tld(json!({}));
    zzz["tld"] = json!("zzz");
    let zzz = sign(zzz, &n.root);
    let t = table();
    t.put(
        "/v1/root",
        &n.zone_with(
            V + 5,
            "2026-01-01T00:00:00Z",
            json!({"fed": n.tld(json!({})), "zzz": zzz}),
            json!({}),
        ),
    );
    let r = resolver(&n, t);
    r.zone().unwrap();
    assert_eq!(
        r.map_http("http://site.fed/").as_deref(),
        Some("fed://site.fed")
    );
    assert_eq!(
        r.map_http("http://x.zzz/"),
        None,
        "a zone TLD not on the built-in list is not taken from the web (the root key cannot capture one)"
    );
    assert_eq!(
        r.map_http("http://lobo.pagina/").as_deref(),
        Some("fed://lobo.pagina"),
        "a built-in TLD the zone lacks stays fed:// (the chain says the zone lacks it), never the web"
    );
    assert_eq!(r.map_http("http://example.com/"), None);
}

#[test]
fn a_subresource_block_has_a_smaller_cap_than_a_document() {
    let n = Net::new();
    let big = vec![b'a'; (16 << 20) + 1];
    let block = verify::address(&big);
    let m = n.manifest(&block, &n.owner);
    let ma = verify::address(&m);
    let t = table();
    t.put(
        "/v1/root",
        &n.zone(V + 5, n.tld(json!({})), n.record(&ma, json!({}), &n.op)),
    );
    t.put(&format!("/v1/manifest/{ma}"), &m);
    t.put(&format!("/v1/block/{block}"), &big);
    let r = resolver(&n, t);
    let u = FedUrl::parse("fed://site.fed").unwrap();
    assert!(matches!(
        r.resolve_load(&u, NOW, federate::Load::Document),
        Outcome::Content { .. }
    ));
    assert!(matches!(
        r.resolve_load(&u, NOW, federate::Load::Subresource),
        Outcome::Network(_)
    ));
    assert_eq!(
        r.serve_load("fed://site.fed", federate::Load::Subresource)
            .status,
        504
    );
}

#[test]
fn held_manifests_stay_within_their_byte_budget() {
    let n = Net::new();
    let t = table();
    let mut domains = serde_json::Map::new();
    let padding = "p".repeat(3 << 20);
    for i in 0..5 {
        let name = format!("s{i}");
        let m = json!({"domain": format!("{name}.fed"), "version": 1, "entry": "index.html",
            "files": {"index.html": verify::address(b"x")}, "owner_public_key": public(&n.owner),
            "created_at": "2026-01-01T00:00:00Z", "pad": padding,
            "signature_algorithm": "ed25519", "signature": null});
        let m = serde_json::to_vec(&sign(m, &n.owner)).unwrap();
        let ma = verify::address(&m);
        let mut rec = n.record(
            &ma,
            json!({"domain": format!("{name}.fed"), "label": name}),
            &n.op,
        );
        rec = sign(rec, &n.op);
        domains.insert(format!("{name}.fed"), rec);
        t.put(&format!("/v1/manifest/{ma}"), &m);
    }
    t.put(&format!("/v1/block/{}", verify::address(b"x")), b"x");
    t.put(
        "/v1/root",
        &n.zone_with(
            V + 5,
            "2026-01-01T00:00:00Z",
            json!({"fed": n.tld(json!({}))}),
            Value::Object(domains),
        ),
    );
    let r = resolver(&n, t);
    for i in 0..5 {
        let o = resolve(&r, &format!("fed://s{i}.fed"));
        assert!(matches!(o, Outcome::Content { .. }), "{o:?}");
        assert!(r.held_manifest_bytes() <= federate::resolve::MANIFEST_BUDGET);
    }
    assert!(r.held_manifest_bytes() > 0);
}

#[test]
fn the_state_file_lives_under_the_state_home() {
    use federate::resolve::default_state;
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    };
    assert_eq!(
        default_state(&env(&[("XDG_STATE_HOME", "/s"), ("HOME", "/h")])),
        Some("/s/wolos/federate/root-version".into())
    );
    assert_eq!(
        default_state(&env(&[("XDG_STATE_HOME", ""), ("HOME", "/h")])),
        Some("/h/.local/state/wolos/federate/root-version".into())
    );
    assert_eq!(default_state(&env(&[])), None);
}

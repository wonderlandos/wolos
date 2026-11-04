// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol install|remove|list|search`, `wol update apps` (ADR 0011, docs/modules/packages.md).
//! `.wol` files and index apps go to wol-admind `wol-install` after a local check; App Store
//! catalogue ids and plain pkg names go to `pkg-install`. The daemon verifies again on its own copy.
use crate::cli::Invocation;
use crate::run::{Env, admin_send, emit_json, fail};
use serde_json::{Value, json};
use shared::admin::{AdminAction, AdminRequest};
use shared::bundle::EXTENSION;
use shared::errors::ErrorCode;
use shared::package::{self as contract, Entry, INDEX_SIG_SUFFIX, INDEX_URL, Index};
use std::path::{Path, PathBuf};
use wol_core::package::{self, Trust, Verified};
use wol_core::store;

/// Overrides for tests and mirrors: index URL, trusted keys dir, install records dir.
pub const INDEX_VAR: &str = "WOL_INDEX_URL";
pub const KEYS_VAR: &str = "WOL_PACKAGE_KEYS";
pub const RECORDS_VAR: &str = "WOL_PACKAGE_RECORDS";

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    let app = inv.get("app").unwrap_or_default().to_string();
    match inv.action {
        "package.install" => install(env, &app),
        "package.remove" => remove(env, &app),
        "package.list" => list(env, json),
        "package.search" => search(env, &inv.all("term").join(" "), json),
        "package.updates" => updates(env, inv.flag("yes"), json),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

fn keys(env: &dyn Env) -> Vec<[u8; 32]> {
    let dir = env
        .var(KEYS_VAR)
        .unwrap_or_else(|| contract::KEYS_DIR.into());
    package::read_keys(Path::new(&dir))
}

fn records(env: &dyn Env) -> Vec<shared::bundle::Info> {
    let dir = env
        .var(RECORDS_VAR)
        .unwrap_or_else(|| contract::RECORDS_DIR.into());
    package::installed(Path::new(&dir))
}

/// Where the administrator's downloads and index live when `XDG_CACHE_HOME` is not set: root's
/// home is in the sealed, read-only slot (QA-STORE-62: curl 23 on every index fetch).
pub const ROOT_CACHE: &str = "/var/cache/wolos";

/// `$XDG_CACHE_HOME/wolos/packages` (downloads and the index); for root without one,
/// [`ROOT_CACHE`]`/packages`.
fn cache_dir(env: &dyn Env) -> PathBuf {
    match env.var("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v).join("wolos/packages"),
        None if env.is_root() => Path::new(ROOT_CACHE).join("packages"),
        None => env.home().join(".cache/wolos/packages"),
    }
}

/// How a download ended. curl runs silent (`-fsL`: its own text never reaches the person,
/// QA-STORE-61); its exit status picks the sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fetched {
    Ok,
    /// The server answered with an HTTP error (curl 22): the file is not published there.
    NotPublished,
    /// Anything else: no network, no name resolution, a cut connection.
    Unreachable,
}

fn fetch(env: &mut dyn Env, url: &str, dest: &Path) -> Fetched {
    let argv = package::fetch_argv(url, &dest.to_string_lossy());
    match env.exec_quiet(&argv) {
        Ok(0) => Fetched::Ok,
        Ok(22) => Fetched::NotPublished,
        _ => Fetched::Unreachable,
    }
}

/// The sentence for an app list that could not be fetched.
fn fetch_sentence(f: Fetched) -> &'static str {
    match f {
        Fetched::NotPublished => "WolOS has not published the app list there yet",
        _ => "the app list could not be downloaded (are you online?)",
    }
}

/// A file of at most [`package::INDEX_MAX_BYTES`] (audit 8 #5): a larger one is an error, never
/// read into memory whole.
fn read_capped(p: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut v = Vec::new();
    std::fs::File::open(p)?
        .take(package::INDEX_MAX_BYTES + 1)
        .read_to_end(&mut v)?;
    if v.len() as u64 > package::INDEX_MAX_BYTES {
        return Err(std::io::Error::other("larger than an index may be"));
    }
    Ok(v)
}

/// A unique name next to `dest` for one download (R2C-23): two runs never write the same file,
/// and the final name only ever holds a complete, verified copy.
fn part_of(dest: &Path) -> PathBuf {
    let nonce = shared::jobs::new_nonce()
        .unwrap_or_else(|_| format!("{}-{}", std::process::id(), shared::sys::monotonic_ns()));
    let mut n = dest.file_name().unwrap_or_default().to_os_string();
    n.push(format!(".{nonce}.part"));
    dest.with_file_name(n)
}

/// Take a freshly downloaded index (and its signature, when it came) into the cache: only when
/// it verifies and parses, then both renamed over the cached pair (R2C-10). Err: the fresh copy
/// was not good; the cached pair is untouched. The downloads are removed either way.
fn adopt(
    dir: &Path,
    fresh: &Path,
    fresh_sig: Option<&Path>,
    keys: &[[u8; 32]],
    required: bool,
) -> Result<(), String> {
    let (file, sig) = (dir.join("index.json"), dir.join("index.json.sig"));
    let done = (|| {
        let text = read_capped(fresh).map_err(|e| format!("the download is unusable ({e})"))?;
        let s = fresh_sig.and_then(|p| read_capped(p).ok());
        package::verify_fetched_index(&text, s.as_deref(), keys, required)?;
        Index::parse(&String::from_utf8_lossy(&text))?;
        // The cached signature belongs to the cached index: replace it, or drop it with it.
        match fresh_sig.filter(|_| s.is_some()) {
            Some(p) => std::fs::rename(p, &sig).map_err(|e| e.to_string())?,
            None => {
                let _ = std::fs::remove_file(&sig);
            }
        }
        std::fs::rename(fresh, &file).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(fresh);
    if let Some(p) = fresh_sig {
        let _ = std::fs::remove_file(p);
    }
    done
}

/// Fresh index, else the cached copy; None when there is neither. A bad signature or a malformed
/// index is an error, never "no index". A fresh copy replaces the cached pair only once it
/// verified (R2C-10): a failed or bad download leaves the last good pair in place.
fn index(env: &mut dyn Env) -> Result<Option<Index>, String> {
    let dir = cache_dir(env);
    let _ = std::fs::create_dir_all(&dir);
    let url = env.var(INDEX_VAR).unwrap_or_else(|| INDEX_URL.into());
    let (file, sig) = (dir.join("index.json"), dir.join("index.json.sig"));
    let keys = keys(env);
    let mark = dir.join(package::INDEX_SIGNED_MARK);
    let required = package::index_sig_required(
        !keys.is_empty(),
        env.var(INDEX_VAR).is_some(),
        mark.exists(),
    );
    let (fresh, fresh_sig) = (part_of(&file), part_of(&sig));
    let cached = |f: &Path| {
        if f.exists() {
            "; using the copy from before"
        } else {
            ""
        }
    };
    let got = fetch(env, &url, &fresh);
    if got != Fetched::Ok {
        let _ = std::fs::remove_file(&fresh);
        env.err(&format!("wol: {}{}\n", fetch_sentence(got), cached(&file)));
    } else {
        let got_sig = fetch(env, &format!("{url}{INDEX_SIG_SUFFIX}"), &fresh_sig);
        let s = (got_sig == Fetched::Ok).then_some(fresh_sig.as_path());
        if fresh.exists()
            && let Err(e) = adopt(&dir, &fresh, s, &keys, required)
        {
            // A signature the server does not have yet is not a damaged list.
            let why = if s.is_none() && got_sig == Fetched::NotPublished {
                "WolOS has not published its signature yet".to_owned()
            } else {
                e
            };
            env.err(&format!(
                "wol: the downloaded app list was not used ({why}){}\n",
                cached(&file)
            ));
        }
        let _ = std::fs::remove_file(&fresh);
        let _ = std::fs::remove_file(&fresh_sig);
    }
    let Ok(text) = read_capped(&file) else {
        return Ok(None);
    };
    let sig = read_capped(&sig).ok();
    if let Trust::Signed(_) = package::verify_fetched_index(&text, sig.as_deref(), &keys, required)?
    {
        let _ = std::fs::write(&mark, ""); // only tightens the next check
    }
    Index::parse(&String::from_utf8_lossy(&text)).map(Some)
}

fn looks_like_file(app: &str) -> bool {
    app.ends_with(&format!(".{EXTENSION}")) || app.contains('/')
}

fn install(env: &mut dyn Env, app: &str) -> i32 {
    if looks_like_file(app) {
        let path = std::path::absolute(app).unwrap_or_else(|_| PathBuf::from(app));
        return install_file(env, &path);
    }
    if store::get(app).is_some() {
        return pkg_install(env, app);
    }
    let found = match index(env) {
        Ok(i) => i.and_then(|i| package::find(&i, app).cloned()),
        // R2C-10: a plain package name still installs from the signed repos (wol-admind asks
        // for the password); only an index app needs the index.
        Err(e) if contract::is_pkg_name(app) && !contract::is_app_id(app) => {
            env.err(&format!(
                "wol: package index: {e}; installing '{app}' from the system repositories\n"
            ));
            None
        }
        Err(e) => return fail(env, ErrorCode::Invalid, &format!("package index: {e}")),
    };
    match found {
        Some(e) => install_entry(env, &e),
        None if contract::is_pkg_name(app) => pkg_install(env, app),
        None => fail(env, ErrorCode::NotFound, &format!("no app named '{app}'")),
    }
}

fn pkg_install(env: &mut dyn Env, name: &str) -> i32 {
    let req = AdminRequest {
        action: AdminAction::PkgInstall,
        args: vec![name.into()],
    };
    admin_send(env, &req)
}

fn describe(v: &Verified) -> String {
    let i = &v.info;
    let mut s = format!(
        "{} {} ({})\n",
        if i.name.is_empty() { &i.id } else { &i.name },
        i.version,
        i.id
    );
    let uses: Vec<&str> = i
        .capabilities
        .iter()
        .filter_map(|c| {
            contract::CAPABILITIES
                .iter()
                .find(|k| k.0 == c)
                .map(|k| k.1)
        })
        .collect();
    if !uses.is_empty() {
        s.push_str(&format!("  uses: {}\n", uses.join(", ")));
    }
    s
}

fn install_file(env: &mut dyn Env, path: &Path) -> i32 {
    let v = match package::verify(path, &keys(env)) {
        Ok(v) => v,
        Err(e) => {
            return fail(
                env,
                ErrorCode::Invalid,
                &format!("{}: {e}; not installed", path.display()),
            );
        }
    };
    env.err(&describe(&v));
    let trust = match v.trust {
        Trust::Signed(_) => "signed",
        Trust::Unsigned => {
            env.err(
                "wol: this package is not signed by WolOS; installing it needs your password\n",
            );
            "unsigned"
        }
    };
    let req = AdminRequest {
        action: AdminAction::WolInstall,
        args: vec![path.to_string_lossy().into_owned(), trust.into()],
    };
    let code = admin_send(env, &req);
    if code == 0 {
        env.out(&format!("installed {} {}\n", v.info.id, v.info.version));
    }
    code
}

/// Download (cached by file name), check size and hash against the index, then install.
fn install_entry(env: &mut dyn Env, e: &Entry) -> i32 {
    let dest = cache_dir(env).join(e.file_name());
    let fresh = package::sha256_file(&dest).is_ok_and(|h| h == e.sha256);
    if !fresh {
        let why = match fetch(env, &e.url, &dest) {
            Fetched::Ok => None,
            Fetched::NotPublished => Some("the server does not have this app's file"),
            Fetched::Unreachable => Some("the download did not finish (are you online?)"),
        };
        if let Some(why) = why {
            return fail(
                env,
                ErrorCode::Unavailable,
                &format!("{} could not be downloaded: {why}", e.name),
            );
        }
    }
    let size = std::fs::metadata(&dest).map_or(0, |m| m.len());
    if size != e.size || package::sha256_file(&dest).ok().as_deref() != Some(&e.sha256) {
        let _ = std::fs::remove_file(&dest);
        return fail(
            env,
            ErrorCode::Invalid,
            &format!("{}: download does not match the index", e.file_name()),
        );
    }
    install_file(env, &dest)
}

fn remove(env: &mut dyn Env, app: &str) -> i32 {
    let Some(i) = records(env)
        .into_iter()
        .find(|i| i.package == app || i.id.eq_ignore_ascii_case(app))
    else {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("'{app}' is not an app installed from a .wol"),
        );
    };
    let req = AdminRequest {
        action: AdminAction::WolRemove,
        args: vec![i.id.clone()],
    };
    let code = admin_send(env, &req);
    if code == 0 {
        env.out(&format!("removed {}\n", i.id));
    }
    code
}

fn list(env: &mut dyn Env, json: bool) -> i32 {
    let rows: Vec<Value> = records(env)
        .iter()
        .map(|i| json!({"id": i.id, "name": i.name, "version": i.version, "package": i.package}))
        .collect();
    print_rows(env, rows, json, "no apps installed from .wol packages")
}

fn print_rows(env: &mut dyn Env, rows: Vec<Value>, json: bool, empty: &str) -> i32 {
    if json {
        emit_json(env, &Value::from(rows));
    } else if rows.is_empty() {
        env.out(&format!("{empty}\n"));
    } else {
        for r in rows {
            let s = |k: &str| r[k].as_str().unwrap_or_default().to_string();
            let tail = [s("version"), s("summary")]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join("  ");
            env.out(&format!("{:<28} {:<36} {tail}\n", s("name"), s("id")));
        }
    }
    0
}

fn search(env: &mut dyn Env, term: &str, json: bool) -> i32 {
    if term.trim().is_empty() {
        return fail(env, ErrorCode::Usage, "missing <term>");
    }
    let mut rows: Vec<Value> = store::search(term)
        .iter()
        .map(|a| json!({"id": a.id, "name": a.name, "summary": a.summary, "source": "pkg"}))
        .collect();
    match index(env) {
        Ok(Some(i)) => rows.extend(package::search(&i, term).iter().map(|e| {
            json!({"id": e.id, "name": e.name, "summary": e.summary, "version": e.version,
                   "source": "wol", "signed": e.signed})
        })),
        Ok(None) => env.err("wol: no app list to search yet\n"),
        Err(e) => env.err(&format!("wol: package index: {e}\n")),
    }
    print_rows(env, rows, json, "no matches")
}

fn updates(env: &mut dyn Env, yes: bool, json: bool) -> i32 {
    let index = match index(env) {
        Ok(Some(i)) => i,
        Ok(None) => {
            return fail(
                env,
                ErrorCode::Unavailable,
                "no app list yet, so app updates could not be checked",
            );
        }
        Err(e) => return fail(env, ErrorCode::Invalid, &format!("package index: {e}")),
    };
    let installed = records(env);
    let up = package::updates(&installed, &index);
    if !yes {
        let rows = up
            .iter()
            .map(|(i, e)| json!({"id": i.id, "name": e.name, "version": e.version, "installed": i.version}))
            .collect();
        return print_rows(env, rows, json, "apps are up to date");
    }
    let entries: Vec<Entry> = up.iter().map(|(_, e)| (*e).clone()).collect();
    entries
        .iter()
        .map(|e| install_entry(env, e))
        .find(|c| *c != 0)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::FakeEnv;
    use wol_core::package::{manifest, public_of, public_pem, sha256_hex, sign, ustar};

    const SEED: [u8; 32] = [7; 32];

    /// QA-STORE-62: the folder root's index lives in is made by the image at boot (tmpfiles.d),
    /// so the first `sudo wol pkg` finds it.
    #[test]
    fn root_cache_is_made_by_tmpfiles() {
        let conf = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../files/usr/lib/tmpfiles.d/wolos.conf"
        ))
        .unwrap();
        let env = FakeEnv {
            root: true,
            ..Default::default()
        };
        let dir = cache_dir(&env);
        for d in [Path::new(ROOT_CACHE), dir.as_path()] {
            assert!(
                conf.lines().any(|l| {
                    let w: Vec<_> = l.split_whitespace().collect();
                    w.len() >= 5 && w[0] == "d" && Path::new(w[1]) == d && w[3] == "root"
                }),
                "tmpfiles makes {}",
                d.display()
            );
        }
    }

    fn info(version: &str) -> String {
        format!(
            r#"{{"id":"org.wolos.Fixture","version":"{version}","name":"Fixture","capabilities":["network"],"package":"wol-fixture","targets":[{{"abi":"freebsd-native","arch":"amd64","entry":"bin/fixture"}}]}}"#
        )
    }

    fn wol(version: &str, seed: Option<&[u8; 32]>) -> Vec<u8> {
        let i = info(version);
        let man = manifest(&[("Info.json", i.as_bytes()), ("payload.pkg", b"pkg")]);
        let sig = seed.map(|s| sign(s, man.as_bytes()));
        let mut m: Vec<(&str, &[u8])> = vec![
            ("Info.json", i.as_bytes()),
            ("payload.pkg", b"pkg"),
            ("SHA256", man.as_bytes()),
        ];
        if let Some(s) = &sig {
            m.push(("SHA256.sig", s));
        }
        ustar(&m)
    }

    /// FakeEnv with a temp home: trusted key, empty records, cache under home.
    fn env(tag: &str) -> (FakeEnv, PathBuf) {
        let home = std::env::temp_dir().join(format!("wol-packages-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["keys", "records", ".cache/wolos/packages"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join("keys/wolos.pub"), public_pem(&public_of(&SEED))).unwrap();
        let mut e = FakeEnv {
            home: home.clone(),
            replies: vec!["ok\n".into()],
            ..FakeEnv::default()
        };
        e.vars = vec![
            (KEYS_VAR.into(), home.join("keys").to_string_lossy().into()),
            (
                RECORDS_VAR.into(),
                home.join("records").to_string_lossy().into(),
            ),
            (INDEX_VAR.into(), "https://example.org/index.json".into()),
            (
                "XDG_CACHE_HOME".into(),
                home.join(".cache").to_string_lossy().into(),
            ),
        ];
        (e, home)
    }

    fn sent(e: &FakeEnv) -> Vec<String> {
        e.requests
            .iter()
            .map(|(_, b)| {
                let (r, _) = shared::admin::decode(b).unwrap();
                format!("{} {}", r.action.name(), r.args.join(" "))
            })
            .collect()
    }

    fn run(e: &mut FakeEnv, line: &str) -> i32 {
        let argv: Vec<&str> = line.split_whitespace().collect();
        crate::run::execute(&crate::cli::parse(&argv).unwrap(), e)
    }

    /// Index with one entry for `bytes`, placed in the cache as if just fetched.
    fn index_with(home: &Path, version: &str, bytes: &[u8]) {
        let i = json!({"format": 1, "packages": [{
            "id": "org.wolos.Fixture", "name": "Fixture", "summary": "Test package",
            "version": version, "package": "wol-fixture", "abi": "FreeBSD:15:amd64",
            "url": format!("https://example.org/wol-fixture-{version}.wol"),
            "sha256": sha256_hex(bytes), "size": bytes.len(), "signed": true}]});
        let text = i.to_string();
        let cache = home.join(".cache/wolos/packages");
        std::fs::write(cache.join("index.json"), &text).unwrap();
        std::fs::write(cache.join("index.json.sig"), sign(&SEED, text.as_bytes())).unwrap();
    }

    #[test]
    fn install_a_local_file() {
        let (mut e, home) = env("file");
        let f = home.join("Fixture.wol");
        std::fs::write(&f, wol("1.0", Some(&SEED))).unwrap();
        assert_eq!(
            run(&mut e, &format!("install {}", f.display())),
            0,
            "{}",
            e.err
        );
        assert_eq!(sent(&e), [format!("wol-install {} signed", f.display())]);
        assert!(
            e.err.contains("Fixture 1.0") && e.err.contains("Internet"),
            "{}",
            e.err
        );
        // Unsigned: said plainly, sent as unsigned (wol-admind asks for the password).
        let (mut e, home) = env("unsigned");
        let f = home.join("Fixture.wol");
        std::fs::write(&f, wol("1.0", None)).unwrap();
        assert_eq!(run(&mut e, &format!("install {}", f.display())), 0);
        assert_eq!(sent(&e), [format!("wol-install {} unsigned", f.display())]);
        assert!(e.err.contains("not signed"), "{}", e.err);
        // Signed by a key nobody trusts, or damaged: refused before the daemon is asked.
        for bytes in [wol("1.0", Some(&[9; 32])), b"junk".to_vec()] {
            let (mut e, home) = env("bad");
            let f = home.join("Bad.wol");
            std::fs::write(&f, bytes).unwrap();
            assert_ne!(run(&mut e, &format!("install {}", f.display())), 0);
            assert!(e.requests.is_empty());
        }
    }

    #[test]
    fn install_by_name_picks_the_source() {
        // App Store catalogue id and unknown pkg names: pkg-install, no index needed.
        let (mut e, _) = env("gimp");
        assert_eq!(run(&mut e, "install gimp"), 0);
        assert_eq!(sent(&e), ["pkg-install gimp"]);
        // Index app: downloaded into the cache, checked against the index, then installed.
        let (mut e, home) = env("index");
        let bytes = wol("2.0", Some(&SEED));
        index_with(&home, "2.0", &bytes);
        let dl = home.join(".cache/wolos/packages/wol-fixture-2.0.wol");
        std::fs::write(&dl, &bytes).unwrap();
        assert_eq!(run(&mut e, "install org.wolos.Fixture"), 0, "{}", e.err);
        assert_eq!(sent(&e), [format!("wol-install {} signed", dl.display())]);
        // Not in the index: plain pkg.
        let (mut e, home) = env("htop");
        index_with(&home, "2.0", &bytes);
        assert_eq!(run(&mut e, "install htop"), 0);
        assert_eq!(sent(&e), ["pkg-install htop"]);
        // Download that does not match the index: refused, file dropped.
        let (mut e, home) = env("mismatch");
        index_with(&home, "2.0", &bytes);
        let dl = home.join(".cache/wolos/packages/wol-fixture-2.0.wol");
        std::fs::write(&dl, wol("2.0", None)).unwrap();
        assert_ne!(run(&mut e, "install wol-fixture"), 0);
        assert!(e.requests.is_empty() && !dl.exists());
        // An index signed by someone else is not used.
        let (mut e, home) = env("badindex");
        index_with(&home, "2.0", &bytes);
        std::fs::write(home.join(".cache/wolos/packages/index.json.sig"), [0u8; 64]).unwrap();
        assert_ne!(run(&mut e, "install org.wolos.Fixture"), 0);
        assert!(e.requests.is_empty());
    }

    #[test]
    fn list_remove_search_and_updates() {
        let (mut e, home) = env("list");
        let rec = home.join("records/org.wolos.Fixture");
        std::fs::create_dir_all(&rec).unwrap();
        std::fs::write(rec.join("Info.json"), info("1.0")).unwrap();
        assert_eq!(run(&mut e, "list --json"), 0);
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(v[0]["id"], "org.wolos.Fixture");
        assert_eq!(v[0]["version"], "1.0");
        // search: catalogue and index together.
        let bytes = wol("2.0", Some(&SEED));
        index_with(&home, "2.0", &bytes);
        e.out.clear();
        assert_eq!(run(&mut e, "search test --json"), 0);
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert!(v.as_array().unwrap().iter().any(|r| r["source"] == "wol"));
        e.out.clear();
        assert_eq!(run(&mut e, "search photo --json"), 0);
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(v[0]["id"], "gimp");
        // update apps: 1.0 installed, index has 2.0.
        e.out.clear();
        assert_eq!(run(&mut e, "update apps --json"), 0);
        let v: Value = serde_json::from_str(&e.out).unwrap();
        assert_eq!(
            (v[0]["installed"].as_str(), v[0]["version"].as_str()),
            (Some("1.0"), Some("2.0"))
        );
        assert!(e.requests.is_empty());
        std::fs::write(
            home.join(".cache/wolos/packages/wol-fixture-2.0.wol"),
            &bytes,
        )
        .unwrap();
        assert_eq!(run(&mut e, "update apps --yes"), 0, "{}", e.err);
        assert_eq!(sent(&e).len(), 1);
        assert!(sent(&e)[0].starts_with("wol-install "));
        // remove by package name or id.
        let (mut e2, _) = env("rm");
        e2.vars = e.vars.clone();
        assert_eq!(run(&mut e2, "remove wol-fixture"), 0);
        assert_eq!(sent(&e2), ["wol-remove org.wolos.Fixture"]);
        let (mut e3, _) = env("rm-missing");
        assert_ne!(run(&mut e3, "remove nothing"), 0);
        assert!(e3.requests.is_empty());
    }

    /// R2C-10: a signature download that fails keeps the last good pair, and a plain package
    /// name still installs when the index cannot be used.
    #[test]
    fn a_failed_signature_download_keeps_the_cached_pair() {
        let bytes = wol("2.0", Some(&SEED));
        let (mut e, home) = env("sigfail");
        index_with(&home, "2.0", &bytes);
        let cache = home.join(".cache/wolos/packages");
        std::fs::write(cache.join("wol-fixture-2.0.wol"), &bytes).unwrap();
        assert_eq!(run(&mut e, "install org.wolos.Fixture"), 0, "{}", e.err);
        assert!(cache.join("index.signed").exists());
        let mut again = FakeEnv {
            home: home.clone(),
            vars: e.vars.clone(),
            replies: vec!["ok\n".into()],
            exec_codes: vec![0, 22], // the index downloads, its signature does not
            ..FakeEnv::default()
        };
        assert_eq!(
            run(&mut again, "install org.wolos.Fixture"),
            0,
            "{}",
            again.err
        );
        assert!(
            cache.join("index.json.sig").exists(),
            "the good signature stays"
        );
        assert_eq!(sent(&again).len(), 1);
        // No usable index at all (signature gone after a signed one): pacman names still work,
        // index app ids do not (they need the index).
        std::fs::remove_file(cache.join("index.json.sig")).unwrap();
        let mut plain = FakeEnv {
            home: home.clone(),
            vars: e.vars.clone(),
            replies: vec!["ok\n".into()],
            ..FakeEnv::default()
        };
        assert_eq!(run(&mut plain, "install htop"), 0, "{}", plain.err);
        assert_eq!(sent(&plain), ["pkg-install htop"]);
        assert!(plain.err.contains("system repositories"), "{}", plain.err);
        // Nothing is left behind under a download name.
        let parts = std::fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .filter(|f| f.file_name().to_string_lossy().ends_with(".part"))
            .count();
        assert_eq!(parts, 0);
    }

    /// QA-STORE-61: an app list or signature the server does not have (HTTP 404, curl 22) is
    /// "not published", not "offline"; no network is "are you online?"; curl's own text is never
    /// shown (curl runs silent).
    #[test]
    fn index_404_says_not_published() {
        let (mut e, home) = env("404");
        let cache = home.join(".cache/wolos/packages");
        let _ = std::fs::remove_dir_all(&cache);
        e.exec_codes = vec![22];
        assert_eq!(run(&mut e, "search gimp"), 0, "{}", e.err);
        assert!(
            e.err.contains("has not published the app list"),
            "{}",
            e.err
        );
        assert!(
            !e.err.contains("offline") && !e.err.contains("online"),
            "{}",
            e.err
        );
        let curl = e.execs.iter().find(|a| a[0] == "curl").unwrap();
        assert!(curl.contains(&"-fsL".to_string()), "{curl:?}");
        let (mut e, home) = env("unreachable");
        let _ = std::fs::remove_dir_all(home.join(".cache/wolos/packages"));
        e.exec_codes = vec![6];
        assert_ne!(run(&mut e, "update apps"), 0);
        assert!(e.err.contains("are you online?"), "{}", e.err);
        // The list came, its signature is not published: said as such, never "damaged".
        let (mut e, home) = env("nosig");
        let _ = std::fs::remove_dir_all(home.join(".cache/wolos/packages"));
        e.exec_codes = vec![0, 22];
        let _ = run(&mut e, "search gimp");
        assert!(!e.err.contains("online"), "{}", e.err);
    }

    /// QA-STORE-62: root's home is in the sealed slot; its cache is under /var, unless
    /// XDG_CACHE_HOME says otherwise. Another account keeps its own home's cache.
    #[test]
    fn root_cache_under_var() {
        let mut e = FakeEnv {
            root: true,
            home: PathBuf::from("/root"),
            ..FakeEnv::default()
        };
        assert_eq!(cache_dir(&e), Path::new("/var/cache/wolos/packages"));
        e.vars = vec![("XDG_CACHE_HOME".into(), "/x".into())];
        assert_eq!(cache_dir(&e), Path::new("/x/wolos/packages"));
        e.vars = vec![("XDG_CACHE_HOME".into(), String::new())];
        assert_eq!(cache_dir(&e), Path::new("/var/cache/wolos/packages"));
        let user = FakeEnv {
            home: PathBuf::from("/home/ana"),
            ..FakeEnv::default()
        };
        assert_eq!(
            cache_dir(&user),
            Path::new("/home/ana/.cache/wolos/packages")
        );
    }

    /// Audit 8 #5: an index above the cap is never read whole, fresh or cached.
    #[test]
    fn an_oversize_index_is_refused() {
        let (_e, home) = env("big");
        let d = home.join("big");
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("i");
        let max = package::INDEX_MAX_BYTES;
        let w = std::fs::File::create(&f).unwrap();
        w.set_len(max).unwrap();
        assert_eq!(read_capped(&f).unwrap().len() as u64, max);
        std::fs::File::create(&f).unwrap().set_len(max + 1).unwrap();
        assert!(read_capped(&f).is_err());
        let keys = vec![wol_core::package::public_of(&SEED)];
        let sig = d.join("s");
        std::fs::write(&sig, [0u8; 64]).unwrap();
        let e = adopt(&d, &f, Some(&sig), &keys, true).unwrap_err();
        assert!(e.contains("unusable"), "{e}");
    }

    /// R2C-10/R2C-23: a fresh pair replaces the cache only when it verifies.
    #[test]
    fn a_fresh_index_is_taken_only_when_it_verifies() {
        let (_e, home) = env("adopt");
        let bytes = wol("2.0", Some(&SEED));
        index_with(&home, "2.0", &bytes);
        let cache = home.join(".cache/wolos/packages");
        let keys = vec![wol_core::package::public_of(&SEED)];
        let good = std::fs::read(cache.join("index.json")).unwrap();
        let good_sig = std::fs::read(cache.join("index.json.sig")).unwrap();
        let text: &[u8] = br#"{"format":1,"packages":[]}"#;
        // A fresh index with a signature by another key: refused, the cache is untouched.
        let (fresh, fresh_sig) = (cache.join("i.part"), cache.join("s.part"));
        std::fs::write(&fresh, text).unwrap();
        std::fs::write(&fresh_sig, sign(&[9; 32], text)).unwrap();
        assert!(adopt(&cache, &fresh, Some(&fresh_sig), &keys, true).is_err());
        assert_eq!(std::fs::read(cache.join("index.json")).unwrap(), good);
        assert_eq!(
            std::fs::read(cache.join("index.json.sig")).unwrap(),
            good_sig
        );
        assert!(!fresh.exists() && !fresh_sig.exists());
        // No signature while one is required: refused too.
        std::fs::write(&fresh, text).unwrap();
        assert!(adopt(&cache, &fresh, None, &keys, true).is_err());
        assert_eq!(std::fs::read(cache.join("index.json")).unwrap(), good);
        // A good pair replaces both.
        std::fs::write(&fresh, text).unwrap();
        std::fs::write(&fresh_sig, sign(&SEED, text)).unwrap();
        assert!(adopt(&cache, &fresh, Some(&fresh_sig), &keys, true).is_ok());
        assert_eq!(std::fs::read(cache.join("index.json")).unwrap(), text);
        assert_ne!(
            std::fs::read(cache.join("index.json.sig")).unwrap(),
            good_sig
        );
    }

    #[test]
    fn a_stripped_index_signature_is_refused() {
        // CORE-8: once a cache held a signed index, an index without its .sig is an error.
        let bytes = wol("2.0", Some(&SEED));
        let (mut e, home) = env("strip");
        index_with(&home, "2.0", &bytes);
        let cache = home.join(".cache/wolos/packages");
        std::fs::write(cache.join("wol-fixture-2.0.wol"), &bytes).unwrap();
        assert_eq!(run(&mut e, "install org.wolos.Fixture"), 0, "{}", e.err);
        assert!(cache.join("index.signed").exists());
        std::fs::remove_file(cache.join("index.json.sig")).unwrap();
        let mut again = FakeEnv {
            home: home.clone(),
            vars: e.vars.clone(),
            replies: vec!["ok\n".into()],
            ..FakeEnv::default()
        };
        assert_ne!(run(&mut again, "install org.wolos.Fixture"), 0);
        assert!(again.requests.is_empty(), "{:?}", again.requests);
        // The official index with trusted keys installed: never unsigned, even on a new cache.
        let (mut off, home) = env("official");
        off.vars.retain(|(k, _)| k != INDEX_VAR);
        index_with(&home, "2.0", &bytes);
        std::fs::remove_file(home.join(".cache/wolos/packages/index.json.sig")).unwrap();
        assert_ne!(run(&mut off, "install org.wolos.Fixture"), 0);
        assert!(off.requests.is_empty());
        assert!(off.err.contains("signature"), "{}", off.err);
    }
}

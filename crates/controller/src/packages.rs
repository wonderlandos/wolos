//! `wol install|remove|list|search`, `wol update apps` (ADR 0011, archicture/modules/packages.md).
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

/// `$XDG_CACHE_HOME/wolos/packages` (downloads and the index).
fn cache_dir(env: &dyn Env) -> PathBuf {
    env.var("XDG_CACHE_HOME")
        .map_or_else(|| env.home().join(".cache"), PathBuf::from)
        .join("wolos/packages")
}

fn fetch(env: &mut dyn Env, url: &str, dest: &Path) -> bool {
    let argv = package::fetch_argv(url, &dest.to_string_lossy());
    env.exec_quiet(&argv).is_ok_and(|c| c == 0)
}

/// Fresh index, else the cached copy; None when there is neither. A bad signature or a malformed
/// index is an error, never "no index".
fn index(env: &mut dyn Env) -> Result<Option<Index>, String> {
    let dir = cache_dir(env);
    let _ = std::fs::create_dir_all(&dir);
    let url = env.var(INDEX_VAR).unwrap_or_else(|| INDEX_URL.into());
    let (file, sig) = (dir.join("index.json"), dir.join("index.json.sig"));
    if !fetch(env, &url, &file) {
        env.err("wol: index unavailable, using the cached copy\n");
    } else if !fetch(env, &format!("{url}{INDEX_SIG_SUFFIX}"), &sig) {
        let _ = std::fs::remove_file(&sig);
    }
    let Ok(text) = std::fs::read(&file) else {
        return Ok(None);
    };
    let sig = std::fs::read(&sig).ok();
    package::verify_index(&text, sig.as_deref(), &keys(env))?;
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
    if !fresh && !fetch(env, &e.url, &dest) {
        return fail(
            env,
            ErrorCode::Unavailable,
            &format!("download failed: {}", e.url),
        );
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
        Ok(None) => env.err("wol: no package index (offline?)\n"),
        Err(e) => env.err(&format!("wol: package index: {e}\n")),
    }
    print_rows(env, rows, json, "no matches")
}

fn updates(env: &mut dyn Env, yes: bool, json: bool) -> i32 {
    let index = match index(env) {
        Ok(Some(i)) => i,
        Ok(None) => return fail(env, ErrorCode::Unavailable, "no package index (offline?)"),
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
}

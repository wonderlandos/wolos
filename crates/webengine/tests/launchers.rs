// SPDX-License-Identifier: BSD-2-Clause
//! System web apps in the image (ADR 0048, QA-WEB-4): the launcher entry of every shipped app is
//! `Manifest::desktop_entry` for the installed folder, kept as a file under `webapps/launchers/`
//! (`WOL_REGEN=1` rewrites them) and installed by the `wolos-webapps` package through
//! `tools/install-webapps.sh`, which leaves out the development files. The entries are not under
//! `files/`: wolos-system installs that whole tree, and two packages must not own one file.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use webengine::bridge::manifest::{KIT, Manifest, SYSTEM_DIR};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
fn launchers() -> PathBuf {
    repo().join("webapps/launchers")
}
/// The slugs `webapps/SHIPPED` lists.
fn shipped() -> Vec<String> {
    read(&repo().join("webapps/SHIPPED"))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}
/// Folders of `webapps/` that are apps (not the kit, not the launcher folder).
fn folders() -> BTreeSet<String> {
    std::fs::read_dir(repo().join("webapps"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|n| n != KIT && n != "launchers")
        .collect()
}
fn entry_file(slug: &str) -> PathBuf {
    launchers().join(format!("org.wolos.webapp.{slug}.desktop"))
}
fn generated(slug: &str) -> String {
    let dir = repo().join("webapps").join(slug);
    let m = Manifest::load(&dir).unwrap_or_else(|e| panic!("{slug}: {e}"));
    m.desktop_entry(&Path::new(SYSTEM_DIR).join(slug))
}

/// Folders that are samples on purpose (a starter to copy, never an app of the image).
const SAMPLES: &[&str] = &["hello"];

#[test]
fn every_web_app_folder_is_shipped_or_a_named_sample() {
    let shipped: BTreeSet<String> = shipped().into_iter().collect();
    let samples: BTreeSet<String> = SAMPLES.iter().map(|s| s.to_string()).collect();
    assert!(!shipped.is_empty(), "webapps/SHIPPED lists no app");
    assert!(shipped.is_disjoint(&samples), "{shipped:?} {samples:?}");
    let all: BTreeSet<String> = shipped.union(&samples).cloned().collect();
    assert_eq!(
        folders(),
        all,
        "a webapps/ folder is neither shipped nor a sample"
    );
    assert!(shipped.contains("doctor"), "Doctor is the app of the image");
}

#[test]
fn every_shipped_app_has_its_generated_launcher_entry() {
    let regen = std::env::var_os("WOL_REGEN").is_some();
    let mut files = BTreeSet::new();
    for slug in shipped() {
        let want = generated(&slug);
        let path = entry_file(&slug);
        if regen {
            std::fs::create_dir_all(launchers()).unwrap();
            std::fs::write(&path, &want).unwrap();
        }
        assert_eq!(read(&path), want, "{slug}: WOL_REGEN=1 rewrites it");
        files.insert(path.file_name().unwrap().to_string_lossy().into_owned());
        // The window class is the app id: the shell matches the window to this entry, the Memory
        // Saver its scope (ADR 0048).
        assert!(
            want.contains("Exec=/usr/bin/wol-webhost /usr/share/wolos/webapps/"),
            "{want}"
        );
        assert!(
            want.contains(&format!("StartupWMClass=org.wolos.webapp.{slug}\n")),
            "{want}"
        );
    }
    // No entry without an app (a removed app must take its launcher with it).
    let on_disk: BTreeSet<String> = std::fs::read_dir(launchers())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        on_disk, files,
        "an entry in webapps/launchers/ without a shipped app"
    );
}

/// Everything below `dir`, relative, sorted.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, base: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

#[test]
fn the_package_script_installs_the_apps_without_the_development_files() {
    let dest = std::env::temp_dir().join(format!("wol-webapps-pkg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    let o = Command::new("sh")
        .arg(repo().join("tools/install-webapps.sh"))
        .arg(&dest)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let sys = dest.join(SYSTEM_DIR.trim_start_matches('/'));
    let all = tree(&dest);
    for dev in ["mock-bridge.js", "fixtures.js"] {
        assert!(
            !all.iter().any(|f| f.ends_with(dev)),
            "{dev} is shipped: {all:?}"
        );
    }
    assert!(
        !all.iter()
            .any(|f| f.contains("/fixtures/") || f.contains("/test/")),
        "{all:?}"
    );
    // The kit's runtime files, and each app as the host loads it.
    for f in [
        "bridge.js",
        "dev.js",
        "error.html",
        "error.js",
        "wolkit.js",
        "wolkit.css",
    ] {
        assert!(sys.join(KIT).join(f).is_file(), "kit/{f}");
    }
    for slug in shipped() {
        let m = Manifest::load(&sys.join(&slug)).unwrap_or_else(|e| panic!("{slug}: {e}"));
        assert_eq!(m.slug(), slug);
        assert_eq!(
            read(&dest.join(format!(
                "usr/share/applications/org.wolos.webapp.{slug}.desktop"
            ))),
            generated(&slug)
        );
        // A page loads only what the package installs: a script or style it names is there, but
        // for the development ones, which `dev.js` loads on ?dev alone (QA-WEB-11).
        let html = read(&sys.join(&slug).join(&m.entry));
        for r in refs(&html) {
            let p = sys.join(&slug).join(&r);
            assert!(
                p.is_file() || p.canonicalize().map(|c| c.is_file()).unwrap_or(false),
                "{slug}: {r} is not installed"
            );
        }
    }
    // The ISO has 21 MiB of room under the release asset limit (prerelease run 2026-11-03): the
    // apps are text and one icon each. A package of this kind that outgrows 1 MiB needs a look.
    let bytes: u64 = tree(&dest)
        .iter()
        .map(|f| std::fs::metadata(dest.join(f)).unwrap().len())
        .sum();
    assert!(bytes < 1 << 20, "wolos-webapps is {bytes} bytes");
    let _ = std::fs::remove_dir_all(&dest);
}

/// A copy of the source tree's `webapps/` and `files/`-free pieces the script needs, in a temp
/// folder with the script beside it, so a piece can be taken away.
fn mini_repo(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("wol-webapps-mini-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tools")).unwrap();
    std::fs::copy(
        repo().join("tools/install-webapps.sh"),
        root.join("tools/install-webapps.sh"),
    )
    .unwrap();
    let copy = Command::new("cp")
        .arg("-r")
        .arg(repo().join("webapps"))
        .arg(&root)
        .status()
        .unwrap();
    assert!(copy.success());
    root
}

fn run_script(root: &Path) -> std::process::Output {
    let dest = root.join("dest");
    Command::new("sh")
        .arg(root.join("tools/install-webapps.sh"))
        .arg(&dest)
        .output()
        .unwrap()
}

#[test]
fn the_script_fails_when_a_piece_is_missing_instead_of_installing_half() {
    // The whole thing works in the copy first, so a failure below is the missing piece's.
    let root = mini_repo("ok");
    assert!(run_script(&root).status.success());
    let _ = std::fs::remove_dir_all(&root);
    for (tag, gone, say) in [
        ("kit", "webapps/kit", "kit"),
        (
            "entry",
            "webapps/launchers/org.wolos.webapp.doctor.desktop",
            "no launcher entry",
        ),
        ("manifest", "webapps/doctor/app.toml", "no app.toml"),
        ("list", "webapps/SHIPPED", "SHIPPED"),
    ] {
        let root = mini_repo(tag);
        let p = root.join(gone);
        if p.is_dir() {
            std::fs::remove_dir_all(&p).unwrap();
        } else {
            std::fs::remove_file(&p).unwrap();
        }
        let o = run_script(&root);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            !o.status.success(),
            "{gone} is gone and the script went on: {err}"
        );
        assert!(err.contains(say), "{gone}: {err}");
        let _ = std::fs::remove_dir_all(&root);
    }
    // An empty kit is no kit.
    let root = mini_repo("emptykit");
    for e in std::fs::read_dir(root.join("webapps/kit")).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            std::fs::remove_dir_all(p).unwrap();
        } else {
            std::fs::remove_file(p).unwrap();
        }
    }
    assert!(!run_script(&root).status.success());
    let _ = std::fs::remove_dir_all(&root);
}

/// The `src` / `href` values of an entry page that are local files.
fn refs(html: &str) -> Vec<String> {
    let mut v = Vec::new();
    for attr in ["src=\"", "href=\""] {
        for (i, _) in html.match_indices(attr) {
            let rest = &html[i + attr.len()..];
            let r = &rest[..rest.find('"').unwrap()];
            if !r.is_empty() && !r.starts_with('#') && !r.contains(':') {
                v.push(r.to_owned());
            }
        }
    }
    v
}

/// A shipped page makes no request for a development file: those are named only in `data-dev`.
#[test]
fn shipped_pages_name_development_scripts_only_in_data_dev() {
    for slug in shipped() {
        let html = read(&repo().join("webapps").join(&slug).join("index.html"));
        for r in refs(&html) {
            assert!(
                !r.ends_with("mock-bridge.js") && !r.ends_with("fixtures.js"),
                "{slug}/index.html loads {r} on every start (QA-WEB-11): name it in dev.js's data-dev"
            );
        }
        assert!(
            html.contains("../kit/dev.js"),
            "{slug}/index.html loads dev.js"
        );
    }
}

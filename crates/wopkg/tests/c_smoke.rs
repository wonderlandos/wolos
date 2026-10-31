//! libwopkg from C (ADR 0021): `tests/c/smoke.c` is compiled with `cc` against `include/wopkg.h`,
//! linked against the built `libwopkg.so.1` by soname, and run on a local pacman database and a
//! Flathub AppStream file: context, index update with progress, unified search, completion, errors.
use std::io::Write;
use std::path::Path;

mod common;

fn tar(files: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, body) in files {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
        h[156] = b'0';
        out.extend_from_slice(&h);
        out.extend_from_slice(body.as_bytes());
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

#[test]
fn c_program_searches_two_sources() {
    let d = std::env::temp_dir().join(format!("wopkg-c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar(&[(
        "firefox-140.0-1/desc",
        "%NAME%\nfirefox\n\n%VERSION%\n140.0-1\n\n%DESC%\nWeb browser\n",
    )]))
    .unwrap();
    std::fs::write(d.join("extra.db"), gz.finish().unwrap()).unwrap();
    std::fs::write(d.join("appstream.xml"), r#"<components><component type="desktop-application"><id>org.mozilla.firefox</id><name>Firefox</name><summary>Browser</summary><bundle type="flatpak">app/org.mozilla.firefox/x86_64/stable</bundle></component></components>"#).unwrap();
    let config = format!(
        r#"{{"home":"{0}","cache_dir":"{0}/cache","state_dir":"{0}/state","pacman_local":"{0}/none"}}"#,
        d.display()
    );
    let req = format!(
        r#"{{"sources":[{{"name":"extra","kind":"pacman","path":"{0}/extra.db"}},{{"name":"flathub","kind":"flatpak","path":"{0}/appstream.xml"}}]}}"#,
        d.display()
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = common::lib_dir();
    let exe = d.join("smoke");
    let st = std::process::Command::new(std::env::var("CC").unwrap_or("cc".into()))
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&exe)
        .arg(root.join("tests/c/smoke.c"))
        .arg(format!("-I{}", root.join("include").display()))
        .arg(format!("-L{}", lib.display()))
        .arg("-lwopkg")
        .status()
        .expect("cc");
    assert!(st.success());
    let out = std::process::Command::new(&exe)
        .arg(&config)
        .arg(&req)
        .env("LD_LIBRARY_PATH", lib)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = |p: &str| {
        text.lines()
            .find(|l| l.starts_with(p))
            .unwrap_or_else(|| panic!("{p} in {text}"))
            .to_owned()
    };
    assert_eq!(line("version:"), "version: 1.0.0 65536");
    let search: serde_json::Value =
        serde_json::from_str(line("search:").trim_start_matches("search: ")).unwrap();
    let r = &search["results"][0];
    assert_eq!(r["key"], "firefox");
    let kinds: Vec<&str> = r["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["pacman", "flatpak"], "one result, both sources");
    assert!(line("events:") != "events: 0");
    assert_eq!(line("complete:"), "complete: firefox");
    assert_eq!(line("info status:"), "info status: 6");
    assert!(line("info failed:").contains("No app or package is called"));
    assert_eq!(line("null context:"), "null context: 1");
}

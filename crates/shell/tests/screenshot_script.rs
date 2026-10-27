//! wol-screenshot contract with the shell: prints the saved path and exits, even though
//! wl-copy leaves a clipboard daemon running. Live Hyprland: the daemon inherited stdout,
//! so the shell waited forever for EOF and never showed the thumbnail.
use std::time::{Duration, Instant};

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../desktop/overlay/usr/local/bin/wol-screenshot"
);

fn stub(dir: &std::path::Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn prints_the_path_and_returns_while_the_clipboard_daemon_lives() {
    let tmp = std::env::temp_dir().join(format!("wol-shot-test-{}", std::process::id()));
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // grim writes the file; wl-copy forks a long-lived daemon like the real one.
    stub(&bin, "grim", r#"for a; do f=$a; done; printf png > "$f""#);
    stub(&bin, "wl-copy", "cat >/dev/null; (sleep 30) &");
    stub(&bin, "notify-send", "exit 0");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let t0 = Instant::now();
    let out = std::process::Command::new("sh")
        .args([SCRIPT, "full", "--dir"])
        .arg(tmp.join("shots"))
        .env("PATH", path)
        .env("WOL_SCREENSHOT_QUIET", "1")
        .output()
        .unwrap();
    let took = t0.elapsed();
    let printed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        took < Duration::from_secs(5),
        "took {took:?}: stdout held open"
    );
    assert!(
        printed.ends_with(".png") && printed.contains("/shots/Screenshot "),
        "{printed}"
    );
}

/// ⇧⌘3 saves to the Desktop (macOS) and the banner names the folder, never the full path; the
/// picture rides along as the banner's thumbnail (`image-path` hint).
#[test]
fn saves_to_the_desktop_and_the_banner_shows_a_thumbnail_not_a_path() {
    let tmp = std::env::temp_dir().join(format!("wol-shot-banner-{}", std::process::id()));
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let said = tmp.join("notify-args");
    stub(&bin, "grim", r#"for a; do f=$a; done; printf png > "$f""#);
    stub(&bin, "wl-copy", "cat >/dev/null");
    stub(
        &bin,
        "notify-send",
        &format!(
            r#"for a; do printf '%s\n' "$a"; done > '{}'"#,
            said.display()
        ),
    );
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("sh")
        .args([SCRIPT, "full"])
        .env("PATH", path)
        .env("HOME", &tmp)
        .env_remove("XDG_DESKTOP_DIR")
        .env_remove("WOL_SCREENSHOT_QUIET")
        .output()
        .unwrap();
    let printed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let args = std::fs::read_to_string(&said).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        printed.starts_with(&format!("{}/Desktop/Screenshot ", tmp.display())),
        "{printed}"
    );
    let args: Vec<&str> = args.lines().collect();
    assert!(args.contains(&"Screenshot saved to Desktop"), "{args:?}");
    assert!(
        args.contains(&format!("string:image-path:{printed}").as_str()),
        "{args:?}"
    );
    assert!(
        !args.contains(&printed.as_str()),
        "the path as text: {args:?}"
    );
}

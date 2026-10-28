//! `wol-run --exe` end to end on the build host: a fake wine (`WOL_WINE`) records its argv and
//! environment, so the prefix step, the DLL copy and the program launch are all visible.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct World {
    root: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("wol-exe-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = World { root };
        std::fs::create_dir_all(w.p("bin")).unwrap();
        write_exec(
            &w.p("bin/wine"),
            r#"#!/bin/sh
{ printf 'argv:'; for a in "$@"; do printf ' [%s]' "$a"; done; printf '\n'; } >> "$FAKE_WINE_LOG"
case "$1" in
  --version) echo wine-11.20; exit 0 ;;
  wineboot) mkdir -p "$WINEPREFIX/drive_c/windows/system32" "$WINEPREFIX/drive_c/windows/syswow64"
            : > "$WINEPREFIX/system.reg"; echo "wineboot output"; exit 0 ;;
esac
env | sort | sed 's/^/env: /' >> "$FAKE_WINE_LOG"
printf 'cwd: %s\n' "$(pwd)" >> "$FAKE_WINE_LOG"
echo "game stdout"
exit 7
"#,
        );
        for d in ["x64", "x32"] {
            std::fs::create_dir_all(w.p(&format!("dxvk/{d}"))).unwrap();
            std::fs::write(w.p(&format!("dxvk/{d}/d3d11.dll")), d).unwrap();
        }
        std::fs::create_dir_all(w.p("games/Game")).unwrap();
        std::fs::write(w.p("games/Game/game.exe"), "MZ").unwrap();
        w
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    fn exe(&self) -> PathBuf {
        self.p("games/Game/game.exe")
    }
    fn prefix(&self) -> PathBuf {
        self.p("home/.local/share/wolos/prefixes")
            .join(proxy::exe::exe_id(&self.exe()))
            .join("pfx")
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.p("log")).unwrap_or_default()
    }
    fn run(&self, args: &[&str]) -> Output {
        let s = |p: &str| self.p(p).to_string_lossy().into_owned();
        Command::new(env!("CARGO_BIN_EXE_wol-run"))
            .args(args)
            .env_clear()
            .env("HOME", s("home"))
            .env("PATH", "/usr/bin:/bin")
            .env("MESA_VK_WSI_PRESENT_MODE", "fifo")
            .env("WOL_WINE", s("bin/wine"))
            .env("WOL_GPU_PRESET", "amd")
            .env("WOL_DXVK_DIR", s("dxvk"))
            .env("FAKE_WINE_LOG", s("log"))
            .output()
            .unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write_exec(p: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(p, text).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn env_of(log: &str) -> Vec<(String, String)> {
    log.lines()
        .filter_map(|l| l.strip_prefix("env: ")?.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

#[test]
fn exe_runs_on_wine_in_its_own_prefix_with_dxvk() {
    let w = World::new("run");
    let exe = w.exe();
    let out = w.run(&["--exe", &exe.to_string_lossy(), "-dx11"]);
    let log = w.log();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(7), "{log}\n{stderr}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "game stdout\n");
    assert!(stderr.contains("(wine, 1 DLLs native-first)"), "{stderr}");
    let steps: Vec<&str> = log.lines().filter(|l| l.starts_with("argv:")).collect();
    let game = format!("argv: [{}] [-dx11]", exe.display());
    assert_eq!(
        steps,
        ["argv: [--version]", "argv: [wineboot] [-u]", game.as_str()]
    );
    let env = env_of(&log);
    let get = |k: &str| env.iter().find(|e| e.0 == k).map(|e| e.1.as_str());
    let pfx = w.prefix();
    let table: &[(&str, Option<&str>)] = &[
        ("WINEPREFIX", Some(&pfx.to_string_lossy())),
        ("WINEDLLOVERRIDES", Some("d3d11=n,b")),
        ("MESA_VK_WSI_PRESENT_MODE", Some("fifo")),
        ("WINEDEBUG", Some("-all")),
        ("WINEESYNC", None),
        ("PATH", Some("/usr/bin:/bin")),
    ];
    for (k, want) in table {
        assert_eq!(get(k), *want, "{k}");
    }
    assert!(log.contains(&format!("cwd: {}", w.p("games/Game").display())));
    let sys = pfx.join("drive_c/windows");
    assert_eq!(
        std::fs::read_to_string(sys.join("system32/d3d11.dll")).unwrap(),
        "x64"
    );
    assert_eq!(
        std::fs::read_to_string(sys.join("syswow64/d3d11.dll")).unwrap(),
        "x32"
    );

    // Second launch: the prefix exists, no wineboot.
    std::fs::remove_file(w.p("log")).unwrap();
    assert_eq!(
        w.run(&["--exe", &exe.to_string_lossy()]).status.code(),
        Some(7)
    );
    assert!(!w.log().contains("wineboot"), "{}", w.log());
}

#[test]
fn missing_program_and_old_flags_fail_cleanly() {
    let w = World::new("bad");
    let out = w.run(&["--exe", "/nonexistent/x.exe"]);
    assert_eq!(
        out.status.code(),
        Some(shared::errors::ErrorCode::NotFound.exit())
    );
    let out = w.run(&["--steam", "run", "x.exe"]);
    assert_eq!(
        out.status.code(),
        Some(shared::errors::ErrorCode::Usage.exit()),
        "Steam runs its own Proton"
    );
}

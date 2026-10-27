//! The generated WolOS Proton tool, end to end on the build host: Steam-like environment ->
//! launcher (sh) -> wol-run --steam -> a fake wine that records argv and environment.
//! The FreeBSD-only part (Linux sh exec'ing a FreeBSD ELF) is image/vm/test/games.sh.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use wol_core::games::steam;

struct World {
    root: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("wol-steam-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = World { root };
        let bin = w.p("wine/bin");
        std::fs::create_dir_all(&bin).unwrap();
        write_exec(
            &bin.join("wine"),
            r#"#!/bin/sh
{ printf 'argv:'; for a in "$@"; do printf ' [%s]' "$a"; done; printf '\n'; } >> "$FAKE_WINE_LOG"
case "$1" in
  --version) echo wine-10.0; exit 0 ;;
  wineboot) mkdir -p "$WINEPREFIX/drive_c/windows/system32" "$WINEPREFIX/drive_c/windows/syswow64"
            : > "$WINEPREFIX/system.reg"; echo "wineboot output"; exit 0 ;;
  winepath) echo 'Z:\games\Game'; exit 0 ;;
esac
env | sort | sed 's/^/env: /' >> "$FAKE_WINE_LOG"
printf 'cwd: %s\n' "$(pwd)" >> "$FAKE_WINE_LOG"
echo "game stdout"
exit 7
"#,
        );
        write_exec(
            &bin.join("wineserver"),
            "#!/bin/sh\nprintf 'wineserver: %s\\n' \"$*\" >> \"$FAKE_WINE_LOG\"\n",
        );
        for d in ["x64", "x32"] {
            std::fs::create_dir_all(w.p(&format!("dxvk/{d}"))).unwrap();
            std::fs::write(w.p(&format!("dxvk/{d}/d3d11.dll")), d).unwrap();
        }
        std::fs::create_dir_all(w.p("lib/steamapps/common/Game")).unwrap();
        std::fs::write(w.p("lib/steamapps/common/Game/game.exe"), "MZ").unwrap();
        std::fs::create_dir_all(w.p("lib/steamapps/compatdata/42")).unwrap();
        let installed = steam::install(&w.steam_root(), env!("CARGO_BIN_EXE_wol-run")).unwrap();
        assert_eq!(installed, steam::Installed::Created);
        w
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    fn steam_root(&self) -> PathBuf {
        self.p("home/.steam/steam")
    }
    fn compat(&self) -> PathBuf {
        self.p("lib/steamapps/compatdata/42")
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.p("log")).unwrap_or_default()
    }
    /// Steam's view, as linux-steam-utils leaves it: Linux PATH and preload, FreeBSD values
    /// saved in LSU_FBSD_*, Steam Runtime and pressure-vessel variables.
    fn steam(&self, args: &[&str]) -> Output {
        let s = |p: &str| self.p(p).to_string_lossy().into_owned();
        let launcher = steam::tool_dir(&self.steam_root()).join(steam::LAUNCHER);
        Command::new("/bin/sh")
            .arg(launcher)
            .args(args)
            .current_dir(self.p("lib/steamapps/common/Game"))
            .env_clear()
            .env("HOME", s("home"))
            // Linux-side values wol-run's own filter would not recognise: only the
            // launcher's LSU_FBSD_* swap removes them.
            .env("PATH", "/opt/steam-linux/bin:/usr/bin:/bin")
            .env("LSU_FBSD_PATH", "/usr/bin:/bin")
            .env("LD_LIBRARY_PATH", "/opt/steam-linux/lib")
            .env("LD_PRELOAD", "/opt/steam-linux/overlay.so")
            .env(
                "STEAM_RUNTIME",
                s("home/.steam/steam/ubuntu12_32/steam-runtime"),
            )
            .env("PRESSURE_VESSEL_FILESYSTEMS_RO", "/compat")
            .env("STEAM_COMPAT_DATA_PATH", self.compat())
            .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", self.steam_root())
            .env("SteamGameId", "42")
            .env("MESA_VK_WSI_PRESENT_MODE", "fifo")
            .env("WOL_WINE", s("wine/bin/wine"))
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
fn waitforexitandrun_runs_the_game_on_wine_with_a_clean_environment() {
    let w = World::new("run");
    let game = w.p("lib/steamapps/common/Game/game.exe");
    let out = w.steam(&["waitforexitandrun", &game.to_string_lossy(), "-dx11"]);
    let log = w.log();
    assert_eq!(
        out.status.code(),
        Some(7),
        "{log}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "game stdout\n",
        "helpers write to stderr"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("wolos-proton: waitforexitandrun"),
        "{stderr}"
    );
    let steps: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("argv:") || l.starts_with("wineserver:"))
        .collect();
    let game_argv = format!("argv: [{}] [-dx11]", game.display());
    assert_eq!(
        steps,
        [
            "argv: [--version]",
            "argv: [wineboot] [-u]",
            "wineserver: -w",
            game_argv.as_str()
        ]
    );
    let env = env_of(&log);
    let get = |k: &str| env.iter().find(|e| e.0 == k).map(|e| e.1.as_str());
    let pfx = w.compat().join("pfx");
    let wine_bin = w.p("wine/bin");
    let table: &[(&str, Option<&str>)] = &[
        ("WINEPREFIX", Some(&pfx.to_string_lossy())),
        (
            "PATH",
            Some(&format!("{}:/usr/bin:/bin", wine_bin.display())),
        ),
        ("LD_PRELOAD", None),
        ("LD_LIBRARY_PATH", None),
        ("LSU_LINUX_PATH", None),
        ("LSU_LINUX_LD_LIBRARY_PATH", None),
        ("STEAM_RUNTIME", None),
        ("PRESSURE_VESSEL_FILESYSTEMS_RO", None),
        ("DISABLE_VK_LAYER_VALVE_steam_overlay_1", Some("1")),
        ("WINEDLLOVERRIDES", Some("d3d11=n,b")),
        ("MESA_VK_WSI_PRESENT_MODE", Some("fifo")),
        ("WINEESYNC", Some("1")),
        ("WINEDEBUG", Some("-all")),
        ("SteamGameId", Some("42")),
    ];
    for (k, want) in table {
        assert_eq!(get(k), *want, "{k}");
    }
    assert!(log.contains(&format!(
        "cwd: {}",
        w.p("lib/steamapps/common/Game").display()
    )));
    let sys = pfx.join("drive_c/windows");
    assert_eq!(
        std::fs::read_to_string(sys.join("system32/d3d11.dll")).unwrap(),
        "x64"
    );
    assert_eq!(
        std::fs::read_to_string(sys.join("syswow64/d3d11.dll")).unwrap(),
        "x32"
    );

    // Second launch: prefix exists, no wineboot.
    std::fs::remove_file(w.p("log")).unwrap();
    assert_eq!(
        w.steam(&["run", &game.to_string_lossy()]).status.code(),
        Some(7)
    );
    let again = w.log();
    assert!(
        !again.contains("wineboot") && !again.contains("wineserver"),
        "{again}"
    );
}

#[test]
fn path_verbs_print_only_the_path() {
    let w = World::new("paths");
    let out = w.steam(&["getcompatpath", "/games/Game"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Z:\\games\\Game\n");
    assert!(w.log().contains("argv: [winepath] [-w] [/games/Game]"));
    let out = w.steam(&["getnativepath", "C:\\users"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(w.log().contains("argv: [winepath] [-u] [C:\\users]"));
}

#[test]
fn destroyprefix_and_bad_verbs() {
    let w = World::new("destroy");
    std::fs::create_dir_all(w.compat().join("pfx/drive_c")).unwrap();
    assert_eq!(w.steam(&["destroyprefix"]).status.code(), Some(0));
    assert!(!w.compat().join("pfx").exists());
    assert!(w.compat().exists(), "only the prefix goes");
    let out = w.steam(&["bogus", "x.exe"]);
    assert_eq!(
        out.status.code(),
        Some(shared::errors::ErrorCode::Usage.exit())
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown verb 'bogus'"));
}

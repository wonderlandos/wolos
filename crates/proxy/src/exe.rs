//! `wol-run --exe <file.exe|file.msi> [args]`: double-click a Windows program. Each program gets
//! its own prefix, named by its path; it runs on native wine (or umu-launcher) from its own
//! directory.
use crate::wine::{EnvInput, Fs, Runner, Wine, WinePlan, pick_dlls, proton_dirs, wine_env};
use crate::{LaunchPlan, PREFIXES, dlls};
use shared::errors::{ErrorCode, WolError};
use std::collections::BTreeMap;
use std::path::Path;
use wol_core::games::steam;

/// FNV-1a 64: stable across Rust releases (unlike `DefaultHasher`).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Prefix id: `exe-<file stem>-<8 hex digits of the path hash>`, a valid bundle id.
pub fn exe_id(path: &Path) -> String {
    let stem: String = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(32)
        .collect();
    let stem = stem.trim_matches('-');
    let hash = fnv1a(path.as_os_str().as_encoded_bytes()) as u32;
    format!(
        "exe-{}-{hash:08x}",
        if stem.is_empty() { "app" } else { stem }
    )
}

/// Windows programs are opened by wine, not started by the kernel, so the first-use check is asked
/// for here: exit 1 from `wol-xguard check` means it found something. Anything else lets the
/// program start: a missing check never stops an app.
pub fn refused(check_exit: Option<i32>, path: &Path) -> Option<WolError> {
    (check_exit == Some(1)).then(|| {
        WolError::new(
            ErrorCode::Failed,
            format!(
                "{} was stopped: it matches known malware. See Privacy & Security in Settings.",
                path.display()
            ),
        )
    })
}

/// Exit code of `wol-xguard check <path>`.
pub fn check(path: &Path) -> Option<i32> {
    std::process::Command::new("wol-xguard")
        .arg("check")
        .arg(path)
        .stdout(std::process::Stdio::null())
        .status()
        .ok()?
        .code()
}

pub fn plan(
    path: &Path,
    args: &[String],
    incoming: &BTreeMap<String, String>,
    wine: &Wine,
    home: &Path,
    fs: &dyn Fs,
) -> Result<WinePlan, WolError> {
    if !path.is_absolute() || !fs.exists(path) {
        return Err(WolError::new(
            ErrorCode::NotFound,
            format!("{}: no such program", path.display()),
        ));
    }
    let msi = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("msi"));
    let file = path.to_string_lossy().into_owned();
    let mut tail = if msi {
        vec!["msiexec".into(), "/i".into(), file]
    } else {
        vec![file]
    };
    tail.extend(args.iter().cloned());
    let runner = wine.runner(fs).ok_or_else(|| {
        WolError::new(
            ErrorCode::NotFound,
            "wine is not installed: sudo pacman -S wine",
        )
    })?;
    let prefix = home.join(PREFIXES).join(exe_id(path)).join("pfx");
    // Proton (umu) brings its own DXVK; wine gets the copy from a Proton Steam installed.
    let sources = match runner {
        Runner::Wine(_) => pick_dlls(
            incoming,
            wine.vendor,
            &proton_dirs(&steam_libraries(home, fs), fs),
            fs,
        ),
        Runner::Umu(_) => vec![],
    };
    let (copies, names) = dlls::plan(&sources, &|d| fs.list(d), &prefix);
    let defaults = runner.defaults(wine.vendor);
    let env = wine_env(&EnvInput {
        incoming,
        prefix: &prefix,
        defaults: &defaults,
        dlls: &names,
    });
    Ok(WinePlan {
        summary: format!(
            "wol-run: {} in {} ({}, {} DLLs native-first)",
            path.display(),
            prefix.display(),
            runner.name(),
            names.len()
        ),
        launch: LaunchPlan {
            argv: std::iter::once(runner.path().to_string_lossy().into_owned())
                .chain(tail)
                .collect(),
            env,
            env_clear: true,
            cwd: path.parent().map(|d| d.to_string_lossy().into_owned()),
            relay_seekable_stdio: true,
        },
        ensure_prefix: matches!(runner, Runner::Wine(_)),
        runner: Some(runner),
        prefix,
        copies,
    })
}

/// Libraries of the Steam client (DXVK lookup): the client's and every `libraryfolders.vdf` entry.
fn steam_libraries(home: &Path, fs: &dyn Fs) -> Vec<std::path::PathBuf> {
    let root = steam::steam_root(home, &|p| fs.exists(p));
    steam::libraries(&root, &|p| fs.read(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_finding_stops_a_windows_program() {
        let p = Path::new("/home/a/setup.exe");
        let found_stops_it = refused(Some(1), p).expect("stopped");
        assert!(found_stops_it.to_string().contains("setup.exe"));
        for other in [Some(0), Some(2), Some(3), None] {
            assert!(refused(other, p).is_none(), "{other:?}");
        }
    }
    use crate::tuning::Vendor;
    use crate::wine::fake::FakeFs;
    use wol_core::games::wine::Version;

    #[test]
    fn ids_are_stable_and_valid() {
        let a = exe_id(Path::new("/home/ana/Downloads/Setup Game (x64).exe"));
        assert_eq!(
            a,
            exe_id(Path::new("/home/ana/Downloads/Setup Game (x64).exe"))
        );
        assert!(a.starts_with("exe-setup-game--x64-"), "{a}");
        assert!(crate::valid_id(&a), "{a}");
        assert_ne!(a, exe_id(Path::new("/home/ana/Other/Setup Game (x64).exe")));
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert!(exe_id(Path::new("/x/\u{fc}.exe")).starts_with("exe-app-"));
    }

    fn wine(umu: bool) -> Wine {
        Wine {
            bin: "/usr/bin/wine".into(),
            umu: umu.then(|| "/usr/bin/umu-run".into()),
            forced: false,
            version: Version::parse("wine-11.20"),
            vendor: Vendor::Amd,
        }
    }

    fn incoming() -> BTreeMap<String, String> {
        [("WOL_DXVK_DIR", "/opt/dxvk"), ("PATH", "/usr/bin")]
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn exe_and_msi_plans() {
        let w = wine(true);
        let fs = FakeFs::with(&[
            ("/usr/bin/wine", b""),
            ("/home/ana/Downloads/app.exe", b"MZ"),
            ("/home/ana/Downloads/tool.msi", b""),
            ("/opt/dxvk/x64/dxgi.dll", b""),
        ]);
        let incoming = incoming();
        let home = Path::new("/home/ana");
        let p = plan(
            Path::new("/home/ana/Downloads/app.exe"),
            &["-x".into()],
            &incoming,
            &w,
            home,
            &fs,
        )
        .unwrap();
        let l = &p.launch;
        assert_eq!(
            l.argv,
            ["/usr/bin/wine", "/home/ana/Downloads/app.exe", "-x"]
        );
        assert_eq!(l.cwd.as_deref(), Some("/home/ana/Downloads"));
        let id = exe_id(Path::new("/home/ana/Downloads/app.exe"));
        assert_eq!(p.prefix, home.join(PREFIXES).join(&id).join("pfx"));
        assert!(p.ensure_prefix);
        assert_eq!(l.env["WINEDLLOVERRIDES"], "dxgi=n,b");
        assert_eq!(l.env["MESA_VK_WSI_PRESENT_MODE"], "mailbox");
        assert!(!l.env.contains_key("WINEESYNC"));
        assert!(!l.env.contains_key("GAMEID"), "umu only");
        assert_eq!(p.copies.len(), 1);
        let m = plan(
            Path::new("/home/ana/Downloads/tool.msi"),
            &[],
            &incoming,
            &w,
            home,
            &fs,
        )
        .unwrap();
        assert_eq!(
            &m.launch.argv[1..],
            ["msiexec", "/i", "/home/ana/Downloads/tool.msi"]
        );
        for bad in ["relative.exe", "/home/ana/missing.exe"] {
            assert_eq!(
                plan(Path::new(bad), &[], &incoming, &w, home, &fs)
                    .unwrap_err()
                    .code,
                ErrorCode::NotFound
            );
        }
    }

    #[test]
    fn umu_runs_the_program_when_wine_is_missing() {
        let fs = FakeFs::with(&[
            ("/usr/bin/umu-run", b""),
            ("/home/ana/g/app.exe", b"MZ"),
            ("/opt/dxvk/x64/dxgi.dll", b""),
        ]);
        let p = plan(
            Path::new("/home/ana/g/app.exe"),
            &[],
            &incoming(),
            &wine(true),
            Path::new("/home/ana"),
            &fs,
        )
        .unwrap();
        assert_eq!(p.launch.argv, ["/usr/bin/umu-run", "/home/ana/g/app.exe"]);
        assert_eq!(p.launch.env["GAMEID"], "umu-default");
        assert!(p.launch.env["WINEPREFIX"].ends_with("/pfx"));
        assert!(
            !p.ensure_prefix && p.copies.is_empty(),
            "Proton owns the prefix and DLLs"
        );

        let err = plan(
            Path::new("/home/ana/g/app.exe"),
            &[],
            &incoming(),
            &wine(false),
            Path::new("/home/ana"),
            &FakeFs::with(&[("/home/ana/g/app.exe", b"MZ")]),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(
            err.message.contains("sudo pacman -S wine"),
            "{}",
            err.message
        );
    }
}

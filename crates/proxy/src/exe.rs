//! `wol-run --exe <file.exe|file.msi> [args]`: double-click a Windows program. Each program gets
//! its own prefix, named by its path; it runs on native wine-proton from its own directory.
use crate::wine::{EnvInput, Fs, Wine, WinePlan, pick_dlls, proton_dirs, wine_env};
use crate::{LaunchPlan, PREFIXES, dlls, steam, tuning};
use shared::errors::{ErrorCode, WolError};
use std::collections::BTreeMap;
use std::path::Path;

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
    let loader = wine
        .loader(home, (!msi).then_some(path), fs)
        .ok_or_else(|| {
            WolError::new(
                ErrorCode::NotFound,
                "wine-proton is not installed: sudo pkg install -y wine-proton",
            )
        })?;
    let prefix = home.join(PREFIXES).join(exe_id(path)).join("pfx");
    let dirs = proton_dirs(
        &steam::steam_libraries(&BTreeMap::new(), home, fs),
        wine.version,
        fs,
    );
    let sources = pick_dlls(incoming, wine.vendor, &dirs, fs);
    let (copies, names) = dlls::plan(&sources, &|d| fs.list(d), &prefix);
    let defaults = tuning::defaults(wine.vendor, wine.version);
    let env = wine_env(&EnvInput {
        incoming,
        loader: &loader,
        prefix: &prefix,
        defaults: &defaults,
        dlls: &names,
        steam: false,
    });
    Ok(WinePlan {
        summary: format!(
            "wol-run: {} in {} ({} DLLs native-first)",
            path.display(),
            prefix.display(),
            names.len()
        ),
        launch: Some(LaunchPlan {
            argv: std::iter::once(loader.to_string_lossy().into_owned())
                .chain(tail)
                .collect(),
            env,
            env_clear: true,
            cwd: path.parent().map(|d| d.to_string_lossy().into_owned()),
            relay_seekable_stdio: true,
            ..Default::default()
        }),
        loader,
        prefix,
        ensure_prefix: true,
        copies,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn exe_and_msi_plans() {
        let w = Wine {
            root: "/usr/local/wine-proton".into(),
            forced: None,
            version: Version::parse("wine-11.0"),
            vendor: Vendor::Amd,
        };
        let fs = FakeFs::with(&[
            ("/usr/local/wine-proton/bin/wine", b""),
            ("/home/ana/Downloads/app.exe", b"MZ"),
            ("/home/ana/Downloads/tool.msi", b""),
            ("/opt/dxvk/x64/dxgi.dll", b""),
        ]);
        let incoming: BTreeMap<String, String> =
            [("WOL_DXVK_DIR", "/opt/dxvk"), ("PATH", "/usr/bin")]
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect();
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
        let l = p.launch.as_ref().unwrap();
        assert_eq!(
            l.argv,
            [
                "/usr/local/wine-proton/bin/wine",
                "/home/ana/Downloads/app.exe",
                "-x"
            ]
        );
        assert_eq!(l.cwd.as_deref(), Some("/home/ana/Downloads"));
        let id = exe_id(Path::new("/home/ana/Downloads/app.exe"));
        assert_eq!(p.prefix, home.join(PREFIXES).join(&id).join("pfx"));
        assert_eq!(l.env["WINEDLLOVERRIDES"], "dxgi=n,b");
        assert_eq!(l.env["MESA_VK_WSI_PRESENT_MODE"], "mailbox");
        assert!(
            !l.env.contains_key("WINEESYNC"),
            "wine-proton 11 has no esync"
        );
        assert!(!l.env.contains_key("DISABLE_VK_LAYER_VALVE_steam_overlay_1"));
        assert_eq!(p.copies.len(), 1);
        assert!(!p.wait_server);
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
            &m.launch.unwrap().argv[1..],
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
}

//! `wol-run --steam <verb> [args]`: backend of the WolOS Proton compatibility tool. Same verbs as
//! Valve's Proton script; the game runs on native wine-proton with the Steam prefix
//! (`$STEAM_COMPAT_DATA_PATH/pfx`).
use crate::wine::{EnvInput, Fs, Wine, WinePlan, pick_dlls, proton_dirs, wine_env};
use crate::{LaunchPlan, dlls, tuning};
use shared::errors::{ErrorCode, WolError};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wol_core::games::steam as tool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// Wait for the prefix's previous wineserver, then run (game launch).
    WaitForExitAndRun,
    /// Run now (install scripts, first-run setup).
    Run,
    /// Run without prefix changes (no DLL install).
    RunInPrefix,
    /// Unix path -> Windows path on stdout.
    GetCompatPath,
    /// Windows path -> Unix path on stdout.
    GetNativePath,
    /// Delete the prefix.
    DestroyPrefix,
}

impl Verb {
    pub fn parse(s: &str) -> Option<Self> {
        use Verb::*;
        let all = [
            WaitForExitAndRun,
            Run,
            RunInPrefix,
            GetCompatPath,
            GetNativePath,
            DestroyPrefix,
        ];
        tool::VERBS.iter().position(|v| *v == s).map(|i| all[i])
    }
}

fn usage(msg: impl Into<String>) -> WolError {
    WolError::new(ErrorCode::Usage, msg)
}

/// Steam libraries for DXVK lookup: the client's, the game's (`<lib>/steamapps/compatdata/<id>`)
/// and every `libraryfolders.vdf` entry.
pub fn steam_libraries(
    incoming: &BTreeMap<String, String>,
    home: &Path,
    fs: &dyn Fs,
) -> Vec<PathBuf> {
    let root = incoming
        .get("STEAM_COMPAT_CLIENT_INSTALL_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| tool::steam_root(home, &|p| fs.exists(p)));
    let mut libs = tool::libraries(&root, &|p| fs.read(p));
    let game_lib = incoming
        .get("STEAM_COMPAT_DATA_PATH")
        .and_then(|d| Path::new(d).ancestors().nth(3).map(Path::to_path_buf));
    if let Some(l) = game_lib.filter(|l| !libs.contains(l)) {
        libs.push(l);
    }
    libs
}

/// Plan one verb. `args`: what Steam passes after the verb (program and its arguments, or a
/// path for getcompatpath/getnativepath).
pub fn plan(
    verb: &str,
    args: &[String],
    incoming: &BTreeMap<String, String>,
    wine: &Wine,
    home: &Path,
    fs: &dyn Fs,
) -> Result<WinePlan, WolError> {
    let v = Verb::parse(verb).ok_or_else(|| {
        usage(format!(
            "unknown verb '{verb}' (one of {})",
            tool::VERBS.join(", ")
        ))
    })?;
    let data = incoming
        .get("STEAM_COMPAT_DATA_PATH")
        .filter(|d| d.starts_with('/') && d.len() > 1)
        .ok_or_else(|| usage("STEAM_COMPAT_DATA_PATH is not set: WolOS Proton runs from Steam"))?;
    let prefix = Path::new(data).join("pfx");
    if v == Verb::DestroyPrefix {
        return Ok(WinePlan {
            summary: format!("wolos-proton: destroyprefix {}", prefix.display()),
            prefix: prefix.clone(),
            remove: Some(prefix),
            ..Default::default()
        });
    }
    let first = args
        .first()
        .ok_or_else(|| usage(format!("{verb}: missing program or path")))?;
    let (tail, program): (Vec<String>, Option<&Path>) = match v {
        Verb::GetCompatPath => (vec!["winepath".into(), "-w".into(), first.clone()], None),
        Verb::GetNativePath => (vec!["winepath".into(), "-u".into(), first.clone()], None),
        _ => (args.to_vec(), Some(Path::new(first))),
    };
    let loader = wine.loader(home, program, fs).ok_or_else(|| {
        WolError::new(
            ErrorCode::NotFound,
            "wine-proton is not installed: sudo pkg install -y wine-proton",
        )
    })?;
    let sources = if matches!(v, Verb::Run | Verb::WaitForExitAndRun) {
        let dirs = proton_dirs(&steam_libraries(incoming, home, fs), wine.version, fs);
        pick_dlls(incoming, wine.vendor, &dirs, fs)
    } else {
        vec![]
    };
    let (copies, names) = dlls::plan(&sources, &|d| fs.list(d), &prefix);
    let defaults = tuning::defaults(wine.vendor, wine.version);
    let env = wine_env(&EnvInput {
        incoming,
        loader: &loader,
        prefix: &prefix,
        defaults: &defaults,
        dlls: &names,
        steam: true,
    });
    let log = crate::wine::flag(incoming, "PROTON_LOG").then(|| {
        let id = ["SteamGameId", "SteamAppId"]
            .iter()
            .find_map(|k| incoming.get(*k))
            .map_or("0", String::as_str);
        let dir = incoming
            .get("PROTON_LOG_DIR")
            .map_or_else(|| home.to_path_buf(), PathBuf::from);
        dir.join(format!("steam-{id}.log"))
    });
    let origins: Vec<String> = sources
        .iter()
        .map(|s| format!("{} {}", s.component.proton_dir(), s.origin))
        .collect();
    let summary = format!(
        "wolos-proton: {verb} {} (wine-proton {}, gpu {}, {})",
        tail.first().map_or("", String::as_str),
        wine.version.map_or("?".into(), |x| x.to_string()),
        wine.vendor.name(),
        if origins.is_empty() {
            "builtin d3d".into()
        } else {
            origins.join(", ")
        }
    );
    Ok(WinePlan {
        launch: Some(LaunchPlan {
            argv: std::iter::once(loader.to_string_lossy().into_owned())
                .chain(tail)
                .collect(),
            env,
            env_clear: true,
            relay_seekable_stdio: true,
            ..Default::default()
        }),
        loader,
        prefix,
        ensure_prefix: true,
        copies,
        wait_server: v == Verb::WaitForExitAndRun,
        remove: None,
        log,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::Vendor;
    use crate::wine::fake::FakeFs;
    use wol_core::games::wine::Version;

    const DATA: &str = "/lib1/steamapps/compatdata/42";

    fn wine() -> Wine {
        Wine {
            root: "/usr/local/wine-proton".into(),
            forced: None,
            version: Version::parse("wine-10.0"),
            vendor: Vendor::Intel,
        }
    }

    fn pe32() -> Vec<u8> {
        let mut b = vec![0u8; 0x100];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c] = 0x80;
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x14cu16.to_le_bytes());
        b
    }

    fn fs() -> FakeFs {
        let proton = "/lib1/steamapps/common/Proton 10.0/files/lib/wine/dxvk";
        FakeFs::with(&[
            ("/usr/local/wine-proton/bin/wine", b""),
            ("/usr/local/wine-proton/bin/wine64", b""),
            ("/usr/local/wine-proton/bin-wow64/wine", b""),
            ("/games/Game/game32.exe", &pe32()),
            (&format!("{proton}/x86_64-windows/d3d11.dll"), b"x"),
            (&format!("{proton}/i386-windows/d3d11.dll"), b"x"),
        ])
    }

    fn env() -> BTreeMap<String, String> {
        [
            ("STEAM_COMPAT_DATA_PATH", DATA),
            ("STEAM_COMPAT_CLIENT_INSTALL_PATH", "/home/ana/.steam/steam"),
            ("SteamGameId", "42"),
        ]
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).into()).collect()
    }

    /// (verb, args, argv after the loader's path, wait for wineserver, DLL copies).
    type Case = (
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
        bool,
        usize,
    );

    #[test]
    fn verb_dispatch_table() {
        let home = Path::new("/home/ana");
        let cases: [Case; 5] = [
            (
                "waitforexitandrun",
                &["/games/Game/game.exe", "-dx11"],
                &["/games/Game/game.exe", "-dx11"],
                true,
                2,
            ),
            (
                "run",
                &["/games/Game/setup.exe"],
                &["/games/Game/setup.exe"],
                false,
                2,
            ),
            (
                "runinprefix",
                &["/games/Game/tool.exe"],
                &["/games/Game/tool.exe"],
                false,
                0,
            ),
            (
                "getcompatpath",
                &["/games/Game"],
                &["winepath", "-w", "/games/Game"],
                false,
                0,
            ),
            (
                "getnativepath",
                &["C:\\users"],
                &["winepath", "-u", "C:\\users"],
                false,
                0,
            ),
        ];
        for (verb, args, tail, wait, ncopies) in cases {
            let p = plan(verb, &s(args), &env(), &wine(), home, &fs()).unwrap();
            let l = p.launch.as_ref().unwrap();
            assert_eq!(l.argv[0], "/usr/local/wine-proton/bin/wine64", "{verb}");
            assert_eq!(&l.argv[1..], tail, "{verb}");
            assert_eq!(p.wait_server, wait, "{verb}");
            assert_eq!(p.copies.len(), ncopies, "{verb}");
            assert!(p.ensure_prefix && l.env_clear && l.relay_seekable_stdio);
            assert_eq!(p.prefix, Path::new(DATA).join("pfx"));
            assert_eq!(l.env["WINEPREFIX"], format!("{DATA}/pfx"));
            assert!(l.modules.is_empty(), "no Linux ABI for the game");
        }
        let d = plan("destroyprefix", &[], &env(), &wine(), home, &fs()).unwrap();
        assert_eq!(d.remove, Some(Path::new(DATA).join("pfx")));
        assert!(d.launch.is_none());
    }

    #[test]
    fn errors_and_32_bit_loader() {
        let home = Path::new("/home/ana");
        let code = |verb: &str, args: &[&str], e: BTreeMap<String, String>, f: &FakeFs| {
            plan(verb, &s(args), &e, &wine(), home, f).unwrap_err().code
        };
        assert_eq!(code("bogus", &["x"], env(), &fs()), ErrorCode::Usage);
        assert_eq!(code("run", &[], env(), &fs()), ErrorCode::Usage);
        assert_eq!(
            code("run", &["x.exe"], BTreeMap::new(), &fs()),
            ErrorCode::Usage
        );
        let relative = [("STEAM_COMPAT_DATA_PATH", "compat")]
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        assert_eq!(code("run", &["x.exe"], relative, &fs()), ErrorCode::Usage);
        assert_eq!(
            code("run", &["x.exe"], env(), &FakeFs::default()),
            ErrorCode::NotFound
        );
        let p = plan(
            "run",
            &s(&["/games/Game/game32.exe"]),
            &env(),
            &wine(),
            home,
            &fs(),
        )
        .unwrap();
        assert_eq!(
            p.launch.unwrap().argv[0],
            "/usr/local/wine-proton/bin-wow64/wine",
            "wine-proton 10 runs 32-bit games with the WoW64 loader"
        );
        let mut forced = wine();
        forced.forced = Some("/opt/wine/bin/wine".into());
        let p = plan(
            "run",
            &s(&["/x.exe"]),
            &env(),
            &forced,
            home,
            &FakeFs::default(),
        )
        .unwrap();
        assert_eq!(p.loader, Path::new("/opt/wine/bin/wine"));
    }

    #[test]
    fn proton_log_goes_to_home() {
        let mut e = env();
        e.insert("PROTON_LOG".into(), "1".into());
        let p = plan(
            "run",
            &s(&["/x.exe"]),
            &e,
            &wine(),
            Path::new("/home/ana"),
            &fs(),
        )
        .unwrap();
        assert_eq!(p.log, Some(PathBuf::from("/home/ana/steam-42.log")));
        assert!(p.summary.contains("dxvk Proton 10.0"), "{}", p.summary);
    }

    #[test]
    fn libraries_include_the_games_own() {
        let libs = steam_libraries(&env(), Path::new("/home/ana"), &FakeFs::default());
        assert_eq!(
            libs,
            [
                PathBuf::from("/home/ana/.steam/steam"),
                PathBuf::from("/lib1")
            ]
        );
    }
}

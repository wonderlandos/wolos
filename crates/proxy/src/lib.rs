//! wol-run: launcher. `.wol` bundle -> resolved target -> launch plan (argv, env); Windows
//! programs on native wine or umu-launcher ([`exe`]). Steam games do not come here: Steam runs
//! its own Proton.
pub mod dlls;
pub mod exe;
pub mod tuning;
pub mod wine;

use shared::bundle::{Abi, Target};
use std::collections::BTreeMap;

/// Under `$HOME`: one Proton prefix per bundle id.
pub const PREFIXES: &str = ".local/share/wolos/prefixes";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchPlan {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// `env` is the whole environment (Steam's Linux-side variables are gone).
    pub env_clear: bool,
    pub cwd: Option<String>,
    /// Proton closes std handles that are regular files (ntdll "handle for fd N is seekable"), so
    /// seekable stdout/stderr go through a pipe that wol-run copies into the original file.
    pub relay_seekable_stdio: bool,
}

/// The runner for a bundle's Proton target: loader and its default variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proton {
    pub loader: String,
    pub defaults: Vec<(&'static str, &'static str)>,
}

pub fn plan(
    bundle_dir: &str,
    target: &Target,
    app_id: &str,
    home: &str,
    args: &[String],
    proton: &Proton,
) -> LaunchPlan {
    let dir = bundle_dir.trim_end_matches('/');
    let entry = format!("{dir}/{}", target.entry);
    let mut env = BTreeMap::new();
    let argv = match target.abi {
        Abi::FreeBsdNative => vec![entry],
        Abi::Linux => {
            // A Linux ELF runs natively; bundled libs first.
            env.insert("LD_LIBRARY_PATH".into(), format!("{dir}/lib"));
            vec![entry]
        }
        Abi::Proton => {
            let data = format!("{}/{PREFIXES}/{app_id}", home.trim_end_matches('/'));
            env.extend(
                proton
                    .defaults
                    .iter()
                    .map(|(k, v)| ((*k).into(), (*v).into())),
            );
            env.insert("WINEPREFIX".into(), format!("{data}/pfx"));
            vec![proton.loader.clone(), entry]
        }
    };
    LaunchPlan {
        relay_seekable_stdio: target.abi == Abi::Proton,
        argv: argv.into_iter().chain(args.iter().cloned()).collect(),
        env,
        ..Default::default()
    }
}

/// Bundle ids name prefix directories: reverse-DNS characters only.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && !id.contains("..")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// What this machine can run: Linux programs always, Windows ones when wine or umu is there.
pub fn host_caps(wine: &wine::Wine) -> wol_core::bundle::HostCaps {
    wol_core::bundle::HostCaps {
        arch: wol_core::bundle::host_arch().into(),
        linux_compat: true,
        proton: wine.runner(&wine::RealFs).is_some(),
    }
}

/// Read `<dir>/Info.json`, resolve, plan. Errors carry a stable code.
pub fn prepare(
    dir: &str,
    host: &wol_core::bundle::HostCaps,
    wine: &wine::Wine,
    home: &str,
    args: &[String],
) -> Result<LaunchPlan, shared::errors::WolError> {
    use shared::errors::{ErrorCode, WolError};
    let path = std::path::Path::new(dir).join(shared::bundle::INFO_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| WolError::new(ErrorCode::NotFound, format!("{}: {e}", path.display())))?;
    let info =
        shared::bundle::Info::parse(&text).map_err(|e| WolError::new(ErrorCode::Invalid, e))?;
    if !valid_id(&info.id) {
        return Err(WolError::new(
            ErrorCode::Invalid,
            format!("bad bundle id {}", info.id),
        ));
    }
    // native > linux > proton among the targets whose entry ships: a missing entry falls through.
    let mut info = info;
    let declared = info.targets.len();
    info.targets
        .retain(|t| std::path::Path::new(dir).join(&t.entry).is_file());
    if declared > 0 && info.targets.is_empty() {
        return Err(WolError::new(
            ErrorCode::NotFound,
            format!("{}: no target entry exists in {dir}", info.id),
        ));
    }
    let target = wol_core::bundle::resolve(&info, host).ok_or_else(|| {
        WolError::new(
            ErrorCode::Unsupported,
            format!("{}: no target runs on this machine", info.id),
        )
    })?;
    let runner = wine.runner(&wine::RealFs);
    let proton = Proton {
        loader: runner
            .as_ref()
            .map_or(wine.bin.as_path(), wine::Runner::path)
            .to_string_lossy()
            .into_owned(),
        defaults: runner.map_or_else(
            || tuning::defaults(wine.vendor),
            |r| r.defaults(wine.vendor),
        ),
    };
    Ok(plan(dir, target, &info.id, home, args, &proton))
}

fn seekable(fd: &std::os::fd::OwnedFd) -> bool {
    use std::os::fd::AsRawFd;
    // SAFETY: lseek on an owned, open descriptor; SEEK_CUR with offset 0 does not move it.
    unsafe { libc::lseek(fd.as_raw_fd(), 0, libc::SEEK_CUR) != -1 }
}

/// Spawn `plan` with `stdout`/`stderr`, relaying seekable ones through pipes when the plan asks.
/// Returns the exit code (128 + signal when killed).
pub fn run(
    plan: &LaunchPlan,
    stdout: std::os::fd::OwnedFd,
    stderr: std::os::fd::OwnedFd,
) -> std::io::Result<i32> {
    use std::process::Stdio;
    let relay_out = plan.relay_seekable_stdio && seekable(&stdout);
    let relay_err = plan.relay_seekable_stdio && seekable(&stderr);
    let pick = |fd: &std::os::fd::OwnedFd, relay: bool| -> std::io::Result<Stdio> {
        Ok(if relay {
            Stdio::piped()
        } else {
            Stdio::from(fd.try_clone()?)
        })
    };
    let mut cmd = std::process::Command::new(&plan.argv[0]);
    if plan.env_clear {
        cmd.env_clear();
    }
    if let Some(d) = &plan.cwd {
        cmd.current_dir(d);
    }
    let mut child = cmd
        .args(&plan.argv[1..])
        .envs(&plan.env)
        .stdout(pick(&stdout, relay_out)?)
        .stderr(pick(&stderr, relay_err)?)
        .spawn()?;
    fn copy(
        r: Option<impl std::io::Read + Send + 'static>,
        fd: std::os::fd::OwnedFd,
    ) -> Option<std::thread::JoinHandle<()>> {
        let mut r = r?;
        let mut w = std::fs::File::from(fd);
        Some(std::thread::spawn(move || {
            let _ = std::io::copy(&mut r, &mut w);
        }))
    }
    let threads = [
        copy(child.stdout.take(), stdout),
        copy(child.stderr.take(), stderr),
    ];
    let status = child.wait()?;
    threads.into_iter().flatten().for_each(|t| {
        let _ = t.join();
    });
    use std::os::unix::process::ExitStatusExt;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(abi: Abi) -> Target {
        Target {
            abi,
            arch: "amd64".into(),
            entry: "bin/app".into(),
        }
    }

    fn proton() -> Proton {
        Proton {
            loader: "/usr/bin/wine".into(),
            defaults: tuning::defaults(tuning::Vendor::Intel),
        }
    }

    fn test_wine() -> wine::Wine {
        wine::Wine {
            bin: "/usr/bin/wine".into(),
            umu: None,
            forced: false,
            version: None,
            vendor: tuning::Vendor::Other,
        }
    }

    #[test]
    fn plan_table() {
        let args = vec!["--x".to_string()];
        let native = plan(
            "/Apps/A.wol",
            &t(Abi::FreeBsdNative),
            "org.a",
            "/home/u",
            &args,
            &proton(),
        );
        assert_eq!(native.argv, ["/Apps/A.wol/bin/app", "--x"]);
        assert!(native.env.is_empty());

        let linux = plan(
            "/Apps/A.wol",
            &t(Abi::Linux),
            "org.a",
            "/home/u",
            &[],
            &proton(),
        );
        assert_eq!(linux.argv, ["/Apps/A.wol/bin/app"]);
        assert_eq!(
            linux.env.get("LD_LIBRARY_PATH").map(String::as_str),
            Some("/Apps/A.wol/lib")
        );

        let p = plan(
            "/Apps/A.wol/",
            &t(Abi::Proton),
            "org.a",
            "/home/u",
            &args,
            &proton(),
        );
        assert_eq!(p.argv, ["/usr/bin/wine", "/Apps/A.wol/bin/app", "--x"]);
        for (k, v) in proton().defaults {
            assert_eq!(p.env.get(k).map(String::as_str), Some(v), "{k}");
        }
        let data = "/home/u/.local/share/wolos/prefixes/org.a";
        assert_eq!(p.env["WINEPREFIX"], format!("{data}/pfx"));
    }

    #[test]
    fn ids() {
        for (id, ok) in [
            ("org.a.App", true),
            ("a-b_c", true),
            ("", false),
            ("../x", false),
            ("a/b", false),
            (".hidden", false),
        ] {
            assert_eq!(valid_id(id), ok, "{id}");
        }
    }

    #[test]
    fn prepare_resolves_best_target() {
        use shared::errors::ErrorCode;
        use wol_core::bundle::HostCaps;
        let d = std::env::temp_dir().join(format!("wol-run-{}.wol", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let dir = d.to_str().unwrap();
        std::fs::write(
            d.join("Info.json"),
            r#"{"id":"org.a","version":"1","targets":[{"abi":"proton","arch":"amd64","entry":"a.exe"},{"abi":"linux","arch":"amd64","entry":"a"}]}"#,
        )
        .unwrap();
        std::fs::write(d.join("a"), "").unwrap();
        std::fs::write(d.join("a.exe"), "").unwrap();
        let host = |linux, proton| HostCaps {
            arch: "amd64".into(),
            linux_compat: linux,
            proton,
        };
        // (host, chosen entry)
        let cases = [
            (host(true, true), Ok("a")),
            (host(true, false), Ok("a")),
            (host(false, true), Ok("a.exe")),
            (host(false, false), Err(ErrorCode::Unsupported)),
        ];
        for (h, want) in cases {
            let got = prepare(dir, &h, &test_wine(), "/home/u", &[])
                .map(|p| {
                    let entry = p.argv.last().unwrap().rsplit('/').next().unwrap();
                    entry.to_owned()
                })
                .map_err(|e| e.code);
            let want = want.map(str::to_owned);
            assert_eq!(got, want, "{h:?}");
        }
        assert_eq!(
            prepare(
                "/nonexistent.wol",
                &host(true, true),
                &test_wine(),
                "/h",
                &[]
            )
            .unwrap_err()
            .code,
            ErrorCode::NotFound
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn prepare_falls_back_when_the_preferred_entry_is_missing() {
        use shared::errors::ErrorCode;
        use wol_core::bundle::HostCaps;
        let d = std::env::temp_dir().join(format!("wol-run-fb-{}.wol", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let dir = d.to_str().unwrap();
        std::fs::write(
            d.join("Info.json"),
            r#"{"id":"org.b","version":"1","targets":[{"abi":"linux","arch":"amd64","entry":"bin/b"},{"abi":"proton","arch":"amd64","entry":"b.exe"}]}"#,
        )
        .unwrap();
        let host = HostCaps {
            arch: "amd64".into(),
            linux_compat: true,
            proton: true,
        };
        let entry = |p: Result<LaunchPlan, shared::errors::WolError>| {
            p.map(|p| {
                p.argv
                    .last()
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .map_err(|e| e.code)
        };
        // (files present, chosen entry)
        let cases: [(&[&str], Result<&str, ErrorCode>); 3] = [
            (&["bin/b", "b.exe"], Ok("b")),
            (&["b.exe"], Ok("b.exe")),
            (&[], Err(ErrorCode::NotFound)),
        ];
        for (files, want) in cases {
            let _ = std::fs::remove_dir_all(d.join("bin"));
            let _ = std::fs::remove_file(d.join("b.exe"));
            for f in files {
                let p = d.join(f);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, "").unwrap();
            }
            assert_eq!(
                entry(prepare(dir, &host, &test_wine(), "/h", &[])),
                want.map(str::to_owned),
                "{files:?}"
            );
        }
        std::fs::remove_dir_all(d).unwrap();
    }
}

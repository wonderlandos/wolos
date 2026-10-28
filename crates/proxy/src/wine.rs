//! Windows programs outside Steam (`--exe`, bundles with a Proton target): the facts wol-run
//! reads, the environment it builds, the plan (prefix, DLLs, argv) and its executor. The runner
//! is the native `wine` package or, when wine is not installed, umu-launcher.
use crate::{LaunchPlan, dlls, tuning::Vendor};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wol_core::games::wine::{self as w, Version};

/// File access the planners need (real: [`RealFs`]; tests: a map).
pub trait Fs {
    fn exists(&self, p: &Path) -> bool;
    /// Entries of a directory, sorted; empty if missing.
    fn list(&self, p: &Path) -> Vec<PathBuf>;
    fn read(&self, p: &Path) -> Option<String>;
}

pub struct RealFs;

impl Fs for RealFs {
    fn exists(&self, p: &Path) -> bool {
        p.exists()
    }
    fn list(&self, p: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
    fn read(&self, p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }
}

/// What runs a Windows program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Runner {
    /// wine (WoW64): wol-run makes the prefix and copies DXVK/vkd3d-proton into it.
    Wine(PathBuf),
    /// umu-launcher: Proton brings its own DXVK and makes the prefix.
    Umu(PathBuf),
}

impl Runner {
    pub fn path(&self) -> &Path {
        match self {
            Runner::Wine(p) | Runner::Umu(p) => p,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Runner::Wine(_) => "wine",
            Runner::Umu(_) => "umu",
        }
    }
    /// Variables the runner needs on top of the vendor defaults.
    pub fn defaults(&self, vendor: Vendor) -> Vec<(&'static str, &'static str)> {
        let mut d = crate::tuning::defaults(vendor);
        if matches!(self, Runner::Umu(_)) {
            d.push(("GAMEID", w::UMU_GAME_ID));
        }
        d
    }
}

/// This machine's wine, as wol-run sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wine {
    pub bin: PathBuf,
    /// umu-launcher, unless `WOL_WINE` picks one loader.
    pub umu: Option<PathBuf>,
    /// `WOL_WINE`: one loader for every program (a custom build).
    pub forced: bool,
    pub version: Option<Version>,
    pub vendor: Vendor,
}

impl Wine {
    /// Wine when installed (works offline), else umu-launcher.
    pub fn runner(&self, fs: &dyn Fs) -> Option<Runner> {
        if fs.exists(&self.bin) {
            return Some(Runner::Wine(self.bin.clone()));
        }
        self.umu
            .as_ref()
            .filter(|u| fs.exists(u))
            .map(|u| Runner::Umu(u.clone()))
    }

    /// Real host: `WOL_WINE`, `wine --version`, `WOL_GPU_PRESET` or the hardware snapshot.
    pub fn detect(var: &dyn Fn(&str) -> Option<String>) -> Self {
        let forced = var("WOL_WINE").map(PathBuf::from);
        let bin = forced.clone().unwrap_or_else(|| PathBuf::from(w::BIN));
        let version = bin
            .exists()
            .then(|| {
                wol_core::system::run_with_timeout(
                    &[&bin.to_string_lossy(), "--version"],
                    std::time::Duration::from_secs(5),
                )
            })
            .flatten()
            .and_then(|t| Version::parse(&t));
        let vendor = var("WOL_GPU_PRESET")
            .and_then(|v| Vendor::parse(&v))
            .unwrap_or_else(|| {
                let names: Vec<&str> = std::fs::read_to_string(shared::hardware::SNAPSHOT_PATH)
                    .ok()
                    .and_then(|t| {
                        serde_json::from_str::<shared::hardware::HardwareSnapshot>(&t).ok()
                    })
                    .map(|s| {
                        s.devices
                            .iter()
                            .filter(|d| d.kind == shared::hardware::Kind::Gpu)
                            .filter_map(|d| wol_core::games::doctor::vendor_name(&d.id))
                            .collect()
                    })
                    .unwrap_or_default();
                crate::tuning::game_vendor(&names)
            });
        Wine {
            bin,
            umu: forced.is_none().then(|| PathBuf::from(w::UMU_BIN)),
            forced: forced.is_some(),
            version,
            vendor,
        }
    }
}

pub fn flag(env: &BTreeMap<String, String>, k: &str) -> bool {
    env.get(k).is_some_and(|v| v == "1")
}

/// Inputs of [`wine_env`].
pub struct EnvInput<'a> {
    pub incoming: &'a BTreeMap<String, String>,
    pub prefix: &'a Path,
    pub defaults: &'a [(&'static str, &'static str)],
    /// DLLs installed native-first.
    pub dlls: &'a [String],
}

/// Environment for the runner: the incoming one, the prefix, DLL overrides, then defaults where
/// unset (a variable already in the environment wins).
pub fn wine_env(i: &EnvInput) -> BTreeMap<String, String> {
    let mut e = i.incoming.clone();
    e.insert("WINEPREFIX".into(), i.prefix.to_string_lossy().into_owned());
    match crate::dlls::overrides(
        i.dlls,
        i.incoming.get("WINEDLLOVERRIDES").map(String::as_str),
    ) {
        Some(o) => e.insert("WINEDLLOVERRIDES".into(), o),
        None => e.remove("WINEDLLOVERRIDES"),
    };
    if i.dlls.iter().any(|d| d.starts_with("nvapi")) {
        e.entry("DXVK_ENABLE_NVAPI".into())
            .or_insert_with(|| "1".into());
    }
    for (k, v) in i.defaults {
        e.entry((*k).into()).or_insert_with(|| (*v).into());
    }
    e
}

/// Which DLL components a launch wants (Proton's switches).
pub fn wanted(c: dlls::Component, incoming: &BTreeMap<String, String>, vendor: Vendor) -> bool {
    use dlls::Component::*;
    let wined3d = flag(incoming, "PROTON_USE_WINED3D");
    match c {
        Dxvk => !wined3d,
        Vkd3d => !flag(incoming, "PROTON_NO_D3D12"),
        Nvapi => {
            !wined3d
                && incoming.get("PROTON_ENABLE_NVAPI").map(String::as_str) != Some("0")
                && (vendor == Vendor::Nvidia || flag(incoming, "PROTON_ENABLE_NVAPI"))
        }
    }
}

/// DLL sources for a launch: found ones, filtered by [`wanted`]; NVAPI only next to DXVK.
pub fn pick_dlls(
    incoming: &BTreeMap<String, String>,
    vendor: Vendor,
    proton_dirs: &[PathBuf],
    fs: &dyn Fs,
) -> Vec<dlls::Source> {
    let var = |k: &str| incoming.get(k).cloned();
    let found: Vec<dlls::Source> = dlls::find(&var, proton_dirs, &|p| fs.exists(p))
        .into_iter()
        .filter(|s| wanted(s.component, incoming, vendor))
        .collect();
    let dxvk = found.iter().any(|s| s.component == dlls::Component::Dxvk);
    found
        .into_iter()
        .filter(|s| dxvk || s.component != dlls::Component::Nvapi)
        .collect()
}

/// Valve Proton installs in the Steam libraries that carry DLLs, best first.
pub fn proton_dirs(libraries: &[PathBuf], fs: &dyn Fs) -> Vec<PathBuf> {
    let mut out = vec![];
    for lib in libraries {
        let common = lib.join("steamapps/common");
        let names: Vec<String> = fs
            .list(&common)
            .iter()
            .filter_map(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
            .collect();
        out.extend(
            wol_core::games::steam::proton_order(&names)
                .into_iter()
                .map(|n| common.join(n))
                .filter(|d| fs.exists(&d.join("files"))),
        );
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WinePlan {
    pub launch: LaunchPlan,
    pub runner: Option<Runner>,
    pub prefix: PathBuf,
    /// Create the prefix (`wine wineboot -u`) when it has no `system.reg`.
    pub ensure_prefix: bool,
    pub copies: Vec<(PathBuf, PathBuf)>,
    /// One line for stderr.
    pub summary: String,
}

/// Run a plan: prefix, DLLs, program. Helper steps write to `stderr` only, so stdout carries just
/// the program's output. Returns the program's exit code.
pub fn execute(
    p: &WinePlan,
    stdout: std::os::fd::OwnedFd,
    stderr: std::os::fd::OwnedFd,
) -> std::io::Result<i32> {
    let launch = &p.launch;
    if let (true, Some(Runner::Wine(wine))) = (p.ensure_prefix, &p.runner)
        && !p.prefix.join("system.reg").exists()
    {
        if let Some(parent) = p.prefix.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // `-u` returns once the prefix is written.
        let mut c = std::process::Command::new(wine);
        c.args(["wineboot", "-u"])
            .env_clear()
            .envs(&launch.env)
            .stdin(std::process::Stdio::null())
            .stdout(stderr.try_clone()?)
            .stderr(stderr.try_clone()?);
        if let Some(d) = &launch.cwd {
            c.current_dir(d);
        }
        c.status()?;
    }
    crate::dlls::copy_changed(&p.copies)?;
    crate::run(launch, stdout, stderr)
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// Paths that exist (files with contents).
    #[derive(Default)]
    pub struct FakeFs(pub BTreeMap<PathBuf, Vec<u8>>);

    impl FakeFs {
        pub fn with(files: &[(&str, &[u8])]) -> Self {
            FakeFs(
                files
                    .iter()
                    .map(|(p, b)| (PathBuf::from(p), b.to_vec()))
                    .collect(),
            )
        }
    }

    impl Fs for FakeFs {
        fn exists(&self, p: &Path) -> bool {
            self.0.keys().any(|k| k == p || k.starts_with(p))
        }
        fn list(&self, p: &Path) -> Vec<PathBuf> {
            let mut v: Vec<PathBuf> = self
                .0
                .keys()
                .filter_map(|k| {
                    let rest = k.strip_prefix(p).ok()?;
                    Some(p.join(rest.components().next()?))
                })
                .collect();
            v.sort();
            v.dedup();
            v
        }
        fn read(&self, p: &Path) -> Option<String> {
            self.0
                .get(p)
                .map(|b| String::from_utf8_lossy(b).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
    }

    #[test]
    fn env_table() {
        let incoming = map(&[
            ("HOME", "/home/ana"),
            ("PATH", "/usr/bin:/bin"),
            ("LD_LIBRARY_PATH", "/opt/lib"),
            ("WINEDLLOVERRIDES", "winhttp=n,b"),
            ("MESA_VK_WSI_PRESENT_MODE", "fifo"),
        ]);
        let defaults = crate::tuning::defaults(Vendor::Intel);
        let dlls = vec!["d3d11".to_string(), "dxgi".to_string()];
        let e = wine_env(&EnvInput {
            incoming: &incoming,
            prefix: Path::new("/c/pfx"),
            defaults: &defaults,
            dlls: &dlls,
        });
        // (variable, expected value; None = absent)
        let table: &[(&str, Option<&str>)] = &[
            ("PATH", Some("/usr/bin:/bin")),
            ("LD_LIBRARY_PATH", Some("/opt/lib")),
            ("WINEPREFIX", Some("/c/pfx")),
            ("WINEDLLOVERRIDES", Some("d3d11,dxgi=n,b;winhttp=n,b")),
            ("MESA_VK_WSI_PRESENT_MODE", Some("fifo")),
            ("WINEDEBUG", Some("-all")),
            ("DXVK_LOG_PATH", Some("none")),
            ("STAGING_SHARED_MEMORY", Some("1")),
            ("WINEESYNC", None),
            ("DXVK_ENABLE_NVAPI", None),
            ("HOME", Some("/home/ana")),
        ];
        for (k, want) in table {
            assert_eq!(e.get(*k).map(String::as_str), *want, "{k}");
        }
    }

    #[test]
    fn nvapi_and_proton_switches() {
        let incoming = map(&[]);
        let defaults = crate::tuning::defaults(Vendor::Nvidia);
        let dlls = vec!["d3d11".to_string(), "nvapi64".to_string()];
        let e = wine_env(&EnvInput {
            incoming: &incoming,
            prefix: Path::new("/p"),
            defaults: &defaults,
            dlls: &dlls,
        });
        assert_eq!(e["DXVK_ENABLE_NVAPI"], "1");

        use dlls::Component::*;
        let m = map;
        // (incoming, vendor, component, wanted)
        let cases = [
            (m(&[]), Vendor::Intel, Dxvk, true),
            (
                m(&[("PROTON_USE_WINED3D", "1")]),
                Vendor::Intel,
                Dxvk,
                false,
            ),
            (m(&[("PROTON_NO_D3D12", "1")]), Vendor::Intel, Vkd3d, false),
            (m(&[]), Vendor::Intel, Nvapi, false),
            (m(&[]), Vendor::Nvidia, Nvapi, true),
            (
                m(&[("PROTON_ENABLE_NVAPI", "0")]),
                Vendor::Nvidia,
                Nvapi,
                false,
            ),
            (m(&[("PROTON_ENABLE_NVAPI", "1")]), Vendor::Amd, Nvapi, true),
            (
                m(&[("PROTON_USE_WINED3D", "1")]),
                Vendor::Nvidia,
                Nvapi,
                false,
            ),
        ];
        for (incoming, vendor, c, want) in cases {
            assert_eq!(
                wanted(c, &incoming, vendor),
                want,
                "{incoming:?} {vendor:?} {c:?}"
            );
        }
    }

    fn wine(umu: bool) -> Wine {
        Wine {
            bin: "/usr/bin/wine".into(),
            umu: umu.then(|| "/usr/bin/umu-run".into()),
            forced: false,
            version: None,
            vendor: Vendor::Other,
        }
    }

    #[test]
    fn runner_prefers_wine_then_umu() {
        use fake::FakeFs;
        let both = FakeFs::with(&[("/usr/bin/wine", b""), ("/usr/bin/umu-run", b"")]);
        let umu_only = FakeFs::with(&[("/usr/bin/umu-run", b"")]);
        let none = FakeFs::default();
        assert_eq!(
            wine(true).runner(&both),
            Some(Runner::Wine("/usr/bin/wine".into()))
        );
        assert_eq!(
            wine(true).runner(&umu_only),
            Some(Runner::Umu("/usr/bin/umu-run".into()))
        );
        assert_eq!(wine(false).runner(&umu_only), None, "WOL_WINE: one loader");
        assert_eq!(wine(true).runner(&none), None);
        let umu = Runner::Umu("/usr/bin/umu-run".into());
        assert!(
            umu.defaults(Vendor::Other)
                .contains(&("GAMEID", "umu-default"))
        );
        assert!(
            !Runner::Wine("/usr/bin/wine".into())
                .defaults(Vendor::Other)
                .iter()
                .any(|kv| kv.0 == "GAMEID")
        );
    }

    #[test]
    fn dlls_follow_switches_and_need_dxvk_for_nvapi() {
        use fake::FakeFs;
        let p = "/s/steamapps/common/Proton 10.0/files/lib/wine";
        let fs = FakeFs::with(&[
            (&format!("{p}/dxvk/x86_64-windows/d3d11.dll"), b""),
            (&format!("{p}/nvapi/x86_64-windows/nvapi64.dll"), b""),
            (&format!("{p}/vkd3d-proton/x86_64-windows/d3d12.dll"), b""),
            ("/s/steamapps/common/Proton 9.0/readme", b""),
        ]);
        let dirs = proton_dirs(&[PathBuf::from("/s")], &fs);
        assert_eq!(
            dirs,
            [PathBuf::from("/s/steamapps/common/Proton 10.0")],
            "only installs that carry files/"
        );
        let names = |kv: &[(&str, &str)], v| -> Vec<dlls::Component> {
            pick_dlls(&map(kv), v, &dirs, &fs)
                .iter()
                .map(|s| s.component)
                .collect()
        };
        use dlls::Component::*;
        assert_eq!(names(&[], Vendor::Nvidia), [Dxvk, Vkd3d, Nvapi]);
        assert_eq!(names(&[], Vendor::Intel), [Dxvk, Vkd3d]);
        assert_eq!(
            names(&[("PROTON_USE_WINED3D", "1")], Vendor::Nvidia),
            [Vkd3d],
            "no DXVK, so no NVAPI"
        );
        assert!(proton_dirs(&[PathBuf::from("/none")], &fs).is_empty());
    }
}

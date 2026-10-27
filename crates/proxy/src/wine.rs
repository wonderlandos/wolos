//! Windows programs on wine-proton (Steam verbs, `--exe`): the facts wol-run reads, the
//! environment it builds, the plan (prefix, DLLs, argv) and its executor.
use crate::{LaunchPlan, dlls, tuning::Vendor};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wol_core::games::wine::{self as w, PeArch, Version};

/// File access the planners need (real: [`RealFs`]; tests: a map).
pub trait Fs {
    fn exists(&self, p: &Path) -> bool;
    /// Entries of a directory, sorted; empty if missing.
    fn list(&self, p: &Path) -> Vec<PathBuf>;
    /// First bytes (PE header); empty if unreadable.
    fn head(&self, p: &Path) -> Vec<u8>;
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
    fn head(&self, p: &Path) -> Vec<u8> {
        use std::io::Read;
        let mut b = vec![0; 4096];
        let n = std::fs::File::open(p)
            .and_then(|mut f| f.read(&mut b))
            .unwrap_or(0);
        b.truncate(n);
        b
    }
    fn read(&self, p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }
}

/// This machine's wine-proton, as wol-run sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wine {
    pub root: PathBuf,
    /// `WOL_WINE`: one loader for every program (a custom build).
    pub forced: Option<PathBuf>,
    pub version: Option<Version>,
    pub vendor: Vendor,
}

impl Wine {
    /// Loader for `program` (its PE machine picks bin/wine64, bin/wine or bin-wow64/wine).
    pub fn loader(&self, home: &Path, program: Option<&Path>, fs: &dyn Fs) -> Option<PathBuf> {
        if let Some(f) = &self.forced {
            return Some(f.clone());
        }
        let pe = program.and_then(|p| w::pe_arch(&fs.head(p)));
        w::loader(&|p| fs.exists(p), &self.root, home, pe)
    }

    /// Real host: `WOL_WINE`, `<loader> --version`, `WOL_GPU_PRESET` or the hardware snapshot.
    pub fn detect(var: &dyn Fn(&str) -> Option<String>, home: &Path) -> Self {
        let root = PathBuf::from(w::ROOT);
        let forced = var("WOL_WINE").map(PathBuf::from);
        let probe = forced
            .clone()
            .or_else(|| w::loader(&|p| p.exists(), &root, home, Some(PeArch::Amd64)));
        let version = probe
            .and_then(|l| {
                wol_core::system::run_with_timeout(
                    &[&l.to_string_lossy(), "--version"],
                    std::time::Duration::from_secs(5),
                )
            })
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
            root,
            forced,
            version,
            vendor,
        }
    }
}

/// FreeBSD's default user PATH (login.conf).
pub const FREEBSD_PATH: &str = "/sbin:/bin:/usr/sbin:/usr/bin:/usr/local/sbin:/usr/local/bin";
/// Steam Runtime and Linux-side variables that must not reach FreeBSD programs.
pub const DROP: [&str; 6] = [
    "STEAM_RUNTIME",
    "STEAM_RUNTIME_LIBRARY_PATH",
    "STEAM_RUNTIME_SCOUT",
    "SYSTEM_LD_LIBRARY_PATH",
    "SYSTEM_PATH",
    "LD_AUDIT",
];
pub const DROP_PREFIXES: [&str; 3] = ["PRESSURE_VESSEL_", "SRT_", "LSU_LINUX_"];
/// Path lists whose Linux entries are removed.
pub const PATH_LISTS: [&str; 9] = [
    "PATH",
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "LIBGL_DRIVERS_PATH",
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "VK_ADD_DRIVER_FILES",
    "VK_LAYER_PATH",
    "VK_ADD_LAYER_PATH",
];
/// Steam's implicit Vulkan layers (overlay, fossilize) are Linux libraries.
pub const STEAM_LAYERS_OFF: [(&str, &str); 2] = [
    ("DISABLE_VK_LAYER_VALVE_steam_overlay_1", "1"),
    ("DISABLE_VK_LAYER_VALVE_steam_fossilize_1", "1"),
];
/// Proton's `PROTON_LOG=1` channels.
pub const LOG_CHANNELS: &str = "+timestamp,+pid,+tid,+seh,+unwind,+threadname,+debugstr,+loaddll";

/// A path that only exists for Linux programs (Linux root, Steam client and runtime dirs).
pub fn is_linux_path(p: &str) -> bool {
    p.starts_with("/compat/linux")
        || [
            "/ubuntu12_32",
            "/ubuntu12_64",
            "/steam-runtime",
            "/steam-utils/lxbin",
            "/steam-utils/lib32",
            "/steam-utils/lib64",
            "gameoverlayrenderer.so",
        ]
        .iter()
        .any(|m| p.contains(m))
}

pub fn flag(env: &BTreeMap<String, String>, k: &str) -> bool {
    env.get(k).is_some_and(|v| v == "1")
}

/// Inputs of [`wine_env`].
pub struct EnvInput<'a> {
    pub incoming: &'a BTreeMap<String, String>,
    pub loader: &'a Path,
    pub prefix: &'a Path,
    pub defaults: &'a [(&'static str, &'static str)],
    /// DLLs installed native-first.
    pub dlls: &'a [String],
    /// Launched by Steam (Linux parent): turn off Steam's Linux Vulkan layers.
    pub steam: bool,
}

/// Environment for wine: the incoming one cleaned of Linux and Steam Runtime state, wine first
/// on PATH, the prefix, DLL overrides, then defaults where unset. `PROTON_*` knobs (read by
/// Valve's Proton script, not by wine) become what that script would set.
pub fn wine_env(i: &EnvInput) -> BTreeMap<String, String> {
    let mut e: BTreeMap<String, String> = i
        .incoming
        .iter()
        .filter(|(k, _)| {
            !DROP.contains(&k.as_str()) && !DROP_PREFIXES.iter().any(|p| k.starts_with(p))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for k in PATH_LISTS {
        let Some(v) = e.get(k) else { continue };
        let sep: &[char] = if k == "LD_PRELOAD" {
            &[':', ' ']
        } else {
            &[':']
        };
        let kept: Vec<&str> = v
            .split(sep)
            .filter(|p| !p.is_empty() && !is_linux_path(p))
            .collect();
        if kept.is_empty() {
            e.remove(k);
        } else {
            e.insert(k.into(), kept.join(":"));
        }
    }
    let bin = i.loader.parent().unwrap_or(Path::new("/"));
    let path = e
        .get("PATH")
        .cloned()
        .unwrap_or_else(|| FREEBSD_PATH.into());
    e.insert("PATH".into(), format!("{}:{path}", bin.display()));
    e.insert("WINEPREFIX".into(), i.prefix.to_string_lossy().into_owned());
    if i.steam {
        for (k, v) in STEAM_LAYERS_OFF {
            e.insert(k.into(), v.into());
        }
    }
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
    let log = flag(i.incoming, "PROTON_LOG");
    if log {
        e.entry("WINEDEBUG".into())
            .or_insert_with(|| LOG_CHANNELS.into());
        e.entry("DXVK_LOG_LEVEL".into())
            .or_insert_with(|| "info".into());
    }
    for (k, v) in i.defaults {
        let skip = (*k == "WINEESYNC" && flag(i.incoming, "PROTON_NO_ESYNC"))
            || (*k == "DXVK_LOG_PATH" && log);
        if !skip {
            e.entry((*k).into()).or_insert_with(|| (*v).into());
        }
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

/// Valve Proton installs in the Steam libraries for this release, best first.
pub fn proton_dirs(libraries: &[PathBuf], version: Option<Version>, fs: &dyn Fs) -> Vec<PathBuf> {
    let Some(v) = version else { return vec![] };
    let names = wol_core::games::steam::proton_names(v);
    names
        .iter()
        .flat_map(|n| {
            libraries
                .iter()
                .map(move |l| l.join("steamapps/common").join(n))
        })
        .filter(|d| fs.exists(&d.join("files")))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WinePlan {
    /// None: nothing to run (destroyprefix).
    pub launch: Option<LaunchPlan>,
    pub loader: PathBuf,
    pub prefix: PathBuf,
    /// Create the prefix (`wine wineboot -u`) when it has no `system.reg`.
    pub ensure_prefix: bool,
    pub copies: Vec<(PathBuf, PathBuf)>,
    /// `wineserver -w` first: the prefix's previous session must end (waitforexitandrun).
    pub wait_server: bool,
    /// Remove this directory (destroyprefix).
    pub remove: Option<PathBuf>,
    /// The program's stderr goes here (`PROTON_LOG=1`).
    pub log: Option<PathBuf>,
    /// One line for stderr.
    pub summary: String,
}

/// Run a plan: prefix, DLLs, wait, program. Helper steps write to `stderr` only, so stdout
/// carries just the program's output (getcompatpath). Returns the program's exit code.
pub fn execute(
    p: &WinePlan,
    stdout: std::os::fd::OwnedFd,
    stderr: std::os::fd::OwnedFd,
) -> std::io::Result<i32> {
    if let Some(d) = &p.remove {
        if d.exists() {
            std::fs::remove_dir_all(d)?;
        }
        return Ok(0);
    }
    let Some(launch) = &p.launch else {
        return Ok(0);
    };
    let helper = |program: &Path, args: &[&str]| -> std::io::Result<()> {
        let mut c = std::process::Command::new(program);
        c.args(args)
            .env_clear()
            .envs(&launch.env)
            .stdin(std::process::Stdio::null())
            .stdout(stderr.try_clone()?)
            .stderr(stderr.try_clone()?);
        if let Some(d) = &launch.cwd {
            c.current_dir(d);
        }
        c.status().map(|_| ())
    };
    if p.ensure_prefix && !p.prefix.join("system.reg").exists() {
        if let Some(parent) = p.prefix.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // `-u` returns once the prefix is written.
        helper(&p.loader, &["wineboot", "-u"])?;
    }
    crate::dlls::copy_changed(&p.copies)?;
    if p.wait_server {
        helper(&w::wineserver(&p.loader), &["-w"])?;
    }
    let err = match &p.log {
        Some(l) => std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(l)?
            .into(),
        None => stderr,
    };
    crate::run(launch, stdout, err)
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
        fn head(&self, p: &Path) -> Vec<u8> {
            self.0.get(p).cloned().unwrap_or_default()
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

    /// Steam (via linux-steam-utils) environment -> what wine gets.
    #[test]
    fn env_translation_table() {
        let incoming = map(&[
            ("HOME", "/home/ana"),
            (
                "PATH",
                "/usr/local/steam-utils/lxbin:/home/ana/.steam/steam/ubuntu12_32/steam-runtime/usr/bin:/compat/linux/bin:/usr/bin:/bin",
            ),
            (
                "LD_PRELOAD",
                "/home/ana/.steam/steam/ubuntu12_32/gameoverlayrenderer.so /home/ana/.steam/steam/ubuntu12_64/gameoverlayrenderer.so",
            ),
            (
                "LD_LIBRARY_PATH",
                "/compat/linux/usr/lib64/nss:/home/ana/.steam/steam/ubuntu12_32",
            ),
            (
                "STEAM_RUNTIME",
                "/home/ana/.steam/steam/ubuntu12_32/steam-runtime",
            ),
            ("SYSTEM_LD_LIBRARY_PATH", "/compat/linux/usr/lib64"),
            ("PRESSURE_VESSEL_FILESYSTEMS_RO", "/compat"),
            ("LSU_LINUX_PATH", "/compat/linux/bin"),
            (
                "VK_ICD_FILENAMES",
                "/compat/linux/usr/share/vulkan/icd.d/intel_icd.x86_64.json",
            ),
            (
                "VK_LAYER_PATH",
                "/usr/local/share/vulkan/explicit_layer.d:/compat/linux/usr/share/vulkan/explicit_layer.d",
            ),
            ("WINEDLLOVERRIDES", "winhttp=n,b"),
            ("MESA_VK_WSI_PRESENT_MODE", "fifo"),
            ("SteamGameId", "1145360"),
            (
                "STEAM_COMPAT_DATA_PATH",
                "/home/ana/.steam/steam/steamapps/compatdata/1145360",
            ),
        ]);
        let defaults = crate::tuning::defaults(Vendor::Intel, Version::parse("wine-10.0"));
        let dlls = vec!["d3d11".to_string(), "dxgi".to_string()];
        let e = wine_env(&EnvInput {
            incoming: &incoming,
            loader: Path::new("/usr/local/wine-proton/bin/wine64"),
            prefix: Path::new("/c/pfx"),
            defaults: &defaults,
            dlls: &dlls,
            steam: true,
        });
        // (variable, expected value; None = absent)
        let table: &[(&str, Option<&str>)] = &[
            ("PATH", Some("/usr/local/wine-proton/bin:/usr/bin:/bin")),
            ("LD_PRELOAD", None),
            ("LD_LIBRARY_PATH", None),
            ("STEAM_RUNTIME", None),
            ("SYSTEM_LD_LIBRARY_PATH", None),
            ("PRESSURE_VESSEL_FILESYSTEMS_RO", None),
            ("LSU_LINUX_PATH", None),
            ("VK_ICD_FILENAMES", None),
            (
                "VK_LAYER_PATH",
                Some("/usr/local/share/vulkan/explicit_layer.d"),
            ),
            ("WINEPREFIX", Some("/c/pfx")),
            ("WINEDLLOVERRIDES", Some("d3d11,dxgi=n,b;winhttp=n,b")),
            ("DISABLE_VK_LAYER_VALVE_steam_overlay_1", Some("1")),
            ("DISABLE_VK_LAYER_VALVE_steam_fossilize_1", Some("1")),
            ("MESA_VK_WSI_PRESENT_MODE", Some("fifo")),
            ("WINEESYNC", Some("1")),
            ("WINEDEBUG", Some("-all")),
            ("DXVK_LOG_PATH", Some("none")),
            ("STAGING_SHARED_MEMORY", Some("1")),
            ("DXVK_ENABLE_NVAPI", None),
            ("SteamGameId", Some("1145360")),
            ("HOME", Some("/home/ana")),
        ];
        for (k, want) in table {
            assert_eq!(e.get(*k).map(String::as_str), *want, "{k}");
        }
    }

    #[test]
    fn proton_switches() {
        let base = |kv: &[(&str, &str)]| {
            let incoming = map(kv);
            let defaults = crate::tuning::defaults(Vendor::Nvidia, Version::parse("wine-10.0"));
            let dlls = vec!["d3d11".to_string(), "nvapi64".to_string()];
            wine_env(&EnvInput {
                incoming: &incoming,
                loader: Path::new("/w/bin/wine"),
                prefix: Path::new("/p"),
                defaults: &defaults,
                dlls: &dlls,
                steam: false,
            })
        };
        let e = base(&[]);
        assert_eq!(e["PATH"], format!("/w/bin:{FREEBSD_PATH}"));
        assert_eq!(e["DXVK_ENABLE_NVAPI"], "1");
        assert!(!e.contains_key("DISABLE_VK_LAYER_VALVE_steam_overlay_1"));
        let e = base(&[("PROTON_NO_ESYNC", "1")]);
        assert!(!e.contains_key("WINEESYNC"));
        let e = base(&[("PROTON_LOG", "1")]);
        assert_eq!(e["WINEDEBUG"], LOG_CHANNELS);
        assert_eq!(e["DXVK_LOG_LEVEL"], "info");
        assert!(!e.contains_key("DXVK_LOG_PATH"));
        let e = base(&[("PROTON_LOG", "1"), ("WINEDEBUG", "+relay")]);
        assert_eq!(e["WINEDEBUG"], "+relay", "the user's channels win");

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

    #[test]
    fn linux_paths() {
        for (p, linux) in [
            ("/compat/linux/usr/lib64", true),
            ("/home/a/.steam/steam/ubuntu12_32", true),
            (
                "/home/a/.steam/steam/ubuntu12_32/steam-runtime/usr/bin",
                true,
            ),
            ("/usr/local/steam-utils/lxbin", true),
            ("/usr/local/steam-utils/bin", false),
            ("/usr/local/bin", false),
            ("/home/a/.local/share/Steam/steamapps/common/Game", false),
        ] {
            assert_eq!(is_linux_path(p), linux, "{p}");
        }
    }

    #[test]
    fn dlls_follow_switches_and_need_dxvk_for_nvapi() {
        use fake::FakeFs;
        let p = "/s/steamapps/common/Proton 10.0/files/lib/wine";
        let fs = FakeFs::with(&[
            (&format!("{p}/dxvk/x86_64-windows/d3d11.dll"), b""),
            (&format!("{p}/nvapi/x86_64-windows/nvapi64.dll"), b""),
            (&format!("{p}/vkd3d-proton/x86_64-windows/d3d12.dll"), b""),
        ]);
        let dirs = proton_dirs(&[PathBuf::from("/s")], Version::parse("wine-10.0"), &fs);
        assert_eq!(dirs, [PathBuf::from("/s/steamapps/common/Proton 10.0")]);
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
        assert!(proton_dirs(&[PathBuf::from("/s")], None, &fs).is_empty());
    }
}

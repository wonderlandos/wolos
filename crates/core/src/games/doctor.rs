//! `wol games doctor`: what Steam and Windows games need on this machine, each with the exact
//! fix. [`gather`] reads facts through [`Host`]; [`checks`] is pure.
use super::{steam, wine};
use crate::system::Host;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Info,
    Warn,
    Fail,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Info => "info",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub detail: String,
    /// Shell command(s) that fix it; empty when nothing to do.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fix: String,
}

/// One `vulkaninfo --summary` device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VkDevice {
    pub name: String,
    pub driver: String,
    pub api: String,
    /// `PHYSICAL_DEVICE_TYPE_*` without the prefix: `INTEGRATED_GPU`, `CPU`, ...
    pub kind: String,
    /// `0x8086`.
    pub vendor: String,
}

/// Linux-ABI `futex_waitv(2)` (syscall 449), probed with [`futex_probe_elf`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FutexWaitv {
    #[default]
    Unknown,
    Present,
    Absent,
    NoLinuxAbi,
}

pub const LINUX_ROOT: &str = "/compat/linux";
pub const ICD_DIR: &str = "/usr/local/share/vulkan/icd.d";
/// Mounts linux-steam-utils requires under the Linux root (rc.d `linux`, `linux_mounts_enable`).
pub const LINUX_MOUNTS: [(&str, &str); 5] = [
    ("/compat/linux/proc", "linprocfs"),
    ("/compat/linux/sys", "linsysfs"),
    ("/compat/linux/dev", "devfs"),
    ("/compat/linux/dev/shm", "tmpfs"),
    ("/compat/linux/dev/fd", "fdescfs"),
];
/// (sysctl, how to persist it). All must be 1 for linux-steam-utils.
pub const SYSCTLS: [(&str, &str, &str); 3] = [
    (
        "unprivileged-chroot",
        "security.bsd.unprivileged_chroot",
        "/etc/sysctl.conf",
    ),
    ("usermount", "vfs.usermount", "/etc/sysctl.conf"),
    (
        "linux-ifnames",
        "compat.linux.use_real_ifnames",
        "/etc/sysctl.kld.d/linux.conf",
    ),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub user: String,
    pub groups: Vec<String>,
    /// `kern.features.linux` (32-bit: the Steam client) and `kern.features.linux64`.
    pub linux32: bool,
    pub linux64: bool,
    /// (mount point, type) from `mount -p`.
    pub mounts: Vec<(String, String)>,
    /// `/compat/linux/etc/os-release` (ID, VERSION_ID).
    pub linux_base: Option<(String, String)>,
    pub nullfs: bool,
    /// Values of [`SYSCTLS`], same order.
    pub sysctls: Vec<Option<i64>>,
    pub machine_id: bool,
    /// `/dev/dri` exists (a DRM driver is loaded).
    pub drm: bool,
    pub steam_utils: bool,
    /// `~/.steam/steam/ubuntu12_32`: the Steam client is bootstrapped.
    pub steam_bootstrapped: bool,
    pub tool_current: bool,
    pub wine: Option<String>,
    pub wine_version: Option<wine::Version>,
    pub i386_package: bool,
    pub bin_wow64: bool,
    /// FreeBSD Vulkan ICD manifests (file names).
    pub icds: Vec<String>,
    /// None: vulkaninfo missing or failed.
    pub vulkan: Option<Vec<VkDevice>>,
    /// `intel` | `amd` | `nvidia`, from the hardware snapshot (else Vulkan vendor ids).
    pub gpu_vendors: Vec<&'static str>,
    /// Kernel module version from `hw.nvidia.version`.
    pub nvidia_version: Option<String>,
    /// `/compat/linux/usr/lib/libGLX_nvidia.so.<version>` (32-bit, the Steam client).
    pub linux_nvidia_libs: bool,
    /// Valve Proton installs carrying DXVK for this wine-proton release.
    pub proton_dirs: Vec<String>,
    pub dxvk_env: bool,
    pub libc6_shim: bool,
    pub python3: bool,
    /// linux-steam-utils' "FreeBSD Wine (emulators/wine-proton)" tool.
    pub lsu_wine_tool: bool,
    pub futex_waitv: FutexWaitv,
}

fn exists(h: &dyn Host, p: &Path) -> bool {
    let (Some(parent), Some(s)) = (p.parent(), p.to_str()) else {
        return false;
    };
    h.list_dir(&parent.to_string_lossy()).iter().any(|e| e == s)
}

/// (mount point, type) per `mount -p` line (fstab format).
pub fn parse_mounts(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (_dev, at, ty) = (f.next()?, f.next()?, f.next()?);
            Some((at.to_owned(), ty.to_owned()))
        })
        .collect()
}

/// (ID, VERSION_ID) from os-release text.
pub fn parse_os_release(text: &str) -> Option<(String, String)> {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim().trim_matches('"').to_owned())
    };
    Some((get("ID")?, get("VERSION_ID").unwrap_or_default()))
}

/// Devices of `vulkaninfo --summary`.
pub fn parse_vulkaninfo(text: &str) -> Vec<VkDevice> {
    let mut out: Vec<VkDevice> = Vec::new();
    let mut in_devices = false;
    for l in text.lines() {
        if l.starts_with("Devices:") {
            in_devices = true;
            continue;
        }
        if !in_devices {
            continue;
        }
        if l.starts_with("GPU") && l.trim_end().ends_with(':') {
            out.push(VkDevice::default());
            continue;
        }
        let (Some(d), Some((k, v))) = (out.last_mut(), l.split_once('=')) else {
            continue;
        };
        let v = v.trim().to_owned();
        match k.trim() {
            "deviceName" => d.name = v,
            "driverInfo" => d.driver = v,
            "driverName" if d.driver.is_empty() => d.driver = v,
            "apiVersion" => d.api = v,
            "vendorID" => d.vendor = v,
            "deviceType" => {
                d.kind = v
                    .strip_prefix("PHYSICAL_DEVICE_TYPE_")
                    .unwrap_or(&v)
                    .to_owned()
            }
            _ => {}
        }
    }
    out
}

/// GPU vendor name for a PCI vendor id (`8086`, `0x8086`, `8086:46a8`).
pub fn vendor_name(id: &str) -> Option<&'static str> {
    let v = id.trim_start_matches("0x").split(':').next()?;
    match v.to_ascii_lowercase().as_str() {
        "8086" => Some("intel"),
        "1002" => Some("amd"),
        "10de" => Some("nvidia"),
        _ => None,
    }
}

/// Version token of `hw.nvidia.version` ("NVIDIA UNIX x86_64 Kernel Module  595.104.02  ...").
pub fn parse_nvidia_version(text: &str) -> Option<String> {
    let after = text.split("Kernel Module").nth(1)?;
    let v = after.split_whitespace().next()?;
    v.chars()
        .all(|c| c.is_ascii_digit() || c == '.')
        .then(|| v.to_owned())
}

/// A static Linux x86-64 ELF (OSABI Linux) that calls `futex_waitv(NULL, 0, 0, NULL, 0)` and
/// exits with the errno: 22 EINVAL = implemented, 38 ENOSYS = missing.
pub fn futex_probe_elf() -> Vec<u8> {
    const BASE: u64 = 0x40_0000;
    #[rustfmt::skip]
    let code: [u8; 32] = [
        0xb8, 0xc1, 0x01, 0x00, 0x00, // mov eax, 449 (futex_waitv)
        0x31, 0xff,                   // xor edi, edi
        0x31, 0xf6,                   // xor esi, esi
        0x31, 0xd2,                   // xor edx, edx
        0x45, 0x31, 0xd2,             // xor r10d, r10d
        0x45, 0x31, 0xc0,             // xor r8d, r8d
        0x0f, 0x05,                   // syscall
        0xf7, 0xd8,                   // neg eax
        0x89, 0xc7,                   // mov edi, eax
        0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, 60 (exit)
        0x0f, 0x05,                   // syscall
        0x0f, 0x0b,                   // ud2
    ];
    let size = (64 + 56 + code.len()) as u64;
    let mut e = Vec::with_capacity(size as usize);
    e.extend_from_slice(b"\x7fELF\x02\x01\x01\x03"); // 64-bit, LE, v1, OSABI Linux
    e.extend_from_slice(&[0; 8]);
    e.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    e.extend_from_slice(&0x3eu16.to_le_bytes()); // x86-64
    e.extend_from_slice(&1u32.to_le_bytes());
    e.extend_from_slice(&(BASE + 64 + 56).to_le_bytes()); // entry
    e.extend_from_slice(&64u64.to_le_bytes()); // phoff
    e.extend_from_slice(&0u64.to_le_bytes()); // shoff
    e.extend_from_slice(&0u32.to_le_bytes()); // flags
    for v in [64u16, 56, 1, 64, 0, 0] {
        e.extend_from_slice(&v.to_le_bytes()); // ehsize phentsize phnum shentsize shnum shstrndx
    }
    e.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    e.extend_from_slice(&5u32.to_le_bytes()); // R+X
    for v in [0, BASE, BASE, size, size, 0x1000] {
        e.extend_from_slice(&v.to_le_bytes()); // offset vaddr paddr filesz memsz align
    }
    e.extend_from_slice(&code);
    e
}

/// Write [`futex_probe_elf`] into `dir`, run it, remove it. On a stock FreeBSD kernel the Linux
/// ABI logs "syscall futex_waitv not implemented" once.
pub fn probe_futex_waitv(dir: &Path) -> FutexWaitv {
    run_probe(dir, &futex_probe_elf())
}

fn run_probe(dir: &Path, elf: &[u8]) -> FutexWaitv {
    use std::os::unix::fs::OpenOptionsExt;
    let p = dir.join(format!("wol-futex-probe-{}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&p)
        .and_then(|mut f| std::io::Write::write_all(&mut f, elf));
    if written.is_err() {
        return FutexWaitv::Unknown;
    }
    // A thread forking elsewhere may briefly hold the write descriptor (ETXTBSY): retry.
    let mut r = Err(std::io::Error::from_raw_os_error(libc::ETXTBSY));
    for _ in 0..20 {
        r = std::process::Command::new(&p)
            .stdin(std::process::Stdio::null())
            .status();
        match &r {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => break,
        }
    }
    let _ = std::fs::remove_file(&p);
    match r.map(|s| s.code()) {
        Ok(Some(22)) => FutexWaitv::Present,
        Ok(Some(38)) => FutexWaitv::Absent,
        Err(e) if e.raw_os_error() == Some(libc::ENOEXEC) => FutexWaitv::NoLinuxAbi,
        _ => FutexWaitv::Unknown,
    }
}

/// Read every fact. `var` reads the caller's environment.
pub fn gather(
    h: &dyn Host,
    home: &Path,
    var: &dyn Fn(&str) -> Option<String>,
    futex_waitv: FutexWaitv,
) -> Facts {
    let read = |p: &Path| h.read_text(&p.to_string_lossy());
    let has = |p: &str| exists(h, Path::new(p));
    let root = Path::new(wine::ROOT);
    let wine = wine::loader(&|p| exists(h, p), root, home, Some(wine::PeArch::Amd64));
    let wine_version = wine
        .as_ref()
        .and_then(|w| h.run(&[&w.to_string_lossy(), "--version"]))
        .and_then(|t| wine::Version::parse(&t));
    let vulkan = h
        .run_slow(&["vulkaninfo", "--summary"])
        .map(|t| parse_vulkaninfo(&t));
    let mut gpu_vendors: Vec<&'static str> = h
        .read_text(shared::hardware::SNAPSHOT_PATH)
        .and_then(|t| serde_json::from_str::<shared::hardware::HardwareSnapshot>(&t).ok())
        .map(|s| {
            s.devices
                .iter()
                .filter(|d| d.kind == shared::hardware::Kind::Gpu)
                .filter_map(|d| vendor_name(&d.id))
                .collect()
        })
        .unwrap_or_default();
    if gpu_vendors.is_empty() {
        gpu_vendors = vulkan
            .iter()
            .flatten()
            .filter_map(|d| vendor_name(&d.vendor))
            .collect();
    }
    gpu_vendors.sort();
    gpu_vendors.dedup();
    let nvidia_version = h
        .sysctl_string("hw.nvidia.version")
        .and_then(|t| parse_nvidia_version(&t));
    let steam_root = steam::steam_root(home, &|p| exists(h, p));
    let proton_dirs = wine_version
        .map(|v| {
            steam::libraries(&steam_root, &read)
                .iter()
                .flat_map(|lib| {
                    steam::proton_names(v)
                        .into_iter()
                        .map(move |n| lib.join("steamapps/common").join(n))
                })
                .filter(|d| exists(h, &d.join("files/lib/wine/dxvk")))
                .map(|d| d.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let i386 = home
        .join(wine::I386_ROOT)
        .join(root.strip_prefix("/").unwrap_or(root))
        .join("bin/wine");
    Facts {
        user: var("USER").unwrap_or_default(),
        groups: h
            .run(&["id", "-Gn"])
            .map(|t| t.split_whitespace().map(String::from).collect())
            .unwrap_or_default(),
        linux32: h.sysctl_i64("kern.features.linux") == Some(1),
        linux64: h.sysctl_i64("kern.features.linux64") == Some(1),
        mounts: h
            .run(&["mount", "-p"])
            .map(|t| parse_mounts(&t))
            .unwrap_or_default(),
        linux_base: h
            .read_text(&format!("{LINUX_ROOT}/etc/os-release"))
            .and_then(|t| parse_os_release(&t)),
        nullfs: h.run(&["kldstat", "-q", "-m", "nullfs"]).is_some(),
        sysctls: SYSCTLS.iter().map(|s| h.sysctl_i64(s.1)).collect(),
        machine_id: has("/etc/machine-id"),
        drm: has("/dev/dri"),
        steam_utils: has(&format!("{}/bin/steam", steam::LSU)),
        steam_bootstrapped: exists(h, &home.join(".steam/steam/ubuntu12_32")),
        tool_current: steam::is_current(&read, &steam_root, steam::WOL_RUN),
        wine: wine.map(|w| w.to_string_lossy().into_owned()),
        wine_version,
        i386_package: exists(h, &i386),
        bin_wow64: exists(h, &root.join("bin-wow64/wine")),
        icds: h
            .list_dir(ICD_DIR)
            .iter()
            .filter(|p| p.ends_with(".json"))
            .filter_map(|p| p.rsplit('/').next().map(String::from))
            .collect(),
        vulkan,
        gpu_vendors,
        linux_nvidia_libs: nvidia_version
            .as_ref()
            .is_some_and(|v| has(&format!("{LINUX_ROOT}/usr/lib/libGLX_nvidia.so.{v}"))),
        nvidia_version,
        proton_dirs,
        dxvk_env: var("WOL_DXVK_DIR").is_some(),
        libc6_shim: has("/usr/local/bin/c6-shim-env"),
        python3: has("/usr/local/bin/python3"),
        lsu_wine_tool: has(&format!("{}/tools/LSU_FreeBSD_Wine/run.rb", steam::LSU)),
        futex_waitv,
    }
}

fn check(
    id: &'static str,
    status: Status,
    detail: impl Into<String>,
    fix: impl Into<String>,
) -> Check {
    Check {
        id,
        status,
        detail: detail.into(),
        fix: fix.into(),
    }
}

/// Every check, Steam client first, then Windows games.
pub fn checks(f: &Facts) -> Vec<Check> {
    use Status::*;
    let mut c = Vec::new();
    let ok_or = |good: bool, bad: Status| if good { Ok } else { bad };
    c.push(check(
        "linux-abi",
        ok_or(f.linux32 && f.linux64, Fail),
        match (f.linux32, f.linux64) {
            (true, true) => {
                "Linux ABI loaded, 32- and 64-bit (the Steam client is a Linux program)"
            }
            (false, true) => "32-bit Linux ABI missing (the Steam client is 32-bit)",
            _ => "Linux ABI not loaded (the Steam client is a Linux program)",
        },
        if f.linux32 && f.linux64 {
            ""
        } else {
            "sudo sysrc linux_enable=YES && sudo service linux start"
        },
    ));
    let missing: Vec<String> = LINUX_MOUNTS
        .iter()
        .filter(|(at, ty)| !f.mounts.iter().any(|m| m.0 == *at && m.1 == *ty))
        .map(|(at, ty)| format!("{ty} on {at}"))
        .collect();
    c.push(check(
        "linux-mounts",
        ok_or(missing.is_empty(), Fail),
        if missing.is_empty() {
            "linprocfs, linsysfs, devfs, tmpfs, fdescfs mounted in /compat/linux".into()
        } else {
            format!("not mounted: {}", missing.join(", "))
        },
        if missing.is_empty() {
            ""
        } else {
            "sudo sysrc linux_mounts_enable=YES && sudo service linux restart"
        },
    ));
    let base_ok = f.linux_base.as_ref().is_some_and(|(id, v)| {
        id == "rocky"
            && v.split('.')
                .next()
                .and_then(|m| m.parse::<u32>().ok())
                .is_some_and(|m| m >= 9)
    });
    c.push(check(
        "linux-base",
        ok_or(base_ok, Fail),
        match &f.linux_base {
            Some((id, v)) => format!("{id} {v} in /compat/linux"),
            None => "no Linux base in /compat/linux".into(),
        },
        if base_ok {
            ""
        } else {
            "sudo pkg install -y linux_base-rl9"
        },
    ));
    c.push(check(
        "steam-utils",
        ok_or(f.steam_utils, Fail),
        if f.steam_utils {
            "linux-steam-utils installed (Steam launcher)"
        } else {
            "linux-steam-utils missing (runs the Linux Steam client)"
        },
        if f.steam_utils {
            ""
        } else {
            "sudo pkg install -y linux-steam-utils"
        },
    ));
    for ((id, name, persist), v) in SYSCTLS.iter().zip(&f.sysctls) {
        let good = *v == Some(1);
        c.push(check(
            id,
            ok_or(good, Fail),
            format!(
                "{name}={}",
                v.map_or("unset".to_string(), |x| x.to_string())
            ),
            if good {
                String::new()
            } else {
                format!("sudo sysctl {name}=1 && echo {name}=1 | sudo tee -a {persist}")
            },
        ));
    }
    c.push(check(
        "nullfs",
        ok_or(f.nullfs, Fail),
        if f.nullfs {
            "nullfs loaded"
        } else {
            "nullfs not loaded (Steam's web helper chroot)"
        },
        if f.nullfs {
            ""
        } else {
            "sudo kldload nullfs && sudo sysrc kld_list+=nullfs"
        },
    ));
    c.push(check(
        "machine-id",
        ok_or(f.machine_id, Fail),
        if f.machine_id {
            "/etc/machine-id present"
        } else {
            "/etc/machine-id missing"
        },
        if f.machine_id {
            ""
        } else {
            "sysctl -n kern.hostuuid | tr -d - | sudo tee /etc/machine-id"
        },
    ));
    let video = f.groups.iter().any(|g| g == "video");
    c.push(match (f.drm, video) {
        (false, _) => check(
            "gpu-access",
            Warn,
            "no GPU driver loaded (/dev/dri missing)",
            "wol hw status",
        ),
        (true, true) => check("gpu-access", Ok, "member of video (GPU access)", ""),
        (true, false) => check(
            "gpu-access",
            Fail,
            format!("{} is not in group video", f.user),
            format!(
                "sudo pw groupmod video -m {}  # then log out and in",
                if f.user.is_empty() { "$USER" } else { &f.user }
            ),
        ),
    });
    c.push(check(
        "steam",
        ok_or(f.steam_bootstrapped, Warn),
        if f.steam_bootstrapped {
            "Steam client installed in ~/.steam/steam"
        } else {
            "Steam client not downloaded yet"
        },
        if f.steam_bootstrapped {
            ""
        } else {
            "wol games steam"
        },
    ));
    c.push(check(
        "wolos-proton",
        ok_or(f.tool_current, Warn),
        if f.tool_current {
            "WolOS Proton compatibility tool installed"
        } else {
            "WolOS Proton compatibility tool missing or outdated"
        },
        if f.tool_current {
            ""
        } else {
            "wol games steam-setup"
        },
    ));
    let pkg_upgrade = "sudo pkg upgrade -y wine-proton  # 11.x (WoW64) is on the latest branch";
    match (&f.wine, f.wine_version) {
        (None, _) => c.push(check(
            "wine-proton",
            Fail,
            "wine-proton not installed (runs Windows games natively)",
            "sudo pkg install -y wine-proton",
        )),
        (Some(w), None) => c.push(check(
            "wine-proton",
            Warn,
            format!("{w}: version unreadable"),
            "sudo pkg install -f -y wine-proton",
        )),
        (Some(w), Some(v)) => {
            c.push(check(
                "wine-proton",
                Ok,
                format!("wine-proton {v} ({w}), native FreeBSD"),
                "",
            ));
            c.push(match wine::win32(v, f.i386_package, f.bin_wow64) {
                wine::Win32::Wow64 => check(
                    "win32",
                    Ok,
                    "32-bit Windows programs: WoW64, no lib32 needed",
                    "",
                ),
                wine::Win32::I386Package => check(
                    "win32",
                    Ok,
                    "32-bit Windows programs: i386 wine-proton in ~/.i386-wine-pkg",
                    "",
                ),
                wine::Win32::Wow64Experimental => check(
                    "win32",
                    Warn,
                    format!("32-bit Windows programs: wine-proton {v} bin-wow64 (experimental)"),
                    pkg_upgrade,
                ),
                wine::Win32::Missing => check(
                    "win32",
                    Warn,
                    "32-bit Windows programs cannot run",
                    pkg_upgrade,
                ),
            });
            c.push(match wine::sync(v) {
                wine::Sync::Esync => check("sync", Info, "esync (eventfd), WINEESYNC=1", ""),
                wine::Sync::Server => check(
                    "sync",
                    Info,
                    "wineserver sync: wine-proton 11 has no esync; its fsync and ntsync are Linux-only",
                    "",
                ),
            });
        }
    }
    c.push(check(
        "futex-waitv",
        if f.futex_waitv == FutexWaitv::Present {
            Ok
        } else {
            Info
        },
        match f.futex_waitv {
            FutexWaitv::Present => {
                "Linux ABI futex_waitv present (Linux games, Linux Proton fsync)"
            }
            FutexWaitv::Absent => {
                "Linux ABI futex_waitv missing: Linux Proton uses esync (native games unaffected)"
            }
            FutexWaitv::NoLinuxAbi => "futex_waitv not probed: Linux ABI not loaded",
            FutexWaitv::Unknown => "futex_waitv not probed",
        },
        "",
    ));
    let nvidia = f.gpu_vendors.contains(&"nvidia");
    let driver_pkg = if nvidia { "nvidia-driver" } else { "mesa-dri" };
    c.push(check(
        "vulkan-icd",
        ok_or(!f.icds.is_empty(), Fail),
        if f.icds.is_empty() {
            format!("no Vulkan driver in {ICD_DIR}")
        } else {
            format!("Vulkan drivers: {}", f.icds.join(", "))
        },
        if f.icds.is_empty() {
            format!("sudo pkg install -y {driver_pkg}")
        } else {
            String::new()
        },
    ));
    c.push(match &f.vulkan {
        None => check(
            "vulkan",
            Warn,
            "vulkaninfo missing or failed",
            "sudo pkg install -y vulkan-tools && vulkaninfo --summary",
        ),
        Some(d) if d.is_empty() => check("vulkan", Fail, "no Vulkan device", "wol hw status"),
        Some(d) => {
            let names: Vec<String> = d
                .iter()
                .map(|x| format!("{} ({}, Vulkan {})", x.name, x.driver, x.api))
                .collect();
            if d.iter().all(|x| x.kind == "CPU") {
                check(
                    "vulkan",
                    Warn,
                    format!("software Vulkan only: {}", names.join("; ")),
                    "wol hw status",
                )
            } else {
                check("vulkan", Ok, names.join("; "), "")
            }
        }
    });
    if nvidia {
        c.push(match (&f.nvidia_version, f.linux_nvidia_libs) {
            (None, _) => check(
                "nvidia",
                Fail,
                "NVIDIA GPU without the nvidia driver",
                "wol hw fix --yes",
            ),
            (Some(v), false) => check(
                "nvidia",
                Fail,
                format!("nvidia {v}: Linux libraries for the Steam client missing"),
                "sudo pkg install -y linux-nvidia-libs",
            ),
            (Some(v), true) => check(
                "nvidia",
                Ok,
                format!("nvidia {v}, Linux libraries match"),
                "",
            ),
        });
    }
    let install_proton = f
        .wine_version
        .and_then(|v| Some((v, steam::proton_app_id(v)?)))
        .map(|(v, id)| format!("in Steam install Proton {v}: steam steam://install/{id}"))
        .unwrap_or_else(|| "set WOL_DXVK_DIR to an extracted DXVK release".into());
    c.push(if let Some(d) = f.proton_dirs.first() {
        check(
            "dxvk",
            Ok,
            format!("DXVK, vkd3d-proton, dxvk-nvapi from {d}"),
            "",
        )
    } else if f.dxvk_env {
        check("dxvk", Ok, "DXVK from WOL_DXVK_DIR", "")
    } else {
        check(
            "dxvk",
            Warn,
            "no DXVK: Direct3D 8-11 use wined3d (OpenGL), Direct3D 12 uses wine's vkd3d",
            install_proton.clone(),
        )
    });
    let steamworks = f.lsu_wine_tool && f.libc6_shim && f.python3 && !f.proton_dirs.is_empty();
    c.push(check(
        "steamworks",
        Info,
        if steamworks {
            "games that need the Steam client API: pick \"FreeBSD Wine (emulators/wine-proton)\""
        } else {
            "games that need the Steam client API need linux-steam-utils' FreeBSD Wine tool"
        },
        if steamworks || !f.steam_utils {
            String::new()
        } else {
            let mut fix = vec![];
            if !(f.libc6_shim && f.python3) {
                fix.push("sudo pkg install -y libc6-shim python3".to_string());
            }
            if f.proton_dirs.is_empty() {
                fix.push(install_proton.clone());
            }
            fix.join(" && ")
        },
    ));
    c
}

/// No check failed.
pub fn ready(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.status != Status::Fail)
}

/// Checks linux-steam-utils needs before the Steam client starts (its `steam` script refuses
/// without them); the others only matter once a game runs.
pub const STEAM_CLIENT: [&str; 10] = [
    "linux-abi",
    "linux-mounts",
    "linux-base",
    "steam-utils",
    "unprivileged-chroot",
    "usermount",
    "linux-ifnames",
    "nullfs",
    "machine-id",
    "nvidia",
];

/// Failed checks that keep the Steam client from starting.
pub fn steam_blockers(checks: &[Check]) -> Vec<&Check> {
    checks
        .iter()
        .filter(|c| c.status == Status::Fail && STEAM_CLIENT.contains(&c.id))
        .collect()
}

/// Directory for the futex probe: `$XDG_RUNTIME_DIR`, else the temp dir.
pub fn probe_dir(var: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests;

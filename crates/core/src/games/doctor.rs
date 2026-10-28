//! `wol games doctor`: what Steam and Windows games need on this machine, each with the exact
//! fix. Steam is the native Arch package (multilib); Proton comes through Steam; wine is native.
//! [`gather`] reads facts through [`Host`]; [`checks`] is pure.
use super::wine;
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

/// `futex_waitv(2)` (syscall 449, Proton's fsync), probed with [`futex_probe_elf`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FutexWaitv {
    #[default]
    Unknown,
    Present,
    Absent,
    NoLinuxAbi,
}

/// Vulkan ICD manifests (64- and 32-bit drivers).
pub const ICD_DIR: &str = "/usr/share/vulkan/icd.d";
pub const PACMAN_CONF: &str = "/etc/pacman.conf";
pub const STEAM_BIN: &str = "/usr/bin/steam";
/// Kernel settings games need; none on Linux (kept for callers that list them).
pub const SYSCTLS: [(&str, &str, &str); 0] = [];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub user: String,
    pub groups: Vec<String>,
    /// `[multilib]` enabled in pacman.conf (Steam and the 32-bit drivers live there).
    pub multilib: bool,
    /// The steam package.
    pub steam: bool,
    /// `~/.local/share/Steam/ubuntu12_32`: the Steam client is bootstrapped.
    pub steam_bootstrapped: bool,
    /// `/dev/dri` exists (a DRM driver is loaded).
    pub drm: bool,
    /// Vulkan ICD manifests (file names).
    pub icds: Vec<String>,
    /// 32-bit Vulkan loader and OpenGL (the Steam client and 32-bit games).
    pub lib32_vulkan: bool,
    pub lib32_gl: bool,
    /// None: vulkaninfo missing or failed.
    pub vulkan: Option<Vec<VkDevice>>,
    /// `intel` | `amd` | `nvidia`, from the hardware snapshot (else Vulkan vendor ids).
    pub gpu_vendors: Vec<&'static str>,
    /// Kernel module version from `/proc/driver/nvidia/version`.
    pub nvidia_version: Option<String>,
    /// `/usr/lib32/libGLX_nvidia.so.<version>` (lib32-nvidia-utils, same version).
    pub lib32_nvidia: bool,
    pub wine: Option<String>,
    pub wine_version: Option<wine::Version>,
    pub futex_waitv: FutexWaitv,
    /// `/dev/ntsync` (Proton's NT sync driver).
    pub ntsync: bool,
    /// Running kernel release (`/proc/sys/kernel/osrelease`).
    pub kernel: String,
    /// WolOS base version (`/etc/os-release` VERSION_ID).
    pub base: Option<String>,
}

fn exists(h: &dyn Host, p: &Path) -> bool {
    let (Some(parent), Some(s)) = (p.parent(), p.to_str()) else {
        return false;
    };
    h.list_dir(&parent.to_string_lossy()).iter().any(|e| e == s)
}

/// `[multilib]` section header not commented out.
pub fn multilib_enabled(pacman_conf: &str) -> bool {
    pacman_conf.lines().any(|l| l.trim() == "[multilib]")
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

/// The running kernel is the WolOS one (`linux-wolos` releases carry `wolos` in their name).
pub fn is_wolos_kernel(release: &str) -> bool {
    release.contains("wolos")
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

/// Driver version from `/proc/driver/nvidia/version` ("NVRM version: NVIDIA UNIX Open Kernel
/// Module for x86_64  570.144  Release Build ..."): the first version token after "Kernel Module".
pub fn parse_nvidia_version(text: &str) -> Option<String> {
    let after = text.split("Kernel Module").nth(1)?;
    after
        .split_whitespace()
        .find(|v| v.contains('.') && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(str::to_owned)
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

/// Write [`futex_probe_elf`] into `dir`, run it, remove it.
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
    let has = |p: &str| exists(h, Path::new(p));
    let wine = has("/usr/bin/wine").then(|| "/usr/bin/wine".to_string());
    let wine_version = wine
        .as_ref()
        .and_then(|w| h.run(&[w, "--version"]))
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
        .read_text("/proc/driver/nvidia/version")
        .and_then(|t| parse_nvidia_version(&t));
    Facts {
        user: var("USER").unwrap_or_default(),
        groups: h
            .run(&["id", "-Gn"])
            .map(|t| t.split_whitespace().map(String::from).collect())
            .unwrap_or_default(),
        multilib: h
            .read_text(PACMAN_CONF)
            .is_some_and(|t| multilib_enabled(&t)),
        steam: has(STEAM_BIN),
        steam_bootstrapped: exists(h, &home.join(".local/share/Steam/ubuntu12_32")),
        drm: has("/dev/dri"),
        icds: h
            .list_dir(ICD_DIR)
            .iter()
            .filter(|p| p.ends_with(".json"))
            .filter_map(|p| p.rsplit('/').next().map(String::from))
            .collect(),
        lib32_vulkan: has("/usr/lib32/libvulkan.so.1"),
        lib32_gl: has("/usr/lib32/libGLX_mesa.so.0") || has("/usr/lib32/libGLX_nvidia.so.0"),
        vulkan,
        gpu_vendors,
        lib32_nvidia: nvidia_version
            .as_ref()
            .is_some_and(|v| has(&format!("/usr/lib32/libGLX_nvidia.so.{v}"))),
        nvidia_version,
        wine,
        wine_version,
        futex_waitv,
        ntsync: has("/dev/ntsync"),
        kernel: h
            .read_text("/proc/sys/kernel/osrelease")
            .map(|t| t.trim().to_owned())
            .unwrap_or_default(),
        base: h
            .read_text("/etc/os-release")
            .and_then(|t| parse_os_release(&t))
            .map(|(_, v)| v)
            .filter(|v| !v.is_empty()),
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

fn ok_or(ok: bool, bad: Status) -> Status {
    if ok { Status::Ok } else { bad }
}

/// 64- and 32-bit Vulkan driver packages for the GPU vendors.
fn vulkan_packages(vendors: &[&str]) -> String {
    let mut p = vec!["vulkan-icd-loader", "lib32-vulkan-icd-loader"];
    for v in vendors {
        p.extend(match *v {
            "intel" => ["vulkan-intel", "lib32-vulkan-intel"],
            "amd" => ["vulkan-radeon", "lib32-vulkan-radeon"],
            _ => ["nvidia-utils", "lib32-nvidia-utils"],
        });
    }
    format!("sudo pacman -S --needed {}", p.join(" "))
}

/// Every check: the WolOS base first, then the Steam client, then games.
pub fn checks(f: &Facts) -> Vec<Check> {
    let mut c = Vec::new();
    c.push(check(
        "base",
        if is_wolos_kernel(&f.kernel) {
            Status::Ok
        } else {
            Status::Info
        },
        format!(
            "WolOS base {}, kernel {}{}",
            f.base.as_deref().unwrap_or("unknown"),
            if f.kernel.is_empty() {
                "unknown"
            } else {
                &f.kernel
            },
            if is_wolos_kernel(&f.kernel) {
                " (linux-wolos)"
            } else {
                " (not linux-wolos: your own kernel, kept)"
            }
        ),
        "",
    ));
    c.push(check(
        "multilib",
        ok_or(f.multilib, Status::Fail),
        if f.multilib {
            "multilib repository on (Steam and 32-bit drivers)"
        } else {
            "multilib repository off: Steam and the 32-bit drivers live there"
        },
        if f.multilib {
            ""
        } else {
            "enable [multilib] in /etc/pacman.conf, then sudo pacman -Sy"
        },
    ));
    c.push(check(
        "steam",
        ok_or(f.steam, Status::Fail),
        if f.steam {
            "Steam installed"
        } else {
            "Steam not installed"
        },
        if f.steam { "" } else { "sudo pacman -S steam" },
    ));
    c.push(check(
        "drm",
        ok_or(f.drm, Status::Fail),
        if f.drm {
            "graphics driver loaded (/dev/dri)"
        } else {
            "no graphics driver loaded (/dev/dri missing)"
        },
        if f.drm { "" } else { "wol hw doctor" },
    ));
    let gl = f.lib32_gl && f.lib32_vulkan;
    c.push(check(
        "lib32-gl",
        ok_or(gl, Status::Fail),
        if gl {
            "32-bit OpenGL and Vulkan present (the Steam client is 32-bit)"
        } else {
            "32-bit OpenGL or Vulkan missing (the Steam client is 32-bit)"
        },
        if gl {
            String::new()
        } else {
            format!("{} lib32-mesa", vulkan_packages(&f.gpu_vendors))
        },
    ));
    c.push(check(
        "vulkan-icd",
        ok_or(!f.icds.is_empty(), Status::Fail),
        if f.icds.is_empty() {
            format!("no Vulkan driver in {ICD_DIR} (DXVK and VKD3D need one)")
        } else {
            format!("Vulkan drivers: {}", f.icds.join(", "))
        },
        if f.icds.is_empty() {
            vulkan_packages(&f.gpu_vendors)
        } else {
            String::new()
        },
    ));
    let gpus: Vec<&VkDevice> = f
        .vulkan
        .iter()
        .flatten()
        .filter(|d| d.kind != "CPU")
        .collect();
    c.push(match (&f.vulkan, gpus.first()) {
        (None, _) => check(
            "vulkan",
            Status::Warn,
            "vulkaninfo missing or failed",
            "sudo pacman -S vulkan-tools",
        ),
        (Some(_), None) => check(
            "vulkan",
            Status::Fail,
            "Vulkan sees only the CPU renderer (llvmpipe): games would crawl",
            vulkan_packages(&f.gpu_vendors),
        ),
        (Some(_), Some(d)) => check(
            "vulkan",
            Status::Ok,
            format!("{} ({}, Vulkan {})", d.name, d.driver, d.api),
            "",
        ),
    });
    if let Some(v) = &f.nvidia_version {
        c.push(check(
            "nvidia",
            ok_or(f.lib32_nvidia, Status::Fail),
            if f.lib32_nvidia {
                format!("nvidia {v}, 32-bit libraries match")
            } else {
                format!("nvidia {v}: 32-bit libraries for the Steam client missing or older")
            },
            if f.lib32_nvidia {
                ""
            } else {
                "sudo pacman -S --needed lib32-nvidia-utils"
            },
        ));
    }
    c.push(check(
        "futex-waitv",
        match f.futex_waitv {
            FutexWaitv::Present => Status::Ok,
            FutexWaitv::Unknown => Status::Info,
            _ => Status::Warn,
        },
        match f.futex_waitv {
            FutexWaitv::Present => "futex_waitv present (Proton fsync)",
            FutexWaitv::Unknown => "futex_waitv not probed",
            _ => "futex_waitv missing: Proton falls back to esync",
        },
        "",
    ));
    c.push(check(
        "ntsync",
        ok_or(f.ntsync, Status::Info),
        if f.ntsync {
            "/dev/ntsync present (Proton NT sync)"
        } else {
            "/dev/ntsync missing: Proton uses fsync"
        },
        "",
    ));
    c.push(check(
        "wine",
        ok_or(f.wine.is_some(), Status::Info),
        match (&f.wine, f.wine_version) {
            (Some(_), Some(v)) => {
                format!("wine {}.{} (Windows apps outside Steam)", v.major, v.minor)
            }
            (Some(w), None) => format!("{w} (Windows apps outside Steam)"),
            _ => "wine not installed: Steam games use Proton, which Steam installs".into(),
        },
        if f.wine.is_some() {
            ""
        } else {
            "sudo pacman -S wine (only for Windows apps outside Steam)"
        },
    ));
    c
}

/// No check failed.
pub fn ready(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.status != Status::Fail)
}

/// Checks the Steam client needs before it starts; the others only matter once a game runs.
pub const STEAM_CLIENT: [&str; 5] = ["multilib", "steam", "drm", "lib32-gl", "nvidia"];

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

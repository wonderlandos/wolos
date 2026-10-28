use super::*;
use crate::system::fake::FakeHost;

const INTEL: &str = include_str!("vulkaninfo-intel.txt");
const NVIDIA: &str = include_str!("vulkaninfo-nvidia-llvmpipe.txt");
const HOME: &str = "/home/ana";

#[test]
fn vulkaninfo_summary() {
    let d = parse_vulkaninfo(INTEL);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].name, "Intel(R) Iris(R) Xe Graphics (ADL GT2)");
    assert_eq!(d[0].kind, "INTEGRATED_GPU");
    assert_eq!(d[0].vendor, "0x8086");
    assert!(d[0].driver.starts_with("Mesa 26"), "{}", d[0].driver);
    assert!(d[0].api.starts_with("1.4."));
    let d = parse_vulkaninfo(NVIDIA);
    let kinds: Vec<&str> = d.iter().map(|x| x.kind.as_str()).collect();
    assert_eq!(kinds, ["DISCRETE_GPU", "CPU"]);
    assert_eq!(d[0].driver, "595.104.02");
    assert_eq!(d[1].name, "llvmpipe (LLVM 19.1.7, 256 bits)");
    assert!(parse_vulkaninfo("ERROR: [Loader Message] no ICD\n").is_empty());
}

#[test]
fn small_parsers() {
    assert_eq!(
        parse_os_release("NAME=\"WolOS\"\nID=wolos\nVERSION_ID=\"2026.11\"\n"),
        Some(("wolos".into(), "2026.11".into()))
    );
    assert_eq!(parse_os_release("NAME=x\n"), None);
    assert_eq!(
        parse_nvidia_version(
            "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  570.144  Release Build  (builder)\n"
        ),
        Some("570.144".into())
    );
    assert_eq!(parse_nvidia_version("garbage"), None);
    assert!(multilib_enabled(
        "[core]\nInclude = x\n\n[multilib]\nInclude = y\n"
    ));
    assert!(!multilib_enabled("#[multilib]\n#Include = y\n"));
    assert!(is_wolos_kernel("6.17.3-wolos1-1"));
    assert!(!is_wolos_kernel("6.17.3-arch1-1"));
    for (id, want) in [
        ("8086:46a8", Some("intel")),
        ("0x1002", Some("amd")),
        ("10DE", Some("nvidia")),
        ("0x10005", None),
    ] {
        assert_eq!(vendor_name(id), want, "{id}");
    }
}

/// A machine where everything is in place (Intel, Steam installed and bootstrapped).
fn ready_host() -> FakeHost {
    let h = FakeHost::new(false);
    for f in [
        STEAM_BIN,
        "/dev/dri/card0",
        "/usr/lib32/libvulkan.so.1",
        "/usr/lib32/libGLX_mesa.so.0",
        "/usr/bin/wine",
        "/dev/ntsync",
        "/usr/share/vulkan/icd.d/intel_icd.x86_64.json",
        "/usr/share/vulkan/icd.d/intel_icd.i686.json",
        &format!("{HOME}/.local/share/Steam/ubuntu12_32/steam"),
    ] {
        h.file(f, "");
    }
    h.file(
        PACMAN_CONF,
        "[core]\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n",
    )
    .file("/proc/sys/kernel/osrelease", "6.17.3-wolos1-1\n")
    .file("/etc/os-release", "ID=wolos\nVERSION_ID=2026.11\n")
    .command("id -Gn", "ana wheel video input audio\n")
    .command("/usr/bin/wine --version", "wine-10.0\n")
    .slow("vulkaninfo --summary", INTEL);
    h
}

fn facts(h: &FakeHost) -> Facts {
    let var = |n: &str| (n == "USER").then(|| "ana".to_string());
    gather(h, Path::new(HOME), &var, FutexWaitv::Present)
}

#[test]
fn gather_reads_a_ready_machine() {
    let f = facts(&ready_host());
    assert!(f.multilib && f.steam && f.steam_bootstrapped && f.drm);
    assert!(f.lib32_gl && f.lib32_vulkan && f.ntsync);
    assert_eq!(f.icds.len(), 2);
    assert_eq!(f.gpu_vendors, ["intel"]);
    assert_eq!(f.base.as_deref(), Some("2026.11"));
    assert_eq!(f.wine.as_deref(), Some("/usr/bin/wine"));
    let all = checks(&f);
    assert!(ready(&all), "{all:?}");
    let base = &all[0];
    assert_eq!((base.id, base.status), ("base", Status::Ok));
    assert_eq!(
        base.detail,
        "WolOS base 2026.11, kernel 6.17.3-wolos1-1 (linux-wolos)"
    );
}

#[test]
fn each_problem_names_its_fix() {
    let base = facts(&ready_host());
    type Edit = fn(&mut Facts);
    let cases: [(Edit, &str, Status, &str); 7] = [
        (
            |f| f.multilib = false,
            "multilib",
            Status::Fail,
            "/etc/pacman.conf",
        ),
        (
            |f| f.steam = false,
            "steam",
            Status::Fail,
            "sudo pacman -S steam",
        ),
        (
            |f| f.lib32_gl = false,
            "lib32-gl",
            Status::Fail,
            "lib32-mesa",
        ),
        (
            |f| f.icds.clear(),
            "vulkan-icd",
            Status::Fail,
            "lib32-vulkan-intel",
        ),
        (
            |f| f.vulkan = Some(vec![]),
            "vulkan",
            Status::Fail,
            "vulkan-intel",
        ),
        (
            |f| f.futex_waitv = FutexWaitv::Absent,
            "futex-waitv",
            Status::Warn,
            "",
        ),
        (
            |f| f.wine = None,
            "wine",
            Status::Info,
            "sudo pacman -S wine",
        ),
    ];
    for (edit, id, status, fix) in cases {
        let mut f = base.clone();
        edit(&mut f);
        let all = checks(&f);
        let c = all.iter().find(|c| c.id == id).unwrap();
        assert_eq!(c.status, status, "{id}");
        assert!(c.fix.contains(fix), "{id}: {}", c.fix);
    }
    // A kernel the person installed is reported, never a failure.
    let mut f = base.clone();
    f.kernel = "6.18.0-zen1-1-zen".into();
    let c = &checks(&f)[0];
    assert_eq!(c.status, Status::Info);
    assert!(c.detail.contains("your own kernel, kept"), "{}", c.detail);
}

#[test]
fn json_shape() {
    let c = check("steam", Status::Fail, "x", "y");
    assert_eq!(
        serde_json::to_string(&c).unwrap(),
        r#"{"id":"steam","status":"fail","detail":"x","fix":"y"}"#
    );
    let ok = check("steam", Status::Ok, "x", "");
    assert!(!serde_json::to_string(&ok).unwrap().contains("fix"));
}

/// The hand-built probe ELF is a valid Linux program: on a Linux host (>= 5.16) the kernel
/// implements futex_waitv and rejects the empty wait list with EINVAL.
#[cfg(target_os = "linux")]
#[test]
fn futex_probe_runs_on_linux() {
    let dir = std::env::temp_dir();
    assert_eq!(probe_futex_waitv(&dir), FutexWaitv::Present);
    let mut elf = futex_probe_elf();
    let at = 64 + 56 + 1;
    assert_eq!(elf[at..at + 2], 449u16.to_le_bytes());
    elf[at..at + 2].copy_from_slice(&999u16.to_le_bytes());
    assert_eq!(run_probe(&dir, &elf), FutexWaitv::Absent);
    assert_eq!(
        run_probe(&dir, b"not an executable"),
        FutexWaitv::NoLinuxAbi
    );
}

/// `wol games steam` stops only on what keeps the Steam client itself from starting.
#[test]
fn steam_blockers_are_client_checks() {
    let base = facts(&ready_host());
    assert!(steam_blockers(&checks(&base)).is_empty());
    type Edit = fn(&mut Facts);
    let cases: [(Edit, &[&str]); 4] = [
        (|f| f.multilib = false, &["multilib"]),
        (
            |f| {
                f.steam = false;
                f.lib32_vulkan = false;
            },
            &["steam", "lib32-gl"],
        ),
        // Games need these, the client does not.
        (|f| f.wine = None, &[]),
        (|f| f.icds.clear(), &[]),
    ];
    for (edit, want) in cases {
        let mut f = base.clone();
        edit(&mut f);
        let all = checks(&f);
        let got: Vec<&str> = steam_blockers(&all).iter().map(|c| c.id).collect();
        assert_eq!(got, want);
    }
    let mut f = base.clone();
    f.gpu_vendors = vec!["nvidia"];
    f.nvidia_version = Some("570.144".into());
    let all = checks(&f);
    let got: Vec<&str> = steam_blockers(&all).iter().map(|c| c.id).collect();
    assert_eq!(
        got,
        ["nvidia"],
        "the 32-bit client needs lib32-nvidia-utils"
    );
}

#[test]
fn wine_vulkan_and_groups_are_probed_concurrently() {
    let h = ready_host();
    let seq = facts(&h);
    let d = std::time::Duration::from_millis(150);
    h.command_delay(d);
    let t = std::time::Instant::now();
    assert_eq!(facts(&h), seq, "same facts");
    assert!(t.elapsed() < d * 2, "{:?} for 3 commands", t.elapsed());
}

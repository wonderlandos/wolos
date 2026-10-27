use super::*;
use crate::system::fake::FakeHost;

const INTEL: &str = include_str!("vulkaninfo-intel.txt");
const NVIDIA: &str = include_str!("vulkaninfo-nvidia-llvmpipe.txt");
const MOUNTS: &str = include_str!("mount-p.txt");
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
    let m = parse_mounts(MOUNTS);
    assert!(m.contains(&("/compat/linux/dev/shm".into(), "tmpfs".into())));
    assert_eq!(m.len(), 8);
    assert_eq!(
        parse_os_release("NAME=\"Rocky Linux\"\nID=\"rocky\"\nVERSION_ID=\"9.8\"\n"),
        Some(("rocky".into(), "9.8".into()))
    );
    assert_eq!(parse_os_release("NAME=x\n"), None);
    assert_eq!(
        parse_nvidia_version("NVIDIA UNIX x86_64 Kernel Module  595.104.02  Tue Sep 9 2026"),
        Some("595.104.02".into())
    );
    assert_eq!(parse_nvidia_version("garbage"), None);
    for (id, want) in [
        ("8086:46a8", Some("intel")),
        ("0x1002", Some("amd")),
        ("10DE", Some("nvidia")),
        ("0x10005", None),
    ] {
        assert_eq!(vendor_name(id), want, "{id}");
    }
}

/// A machine where everything is in place (Intel, wine-proton 10, Steam bootstrapped).
fn ready_host() -> FakeHost {
    let h = FakeHost::new(true);
    let tool = steam::tool_dir(Path::new(&format!("{HOME}/.steam/steam")));
    for (name, text, _) in steam::tool_files(steam::WOL_RUN) {
        h.file(&tool.join(name).to_string_lossy(), &text);
    }
    let proton = format!("{HOME}/.steam/steam/steamapps/common/Proton 10.0/files/lib/wine/dxvk");
    for f in [
        "/etc/machine-id",
        "/dev/dri/renderD128",
        "/usr/local/steam-utils/bin/steam",
        "/usr/local/steam-utils/tools/LSU_FreeBSD_Wine/run.rb",
        "/usr/local/wine-proton/bin/wine",
        "/usr/local/wine-proton/bin/wine64",
        "/usr/local/wine-proton/bin-wow64/wine",
        "/usr/local/share/vulkan/icd.d/intel_icd.x86_64.json",
        "/usr/local/bin/c6-shim-env",
        "/usr/local/bin/python3",
        &format!("{HOME}/.steam/steam/ubuntu12_32/steam"),
        &format!("{proton}/version"),
    ] {
        h.file(f, "");
    }
    h.file(
        "/compat/linux/etc/os-release",
        "NAME=\"Rocky Linux\"\nID=\"rocky\"\nVERSION_ID=\"9.8\"\n",
    );
    h.file(
        shared::hardware::SNAPSHOT_PATH,
        r#"{"devices":[{"kind":"gpu","id":"8086:46a8","name":"Iris Xe"}]}"#,
    );
    h.command("id -Gn", "ana wheel video operator\n")
        .command("mount -p", MOUNTS)
        .command("kldstat -q -m nullfs", "")
        .command("/usr/local/wine-proton/bin/wine64 --version", "wine-10.0\n")
        .slow("vulkaninfo --summary", INTEL);
    h.int("kern.features.linux", 1)
        .int("kern.features.linux64", 1)
        .int("security.bsd.unprivileged_chroot", 1)
        .int("vfs.usermount", 1)
        .int("compat.linux.use_real_ifnames", 1);
    h
}

fn facts(h: &FakeHost) -> Facts {
    let var = |k: &str| (k == "USER").then(|| "ana".to_string());
    gather(h, Path::new(HOME), &var, FutexWaitv::Present)
}

#[test]
fn gather_reads_a_ready_machine() {
    let f = facts(&ready_host());
    assert!(f.linux32 && f.linux64 && f.nullfs && f.machine_id && f.drm);
    assert!(f.steam_utils && f.steam_bootstrapped && f.tool_current);
    assert_eq!(f.wine.as_deref(), Some("/usr/local/wine-proton/bin/wine64"));
    assert_eq!(
        f.wine_version.map(|v| v.to_string()).as_deref(),
        Some("10.0")
    );
    assert_eq!(f.gpu_vendors, ["intel"]);
    assert_eq!(f.icds, ["intel_icd.x86_64.json"]);
    assert_eq!(
        f.proton_dirs,
        [format!("{HOME}/.steam/steam/steamapps/common/Proton 10.0")]
    );
    assert_eq!(f.sysctls, [Some(1), Some(1), Some(1)]);
    let all = checks(&f);
    let bad: Vec<_> = all
        .iter()
        .filter(|c| c.status > Status::Info && c.id != "win32")
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
    assert!(ready(&all));
    let win32 = all.iter().find(|c| c.id == "win32").unwrap();
    assert_eq!(
        win32.status,
        Status::Warn,
        "10.x without i386 package: bin-wow64"
    );
    let sync = all.iter().find(|c| c.id == "sync").unwrap();
    assert!(sync.detail.contains("WINEESYNC=1"));
}

#[test]
fn vendor_falls_back_to_vulkan_ids() {
    let h = ready_host();
    h.file(shared::hardware::SNAPSHOT_PATH, "{}")
        .slow("vulkaninfo --summary", NVIDIA);
    assert_eq!(facts(&h).gpu_vendors, ["nvidia"]);
}

/// One broken fact -> (check id, status, text the fix contains).
#[test]
fn each_problem_names_its_fix() {
    let base = facts(&ready_host());
    type Edit = fn(&mut Facts);
    let cases: Vec<(Edit, &str, Status, &str)> = vec![
        (
            |f| f.linux32 = false,
            "linux-abi",
            Status::Fail,
            "service linux start",
        ),
        (
            |f| f.mounts.retain(|m| m.1 != "tmpfs"),
            "linux-mounts",
            Status::Fail,
            "service linux restart",
        ),
        (
            |f| f.linux_base = None,
            "linux-base",
            Status::Fail,
            "linux_base-rl9",
        ),
        (
            |f| f.linux_base = Some(("centos".into(), "7".into())),
            "linux-base",
            Status::Fail,
            "linux_base-rl9",
        ),
        (
            |f| f.steam_utils = false,
            "steam-utils",
            Status::Fail,
            "linux-steam-utils",
        ),
        (
            |f| f.sysctls[0] = Some(0),
            "unprivileged-chroot",
            Status::Fail,
            "sysctl security.bsd.unprivileged_chroot=1 && echo security.bsd.unprivileged_chroot=1 | sudo tee -a /etc/sysctl.conf",
        ),
        (
            |f| f.sysctls[2] = None,
            "linux-ifnames",
            Status::Fail,
            "/etc/sysctl.kld.d/linux.conf",
        ),
        (
            |f| f.nullfs = false,
            "nullfs",
            Status::Fail,
            "kld_list+=nullfs",
        ),
        (
            |f| f.machine_id = false,
            "machine-id",
            Status::Fail,
            "kern.hostuuid",
        ),
        (
            |f| f.groups.retain(|g| g != "video"),
            "gpu-access",
            Status::Fail,
            "pw groupmod video -m ana",
        ),
        (
            |f| f.drm = false,
            "gpu-access",
            Status::Warn,
            "wol hw status",
        ),
        (
            |f| f.steam_bootstrapped = false,
            "steam",
            Status::Warn,
            "wol games steam",
        ),
        (
            |f| f.tool_current = false,
            "wolos-proton",
            Status::Warn,
            "wol games steam-setup",
        ),
        (
            |f| (f.wine, f.wine_version) = (None, None),
            "wine-proton",
            Status::Fail,
            "pkg install -y wine-proton",
        ),
        (
            |f| f.bin_wow64 = false,
            "win32",
            Status::Warn,
            "pkg upgrade -y wine-proton",
        ),
        (
            |f| f.icds.clear(),
            "vulkan-icd",
            Status::Fail,
            "pkg install -y mesa-dri",
        ),
        (|f| f.vulkan = None, "vulkan", Status::Warn, "vulkan-tools"),
        (
            |f| f.vulkan = Some(vec![]),
            "vulkan",
            Status::Fail,
            "wol hw status",
        ),
        (
            |f| f.vulkan = Some(parse_vulkaninfo(NVIDIA)[1..].to_vec()),
            "vulkan",
            Status::Warn,
            "wol hw status",
        ),
        (
            |f| f.proton_dirs.clear(),
            "dxvk",
            Status::Warn,
            "steam steam://install/3658110",
        ),
        (
            |f| (f.libc6_shim, f.proton_dirs) = (false, vec![]),
            "steamworks",
            Status::Info,
            "pkg install -y libc6-shim python3 && in Steam install Proton 10.0",
        ),
        (
            |f| {
                f.gpu_vendors = vec!["nvidia"];
                f.nvidia_version = Some("595.104.02".into());
            },
            "nvidia",
            Status::Fail,
            "linux-nvidia-libs",
        ),
        (
            |f| f.gpu_vendors = vec!["nvidia"],
            "nvidia",
            Status::Fail,
            "wol hw fix --yes",
        ),
        (
            |f| {
                f.gpu_vendors = vec!["nvidia"];
                f.icds.clear();
            },
            "vulkan-icd",
            Status::Fail,
            "pkg install -y nvidia-driver",
        ),
    ];
    for (edit, id, status, fix) in cases {
        let mut f = base.clone();
        edit(&mut f);
        let all = checks(&f);
        let c = all
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no check {id}"));
        assert_eq!(c.status, status, "{id}: {}", c.detail);
        assert!(c.fix.contains(fix), "{id}: fix {:?} lacks {fix:?}", c.fix);
        assert_eq!(ready(&all), status != Status::Fail, "{id}");
    }
}

#[test]
fn wine_proton_11_is_wow64_with_server_sync() {
    let mut f = facts(&ready_host());
    f.wine_version = wine::Version::parse("wine-11.0");
    let all = checks(&f);
    let get = |id: &str| all.iter().find(|c| c.id == id).unwrap().clone();
    assert_eq!(get("win32").status, Status::Ok);
    assert!(get("win32").detail.contains("WoW64"));
    assert!(get("sync").detail.contains("no esync"));
}

#[test]
fn json_shape() {
    let c = check("nullfs", Status::Fail, "x", "y");
    assert_eq!(
        serde_json::to_string(&c).unwrap(),
        r#"{"id":"nullfs","status":"fail","detail":"x","fix":"y"}"#
    );
    let ok = check("nullfs", Status::Ok, "x", "");
    assert!(!serde_json::to_string(&ok).unwrap().contains("fix"));
}

/// The hand-built probe ELF is a valid Linux program: on a Linux host (>= 5.16) the kernel
/// implements futex_waitv and rejects the empty wait list with EINVAL.
#[cfg(target_os = "linux")]
#[test]
fn futex_probe_runs_on_linux() {
    let dir = std::env::temp_dir();
    assert_eq!(probe_futex_waitv(&dir), FutexWaitv::Present);
    // Same program calling an unassigned syscall number: ENOSYS, what a stock Linux ABI returns.
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
    let cases: [(Edit, &[&str]); 5] = [
        (|f| f.nullfs = false, &["nullfs"]),
        (|f| f.machine_id = false, &["machine-id"]),
        (
            |f| {
                f.linux32 = false;
                f.sysctls[1] = Some(0);
            },
            &["linux-abi", "usermount"],
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
    f.nvidia_version = Some("595.104.02".into());
    let all = checks(&f);
    let got: Vec<&str> = steam_blockers(&all).iter().map(|c| c.id).collect();
    assert_eq!(
        got,
        ["nvidia"],
        "LSU refuses to start without linux-nvidia-libs"
    );
}

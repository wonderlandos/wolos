// SPDX-License-Identifier: BSD-2-Clause
//! System Monitor samplers on fixture trees (docs/modules/monitor.md): `fixtures/monitor/t0` and
//! `t1` are the same machine two seconds apart, laid out like `/` (proc, sys, cgroup v2,
//! the Memory Saver status). Every number below is worked out by hand from those files.
use wol_core::monitor::sampler::{Sampler, Want};
use wol_core::monitor::{Root, apps, cpu, disk, gpu, net, procs, startup, temps};

fn fixture(t: &str) -> Root {
    Root::at(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/monitor/").to_string() + t)
}

const ALL: Want = Want {
    procs: true,
    disks: true,
    pss: true,
};

/// Two samples two seconds apart: rates come from the deltas only.
fn two_samples() -> wol_core::monitor::sampler::Sample {
    let mut s = Sampler::new(fixture("t0"), Some(1000));
    let first = s.sample(10.0, ALL);
    assert!(
        first.cpu.iter().all(|c| *c == 0.0),
        "nothing to compare with yet"
    );
    s.set_root(fixture("t1"));
    s.sample(12.0, ALL)
}

#[test]
fn cpu_is_per_core_from_proc_stat() {
    let t0 = cpu::parse(&fixture("t0").read("/proc/stat").unwrap());
    let t1 = cpu::parse(&fixture("t1").read("/proc/stat").unwrap());
    assert_eq!(t0.len(), 3, "the total and two cores");
    assert_eq!(cpu::usage(&t0, &t1), vec![30.0, 50.0, 10.0]);
    assert_eq!(
        cpu::usage(&t0[..1], &t1),
        vec![0.0; 3],
        "a core came or went"
    );
    assert_eq!(two_samples().cpu, vec![30.0, 50.0, 10.0]);
}

#[test]
fn memory_and_swap_come_from_meminfo() {
    let s = two_samples();
    assert_eq!(s.memory.total, 16_000_000 * 1024);
    assert_eq!(s.memory.used, 4_000_000 * 1024, "available is not used");
    assert_eq!(
        (s.memory.swap_total, s.memory.swap_used),
        (8_000_000 * 1024, 1_000_000 * 1024)
    );
    assert_eq!(s.load.map(|l| l.one), Some(0.5));
}

#[test]
fn network_rates_skip_loopback() {
    let c = net::parse_net_dev(&fixture("t0").read("/proc/net/dev").unwrap());
    assert_eq!(
        c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["wlan0", "eth0"]
    );
    let s = two_samples();
    let wlan = &s.net[0];
    assert_eq!(
        (wlan.name.as_str(), wlan.kind, wlan.up),
        ("wlan0", net::Kind::Wireless, true)
    );
    assert_eq!((wlan.rx_rate, wlan.tx_rate), (250_000.0, 50_000.0));
    assert_eq!((wlan.rx_total, wlan.tx_total), (1_500_000, 300_000));
    let eth = &s.net[1];
    assert_eq!(
        (eth.kind, eth.up, eth.rx_rate),
        (net::Kind::Wired, false, 0.0)
    );
}

#[test]
fn disks_are_whole_devices_with_io_rates() {
    let r = fixture("t0");
    let all = disk::parse_diskstats(&r.read("/proc/diskstats").unwrap());
    assert_eq!(all.len(), 4);
    assert!(disk::is_disk(&r, "nvme0n1"));
    for not in ["nvme0n1p1", "loop0", "zram0", "sda"] {
        assert!(!disk::is_disk(&r, not), "{not}");
    }
    let s = two_samples();
    assert_eq!(s.disk_io.len(), 1);
    let d = &s.disk_io[0];
    assert_eq!(d.name, "nvme0n1");
    assert_eq!((d.read_rate, d.write_rate), (1_024_000.0, 512_000.0));
    let m = s.mounts.expect("asked for disks");
    assert_eq!(
        m.iter().map(|m| m.mount.as_str()).collect::<Vec<_>>(),
        ["/", "/boot"],
        "one row per device, no tmpfs"
    );
    assert_eq!(m[0].used, 200_000_000 * 1024);
}

#[test]
fn gpus_are_cards_not_connectors() {
    let g = gpu::read(&fixture("t1"));
    assert_eq!(g.len(), 2);
    assert_eq!(
        (g[0].card.as_str(), g[0].driver.as_str(), g[0].busy),
        ("card0", "amdgpu", Some(61.0))
    );
    assert_eq!(
        (g[0].vram_used, g[0].vram_total),
        (Some(1 << 30), Some(8 << 30))
    );
    assert_eq!((g[1].driver.as_str(), g[1].busy), ("i915", None));
}

#[test]
fn temperatures_are_named_by_chip_and_label() {
    let t = temps::read(&fixture("t0"));
    let names: Vec<_> = t.iter().map(|t| (t.name.as_str(), t.celsius)).collect();
    assert_eq!(names, [("k10temp Tctl", 54.25), ("nvme Composite", 41.85)]);
    assert_eq!(temps::hottest(&t).map(|t| t.celsius), Some(54.25));
}

#[test]
fn app_scopes_are_found_with_their_memory_and_state() {
    assert_eq!(
        apps::app_of("app-gnome-org.gnome.TextEditor-4242.scope").as_deref(),
        Some("org.gnome.TextEditor")
    );
    assert_eq!(
        apps::app_of("app-wol-code_-_oss-20.scope").as_deref(),
        Some("code_-_oss")
    );
    assert_eq!(
        apps::app_of("app-glib-foot-9.scope").as_deref(),
        Some("foot")
    );
    assert_eq!(
        apps::app_of("app-Hyprland-firefox-6d3f0a1b2c.scope").as_deref(),
        Some("firefox")
    );
    assert_eq!(
        apps::app_of("app-flatpak-org.mozilla.firefox-77.scope").as_deref(),
        Some("org.mozilla.firefox")
    );
    assert_eq!(apps::app_of("session-2.scope"), None);
    assert_eq!(apps::app_of("app-.scope"), None);

    let mine = apps::scopes(&fixture("t0"), Some(1000));
    assert_eq!(mine.len(), 2, "user 1001's foot is not ours: {mine:?}");
    let all = apps::scopes(&fixture("t0"), None);
    assert_eq!(all.len(), 3);

    let s = two_samples();
    assert_eq!(s.apps.len(), 2);
    let d = s.apps.iter().find(|a| a.app == "discord").unwrap();
    assert_eq!(
        (d.memory, d.swap, d.pss),
        (524_288_000, 104_857_600, Some(400_000 * 1024))
    );
    assert_eq!((d.cpu, d.frozen, d.rest), (0.0, true, apps::Rest::Frozen));
    let te = s
        .apps
        .iter()
        .find(|a| a.app == "org.gnome.TextEditor")
        .unwrap();
    assert_eq!(te.pids, [4242, 4243]);
    assert_eq!(
        te.pss,
        Some(150_000 * 1024),
        "PSS of every process in the scope"
    );
    assert_eq!(
        (te.cpu, te.rest),
        (25.0, apps::Rest::Running),
        "0.5 s of CPU in 2 s"
    );
    assert!(s.saver_on);
}

#[test]
fn energy_impact_follows_cpu_over_the_minute() {
    assert_eq!(apps::energy(&[]), apps::Energy::Low);
    assert_eq!(apps::energy(&[1.0, 2.0, 3.0]), apps::Energy::Low);
    assert_eq!(apps::energy(&[10.0, 12.0]), apps::Energy::Moderate);
    assert_eq!(apps::energy(&[80.0, 40.0]), apps::Energy::High);
}

#[test]
fn processes_carry_parent_cpu_and_memory() {
    let s = two_samples();
    let p = s.procs.expect("asked for processes");
    let pids: Vec<i32> = p.iter().map(|p| p.pid).collect();
    assert_eq!(
        pids,
        [1, 4242, 4243, 5000],
        "self and other names are not processes"
    );
    let te = &p[1];
    assert_eq!((te.name.as_str(), te.ppid), ("gnome-text-edit", 1));
    assert_eq!(te.cpu, 25.0, "0.5 s of CPU in 2 s");
    assert_eq!(te.rss, 30_000 * wol_core::system::proc::page_size() as u64);
    assert_eq!(p[2].name, "spell (helper)", "names may hold brackets");
    assert_eq!(p[3].state, wol_core::system::proc::ProcessState::Stopped);
}

#[test]
fn what_was_not_asked_for_is_not_read() {
    let mut s = Sampler::new(fixture("t0"), Some(1000));
    let x = s.sample(
        1.0,
        Want {
            procs: false,
            disks: false,
            pss: false,
        },
    );
    assert!(x.procs.is_none() && x.mounts.is_none());
    assert!(x.apps.iter().all(|a| a.pss.is_none()));
}

#[test]
fn an_empty_root_samples_nothing_and_does_not_panic() {
    let dir = std::env::temp_dir().join(format!("wol-monitor-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Sampler::new(Root::at(&dir), None);
    let x = s.sample(1.0, ALL);
    assert!(x.cpu.is_empty() && x.net.is_empty() && x.apps.is_empty() && x.gpus.is_empty());
    assert_eq!(x.memory.total, 0);
    assert!(!x.saver_on);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_lists_user_services_people_can_switch() {
    let files = [
        (
            "/home/ana/.config/systemd/user/syncthing.service",
            "enabled",
        ),
        ("/usr/lib/systemd/user/pipewire.service", "enabled"),
        ("/usr/lib/systemd/user/pipewire.socket", "enabled"),
        ("/usr/lib/systemd/user/dbus.service", "static"),
        ("/usr/lib/systemd/user/backup.service", "disabled"),
        ("/etc/systemd/user/masked.service", "masked"),
    ]
    .map(|(p, s)| (p.to_string(), s.to_string()));
    let rows = startup::rows(&files, "/home/ana/.config");
    let names: Vec<_> = rows
        .iter()
        .map(|u| (u.name.as_str(), u.enabled, u.mine))
        .collect();
    assert_eq!(
        names,
        [
            ("backup.service", false, false),
            ("pipewire.service", true, false),
            ("syncthing.service", true, true)
        ]
    );
    let unit = "[Unit]\nDescription=File synchronization\nAfter=network.target\n[Service]\nDescription=no\n";
    assert_eq!(
        startup::description(unit).as_deref(),
        Some("File synchronization")
    );
    assert_eq!(startup::description("[Service]\nExecStart=x\n"), None);
}

#[test]
fn a_frozen_scope_marks_its_processes_frozen() {
    let s = two_samples();
    let p = s.procs.unwrap();
    let by = |pid| p.iter().find(|x| x.pid == pid).unwrap();
    assert!(by(5000).frozen, "Discord's scope is frozen");
    assert!(!by(4242).frozen && !by(4243).frozen);
    assert!(
        !procs::in_frozen_cgroup(&fixture("t1"), 4243),
        "its cgroup says frozen 0"
    );
    // Quit targets carry the start time the sampler saw (5000 ticks at 100 Hz).
    let te = s
        .apps
        .iter()
        .find(|a| a.app == "org.gnome.TextEditor")
        .unwrap();
    let tck = wol_core::system::proc::clock_ticks();
    assert_eq!(te.targets[0].pid, 4242);
    assert_eq!(te.targets[0].start, 5000 * 1_000_000 / tck);
    assert_eq!(by(4242).start, te.targets[0].start);
}

fn fake_proc(name: &str, procs: &[(i32, i64)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wol-monitor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (pid, start) in procs {
        let d = dir.join(format!("proc/{pid}"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("stat"),
            format!("{pid} (p) S 1 0 0 0 0 0 0 0 0 0 1 1 0 0 20 0 1 0 {start} 1 1\n"),
        )
        .unwrap();
    }
    dir
}

#[test]
fn a_signal_reaches_only_the_process_the_person_saw() {
    use std::os::unix::fs::MetadataExt;
    let dir = fake_proc("signal", &[(1, 10), (300, 20), (301, 30)]);
    let root = Root::at(&dir);
    let uid = std::fs::metadata(&dir).unwrap().uid();
    let tck = wol_core::system::proc::clock_ticks();
    let us = |t: i64| t * 1_000_000 / tck;
    let t = |pid, start| procs::Target { pid, start };
    let targets = [
        t(1, us(10)),   // init: never
        t(300, us(20)), // the same process
        t(301, us(99)), // the pid was reused since
        t(302, us(1)),  // gone
        t(-5, us(1)),   // a process group, never
        t(0, us(1)),
    ];
    assert_eq!(procs::still_ours(&root, uid, &targets), (vec![300], 5));
    assert_eq!(
        procs::still_ours(&root, uid.wrapping_add(1), &targets[1..2]),
        (vec![], 1),
        "another account's process"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn only_a_unit_file_in_the_own_folder_can_be_switched() {
    let dir = std::env::temp_dir().join(format!("wol-monitor-units-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let own = dir.join("systemd/user");
    std::fs::create_dir_all(own.join("pipewire.service.d")).unwrap();
    std::fs::write(own.join("syncthing.service"), "[Unit]\n").unwrap();
    std::os::unix::fs::symlink(
        "/usr/lib/systemd/user/x.service",
        own.join("linked.service"),
    )
    .unwrap();
    assert!(startup::own_unit(&dir, "syncthing.service"));
    for no in [
        "pipewire.service",   // shipped: not in the folder
        "linked.service",     // a link to a shipped unit
        "pipewire.service.d", // a folder
        "../user/syncthing.service",
        "a@.service",
        "syncthing.socket",
        ".service",
    ] {
        assert!(!startup::own_unit(&dir, no), "{no}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_corrupt_tree_reads_as_values_or_nothing_never_a_panic() {
    let r = fixture("bad");
    for f in [
        "/proc/stat",
        "/proc/net/dev",
        "/proc/diskstats",
        "/proc/meminfo",
    ] {
        assert!(r.read(f).is_some(), "{f} is in the fixture");
    }
    let c = cpu::parse(&r.read("/proc/stat").unwrap());
    assert_eq!(c.len(), 1, "the saturated total line only");
    assert!(c[0].total >= c[0].busy && c[0].busy >= 0);
    let _ = net::parse_net_dev(&r.read("/proc/net/dev").unwrap());
    let _ = disk::parse_diskstats(&r.read("/proc/diskstats").unwrap());
    let _ = disk::mounts(&r);
    let _ = gpu::read(&r);
    assert!(temps::read(&r).iter().all(|t| t.celsius.is_finite()));
    assert_eq!(
        apps::parse_pss("Pss: 18446744073709551615 kB"),
        Some(u64::MAX)
    );
    assert!(
        apps::saver(&r).is_none(),
        "a cut-off status file is no status"
    );
    let scopes = apps::scopes(&r, Some(1000));
    assert!(
        scopes.iter().all(|s| s.pids.iter().all(|p| *p > 0)),
        "{scopes:?}"
    );
    let mut s = Sampler::new(r.clone(), Some(1000));
    for at in [1.0, 2.0, 2.0, 1.5] {
        let x = s.sample(at, ALL);
        assert!(x.cpu.iter().all(|v| v.is_finite()));
        assert!(
            x.net
                .iter()
                .all(|n| n.rx_rate.is_finite() && n.rx_rate >= 0.0)
        );
        assert!(x.apps.iter().all(|a| a.cpu.is_finite() && a.cpu >= 0.0));
    }
}

#[test]
fn a_huge_file_is_read_only_up_to_the_cap() {
    let dir = std::env::temp_dir().join(format!("wol-monitor-cap-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("proc")).unwrap();
    std::fs::write(dir.join("proc/stat"), vec![b'a'; 3 << 20]).unwrap();
    let t = Root::at(&dir).read("/proc/stat").unwrap();
    assert_eq!(t.len() as u64, wol_core::monitor::MAX_READ);
    std::fs::remove_dir_all(&dir).ok();
}

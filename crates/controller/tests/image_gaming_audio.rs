//! Wine/Proton and PipeWire defaults ship as plain config values (ported from the CachyOS and
//! Bazzite settings): kernel limits, file-descriptor limits, realtime audio, Bluetooth codecs.
use std::path::PathBuf;

fn read(p: &str) -> String {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p);
    std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn has(file: &str, lines: &[&str]) {
    let s = read(file);
    for l in lines {
        assert!(s.lines().any(|x| x.trim() == *l), "{file} lacks `{l}`");
    }
}

#[test]
fn games_get_split_lock_and_map_count_headroom() {
    has(
        "image/files/usr/lib/sysctl.d/70-wolos-performance.conf",
        &["kernel.split_lock_mitigate = 0"],
    );
    // 90-wolos.conf already sets vm.max_map_count above Proton's 1048576 minimum.
    has(
        "image/files/usr/lib/sysctl.d/90-wolos.conf",
        &["vm.max_map_count = 2147483642"],
    );
}

#[test]
fn esync_has_a_high_open_file_limit() {
    has(
        "image/files/usr/lib/systemd/system.conf.d/70-wolos-limits.conf",
        &["[Manager]", "DefaultLimitNOFILE=2048:2097152"],
    );
    has(
        "image/files/usr/lib/systemd/user.conf.d/70-wolos-limits.conf",
        &["[Manager]", "DefaultLimitNOFILE=1024:1048576"],
    );
}

#[test]
fn audio_group_may_run_realtime() {
    has(
        "image/files/etc/security/limits.d/70-wolos-audio.conf",
        &["@audio - rtprio 95", "@audio - memlock unlimited"],
    );
}

#[test]
fn pipewire_quantum_is_low_but_vm_safe() {
    let f = "image/files/usr/share/pipewire/pipewire.conf.d/70-wolos-latency.conf";
    has(
        f,
        &[
            "default.clock.rate = 48000",
            "default.clock.quantum = 1024",
            "default.clock.min-quantum = 64",
            "default.clock.max-quantum = 2048",
        ],
    );
    has(
        "image/files/usr/share/pipewire/pipewire-pulse.conf.d/70-wolos-latency.conf",
        &[
            "pulse.min.req = 64/48000",
            "pulse.min.quantum = 64/48000",
            "pulse.default.req = 512/48000",
        ],
    );
}

#[test]
fn bluetooth_prefers_high_quality_codecs() {
    let f = "image/files/usr/share/wireplumber/wireplumber.conf.d/70-wolos-bluetooth.conf";
    has(
        f,
        &[
            "bluez5.enable-sbc-xq = true",
            "bluez5.enable-msbc = true",
            "bluez5.enable-hw-volume = true",
        ],
    );
    assert!(read(f).contains("ldac") && read(f).contains("aac"));
}

#[test]
fn end_user_image_has_no_toolchain_or_gnu_docs() {
    let all = format!(
        "{}\n{}\n{}",
        read("desktop/packages.txt"),
        read("image/arch/packages.x86_64"),
        read("image/packages-base.txt")
    );
    for banned in [
        "gcc",
        "texinfo",
        "man-pages",
        "base-devel",
        "make",
        "autoconf",
    ] {
        assert!(
            !all.lines()
                .any(|l| l.split_whitespace().next() == Some(banned)),
            "{banned} ships on the image"
        );
    }
}

#[test]
fn rust_builds_use_mold_and_sccache_when_present() {
    let b = read("image/build-rust.sh");
    assert!(b.contains("command -v mold") && b.contains("-fuse-ld=mold"));
    assert!(b.contains("command -v sccache") && b.contains("RUSTC_WRAPPER"));
    let w = read(".github/workflows/packages.yml");
    assert!(w.contains("mold") && w.contains("sccache") && w.contains("out/sccache"));
    let c = read("Cargo.toml");
    assert!(c.contains("[profile.release]") && c.contains("lto = \"thin\""));
    assert!(c.contains("[profile.dev.package.\"*\"]"));
}

#[test]
fn gamemode_and_rtkit_are_installed() {
    let p = read("desktop/packages.txt");
    for want in ["gamemode", "lib32-gamemode", "rtkit"] {
        assert!(
            p.lines().any(|l| l.split_whitespace().next() == Some(want)),
            "{want}"
        );
    }
}

// SPDX-License-Identifier: GPL-2.0-or-later
//! Wine/Proton and PipeWire defaults ship as plain config values (ported from the CachyOS and
//! Bazzite settings, then the audio audit of 2026-11): kernel limits, file-descriptor limits,
//! realtime audio, the 0 dB input rule, pulse client floors, Bluetooth codecs. Every key a
//! fragment sets has one `has()` line here (docs/modules/audio.md).
#[path = "support/repo.rs"]
mod repo;
use repo::read;

fn has(file: &str, lines: &[&str]) {
    let s = read(file);
    for l in lines {
        assert!(s.lines().any(|x| x.trim() == *l), "{file} lacks `{l}`");
    }
}

const PW: &str = "files/usr/share/pipewire/pipewire.conf.d/70-wolos-latency.conf";
const PULSE: &str = "files/usr/share/pipewire/pipewire-pulse.conf.d/70-wolos-latency.conf";
const BT: &str = "files/usr/share/wireplumber/wireplumber.conf.d/70-wolos-bluetooth.conf";
const INPUT: &str = "files/usr/share/wireplumber/wireplumber.conf.d/71-wolos-input.conf";
const ALSA: &str = "files/usr/share/wireplumber/wireplumber.conf.d/72-wolos-alsa.conf";
const ALL: [&str; 6] = [
    PW,
    PULSE,
    "files/usr/share/pipewire/pipewire.conf.d/71-wolos-airplay.conf",
    BT,
    INPUT,
    ALSA,
];

#[test]
fn games_get_split_lock_and_map_count_headroom() {
    has(
        "files/usr/lib/sysctl.d/70-wolos-performance.conf",
        &["kernel.split_lock_mitigate = 0"],
    );
    // 90-wolos.conf already sets vm.max_map_count above Proton's 1048576 minimum.
    has(
        "files/usr/lib/sysctl.d/90-wolos.conf",
        &["vm.max_map_count = 2147483642"],
    );
}

#[test]
fn esync_has_a_high_open_file_limit() {
    has(
        "files/usr/lib/systemd/system.conf.d/70-wolos-limits.conf",
        &["[Manager]", "DefaultLimitNOFILE=2048:2097152"],
    );
    has(
        "files/usr/lib/systemd/user.conf.d/70-wolos-limits.conf",
        &["[Manager]", "DefaultLimitNOFILE=1024:1048576"],
    );
}

#[test]
fn audio_group_may_run_realtime() {
    has(
        "files/etc/security/limits.d/70-wolos-audio.conf",
        &["@audio - rtprio 95", "@audio - memlock unlimited"],
    );
}

/// The graph: 48 kHz default but 44.1/88.2/96 followed instead of resampled; 64-frame floor for
/// games, 1024 default, 2048 ceiling.
#[test]
fn pipewire_quantum_is_low_but_vm_safe() {
    has(
        PW,
        &[
            "default.clock.rate = 48000",
            "default.clock.allowed-rates = [ 44100 48000 88200 96000 ]",
            "default.clock.quantum = 1024",
            "default.clock.min-quantum = 64",
            "default.clock.max-quantum = 2048",
        ],
    );
}

/// PulseAudio clients: 256 frames for everyone (a browser cannot drag the graph to 64), 64 only
/// for Wine/Proton, Steam and gamescope, 1024 inside a VM. The old 64-for-all is gone.
#[test]
fn pulse_floor_is_256_and_only_games_get_64() {
    has(
        PULSE,
        &[
            "pulse.min.req = 256/48000",
            "pulse.min.quantum = 256/48000",
            "pulse.default.req = 512/48000",
            "pulse.min.req = 64/48000",
            "pulse.min.quantum = 64/48000",
            "pulse.min.req = 1024/48000",
            "pulse.min.quantum = 1024/48000",
            "{ cpu.vm.name = !null }",
            "{ application.process.binary = \"~wine.*\" }",
            "{ application.process.binary = \"~steam.*\" }",
            "{ application.process.binary = \"~gamescope.*\" }",
            "pulse.properties.rules = [",
            "pulse.rules = [",
        ],
    );
    let t = read(PULSE);
    let props = t.split("pulse.properties.rules").next().unwrap();
    assert!(
        !props.contains("64/48000"),
        "the global pulse.properties block must not go below 256"
    );
}

/// The 0 dB rule's config half: new microphones start at 40% (0.4^3 on WirePlumber's cubic
/// scale, the same start sinks get), not at 100% = hardware maximum.
#[test]
fn microphones_start_at_forty_percent_not_hardware_max() {
    has(
        INPUT,
        &[
            "wireplumber.settings = {",
            "device.routes.default-source-volume = 0.064",
            "bluetooth.autoswitch-to-headset-profile = true",
            "bluetooth.profile-preference = \"quality\"",
        ],
    );
}

/// The hardware mixer stays in charge (USB preamp gain is only there), dB tables are trusted
/// (0 dB is a real slider position), devices sleep after 5 s. Per-device overrides are the
/// user's, in ~/.config, never here.
#[test]
fn alsa_keeps_the_hardware_mixer_and_the_db_table() {
    has(
        ALSA,
        &[
            "monitor.alsa.rules = [",
            "{ device.name = \"~alsa_card.*\" }",
            "api.alsa.soft-mixer = false",
            "api.alsa.ignore-dB = false",
            "{ node.name = \"~alsa_input.*\" }",
            "{ node.name = \"~alsa_output.*\" }",
            "session.suspend-timeout-seconds = 5",
        ],
    );
}

#[test]
fn bluetooth_prefers_high_quality_codecs() {
    has(
        BT,
        &[
            "bluez5.enable-sbc-xq = true",
            "bluez5.enable-msbc = true",
            "bluez5.enable-hw-volume = true",
            "bluez5.hfphsp-backend = \"native\"",
            "monitor.bluez.rules = [",
            "{ device.name = \"~bluez_card.*\" }",
            "bluez5.a2dp.ldac.quality = \"auto\"",
        ],
    );
    assert!(read(BT).contains("ldac") && read(BT).contains("aac"));
}

/// Echo cancel, RNNoise, loopbacks and the App Mix are pipewire-pulse modules the audio service
/// loads on demand (`wol_core::audio::*_argv`), never static graph config.
#[test]
fn nothing_loads_filters_or_loopbacks_statically() {
    for f in ALL {
        let t = read(f);
        for banned in [
            "module-echo-cancel",
            "module-loopback",
            "module-filter-chain",
            "module-null-sink",
            "module-combine",
            "rnnoise",
        ] {
            assert!(!t.contains(banned), "{f} loads {banned} statically");
        }
    }
}

/// Every WolOS fragment is a `.conf` under the right conf.d, sorted after the distribution's
/// (70+), with a first-line comment saying why.
#[test]
fn fragments_are_numbered_and_explained() {
    for f in ALL {
        let name = f.rsplit('/').next().unwrap();
        assert!(name.starts_with("7") && name.ends_with(".conf"), "{f}");
        assert!(
            read(f).starts_with("# "),
            "{f}: first line explains the fragment"
        );
    }
}

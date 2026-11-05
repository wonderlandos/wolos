// SPDX-License-Identifier: GPL-2.0-or-later
//! ADR 0038 names `wol volume`, ADR 0032 names `wol audio profile low-latency`: both are verbs
//! (QA-ADR-22, QA-ADR-24), and the docs that name them keep naming verbs the tree has.
use controller::cli::{TREE, parse};
use controller::run::{FakeEnv, execute};
use std::sync::Arc;

fn temp_home(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wol-audio-verbs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    env.out.clear();
    env.err.clear();
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn host() -> Arc<wol_core::system::fake::FakeHost> {
    let host = Arc::new(wol_core::system::fake::FakeHost::new(false));
    host.command("wpctl set-volume @DEFAULT_AUDIO_SINK@ 40%", "")
        .command("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.40\n");
    host
}

#[test]
fn wol_volume_sets_like_wol_audio_volume() {
    let mut e = FakeEnv {
        home: temp_home("set"),
        host: Some(host()),
        ..Default::default()
    };
    assert_eq!(run("volume 40 --json", &mut e), 0, "{}", e.err);
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    assert_eq!(v["volume"], 40);
    assert_eq!(run("volume 101", &mut e), 64, "the same 0..100 check");
    assert_eq!(run("volume loud", &mut e), 64);
}

#[test]
fn wol_volume_alone_reads_and_changes_nothing() {
    let h = host();
    let mut e = FakeEnv {
        home: temp_home("read"),
        host: Some(h),
        ..Default::default()
    };
    assert_eq!(run("volume --json", &mut e), 0, "{}", e.err);
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    assert_eq!(v["volume"], 40);
    // `wol audio volume` keeps asking for the number.
    assert!(parse(&["audio", "volume"]).is_err());
}

#[test]
fn both_verbs_are_in_the_tree_and_the_profile_is_in_the_help() {
    assert!(
        TREE.iter()
            .any(|c| c.path == ["volume"] && c.action == "audio.volume")
    );
    let p = TREE
        .iter()
        .find(|c| c.path == ["audio", "profile"])
        .expect("wol audio profile");
    assert!(!p.hidden && !p.help.is_empty());
    assert_eq!(p.args[0].choices, ["default", "low-latency", "status"]);
    assert!(controller::cli::help(&["audio".into()]).contains("audio profile"));
    for line in [
        "audio profile low-latency",
        "audio profile default",
        "audio profile",
    ] {
        let argv: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parse(&argv).unwrap().action, "audio.profile", "{line}");
    }
}

#[test]
fn the_profile_verb_runs_through_the_dispatcher() {
    let mut e = FakeEnv {
        home: temp_home("profile"),
        ..Default::default()
    };
    assert_eq!(run("audio profile low-latency", &mut e), 0, "{}", e.err);
    assert!(e.out.contains("low-latency"), "{}", e.out);
    assert!(
        e.execs
            .iter()
            .any(|a| a.join(" ") == "pw-metadata -n settings 0 clock.force-quantum 128"),
        "{:?}",
        e.execs
    );
}

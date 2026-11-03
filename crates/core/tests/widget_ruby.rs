// SPDX-License-Identifier: BSD-2-Clause
//! A widget's command source may be a Ruby script exactly as it may be a shell script (ADR 0041):
//! `widgets::run_limited` starts the argv it is given, never a shell, so the same limits hold for
//! both: the JSON it prints comes back, a script that runs too long is stopped at the timeout, and
//! output past the cap is refused. Each case runs for sh and for ruby; a missing interpreter skips
//! its cases with a message.
use std::path::Path;
use std::time::{Duration, Instant};
use wol_core::widgets::{SourceError, run_limited};

/// (interpreter, prints JSON, never ends, prints too much).
const KINDS: &[(&str, &str, &str, &str)] = &[
    (
        "/bin/sh",
        "printf '{\"value\": %s}\\n' 42\n",
        "while :; do :; done\n",
        "i=0; while [ $i -lt 2000 ]; do printf 'xxxxxxxxxx'; i=$((i+1)); done\n",
    ),
    (
        "/usr/bin/ruby",
        "puts '{\"value\": 42}'\n",
        "loop { }\n",
        "print 'x' * 20_000\n",
    ),
];

fn script(dir: &Path, name: &str, interp: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, format!("#!{interp}\n{body}")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn a_ruby_source_meets_the_same_limits_as_a_shell_source() {
    for (interp, json, forever, flood) in KINDS {
        if !Path::new(interp).exists() {
            eprintln!("{interp} not installed; its widget cases skipped");
            continue;
        }
        let tag = interp.rsplit('/').next().unwrap();
        let dir = std::env::temp_dir().join(format!("wol-widget-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = script(&dir, "ok", interp, json);
        let out = run_limited(&[ok], &dir, Duration::from_secs(10), 4096).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["value"], 42, "{tag}");

        let slow = script(&dir, "slow", interp, forever);
        let t = Instant::now();
        let r = run_limited(&[slow], &dir, Duration::from_millis(500), 4096);
        assert!(matches!(r, Err(SourceError::Timeout(_))), "{tag}: {r:?}");
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "{tag}: stopped at the timeout"
        );

        let big = script(&dir, "big", interp, flood);
        let r = run_limited(&[big], &dir, Duration::from_secs(10), 1024);
        assert!(matches!(r, Err(SourceError::TooLarge)), "{tag}: {r:?}");
    }
}

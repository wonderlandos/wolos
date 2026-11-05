// SPDX-License-Identifier: GPL-2.0-or-later
//! The session transcripts of `crates/shared/tests/fixtures/atlas/sessions/`, played against a real
//! bridge with a fake wold behind it. Lines: `> ` the browser sends, `< ` the bridge must answer
//! next, `closed` the bridge must close; `file`/`dir` make entries in the scratch directory (`@DIR`),
//! `wold <call> <answer>` scripts the fake wold (`wold down`: no wold at all), `opener ok|fail` the
//! fallback opener, `asked <json>` the calls wold must have had (in order), `opened <path>` what the
//! opener must have been given. `@PID` is this process (the fake browser).
mod support;
use std::path::Path;
use std::time::Duration;
use support::{Browser, FakeWold, bridge, me, opener, scratch};

fn play(name: &str) {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../shared/tests/fixtures/atlas/sessions")
            .join(name),
    )
    .unwrap();
    let dir = scratch(name.trim_end_matches(".txt"));
    let files = dir.join("files");
    std::fs::create_dir(&files).unwrap();
    let at = files.to_string_lossy().into_owned();
    let sub = |l: &str| {
        l.replace("@DIR", &at)
            .replace("@PID", &std::process::id().to_string())
    };
    let wold = FakeWold::default();
    let wold_sock = dir.join("wold.sock");
    let mut down = false;
    let log = dir.join("opened");
    let mut open = opener(&dir, &log, 1);
    let (mut asked, mut opened) = (Vec::new(), Vec::new());
    let mut steps = Vec::new();
    for l in text.lines() {
        let (word, rest) = l.split_once(' ').unwrap_or((l, ""));
        match word {
            "#" | "" => {}
            "file" => std::fs::write(files.join(rest), b"x").unwrap(),
            "dir" => std::fs::create_dir(files.join(rest)).unwrap(),
            "wold" if rest == "down" => down = true,
            "wold" => {
                let (call, answer) = rest.split_once(' ').unwrap();
                wold.answer(call, answer);
            }
            "opener" => open = opener(&dir, &log, if rest == "ok" { 0 } else { 1 }),
            "asked" => asked.push(sub(rest)),
            "opened" => opened.push(sub(rest)),
            ">" | "<" | "closed" => steps.push((word.to_owned(), sub(rest))),
            _ => panic!("{name}: unknown line {l}"),
        }
    }
    if !down {
        wold.serve(&wold_sock);
    }
    let sock = bridge(&dir, &wold_sock, me(), open, Duration::from_secs(60));
    let mut b = Browser::connect(&sock);
    for (word, line) in steps {
        match word.as_str() {
            ">" => b.send(&line),
            "<" => assert_eq!(b.read(), Some(line), "{name}"),
            _ => assert_eq!(b.read(), None, "{name}: the bridge closes"),
        }
    }
    assert_eq!(
        *wold.asked.lock().unwrap(),
        asked,
        "{name}: what wold was asked"
    );
    let got = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(
        got.lines().collect::<Vec<_>>(),
        opened,
        "{name}: what was opened"
    );
}

#[test]
fn every_session_is_played() {
    let mut names: Vec<String> = std::fs::read_dir(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/tests/fixtures/atlas/sessions"),
    )
    .unwrap()
    .flatten()
    .map(|e| e.file_name().to_string_lossy().into_owned())
    .collect();
    names.sort();
    assert!(names.len() >= 12, "{names:?}");
    for n in names {
        play(&n);
    }
}

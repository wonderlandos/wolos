// SPDX-License-Identifier: BSD-2-Clause
//! QA-A9-SEC-8: a name from outside is opened with `O_PATH` first and read-opened only when the
//! descriptor is a regular file (`shared::sys::open_regular`). A read-open of a device node has
//! driver side effects (a watchdog arms, a tape or tty-class node acts); a FIFO read-open is an
//! event for its writer. The strace test watches the system calls themselves.
use shared::sys::{open_named, open_path_regular, open_regular};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-open-regular-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fifo(dir: &Path) -> PathBuf {
    let p = dir.join("fifo");
    assert!(Command::new("mkfifo").arg(&p).status().unwrap().success());
    p
}

#[test]
fn a_regular_file_is_opened_for_reading() {
    let d = scratch("regular");
    let f = d.join("pkg");
    std::fs::write(&f, b"data").unwrap();
    let mut text = String::new();
    open_regular(&f).unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "data");
    // The anchor names the inode and cannot be read.
    let mut anchor = open_path_regular(&f).unwrap();
    assert!(
        anchor.read(&mut [0u8; 1]).is_err(),
        "an O_PATH descriptor reads nothing"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_fifo_a_device_a_directory_and_a_link_are_refused() {
    let d = scratch("refused");
    let link = d.join("link");
    std::fs::write(d.join("real"), b"x").unwrap();
    std::os::unix::fs::symlink(d.join("real"), &link).unwrap();
    for (what, p) in [
        ("fifo", fifo(&d)),
        ("char device", PathBuf::from("/dev/null")),
        ("directory", d.clone()),
        ("link", link),
    ] {
        let e = open_regular(&p).expect_err(what);
        assert!(
            e.kind() == std::io::ErrorKind::InvalidInput || e.raw_os_error().is_some(),
            "{what}: {e}"
        );
        assert!(open_path_regular(&p).is_err(), "{what}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Child half of [`no_read_open_happens_for_a_node_that_is_not_a_file`]: opens the path named by
/// `WOL_OPEN_PROBE` (`regular:PATH` or `named:PATH`); does nothing without it.
#[test]
fn the_child_probe_opens_the_named_path() {
    let Ok(spec) = std::env::var("WOL_OPEN_PROBE") else {
        return;
    };
    let (how, path) = spec.split_once(':').expect("how:path");
    let _ = if how == "regular" {
        open_regular(path).map(drop)
    } else {
        open_named(path).map(drop)
    };
}

/// The openat calls a child made for `path`, with strace.
fn opens_of(how: &str, path: &Path, log: &Path) -> Option<Vec<String>> {
    let st = Command::new("strace")
        .args(["-f", "-e", "trace=open,openat,openat2", "-o"])
        .arg(log)
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "the_child_probe_opens_the_named_path",
            "--test-threads=1",
        ])
        .env("WOL_OPEN_PROBE", format!("{how}:{}", path.display()))
        .output()
        .ok()?;
    st.status.success().then_some(())?;
    let text = std::fs::read_to_string(log).ok()?;
    let want = path.to_string_lossy();
    Some(
        text.lines()
            .filter(|l| l.contains(want.as_ref()))
            .map(str::to_owned)
            .collect(),
    )
}

#[test]
fn no_read_open_happens_for_a_node_that_is_not_a_file() {
    let d = scratch("strace");
    let log = d.join("strace.log");
    let node = fifo(&d);
    // The old door read-opens the FIFO: the probe sees it.
    let Some(old) = opens_of("named", &node, &log) else {
        eprintln!("skipped: strace is not usable here");
        return;
    };
    assert!(
        old.iter()
            .any(|l| !l.contains("O_PATH") && l.contains("O_RDONLY")),
        "open_named read-opens a FIFO: {old:?}"
    );
    // The new door only ever opens it with O_PATH.
    let new = opens_of("regular", &node, &log).expect("strace worked a moment ago");
    assert!(!new.is_empty(), "the probe opened the name");
    assert!(
        new.iter().all(|l| l.contains("O_PATH")),
        "open_regular read-opened a FIFO: {new:?}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The class: four root-side readers (wold's `room` and `stage`, wol-admind's `copy_in`, the guard's
/// `read_user_dirs`) took a caller's name through `open_named`, which read-opens any node.
#[test]
fn a_name_from_outside_is_opened_through_open_regular() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for e in std::fs::read_dir(&crates).expect("crates/").flatten() {
        rs_files(&e.path().join("src"), &mut files);
    }
    assert!(files.len() > 100, "found the sources");
    let mut bad = Vec::new();
    for f in &files {
        if f.ends_with("shared/src/sys/mod.rs") {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        for (n, l) in code.lines().enumerate() {
            if !l.trim_start().starts_with("//") && l.contains("open_named(") {
                bad.push(format!("{}:{}: {}", f.display(), n + 1, l.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "open a name that someone else gave with shared::sys::open_regular (O_PATH, fstat, then \
         the read-open):\n{}",
        bad.join("\n")
    );
}

// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol` reads the Give me more time setting (QA-ACC-6) the way wold does: a small regular file the
//! account owns, opened without following a link or waiting on a pipe. A FIFO, a link or a big
//! file at that path is "normal", and never a `wol` that hangs on a helper call.
use controller::run::{Env, RealEnv};
use shared::patience::Patience;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn a_pipe_or_a_link_in_place_of_the_file_is_normal_and_never_hangs() {
    let d = std::env::temp_dir().join(format!("wol-patience-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("wol")).unwrap();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &d) };
    let file = d.join(shared::patience::FILE);
    // Asked from a thread: a read that waits on the pipe would end this test by timeout, not hang it.
    let ask = || {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(RealEnv.patience());
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("patience() waited on the file")
    };
    assert_eq!(ask(), Patience::Normal, "no file");
    std::fs::write(&file, "5\n").unwrap();
    assert_eq!(ask(), Patience::Five, "a small file of the account");
    std::fs::remove_file(&file).unwrap();
    let c = std::ffi::CString::new(file.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert_eq!(ask(), Patience::Normal, "a pipe");
    std::fs::remove_file(&file).unwrap();
    let target = d.join("target");
    std::fs::write(&target, "never\n").unwrap();
    std::os::unix::fs::symlink(&target, &file).unwrap();
    assert_eq!(ask(), Patience::Normal, "a link");
    std::fs::remove_file(&file).unwrap();
    std::fs::write(&file, "5".repeat(64)).unwrap();
    assert_eq!(ask(), Patience::Normal, "a big file");
    let _ = std::fs::remove_dir_all(&d);
}

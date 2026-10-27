//! Proton (ntdll "handle for fd N is seekable": closes it) drops std handles that are regular files, so
//! `wol-run app.wol > log` printed nothing. Proton plans relay seekable stdout/stderr through pipes.
use std::collections::BTreeMap;

fn plan(relay: bool) -> proxy::LaunchPlan {
    proxy::LaunchPlan {
        // This test binary re-run as the probe child (`probe` below): portable, no /dev/stdout checks.
        argv: vec![
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--exact".into(),
            "probe".into(),
            "--nocapture".into(),
        ],
        env: BTreeMap::from([("WOL_RELAY_PROBE".into(), "1".into())]),
        relay_seekable_stdio: relay,
        ..Default::default()
    }
}

fn run(relay: bool) -> (i32, String, String) {
    let d = std::env::temp_dir().join(format!("wol-relay-{}-{relay}", std::process::id()));
    let (o, e) = (d.with_extension("out"), d.with_extension("err"));
    let code = proxy::run(
        &plan(relay),
        std::fs::File::create(&o).unwrap().into(),
        std::fs::File::create(&e).unwrap().into(),
    )
    .unwrap();
    let r = (
        code,
        std::fs::read_to_string(&o).unwrap(),
        std::fs::read_to_string(&e).unwrap(),
    );
    let _ = (std::fs::remove_file(o), std::fs::remove_file(e));
    r
}

/// Child side: report whether fd 1/2 are seekable (what Proton's ntdll checks), exit 7.
#[test]
fn probe() {
    if std::env::var_os("WOL_RELAY_PROBE").is_none() {
        return;
    }
    for (fd, name) in [(1, "out"), (2, "err")] {
        // SAFETY: lseek/write on the process's own std descriptors.
        let kind = if unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) } == -1 {
            "piped"
        } else {
            "file"
        };
        let m = format!("\n{name}-{kind}\n");
        unsafe { libc::write(fd, m.as_ptr().cast(), m.len()) };
    }
    std::process::exit(7);
}

#[test]
fn proton_plans_see_pipes_and_output_reaches_the_file() {
    let (code, out, err) = run(true);
    assert_eq!(code, 7);
    assert!(out.contains("\nout-piped\n"), "{out}");
    assert!(err.contains("\nerr-piped\n"), "{err}");
}

#[test]
fn other_plans_keep_the_file() {
    let (code, out, err) = run(false);
    assert_eq!(code, 7);
    assert!(out.contains("\nout-file\n"), "{out}");
    assert!(err.contains("\nerr-file\n"), "{err}");
}

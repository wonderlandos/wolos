//! rc.d scripts that supervise with `daemon -P <pidfile>`: the pidfile holds the daemon(8) pid, whose
//! title is `daemon: <prog>[pid]`. rc.subr only matches that title against procname `daemon`, so a
//! procname of the child binary made `service wold status|stop|restart` report "not running" and do
//! nothing (installed disk: wold could not be restarted).
use std::path::Path;

#[test]
fn supervised_rc_scripts_keep_the_daemon_procname() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0;
    for dir in [
        "image/files/usr/local/etc/rc.d",
        "image/files/etc/rc.d",
        "desktop/overlay/usr/local/etc/rc.d",
    ] {
        let Ok(rd) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for f in rd.flatten() {
            let s = std::fs::read_to_string(f.path()).unwrap_or_default();
            if !(s.contains("command=/usr/sbin/daemon") && s.contains("-P ")) {
                continue;
            }
            seen += 1;
            let procname = s.lines().find_map(|l| l.trim().strip_prefix("procname="));
            assert!(
                procname.is_none_or(|p| p.trim_matches('"').ends_with("daemon")),
                "{}: procname={procname:?} never matches the daemon(8) pid in the -P pidfile",
                f.path().display()
            );
        }
    }
    assert!(seen > 0, "no daemon -P rc.d script found");
}

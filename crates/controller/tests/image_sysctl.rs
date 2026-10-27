//! No core files in home folders: a crashing session compositor left a 27 MB `Hyprland.core` in the
//! new user's home (installed disk, Setup Assistant -> first session). Crash details go to logs instead.
#[test]
fn user_core_dumps_are_off() {
    let f = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../desktop/overlay/etc/sysctl.conf.local");
    let s = std::fs::read_to_string(&f).unwrap_or_default();
    assert!(
        s.lines()
            .any(|l| l.split('#').next().unwrap().trim() == "kern.coredump=0"),
        "{} must set kern.coredump=0",
        f.display()
    );
}

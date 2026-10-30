//! glibc malloc keeps 64 chunks per thread-cache bin (glibc default: 7) for every service and
//! user session: +26% on a 16-thread request/JSON workload at the same RSS. hugetlb stays off
//! (it raised RSS 4-12x for no gain).
use std::path::PathBuf;

fn read(p: &str) -> String {
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p);
    std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("{p}: {e}"))
}

const TUNABLE: &str = "DefaultEnvironment=GLIBC_TUNABLES=glibc.malloc.tcache_count=64";

#[test]
fn services_and_sessions_get_a_bigger_malloc_tcache() {
    for f in [
        "image/files/usr/lib/systemd/system.conf.d/70-wolos-malloc.conf",
        "image/files/usr/lib/systemd/user.conf.d/70-wolos-malloc.conf",
    ] {
        let s = read(f);
        assert!(s.lines().any(|l| l.trim() == "[Manager]"), "{f} lacks [Manager]");
        assert!(s.lines().any(|l| l.trim() == TUNABLE), "{f} lacks `{TUNABLE}`");
        assert!(!s.contains("hugetlb"), "{f}: hugetlb raised RSS for no gain");
    }
}

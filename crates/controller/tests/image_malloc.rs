// SPDX-License-Identifier: GPL-2.0-or-later
//! glibc malloc keeps 64 chunks per thread-cache bin (glibc default: 7) for every service and
//! user session: +26% on a 16-thread request/JSON workload at the same RSS. hugetlb stays off
//! (it raised RSS 4-12x for no gain).
#[path = "support/repo.rs"]
mod repo;
use repo::read;

const TUNABLE: &str = "DefaultEnvironment=GLIBC_TUNABLES=glibc.malloc.tcache_count=64";

#[test]
fn services_and_sessions_get_a_bigger_malloc_tcache() {
    for f in [
        "files/usr/lib/systemd/system.conf.d/70-wolos-malloc.conf",
        "files/usr/lib/systemd/user.conf.d/70-wolos-malloc.conf",
    ] {
        let s = read(f);
        assert!(
            s.lines().any(|l| l.trim() == "[Manager]"),
            "{f} lacks [Manager]"
        );
        assert!(
            s.lines().any(|l| l.trim() == TUNABLE),
            "{f} lacks `{TUNABLE}`"
        );
        assert!(
            !s.contains("hugetlb"),
            "{f}: hugetlb raised RSS for no gain"
        );
    }
}

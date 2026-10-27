//! Index-range fork/join (Iara.Par `For`/`Reduce`, ADR 0017 in IaraOS): split only by grain, so the
//! combine tree is fixed and results are deterministic. Cancellation skips remaining leaves.
use super::{fork_budget, join};
use std::sync::atomic::{AtomicBool, Ordering};

/// Range operation was cancelled before every leaf ran.
#[derive(Debug, PartialEq, Eq)]
pub struct Cancelled;

/// Call `body(from, to)` on disjoint chunks covering `range`, in parallel. Each index visited once.
pub fn for_range<B>(
    range: std::ops::Range<usize>,
    grain: usize,
    cancel: &AtomicBool,
    body: B,
) -> Result<(), Cancelled>
where
    B: Fn(usize, usize) + Sync,
{
    reduce_range(range, grain, cancel, || (), body, |(), ()| ())
}

/// Parallel reduction over `range`: `leaf(from, to)` per chunk, `combine(left, right)` in index order.
pub fn reduce_range<T, I, L, C>(
    range: std::ops::Range<usize>,
    grain: usize,
    cancel: &AtomicBool,
    identity: I,
    leaf: L,
    combine: C,
) -> Result<T, Cancelled>
where
    T: Send,
    I: Fn() -> T + Sync,
    L: Fn(usize, usize) -> T + Sync,
    C: Fn(T, T) -> T + Sync,
{
    #[allow(clippy::too_many_arguments)] // recursion state; a struct adds noise
    fn go<T: Send>(
        from: usize,
        to: usize,
        grain: usize,
        budget: u32,
        cancel: &AtomicBool,
        identity: &(dyn Fn() -> T + Sync),
        leaf: &(dyn Fn(usize, usize) -> T + Sync),
        combine: &(dyn Fn(T, T) -> T + Sync),
    ) -> T {
        if cancel.load(Ordering::Relaxed) {
            return identity();
        }
        if to - from <= grain || budget == 0 {
            // Without fork budget, still split by grain sequentially: the tree stays fixed.
            if to - from <= grain {
                return leaf(from, to);
            }
        }
        let mid = from + (to - from) / 2;
        if budget == 0 {
            let a = go(from, mid, grain, 0, cancel, identity, leaf, combine);
            let b = go(mid, to, grain, 0, cancel, identity, leaf, combine);
            return combine(a, b);
        }
        let (a, b) = join(
            || {
                go(
                    from,
                    mid,
                    grain,
                    budget - 1,
                    cancel,
                    identity,
                    leaf,
                    combine,
                )
            },
            || go(mid, to, grain, budget - 1, cancel, identity, leaf, combine),
        );
        combine(a, b)
    }
    if range.is_empty() {
        return Ok(identity());
    }
    let r = go(
        range.start,
        range.end,
        grain.max(1),
        fork_budget(),
        cancel,
        &identity,
        &leaf,
        &combine,
    );
    if cancel.load(Ordering::Relaxed) {
        Err(Cancelled)
    } else {
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn for_visits_each_index_once() {
        for (n, grain) in [
            (0, 1),
            (1, 1),
            (7, 3),
            (1000, 1),
            (1000, 64),
            (100_000, 4096),
        ] {
            let hits: Vec<AtomicUsize> = (0..n).map(|_| AtomicUsize::new(0)).collect();
            let no = AtomicBool::new(false);
            for_range(0..n, grain, &no, |a, b| {
                assert!(b - a <= grain.max(1));
                for h in &hits[a..b] {
                    h.fetch_add(1, Ordering::Relaxed);
                }
            })
            .unwrap();
            assert!(
                hits.iter().all(|h| h.load(Ordering::Relaxed) == 1),
                "n={n} grain={grain}"
            );
        }
    }

    #[test]
    fn reduce_equals_sequential_and_keeps_order() {
        let no = AtomicBool::new(false);
        let sum = reduce_range(
            0..100_000,
            1000,
            &no,
            || 0u64,
            |a, b| (a..b).map(|i| i as u64).sum(),
            |x, y| x + y,
        );
        assert_eq!(sum, Ok((0..100_000u64).sum()));
        let cat = reduce_range(
            0..50,
            3,
            &no,
            String::new,
            |a, b| (a..b).map(|i| format!("{i},")).collect(),
            |x, y| x + &y,
        );
        assert_eq!(
            cat.unwrap(),
            (0..50).map(|i| format!("{i},")).collect::<String>()
        );
    }

    #[test]
    fn cancellation_stops_and_reports() {
        let cancel = AtomicBool::new(false);
        let ran = AtomicUsize::new(0);
        let r = for_range(0..1_000_000, 1, &cancel, |_, _| {
            if ran.fetch_add(1, Ordering::Relaxed) == 10 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert_eq!(r, Err(Cancelled));
        assert!(ran.load(Ordering::Relaxed) < 1_000_000);
    }

    #[test]
    fn panic_propagates_after_both_halves() {
        let no = AtomicBool::new(false);
        let r =
            std::panic::catch_unwind(|| for_range(0..100, 1, &no, |a, _| assert!(a != 42, "boom")));
        assert!(r.is_err());
        // pool-less design: later calls still work
        assert!(for_range(0..10, 1, &no, |_, _| ()).is_ok());
    }

    #[test]
    fn many_external_threads_share_it() {
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    let no = AtomicBool::new(false);
                    let v =
                        reduce_range(0..10_000, 256, &no, || 0usize, |a, b| b - a, |x, y| x + y);
                    assert_eq!(v, Ok(10_000));
                });
            }
        });
    }
}

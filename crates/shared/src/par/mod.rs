//! Fork/join parallelism in Bend's style (ADR 0014): write divide and conquer; independent halves run in parallel.

mod range;
pub use range::{Cancelled, for_range, reduce_range};

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

/// Run `a` and `b` in parallel; return both results. A panic in either propagates to the caller.
pub fn join<A, B, RA, RB>(a: A, b: B) -> (RA, RB)
where
    A: FnOnce() -> RA + Send,
    B: FnOnce() -> RB + Send,
    RA: Send,
    RB: Send,
{
    std::thread::scope(|s| {
        let right = s.spawn(b);
        let left = catch_unwind(AssertUnwindSafe(a));
        let right = right.join();
        match (left, right) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(p), _) | (_, Err(p)) => resume_unwind(p),
        }
    })
}

/// Fork depth that yields about two tasks per core: deeper forks only add thread overhead.
fn fork_budget() -> u32 {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
    u32::BITS - cores.leading_zeros() + 1
}

/// Divide and conquer, Bend-style: `split` halves the input, `leaf` solves pieces of at most `grain`
/// items (or bigger ones once every core is busy), `combine` merges left then right. Independent halves
/// run in parallel; the result equals running the same recursion sequentially.
pub fn fold<'a, T, R, S, L, C>(input: &'a [T], grain: usize, split: S, leaf: L, combine: C) -> R
where
    T: Sync,
    R: Send,
    S: Fn(&'a [T]) -> (&'a [T], &'a [T]) + Sync,
    L: Fn(&'a [T]) -> R + Sync,
    C: Fn(R, R) -> R + Sync,
{
    fn go<'a, T: Sync, R: Send>(
        input: &'a [T],
        grain: usize,
        budget: u32,
        split: &(dyn Fn(&'a [T]) -> (&'a [T], &'a [T]) + Sync),
        leaf: &(dyn Fn(&'a [T]) -> R + Sync),
        combine: &(dyn Fn(R, R) -> R + Sync),
    ) -> R {
        if input.len() <= grain.max(1) || budget == 0 {
            return leaf(input);
        }
        let (l, r) = split(input);
        if l.is_empty() || r.is_empty() {
            return leaf(input); // split made no progress
        }
        let (a, b) = join(
            || go(l, grain, budget - 1, split, leaf, combine),
            || go(r, grain, budget - 1, split, leaf, combine),
        );
        combine(a, b)
    }
    go(input, grain, fork_budget(), &split, &leaf, &combine)
}

/// An associative operation with an identity: what parallel (and later GPU) reduction needs.
pub trait Monoid<T> {
    fn identity(&self) -> T;
    fn combine(&self, a: T, b: T) -> T;
}

pub struct Sum;
pub struct Min;
pub struct Max;

impl<T: Copy + std::ops::Add<Output = T> + Default> Monoid<T> for Sum {
    fn identity(&self) -> T {
        T::default()
    }
    fn combine(&self, a: T, b: T) -> T {
        a + b
    }
}

impl Monoid<i64> for Min {
    fn identity(&self) -> i64 {
        i64::MAX
    }
    fn combine(&self, a: i64, b: i64) -> i64 {
        a.min(b)
    }
}

impl Monoid<i64> for Max {
    fn identity(&self) -> i64 {
        i64::MIN
    }
    fn combine(&self, a: i64, b: i64) -> i64 {
        a.max(b)
    }
}

/// Reduce a slice with a monoid, in parallel. Grain keeps leaves large enough to amortise forking.
pub fn reduce<T, M>(items: &[T], m: &M) -> T
where
    T: Copy + Send + Sync,
    M: Monoid<T> + Sync,
{
    const GRAIN: usize = 16 * 1024;
    fold(
        items,
        GRAIN,
        |s| s.split_at(s.len() / 2),
        |s| s.iter().fold(m.identity(), |acc, x| m.combine(acc, *x)),
        |a, b| m.combine(a, b),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[test]
    fn join_runs_both_and_returns_both() {
        let (a, b) = join(|| (1..=1000u64).sum::<u64>(), || "right");
        assert_eq!((a, b), (500_500, "right"));
    }

    #[test]
    fn fold_equals_sequential_for_any_size_and_grain() {
        for n in [0u64, 1, 2, 7, 1_000, 100_003] {
            for grain in [1usize, 3, 64, 10_000] {
                let v: Vec<u64> = (0..n).collect();
                let par = sum_squares(&v, grain);
                let seq: u64 = v.iter().map(|x| x * x).sum();
                assert_eq!(par, seq, "n={n} grain={grain}");
            }
        }
    }

    fn sum_squares(v: &[u64], grain: usize) -> u64 {
        fold(
            v,
            grain,
            |s| s.split_at(s.len() / 2),
            |s| s.iter().map(|x| x * x).sum(),
            |a, b| a + b,
        )
    }

    #[test]
    fn combine_order_is_preserved_for_non_commutative_ops() {
        let words: Vec<String> = (0..500).map(|i| i.to_string()).collect();
        let par = fold(
            &words[..],
            7,
            |s| s.split_at(s.len() / 2),
            |s| s.concat(),
            |a, b| a + &b,
        );
        assert_eq!(par, words.concat());
    }

    #[test]
    fn large_work_actually_uses_more_than_one_thread() {
        if std::thread::available_parallelism().map_or(1, |n| n.get()) < 2 {
            return; // single-core machine: nothing to prove
        }
        let seen = Mutex::new(HashSet::new());
        let v: Vec<u64> = (0..2_000_000).collect();
        fold(
            &v[..],
            10_000,
            |s| s.split_at(s.len() / 2),
            |s| {
                seen.lock().unwrap().insert(std::thread::current().id());
                s.len()
            },
            |a, b| a + b,
        );
        assert!(seen.lock().unwrap().len() > 1);
    }

    #[test]
    fn monoid_reductions() {
        let v: Vec<i64> = (-500..500).collect();
        assert_eq!(reduce(&v, &Sum), v.iter().sum::<i64>());
        assert_eq!(reduce(&v, &Min), -500);
        assert_eq!(reduce(&v, &Max), 499);
        assert_eq!(reduce(&[] as &[i64], &Sum), 0);
    }
}

//! Job restart policy and process trees (wold). Wire/schema in `shared::jobs`.
use shared::jobs::JobSpec;
use std::collections::{HashMap, HashSet};

/// When to start the job again after it exited (seconds, same clock as inputs), or None to leave it stopped.
pub fn next_start(job: &JobSpec, last_start: f64, exited_at: f64) -> Option<f64> {
    job.keep_alive
        .then(|| exited_at.max(last_start + f64::from(job.throttle_seconds)))
}

/// ps(1) args for [`process_tree`]: one -o per column (FreeBSD reads "pid=,ppid=" as one header).
pub const PS_ARGS: [&str; 5] = ["-ax", "-o", "pid=", "-o", "ppid="];

/// `root` and its descendants from (pid, ppid) pairs, children before parents, root last.
pub fn process_tree(root: i32, pairs: &[(i32, i32)]) -> Vec<i32> {
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    let known: HashSet<i32> = pairs.iter().map(|p| p.0).collect();
    for &(pid, ppid) in pairs {
        children.entry(ppid).or_default().push(pid);
    }
    let mut order = Vec::new();
    fn visit(p: i32, c: &HashMap<i32, Vec<i32>>, out: &mut Vec<i32>, depth: usize) {
        if depth > 4096 {
            return;
        }
        for &k in c.get(&p).into_iter().flatten() {
            if k != p {
                visit(k, c, out, depth + 1);
            }
        }
        out.push(p);
    }
    if known.contains(&root) {
        visit(root, &children, &mut order, 0);
    }
    order
}

/// `ps -ax -o pid= -o ppid=` output into pairs; bad lines skipped.
pub fn parse_ps_pairs(text: &str) -> Vec<(i32, i32)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keep() -> JobSpec {
        JobSpec::parse(r#"{"label":"org.wol.test","program":["/bin/sleep","1"],"keepAlive":true,"throttleSeconds":10}"#).unwrap()
    }

    #[test]
    fn restart_policy_table() {
        let mut once = keep();
        once.keep_alive = false;
        for (job, last, exit, want) in [
            (once, 0.0, 30.0, None),
            (keep(), 0.0, 30.0, Some(30.0)),
            (keep(), 0.0, 1.0, Some(10.0)),
        ] {
            assert_eq!(next_start(&job, last, exit), want);
        }
    }

    #[test]
    fn tree_children_first_root_last() {
        let ps = "    1     0\n   10     1\n   11    10\n   12    11\n   13    12\n   14    12\n   20     1\nbad\n";
        let pairs = parse_ps_pairs(ps);
        assert_eq!(process_tree(10, &pairs), vec![13, 14, 12, 11, 10]);
        assert!(process_tree(99, &pairs).is_empty());
        assert_eq!(PS_ARGS, ["-ax", "-o", "pid=", "-o", "ppid="]);
    }
}

//! wold control endpoint (launchctl): anyone may list; only root may start or stop.
use shared::jobs::{JobRequest, JobStatus, Reply};

/// What the control endpoint needs from the supervisor.
pub trait JobTable {
    fn list(&self) -> Vec<JobStatus>;
    /// None if no such job; Some(error) otherwise (Some(None) = ok).
    fn start(&self, label: &str) -> Option<Option<String>>;
    fn stop(&self, label: &str) -> Option<Option<String>>;
}

/// One reply line (JSON) for a request-response request. Subscribe/Publish are handled by the server.
pub fn handle(req: Option<&JobRequest>, uid: Option<u32>, jobs: &dyn JobTable) -> String {
    let Some(req) = req else {
        return Reply::of(Some(
            "bad request: list | start <label> | stop <label> | subscribe | publish <event>".into(),
        ));
    };
    if req.needs_root() && uid != Some(0) {
        return Reply::of(Some("permission denied: root only".into()));
    }
    let (label, result) = match req {
        JobRequest::List => {
            let mut all = jobs.list();
            all.sort_by(|a, b| a.label.cmp(&b.label));
            return serde_json::to_string(&all).unwrap_or_else(|_| "[]".into());
        }
        JobRequest::Start(l) => (l, jobs.start(l)),
        JobRequest::Stop(l) => (l, jobs.stop(l)),
        JobRequest::Subscribe
        | JobRequest::SubscribeAccount
        | JobRequest::Publish(_)
        | JobRequest::Call(_) => {
            return Reply::of(Some("not a request-response request".into()));
        }
    };
    match result {
        None => Reply::of(Some(format!("no such job: {label}"))),
        Some(err) => Reply::of(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
    }

    impl JobTable for Fake {
        fn list(&self) -> Vec<JobStatus> {
            ["b", "a"]
                .iter()
                .map(|l| JobStatus {
                    label: (*l).into(),
                    state: "stopped".into(),
                    pid: None,
                    starts: 0,
                    last_exit: None,
                })
                .collect()
        }
        fn start(&self, label: &str) -> Option<Option<String>> {
            self.calls.borrow_mut().push(format!("start {label}"));
            (label == "a").then_some(None)
        }
        fn stop(&self, label: &str) -> Option<Option<String>> {
            self.calls.borrow_mut().push(format!("stop {label}"));
            (label == "a").then(|| Some("not started".into()))
        }
    }

    #[test]
    fn handle_table() {
        let start_a = JobRequest::Start("a".into());
        let start_x = JobRequest::Start("x".into());
        let stop_a = JobRequest::Stop("a".into());
        let cases: [(Option<&JobRequest>, Option<u32>, &str); 7] = [
            (
                None,
                Some(0),
                r#"{"ok":false,"error":"bad request: list | start <label> | stop <label> | subscribe | publish <event>"}"#,
            ),
            (
                Some(&start_a),
                Some(1000),
                r#"{"ok":false,"error":"permission denied: root only"}"#,
            ),
            (
                Some(&start_a),
                None,
                r#"{"ok":false,"error":"permission denied: root only"}"#,
            ),
            (Some(&start_a), Some(0), r#"{"ok":true,"error":null}"#),
            (
                Some(&start_x),
                Some(0),
                r#"{"ok":false,"error":"no such job: x"}"#,
            ),
            (
                Some(&stop_a),
                Some(0),
                r#"{"ok":false,"error":"not started"}"#,
            ),
            (
                Some(&JobRequest::List),
                Some(1000),
                r#"[{"label":"a","state":"stopped","pid":null,"starts":0,"lastExit":null},{"label":"b","state":"stopped","pid":null,"starts":0,"lastExit":null}]"#,
            ),
        ];
        for (req, uid, want) in cases {
            assert_eq!(handle(req, uid, &Fake::default()), want, "{req:?} {uid:?}");
        }
        let f = Fake::default();
        handle(Some(&start_a), Some(1000), &f);
        assert!(
            f.calls.borrow().is_empty(),
            "denied requests never reach the table"
        );
    }
}

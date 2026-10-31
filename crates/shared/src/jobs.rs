//! launchd-style jobs: one JSON file per job in [`JOBS_DIR`], wold control socket wire.
use serde::{Deserialize, Serialize};

pub const JOBS_DIR: &str = "/etc/wolos/jobs";
pub const SOCKET: &str = "/var/run/wold.sock";

/// Job definition. `run_at_load = false` = on demand (`wol start <label>`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSpec {
    pub label: String,
    pub program: Vec<String>,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default = "ten")]
    pub throttle_seconds: u32,
    #[serde(default = "yes")]
    pub run_at_load: bool,
    /// QoS class name (`shared::qos::QosClass::name`); default "default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
    /// Run as this account, with a clean environment; default root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<u32>,
}

fn ten() -> u32 {
    10
}
fn yes() -> bool {
    true
}

impl JobSpec {
    pub fn parse(json: &str) -> Result<Self, String> {
        let j: JobSpec = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if j.label.is_empty() || j.program.is_empty() || j.program[0].is_empty() {
            return Err("job needs label and program".into());
        }
        if let Some(q) = &j.qos
            && crate::qos::QosClass::parse(q).is_none()
        {
            return Err(format!("unknown qos {q}"));
        }
        Ok(j)
    }
}

/// One job as reported by `list`. state: running | stopped | waiting.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub label: String,
    pub state: String,
    pub pid: Option<i32>,
    pub starts: u32,
    pub last_exit: Option<i32>,
}

/// Control request: one line in, one JSON line out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobRequest {
    List,
    Start(String),
    Stop(String),
    /// Stream events (`shared::events`) until the client disconnects.
    Subscribe,
    /// Also this account's events (notifications): only the session's notification shower.
    SubscribeAccount,
    /// Publish one `shared::events::Event` JSON line to subscribers.
    Publish(String),
    /// libwol service call (`shared::portal::Call` JSON, decoded and checked by wold).
    Call(String),
}

impl JobRequest {
    pub fn parse(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["list"] => Some(Self::List),
            ["subscribe"] => Some(Self::Subscribe),
            ["subscribe", "account"] => Some(Self::SubscribeAccount),
            ["publish", ..] => {
                let json = line.trim().strip_prefix("publish")?.trim();
                crate::events::Event::parse(json).map(|_| Self::Publish(json.into()))
            }
            ["call", ..] => Some(Self::Call(line.trim().strip_prefix("call")?.trim().into())),
            ["start", l] => Some(Self::Start((*l).into())),
            ["stop", l] => Some(Self::Stop((*l).into())),
            _ => None,
        }
    }

    pub fn line(&self) -> String {
        match self {
            Self::List => "list".into(),
            Self::Subscribe => "subscribe".into(),
            Self::SubscribeAccount => "subscribe account".into(),
            Self::Publish(j) => format!("publish {j}"),
            Self::Call(j) => format!("call {j}"),
            Self::Start(l) => format!("start {l}"),
            Self::Stop(l) => format!("stop {l}"),
        }
    }

    /// Mutating requests need root (peer uid 0).
    pub fn needs_root(&self) -> bool {
        matches!(self, Self::Start(_) | Self::Stop(_))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub ok: bool,
    pub error: Option<String>,
}

impl Reply {
    pub fn of(error: Option<String>) -> String {
        serde_json::to_string(&Reply {
            ok: error.is_none(),
            error,
        })
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_defaults_and_validation() {
        let j = JobSpec::parse(r#"{"label":"a","program":["/bin/a"]}"#).unwrap();
        assert!(!j.keep_alive && j.run_at_load && j.throttle_seconds == 10 && j.qos.is_none());
        for bad in [
            "{}",
            r#"{"label":"","program":["x"]}"#,
            r#"{"label":"a","program":[]}"#,
            r#"{"label":"a","program":["x"],"qos":"turbo"}"#,
            "not json",
        ] {
            assert!(JobSpec::parse(bad).is_err(), "{bad}");
        }
        let j = JobSpec::parse(r#"{"label":"a","program":["x"],"keepAlive":true,"throttleSeconds":3,"runAtLoad":false,"qos":"utility"}"#).unwrap();
        assert!(j.keep_alive && !j.run_at_load && j.throttle_seconds == 3);
    }

    #[test]
    fn request_wire_table() {
        for (line, want) in [
            ("list", Some(JobRequest::List)),
            (" list \n", Some(JobRequest::List)),
            ("subscribe", Some(JobRequest::Subscribe)),
            ("subscribe account", Some(JobRequest::SubscribeAccount)),
            ("subscribe all", None),
            ("start a", Some(JobRequest::Start("a".into()))),
            ("stop a", Some(JobRequest::Stop("a".into()))),
            ("start", None),
            ("start a b", None),
            ("rm a", None),
            (
                "publish {\"event\":\"workspace\",\"name\":\"1\"}",
                Some(JobRequest::Publish(
                    "{\"event\":\"workspace\",\"name\":\"1\"}".into(),
                )),
            ),
            ("publish nope", None),
            (
                "call {\"call\":\"notification.post\",\"title\":\"a b\"}\n",
                Some(JobRequest::Call(
                    "{\"call\":\"notification.post\",\"title\":\"a b\"}".into(),
                )),
            ),
        ] {
            assert_eq!(JobRequest::parse(line), want, "{line}");
            if let Some(r) = want {
                assert_eq!(JobRequest::parse(&r.line()), Some(r));
            }
        }
        assert!(!JobRequest::List.needs_root() && JobRequest::Stop("a".into()).needs_root());
        assert!(!JobRequest::Call(String::new()).needs_root());
    }

    #[test]
    fn status_and_reply_json() {
        let s = JobStatus {
            label: "a".into(),
            state: "running".into(),
            pid: Some(3),
            starts: 1,
            last_exit: None,
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(
            j,
            r#"{"label":"a","state":"running","pid":3,"starts":1,"lastExit":null}"#
        );
        assert_eq!(Reply::of(None), r#"{"ok":true,"error":null}"#);
        assert_eq!(Reply::of(Some("x".into())), r#"{"ok":false,"error":"x"}"#);
    }
}

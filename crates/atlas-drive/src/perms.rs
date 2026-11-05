// SPDX-License-Identifier: GPL-2.0-or-later
//! Permissions without permission spam (ADR 0063). One consent per agent (peer executable) with
//! scopes, remembered; an explicit AUTO mode (global or per site, time-boxed or until revoked)
//! that stops prompting; never a prompt per verb. The store is a JSON file in the person's config
//! directory, 0600, replaced atomically. The consent sheet itself is the shell's: a configured
//! command (`ATLAS_DRIVE_CONSENT_CMD`) is asked once per new peer; exit 0 grants.
//!
//! Scopes: read (page content), act (input, navigation, eval), network (requests, interception),
//! storage (cookies, storage), files (downloads, uploads, pdf, traces, recordings).
use crate::util::now_ms;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const SCOPES: &[&str] = &["read", "act", "network", "storage", "files"];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Auto {
    /// Unix ms; None = until revoked.
    pub until: Option<u64>,
    /// Empty = every site.
    pub sites: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub exe: String,
    pub scopes: Vec<String>,
    pub granted_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Grants {
    pub peers: Vec<Peer>,
    pub auto: Option<Auto>,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Allow,
    /// Allowed because AUTO mode is on.
    Auto,
    /// Needs a grant: ask the person once.
    Ask,
}

impl Grants {
    pub fn from_json(v: &Value) -> Grants {
        let peers = v["peers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some(Peer {
                            exe: p["exe"].as_str()?.to_string(),
                            scopes: p["scopes"]
                                .as_array()?
                                .iter()
                                .filter_map(Value::as_str)
                                .filter(|s| SCOPES.contains(s))
                                .map(String::from)
                                .collect(),
                            granted_ms: p["granted_ms"].as_u64().unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let auto = v.get("auto").filter(|a| a.is_object()).map(|a| Auto {
            until: a["until_ms"].as_u64(),
            sites: a["sites"]
                .as_array()
                .map(|s| {
                    s.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
        });
        Grants { peers, auto }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "peers": self.peers.iter().map(|p| json!({"exe": p.exe, "scopes": p.scopes, "granted_ms": p.granted_ms})).collect::<Vec<_>>(),
            "auto": self.auto.as_ref().map(|a| json!({"until_ms": a.until, "sites": a.sites})),
        })
    }

    pub fn auto_active(&self, now: u64, site: Option<&str>) -> bool {
        match &self.auto {
            None => false,
            Some(a) => {
                if a.until.is_some_and(|u| now >= u) {
                    return false;
                }
                a.sites.is_empty()
                    || site.is_some_and(|s| {
                        a.sites
                            .iter()
                            .any(|x| s == x || s.ends_with(&format!(".{x}")))
                    })
            }
        }
    }

    pub fn decide(&self, exe: &str, scope: &str, now: u64, site: Option<&str>) -> Decision {
        if self
            .peers
            .iter()
            .any(|p| p.exe == exe && p.scopes.iter().any(|s| s == scope))
        {
            return Decision::Allow;
        }
        if self.auto_active(now, site) {
            return Decision::Auto;
        }
        Decision::Ask
    }

    pub fn grant(&mut self, exe: &str, scopes: &[String], now: u64) {
        let scopes: Vec<String> = scopes
            .iter()
            .filter(|s| SCOPES.contains(&s.as_str()))
            .cloned()
            .collect();
        match self.peers.iter_mut().find(|p| p.exe == exe) {
            Some(p) => {
                for s in scopes {
                    if !p.scopes.contains(&s) {
                        p.scopes.push(s);
                    }
                }
            }
            None => self.peers.push(Peer {
                exe: exe.into(),
                scopes,
                granted_ms: now,
            }),
        }
    }

    pub fn revoke(&mut self, exe: &str) -> bool {
        let n = self.peers.len();
        self.peers.retain(|p| p.exe != exe);
        self.peers.len() != n
    }

    pub fn list(&self, now: u64) -> String {
        let mut s = String::new();
        match &self.auto {
            Some(a) if a.until.is_none_or(|u| now < u) => {
                s.push_str(&format!(
                    "AUTO on: {} {}\n",
                    if a.sites.is_empty() {
                        "every site".to_string()
                    } else {
                        a.sites.join(", ")
                    },
                    a.until
                        .map(|u| format!("for {} more minutes", (u - now) / 60000))
                        .unwrap_or_else(|| "until revoked".into())
                ));
            }
            _ => s.push_str("AUTO off\n"),
        }
        for p in &self.peers {
            s.push_str(&format!("{}  scopes: {}\n", p.exe, p.scopes.join(",")));
        }
        if self.peers.is_empty() {
            s.push_str("(no agents have been allowed)\n");
        }
        s
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("wolos").join("atlas-drive").join("grants.json")
}

pub fn load(path: &Path) -> Grants {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| Grants::from_json(&v))
        .unwrap_or_default()
}

/// Write 0600 through a temporary file and rename: a reader never sees half a file.
pub fn save(path: &Path, g: &Grants) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(g.to_json().to_string().as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Parse `90m`, `2h`, `1d`, `45s` to milliseconds.
pub fn parse_span(s: &str) -> Option<u64> {
    let (n, unit) = s.split_at(s.len().checked_sub(1)?);
    let n: u64 = n.parse().ok()?;
    Some(
        n * match unit {
            "s" => 1000,
            "m" => 60_000,
            "h" => 3_600_000,
            "d" => 86_400_000,
            _ => return None,
        },
    )
}

/// Ask the person once, through the configured sheet command. Exit 0 = allow.
pub fn ask(cmd: &str, exe: &str, scopes: &[&str]) -> bool {
    use std::process::{Command, Stdio};
    let Ok(mut c) = Command::new(cmd)
        .arg(exe)
        .arg(scopes.join(","))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let end = std::time::Instant::now() + crate::util::WAIT_120S;
    loop {
        match c.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if std::time::Instant::now() < end => {
                std::thread::sleep(crate::util::WAIT_50MS)
            }
            _ => {
                let _ = c.kill();
                let _ = c.wait();
                return false;
            }
        }
    }
}

pub fn site_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    Some(
        rest.split(['/', '?', '#'])
            .next()?
            .split(':')
            .next()?
            .to_string(),
    )
}

pub fn now() -> u64 {
    now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_is_asked_once_then_remembered_per_scope() {
        let mut g = Grants::default();
        assert_eq!(g.decide("/usr/bin/agent", "read", 1, None), Decision::Ask);
        g.grant(
            "/usr/bin/agent",
            &["read".into(), "act".into(), "bogus".into()],
            1,
        );
        assert_eq!(g.decide("/usr/bin/agent", "read", 2, None), Decision::Allow);
        assert_eq!(g.decide("/usr/bin/agent", "act", 2, None), Decision::Allow);
        assert_eq!(
            g.decide("/usr/bin/agent", "files", 2, None),
            Decision::Ask,
            "a scope not granted is asked, not assumed"
        );
        assert_eq!(g.decide("/usr/bin/other", "read", 2, None), Decision::Ask);
        assert!(g.revoke("/usr/bin/agent"));
        assert_eq!(g.decide("/usr/bin/agent", "read", 3, None), Decision::Ask);
        assert!(!g.revoke("/usr/bin/agent"));
    }

    #[test]
    fn auto_mode_is_time_boxed_and_site_scoped() {
        let mut g = Grants {
            auto: Some(Auto {
                until: Some(1000),
                sites: vec![],
            }),
            ..Grants::default()
        };
        assert_eq!(g.decide("/x", "act", 999, None), Decision::Auto);
        assert_eq!(g.decide("/x", "act", 1000, None), Decision::Ask, "expired");
        g.auto = Some(Auto {
            until: None,
            sites: vec!["example.com".into()],
        });
        assert_eq!(
            g.decide("/x", "act", 5, Some("example.com")),
            Decision::Auto
        );
        assert_eq!(
            g.decide("/x", "act", 5, Some("app.example.com")),
            Decision::Auto
        );
        assert_eq!(g.decide("/x", "act", 5, Some("evil.com")), Decision::Ask);
        assert_eq!(
            g.decide("/x", "act", 5, Some("notexample.com")),
            Decision::Ask
        );
        assert_eq!(g.decide("/x", "act", 5, None), Decision::Ask);
    }

    #[test]
    fn the_store_round_trips_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ad-perms-{}", std::process::id()));
        let path = dir.join("wolos/atlas-drive/grants.json");
        let mut g = Grants::default();
        g.grant("/opt/agent", &["read".into()], 7);
        g.auto = Some(Auto {
            until: Some(99),
            sites: vec!["a.test".into()],
        });
        save(&path, &g).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let back = load(&path);
        assert_eq!(back.peers, g.peers);
        assert_eq!(back.auto, g.auto);
        assert_eq!(
            load(&dir.join("nothing")).peers.len(),
            0,
            "a missing file is an empty store"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn spans_and_sites() {
        assert_eq!(parse_span("90m"), Some(5_400_000));
        assert_eq!(parse_span("2h"), Some(7_200_000));
        assert_eq!(parse_span("x"), None);
        assert_eq!(
            site_of("https://app.example.com:8443/a?b"),
            Some("app.example.com".into())
        );
        assert_eq!(site_of("about:blank"), None);
    }

    #[test]
    fn the_sheet_command_decides() {
        assert!(ask("true", "/x", &["read"]));
        assert!(!ask("false", "/x", &["read"]));
        assert!(!ask("/nonexistent/sheet", "/x", &["read"]));
    }
}

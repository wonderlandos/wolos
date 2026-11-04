// SPDX-License-Identifier: GPL-2.0-or-later
//! The host's command socket (ADR 0053 D1, D3): how the session asks a running Tor group to open
//! a tab or to give one site a new circuit.
//!
//! One line per connection, at most [`MAX_COMMAND`] bytes: `open <url>` or `newcircuit <host>`.
//! A URL reaches Atlas as an argument, so it must not look like a flag (`--...`), and only
//! `http`, `https` and the browser's own pages are accepted: a `file:` URL in the Tor instance
//! would read the (empty) private home and is never useful.
use std::fmt;

/// Longest command line.
pub const MAX_COMMAND: usize = 4096;

/// A command for a running group.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Open this URL in a new tab (`None` = the new-tab page).
    Open(Option<String>),
    /// New circuit for this site.
    NewCircuit(String),
}

/// A command the host refused.
#[derive(Debug, PartialEq, Eq)]
pub struct BadCommand(pub String);

impl fmt::Display for BadCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the Tor group did not accept this request: {}", self.0)
    }
}

impl std::error::Error for BadCommand {}

/// Checks a URL for the Tor instance.
pub fn check_url(url: &str) -> Result<(), BadCommand> {
    let lower = url.to_ascii_lowercase();
    let scheme_ok = lower
        .split_once("://")
        .is_some_and(|(scheme, rest)| matches!(scheme, "http" | "https") && !rest.is_empty())
        || lower.starts_with("atlas://newtab")
        || lower.starts_with("about:blank");
    if url.starts_with('-')
        || !scheme_ok
        || url.len() > MAX_COMMAND
        || url.bytes().any(|b| b <= 0x20 || b == 0x7f)
    {
        return Err(BadCommand(format!("{url:?} is not a web address")));
    }
    Ok(())
}

/// Parses one command line (without its line break).
pub fn parse(line: &str) -> Result<Command, BadCommand> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.len() > MAX_COMMAND {
        return Err(BadCommand("the request is too long".into()));
    }
    let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
    match verb {
        "open" if arg.is_empty() => Ok(Command::Open(None)),
        "open" => check_url(arg).map(|()| Command::Open(Some(arg.to_string()))),
        "newcircuit"
            if !arg.is_empty()
                && arg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)) =>
        {
            Ok(Command::NewCircuit(arg.to_ascii_lowercase()))
        }
        _ => Err(BadCommand(format!(
            "{verb:?} is not a request the Tor group knows"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_newcircuit_parse() {
        assert_eq!(
            parse("open https://example.org/a?b=c\n"),
            Ok(Command::Open(Some("https://example.org/a?b=c".into())))
        );
        assert_eq!(parse("open"), Ok(Command::Open(None)));
        assert_eq!(
            parse("newcircuit WWW.Example.org"),
            Ok(Command::NewCircuit("www.example.org".into()))
        );
    }

    #[test]
    fn urls_cannot_become_browser_flags() {
        for bad in [
            "--no-sandbox",
            "-x",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://a b",
            "chrome://settings",
            "data:text/html,x",
        ] {
            assert!(parse(&format!("open {bad}")).is_err(), "{bad}");
        }
    }

    #[test]
    fn unknown_or_oversized_requests_are_refused() {
        assert!(parse("halt").is_err());
        assert!(parse("newcircuit a/b").is_err());
        assert!(parse("newcircuit").is_err());
        assert!(parse(&format!("open https://{}", "a".repeat(MAX_COMMAND))).is_err());
    }
}

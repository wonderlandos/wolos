// SPDX-License-Identifier: BSD-2-Clause
//! The origin gate: what a web app may load. The engine asks it for every navigation and every
//! resource load (`wwe_config.allow`); a `false` is a network error for a resource and no
//! navigation for a page. Invariant: nothing is reachable unless the policy names its origin,
//! except the inline schemes a page needs to work (`about:blank`, `data:` and `blob:` resources of
//! an allowed origin); a `file:` URL only inside a folder the policy names. The profile directory is
//! the engine's own (storage, cache) and never a page's: it is not reachable through `file:`
//! unless the policy names it, which [`Gate::new`] refuses.
//!
//! The URLs come from the engine already parsed and serialised (lower-case scheme and host,
//! default port dropped, dot segments resolved), so this is matching, not parsing; anything that
//! does not look like that is refused.
use crate::policy::{Policy, TelemetryOff};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Load {
    /// A page or frame navigation.
    Navigation,
    /// A subresource, fetch, socket or worker script.
    Resource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    /// `scheme://host[:port]`; a host `*.base` matches subdomains of `base` only.
    Web {
        https: bool,
        host: String,
        port: Option<u16>,
    },
    /// `file:///dir/` (ends with `/`).
    Folder(String),
    /// `fed://label.tld`: a Federate Network name (ADR 0052), every path under it.
    Fed(String),
}

#[derive(Debug, Clone)]
pub struct Gate {
    rules: Vec<Rule>,
    telemetry: TelemetryOff,
}

/// Split `scheme://authority/rest` into its parts (authority without user info). `*` is accepted in
/// the host so policy entries can use it; a URL host with one is refused by [`Gate::allows`].
fn split_web(url: &str) -> Option<(&str, &str, Option<u16>, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.contains('@') {
        return None;
    }
    // `[v6]` or `[v6]:port`, else `host` or `host:port`.
    let (host, port) = if authority.starts_with('[') {
        match authority.split_once("]:") {
            Some((h, p)) => (&authority[..h.len() + 1], Some(p.parse().ok()?)),
            None if authority.ends_with(']') => (authority, None),
            None => return None,
        }
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h, Some(p.parse().ok()?)),
            None => (authority, None),
        }
    };
    let host_ok = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.[]:*".contains(&b));
    host_ok.then_some((scheme, host, port, path))
}

fn parse_rule(entry: &str) -> Option<Rule> {
    if let Some(dir) = entry.strip_prefix("file://") {
        let ok = dir.starts_with('/') && dir.ends_with('/') && !dir.contains("/../");
        return ok.then(|| Rule::Folder(dir.to_owned()));
    }
    if let Some(name) = entry.strip_prefix("fed://") {
        let name = name.strip_suffix('/').unwrap_or(name);
        let ok = federate::FedUrl::parse(entry).is_ok_and(|u| u.domain() == name);
        return ok.then(|| Rule::Fed(name.to_owned()));
    }
    let (scheme, host, port, path) = split_web(entry)?;
    let https = match scheme {
        "https" => true,
        "http" => false,
        _ => return None,
    };
    if !(path.is_empty() || path == "/") {
        return None;
    }
    let base = host.strip_prefix("*.").unwrap_or(host);
    (!base.contains('*') && !base.is_empty()).then(|| Rule::Web {
        https,
        host: host.to_owned(),
        port,
    })
}

fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(base) => {
            host.len() > base.len()
                && host.ends_with(base)
                && host[..host.len() - base.len()].ends_with('.')
        }
        None => host == pattern,
    }
}

impl Gate {
    /// The gate for `policy`. Refuses an origin entry it cannot read, and a `file:` folder that is
    /// the profile directory or inside it (the engine's storage is never a page).
    pub fn new(policy: &Policy, profile_dir: &Path) -> Result<Gate, String> {
        let mut rules = Vec::new();
        let profile = format!("{}/", profile_dir.to_string_lossy().trim_end_matches('/'));
        for o in &policy.origins {
            if let Some((scheme, host, _, _)) = split_web(o)
                && matches!(scheme, "http" | "https" | "ws" | "wss")
                && crate::fed::is_fed_host(host)
            {
                return Err(format!(
                    "origin {o:?} is a Federate name: it is fed://, never http(s)"
                ));
            }
            let rule = parse_rule(o).ok_or_else(|| format!("origin {o:?} is not an origin"))?;
            if let Rule::Folder(d) = &rule
                && (d.starts_with(&profile) || profile.starts_with(d.as_str()))
            {
                return Err(format!("origin {o:?} overlaps the profile directory"));
            }
            rules.push(rule);
        }
        Ok(Gate {
            rules,
            telemetry: policy.telemetry_off.clone(),
        })
    }

    fn web_allowed(&self, https: bool, host: &str, port: Option<u16>, path: &str) -> bool {
        // A Federate TLD host is `fed://` only (ADR 0052, exclusive): no policy opens it over the
        // web, so no socket and no DNS lookup is ever made for it.
        if crate::fed::is_fed_host(host) {
            return false;
        }
        if self.telemetry.hosts.iter().any(|h| host_matches(h, host)) {
            return false;
        }
        let path_only = path.split(['?', '#']).next().unwrap_or("");
        // A path entry covers itself and what is below it (`/beacon` is not `/beaconless`).
        let under = |p: &str| {
            !p.is_empty()
                && path_only.starts_with(p)
                && (p.ends_with('/')
                    || matches!(path_only.as_bytes().get(p.len()), None | Some(b'/')))
        };
        if self.telemetry.beacon_paths.iter().any(|p| under(p)) {
            return false;
        }
        self.rules.iter().any(|r| match r {
            Rule::Web {
                https: s,
                host: h,
                port: p,
            } => *s == https && *p == port && host_matches(h, host),
            Rule::Folder(_) | Rule::Fed(_) => false,
        })
    }

    /// Whether the app may load `url`.
    pub fn allows(&self, url: &str, load: Load) -> bool {
        if url == "about:blank" || url == "about:srcdoc" {
            return true;
        }
        if url.starts_with("data:") {
            return load == Load::Resource;
        }
        if let Some(inner) = url.strip_prefix("blob:") {
            return inner.contains("://") && self.allows(inner, load);
        }
        if let Some(path) = url.strip_prefix("file://") {
            let lower = path.to_ascii_lowercase();
            if !path.starts_with('/')
                || path.contains("/../")
                || lower.contains("%2e")
                || lower.contains("%2f")
            {
                return false;
            }
            let path = path.split(['?', '#']).next().unwrap_or("");
            return self
                .rules
                .iter()
                .any(|r| matches!(r, Rule::Folder(d) if path.starts_with(d.as_str())));
        }
        if url.starts_with("fed://") {
            // Already in the canonical spelling the engine serialises; anything else is refused.
            return federate::FedUrl::parse(url).is_ok_and(|u| {
                let d = u.domain();
                url[6..].starts_with(&d)
                    && self
                        .rules
                        .iter()
                        .any(|r| matches!(r, Rule::Fed(n) if *n == d))
            });
        }
        let Some((scheme, host, port, path)) =
            split_web(url).filter(|(_, h, _, _)| !h.contains('*'))
        else {
            return false;
        };
        let https = match scheme {
            "https" | "wss" => true,
            "http" | "ws" => false,
            _ => return false,
        };
        self.web_allowed(https, host, port, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(origins: &[&str], hosts: &[&str], paths: &[&str]) -> Gate {
        let p = Policy {
            origins: origins.iter().map(|s| s.to_string()).collect(),
            telemetry_off: TelemetryOff {
                hosts: hosts.iter().map(|s| s.to_string()).collect(),
                beacon_paths: paths.iter().map(|s| s.to_string()).collect(),
            },
            ..Policy::default()
        };
        Gate::new(
            &p,
            Path::new("/home/a/.local/share/wolos/webapps/profiles/x"),
        )
        .unwrap()
    }

    /// ADR 0052: a Federate name is an origin like a web host: every path under it, nothing else.
    #[test]
    fn a_federate_name_is_an_origin() {
        let g = gate(&["fed://home.fed"], &[], &[]);
        for ok in [
            "fed://home.fed",
            "fed://home.fed/",
            "fed://home.fed/style.css?v=1",
            "fed://home.fed#top",
        ] {
            assert!(g.allows(ok, Load::Navigation), "{ok}");
            assert!(g.allows(ok, Load::Resource), "{ok}");
        }
        for no in [
            "fed://lobo.pagina/",
            "fed://home.fedx/",
            "fed://home.fed:80/",
            "fed://x.home.fed/",
            "http://home.fed/",
            "fed://HOME.fed/",
        ] {
            assert!(!g.allows(no, Load::Navigation), "{no}");
        }
        assert!(gate(&["fed://home.fed/"], &[], &[]).allows("fed://home.fed/a", Load::Resource));
        let bad = Policy {
            origins: vec!["fed://home.fed/about".into()],
            ..Policy::default()
        };
        assert!(
            Gate::new(&bad, Path::new("/p")).is_err(),
            "an origin has no path"
        );
        assert!(
            !gate(&[], &[], &[]).allows("fed://home.fed/", Load::Navigation),
            "no origin, no load"
        );
    }

    /// Lead rule 2026-11-05: a Federate name is exclusively `fed://`. The gate answers before any
    /// socket or DNS lookup exists, whatever the policy names, for every scheme and every load.
    #[test]
    fn a_federate_tld_host_is_never_reachable_over_the_web() {
        let g = gate(&["fed://home.fed", "https://example.com"], &[], &[]);
        for no in [
            "http://home.fed/",
            "https://home.fed/style.css",
            "https://www.home.fed/",
            "wss://home.fed/socket",
            "ws://x.pagina:8080/",
            "https://lobo.pagina/",
        ] {
            assert!(!g.allows(no, Load::Navigation), "{no}");
            assert!(!g.allows(no, Load::Resource), "{no}");
        }
        assert!(g.allows("https://example.com/", Load::Resource));
        // A policy cannot name one: not as an origin, not as a wildcard.
        for entry in ["https://home.fed", "http://lobo.pagina", "https://*.fed"] {
            let p = Policy {
                origins: vec![entry.into()],
                ..Policy::default()
            };
            let err = Gate::new(&p, Path::new("/p")).unwrap_err();
            assert!(err.contains("fed://"), "{entry}: {err}");
        }
    }

    #[test]
    fn only_named_origins_are_reachable() {
        let g = gate(&["https://app.example", "http://localhost:8080"], &[], &[]);
        let nav = Load::Navigation;
        for (url, want) in [
            ("https://app.example/", true),
            ("https://app.example/a/b?c#d", true),
            ("wss://app.example/socket", true),
            ("http://app.example/", false),
            ("https://evil.example/", false),
            ("https://app.example.evil.example/", false),
            ("https://x.app.example/", false),
            ("https://app.example:444/", false),
            ("http://localhost:8080/", true),
            ("http://localhost/", false),
            ("http://[::1]:8080/", false),
            ("http://[::1/", false),
            ("https://user@app.example/", false),
            ("https://*.app.example/", false),
            ("ftp://app.example/", false),
            ("javascript:alert(1)", false),
            ("about:blank", true),
            ("about:config", false),
            ("", false),
        ] {
            assert_eq!(g.allows(url, nav), want, "{url}");
        }
    }

    #[test]
    fn subdomain_wildcards_match_subdomains_only() {
        let g = gate(&["https://*.example.com"], &[], &[]);
        assert!(g.allows("https://cdn.example.com/x.js", Load::Resource));
        assert!(g.allows("https://a.b.example.com/", Load::Resource));
        assert!(!g.allows("https://example.com/", Load::Resource));
        assert!(!g.allows("https://badexample.com/", Load::Resource));
    }

    #[test]
    fn inline_schemes_follow_their_origin() {
        let g = gate(&["https://app.example"], &[], &[]);
        assert!(g.allows("data:image/png;base64,AAAA", Load::Resource));
        assert!(!g.allows("data:text/html,<p>hi", Load::Navigation));
        assert!(g.allows("blob:https://app.example/1234", Load::Resource));
        assert!(!g.allows("blob:https://evil.example/1234", Load::Resource));
        assert!(!g.allows("blob:null/1234", Load::Resource));
    }

    #[test]
    fn telemetry_is_refused_inside_allowed_origins() {
        let g = gate(
            &["https://app.example", "https://*.example.com"],
            &["stats.example.com"],
            &["/beacon"],
        );
        assert!(!g.allows("https://stats.example.com/x", Load::Resource));
        assert!(g.allows("https://cdn.example.com/x", Load::Resource));
        assert!(!g.allows("https://app.example/beacon?id=1", Load::Resource));
        assert!(!g.allows("https://app.example/beacon/2", Load::Resource));
        assert!(g.allows("https://app.example/beaconless", Load::Resource));
        assert!(g.allows("https://app.example/app/beacon", Load::Resource));
    }

    #[test]
    fn files_only_inside_named_folders() {
        let g = gate(&["file:///usr/share/wolos/webapps/notes/"], &[], &[]);
        assert!(g.allows(
            "file:///usr/share/wolos/webapps/notes/index.html",
            Load::Navigation
        ));
        assert!(g.allows(
            "file:///usr/share/wolos/webapps/notes/css/a.css?v=1",
            Load::Resource
        ));
        assert!(!g.allows(
            "file:///usr/share/wolos/webapps/notesx/a.html",
            Load::Navigation
        ));
        assert!(!g.allows(
            "file:///usr/share/wolos/webapps/notes/../../../etc/passwd",
            Load::Resource
        ));
        assert!(!g.allows(
            "file:///usr/share/wolos/webapps/notes/%2e%2e/x",
            Load::Resource
        ));
        assert!(!g.allows("file:///etc/passwd", Load::Resource));
        assert!(!g.allows(
            "file://host/usr/share/wolos/webapps/notes/a",
            Load::Resource
        ));
    }

    #[test]
    fn bad_entries_and_the_profile_directory_are_refused() {
        let profile = Path::new("/home/a/p");
        for bad in [
            "app.example",
            "https://app.example/path",
            "https://*.*.example",
            "chrome://settings",
            "file://relative/",
            "file:///no/slash",
            "file:///home/a/p/",
            "file:///home/a/p/sub/",
            "file:///home/",
        ] {
            let p = Policy {
                origins: vec![bad.into()],
                ..Policy::default()
            };
            assert!(Gate::new(&p, profile).is_err(), "{bad}");
        }
        let empty = Gate::new(&Policy::default(), profile).unwrap();
        assert!(!empty.allows("https://app.example/", Load::Navigation));
        assert!(empty.allows("about:blank", Load::Navigation));
    }
}

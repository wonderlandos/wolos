// SPDX-License-Identifier: BSD-2-Clause
//! Federate addresses: `fed://<label>.<tld>[/path][?query]`, the protocol's grammar
//! (docs/en-US/federate-uri.md of the reference implementation): a label of a-z, 0-9 and hyphen
//! (1 to 63, no hyphen at either end), a TLD of a-z (2 to 32), no port, no user info, no IP
//! literal, an absolute path of at most 2048 bytes, the query kept as is, the fragment dropped.
//! Parsing is syntax only: whether the name exists is the signed root zone's answer.

/// Longest path accepted (the protocol's gateway cap).
pub const MAX_PATH: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FedUrl {
    pub label: String,
    pub tld: String,
    /// Absolute, `/` at least.
    pub path: String,
    /// Without the `?`; `None` when empty.
    pub query: Option<String>,
}

pub fn label_ok(l: &str) -> bool {
    (1..=63).contains(&l.len())
        && l.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !l.starts_with('-')
        && !l.ends_with('-')
}

pub fn tld_ok(t: &str) -> bool {
    (2..=32).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_lowercase())
}

/// `label.tld`, lower-cased and checked, or why not.
fn name(authority: &str) -> Result<(String, String), String> {
    if authority.contains('@') {
        return Err("a Federate address has no user name".into());
    }
    if authority.contains(':') || authority.starts_with('[') {
        return Err("a Federate address has no port and no IP address".into());
    }
    let host = authority.to_ascii_lowercase();
    let Some((label, tld)) = host.split_once('.') else {
        return Err(format!("{host} is not a label followed by a TLD"));
    };
    if tld.contains('.') {
        return Err(format!("{host}: exactly one label and one TLD"));
    }
    if !label_ok(label) {
        return Err(format!("{label} is not a valid Federate label"));
    }
    if !tld_ok(tld) {
        return Err(format!(".{tld} is not a valid Federate TLD"));
    }
    Ok((label.to_owned(), tld.to_owned()))
}

/// `(path, query)` of what follows the authority: fragment dropped, path `/` when empty.
fn tail(rest: &str) -> Result<(String, Option<String>), String> {
    let rest = rest.split('#').next().unwrap_or("");
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, (!q.is_empty()).then(|| q.to_owned())),
        None => (rest, None),
    };
    let path = if path.is_empty() { "/" } else { path };
    if !path.starts_with('/') {
        return Err("the path must start with /".into());
    }
    if path.len() > MAX_PATH {
        return Err("the path is too long".into());
    }
    Ok((path.to_owned(), query))
}

fn split_authority(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?', '#']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    }
}

impl FedUrl {
    /// A `fed://` address (the scheme in any case, as an engine may hand it over).
    pub fn parse(input: &str) -> Result<FedUrl, String> {
        let s = input.trim();
        let rest = s
            .get(..6)
            .filter(|p| p.eq_ignore_ascii_case("fed://"))
            .map(|_| &s[6..])
            .ok_or_else(|| format!("{s} does not start with fed://"))?;
        let (authority, rest) = split_authority(rest);
        let (label, tld) = name(authority)?;
        let (path, query) = tail(rest)?;
        Ok(FedUrl {
            label,
            tld,
            path,
            query,
        })
    }

    /// The `fed://` address an `http://` or `https://` URL names when its host is a Federate name
    /// (the network's compatibility spelling, `http://home.fed`); the port is dropped, as the
    /// protocol's gateway does. Syntax only: the caller decides whether the TLD is the network's.
    pub fn from_http(url: &str) -> Option<FedUrl> {
        let (scheme, rest) = url.split_once("://")?;
        if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
            return None;
        }
        let (authority, rest) = split_authority(rest);
        let host = match authority.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => h,
            _ => authority,
        };
        let (label, tld) = name(host).ok()?;
        let (path, query) = tail(rest).ok()?;
        Some(FedUrl {
            label,
            tld,
            path,
            query,
        })
    }

    /// `label.tld`.
    pub fn domain(&self) -> String {
        format!("{}.{}", self.label, self.tld)
    }

    /// The canonical spelling: `fed://home.fed`, `fed://home.fed/about?x=1`.
    pub fn to_url(&self) -> String {
        let mut s = format!("fed://{}", self.domain());
        if self.path != "/" {
            s.push_str(&self.path);
        }
        if let Some(q) = &self.query {
            s.push('?');
            s.push_str(q);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protocols_examples_parse() {
        let u = FedUrl::parse("fed://home.fed").unwrap();
        assert_eq!(
            (u.label.as_str(), u.tld.as_str(), u.path.as_str()),
            ("home", "fed", "/")
        );
        assert_eq!(u.to_url(), "fed://home.fed");
        let u = FedUrl::parse("fed://fed.busca/?q=manifesto").unwrap();
        assert_eq!(
            (u.path.as_str(), u.query.as_deref()),
            ("/", Some("q=manifesto"))
        );
        let u = FedUrl::parse("FED://Joao.Pagina/about#top").unwrap();
        assert_eq!(u.to_url(), "fed://joao.pagina/about");
        assert_eq!(
            FedUrl::parse("fed://arcade.mosca/play?level=2")
                .unwrap()
                .to_url(),
            "fed://arcade.mosca/play?level=2"
        );
    }

    #[test]
    fn what_the_grammar_refuses_is_refused() {
        for bad in [
            "fed://",
            "fed://home",
            "fed://home.fed:4077/",
            "fed://user@home.fed/",
            "fed://a.b.fed/",
            "fed://-home.fed/",
            "fed://home-.fed/",
            "fed://home.f/",
            "fed://home.fed1/",
            "fed://hóme.fed/",
            "fed://[::1]/",
            "http://home.fed/",
        ] {
            assert!(FedUrl::parse(bad).is_err(), "{bad}");
        }
        let long = format!("fed://home.fed/{}", "a".repeat(MAX_PATH));
        assert!(FedUrl::parse(&long).is_err());
    }

    #[test]
    fn the_http_spelling_maps_to_the_native_one() {
        let u = FedUrl::from_http("http://home.fed/about?x=1").unwrap();
        assert_eq!(u.to_url(), "fed://home.fed/about?x=1");
        assert_eq!(
            FedUrl::from_http("https://home.fed:8443").unwrap().to_url(),
            "fed://home.fed"
        );
        assert_eq!(
            FedUrl::from_http("http://www.example.com/"),
            None,
            "two labels"
        );
        assert_eq!(FedUrl::from_http("http://127.0.0.1/"), None);
        // Syntax only: `.com` parses, and the resolver's TLD list is what keeps it on the web.
        assert!(FedUrl::from_http("https://example.com/").is_some());
        assert_eq!(FedUrl::from_http("file:///home.fed"), None);
    }
}

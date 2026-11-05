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

/// The one spelling of a web host every Federate decision reads (the resolver's TLD rule, the http
/// mapping, webengine's origin gate): the root label dropped (`home.fed.` is `home.fed`, every
/// trailing dot) and ASCII lower case. Two readers that spelled it apart let `https://home.fed.`
/// past the fed-only rule (QA-A10-CS-2). Borrowed when `host` is already in that spelling.
pub fn web_host(host: &str) -> std::borrow::Cow<'_, str> {
    let host = host.trim_end_matches('.');
    if host.bytes().any(|b| b.is_ascii_uppercase()) {
        std::borrow::Cow::Owned(host.to_ascii_lowercase())
    } else {
        std::borrow::Cow::Borrowed(host)
    }
}

/// `(host, rest)` of an `http(s)` URL: the host as [`FedUrl::http_host`] gives it, and everything
/// from the first `/`, `?` or `#` after the authority.
fn http_parts(url: &str) -> Option<(String, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return None;
    }
    let (authority, rest) = split_authority(rest);
    // The host is what follows the last `@` (`http://home.fed@evil.com/` is evil.com).
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match hostport.rsplit_once(':') {
        Some((h, p)) if !hostport.ends_with(']') && p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => hostport,
    };
    Some((web_host(host).into_owned(), rest))
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
    /// (the network's compatibility spelling, `http://home.fed`); user info, port and a trailing
    /// dot of the host are dropped, as the protocol's gateway does. Syntax only: the caller decides
    /// whether the TLD is the network's.
    pub fn from_http(url: &str) -> Option<FedUrl> {
        let (host, rest) = http_parts(url)?;
        let (label, tld) = name(&host).ok()?;
        let (path, query) = tail(rest).ok()?;
        Some(FedUrl {
            label,
            tld,
            path,
            query,
        })
    }

    /// The host of an `http://` or `https://` URL as a DNS lookup would see it: lower case, without
    /// user info, port and trailing dot; `None` for another scheme.
    pub fn http_host(url: &str) -> Option<String> {
        http_parts(url).map(|(h, _)| h)
    }

    /// The `fed://` spelling of an `http(s)://` URL for any host, valid name or not: the host as
    /// [`FedUrl::http_host`] gives it, the path and query kept, the fragment dropped. A name that is
    /// not `label.tld` is then refused by the `fed:` handler with a page that says why, so a
    /// Federate TLD host is never left to the web.
    pub fn raw_fed(url: &str) -> Option<String> {
        let (host, rest) = http_parts(url)?;
        let rest = rest.split('#').next().unwrap_or("");
        Some(format!("fed://{host}{rest}"))
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

    #[test]
    fn the_host_of_an_http_url_is_the_one_a_lookup_would_use() {
        let h = FedUrl::http_host;
        assert_eq!(h("http://HOME.fed/x").as_deref(), Some("home.fed"));
        assert_eq!(h("https://home.fed.:8443/x").as_deref(), Some("home.fed"));
        assert_eq!(h("http://home.fed@evil.com/").as_deref(), Some("evil.com"));
        assert_eq!(h("http://evil.com@home.fed/").as_deref(), Some("home.fed"));
        assert_eq!(h("ftp://home.fed/"), None);
        // user info and a trailing dot do not keep a valid name from mapping.
        assert_eq!(
            FedUrl::from_http("http://u:p@home.fed./a")
                .unwrap()
                .to_url(),
            "fed://home.fed/a"
        );
    }

    #[test]
    fn a_host_that_is_not_a_name_is_still_spelled_fed() {
        assert_eq!(
            FedUrl::raw_fed("http://www.home.fed/a?b=1#f").as_deref(),
            Some("fed://www.home.fed/a?b=1")
        );
        assert!(FedUrl::parse("fed://www.home.fed/").is_err());
        assert_eq!(FedUrl::raw_fed("file:///x"), None);
    }
}

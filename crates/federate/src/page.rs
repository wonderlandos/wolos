// SPDX-License-Identifier: BSD-2-Clause
//! What the engine gets for one load: the verified bytes, or a page in plain words saying what
//! stopped the load. A failed check never shows the content, not even part of it.
use crate::resolve::Outcome;
use crate::verify::Layer;

/// One answer: an HTTP status, a content type, a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub mime: String,
    pub body: Vec<u8>,
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn layer_words(l: Layer) -> &'static str {
    match l {
        Layer::Root => "the network's root zone (signed by the Federate root key)",
        Layer::Tld => "the TLD record (signed by the Federate root key)",
        Layer::Domain => "the name's record (signed by the TLD's operator)",
        Layer::Manifest => "the site's manifest (signed by the name's owner)",
        Layer::Content => "the page's content (its BLAKE3 address)",
    }
}

/// The page for a load that did not end in content.
fn html(url: &str, title: &str, lines: &[String]) -> Vec<u8> {
    let mut body = String::new();
    for l in lines {
        body.push_str("<p>");
        body.push_str(&escape(l));
        body.push_str("</p>\n");
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>{t}</title>\n\
<style>:root{{color-scheme:light dark}}body{{font:16px/1.5 system-ui,sans-serif;max-width:40rem;\
margin:12vh auto;padding:0 1.5rem}}h1{{font-size:1.4rem}}code{{word-break:break-all}}</style>\n\
</head>\n<body>\n<h1>{t}</h1>\n<p><code>{u}</code></p>\n{body}</body>\n</html>\n",
        t = escape(title),
        u = escape(url),
    )
    .into_bytes()
}

/// The reply for `url` (canonical spelling) and how its load ended.
pub fn reply(url: &str, outcome: Outcome) -> Reply {
    let page = |status: u16, title: &str, lines: Vec<String>| Reply {
        status,
        mime: "text/html; charset=utf-8".into(),
        body: html(url, title, &lines),
    };
    match outcome {
        Outcome::Content { mime, bytes, .. } => Reply {
            status: 200,
            mime,
            body: bytes,
        },
        Outcome::Refused(f) => page(
            502,
            "This Federate page did not pass its checks",
            vec![
                format!("What failed: {}, for {}.", layer_words(f.layer), f.subject),
                format!("Why: {}.", f.reason),
                concat!(
                    "Nothing from this page was shown. WolOS opens a Federate page only when every ",
                    "signature and every hash checks out, so a server cannot change what the page's ",
                    "owner published."
                )
                .into(),
            ],
        ),
        Outcome::NotFound(why) => page(404, "This Federate page does not exist", vec![why]),
        Outcome::Unavailable(why) => page(410, "This Federate name does not resolve now", vec![why]),
        Outcome::Unsupported(why) => page(501, "WolOS cannot open this Federate name yet", vec![why]),
        Outcome::Network(why) => page(
            504,
            "The Federate Network could not be reached",
            vec![
                format!("{why}."),
                "Check the connection and load the page again.".into(),
            ],
        ),
        Outcome::BadAddress(why) => page(400, "This is not a Federate address", vec![format!("{why}.")]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::Failure;

    #[test]
    fn a_refused_load_says_which_layer_and_why_and_shows_nothing_else() {
        let r = reply(
            "fed://home.fed/<x>",
            Outcome::Refused(Failure {
                layer: Layer::Content,
                subject: "home.fed/index.html".into(),
                reason: "the bytes hash to 00, not to their address".into(),
            }),
        );
        assert_eq!(r.status, 502);
        assert!(r.mime.starts_with("text/html"));
        let t = String::from_utf8(r.body).unwrap();
        assert!(t.contains("did not pass its checks"));
        assert!(t.contains("the page&#39;s content (its BLAKE3 address), for home.fed/index.html"));
        assert!(
            t.contains("fed://home.fed/&lt;x&gt;"),
            "the address is escaped"
        );
    }

    #[test]
    fn content_is_passed_through_untouched() {
        let r = reply(
            "fed://home.fed",
            Outcome::Content {
                mime: "text/css; charset=utf-8".into(),
                bytes: b"a{}".to_vec(),
                address: "x".into(),
            },
        );
        assert_eq!(
            (r.status, r.mime.as_str(), r.body.as_slice()),
            (200, "text/css; charset=utf-8", &b"a{}"[..])
        );
    }
}

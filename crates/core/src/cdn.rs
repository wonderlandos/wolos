// SPDX-License-Identifier: BSD-2-Clause
//! The CDN server's addresses, in one place (wonderlandos/wolos-cdn, on Square Cloud). One server,
//! two names: [`PRIMARY`] (`cdn.wolos.org`, the project's own domain, a CNAME to the server) and
//! [`MIRROR`] (`wolos.squareweb.app`, the name Square Cloud gave it). Every reader tries
//! [`HOSTS`] in order, so a machine keeps working while the new name's DNS is not live, and later if
//! the domain ever lapses. Files the release tooling (wolos-build) also attaches to the latest
//! wolos-packages release are read from GitHub last ([`GITHUB_LATEST`]).
//!
//! Readers: the release feed (`wopkg::FEEDS`, `FEEDS_MIRROR`, `FEEDS_FALLBACK` = [`LATEST`] in
//! order), the daily update check (`files/usr/lib/wolos/update-check`: `URL`, `MIRROR`,
//! `FALLBACK` = [`latest`]`("release.json")`) and error reports (`files/usr/lib/wolos/report`:
//! [`REPORT`]). The shell scripts cannot import this; tests hold their defaults to it. The
//! `[wolos]` package repo is CloudFront (ADR 0030), not this server.
//!
//! Also here, as the one place for the project's own addresses: [`SHARD_INDEX`], the Crystal shard
//! index (a Cloudflare Worker on the project's domain, not the CDN server).

/// `https://cdn.wolos.org` as a literal, so `concat!` can build the constants below.
macro_rules! primary {
    () => {
        "https://cdn.wolos.org"
    };
}
/// `https://wolos.squareweb.app` as a literal.
macro_rules! mirror {
    () => {
        "https://wolos.squareweb.app"
    };
}

/// The server under the project's domain: tried first.
pub const PRIMARY: &str = primary!();
/// The same server under its Square Cloud name: tried when [`PRIMARY`] answers 404, an error or
/// cannot be reached (DNS not live, certificate not issued yet).
pub const MIRROR: &str = mirror!();
/// The server's names, in the order tried.
pub const HOSTS: [&str; 2] = [PRIMARY, MIRROR];
/// The latest wolos-packages release's assets on GitHub (it redirects to the asset): the last
/// place for the files the release tooling attaches there too (the feeds, `release.json`).
pub const GITHUB_LATEST: &str =
    "https://github.com/wonderlandos/wolos-packages/releases/latest/download";
/// Bases of the newest published files (`latest/` on the server, written after each publication
/// is uploaded and verified), in the order tried: both names of the server, then GitHub.
pub const LATEST: [&str; 3] = [
    concat!(primary!(), "/latest"),
    concat!(mirror!(), "/latest"),
    GITHUB_LATEST,
];
/// Where an error report is posted, in the order tried (the server keeps them, `GET /report`).
pub const REPORT: [&str; 2] = [
    concat!(primary!(), "/report"),
    concat!(mirror!(), "/report"),
];

/// The revocation database's checkpoint (`{"format":1,"serial":N,"sha512_256":".."}`, ADR 0046
/// "RKDB"), published twice and read from both: the CDN under the project's domain, and the
/// public log, the hub-index repository on GitHub (its history is public and append-only in
/// practice). They must agree; when they do not, `wol trust update` warns and keeps the higher
/// serial as its floor.
pub const RKDB_CHECKPOINTS: [&str; 2] = [
    concat!(primary!(), "/plugins/revoked.checkpoint.json"),
    "https://raw.githubusercontent.com/wonderlandos/hub-index/main/revoked.checkpoint.json",
];

/// The Crystal shard index (`index.json`, its signature at `index.json.sig`): a Cloudflare Worker
/// on the project's domain serving the `wolos-shards` repository's published files. wopkg's shard
/// index (`wopkg::index::shards::DEFAULT_URL`) reads it.
pub const SHARD_INDEX: &str = "https://crystal.wolos.org/index.json";

/// `file` in each [`LATEST`] base, in the order tried.
pub fn latest(file: &str) -> Vec<String> {
    LATEST
        .iter()
        .map(|b| format!("{b}/{}", file.trim_start_matches('/')))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The project's domain first, the server's own name second, GitHub last; all over TLS, no
    /// trailing slash to double.
    #[test]
    fn the_new_name_comes_first_then_the_old_then_github() {
        assert_eq!(
            HOSTS,
            ["https://cdn.wolos.org", "https://wolos.squareweb.app"]
        );
        assert_eq!(
            LATEST,
            [
                "https://cdn.wolos.org/latest",
                "https://wolos.squareweb.app/latest",
                "https://github.com/wonderlandos/wolos-packages/releases/latest/download",
            ]
        );
        assert_eq!(
            REPORT,
            [
                "https://cdn.wolos.org/report",
                "https://wolos.squareweb.app/report"
            ]
        );
        for u in HOSTS.iter().chain(&LATEST).chain(&REPORT) {
            assert!(u.starts_with("https://") && !u.ends_with('/'), "{u}");
        }
        assert_eq!(SHARD_INDEX, "https://crystal.wolos.org/index.json");
        assert_eq!(
            latest("/release.json"),
            [
                "https://cdn.wolos.org/latest/release.json",
                "https://wolos.squareweb.app/latest/release.json",
                "https://github.com/wonderlandos/wolos-packages/releases/latest/download/release.json",
            ]
        );
    }
}

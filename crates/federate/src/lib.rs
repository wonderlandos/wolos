// SPDX-License-Identifier: BSD-2-Clause
//! Native support for Federate Network addresses (ADR 0052): `fed://<label>.<tld>[/path][?query]`
//! opens in WolOS's web engine with no extension and no setting.
//!
//! The chain, every layer checked before the next is used:
//! root zone (signed by the pinned root key) -> TLD record (root-signed) -> domain record (signed
//! by the TLD's operator key) -> manifest (fetched by its BLAKE3 address, signed by the domain
//! owner's key) -> content block (fetched by its BLAKE3 address). Bytes that fail a check are
//! never served: the page says which layer failed and why ([`page`]).
//!
//! Where bytes come from is a [`Source`]; the network's HTTPS door ([`Https`]) is the one that
//! works today. A source is untrusted: it can refuse, never forge.
#![forbid(unsafe_code)]
pub mod canon;
pub mod page;
pub mod resolve;
pub mod source;
pub mod state;
pub mod time;
pub mod uri;
pub mod verify;

pub use page::Reply;
pub use resolve::{Load, Outcome, Resolver};
pub use source::{Https, Source};
pub use uri::FedUrl;
pub use verify::{Failure, Layer};

/// The network's root public key (Ed25519, hex), pinned: the trust anchor every signature chains
/// to. Read from the network's bootstrap answer and its signed root zone (2026-11-04,
/// `root_version` 1783402193). A new root key comes with a WolOS update, never from the network.
pub const ROOT_KEY: &str = "f7942f5c05b03a58ad81140ea22d279d524866bf972c177582d5f114a6ffe5e6";

/// The network's HTTPS compatibility door (`/v1/root`, `/v1/manifest/{hash}`, `/v1/block/{hash}`).
pub const BOOTSTRAP: &str = "https://federate.network";

/// The URL scheme.
pub const SCHEME: &str = "fed";

/// The answer for one `fed://` load (or a mapped `http(s)://` one), from the process's resolver:
/// verified content, or a page that says what failed. Any thread; blocks on the network.
pub fn serve(url: &str) -> Reply {
    resolve::shared().serve(url)
}

/// [`serve`] for a load of `kind` (a subresource has the smaller block cap).
pub fn serve_load(url: &str, kind: Load) -> Reply {
    resolve::shared().serve_load(url, kind)
}

/// The `fed://` spelling of an `http(s)://` URL whose host ends in a Federate TLD (the network's built-in
/// list): always, there is no setting. Never touches the
/// network. A host that is not `label.tld` is mapped as it stands, and the `fed:` handler refuses
/// it with a page that says why.
pub fn map_http(url: &str) -> Option<String> {
    resolve::shared().map_http(url)
}

/// Whether `host` (lower case, no port) ends in a Federate TLD of the process's resolver: such a
/// host is never looked up in DNS and never fetched over http(s).
pub fn is_fed_host(host: &str) -> bool {
    resolve::shared().is_fed_host(host)
}

// SPDX-License-Identifier: BSD-2-Clause
//! Search by meaning, on the device (docs/modules/search.md). A static token-embedding table
//! (`.wst`, [`Table`]) turns a text into one unit vector; an [`Index`] holds the vectors of the
//! items to find (settings rows, manual sections, verbs) and ranks them by cosine, fused with a
//! keyword score ([`rrf`]); a [`Procrustes`] map carries a query from another language's
//! vectors to the English ones. Pure Rust, no network, no model runtime, no `unsafe`.
#![forbid(unsafe_code)]

mod b64;
mod corpus;
mod fuzzy;
mod index;
mod kw;
mod map;
mod system;
mod table;

pub use b64::decode as base64_decode;

/// Characters of a query that count: a pasted page of text is cut here before any ranking,
/// spelling or embedding work, which grows with the length (100 000 characters took seconds).
pub const QUERY_MAX: usize = 64;

/// `q` cut to [`QUERY_MAX`] characters (on a character boundary).
pub fn capped(q: &str) -> &str {
    q.char_indices().nth(QUERY_MAX).map_or(q, |(i, _)| &q[..i])
}

pub use corpus::Corpus;
pub use index::{Hit, Index, ItemIn, Opts, rrf};
pub use map::Procrustes;
pub use system::{DIMS, DIR, Installed, installed, is_installed};
pub use table::{MAX_TOKENS, MAX_WORD, Table, tokenize_words};

/// What went wrong reading a table, an index or a map: one sentence for a log, never shown to a
/// person (the callers fall back to keyword search).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

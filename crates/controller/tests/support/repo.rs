// SPDX-License-Identifier: GPL-2.0-or-later
//! The wolos checkout the contract tests read, by repo-relative path. Include it with
//! `#[path = "support/repo.rs"] mod repo;`. The overlaid three-repo tree is `siblings::root()`.
#![allow(dead_code)] // each test file that includes this module calls only some of its helpers
use std::path::PathBuf;

/// `<wolos checkout>/<p>`.
pub fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

/// The text of `repo(p)`; panics naming the file when it cannot be read.
pub fn read(p: &str) -> String {
    std::fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

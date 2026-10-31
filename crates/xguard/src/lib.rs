// SPDX-License-Identifier: GPL-2.0-or-later
//! libxguard (ADR 0022): checks a file against signed signature data, without a resident scanner.
//! `automaton` + `db`: the memory-mapped signature file; `sigs`: reads public signature lists;
//! `scan`: one file's bytes and the stable [`Report`]; `walk`: files and what is packed in them.
pub mod automaton;
pub mod db;
pub mod ffi;
pub mod scan;
pub mod sigs;
pub mod walk;
#[cfg(feature = "yara")]
pub mod yara;

pub use scan::{Deep, Engine, Finding, Report, Skipped, Verdict};
pub use walk::{Limits, scan_bytes, scan_file};

/// Signatures delivered by updates.
pub const RULES_DIR: &str = "/var/lib/wolos/xguard/rules";
/// Signatures the system shipped with.
pub const SHIPPED_DIR: &str = "/usr/share/wolos/xguard";

/// The newest signatures on this machine: delivered ones, else the shipped ones.
pub fn rules_dir() -> std::path::PathBuf {
    let d = std::path::Path::new(RULES_DIR);
    let usable = std::fs::read_dir(d).is_ok_and(|mut e| {
        e.any(|f| f.is_ok_and(|f| f.path().extension().is_some_and(|x| x == "xgdb")))
    });
    if usable {
        d
    } else {
        std::path::Path::new(SHIPPED_DIR)
    }
    .to_path_buf()
}

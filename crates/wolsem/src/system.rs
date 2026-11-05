// SPDX-License-Identifier: BSD-2-Clause
//! What the image ships for search, under `/usr/share/wolos/manual/`: `static-mrl.wst` (the
//! table), `sections.json` (the manual's sections, [`crate::Corpus`]) and, optionally,
//! `map.<lang>.wprc` (a [`crate::Procrustes`] map for a language the table keeps apart).
//! Loaded once per process; a computer without the files searches by words only.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::{Corpus, Error, Index, Procrustes, Table};

/// Where the image puts the files.
pub const DIR: &str = "/usr/share/wolos/manual";
/// The vector size used for search (the table's own, cut to this at most).
pub const DIMS: usize = 256;

pub struct Installed {
    pub table: Table,
    pub manual: Corpus,
    pub manual_index: Index,
    dir: PathBuf,
}

impl Installed {
    /// Loads `dir`; the table is needed, the manual is not (an empty one then).
    pub fn load(dir: &Path) -> Result<Installed, Error> {
        let table = Table::open(&dir.join("static-mrl.wst"))?;
        // A manual that cannot be read costs the manual rows, not the table.
        let manual = std::fs::read_to_string(dir.join("sections.json"))
            .ok()
            .and_then(|t| Corpus::from_json(&t).ok())
            .map_or_else(|| Corpus::from_json(r#"{"sections":[]}"#), Ok)?;
        Ok(Installed::from_parts(table, manual, dir))
    }

    /// An `Installed` from a table and a manual already in memory (tests, build tools).
    pub fn from_parts(table: Table, manual: Corpus, dir: &Path) -> Installed {
        let manual_index = manual.index(&table, DIMS);
        Installed {
            table,
            manual,
            manual_index,
            dir: dir.to_path_buf(),
        }
    }

    /// The map for a query in `lang` (`pt-BR`), if the image ships one.
    pub fn map_for(&self, lang: &str) -> Option<Procrustes> {
        if lang.is_empty() || lang.contains(['/', '.']) {
            return None;
        }
        let b = std::fs::read(self.dir.join(format!("map.{lang}.wprc"))).ok()?;
        Procrustes::from_bytes(&b).ok()
    }
}

static INSTALLED: OnceLock<Option<Installed>> = OnceLock::new();

/// The shipped table and manual, loaded on first use (tens of milliseconds: call it off the
/// interface thread); None where they are not installed.
pub fn installed() -> Option<&'static Installed> {
    INSTALLED
        .get_or_init(|| Installed::load(Path::new(DIR)).ok())
        .as_ref()
}

/// Whether the shipped files are there, without loading them.
pub fn is_installed() -> bool {
    Path::new(DIR).join("static-mrl.wst").is_file()
}

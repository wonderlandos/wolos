// SPDX-License-Identifier: BSD-2-Clause
//! `wolkit-stylec`: compile `.wolcss` sheets into the `WSB1` style table and the Crystal constants
//! of the shard. `wol app build` runs it. It uses the parser the engine's dev loader uses.
//!
//! ```text
//! wolkit-stylec [--out build/styles.wsb] [--crystal src/gen/styles.cr] [--spdx ID] [--deny-warnings] SHEET...
//! wolkit-stylec --doc          print docs/modules/wolkit-style.md
//! ```
//! Exit status: 0 compiled, 1 the sheets have errors (or warnings with `--deny-warnings`), 2 the
//! command line is wrong or a file cannot be read or written. `--spdx` is the licence of the app the
//! Crystal file belongs to (default `BSD-2-Clause`, the app template's).
use std::process::ExitCode;
use wolkit::style::{compile_sources, doc, wsb};

fn usage() -> ExitCode {
    eprintln!(
        "usage: wolkit-stylec [--out FILE.wsb] [--crystal FILE.cr] [--spdx ID] [--deny-warnings] SHEET.wolcss..."
    );
    eprintln!("       wolkit-stylec --doc");
    ExitCode::from(2)
}

/// An SPDX expression as a line can hold it: ids, `AND`, `OR`, `WITH`, parentheses.
fn spdx_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+() ".contains(&b))
}

fn main() -> ExitCode {
    let mut out = None;
    let mut crystal = None;
    let mut spdx = String::from("BSD-2-Clause");
    let mut deny = false;
    let mut files: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--doc" => {
                print!("{}", doc::render());
                return ExitCode::SUCCESS;
            }
            "--out" => match args.next() {
                Some(v) => out = Some(v),
                None => return usage(),
            },
            "--crystal" => match args.next() {
                Some(v) => crystal = Some(v),
                None => return usage(),
            },
            "--spdx" => match args.next() {
                Some(v) if spdx_ok(&v) => spdx = v,
                _ => return usage(),
            },
            "--deny-warnings" => deny = true,
            "-h" | "--help" => return usage(),
            s if s.starts_with("--") => {
                eprintln!("wolkit-stylec: `{s}` is not an option.");
                return usage();
            }
            _ => files.push(a),
        }
    }
    if files.is_empty() {
        return usage();
    }
    // The same sheets always give the same table, whatever order a shell glob lists them in.
    files.sort();
    let mut sources = Vec::with_capacity(files.len());
    for f in &files {
        match std::fs::read_to_string(f) {
            Ok(s) => sources.push((f.clone(), s)),
            Err(e) => {
                eprintln!("wolkit-stylec: {f} cannot be read: {e}.");
                return ExitCode::from(2);
            }
        }
    }
    let refs: Vec<(&str, &str)> = sources
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    match compile_sources(&refs) {
        Ok(c) => {
            for w in &c.warnings {
                eprint!("{}", w.render());
            }
            if deny && !c.warnings.is_empty() {
                eprintln!(
                    "wolkit-stylec: {} warning(s) and --deny-warnings is on.",
                    c.warnings.len()
                );
                return ExitCode::from(1);
            }
            if let Some(path) = out
                && let Err(e) = std::fs::write(&path, &c.bytes)
            {
                eprintln!("wolkit-stylec: {path} cannot be written: {e}.");
                return ExitCode::from(2);
            }
            if let Some(path) = crystal
                && let Err(e) = std::fs::write(&path, wsb::crystal_constants(&c.table, &spdx))
            {
                eprintln!("wolkit-stylec: {path} cannot be written: {e}.");
                return ExitCode::from(2);
            }
            eprintln!(
                "wolkit-stylec: {} classes, {} bytes.",
                c.table.classes.len(),
                c.bytes.len()
            );
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                eprint!("{}", d.render());
            }
            let errors = diags.iter().filter(|d| d.is_error()).count();
            eprintln!("wolkit-stylec: {errors} error(s); no table was written.");
            ExitCode::from(1)
        }
    }
}

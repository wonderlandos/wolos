// SPDX-License-Identifier: BSD-2-Clause
//! A unit's directory, `locale/<unit>/`: `en.tsv`, one `<lang>.tsv` per translation, and, for a
//! unit whose English is still extracted from Rust, `sources.tsv` (`path  module`, the path
//! relative to the directory). The build (`build.rs`), `wol-i18n` and the tests read it here.
use super::compile::{self, Problem, Unit};
use super::tsv::{self, LangRow, SourceRow};
use super::{Lang, extract};
use std::path::{Path, PathBuf};

/// The unit's name: the directory's name.
pub fn unit_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn lines_err(p: &Path, e: Vec<tsv::LineError>) -> String {
    e.iter()
        .map(|(n, why)| format!("{}:{n}: {why}", p.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines of `text` whose first field is `key` or one of its plural forms (`key#one`): their
/// numbers from 1. A leading byte-order mark is not part of the first line.
fn key_lines(text: &str, key: &str) -> Vec<usize> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.starts_with('#'))
        .filter(|(_, l)| {
            let k = l.split('\t').next().unwrap_or_default();
            k == key || k.split('#').next() == Some(key)
        })
        .map(|(i, _)| i + 1)
        .collect()
}

/// Problems as lines for a terminal or a build log: `file:LINE: error: key: what`, the line of
/// the row in `file` (for a key listed twice, the second row, with the first named).
pub fn problems_text(file: &Path, problems: &[Problem]) -> String {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    problems
        .iter()
        .map(|p| {
            let level = if p.error { "error" } else { "warning" };
            let lines = key_lines(&text, &p.key);
            let (at, also) = match lines.as_slice() {
                [] => (String::new(), String::new()),
                [one] => (format!(":{one}"), String::new()),
                [first, second, ..] if p.what.contains("listed twice") => {
                    (format!(":{second}"), format!(" (first on line {first})"))
                }
                [first, ..] => (format!(":{first}"), String::new()),
            };
            format!(
                "{}{at}: {level}: {}: {}{also}",
                file.display(),
                p.key,
                p.what
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The source files of `dir/sources.tsv`: (path, module). Ok(empty): no sources.tsv (en.tsv is
/// the source). Err: a file that cannot be read or a line that is not `path  module`.
pub fn sources(dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let p = dir.join("sources.tsv");
    if !p.exists() {
        return Ok(Vec::new());
    }
    let text = read(&p)?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((path, module)) = line.split_once('\t') else {
            return Err(format!(
                "{}:{}: expected “path<TAB>module”",
                p.display(),
                i + 1
            ));
        };
        out.push((dir.join(path), module.trim().to_string()));
    }
    Ok(out)
}

/// en.tsv as the sources make it (every copy module's string consts and every file's
/// translating calls, in sources.tsv order).
/// Err: a source file that cannot be read or holds a const that is not one literal.
pub fn extract_dir(dir: &Path) -> Result<Vec<SourceRow>, String> {
    let mut rows = Vec::new();
    for (path, module) in sources(dir)? {
        let src = read(&path)?;
        // String consts are text only in a copy module (`copy_*.rs`); elsewhere a const is a
        // name or an id, and only the translating calls are read.
        let copy_module = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("copy"));
        if copy_module {
            rows.extend(extract::consts(&module, &src)?);
        }
        // Text a surface translates where it uses it: a row unless the unit has that English
        // already (a plural call is one group, `#one` then `#other`, under one key).
        let mut groups: Vec<Vec<SourceRow>> = Vec::new();
        for r in extract::calls(&module, &src)? {
            let joins = |g: &Vec<SourceRow>| {
                r.key.ends_with("#other") && g[0].key.ends_with("#one") && g.len() == 1
            };
            if let Some(g) = groups.last_mut().filter(|g| joins(g)) {
                g.push(r);
            } else {
                groups.push(vec![r]);
            }
        }
        for g in groups {
            let plural = g.len() > 1;
            if !plural
                && rows
                    .iter()
                    .any(|x| x.source == g[0].source && !x.key.contains('#'))
            {
                continue;
            }
            let base = g[0].key.split('#').next().unwrap_or_default().to_string();
            let (mut key, mut i) = (base.clone(), 2);
            while rows
                .iter()
                .any(|x| x.key.split('#').next() == Some(key.as_str()))
            {
                key = format!("{base}_{i}");
                i += 1;
            }
            for r in g {
                let form = r
                    .key
                    .split_once('#')
                    .map(|f| format!("#{}", f.1))
                    .unwrap_or_default();
                rows.push(SourceRow {
                    key: format!("{key}{form}"),
                    ..r
                });
            }
        }
    }
    apply_budgets(dir, &mut rows)?;
    Ok(rows)
}

/// `dir/budgets.tsv` (`key<TAB>columns`, `#` comments): the length budget of rows whose source
/// has none of its own (a text written where it is used, a label in a call). A key the unit does
/// not have is an error, so the file cannot outlive the texts it bounds.
fn apply_budgets(dir: &Path, rows: &mut [SourceRow]) -> Result<(), String> {
    let p = dir.join("budgets.tsv");
    if !p.exists() {
        return Ok(());
    }
    let text = read(&p)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let at = format!("{}:{}", p.display(), i + 1);
        let (key, cols) = line
            .split_once('\t')
            .ok_or_else(|| format!("{at}: expected “key<TAB>columns”"))?;
        let cols: u32 = cols
            .trim()
            .parse()
            .map_err(|_| format!("{at}: “{}” is not a number of columns", cols.trim()))?;
        let row = rows
            .iter_mut()
            .find(|r| r.key == key)
            .ok_or_else(|| format!("{at}: no text has the key {key}"))?;
        if row.budget.is_none() {
            row.budget = Some(cols);
        }
    }
    Ok(())
}

/// en.tsv's text for rows extracted from `dir`'s sources.
pub fn extracted_text(dir: &Path, rows: &[SourceRow]) -> String {
    tsv::write_source(
        &unit_name(dir),
        "Generated by `wol-i18n extract` from the copy modules in sources.tsv: change the module, not this file.",
        rows,
    )
}

/// The checked English of `dir`. Err: en.tsv missing, unreadable, or breaking a source rule.
pub fn load_unit(dir: &Path) -> Result<Unit, String> {
    let p = dir.join("en.tsv");
    let rows = tsv::parse_source(&read(&p)?).map_err(|e| lines_err(&p, e))?;
    Unit::new(rows).map_err(|e| problems_text(&p, &e))
}

/// The path of `<lang>.tsv` in `dir`.
pub fn lang_file(dir: &Path, l: Lang) -> PathBuf {
    dir.join(format!("{}.tsv", l.tag()))
}

/// The rows of `<lang>.tsv`; Ok(empty) when there is no such file (and for English, whose file is
/// the source). Err: a line that is not a row.
pub fn load_lang(dir: &Path, l: Lang) -> Result<Vec<LangRow>, String> {
    let p = lang_file(dir, l);
    // en.tsv is the source (key, English, comment, budget), not a translation of itself.
    if l == Lang::En || !p.exists() {
        return Ok(Vec::new());
    }
    tsv::parse_lang(&read(&p)?).map_err(|e| lines_err(&p, e))
}

/// Every table of `dir`: (language, bytes, warnings as text) for English, `xx` and each
/// translation that has at least one row translated. Err: every error, as text (the build stops).
pub fn compile_dir(dir: &Path) -> Result<Vec<(Lang, Vec<u8>, String)>, String> {
    let unit = load_unit(dir)?;
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for l in Lang::ALL {
        let rows = load_lang(dir, l)?;
        let c = compile::compile(&unit, l, &rows).map_err(|e| e.to_string())?;
        let text = problems_text(&lang_file(dir, l), &c.problems);
        if !c.ok() {
            errors.push(text);
            continue;
        }
        if matches!(l, Lang::En | Lang::Xx) || c.translated > 0 {
            out.push((l, c.bytes, text));
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_directory_extracts_compiles_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("wol-i18n-unit-{}", std::process::id()));
        let unit = dir.join("demo");
        std::fs::create_dir_all(&unit).expect("temp dir");
        std::fs::write(
            unit.join("copy.rs"),
            "//! Demo.\n/// Title.\n/// budget: 12\npub const TITLE: &str = \"Hello\";\n",
        )
        .expect("copy.rs");
        std::fs::write(unit.join("sources.tsv"), "# path\tmodule\ncopy.rs\tdemo\n")
            .expect("sources");
        let rows = extract_dir(&unit).expect("extract");
        std::fs::write(unit.join("en.tsv"), extracted_text(&unit, &rows)).expect("en.tsv");
        std::fs::write(
            unit.join("de.tsv"),
            "# key\tsource\ttext\thash\ndemo.title\tHello\tHallo\t\n",
        )
        .expect("de.tsv");
        // The fixture's budget sits in en.tsv's fourth column: English is never read as a translation.
        assert!(load_lang(&unit, Lang::En).expect("en rows").is_empty());
        let tables = compile_dir(&unit).expect("compile");
        assert!(
            tables[0].2.is_empty(),
            "English has no warnings: {}",
            tables[0].2
        );
        let langs: Vec<Lang> = tables.iter().map(|t| t.0).collect();
        assert_eq!(langs, [Lang::En, Lang::De, Lang::Xx]);
        std::fs::write(unit.join("fr.tsv"), "demo.title\tHello\tBonjour {x}\t\n").expect("fr.tsv");
        let e = compile_dir(&unit).unwrap_err();
        assert!(
            e.contains("fr.tsv:1: error: demo.title: placeholders"),
            "{e}"
        );
        for f in ["copy.rs", "sources.tsv", "en.tsv", "de.tsv", "fr.tsv"] {
            std::fs::remove_file(unit.join(f)).expect("cleanup");
        }
        std::fs::remove_dir(&unit).expect("cleanup");
        std::fs::remove_dir(&dir).expect("cleanup");
    }

    #[test]
    fn a_problem_names_the_line_and_a_duplicate_names_both() {
        let dir = std::env::temp_dir().join(format!("wol-i18n-lines-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let f = dir.join("fr.tsv");
        std::fs::write(
            &f,
            "\u{feff}# header\na.one\tOne {n}\tUn\t\na.two\tTwo\tDeux\t\na.one\tOne {n}\tUn {n}\t\n",
        )
        .expect("write");
        let p = |key: &str, what: &str| Problem {
            error: true,
            key: key.into(),
            what: what.into(),
        };
        let text = problems_text(
            &f,
            &[
                p("a.one", "the placeholders differ"),
                p("a.two", "wrong"),
                p("a.one", "this key is listed twice"),
                p("a.none", "no such row"),
            ],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains("fr.tsv:2: error: a.one: the placeholders differ"));
        assert!(lines[1].contains("fr.tsv:3: error: a.two"));
        assert!(
            lines[2].contains("fr.tsv:4: error: a.one: this key is listed twice (first on line 2)")
        );
        assert!(lines[3].contains("fr.tsv: error: a.none"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn budgets_tsv_bounds_rows_that_have_none_and_refuses_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("wol-i18n-budgets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("copy.rs"),
            "/// Own.\n/// budget: 12\npub const OWN: &str = \"Hello\";\npub const FREE: &str = \"World\";\n",
        )
        .expect("copy.rs");
        std::fs::write(dir.join("sources.tsv"), "copy.rs\tdemo\n").expect("sources");
        std::fs::write(
            dir.join("budgets.tsv"),
            "# key\tcolumns\ndemo.own\t99\ndemo.free\t14\n",
        )
        .expect("budgets");
        let rows = extract_dir(&dir).expect("extract");
        let budget = |k: &str| rows.iter().find(|r| r.key == k).and_then(|r| r.budget);
        assert_eq!(budget("demo.own"), Some(12), "the source's own budget wins");
        assert_eq!(budget("demo.free"), Some(14));
        std::fs::write(dir.join("budgets.tsv"), "demo.gone\t9\n").expect("budgets");
        let e = extract_dir(&dir).unwrap_err();
        assert!(
            e.contains("budgets.tsv:1") && e.contains("demo.gone"),
            "{e}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

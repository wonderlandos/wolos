// SPDX-License-Identifier: BSD-2-Clause
//! `wol-i18n`: the locale tool (ADR 0045) for developers and translators, over `locale/<unit>/`.
//!
//! ```text
//! wol-i18n extract <unit-dir>...        en.tsv from the copy modules in sources.tsv
//! wol-i18n check <unit-dir>...          en.tsv matches its sources and every language compiles
//! wol-i18n status <unit-dir>...         coverage per language: translated, stale, errors
//! wol-i18n sync <unit-dir> <lang>|all   (re)write <lang>.tsv: every row, the current English
//! wol-i18n compile <unit-dir> <out-dir> <out-dir>/<unit>.<lang>.wtx for a shipped app
//! wol-i18n pseudo <text>                the text as the pseudo-locale xx shows it
//! ```
//! Exit status: 0 done, 1 a check found errors, 2 usage or a file that cannot be read or written.
use shared::i18n::{Lang, compile, files, status, tsv};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: wol-i18n extract|check|status <unit-dir>... | sync <unit-dir> <lang>|all | compile <unit-dir> <out-dir> | pseudo <text>";

fn write(p: &Path, text: &str) -> Result<(), String> {
    std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
}

fn extract(dirs: &[PathBuf]) -> Result<bool, String> {
    for d in dirs {
        let rows = files::extract_dir(d)?;
        write(&d.join("en.tsv"), &files::extracted_text(d, &rows))?;
        println!("{}: {} rows", d.join("en.tsv").display(), rows.len());
    }
    Ok(true)
}

fn check(dirs: &[PathBuf]) -> Result<bool, String> {
    let mut ok = true;
    for d in dirs {
        if !files::sources(d)?.is_empty() {
            let want = files::extracted_text(d, &files::extract_dir(d)?);
            let have = std::fs::read_to_string(d.join("en.tsv")).unwrap_or_default();
            if want != have {
                ok = false;
                eprintln!(
                    "{}: out of date: run wol-i18n extract {}",
                    d.join("en.tsv").display(),
                    d.display()
                );
            }
        }
        match files::compile_dir(d) {
            Ok(tables) => {
                for (l, _, warnings) in tables {
                    if !warnings.is_empty() {
                        eprintln!("{warnings}");
                    }
                    println!("{}: {} ok", files::unit_name(d), l.tag());
                }
            }
            Err(e) => {
                ok = false;
                eprintln!("{e}");
            }
        }
    }
    Ok(ok)
}

fn status_of(dirs: &[PathBuf]) -> Result<bool, String> {
    println!("unit\tlang\trows\ttranslated\t%\tstale\terrors\twarnings");
    let mut ok = true;
    for d in dirs {
        let unit = files::load_unit(d)?;
        for l in Lang::SHIPPED {
            let rows = files::load_lang(d, l)?;
            let (c, _) = status::coverage(&unit, l, &rows).map_err(|e| e.to_string())?;
            ok &= c.errors == 0;
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                files::unit_name(d),
                l.tag(),
                c.rows,
                c.translated,
                c.percent(),
                c.stale,
                c.errors,
                c.warnings
            );
        }
    }
    Ok(ok)
}

fn sync(dir: &Path, which: &str) -> Result<bool, String> {
    let langs: Vec<Lang> = if which == "all" {
        Lang::SHIPPED.into_iter().skip(1).collect()
    } else {
        vec![
            Lang::from_tag(which)
                .filter(|l| !matches!(l, Lang::En | Lang::Xx))
                .ok_or(format!("{which}: not a language to translate into"))?,
        ]
    };
    let unit = files::load_unit(dir)?;
    for l in langs {
        let (rows, dropped) = status::sync(&unit, l, &files::load_lang(dir, l)?);
        let p = files::lang_file(dir, l);
        write(&p, &tsv::write_lang(&files::unit_name(dir), l, &rows))?;
        println!(
            "{}: {} rows, {dropped} dropped (English no longer has them)",
            p.display(),
            rows.len()
        );
    }
    Ok(true)
}

fn compile_to(dir: &Path, out: &Path) -> Result<bool, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let unit = files::unit_name(dir);
    for (l, bytes, _) in files::compile_dir(dir)? {
        let p = out.join(format!("{unit}.{}.wtx", l.tag()));
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        println!("{}", p.display());
    }
    Ok(true)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dirs = || args.iter().skip(1).map(PathBuf::from).collect::<Vec<_>>();
    let run = match args.first().map(String::as_str) {
        Some("extract") if args.len() > 1 => extract(&dirs()),
        Some("check") if args.len() > 1 => check(&dirs()),
        Some("status") if args.len() > 1 => status_of(&dirs()),
        Some("sync") if args.len() == 3 => sync(Path::new(&args[1]), &args[2]),
        Some("compile") if args.len() == 3 => compile_to(Path::new(&args[1]), Path::new(&args[2])),
        Some("pseudo") if args.len() == 2 => {
            println!("{}", compile::pseudo(&args[1]));
            Ok(true)
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("wol-i18n: {e}");
            ExitCode::from(2)
        }
    }
}

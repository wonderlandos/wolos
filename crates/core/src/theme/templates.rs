// SPDX-License-Identifier: BSD-2-Clause
//! User templates (ADR 0031 themes v2): every `$XDG_CONFIG_HOME/wolos/themed/<name>.tpl` is
//! rendered on each theme switch to `<toolkit dir>/themed/<name>`, so any app the system does not
//! know can follow the theme. Placeholders: `{{token}}` (a theme token, `#rrggbb`), `{{color0}}`
//! .. `{{color15}}` and the ANSI names (`{{brightRed}}`), `{{background}}`, `{{foreground}}`,
//! `{{cursor}}`, `{{selection}}`, `{{id}}`, `{{name}}`, `{{variant}}`. Filters: `|strip`
//! (`rrggbb`), `|rgb` (`r,g,b`). An unknown name or filter fails that template only.
use super::ResolvedTheme;
use super::apps::token_color;
use super::terminal;
use shared::theme::{ANSI_NAMES, Color, TOKENS};
use std::path::{Path, PathBuf};

pub use shared::theme::TEMPLATES_DIR;
/// Output folder under the toolkit dir.
pub const OUT_DIR: &str = "themed";
pub const EXT: &str = "tpl";
/// Bounds: a template folder is not a place for big files.
pub const MAX_TEMPLATES: usize = 64;
pub const MAX_SIZE: u64 = 256 * 1024;

fn value(t: &ResolvedTheme, name: &str) -> Option<Result<Color, String>> {
    let p = terminal::palette(t);
    if TOKENS.contains(&name) {
        return Some(Ok(token_color(t, name)));
    }
    if let Some(i) = name
        .strip_prefix("color")
        .and_then(|n| n.parse::<usize>().ok())
    {
        return p.ansi.get(i).copied().map(Ok);
    }
    if let Some(i) = ANSI_NAMES.iter().position(|n| *n == name) {
        return Some(Ok(p.ansi[i]));
    }
    let c = match name {
        "background" => p.background,
        "foreground" => p.foreground,
        "cursor" => p.cursor,
        "selection" => p.selection_bg,
        "id" => return Some(Err(t.id.clone())),
        "name" => return Some(Err(t.name.clone())),
        "variant" => return Some(Err(t.variant.name().to_string())),
        _ => return None,
    };
    Some(Ok(c))
}

/// One template's text with every placeholder replaced, or the first problem.
pub fn render(t: &ResolvedTheme, tpl: &str) -> Result<String, String> {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| "a {{ is never closed".to_string())?;
        let inner = after[..end].trim();
        let (name, filter) = match inner.split_once('|') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner, None),
        };
        let v = value(t, name).ok_or_else(|| format!("unknown name {{{{{name}}}}}"))?;
        let text = match (v, filter) {
            (Ok(c), None) => c.to_hex(),
            (Ok(c), Some("strip")) => c.to_hex()[1..].to_string(),
            (Ok(c), Some("rgb")) => format!("{},{},{}", c.r, c.g, c.b),
            (Err(s), None) => s,
            (_, Some(f)) => return Err(format!("unknown filter |{f} on {name}")),
        };
        out.push_str(&text);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `<config_home>/wolos/themed/*.tpl`, sorted, at most [`MAX_TEMPLATES`].
pub fn list(config_home: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(config_home.join(TEMPLATES_DIR)) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXT) && p.is_file())
        .collect();
    v.sort();
    v.truncate(MAX_TEMPLATES);
    v
}

/// Renders every template into `<dir>/themed/<name>` (atomic per file). Returns one sentence per
/// template that could not be rendered; the others are still written.
pub fn install(t: &ResolvedTheme, config_home: &Path, dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for p in list(config_home) {
        let name = p.file_stem().map(|s| s.to_string_lossy().into_owned());
        let Some(name) = name.filter(|n| !n.is_empty() && !n.starts_with('.')) else {
            continue;
        };
        let text = match std::fs::metadata(&p) {
            Ok(m) if m.len() > MAX_SIZE => Err(format!("larger than {} KiB", MAX_SIZE / 1024)),
            _ => std::fs::read_to_string(&p).map_err(|e| e.to_string()),
        }
        .and_then(|tpl| render(t, &tpl));
        match text {
            Ok(text) => {
                let out = dir.join(OUT_DIR).join(&name);
                let r = std::fs::create_dir_all(dir.join(OUT_DIR))
                    .and_then(|()| shared::atomic::atomic_write(&out, 0o644, text.as_bytes()));
                if let Err(e) = r {
                    problems.push(format!("{name}.{EXT}: {e}"));
                }
            }
            Err(e) => problems.push(format!("{name}.{EXT}: {e}")),
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeCatalog;

    fn theme() -> ResolvedTheme {
        ThemeCatalog::builtin().resolve("wol-dark").unwrap()
    }

    #[test]
    fn placeholders_and_filters() {
        let t = theme();
        let acc = token_color(&t, "accent");
        let p = terminal::palette(&t);
        let out = render(
            &t,
            "a={{accent}} s={{ accent|strip }} r={{accent | rgb}} c3={{color3}} y={{yellow}} bg={{background}} {{id}}/{{variant}}",
        )
        .unwrap();
        assert_eq!(
            out,
            format!(
                "a={} s={} r={},{},{} c3={} y={} bg={} wol-dark/dark",
                acc.to_hex(),
                &acc.to_hex()[1..],
                acc.r,
                acc.g,
                acc.b,
                p.ansi[3].to_hex(),
                p.ansi[3].to_hex(),
                p.background.to_hex()
            )
        );
        assert_eq!(render(&t, "no placeholders").unwrap(), "no placeholders");
    }

    #[test]
    fn mistakes_are_named() {
        let t = theme();
        for (tpl, want) in [
            ("{{nope}}", "unknown name"),
            ("{{accent|upper}}", "unknown filter"),
            ("{{name|strip}}", "unknown filter"),
            ("{{color16}}", "unknown name"),
            ("{{accent", "never closed"),
        ] {
            let e = render(&t, tpl).unwrap_err();
            assert!(e.contains(want), "{tpl}: {e}");
        }
    }

    #[test]
    fn every_template_is_rendered_on_switch_and_a_bad_one_does_not_stop_the_rest() {
        let root = std::env::temp_dir().join(format!("wol-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (cfg, dir) = (root.join("cfg"), root.join("toolkits"));
        let src = cfg.join(TEMPLATES_DIR);
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("btop.theme.tpl"),
            "theme[main_bg]=\"{{surface}}\"\n",
        )
        .unwrap();
        std::fs::write(src.join("bad.tpl"), "{{oops}}").unwrap();
        std::fs::write(src.join("readme.txt"), "{{oops}}").unwrap();
        let problems = install(&theme(), &cfg, &dir);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("bad.tpl: unknown name"));
        let out = std::fs::read_to_string(dir.join(OUT_DIR).join("btop.theme")).unwrap();
        assert_eq!(
            out,
            format!(
                "theme[main_bg]=\"{}\"\n",
                token_color(&theme(), "surface").to_hex()
            )
        );
        assert!(!dir.join(OUT_DIR).join("bad").exists());
        assert!(!dir.join(OUT_DIR).join("readme").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

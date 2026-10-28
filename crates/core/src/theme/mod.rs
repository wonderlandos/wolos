//! Theme catalog and resolver (IaraOS ADR 0016). Contract data lives in `shared::theme`.
pub mod accent;
pub mod apps;
pub mod firefox;
pub mod helium;
pub mod kvantum;
pub mod scaffold;
pub mod sourceview;

use shared::theme::{self as contract, Variant};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Palette credits: `builtin/NOTICE`.
const BUILTIN: [&str; 23] = [
    include_str!("builtin/wol-light.json"),
    include_str!("builtin/wol-dark.json"),
    include_str!("builtin/catppuccin-latte.json"),
    include_str!("builtin/catppuccin-mocha.json"),
    include_str!("builtin/everforest.json"),
    include_str!("builtin/gruvbox.json"),
    include_str!("builtin/kanagawa.json"),
    include_str!("builtin/nord.json"),
    include_str!("builtin/rose-pine.json"),
    include_str!("builtin/tokyo-night.json"),
    include_str!("builtin/matte-black.json"),
    include_str!("builtin/jade-horse.json"),
    include_str!("builtin/ristretto.json"),
    include_str!("builtin/flexoki-light.json"),
    include_str!("builtin/ethereal.json"),
    include_str!("builtin/hackerman.json"),
    include_str!("builtin/vantablack.json"),
    include_str!("builtin/white.json"),
    include_str!("builtin/miasma.json"),
    include_str!("builtin/lumon.json"),
    include_str!("builtin/lupine.json"),
    include_str!("builtin/retro-82.json"),
    include_str!("builtin/solitude.json"),
];

/// One `theme.json` as written; tokens may be partial.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeSpec {
    pub id: String,
    pub name: String,
    pub variant: Variant,
    pub extends: Option<String>,
    pub tokens: BTreeMap<String, String>,
    pub wallpaper: Option<String>,
    pub icon_theme: Option<String>,
    /// Theme folder; a relative wallpaper resolves against it.
    pub dir: PathBuf,
}

impl ThemeSpec {
    /// Unknown token, bad variant or bad JSON = error (file skipped by the catalog).
    pub fn parse(json: &str, dir: &Path) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
        let id = s("id").ok_or("missing id")?;
        let variant = s("variant")
            .as_deref()
            .and_then(Variant::parse)
            .ok_or("variant must be light or dark")?;
        let mut tokens = BTreeMap::new();
        if let Some(t) = v.get("tokens") {
            let t = t.as_object().ok_or("tokens must be an object")?;
            for (k, val) in t {
                if !contract::is_token(k) {
                    return Err(format!("unknown token {k}"));
                }
                tokens.insert(
                    k.clone(),
                    val.as_str().ok_or("token must be a string")?.to_owned(),
                );
            }
        }
        Ok(Self {
            name: s("name").unwrap_or_else(|| id.clone()),
            id,
            variant,
            extends: s("extends"),
            tokens,
            wallpaper: s("wallpaper"),
            icon_theme: s("iconTheme"),
            dir: dir.to_owned(),
        })
    }
}

/// Every token filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTheme {
    pub id: String,
    pub name: String,
    pub variant: Variant,
    pub tokens: BTreeMap<String, String>,
    pub wallpaper: Option<PathBuf>,
    pub icon_theme: Option<String>,
}

impl ResolvedTheme {
    /// Readability errors; empty = valid.
    pub fn check(&self) -> Vec<String> {
        contract::validate(|t| self.tokens.get(t).cloned())
    }

    /// GTK CSS variable block: every token + the fixed scale.
    pub fn css(&self) -> String {
        let mut s = String::from("* {");
        for n in contract::TOKENS {
            s.push_str(&format!(
                " {}: {};",
                contract::css_var(n),
                self.tokens.get(n).map_or("", String::as_str)
            ));
        }
        for (v, val) in contract::SCALE {
            s.push_str(&format!(" {v}: {val};"));
        }
        s.push_str(" }\n");
        s
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    Unknown(String),
    Cycle(String),
}

/// Themes by id: built-ins first, then each search path (later wins).
#[derive(Clone, Debug, Default)]
pub struct ThemeCatalog {
    specs: BTreeMap<String, ThemeSpec>,
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
pub fn config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => x.into(),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"),
    }
}

pub fn search_paths(config_home: &Path) -> Vec<PathBuf> {
    vec![
        contract::SYSTEM_THEMES_DIR.into(),
        config_home.join(contract::USER_THEMES_DIR),
    ]
}

impl ThemeCatalog {
    pub fn builtin() -> Self {
        let mut specs = BTreeMap::new();
        for j in BUILTIN {
            let s = ThemeSpec::parse(j, Path::new("")).expect("built-in theme");
            specs.insert(s.id.clone(), s);
        }
        Self { specs }
    }

    /// Built-ins plus every readable `<path>/<id>/theme.json`; broken files are skipped.
    pub fn load(paths: &[PathBuf]) -> Self {
        let mut cat = Self::builtin();
        for dir in paths {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path().join("theme.json"))
                .filter(|f| f.is_file())
                .collect();
            files.sort();
            for f in files {
                let parsed = std::fs::read_to_string(&f)
                    .ok()
                    .and_then(|t| ThemeSpec::parse(&t, f.parent().unwrap_or(dir)).ok());
                if let Some(s) = parsed {
                    cat.specs.insert(s.id.clone(), s);
                }
            }
        }
        cat
    }

    pub fn with(mut self, spec: ThemeSpec) -> Self {
        self.specs.insert(spec.id.clone(), spec);
        self
    }

    /// Sorted by id.
    pub fn list(&self) -> Vec<&ThemeSpec> {
        self.specs.values().collect()
    }

    pub fn resolve(&self, id: &str) -> Result<ResolvedTheme, ResolveError> {
        let mut chain: Vec<&ThemeSpec> = Vec::new();
        let mut cur = Some(id.to_owned());
        while let Some(c) = cur {
            let s = self
                .specs
                .get(&c)
                .ok_or_else(|| ResolveError::Unknown(c.clone()))?;
            if chain.iter().any(|x| x.id == s.id) {
                return Err(ResolveError::Cycle(c));
            }
            chain.push(s);
            let base = [
                Variant::Light.default_theme(),
                Variant::Dark.default_theme(),
            ];
            cur = s.extends.clone().or_else(|| {
                (!base.contains(&s.id.as_str())).then(|| s.variant.default_theme().to_owned())
            });
        }
        let mut tokens = BTreeMap::new();
        let (mut wallpaper, mut icons) = (None, None);
        for s in chain.iter().rev() {
            tokens.extend(s.tokens.clone());
            if let Some(w) = &s.wallpaper {
                wallpaper = Some(if Path::new(w).is_absolute() {
                    PathBuf::from(w)
                } else {
                    s.dir.join(w)
                });
            }
            if s.icon_theme.is_some() {
                icons.clone_from(&s.icon_theme);
            }
        }
        let top = chain[0];
        Ok(ResolvedTheme {
            id: top.id.clone(),
            name: top.name.clone(),
            variant: top.variant,
            tokens,
            wallpaper,
            icon_theme: icons,
        })
    }
}

/// The user's chosen theme id: one line in `$XDG_CONFIG_HOME/wolos/theme`. No file = follow appearance.
pub mod active {
    use super::*;

    pub fn default_file(config_home: &Path) -> PathBuf {
        config_home.join(contract::ACTIVE_FILE)
    }

    pub fn get(file: &Path) -> Option<String> {
        let t = std::fs::read_to_string(file).ok()?;
        let t = t.trim();
        (!t.is_empty()).then(|| t.to_owned())
    }

    /// Atomic replace: watchers never read a half-written file.
    pub fn set(file: &Path, id: &str) -> std::io::Result<()> {
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, format!("{id}\n"))?;
        std::fs::rename(tmp, file)
    }

    pub fn effective(saved: Option<&str>, dark: bool) -> String {
        saved.map_or_else(
            || {
                if dark { Variant::Dark } else { Variant::Light }
                    .default_theme()
                    .to_owned()
            },
            str::to_owned,
        )
    }

    /// `wol theme light|dark|toggle`: the id to write given the current effective theme.
    pub fn switch(cmd: &str, current: &ResolvedTheme) -> Option<&'static str> {
        match cmd {
            "light" => Some(Variant::Light.default_theme()),
            "dark" => Some(Variant::Dark.default_theme()),
            "toggle" => Some(match current.variant {
                Variant::Light => Variant::Dark.default_theme(),
                Variant::Dark => Variant::Light.default_theme(),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-theme-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, id: &str, json: &str) {
        std::fs::create_dir_all(dir.join(id)).unwrap();
        std::fs::write(dir.join(id).join("theme.json"), json).unwrap();
    }

    #[test]
    fn builtins_are_complete_and_readable() {
        let cat = ThemeCatalog::builtin();
        for (id, variant) in [
            ("wol-light", Variant::Light),
            ("wol-dark", Variant::Dark),
            ("tokyo-night", Variant::Dark),
            ("catppuccin-mocha", Variant::Dark),
            ("catppuccin-latte", Variant::Light),
            ("gruvbox", Variant::Dark),
            ("nord", Variant::Dark),
            ("everforest", Variant::Dark),
            ("rose-pine", Variant::Dark),
            ("kanagawa", Variant::Dark),
            ("matte-black", Variant::Dark),
            ("jade-horse", Variant::Dark),
            ("ristretto", Variant::Dark),
            ("flexoki-light", Variant::Light),
            ("ethereal", Variant::Dark),
            ("hackerman", Variant::Dark),
            ("vantablack", Variant::Dark),
            ("white", Variant::Light),
            ("miasma", Variant::Dark),
            ("lumon", Variant::Dark),
            ("lupine", Variant::Light),
            ("retro-82", Variant::Dark),
            ("solitude", Variant::Dark),
        ] {
            let t = cat.resolve(id).unwrap();
            assert_eq!((t.id.as_str(), t.variant), (id, variant));
            assert_eq!(t.tokens.len(), 13, "{id}");
        }
        assert_eq!(cat.list().len(), BUILTIN.len());
    }

    /// Every catalogue theme (built-ins and accent presets) passes the contrast contract.
    #[test]
    fn every_theme_passes_the_contrast_contract() {
        let mut cat = ThemeCatalog::builtin();
        for v in accent::variants() {
            cat = cat.with(v);
        }
        for s in cat.list() {
            let t = cat.resolve(&s.id).unwrap();
            assert!(t.check().is_empty(), "{}: {:?}", s.id, t.check());
        }
        assert!(cat.list().len() >= 23 + 14);
    }

    /// Each theme states its own JSON tokens fully (no silent fallback to the WolOS palette) and
    /// pairs only with a wallpaper that ships (`design/wallpapers`).
    #[test]
    fn builtin_files_are_self_contained_and_pair_shipped_wallpapers() {
        let walls = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/wallpapers");
        for s in ThemeCatalog::builtin().list() {
            assert_eq!(s.tokens.len(), 13, "{}", s.id);
            if let Some(w) = &s.wallpaper {
                let name = w
                    .strip_prefix("/usr/share/wolos/wallpapers/")
                    .unwrap_or_else(|| panic!("{}: {w}", s.id));
                assert!(walls.join(name).is_file(), "{}: {w}", s.id);
            }
        }
    }

    #[test]
    fn builtins_match_palette() {
        let cat = ThemeCatalog::builtin();
        assert_eq!(cat.resolve("wol-dark").unwrap().tokens["card"], "#2c2c30");
        assert_eq!(
            cat.resolve("wol-light").unwrap().tokens["sidebar"],
            "rgba(236,236,240,0.92)"
        );
        assert_eq!(cat.resolve("wol-dark").unwrap().variant, Variant::Dark);
    }

    #[test]
    fn builtins_use_brand_accent() {
        // design/tokens: violet brand accent; dark is lighter but keeps white text >= 3.0.
        let cat = ThemeCatalog::builtin();
        for (id, accent) in [("wol-light", "#7c5cff"), ("wol-dark", "#8b6cff")] {
            assert_eq!(cat.resolve(id).unwrap().tokens["accent"], accent, "{id}");
        }
    }

    #[test]
    fn missing_tokens_fall_back_to_variant_base() {
        let d = temp("fallback");
        put(
            &d,
            "mine",
            r##"{"id":"mine","name":"Mine","variant":"dark","tokens":{"accent":"#ff00ff"}}"##,
        );
        let t = ThemeCatalog::load(&[d]).resolve("mine").unwrap();
        assert_eq!(
            (t.tokens["accent"].as_str(), t.tokens["card"].as_str()),
            ("#ff00ff", "#2c2c30")
        );
    }

    #[test]
    fn extends_and_user_dirs_override() {
        let (sys, user) = (temp("sys"), temp("user"));
        put(
            &sys,
            "a",
            r##"{"id":"a","name":"A","variant":"light","tokens":{"accent":"#111111"}}"##,
        );
        put(
            &user,
            "b",
            r#"{"id":"b","name":"B","variant":"light","extends":"a","tokens":{}}"#,
        );
        put(
            &user,
            "nord",
            r#"{"id":"nord","name":"My Nord","variant":"dark","tokens":{}}"#,
        );
        let cat = ThemeCatalog::load(&[sys, user]);
        assert_eq!(cat.resolve("b").unwrap().tokens["accent"], "#111111");
        assert_eq!(cat.resolve("nord").unwrap().name, "My Nord");
        assert!(cat.list().iter().any(|s| s.id == "tokyo-night"));
    }

    #[test]
    fn wallpaper_relative_to_folder() {
        let d = temp("wall");
        put(
            &d,
            "w",
            r#"{"id":"w","name":"W","variant":"light","tokens":{},"wallpaper":"bg.png","iconTheme":"WhiteSur"}"#,
        );
        let t = ThemeCatalog::load(std::slice::from_ref(&d))
            .resolve("w")
            .unwrap();
        assert_eq!(t.wallpaper, Some(d.join("w").join("bg.png")));
        assert_eq!(t.icon_theme.as_deref(), Some("WhiteSur"));
    }

    #[test]
    fn cycle_and_unknown_fail() {
        let d = temp("cycle");
        put(
            &d,
            "x",
            r#"{"id":"x","name":"X","variant":"light","extends":"y","tokens":{}}"#,
        );
        put(
            &d,
            "y",
            r#"{"id":"y","name":"Y","variant":"light","extends":"x","tokens":{}}"#,
        );
        let cat = ThemeCatalog::load(&[d]);
        assert!(matches!(cat.resolve("x"), Err(ResolveError::Cycle(_))));
        assert_eq!(
            cat.resolve("nope"),
            Err(ResolveError::Unknown("nope".into()))
        );
    }

    #[test]
    fn broken_files_skipped() {
        let d = temp("broken");
        put(&d, "bad", "{ not json");
        put(
            &d,
            "bad2",
            r#"{"id":"bad2","name":"B","variant":"sepia","tokens":{}}"#,
        );
        put(
            &d,
            "bad3",
            r##"{"id":"bad3","name":"B","variant":"dark","tokens":{"chrome":"#fff"}}"##,
        );
        let cat = ThemeCatalog::load(&[d]);
        assert!(!cat.list().iter().any(|s| s.id.starts_with("bad")));
    }

    #[test]
    fn validator_flags_low_contrast_and_bad_colors() {
        let d = temp("low");
        put(
            &d,
            "low",
            r##"{"id":"low","name":"L","variant":"light","tokens":{"text":"#cccccc","danger":"nope"}}"##,
        );
        let e = ThemeCatalog::load(&[d]).resolve("low").unwrap().check();
        assert!(e.iter().any(|e| e.contains("text/surface")));
        assert!(e.iter().any(|e| e.contains("danger")));
    }

    #[test]
    fn search_paths_system_then_user() {
        assert_eq!(
            search_paths(Path::new("/home/u/.config")),
            vec![
                PathBuf::from("/usr/share/wolos/themes"),
                PathBuf::from("/home/u/.config/wolos/themes")
            ]
        );
    }

    #[test]
    fn active_set_get_effective_switch() {
        let f = temp("active").join("wolos").join("theme");
        assert_eq!(active::get(&f), None);
        active::set(&f, "nord").unwrap();
        assert_eq!(active::get(&f).as_deref(), Some("nord"));
        for (saved, dark, want) in [
            (None, false, "wol-light"),
            (None, true, "wol-dark"),
            (Some("nord"), false, "nord"),
        ] {
            assert_eq!(active::effective(saved, dark), want);
        }
        let cat = ThemeCatalog::builtin();
        let dark = cat.resolve("nord").unwrap();
        for (cmd, want) in [
            ("light", Some("wol-light")),
            ("dark", Some("wol-dark")),
            ("toggle", Some("wol-light")),
            ("x", None),
        ] {
            assert_eq!(active::switch(cmd, &dark), want, "{cmd}");
        }
    }

    #[test]
    fn css_has_tokens_and_scale() {
        let css = ThemeCatalog::builtin().resolve("wol-dark").unwrap().css();
        for want in [
            "--card: #2c2c30;",
            "--accent-text: #ffffff;",
            "--radius-m: 10px;",
            "--motion-fast: 150ms;",
        ] {
            assert!(css.contains(want), "{want}");
        }
        assert!(css.starts_with("* {"));
    }
}

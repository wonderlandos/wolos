// SPDX-License-Identifier: BSD-2-Clause
//! Theme packs: the `[[theme]] file` entries of enabled plugins (frozen manifest,
//! `shared::plugins::Manifest::theme`). A plugin folder that fails `plugins::check` brings
//! nothing; a pack theme never replaces an id the catalogue already has (built-in, system or the
//! person's own), so a plugin cannot take over `wol-light`.
use super::{ThemeCatalog, ThemeSpec};
use crate::plugins;
use std::path::Path;

/// One theme from a plugin.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    /// Plugin id and display name, for "from <plugin>" in Appearance.
    pub plugin: String,
    pub plugin_name: String,
    pub spec: ThemeSpec,
}

/// Themes of every enabled, valid plugin under `data_home` (user plugins first), in plugin order.
pub fn themes(config_home: &Path, data_home: &Path) -> Vec<Pack> {
    let on = plugins::load_enabled(config_home);
    from(&plugins::load_all(&plugins::dirs(data_home)), &on)
}

pub fn from(list: &[plugins::Plugin], on: &plugins::Enabled) -> Vec<Pack> {
    let mut out = Vec::new();
    for p in list {
        let Ok(m) = &p.manifest else { continue };
        if !on.is_enabled(&p.id) {
            continue;
        }
        for e in &m.theme {
            // `check` (run by `load`) already refused bad paths, symlinks and sizes.
            let file = p.dir.join(&e.file);
            let Some(spec) = std::fs::read_to_string(&file)
                .ok()
                .and_then(|j| ThemeSpec::parse(&j, file.parent().unwrap_or(&p.dir)).ok())
            else {
                continue;
            };
            out.push(Pack {
                plugin: p.id.clone(),
                plugin_name: m.name.clone(),
                spec,
            });
        }
    }
    out
}

/// `cat` plus every pack theme whose id is new.
pub fn add(cat: ThemeCatalog, packs: &[Pack]) -> ThemeCatalog {
    packs.iter().fold(cat, |c, p| c.with_new(p.spec.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::plugins::Manifest;

    fn plugin(root: &Path, id: &str, themes: &[(&str, &str)]) -> plugins::Plugin {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Manifest {
            schema: 1,
            id: id.into(),
            name: format!("{id} pack"),
            version: "1.0.0".into(),
            author: "a".into(),
            license: "MIT".into(),
            description: "Themes".into(),
            ..Default::default()
        };
        for (i, (file, json)) in themes.iter().enumerate() {
            std::fs::write(dir.join(file), json).unwrap();
            m.theme.push(shared::plugins::FileEntry {
                id: format!("t{i}"),
                file: (*file).into(),
            });
        }
        std::fs::write(dir.join(shared::plugins::MANIFEST_FILE), m.to_toml()).unwrap();
        plugins::load(&dir, false)
    }

    #[test]
    fn enabled_plugins_bring_their_themes_without_replacing_known_ones() {
        let root = std::env::temp_dir().join(format!("wol-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let good = r##"{"id":"sunset","name":"Sunset","variant":"dark","extends":"wol-dark","tokens":{"accent":"#ff7a59"}}"##;
        let takeover = r##"{"id":"wol-light","name":"Mine","variant":"light","extends":"wol-dark","tokens":{}}"##;
        let a = plugin(
            &root,
            "com.example.sunset",
            &[("sunset.json", good), ("x.json", takeover)],
        );
        assert!(a.manifest.is_ok(), "{:?}", a.manifest);
        let b = plugin(
            &root,
            "com.example.off",
            &[("off.json", &good.replace("sunset", "off"))],
        );
        let mut on = plugins::Enabled::default();
        on.enable(a.manifest.as_ref().unwrap());
        let packs = from(&[a, b], &on);
        assert_eq!(
            packs.iter().map(|p| p.spec.id.as_str()).collect::<Vec<_>>(),
            ["sunset", "wol-light"]
        );
        assert_eq!(packs[0].plugin_name, "com.example.sunset pack");
        let cat = add(ThemeCatalog::builtin(), &packs);
        assert_eq!(cat.resolve("sunset").unwrap().tokens["accent"], "#ff7a59");
        assert_eq!(cat.resolve("wol-light").unwrap().name, "WolOS Light");
        assert!(
            cat.resolve("off").is_err(),
            "a disabled plugin brings nothing"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

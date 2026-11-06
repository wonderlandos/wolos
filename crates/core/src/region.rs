// SPDX-License-Identifier: BSD-2-Clause
//! Date & time (zone, network time) and language choices for Settings > General.
use crate::system::Host;

pub const ZONE_TABS: [&str; 2] = [
    "/usr/share/zoneinfo/zone1970.tab",
    "/usr/share/zoneinfo/zone.tab",
];

/// zone1970.tab / zone.tab: the TZ column, sorted, unique.
pub fn parse_zone_tab(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::trim)
        .filter(|z| !z.is_empty())
        .map(String::from)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Every zone the system knows; empty without tzdata.
pub fn zones(h: &dyn Host) -> Vec<String> {
    ZONE_TABS
        .iter()
        .filter_map(|p| h.read_text(p))
        .map(|t| parse_zone_tab(&t))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// `/etc/localtime` (timedatectl's symlink); "UTC" when missing or odd.
pub fn current_zone(h: &dyn Host) -> String {
    let link = h.read_link("/etc/localtime");
    let z = crate::settings::timezones::current(link.as_deref());
    if crate::settings::timezones::is_valid(z) {
        z.into()
    } else {
        "UTC".into()
    }
}

/// Network time on (systemd-timesyncd through timedatectl)? None when unknown.
pub fn ntp_enabled(h: &dyn Host) -> Option<bool> {
    match h
        .run(&["timedatectl", "show", "-p", "NTP", "--value"])?
        .trim()
    {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// "America/Sao_Paulo" -> "Sao Paulo (America)".
pub fn zone_label(zone: &str) -> String {
    let city = zone.rsplit('/').next().unwrap_or(zone).replace('_', " ");
    match zone.split_once('/') {
        Some((region, _)) => format!("{city} ({region})"),
        None => city,
    }
}

/// (locale, label) the languages a person can pick, label in its own language, in the order the
/// installer, Setup and Settings list them: English first, then the lead's nine (2026-11-06):
/// exactly `shared::i18n::Lang::OFFERED` (`languages_are_the_offered_ones`). Every one has its
/// translator's rules (`docs/l10n/RULES-<tag>.md`, `rules_set_equals_languages`) and a
/// `<tag>.tsv` in the shell's unit; es, ko, zh-Hans, id and tl are still being filled (their
/// empty rows read English, `Lang::stage`).
pub const LANGUAGES: [(&str, &str); 10] = [
    ("en_US.UTF-8", "English"),
    ("pt_BR.UTF-8", "Português (Brasil)"),
    ("es_ES.UTF-8", "Español"),
    ("fr_FR.UTF-8", "Français"),
    ("de_DE.UTF-8", "Deutsch"),
    ("ja_JP.UTF-8", "日本語"),
    ("ko_KR.UTF-8", "한국어"),
    ("zh_CN.UTF-8", "简体中文"),
    ("id_ID.UTF-8", "Bahasa Indonesia"),
    ("tl_PH.UTF-8", "Tagalog"),
];

/// Languages whose catalogue is kept as a draft and not offered (the lead, 2026-11-06): no picker
/// lists them, a computer already set to one still reads it (`Lang::from_posix`).
pub const DRAFTS: [(&str, &str); 2] = [("ru_RU.UTF-8", "Русский"), ("it_IT.UTF-8", "Italiano")];

/// The new system's `/etc/locale.gen`: every locale of [`LANGUAGES`], then `chosen` when it is not
/// one of them. Settings > Language & Region offers all of them, so all are generated at install;
/// choosing one later needs no root and no wait (QA-SET-27: the pane exported a locale the machine
/// lacked and every program fell back to C). `chosen` must be [`is_valid_locale`] (else left out).
pub fn locale_gen(chosen: &str) -> String {
    let mut all: Vec<&str> = LANGUAGES.iter().map(|l| l.0).collect();
    if is_valid_locale(chosen) && !all.contains(&chosen) {
        all.push(chosen);
    }
    all.iter().map(|l| format!("{l} UTF-8\n")).collect()
}

/// The language a locale belongs to, in its preferred form: the code translators and message
/// catalogs use. `pt_BR` is an alias of `pt` and `pt_PT` a variation of it, so all three are
/// `pt`; `en_US` and `en_GB` are `en`; `zh_CN` is `zh`. "" for anything that is not a locale.
pub fn language_of(locale: &str) -> &str {
    if !is_valid_locale(locale) {
        return "";
    }
    locale.split('_').next().unwrap_or_default()
}

/// Languages planned next: their translators' rules are written, they are not offered yet.
pub const LATER: [(&str, &str); 2] = [("nl_NL.UTF-8", "Nederlands"), ("pl_PL.UTF-8", "Polski")];

/// The code a translation table and its rules page use (`pt-BR.tsv`, `RULES-pt-BR.md`): the
/// language of `locale`, with Brazilian Portuguese named apart from other Portuguese and
/// Simplified Chinese by its script (`zh_CN`, `zh_SG`: `zh-Hans`).
pub fn translation_tag(locale: &str) -> &str {
    match locale.split('.').next().unwrap_or_default() {
        "pt_BR" => "pt-BR",
        "zh_CN" | "zh_SG" => "zh-Hans",
        _ => language_of(locale),
    }
}

/// `ll_CC.UTF-8` (2-3 lowercase letters, 2 uppercase).
pub fn is_valid_locale(s: &str) -> bool {
    s.strip_suffix(".UTF-8")
        .and_then(|l| l.split_once('_'))
        .is_some_and(|(l, r)| {
            (2..=3).contains(&l.len())
                && l.bytes().all(|b| b.is_ascii_lowercase())
                && r.len() == 2
                && r.bytes().all(|b| b.is_ascii_uppercase())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::fake::FakeHost;

    const TAB: &str = "# tzdb timezone descriptions\n#\n#codes\tcoordinates\tTZ\tcomments\nBR\t-2332-04637\tAmerica/Sao_Paulo\tBrazil (southeast)\nPT\t+3843-00908\tEurope/Lisbon\tPortugal (mainland)\nCH,DE,LI\t+4723+00832\tEurope/Zurich\tBüsingen\nAR\t-3436-05827\tAmerica/Argentina/Buenos_Aires\tBuenos Aires (BA, CF)\nPT\t+3843-00908\tEurope/Lisbon\n\n";

    #[test]
    fn zone_tab() {
        assert_eq!(
            parse_zone_tab(TAB),
            [
                "America/Argentina/Buenos_Aires",
                "America/Sao_Paulo",
                "Europe/Lisbon",
                "Europe/Zurich"
            ]
        );
        assert!(parse_zone_tab("").is_empty());
        let h = FakeHost::linux();
        assert!(zones(&h).is_empty());
        h.file(
            "/usr/share/zoneinfo/zone.tab",
            "BR\t-2332-04637\tAmerica/Sao_Paulo\n",
        );
        assert_eq!(zones(&h), ["America/Sao_Paulo"]);
        h.file("/usr/share/zoneinfo/zone1970.tab", TAB);
        assert_eq!(zones(&h).len(), 4, "zone1970.tab wins");
    }

    #[test]
    fn current_zone_is_the_localtime_link() {
        let h = FakeHost::linux();
        assert_eq!(current_zone(&h), "UTC");
        h.link("/etc/localtime", "../usr/share/zoneinfo/America/Sao_Paulo");
        assert_eq!(current_zone(&h), "America/Sao_Paulo");
        h.link("/etc/localtime", "/usr/share/zoneinfo/Europe/Lisbon");
        assert_eq!(current_zone(&h), "Europe/Lisbon");
        h.link("/etc/localtime", "/usr/share/zoneinfo/../../etc/passwd");
        assert_eq!(current_zone(&h), "UTC");
        h.link("/etc/localtime", "/etc/passwd");
        assert_eq!(current_zone(&h), "UTC");
    }

    #[test]
    fn network_time() {
        let l = FakeHost::linux();
        assert_eq!(ntp_enabled(&l), None);
        l.command("timedatectl show -p NTP --value", "yes\n");
        assert_eq!(ntp_enabled(&l), Some(true));
        l.command("timedatectl show -p NTP --value", "no\n");
        assert_eq!(ntp_enabled(&l), Some(false));
    }

    #[test]
    fn real_host_zone_is_valid() {
        let z = current_zone(&crate::system::PosixHost);
        assert!(crate::settings::timezones::is_valid(&z), "{z}");
    }

    #[test]
    fn zone_labels() {
        for (z, want) in [
            ("America/Sao_Paulo", "Sao Paulo (America)"),
            ("America/Argentina/Buenos_Aires", "Buenos Aires (America)"),
            ("Europe/Lisbon", "Lisbon (Europe)"),
            ("UTC", "UTC"),
        ] {
            assert_eq!(zone_label(z), want, "{z}");
        }
    }

    #[test]
    fn locales() {
        for (l, ok) in [
            ("pt_BR.UTF-8", true),
            ("ast_ES.UTF-8", true),
            ("pt_BR", false),
            ("PT_br.UTF-8", false),
            ("pt_BR.UTF-8; rm", false),
            ("", false),
        ] {
            assert_eq!(is_valid_locale(l), ok, "{l}");
        }
        assert!(
            LANGUAGES
                .iter()
                .chain(DRAFTS.iter())
                .chain(LATER.iter())
                .all(|(l, _)| is_valid_locale(l))
        );
        // A later language or a draft is not offered.
        assert!(LATER.iter().all(|l| !LANGUAGES.iter().any(|o| o.0 == l.0)));
        assert!(DRAFTS.iter().all(|l| !LANGUAGES.iter().any(|o| o.0 == l.0)));
    }

    #[test]
    fn portuguese_is_one_language() {
        assert_eq!(language_of("pt_BR.UTF-8"), "pt");
        assert_eq!(language_of("pt_PT.UTF-8"), "pt");
        assert_eq!(language_of("en_GB.UTF-8"), "en");
        assert_eq!(language_of("pt"), "");
        // One row per language: no two rows a translation could not tell apart.
        let tags: std::collections::BTreeSet<_> =
            LANGUAGES.iter().map(|l| translation_tag(l.0)).collect();
        assert_eq!(tags.len(), LANGUAGES.len());
        assert_eq!(translation_tag("pt_BR.UTF-8"), "pt-BR");
        assert_eq!(translation_tag("pt_PT.UTF-8"), "pt");
        assert_eq!(translation_tag("ja_JP.UTF-8"), "ja");
        assert_eq!(translation_tag("zh_CN.UTF-8"), "zh-Hans");
        assert_eq!(translation_tag("tl_PH.UTF-8"), "tl");
    }

    /// The pickers list exactly the languages `Lang` offers (English and the lead's nine), in its
    /// order, each label the language's own name, each tag the name of its catalogue; the drafts
    /// (ru, it) still map to their catalogue but are not offered.
    #[test]
    fn languages_are_the_offered_ones() {
        use shared::i18n::Lang;
        let picked: Vec<Option<Lang>> = LANGUAGES.iter().map(|l| Lang::from_posix(l.0)).collect();
        let offered: Vec<Option<Lang>> = Lang::OFFERED.into_iter().map(Some).collect();
        assert_eq!(picked, offered);
        let tags: Vec<&str> = LANGUAGES.iter().map(|l| translation_tag(l.0)).collect();
        assert_eq!(
            tags,
            [
                "en", "pt-BR", "es", "fr", "de", "ja", "ko", "zh-Hans", "id", "tl"
            ]
        );
        for (locale, label) in LANGUAGES {
            let l = Lang::from_posix(locale).expect(locale);
            assert_eq!(l.tag(), translation_tag(locale), "{locale}");
            assert_eq!(
                l.endonym().split(" (").next(),
                label.split(" (").next(),
                "{locale}"
            );
            assert!(l.offered(), "{locale}");
        }
        for (locale, _) in DRAFTS {
            let l = Lang::from_posix(locale).expect(locale);
            assert!(!l.offered(), "{locale} is a draft");
        }
    }

    /// QA-ACC-16 / QA-L10N-8: the translators' rule pages (`docs/l10n/RULES-<tag>.md`) are exactly
    /// the languages people can pick ([`LANGUAGES`]), the drafts kept ([`DRAFTS`]) and the ones
    /// planned next ([`LATER`]): no rules for a language no one translates, no language offered
    /// without rules.
    #[test]
    fn rules_set_equals_languages() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/l10n");
        let rules: std::collections::BTreeSet<String> = std::fs::read_dir(&dir)
            .expect("docs/l10n")
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter_map(|n| Some(n.strip_prefix("RULES-")?.strip_suffix(".md")?.to_owned()))
            .collect();
        let listed: std::collections::BTreeSet<String> = LANGUAGES
            .iter()
            .chain(DRAFTS.iter())
            .chain(LATER.iter())
            .map(|l| translation_tag(l.0).to_owned())
            .collect();
        assert_eq!(rules, listed, "RULES-*.md vs LANGUAGES + DRAFTS + LATER");
        // The translators' guide lists the same pages.
        let guide = std::fs::read_to_string(dir.join("README.md")).expect("README.md");
        let linked: std::collections::BTreeSet<String> = guide
            .split("](RULES-")
            .skip(1)
            .filter_map(|t| Some(t.split_once(".md)")?.0.to_owned()))
            .collect();
        assert_eq!(
            linked, listed,
            "docs/l10n/README.md links vs LANGUAGES + DRAFTS + LATER"
        );
    }

    #[test]
    fn locale_gen_has_every_language_and_the_chosen_one_once() {
        let g = locale_gen("pt_BR.UTF-8");
        for (l, _) in LANGUAGES {
            assert_eq!(g.matches(&format!("{l} UTF-8\n")).count(), 1, "{l}");
        }
        assert!(locale_gen("ast_ES.UTF-8").ends_with("ast_ES.UTF-8 UTF-8\n"));
        assert!(!locale_gen("x; rm -rf").contains("rm"));
        assert_eq!(locale_gen("").lines().count(), LANGUAGES.len());
    }
}

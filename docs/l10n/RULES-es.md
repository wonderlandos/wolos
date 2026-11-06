# WolOS in Spanish: rules for translators

Copied from the joint rulebook `RULEBOOK-es.md` (TRANSL-ARCH, 2026-11-06) by OS-LANGS; it binds the OS rows too.

These rules bind every line of Spanish on three surfaces: the site (chrome `src/i18n/es.json`, page prose
`src/content/es/pages/*.json`, posts `src/content/es/blog/*.md`), the manual (`src/content/es/manual/*.md`) and the
OS (`crates/shell/locale/shell/es.tsv`, and every later unit). In the OS this page is `docs/l10n/RULES-es.md` (`Lang::Es`, stage Filling: offered, every row of `es.tsv` present and empty until translated, an empty row reads English); `GLOSSARY.csv`, `STATE.md` and `PIPELINE.md` are the l10n planning files that came with it (`.claude/v014/l10n/` of the main checkout).
Status: proposed by TRANSL-ARCH 2026-11-06; **the es worker reads it first, raises any rule it disagrees with in #plan
before translating, and a native reviewer confirms it before the language goes live**. Terms: `GLOSSARY.csv` (same folder),
column `es`; a glossary word is used every time, and a change to it is a glossary change first.

## Register and tone

**tú**, never *usted* and never *vos*/*vosotros*. Imperative of *tú* (`Elige`, `Abre`, `Vuelve a intentarlo`). Neutral international Spanish that reads naturally in Mexico, Argentina, Colombia and Spain: avoid words marked to one region: **computadora** (not *ordenador*), *el equipo* where a neutral word fits, *teléfono* (not *móvil* or *celular*), *archivo*, *descargar*, *app*, *hacer clic* (not *pinchar*). The site's Open Graph locale is `es_ES` (`src/lib/locale.js:10`): flag to the CTO whether it should be `es_419`/neutral; the text rule stays neutral either way.

## Capitals

Sentence case for titles, headings, buttons and labels (`Modo oscuro`, `Empieza aquí`). Days and months lower case.

## Punctuation and quotation marks

Opening marks are required: `¿…?`, `¡…!` (but no exclamations at all, rule 5). Curly quotes **“…”** for quotes and on-screen labels, one style everywhere (no «…»), as the other Latin-script WolOS languages except fr and de. Ellipsis … without a space before. Accents in capitals (`Á`).

## Numbers, dates, units, spelling

Decimal comma, dot or thin space for thousands: **`1.234,5`**, one style, `100 %` with a no-break space (RAE), `16 GB`. Dates `6 de noviembre de 2026` in prose, `06/11/2026` in tables; 24-hour `14:30`.

## Plural forms

`one` for 1, `many` for whole millions (`1 millón de archivos`), `other` for the rest (CLDR es); OS rows wait for the enum.

## Length

+20 to +30 % over English. Avoid *de* chains (`Ajustes de sonido de entrada` -> `Entrada de sonido`).

## Names in a sentence

*WolOS* without article (*Instala WolOS*), *el Atlas*/*Atlas* (as a browser name, without article in titles), *la Store*, *el Hub*, *el Dock*, *el Control Center*, *el Memory Saver*, *la Craft Suite*.

## Banned in Spanish (besides rule 5)

*todavía no*, *próximamente*, *previsto*, *vista previa*, *sin verificar*, *en revisión*; *falló*, *error*, *no válido*, *no disponible* in OS sentences (say *No se pudo…*).

## Examples

| English | Good | Bad | Why bad |
|---|---|---|---|
| “{network}” didn’t answer. Try again, or choose Not Now and join later. | “{network}” no respondió. Vuelve a intentarlo o elige Ahora no y conéctate más tarde. | La red “{network}” no ha podido ser contactada. Por favor, reintente. | Passive, *usted*, *por favor*, longer, no next step. |
| Control Center, at the top right, has Wi-Fi, sound, brightness and Do Not Disturb. | El Control Center, arriba a la derecha, tiene Wi-Fi, sonido, brillo y No molestar. | En el Centro de Control usted encontrará el `WiFi`, el sonido... | Translated name, *usted*, respelled Wi-Fi. |
| Ask the manual | Pregúntale al manual | Preguntar Al Manual | Title Case, label tone. |
| This disk is empty, so there’s nothing to install alongside. Use Erase Disk. | Este disco está vacío, así que no hay nada junto a lo que instalar. Usa Borrar disco. | Error: el disco no es válido. | Banned words, no action. |

Examples are written to show the rules; the es worker replaces any that a native writer would phrase differently and
says so in its report.

## Notes for this language

Spanish has no OS rules page yet: when the CTO adds `Lang::Es`, this rulebook becomes `docs/l10n/RULES-es.md` in the OS repo (CTO's call).

## Rules for every language (same text in all nine rulebooks)

1. **Natural, not literal.** Write what a native speaker would write in the same moment, keeping the English tone: calm,
   plain, kind, never blaming, never selling. The manual's voice is the plain voice of the English: short sentences, one idea
   each, the reader as "you", what you see and then what to do. A line that reads like a translation is a bug.
2. **English is the truth.** Translate what the English says, no more. Do not add claims, numbers, warnings, jokes or
   marketing. If the English is wrong, unclear or out of date, do not fix it in your language: post `@main` in #plan with the
   file:line and wait for the English fix (PIPELINE.md, source freeze).
3. **Never translated:** the names in GLOSSARY.csv with kind `keep` (WolOS, WolKernel, Wonderland, Atlas, Atlas Drive,
   Cryswol, Wolkit, libWolkit, libwol, Hub, Store, Memory Saver, Craft Suite, Control Center, Plugin Maker, Theme Studio,
   wol-hue, wopkg, Federate, Dock, Night Shift); Wi-Fi, Bluetooth, USB, HDMI; codec, format and unit names (AAC, LDAC, Opus,
   PNG, PDF, kHz, ms, dB, GB); everything in `code` (commands, flags, paths, file names, keys, package names); every
   `{placeholder}` (spelled the same, moved freely); Markdown marks `**`, `` ` ``, `~~key~~`, link targets `(/manual/...)`
   (link targets stay English paths: the site prefixes them, `src/lib/locale.js` `localizeHtml`); the firmware words of the key
   screen (Enroll MOK, Continue, Yes, Reboot).
4. **Quote what the screen shows in this language.** A button, a pane or a message the manual quotes is written exactly as
   the OS shows it **in Spanish**: take it from `es.tsv`. If that text is not in the catalogue yet (most of Settings, the
   Control Center, the Store, widgets: STATE.md 1c), the screen shows English, so the manual quotes the **English** label and
   may add the meaning once in parentheses: haz clic en **Show date** (mostrar la fecha). Never invent a localized label the screen does not show.
5. **Banned words** (GLOSSARY.csv kind `banned`): upstream project names (Hyprland: say the compositor or the desktop; AUR:
   the community package collection; nobody credited in THIRD-PARTY.md), the status hedges of the site ("not yet", "coming
   soon", "planned", "preview", "unverified", "in review", "claimed") and their Spanish equivalents listed below, "the user",
   "please" in every line, exclamation marks, and in OS sentences the copy-lint words failed / error / invalid / unavailable /
   daemon / socket and their equivalents. The English lints do not read your language (STATE.md gap 5): you are the check.
6. **No engineer talk on reader pages** (everything above the Developers rung): no repository paths, file names, ADR numbers,
   commit ids, test names. Same rule as English (`scripts/lib/visible.mjs`).
7. **Front matter of a manual page:** translate `title` and `summary`; keep `section`, `order` and every other field as in
   English (`docs/l10n/README.md:14` of the site). Same file name as the English page. Headings translated; the site derives
   anchors from them, so a link to `#anchor` in another page must use that page's translated anchor (check-links finds a broken
   one).
8. **Screenshots: English images, translated alt text and captions.** Do not edit, re-shoot or replace images. Why: (a) 166 of
   the manual's images are shell goldens drawn from English, and the OS shows English in most panes today, so an English
   screenshot is what a Spanish reader really sees; (b) a hand-made localized image cannot be regenerated when the UI changes
   and silently goes stale; (c) the lead's image rule (2x, lossless) is met by the golden pipeline only. A localized image
   comes later, only for a pane whose catalogue rows exist in es.tsv, rendered by the golden harness with
   `WOL_LANG=es` (the way `setup-language-ja-1280` is made), never by hand. Alt text describes the picture in Spanish and
   names on-screen labels as rule 4 says.
9. **Length budgets.** Site (DESIGN.md "Languages"): navigation label 14 characters (same in this language), button 28, eyebrow 36, h1/h2/h3
   60/70/50 (same in this language), card title 50, card sentence 140, table header 24, search placeholder 48, page title 60, meta description
   120 to 160 characters, share-image headline 70 characters. OS: the `budget` column of `en.tsv` and `budgets.tsv`, in columns (a Latin letter is 1 column); the build refuses
   a longer line. If a natural line does not fit, shorten with a truer, shorter word first; if no honest wording fits, keep
   the natural line out of the file and post `@main` with the key, the line and its width: budgets are raised by the CTO, never
   worked around with abbreviations or by cutting meaning.
10. **One voice per language.** Only the es worker writes es files. Do not touch English, other languages, code or CSS.
11. **The Doctrine** stays English as the reference with the translation beside it (lead #1147); translate
    `src/content/doctrine.md` into `src/content/es/pages/doctrine.md` without softening or adding a word.
12. **Release-tied words.** Version numbers (0.12, 0.13) stay as numbers. "Arriving after 0.13" and similar dated lines are
    not translated until the English is corrected (lead 01:17Z: the Craft Suite has shipped).

## Verify before handing back

Site/manual: `node scripts/locales.mjs` after `npm run gen` (no `locales:` problem lines for es), `npm run test:l10n`,
`npm run l10n:status -- --missing es`, `CHROME=<chromium> node scripts/check-l10n.mjs --only /es,/es/manual/getting-started`
(only once es is in a build), `node scripts/check-links.mjs`, `node scripts/check-seo.mjs`. OS (when the CTO opens it):
`wol-i18n check crates/shell/locale/shell`, `wol-i18n status ...` (es 100 %, 0 stale, 0 errors), `wol-i18n budgets ...`
(list every key with headroom < 0 instead of shortening it).

# WolOS in Simplified Chinese: rules for translators

Copied from the joint rulebook `RULEBOOK-zh-Hans.md` (TRANSL-ARCH, 2026-11-06) by OS-LANGS; it binds the OS rows too.

These rules bind every line of Simplified Chinese on three surfaces: the site (chrome `src/i18n/zh-Hans.json`, page prose
`src/content/zh-Hans/pages/*.json`, posts `src/content/zh-Hans/blog/*.md`), the manual (`src/content/zh-Hans/manual/*.md`) and the
OS (`crates/shell/locale/shell/zh-Hans.tsv`, and every later unit). In the OS this page is `docs/l10n/RULES-zh-Hans.md` (`Lang::ZhHans`, stage Filling: offered, every row of `zh-Hans.tsv` present and empty until translated, an empty row reads English); `GLOSSARY.csv`, `STATE.md` and `PIPELINE.md` are the l10n planning files that came with it (`.claude/v014/l10n/` of the main checkout).
Status: proposed by TRANSL-ARCH 2026-11-06; **the zh-Hans worker reads it first, raises any rule it disagrees with in #plan
before translating, and a native reviewer confirms it before the language goes live**. Terms: `GLOSSARY.csv` (same folder),
column `zh-Hans`; a glossary word is used every time, and a change to it is a glossary change first.

## Register and tone

Neutral and polite, **你** where a pronoun is unavoidable (never 您 in UI; never 亲), but leave the subject out where Chinese allows it. Instructions as plain imperatives (`选择…`, `打开…`); 请 only where a request would read as an order, not on every line. Buttons and labels are short verbs or nouns (`取消`, `重试`). No ！.

## Capitals

No capitalisation. Latin names keep their spelling.

## Punctuation and quotation marks

Full-width ，。：；？（） in Chinese text; curly quotes “…” around a filled-in name or on-screen label (full-width 「」 are not used in zh-Hans). Enumeration comma 、 between list items. Ellipsis …… (two U+2026) in prose, single … only where the UI string has one. **One half-width space between Han characters and Latin words or digits** (`在 Store 中`, `16 GB`), none next to full-width punctuation. This is the opposite of the ja rule on purpose: it is the common practice of current Simplified Chinese UI.

## Numbers, dates, units, spelling

Half-width digits, comma for thousands only above 9 999 in tables, `100%`. Dates `2026年11月6日`, `2026-11-06` in tables; 24-hour `14:30`.

## Plural forms

`other` only; measure words (`{n} 个文件`, `{n} 项`).

## Length

Han = 2 columns; Chinese is the shortest language (often 0.8 to 1.1 times the English columns). Site CJK budgets apply (nav 7, h1 30, description 40 to 110: write full sentences, do not pad).

## Names in a sentence

No articles: `打开 Store`, `在 Control Center 中`.

## Banned in Simplified Chinese (besides rule 5)

尚未 (as a status), 即将推出, 计划中, 预览, 未经验证, 审核中; 失败, 错误, 无效, 不可用 in OS sentences (say 无法…).

## Examples

| English | Good | Bad | Why bad |
|---|---|---|---|
| “{network}” didn’t answer. Try again, or choose Not Now and join later. | “{network}”没有响应。请重试，或选择“以后”，稍后再加入。 | 连接 {network} 失败！ | Banned word, exclamation, no next step. |
| Control Center, at the top right, has Wi-Fi, sound, brightness and Do Not Disturb. | 右上角的 Control Center 中有 Wi-Fi、声音、亮度和勿扰模式。 | 您可以在右上角的控制中心找到 `WiFi`… | 您, translated kept name, respelled Wi-Fi. |
| Ask the manual | 向手册提问 | 请您咨询手册！ | 您, 请 on a button, exclamation. |
| This disk is empty, so there’s nothing to install alongside. Use Erase Disk. | 此磁盘是空的，没有可以与之并存安装的系统。请使用“抹掉磁盘”。 | 错误：无效磁盘。 | Banned words, no action. |

Examples are written to show the rules; the zh-Hans worker replaces any that a native writer would phrase differently and
says so in its report.

## Notes for this language

Simplified characters only, mainland vocabulary (文件 not 档案, 软件 not 软体, 视频 not 影片). The site needs `Noto Sans CJK SC` for share images on the build box.

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
   the OS shows it **in Simplified Chinese**: take it from `zh-Hans.tsv`. If that text is not in the catalogue yet (most of Settings, the
   Control Center, the Store, widgets: STATE.md 1c), the screen shows English, so the manual quotes the **English** label and
   may add the meaning once in parentheses: 点按 **Show date**（显示日期）. Never invent a localized label the screen does not show.
5. **Banned words** (GLOSSARY.csv kind `banned`): upstream project names (Hyprland: say the compositor or the desktop; AUR:
   the community package collection; nobody credited in THIRD-PARTY.md), the status hedges of the site ("not yet", "coming
   soon", "planned", "preview", "unverified", "in review", "claimed") and their Simplified Chinese equivalents listed below, "the user",
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
   screenshot is what a Simplified Chinese reader really sees; (b) a hand-made localized image cannot be regenerated when the UI changes
   and silently goes stale; (c) the lead's image rule (2x, lossless) is met by the golden pipeline only. A localized image
   comes later, only for a pane whose catalogue rows exist in zh-Hans.tsv, rendered by the golden harness with
   `WOL_LANG=zh-Hans` (the way `setup-language-ja-1280` is made), never by hand. Alt text describes the picture in Simplified Chinese and
   names on-screen labels as rule 4 says.
9. **Length budgets.** Site (DESIGN.md "Languages"): navigation label 14 characters (7 in this language), button 28, eyebrow 36, h1/h2/h3
   60/70/50 (30/35/25 in this language), card title 50, card sentence 140, table header 24, search placeholder 48, page title 60, meta description
   40 to 110 characters, share-image headline 35 characters. OS: the `budget` column of `en.tsv` and `budgets.tsv`, in columns (a CJK character counts 2 columns); the build refuses
   a longer line. If a natural line does not fit, shorten with a truer, shorter word first; if no honest wording fits, keep
   the natural line out of the file and post `@main` with the key, the line and its width: budgets are raised by the CTO, never
   worked around with abbreviations or by cutting meaning.
10. **One voice per language.** Only the zh-Hans worker writes zh-Hans files. Do not touch English, other languages, code or CSS.
11. **The Doctrine** stays English as the reference with the translation beside it (lead #1147); translate
    `src/content/doctrine.md` into `src/content/zh-Hans/pages/doctrine.md` without softening or adding a word.
12. **Release-tied words.** Version numbers (0.12, 0.13) stay as numbers. "Arriving after 0.13" and similar dated lines are
    not translated until the English is corrected (lead 01:17Z: the Craft Suite has shipped).

## Verify before handing back

Site/manual: `node scripts/locales.mjs` after `npm run gen` (no `locales:` problem lines for zh-Hans), `npm run test:l10n`,
`npm run l10n:status -- --missing zh-Hans`, `CHROME=<chromium> node scripts/check-l10n.mjs --only /zh-Hans,/zh-Hans/manual/getting-started`
(only once zh-Hans is in a build), `node scripts/check-links.mjs`, `node scripts/check-seo.mjs`. OS (when the CTO opens it):
`wol-i18n check crates/shell/locale/shell`, `wol-i18n status ...` (zh-Hans 100 %, 0 stale, 0 errors), `wol-i18n budgets ...`
(list every key with headroom < 0 instead of shortening it); golden renders of Setup and the installer in zh-Hans (`crates/shell/tests/golden_cases/locale.rs`) with noto-fonts-cjk.

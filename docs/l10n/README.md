# Translating WolOS

This is the guide for people who translate WolOS and for the people who add text to it. You do not need to know how to program. You need a text editor that keeps tabs as tabs (any editor that shows a tab character will do) and care for the person who will read your words. The design behind it is [ADR 0045](../adr/0045-locale-system.md).

## Supported languages and the order they are translated in

WolOS is offered in seven official languages, in this order (the order the installer, Setup and Settings list them, `wol_core::region::LANGUAGES`). Five more are planned next (`wol_core::region::LATER`): their rules are written, and they are not offered until their translation is complete and checked. A language not on this list shows English. The test `rules_set_equals_languages` (`crates/core/src/region.rs`) keeps the rule pages below and the two lists the same.

| Step | Language | File | Rules | Status |
|---|---|---|---|---|
| 1 | English, the source, written and debugged first (every pane in the catalog, every `xx` golden clean) | `en.tsv` | [RULES-en.md](RULES-en.md) | official |
| 2 | Brazilian Portuguese | `pt-BR.tsv` | [RULES-pt-BR.md](RULES-pt-BR.md) | official |
| 3 | Japanese | `ja.tsv` | [RULES-ja.md](RULES-ja.md) | official |
| 4 | French | `fr.tsv` | [RULES-fr.md](RULES-fr.md) | official |
| 5 | Russian | `ru.tsv` | [RULES-ru.md](RULES-ru.md) | official |
| 6 | Italian | `it.tsv` | [RULES-it.md](RULES-it.md) | official |
| 7 | German | `de.tsv` | [RULES-de.md](RULES-de.md) | official |
| 8 | Korean | `ko.tsv` | [RULES-ko.md](RULES-ko.md) | planned next |
| 9 | Indonesian | `id.tsv` | [RULES-id.md](RULES-id.md) | planned next |
| 10 | Tagalog | `tl.tsv` | [RULES-tl.md](RULES-tl.md) | planned next |
| 11 | Dutch | `nl.tsv` | [RULES-nl.md](RULES-nl.md) | planned next |
| 12 | Polish | `pl.tsv` | [RULES-pl.md](RULES-pl.md) | planned next |

An official language ships whole: every line of every unit translated, none stale (`wol-i18n status` at 100 %, test `every_official_language_is_complete` in `crates/shell/tests/locale_source.rs`).

How a language is translated (the rules of the project, which every `RULES-<lang>.md` repeats for its language):

- **One dedicated translator per language.** Each language is translated by its own translator agent, briefed with its `RULES-<lang>.md` and this guide. The main agent never translates, and one translator never does two languages, so each language keeps one voice.
- **Natural, not literal.** Write what a native speaker would say in the same moment, in a natural equivalent language that keeps the tone and the emotion of the English: calm, plain, kind, never blaming. A word-for-word copy that reads like a translation is a bug. Idioms are replaced by the local way of saying the same thing; humour and warmth stay.
- **Rigid parts stay fixed.** Product names (WolOS, Cryswol, Wolkit and the app names), commands and program names (`wol`, `wol-settings`), file names and keys, codec and unit names, and every `{placeholder}` are not translated, not respelled and not reordered inside their braces. The list is in each RULES page, "Terms that are never translated".
- **Nothing ships on trust.** The build refuses a line that breaks a rule (placeholders, plural forms, length budget); `wol-i18n status` shows the coverage; a native reviewer confirms the language’s RULES page before its first line lands and its panes after.

The language codes are `en`, `pt-BR`, `ja`, `fr`, `ru`, `it` and `de` (and, when they land, `ko`, `id`, `tl`, `nl` and `pl`); `xx` is the test language below and is never translated.

## Where the words live

Every part of WolOS that shows text is a unit with its own folder, `locale/<unit>/`. The shell (the menu bar, Settings, the Control Center) is `crates/shell/locale/shell/`. In a unit folder:

| File | Who edits it | What it is |
|---|---|---|
| `en.tsv` | developers | The English. Never edit it to translate. |
| `<lang>.tsv`, for example `de.tsv` | you | Your language: one line for every English line. |

A `.tsv` file is a table. Each line is one piece of text, and the columns are separated by a tab character. Lines that start with `#` are notes and are ignored.

## Your file

Start or update your file with the helper tool, which writes every line for you and keeps your earlier work:

```
cargo run -p shared --bin wol-i18n -- sync crates/shell/locale/shell de
```

Your file has four columns:

```
# key  |  source  |  text  |  hash
settings.sound_louder  |  Lets volume sliders pass 100%. Sound above 100% can distort.  |  |
```

(Above, ` | ` stands for one tab character, which is what separates the columns in the real file.)

1. **key** is the name of the text. Never change it.
2. **source** is the English as it is now. Read it, do not change it. The tool rewrites it.
3. **text** is yours. Write the translation here. Empty means "not translated yet", and WolOS shows the English until you do.
4. **hash** is a fingerprint of the English you translated. Leave it alone. When the English changes later, the tool notices that your line was made for older English and marks it, so you know to look at it again. Your line keeps working until you do.

The English file also has a comment for each text (where it appears, what a `{name}` stands for). Read it in `en.tsv` before you translate a line you are not sure about.

## Rules that the build checks

If you break one of these, the build stops and tells you the file and the key. That is on purpose: a wrong line never reaches a person.

- **Keep every `{placeholder}` exactly.** `Join “{name}”.` must keep `{name}` in your translation, spelled the same, with nothing added and nothing missing. It is filled in with a real value (a network name, a number). You may move it anywhere in the sentence. To show a real brace write `{{` or `}}`.
- **Plurals.** Some texts come in forms for a count, written as `files_count#one` and `files_count#other`. Write one line for each form your language uses; the tool lists exactly those. English and German use `one` and `other`. French, Italian and Portuguese also have `many` (for whole millions: "1 000 000 de arquivos"). Russian uses `one`, `few`, `many` and `other` ("1 минута", "2 минуты", "5 минут"). Japanese uses `other` only. `{n}` is the count.
- **Length.** Screens do not stretch. Some texts have a length limit (the `budget` column in `en.tsv`, counted in columns; a Japanese character counts as two). Your text may not be longer. If it will not fit, say the same thing with fewer words. Tell the developers about a limit that is too small instead of cutting meaning.
- **One tab per column, no tab or line break inside a text.** If you need a line break inside a text, write `\n`; for a real backslash write `\\`.
- **Plain, kind, short.** WolOS says what happened and what to do, in one or two sentences, and never blames the person. Keep that. Do not translate names of products and programs (WolOS, Wi-Fi, Bluetooth). Use the words people already know in your language for things like "Wi-Fi" and "password", not a literal copy of the English.
- **Do not name other projects or people.** WolOS never mentions the software it is built from; if the English does not, you do not.

Each language has its own written rules (formal or informal "you", quotation marks, number formats, plural forms, length, what `xx` tests for it): [en](RULES-en.md) (the source), [pt-BR](RULES-pt-BR.md), [ja](RULES-ja.md), [fr](RULES-fr.md), [ru](RULES-ru.md), [it](RULES-it.md), [de](RULES-de.md); planned next: [ko](RULES-ko.md), [id](RULES-id.md), [tl](RULES-tl.md), [nl](RULES-nl.md), [pl](RULES-pl.md). Read yours first. A native reviewer confirms each page before the first line of that language is translated.

## Check your work

```
cargo run -p shared --bin wol-i18n -- check crates/shell/locale/shell
cargo run -p shared --bin wol-i18n -- status crates/shell/locale/shell
cargo run -p shared --bin wol-i18n -- budgets crates/shell/locale/shell
```

- `check` compiles every language and prints each problem with the file and key. No output but `ok` means it will build.
- `status` shows, for each language, how many lines exist, how many are translated, how many are stale, and how many have errors.
- `budgets` shows, for each text, its limit, how wide the English is, how wide the longest translation is, and how many columns are left.

## See your words on screen: the pseudo-language

The language `xx` is a test language. Every English text is shown with accents, about 40% longer, and inside brackets: `[Ŀåñĝüåĝé ~~]`. Developers use it to find places where a longer language would be cut off, and to find any text that is still written in the program instead of in these files (it stays plain English, with no brackets). You never translate `xx`.

To see a program in it: `WOL_LANG=xx wol-settings`.

To see your own language before it is shipped, put your language in the session instead: `WOL_LANG=de wol-settings`.

## Where a person chooses the language

- **Installing.** The installer's Language and Keyboard page lists the seven official languages in the order above: English, Português (Brasil), 日本語, Français, Русский, Italiano, Deutsch. The new system gets that language (`LANG` in `/etc/locale.conf`) and every official locale is generated, so a later change needs no root.
- **First start.** The setup assistant's Language & Region page lists the same seven, in the same order (test `languages_match_settings`). It starts in the language the installer chose and speaks the one the person picks at once; the choice is saved as the new account's language.
- **Later.** System Settings, General, Language & Region, "Preferred language", the same list. Apps use the new language the next time the person logs in.
- **How WolOS decides when it starts.** In order: `WOL_LANG` (for tests), `LC_ALL`, `LC_MESSAGES`, `LANG`. Portuguese of any region reads Brazilian Portuguese; a language with no translation, such as `es_ES` or `zh_CN` saved by an older WolOS, reads English.
- **Any line missing in your language** shows the English for that line, never a blank.

## For developers adding text

1. A sentence of a copy module (`copy_*.rs`) is a `pub const` with a `///` comment that tells the translator where it appears and what each `{name}` is; add a line `/// budget: N` after it if its space is limited (N must leave room for the pseudo-language, 40% more). A text written where it is used, or a const without that line, gets its limit from `locale/<unit>/budgets.tsv` (`key`, tab, columns; a key the unit does not have is an error). The shell's unit bounds all 223 of its texts of 40 columns or fewer (buttons, labels, titles, row texts): the larger of the pseudo-language width and the widest of the seven translations plus 15%; longer sentences wrap and have none. A surface shows it with `i18n::tr(copy::X)`.
2. Text written where it is used is wrapped where it is used: `tr("Choose a password.")`, `tr_fill("Joining “{network}”…", &[("network", ssid)])` for a template, `trn("About {n} minute remaining", "About {n} minutes remaining", n)` for a count, and `n_("Swap file")` inside a `const` table (then `tr(row.1)` where it is shown). A `// l10n: …` line right above the call is the translator's comment. The key is made from the English (`setup.choose_a_password`).
3. List the file in `locale/<unit>/sources.tsv` and run `wol-i18n extract <unit-dir>` to refresh `en.tsv`. The build refuses a stale `en.tsv`.
4. Look at the pane in `xx`. If it overflows or breaks, fix the layout before any translator sees it.
5. Add or update the golden in `crates/shell/tests/golden_cases/locale.rs`.

For a Crystal app (once the libWolkit door of `feat/i18n-os` is merged; this tree has the shell's door only): put `locale/<unit>/en.tsv` in the app, run `wol-i18n compile locale/<unit> build/locale`, and call `Wolkit::I18n.load("build/locale", "<unit>")` once at start and `Wolkit::I18n.language = ENV["LANG"]` to pick the language; then `text t("greeting")` in a view relabels when the language changes.

## Migration status (ADR 0045, step 3; L10N-2)

In the catalog and translated into the seven official languages: the installer (`copy_installer.rs`, `install_flow.rs`, `ui/installer.rs`, `ui/installer/disks.rs`), Setup (`setup.rs`, `ui/setup.rs`), the assistant kit's buttons (`kit/assistant.rs`), Settings > Language & Region (`ui/settings/language.rs`) and the shell's sentence table `copy.rs` (its `pub const` sentences): 380 keys, `locale/shell/sources.tsv`. Still English in the code, for the next worker: the functions of `copy.rs` that build a sentence, `copy_settings.rs` and `copy_maker.rs`, the rest of Settings, the Control Center, the Store and the desktop widgets; the key-screen steps of `wol_core::boot` and the keyboard layout names of `wol_core::prefs::LAYOUTS` (core text, shown by Setup and the installer). Do one pane at a time: list its file in `locale/shell/sources.tsv`, wrap its text (`tr`, `tr_fill`, `trn`, `n_`), run `wol-i18n extract`, translate the new rows in every official language, add its `xx` golden to `tests/golden_cases/locale.rs` and fix whatever overflows.

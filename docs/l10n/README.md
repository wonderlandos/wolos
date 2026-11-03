# Translating WolOS

This is the guide for people who translate WolOS and for the people who add text to it. You do not need to know how to program. You need a text editor that keeps tabs as tabs (any editor that shows a tab character will do) and care for the person who will read your words. The design behind it is [ADR 0045](../adr/0045-locale-system.md).

WolOS is translated into ten languages: English (the source), Brazilian Portuguese `pt-BR`, Spanish `es`, French `fr`, German `de`, Japanese `ja`, Korean `ko`, simplified Chinese `zh-Hans`, Indonesian `id` and Tagalog `tl`. A language not on this list shows English.

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
- **Plurals.** Some texts come in forms for a count, written as `files_count#one` and `files_count#other`. Write one line for each form your language uses; the tool lists exactly those. English and German use `one` and `other`. Spanish, French and Portuguese also have `many` (for whole millions: "1 000 000 de arquivos"). Japanese, Korean, Chinese and Indonesian use `other` only, and Tagalog uses `one` and `other`. `{n}` is the count.
- **Length.** Screens do not stretch. Some texts have a length limit (the `budget` column in `en.tsv`, counted in columns; a Japanese, Korean or Chinese character counts as two). Your text may not be longer. If it will not fit, say the same thing with fewer words. Tell the developers about a limit that is too small instead of cutting meaning.
- **One tab per column, no tab or line break inside a text.** If you need a line break inside a text, write `\n`; for a real backslash write `\\`.
- **Plain, kind, short.** WolOS says what happened and what to do, in one or two sentences, and never blames the person. Keep that. Do not translate names of products and programs (WolOS, Wi-Fi, Bluetooth). Use the words people already know in your language for things like "Wi-Fi" and "password", not a literal copy of the English.
- **Do not name other projects or people.** WolOS never mentions the software it is built from; if the English does not, you do not.

Each language gets its own written rules (formal or informal "you", spelling variant, how to write numbers and dates, glossary) in `docs/l10n/RULES-<lang>.md`. They are written before the translation of that language starts; read yours first.

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

- **First start.** The setup assistant's Language & Region page (after Welcome) shows the languages. The person's choice is used at once by the assistant and is saved as the new account's language.
- **Later.** System Settings, General, Language & Region, "Preferred language". The shell and Settings change at once; other apps use the new language the next time the person logs in.
- **How WolOS decides when it starts.** In order: `WOL_LANG` (for tests), `LC_ALL`, `LC_MESSAGES`, `LANG`. `pt_PT` and other Portuguese read Brazilian Portuguese; `zh_CN` and `zh_SG` read simplified Chinese; a language with no translation, such as `it_IT` or `zh_TW`, reads English.
- **Any line missing in your language** shows the English for that line, never a blank.

## For developers adding text

1. Put the sentence in the unit's copy module, with a `///` comment that tells the translator where it appears and what each `{name}` is. Add a line `/// budget: N` after it if its space is limited (N must leave room for the pseudo-language, 40% more).
2. Run `wol-i18n extract <unit-dir>` to refresh `en.tsv`. The build refuses a stale `en.tsv`.
3. In the surface, never write the sentence: ask for the key, `i18n::t(keys::SETTINGS_SOUND_LOUDER)` in the shell, `t("settings.sound_louder")` in a Wolkit app (Crystal), or the C functions `wolkit_catalog_find` and `wolkit_catalog_text` from `wolkit.h`.
4. Look at the pane in `xx`. If it overflows or breaks, fix the layout before any translator sees it.
5. Add or update the golden in `crates/shell/tests/golden_cases/locale.rs`.

For a Crystal app: put `locale/<unit>/en.tsv` in the app, run `wol-i18n compile locale/<unit> build/locale`, and call `Wolkit::I18n.load("build/locale", "<unit>")` once at start and `Wolkit::I18n.language = ENV["LANG"]` to pick the language; then `text t("greeting")` in a view relabels when the language changes.

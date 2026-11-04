# WolOS in Japanese: rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `ja.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 3 of 12, an official language (README.md, "Supported languages"): offered in the installer, Setup and Settings, in this order. It is translated in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Japanese, file `ja.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Polite form (**です・ます体**), never plain form and never honorific-heavy (*でございます*). No second-person pronoun at all (do not write *あなた*); leave the subject out. No exclamation marks. Buttons are short nouns or verb stems (`キャンセル`, `参加`), sentences end in `です`/`ます`/`ください`.

## Capitals and sentence ends

No capitalisation. Latin product terms keep their spelling (`Wi-Fi`, `Bluetooth`).

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Corner brackets 「…」 around a filled-in name (`「{name}」に参加`). Full-width punctuation `。` `、` `？` in sentences; none at the end of a label. The ellipsis is … (U+2026), the same character as the English. No space between Japanese and Latin letters or digits.

## Numbers, dates, times and spelling

Half-width digits, comma for thousands (`1,234.5`), `100%` without a space. Dates `2026/11/03`, 24-hour time `14:30`, formatted by the system. Long vowels in katakana loanwords end with ー (`ユーザー`, `フォルダー`, `ブラウザー`): choose this style everywhere.

## Plural forms

`other` only. There is one row per key, no `#one`. Use a counter where the language needs one (`{n} 個`, `{n} 件`) and keep `{n}`.

## Length: budgets per kind of key

Counted in columns, a Japanese character is **2 columns**. A label of 8 characters is 16 columns; the budget in `en.tsv` is in columns, so compare like with like: expect 1.0 to 1.4 times the English columns. Short true words win (`音量` not `ボリューム` when both fit the register).

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Line breaks inside a sentence are not needed: the pane wraps at any character. Do not insert spaces to imitate word breaks. Use the full-width form only for Japanese punctuation; ASCII punctuation stays for URLs, versions and commands.

## What the pseudo-locale `xx` tests for this language

`xx` is Latin text: it cannot show Japanese glyph clipping. After translating, look at each pane with a real Japanese string and check the font fallback draws every character. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

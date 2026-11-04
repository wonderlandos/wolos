# WolOS in Chinese (Simplified): rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `zh-Hans.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 5 of 10 in the gradual plan (README.md, "Supported languages"). This language is translated by its own dedicated translator agent, never by the main agent, in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Chinese (Simplified), file `zh-Hans.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Plain and polite, with no pronoun where possible. If the person must be named, use **你**, not *您*. No exclamation marks, no *请* in front of every instruction (only where a request would otherwise read as a command).

## Capitals and sentence ends

No capitalisation. Latin product terms keep their spelling (`Wi-Fi`, `Bluetooth`), with no space between them and the Chinese around them.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Full-width punctuation: `，` `。` `：` `；` `？`. Quotation marks “…” (U+201C, U+201D) around a filled-in name (`加入“{name}”`). The ellipsis … is the single character, as in English. Half-width digits and Latin letters, no spaces around them.

## Numbers, dates, times and spelling

Half-width digits, comma for thousands (`1,234.5`), `100%` without a space. Dates `2026/11/3` or `2026年11月3日` as the system formats them, 24-hour time. Use **Mainland** vocabulary in simplified characters only: 文件, 软件, 视频, 默认, 登录, never Taiwan or Hong Kong terms and never traditional characters.

## Plural forms

`other` only. One row per key. Keep `{n}` and the measure word (`{n} 个`, `{n} 项`).

## Length: budgets per kind of key

A Chinese character is **2 columns**. Expect 0.9 to 1.2 times the English columns. Four-character labels are usual (`深色模式`).

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Do not add spaces between Chinese words. A long unbroken sentence wraps at any character, so write short sentences with `，` and `。`.

## What the pseudo-locale `xx` tests for this language

`xx` is Latin text: it cannot show Chinese glyph clipping. After translating, look at each pane with a real Chinese string and check the font fallback. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

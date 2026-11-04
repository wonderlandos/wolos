# WolOS in Tagalog (Filipino): rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `tl.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 10 of 10 in the gradual plan (README.md, "Supported languages"). This language is translated by its own dedicated translator agent, never by the main agent, in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Tagalog (Filipino), file `tl.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Address the person with the neutral **ka / mo / iyong** forms, never the deferential *po/ho* in every sentence and never *ikaw* as a subject where it can be left out. Instructions are imperatives (`Pumili`, `Subukan muli`) kept short. No exclamation marks.

## Capitals and sentence ends

Sentence case for labels, buttons and titles (`Madilim na mode`); only the first word and proper names are capitalised. A sentence ends with a full stop; labels and state lines do not.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Curly quotes “…” around a filled-in name (`Sumali sa “{name}”`). The ellipsis … has no space before it.

## Numbers, dates, times and spelling

Decimal point, comma for thousands (`1,234.5`), `100%` without a space. Dates `11/3/2026` and 12-hour time as the system formats them. Use the Filipino most people use on a screen: keep widely used English words (*computer*, *password*, *Wi-Fi*, *file*, *app*) and translate the rest; do not invent deep-Tagalog words (*salumpuwesto*) for common things.

## Plural forms

`one` for the counts 0, 1, 2, 3, 5, 7, 8, 10 and so on and `other` for counts ending in 4, 6 or 9 (4, 6, 9, 14, 16, 19, …), the CLDR rule (`crates/shared/src/i18n/plural.rs`). Do not treat `one` as "exactly one": write a form that reads right with 0 to 3 and with 5. The plural marker `mga` goes before the noun in both forms when a count is shown.

## Length: budgets per kind of key

Expect +25 to +35 % over English; Tagalog uses longer function words. Prefer the shorter common word.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Linkers (`na`, `-ng`) change with the last sound of the word before them: a filled-in `{name}` can end in either kind, so keep `{name}` in quotes and write the linker on the noun that follows (`ang network na “{name}”`).

## What the pseudo-locale `xx` tests for this language

The accents of `xx` do not occur in Tagalog; they only test height. The 40 % extra covers the usual growth. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

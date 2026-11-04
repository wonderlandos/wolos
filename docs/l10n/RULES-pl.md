# WolOS in Polish: rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `pl.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 12 of 12, planned next (README.md, "Supported languages"): this language is listed but not offered in the installer, Setup or Settings until its translation is complete and checked. It is translated in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Polish, file `pl.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Address the person directly in the second person singular, never *Pan/Pani*, and prefer impersonal forms where a gendered past tense would show (`Zalogowano`, not `Zalogowałeś`). Instructions use the imperative (`Wybierz`, `Spróbuj ponownie`). Calm and plain; no *proszę*, no exclamation marks.

## Capitals and sentence ends

Sentence case for labels, buttons and titles (`Tryb ciemny`, `Duplikowanie ekranu`). A sentence ends with a full stop; labels and state lines do not.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Polish quotation marks „…” around a filled-in name (`Połącz z „{name}”`). The dash in a sentence is an en dash with spaces ( – ). The ellipsis … has no space before it. Never drop ą, ć, ę, ł, ń, ó, ś, ź, ż.

## Numbers, dates, times and spelling

Decimal comma, a no-break space for thousands (`1 234,5`), `100%` without a space. Dates `03.11.2026`, 24-hour time `14:30`, formatted by the system.

## Plural forms

`one` for 1, `few` for 2–4, 22–24… (not 12–14), `many` for 0, 5–21, 25–31…, `other` for fractions. Write all four rows of a key with a count (`{n} plik`, `{n} pliki`, `{n} plików`, `{n} pliku`).

## Length: budgets per kind of key

Expect +20 to +35 % over English. Prefer the short verb; avoid noun chains in the genitive.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Latin product names stay as written and are not declined when that can be avoided by the sentence (`w systemie WolOS`).

## What the pseudo-locale `xx` tests for this language

Diacritics in `xx` show that the pane carries Polish letters without clipping; the 40 % extra covers the usual growth. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

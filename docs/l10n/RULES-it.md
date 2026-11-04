# WolOS in Italian: rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `it.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 6 of 12, an official language (README.md, "Supported languages"): offered in the installer, Setup and Settings, in this order. It is translated in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Italian, file `it.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Address the person as **tu**, never *Lei* or *voi*, with the *tu* imperative in sentences (`Scegli`, `Riprova`) and on buttons and short actions (`Annulla`, `Salva`, `Continua`). Calm and plain; no *per favore*, no exclamation marks. Avoid gendered words for the reader: `Accesso eseguito`, not `Sei connesso`.

## Capitals and sentence ends

Sentence case for labels, buttons and titles (`Modalità scura`, `Duplica schermo`); only the first word and proper names are capitalised. A sentence ends with a full stop; labels and state lines do not.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Curly quotes “…” around a filled-in name (`Connetti a “{name}”`); « » are not used on screen. Apostrophe ’ for elisions (`l’app`, `un’altra`, `c’è`). The ellipsis … has no space before it. Accented capitals are written with the accent (`È`), never `E’`.

## Numbers, dates, times and spelling

Decimal comma, dot for thousands (`1.234,5`), `100%` without a space. Dates `03/11/2026`, 24-hour time `14:30`, formatted by the system. Words people use: *file* (invariable), *cartella*, *password*, *scaricare*, *schermo*, *computer*, *app* (invariable).

## Plural forms

`one` for exactly 1, `many` for whole millions (`1 milione di file` takes *di*), `other` for the rest, 0 included (`0 file`). Write all three rows; `many` usually repeats `other` with *di*.

## Length: budgets per kind of key

Expect +15 to +30 % over English. Prefer the short verb (`Esci`, `Entra`) and drop *di* chains.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

English words Italians already use on a computer stay (*file*, *password*, *app*, *backup*); choose one form per concept and use it everywhere. Articles before English nouns follow the Italian sound (`il Wi-Fi`, `lo spazio`).

## What the pseudo-locale `xx` tests for this language

Accents (à, è, é, ì, ò, ù) in `xx` show that the pane carries Italian diacritics without clipping; the 40 % extra covers the usual growth. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

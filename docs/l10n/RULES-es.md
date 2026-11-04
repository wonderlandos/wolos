# WolOS in Spanish: rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `es.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 6 of 10 in the gradual plan (README.md, "Supported languages"). This language is translated by its own dedicated translator agent, never by the main agent, in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Spanish, file `es.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Address the person as **tú**, never *usted* and never *vos*. Instructions use the *tú* imperative (`Elige`, `Inténtalo de nuevo`) everywhere. Plain and warm like the English; no *por favor*, no exclamation marks, no inverted exclamation at all.

## Capitals and sentence ends

Sentence case for every label, button and title (`Modo oscuro`, `Duplicar pantalla`); only the first word and proper names are capitalised. A sentence ends with a full stop; labels and state lines do not. Questions and the rare exclamation take the opening mark (`¿…?`).

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Curly quotes “…” around a filled-in name, the same as the English template. The ellipsis … has no space before it. Never drop accents, in capitals either.

## Numbers, dates, times and spelling

Decimal comma, dot for thousands from five digits (`12.345`, `1234`), and a no-break space before the percent sign (`100 %`). Dates `3/11/2026`, 24-hour time `14:30`, formatted by the system. Use neutral international Spanish: *equipo* for the computer, *archivo*, *pantalla*, *contraseña*, *descargar*; avoid words of one country only (*ordenador*, *computadora*, *celular*, *móvil*: write *teléfono*).

## Plural forms

`one` for exactly 1, `many` for whole millions (`1 000 000 de archivos` needs *de*), `other` for all else. Write all three rows; `many` repeats `other` with *de*.

## Length: budgets per kind of key

Expect +20 to +30 % over English. Prefer short true words and the infinitive or noun where English uses a verb phrase.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Gender-neutral wording for the reader: avoid `-o/-a` pairs and `@`; write a form that needs no agreement (`Te damos la bienvenida`, not `Bienvenido/a`).

## What the pseudo-locale `xx` tests for this language

Accents, ñ and the opening marks are not in `xx`; the pseudo accents stand for them. The 40 % extra covers the usual growth. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

# WolOS in Portuguese (Brazil): rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `pt-BR.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 2 of 12, an official language (README.md, "Supported languages"): offered in the installer, Setup and Settings, in this order. It is translated in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Portuguese (Brazil), file `pt-BR.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Address the person as **você**, never *tu* or *o senhor*. Instructions use the imperative of *você* (`Escolha`, `Tente de novo`), one form everywhere. Warm and plain, like the English; no *por favor*, no exclamation marks. Say the neutral *a pessoa* only where English says "someone"; avoid gendered words for the reader (write `Conectado`/`Conectada` as `Conexão ativa`, `Bem-vindo` as `Boas-vindas`).

## Capitals and sentence ends

Sentence case for labels, buttons and titles (`Modo escuro`, `Espelhamento de tela`), not Title Case; only the first word and proper names are capitalised. A sentence ends with a full stop; labels and state lines do not.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Curly quotes “…” around a filled-in name (`Entrar em “{name}”`), curly apostrophe ’ where needed. The ellipsis … has no space before it. Accents are never dropped, even in capitals.

## Numbers, dates, times and spelling

Decimal comma, dot for thousands (`1.234,5`). `100%` has no space in Brazil (`100%`). A time is 24-hour (`14:30`) and a date `03/11/2026`, but the system formats them; do not type them into a string. Use the Brazilian spelling (*arquivo*, *tela*, *usuário*, *baixar*, *celular*), never the Portuguese-from-Portugal one.

## Plural forms

`one` for 0 and 1, `many` for whole millions (`1 milhão de arquivos` takes *de*), `other` for the rest. Write all three rows of a key with a count; `many` usually repeats `other` with *de*.

## Length: budgets per kind of key

Expect +20 to +30 % over English. Prefer a shorter true word (*Tela*, not *Visualização*), drop *de* chains, and use the verb where English uses a noun (`Entrar` for `Join`).

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Do not translate a product term for the sake of it. A technical word people already use in Brazil may stay in English when there is no common Portuguese word; choose one form per concept and use it everywhere (the glossary of this language fixes it).

## What the pseudo-locale `xx` tests for this language

Accents and ç in `xx` show that the pane carries Portuguese diacritics without clipping; the 40 % extra covers the usual growth. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

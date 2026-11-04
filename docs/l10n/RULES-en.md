# WolOS in English: the source rules

English is the source of every key. These rules say how the English is written so that the translations of it stay faithful. The general guide is [README.md](README.md).

Order: step 1 of 12 in the plan (README.md, "Supported languages"). English is written and debugged first, before any translation starts.

Language: English, file `en.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

WolOS speaks to one person, as "you". It does not say "the user", "we" or "please". It is calm and plain: what happened, then what to do, in one or two short sentences. It never blames ("couldn’t be read", not "you entered an invalid value").

## Capitals and sentence ends

Titles of tiles, menu items, buttons and page headings use Title Case (`Dark Mode`, `Screen Mirroring`). Row labels and everything after them use sentence case (`Allow louder`, `Keep device awake`). A sentence ends with a full stop; a label, a title and a state line (`Off`, `Connected`) do not.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Curly quotes “…” around a name that is filled in (`Join “{name}”`) and a curly apostrophe ’ (`couldn’t`, `WolOS’s`), never the straight ones. An ellipsis is the single character … and marks an item that opens more to answer (`Wi-Fi Settings…`).

## Numbers, dates, times and spelling

`1,234.5`; `100%` without a space; dates and times are formatted by the system from the person’s settings, never typed into a string.

## Plural forms

`one` (1) and `other` (everything else, 0 included). A key with a count has a `#one` and a `#other` row.

## Length: budgets per kind of key

This is the reference: every budget in `en.tsv` was set from the English plus room for 40 %. Other languages are measured against it.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Banned words in sentences (`copy::lint` in the shell, `tests/copy_lint.rs`): `failed`, `error`, `invalid`, `unavailable`, `daemon`, `socket`, program names and raw error text. English is the only language the lint reads, so it is also the source of the rule: if the English could not say it, the translation does not either.

## What the pseudo-locale `xx` tests for this language

English is the language `xx` is made from: every English string shown in brackets, accented and 40 % longer. If a string in a pane does not show brackets in `xx`, it is not in the catalog yet. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

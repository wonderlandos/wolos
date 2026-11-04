# WolOS in German: rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `de.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 7 of 10 in the gradual plan (README.md, "Supported languages"). This language is translated by its own dedicated translator agent, never by the main agent, in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: German, file `de.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Address the person as **du** (lower case), never *Sie*, with the *du* imperative (`Wähle`, `Versuche es erneut`) everywhere. Plain and friendly; no *bitte*, no exclamation marks.

## Capitals and sentence ends

Nouns are capitalised as German requires, but headings and labels otherwise use sentence case (`Dunkler Modus`, `Bildschirm spiegeln`). A sentence ends with a full stop; labels and state lines do not.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

German quotation marks „…“ around a filled-in name (`Mit „{name}“ verbinden`), apostrophe ’ only where the language needs one (`WolOS’`). The ellipsis … has no space before it. Never replace ä, ö, ü, ß with ae, oe, ue, ss.

## Numbers, dates, times and spelling

Decimal comma, dot for thousands (`1.234,5`), a no-break space before the percent sign (`100 %`). Dates `03.11.2026`, 24-hour time `14:30`, formatted by the system. Gender-neutral wording without the asterisk or colon forms: use a neutral noun or a plural (`Personen`, `alle`) instead of `Benutzer*innen`.

## Plural forms

`one` for exactly 1, `other` for the rest. Two rows per key with a count.

## Length: budgets per kind of key

Expect +30 to +40 % over English because of compounds. A long compound does not wrap: if one does not fit, rephrase with a shorter noun or a verb. Watch the budgets of tile titles and buttons first.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Keep English loanwords people use (*Backup*, *Download* → prefer *Herunterladen* in sentences, one choice per word everywhere). Verb at the end of a subordinate clause pushes the key word late: check the sentence still reads when it is cut at the budget.

## What the pseudo-locale `xx` tests for this language

`xx` is 40 % longer but breaks at spaces; German compounds do not. Test long compounds by hand against the key’s budget; the Control Center tile titles ellipsize. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

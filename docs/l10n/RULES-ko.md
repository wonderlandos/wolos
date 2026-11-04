# WolOS in Korean: rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `ko.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 8 of 10 in the gradual plan (README.md, "Supported languages"). This language is translated by its own dedicated translator agent, never by the main agent, in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Korean, file `ko.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Polite informal (**해요체**, `-요` endings: `선택하세요`, `다시 시도해 주세요`), never the plain form and never the formal `-습니다` register. No second-person pronoun (do not write *당신*); leave the subject out. No exclamation marks. Buttons and labels are noun or verb-stem forms (`취소`, `연결`).

## Capitals and sentence ends

No capitalisation. Latin product terms keep their spelling (`Wi-Fi`, `Bluetooth`).

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Curly quotes “…” around a filled-in name (`“{name}” 참가`); full stop `.` and the ellipsis … as in English. A space between words as Korean spelling needs it; no space between a Latin term and the particle that follows it unless the spelling rules need one.

## Numbers, dates, times and spelling

Half-width digits, comma for thousands, `100%` without a space. Dates `2026. 11. 3.`, time with `오전/오후` or 24-hour as the person chose, formatted by the system. Keep loanword spelling consistent with the national standard (`컴퓨터`, `사용자`).

## Plural forms

`other` only. One row per key. Use a counter (`{n}개`, `{n}명`) and keep `{n}`.

## Length: budgets per kind of key

A Hangul syllable is **2 columns**. Expect 1.0 to 1.3 times the English columns. Prefer short Sino-Korean nouns for labels (`음량`) when the register stays friendly.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Particles depend on the last sound of the word they follow (`을/를`, `이/가`, `은/는`). A filled-in `{name}` can end in either kind: write the sentence so no particle follows `{name}` directly (use a word between, or `{name}` in quotes followed by a noun).

## What the pseudo-locale `xx` tests for this language

`xx` is Latin text: it cannot show Hangul clipping. After translating, look at each pane with a real Korean string. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

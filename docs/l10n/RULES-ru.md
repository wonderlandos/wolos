# WolOS in Russian: rules for translators

These rules are written before the translation of this language starts and bind every line of it. They are the work of the project, not of a native reviewer yet: **a native reviewer confirms each rule below before the first `ru.tsv` lands**, and a rule they change is changed here first. The general guide (files, columns, placeholders, checks) is [README.md](README.md); this page only says what is particular to the language.

Order: step 5 of 12, an official language (README.md, "Supported languages"): offered in the installer, Setup and Settings, in this order. It is translated in natural language that keeps the tone and the emotion of the English; product names, commands, keys and placeholders stay fixed.

Language: Russian, file `ru.tsv` in each unit folder, plural categories as below. Words in backticks that are not part of the English text are examples of a rule, not approved translations.

## Address and tone

Address the person as **вы** (lower case inside a sentence), never *ты*. Instructions use the imperative of *вы* in sentences (`Выберите`, `Попробуйте ещё раз`) and the infinitive or a noun on buttons and short actions (`Отмена`, `Копировать`, `Продолжить`). Calm and plain; no *пожалуйста*, no exclamation marks. Avoid past-tense verbs that show the reader’s gender: write `Вход выполнен`, not `Вы вошли`/`вошла`.

## Capitals and sentence ends

Sentence case for labels, buttons and titles (`Тёмная тема`, `Повтор экрана`); only the first word and proper names are capitalised. A sentence ends with a full stop; labels and state lines do not.

## Terms that are never translated

WolOS, WolKernel, Cryswol, Wolkit and libWolkit, libwol, Wonderland; Wi-Fi, Bluetooth, USB, HDMI; the names of WolOS apps and tools as they appear in the Dock and in commands (`wol`, `wol-settings`); codec and format names (AAC, aptX, LDAC, Opus, SBC, PNG, PDF); units (kHz, ms, dB, GB, MB, %). A `{placeholder}` is never translated, reordered inside its braces or given a space. Do not name any other software, project or person in a text; if the English does not, you do not.

## Punctuation and quotation marks

Guillemets «…» around a filled-in name (`Подключиться к «{name}»`), „…“ only for a quote inside a quote. The dash in a sentence is an em dash with spaces ( — ). The ellipsis … has no space before it. Write ё where the word has it (`ещё`, `всё`, `тёмный`): it changes meaning in a few words and is the house style.

## Numbers, dates, times and spelling

Decimal comma, a no-break space for thousands (`1 234,5`), a no-break space before the percent sign (`100 %`). Dates `03.11.2026`, 24-hour time `14:30`, formatted by the system. Words a person knows from phones and computers: *файл*, *папка*, *пароль*, *загрузить* (download) and *выгрузить* only where English says upload, *экран*, *компьютер*, *приложение* (not *программа* for an app in the Dock).

## Plural forms

`one` for 1, 21, 31… (not 11), `few` for 2–4, 22–24… (not 12–14), `many` for 0, 5–20, 25–30…, `other` for fractions. Write all four rows of a key with a count (`{n} файл`, `{n} файла`, `{n} файлов`, `{n} файла`). The words after a count change case with it; never write a count with a fixed noun.

## Length: budgets per kind of key

Expect +15 to +30 % over English; words are long and buttons grow most (`Reset` → `Сбросить`, `Settings` → `Настройки`). Prefer the short noun or the infinitive; drop *данный*, *осуществить* and other office words.

| Kind of key | Budget in `en.tsv` (columns) | Notes |
|---|---|---|
| Tile title, state line, button | 8 to 24 | Single line. A tile title longer than about 12 columns ends in an ellipsis in the Control Center; the circle’s tooltip shows it whole. |
| Row label, picker label, section title | 16 to 56 | Single line; ellipsized past the budget in a narrow window. |
| Row subtitle | 90 | Wraps to two lines. |
| Footnote, note under a list | 160 | Wraps to three lines. |
| Sentence (message after an action, empty state) | set per key, at least English plus 40 % | Wraps; read `wol-i18n budgets` for the number. |

The number is the most columns the translation may take; a wide character (Japanese, Korean, Chinese) counts two. The build refuses a longer text and names the key. `wol-i18n budgets <unit-dir>` shows each key’s budget, the English and pseudo-locale width and the widest translation written so far, so you can see the room left before you write.

## Other rules

Cyrillic only for Russian words; Latin product names stay Latin and take no case ending (`в WolOS`, `для Bluetooth`); write the sentence so the name does not need one. *Wi-Fi* stays as written; *вайфай* is never used.

## What the pseudo-locale `xx` tests for this language

`xx` is Latin text: it cannot show Cyrillic glyph clipping or the extra width of Cyrillic capitals. After translating, look at each pane with a real Russian string, longest form of each plural. `xx` makes every string about 40 % longer, accents the letters, wraps it in brackets and keeps the `{placeholders}`; see README.md, "See your words on screen". A pane that overflows in `xx` is a layout bug to fix before translation, not a reason to shorten a text below its meaning.

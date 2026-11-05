# Search by meaning (ADR 0057)

Decision record: ADR 0057. Code: `crates/wolsem` (the engine, BSD-2-Clause), `crates/shell/src/ui/settings/search.rs` and `search_rows.rs` (Settings), `crates/shell/src/search_sem.rs` and `ui/searchbar.rs` (the Search bar), `crates/controller/src/helpfind.rs` (`wol help <words>`).

Settings search, the Search bar and `wol help` find things by what a question means, not only by the words it shares with a title, and they do it on the computer: a small table of word pieces and a few multiplications, no network, no model runtime, no account. The same table and the same reader as the manual's search on the website (`site/src/lib/docs-ai/static-embed.js`); this page is the OS side.

## The table

**Concept.** A static embedding table (`static-mrl.wst`): one vector of 256 numbers per word piece of a multilingual vocabulary (`sentence-transformers/static-similarity-mrl-multilingual-v1`, Apache-2.0, 105 775 pieces, 4 bits a value, 15.2 MB). The meaning of a text is the mean of the vectors of its pieces, normalised. Two texts mean the same when the dot product of their vectors is near 1 (cosine). The first 64 or 128 numbers are a smaller embedding of their own (Matryoshka), so a vector can be cut.

**Why.** A language model would be hundreds of megabytes and a runtime; this is 15 MB, loads in 5 ms (4.5 ms measured on the build box) and answers a question in 0.01 ms (embedding) plus 0.12 ms (comparing it with 2020 items). One table covers every language WolOS offers, so a question in pt-BR meets English vectors without any pt-BR text in the index: "deixar o texto maior" finds Larger text although no row is translated.

**How.** The byte layout is written once, in `site/src/lib/docs-ai/README.md` (reader `static-embed.js`, builder `site/scripts/lib/search-table.mjs`); `crates/wolsem/src/table.rs` is a port of the reader and says the layout again in its header. Only the fold section, the token strings and the scales are read at start (about 1.7 MB resident); a vector row is read with `pread` when a token needs it, so there is no `mmap`, no `unsafe` (RR-068) and the 13.5 MB of vectors stay in the page cache. Text becomes pieces as the fold section says (lowercase, accents off, split at spaces and punctuation, one word per Chinese character, Hangul as jamo), then greedy longest match with `##` for a piece inside a word; a word with a part no piece covers is dropped; 100 code points a word and 256 pieces a text at most. A question typed by a person (`Table::embed_query`) also drops a word cut into many tiny pieces (`qwertyuiop`: seven pieces for ten letters), which would pull the meaning of the rest towards anything.

**Conformance.** `crates/wolsem/tests/conformance.rs` holds what the JavaScript reader made of 27 texts on the test table (pieces, and the first 16 values of the 256-number vector) and requires the Rust reader to agree to 2e-4. Regenerate `tests/data/conformance.json` with a node script that imports `static-embed.js` (`readTable`, `tokenize`, `embed`) and writes `{dims, cases: [{text, tokens, vec}]}`, whenever the format or the tokenizer changes.

## The index and the hybrid rank

**Concept.** An `Index` holds items (an id, a title, a text), their vectors as int8 with one scale each, and a word index. A search runs both halves and fuses them by reciprocal rank (RRF, `sum 1/(60 + rank)`): the cosine rank, and a BM25 word score (title words count three times, the word being typed also matches as a prefix, words that most items hold are ignored, an item must hold half of the other words, `Wi-Fi` is also `wifi`).

**Why.** Words alone miss paraphrases and other languages (keyword-only found 18 of 41 manual questions, 5 of 18 Settings questions); meaning alone ranks an exact name below a near neighbour and answers nonsense. Fused, an exact word hit and a close meaning both rise.

**How.** An item's vector is the mean of its title (counted six times) and its text. The query vector is compared with every item in an integer dot product (`Index::semantic`, 0.1 ms for 2020 items of 256 numbers). A meaning match must have at least a cosine of 0.26 (`Opts::default`; Settings and the Search bar use `Opts::strict`, 0.30) and stand 3.0 (strict: 3.5) standard deviations above the mean cosine of the whole index, and be within 0.2 of the best: non-English questions score 0.3 to 0.5 where English ones score 0.6 to 0.9, so a fixed cosine cannot tell a question about nothing from a foreign-language question, and the spread can. "Did you mean": a query word that is in no title is put right by the nearest title word (Damerau-Levenshtein 1, or 2 for words of 7 letters or more), else the nearest title by meaning is offered.

## The query map between languages

`Procrustes` (`crates/wolsem/src/map.rs`) is the orthogonal matrix that carries a vector of language L to the English one, trained on aligned pairs: the rows of `locale/shell/<lang>.tsv` against `en.tsv`, 380 pairs a language, 0.3 s to train at 256 numbers, 256 KB as `map.<lang>.wprc` (`WPRC`, u16 version, u16 dim, `dim * dim` little-endian `f32`). With the multilingual table above it changed nothing: recall of the pt-BR, ja, fr, de, ru and it questions was the same with and without it, so the image ships none. `Installed::map_for(lang)` loads `/usr/share/wolos/manual/map.<lang>.wprc` when a table that keeps languages apart makes one worth shipping; `hits` passes `None` until a language is wired in (L10N-2's `i18n` is not in this tree yet).

## Where it is used

- **Settings** (`ui/settings/search.rs`). Pages, subpages and every row of `ROWS` are items, found by the row title, its page, the page's search words and, for rows that have one, the sentence the pane shows under it (`DESCRIBED` in `search_rows.rs`, read from `copy_settings.rs`). Word-prefix hits stay first and unchanged; with word hits at most three rows by meaning follow them, each of cosine 0.5 or more; with none, the meaning list is all there is. The index is built off the interface thread when Settings opens; until it is ready the search is by words only. A table that is not installed changes nothing.
- **The Search bar** (`search_sem.rs`, `ui/searchbar.rs`). Apps, system actions, Settings pages and every Settings row are items; the manual's sections are the second index. Word hits first as before, then rows by meaning fused by RRF (two after word hits, each of cosine 0.5 or more; up to the free rows when there are none), at most two manual sections (cosine 0.28 or more), files after them as before. A manual section opens `https://wolos.org/manual/<page>#<anchor>` in the browser (the image has no local copy of the pages). When nothing matches, a "Did you mean" row offers the corrected word's hit or the nearest title; the web search row stays last.
- **`wol help <words>`** (`helpfind.rs`). Words that are not the start of a command's path are a question: the five commands whose help line means it, and up to two manual sections with their URLs; with no table the same words are matched as words only. Nothing found exits 66 (`not-found`). An unknown first word (not a flag, not an unknown subcommand) with no near spelling adds "did you mean 'wol power restart'?" when it means that command (the word "reboot", typed after `wol`).

## What a result opens

Every result lands on the exact thing it names, by an id and not by the title it shows (QA-SEARCH-1: "privacy settings" opened Settings on its first pane).

| Result | Opens | Lands on |
|---|---|---|
| App | its launcher | the app |
| System action | the command the menu bar runs | the action (Restart, Shut Down and Log Out ask first) |
| Settings page | `wol-settings <page-id>` (`privacy`, `wifi`, `date-time`) | the page |
| Settings row (and a Control Center tile or module, a kind of app permission) | `wol-settings <page-id>#<row-id>` (`privacy#screen-recording`) | the page, scrolled to the row and lit until the person leaves the page |
| File or folder | `gio open <path>` | the file |
| Row an app provided | wold, by the app's key | the app's own row |
| Manual section | the browser at `wolos.org/manual/<page>#<anchor>` | the section |

Page ids are written once in `ui/settings/ids.rs` (`PAGE_IDS`); a row id is the slug of its English title. `wol-settings` also takes an older title or alias and a title in any shipped language, and resolves it to the same page and row. A query word that only names the place ("settings", "preferences", "ajustes") is dropped when another word remains, so "privacy settings" finds the Privacy pane. Rows rank one step below an app, an action or a page that matches the same way. A row a pane draws only in some state (a permission needs an app that asked, a slider hidden by its own switch) opens its page and lights nothing. A link in the shell names a page id, never a title (RU-051, RC-47).

## What is shipped

`/usr/share/wolos/manual/static-mrl.wst` (+ `.sha256`) and `sections.json` (`{"v":1,"model":"static-mrl","sections":[{"id":"/manual/page#anchor","title":"Page: Section","text":"...","vec":"<optional base64: dim int8 and an f32 scale>"}]}`, or a bare array), both produced by the site build (`site/scripts/lib/search-table.mjs --dim 256 --bits 4 --scripts all`, and the sections export); without `vec` the OS computes the vectors itself (90 ms for 505 sections, once per process, off the interface thread). `wolsem::installed()` loads them once. The package that installs them is `wolos-system` (INT-2: the `package()` of its PKGBUILD installs the two files with `install -Dm644 ... "$pkgdir/usr/share/wolos/manual/"` and `wolos-pkgbuilds/docs` lists the paths; the PKGBUILD is not edited here).

## Measured

On the build box (32 cores), release. The test table (`crates/wolsem/tests/data/mini.wst`, 0.95 MB) is the shipped table cut to the tokens that the manual, the Settings rows, the 7-language copy tables and the questions use (`examples/prune.rs`); a question's pieces are the same as in the full table.

| What | Result |
|---|---|
| Open the 15.2 MB table | 4.5 ms |
| Embed a question | 2.4 to 6.6 us |
| Search 2020 items (hybrid) | median 0.12 ms, worst 0.2 ms |
| Build the index of 505 manual sections | about 90 ms, once |
| Manual, page in the first 3, 41 questions (21 en, 20 pt-BR/ja/fr/de/ru/it) | hybrid 33, keyword only 18 |
| Settings, row or page in the first 3, 18 questions | hybrid 16, keyword only 5 |
| Questions about nothing (4) | no result |

The question set (`tests/data/queries.json`) was written for this and its thresholds were set on it, so the figures are optimistic for questions nobody has asked yet. The misses are mostly short generic wishes ("install an app" meets four pages that mention installing) and French, Italian and Russian questions whose pieces are shared with other words.

## Decisions

- One reader, one table, one tokenizer for the site and the OS (`static-mrl.wst`); the OS port is checked against the JavaScript reader by test.
- Meaning search is a second source after word hits, never instead of them: an exact name still opens its page first.
- No `mmap`: `pread` of the rows, so the crate has no `unsafe`. RR-068 stays as it is.
- Strict thresholds for the short lists (Settings, the bar's own items), looser for the manual; a non-English question is told from nonsense by how far it stands above the other items.
- Private by construction: the table and the sections are read from the image, nothing is sent, nothing is stored.
- No Procrustes map is shipped (it changed nothing); the loader is in place for a table that needs one.

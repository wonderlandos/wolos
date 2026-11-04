# 0057 - Search by meaning in Settings, the Search bar and `wol help`

Status: accepted (CTO, 2026-11-04). Lead order 2026-11-04: the manual search uses embeddings with recommendations and gets better, and the same search serves the OS tools; translations reuse the source vectors by aligned position.

## Context

Settings search, the Search bar and `wol help` matched words only: a question in other words, or in another language, found nothing (keyword-only found 18 of 41 manual questions and 5 of 18 Settings questions on the test set). The website's manual search gained a static embedding table (`static-mrl.wst`, `site/src/lib/docs-ai/README.md`); the OS can read the same table in pure Rust with no model runtime.

## Decision

- One reader, one table, one tokenizer for the site and the OS (`static-mrl.wst`, 256 dims, 4 bits a value, about 15 MB); the OS port (`crates/wolsem`) is checked against the JavaScript reader by a conformance test.
- Meaning search is a second source after word hits, never instead of them: an exact name still opens its page first.
- No `mmap`: the vector rows are read with `pread`, so the crate has no `unsafe` and RR-068 stays as it is.
- Strict thresholds for the short lists (Settings, the bar's own items), looser for the manual; a non-English question is told from nonsense by how far it stands above the other items (cosine floor plus a spread test), not by cosine alone.
- Private by construction: the table and the manual sections are read from the image (`/usr/share/wolos/manual/`), nothing is sent, nothing is stored.
- Translations reuse the English vectors by aligned item (settings row id, manual anchor). No Procrustes query map ships: with the multilingual table it changed recall by nothing; the loader stays for a table that needs one.
- The table and `sections.json` are fetched by the wolos-system recipe from cdn.wolos.org with a pinned sha256, not committed to the wolos tree.

## Consequences

- `crates/wolsem` (BSD-2-Clause) joins the workspace; `controller -> wolsem` and `shell -> wolsem` are allowed edges.
- Settings and the Search bar add meaning rows after word hits only when the table is installed; goldens are unchanged without it.
- `wol help <words>` answers questions and exits 66 (`not-found`) when nothing matches.
- Later: per-sentence vectors per manual section (the site already indexes sentences) to lift manual recall; wiring the UI language when a per-language table ships.

Details and measurements: `docs/modules/search.md`.

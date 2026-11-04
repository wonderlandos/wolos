# 0050 - Release verification: every claim about a release, proven offline from the file and the public log
Status: proposed (draft by VERIFY-design 2026-11-04 for the lead order of 03:42Z, "make the system of verification so that we can verify EVERYTHING of the package and the release", and the lead addition of 03:43Z, the exhaustive scan of readable code; lead decisions V1-V9 at the end). Builds on 0046 (certificates, file certificates, revocation, signature lines, the `.rkdb` database, witnesses), 0047 (the hub, pipelines, the public log), 0049 (what the hub requires at Origin and at Trusted), 0022 (judged by laws, never "safe").

Labels: [mem] = from project memory, the code as it is on `wave3/trust-core` 0b00c1e and the hub repositories; [fresh] = reasoned for this record. Research behind the threat model: `.claude/v012/hub/supply-chain-research.md` (names incidents; this record does not).

## Context
- [mem] Today a release already carries several proofs, each checked by different code in different places: the author's hybrid signature over the `SHA256` member with an origin inside it (`woltrust::verify`, `woltrust::origin`); the file certificate the Index signs after the pipeline checked the file (`woltrust::filecert`, label `wolos/release/1`); the member hashes in `SHA256`; the chain to the pinned roots with both halves, revocation in `revoked.json` and the `.rkdb` database with its serial floor and checkpoints (`woltrust::rkdb`); witness signatures on published files (`woltrust::witness`); and a canonical signature line per signed body (`woltrust::line`). The pipeline (`hub-index`, `check-release.mjs`) pins the asset's size and sha256, runs hubcheck, and `verify-owners.mjs` checks the repository owner by numeric id against GitHub before the index is signed. hubcheck produces the rule findings and the listing facts (`wasm/src/{files,listing,trust,report}.rs`).
- [mem] What a person can check today: `wol trust verify FILE --file-cert F` (the site's version page already prints that command) covers the signature, the chain, the revocation state and the file certificate. Nothing tells them, offline, which workflow built the file, on which commit, whether a second build matched, what is inside, which scan ran with which lists, or that the tag really existed on the default branch: those facts live in the pipeline's run logs and in the Worker's database, not in signed bytes.
- [fresh] The lead's order: one command and one hub page where every claim about a release is a line, green or red, each provable offline from the file plus the public log; and, for readable code, an exhaustive automatic check that is itself a signed claim with exact matches only.

## Decision in one paragraph
Every fact the hub asserts about a release becomes a **signed statement**: a small body of exact bytes, a label naming its kind, a hybrid signature by a key that reaches the pinned roots, and a canonical signature line derived from the body. The author signs the release and its origin (as today); the publisher's build signs its provenance with a short-lived workflow certificate (or the author does, at Origin); the Index signs the file certificate, the contents statement, the scan statement, the reproducible-build statement and the match lists. All statements name the same subject (the file's SHA-256 and its file id, the SHA-512 of the `SHA256` member), so none can be moved to another file. They are published in the public log and on the CDN, bundled per version as one `.wolproof` file, and checked by one pure function, `woltrust::checklist`, that prints a fixed-order list of eleven lines, each `PASS`, `FAIL` or `NOT PRESENT`, with the signature line beside it. `wol trust verify`, the pipeline, the hub Worker and the browser all run that one function, so the page and the command cannot disagree. A statement proves that a named key said a thing about these bytes; the checklist says "checked", never "safe" (ADR 0022).

## The claims
Each claim says what is asserted, which bytes prove it, who signs them, and where they sit in the log. "Subject" everywhere means `{"file_sha256": <64 hex>, "file_id": <128 hex, SHA-512 of the SHA256 member>, "id", "version", "kind"}`. "Index" is the Index intermediate (usage `sign-index`); "Author" is the leaf certificate in the file's `CHAIN` (person, organisation or WolOS signer).

| # | Claim | Proof (what is checked, offline) | Signed by | In the log |
|---|---|---|---|---|
| 1 | **Author signature**: this maker signed these member hashes, this origin and these terms | `SHA256.wsig` verifies with both halves of the Author's keys over `wolos/package/1 \|\| 0x00 \|\| SHA-512(SHA256) \|\| SHA-512(origin)`; the id lies in the Author's namespaces; the terms version in the certificate is the one the signature line names ([mem] `woltrust::verify`) | Author | the file itself; `certs/<id>.wolcert` |
| 2 | **File certificate**: the hub checked exactly this file on this date with this verdict | `wolos/release/1` body with `file_sha256`, `manifest_sha256`, id, version, kind, origin, Author id and class, verdict, findings count, `checked_at`, `rkdb_serial`, `terms` ([mem] `woltrust::filecert`), plus the new fields of this record (`origin_checks`, `statements`) | Index | `statements/<id>/<version>/file-cert.wsig` |
| 3 | **Member integrity**: every payload member is hashed in `SHA256`, no member is missing or extra, the order is canonical | The verifier re-reads the archive: every member except `SHA256`, `SHA256.sig`, `SHA256.wsig`, `CHAIN` has one line in `SHA256` whose hash matches; no line without a member; lines sorted by path bytes; no duplicate path; no path outside the format's rules ([mem] today's `files/*` rules in hubcheck; the system's `wol_core::package::verify`) | Author (through claim 1) | the file itself |
| 4 | **Origin**: the repository belongs to the certificate's login or organisation; the tag existed; its commit is reachable from the default branch; the release asset's hash is this file's hash | Offline part: the origin in `SHA256.wsig` parses and `Origin::belongs_to` holds ([mem]). Pipeline-checked part, recorded in claim 2 as `origin_checks`: `{"repo_id", "owner_id", "tag_commit", "on_default_branch": true, "release_created_at", "asset_sha256", "asset_size", "immutable": bool, "checked_at"}`, filled by `verify-owners.mjs` from GitHub's own answers (repository by numeric id, the compare API for reachability, the release's asset list) | Index (the facts), Author (the origin) | claim 2's file; `verdicts/<id>/<version>.json` keeps the raw answers |
| 5 | **Build provenance**: which workflow file, on which commit and ref, in which run, on which runner, produced this asset | `wolos/provenance/1` body (below); its signer is a **workflow certificate** whose subject names the same `repo_id`, workflow path and ref (option C), or the Author (option A fallback, shown as "declared by the author", not "witnessed"); claim 2 records `provenance_match: {"workflow", "ref", "run_id", "matched": true}` when the pipeline confirmed the run with GitHub | workflow certificate or Author; match by Index | `statements/<id>/<version>/provenance.wstmt`; `certs/workflows/<id>.wolcert` |
| 6 | **Reproducible build**: a second, independent build from the same commit produced the same bytes | `wolos/reproducible/1` body: the rebuild's inputs (commit, recipe file hash, toolchain digests, runner image digest), `result: identical\|differs`, the rebuilt file's sha256, and for `differs` the hash of a published diff list. Bonus at Origin, required at Trusted (ADR 0049 O9, T4, P6) | Index (the hub's `rebuild` job); later also other rebuilders with a witness-class certificate (V7) | `statements/<id>/<version>/reproducible.wstmt`, `reproducible.diff` |
| 7 | **Contents**: what is inside, exactly | `wolos/contents/1` body: counts and the SHA-512 of three side files it names: `contents.members` (one line per member, `sha256 size path`, sorted by path bytes: the verifier recomputes it from the file and the two must be equal), `contents.deps` (every dependency found in lockfiles or manifests inside: ecosystem, name, version, hash when the lockfile has one, and every embedded copy of a known library recognised by file hash), `contents.findings` (every hubcheck finding, rule, sentence, evidence); plus `binaries` (path, sha256, format, architecture), `scripts` (path, language, lines), `runtimes` bundled, `permissions` and `capabilities` declared, hubcheck version | Index (hubcheck produced it in `check`) | `statements/<id>/<version>/contents.*` |
| 8 | **Code scan** (lead addition 03:43Z): for readable code, the exhaustive exact-match check ran with named lists and found nothing, or found exactly these things at these lines | `wolos/scan/1` body: the lists used (name, serial, SHA-512 each), the scope (files, bytes, languages), `result: clean\|findings`, counts per class, and the SHA-512 of `scan.findings` (class, rule, list entry id, file, line, column, a snippet of at most 200 characters, the sentence). Binaries are not in scope here: they are named in claim 7, built in claim 5, rebuilt in claim 6 | Index | `statements/<id>/<version>/scan.*`; `lists/*.wlist` |
| 9 | **Certificate chain and revocation**: every signer reaches both roots, both halves verify, nothing in any path is withdrawn, the database is at least as new as every floor named | Author chain, workflow certificate chain, Index chain and the witness: each walks to a pinned root with the second root's co-signature on the intermediate ([mem] lead 03:17Z); `revoked.rkdb` verifies, its serial is at least the highest `rkdb_serial` named in claims 1, 2 and 5 and the machine's floor; the two checkpoints (CDN and log) agree ([mem] `rkdb::checkpoints`); no `withdrawn-version` names claim 2's id | roots, Index, witnesses | `certs/`, `signed/revoked.rkdb`, `signed/checkpoints/` |
| 10 | **Hub statement**: this version is listed, not withdrawn, not yanked, in a signed index that is fresh | the `index.json` entry for id and version names this `sha256`, this file certificate id and the statement ids; `index.json.wsig` verifies with the Index; `serial` is not below the one last seen; `expires` is ahead of the clock or the stale line is printed; the entry has no `withdrawn` or `yanked` | Index | `signed/index.json` |
| 11 | **Signature lines**: every line shown equals the line derived from the signed bytes | `woltrust::line` recomputes the line of each signed body in claims 1, 2, 5, 6, 7, 8 and of each certificate and revocation touched in 9; the published `lines.txt` must be equal, byte for byte. A line is never read as truth; it is only ever derived ([mem] lead 02:56Z) | nobody (derived) | `statements/<id>/<version>/lines.txt` |

Required at **Origin**: 1, 2, 3, 4, 7, 9, 10, 11, and 8 when the file holds readable code. **Bonus** at Origin: 5, 6. Required at **Trusted**: all eleven (ADR 0049 T3, T4). A claim that is required and `NOT PRESENT` makes the whole result incomplete (exit 3), never `PASS`.

## The statement format (interface I9)
One envelope for every new kind, the shape of the file certificate with the kind made explicit:

```json
{"format":1,"kind":"wolos/provenance/1","body":"<base64url of the exact signed bytes>","cert":"<signer id, 128 hex>","sig":{"ed25519":"<b64url>","second":"<b64url>"}}
```

- `kind` is the label the signature uses (`label || 0x00 || SHA-512(body)`), so a statement of one kind can never be read as another. Kinds of this record: `wolos/provenance/1`, `wolos/contents/1`, `wolos/reproducible/1`, `wolos/scan/1`, `wolos/list/1`. The file certificate keeps its own shape and label (`wolos/release/1`); a statement file carries the extension `.wstmt`, a list `.wlist`.
- `body` is UTF-8 JSON, at most 8 KiB, never re-serialised, unknown fields refused, no floats; every body starts with `"format":1` and `"subject"` (above), except a list, whose subject is itself. Anything large (members, dependencies, findings, diffs) goes to a **side file** named in the body by SHA-512 and byte count; a side file is at most 16 MiB and is plain text, one record per line, sorted, so two producers write the same bytes.
- The **id** of a statement is the SHA-512 of its body bytes (128 hex), the IDS rule; a `withdrawn-version` entry may name a statement id as well as a file certificate id.
- `issued` (Unix seconds) is in every body; a statement is never edited: a new one for the same subject (a re-scan with newer lists) is a new file with a higher `issued`, and the older one moves to `history/`.
- **Signature lines** (one per kind, same rules as `woltrust::line`: angle brackets, full ids, UTC dates, `none` for absent):
  - provenance `<WHO> <github:OWNER/REPO@TAG COMMIT7> <workflow: PATH@REF> <run: RUN_ID.ATTEMPT> <runner: LABEL> <sha512:FILE_ID> <DATE> <witnessed|declared>` where WHO is the workflow certificate's name (`ci/OWNER/REPO`) or the Author's login;
  - contents `<WolOS Index> <wolos.org/hub/APP/VERSION> <members: N> <deps: N> <binaries: N> <scripts: N> <findings: N> <sha512:FILE_ID> <DATE>`;
  - reproducible `<WolOS Index> <wolos.org/hub/APP/VERSION> <rebuilt: github:OWNER/REPO@COMMIT7> <identical|differs:N> <sha512:FILE_ID> <DATE>`;
  - scan `<WolOS Index> <wolos.org/hub/APP/VERSION> <lists: malware/57,vulnerable/212,capabilities/9,secrets/14> <clean|findings:N> <sha512:FILE_ID> <DATE>`;
  - list `<WolOS Index> <wolos.org/lists/NAME> <serial: N> <entries: N> <sha512:LIST_ID> <DATE>`.

### `wolos/provenance/1` body
```json
{"format":1,"subject":{...},"repo":"https://github.com/OWNER/NAME","repo_id":123,"tag":"v1.2","commit":"<40 hex>",
 "workflow":".github/workflows/release.yml","workflow_sha":"<40 hex>","ref":"refs/heads/main","run_id":9876,"run_attempt":1,
 "runner":{"kind":"hosted|self-hosted","label":"ubuntu-24.04","image":"<digest or none>"},
 "inputs":[{"name":"lockfile","sha256":"..."}],"outputs":[{"name":"notes-1.2.wol","sha256":"...","size":1234}],
 "built":1793700000,"issued":1793700100,"rkdb_serial":412}
```
`outputs` must contain the subject's file with its sha256; `ref` is the ref the workflow ran on (a tag ref is allowed only when the listing registered tag-triggered releases). The hub compares `repo_id`, `workflow`, `ref` with the listing's registration (ADR 0049 T3) and, with GitHub, that run `run_id` exists for that workflow, has `head_sha` = `commit`, and ended before the asset was uploaded; the result is `provenance_match` in claim 2. [fresh, non-normative] The fields map one to one onto the common build-attestation format used by the big registries: `subject` to its subject list, `repo`+`commit`+`workflow`+`ref` to its build definition's external parameters and resolved source, `runner` and `run_id` to its run details and builder id, `inputs` to its resolved dependencies, `built` to its build start time; a converter in the pipeline can read such an attestation from a release and produce this body, so a publisher who already emits one need not emit two.

### The workflow certificate (option C)
A new certificate kind `workflow`: subject `{"repo_id", "repo", "workflow", "ref", "run_id"}`, usage `sign-provenance` only, keys made inside the publisher's job and never written to disk, validity 2 hours, issued by a new intermediate **Workflows CA** (usage `sign-certs`, namespaces `*`, issues `workflow` only), on an identity token the job requests from its platform (audience `hub.wolos.org`) whose claims name exactly that repository id, workflow path, ref and run. Its signature line reads `<ci/OWNER/REPO> <workflow: PATH@REF> <issued-by: WolOS Workflows> <wolos.org/trust/ID> valid-until <DATE> <ed25519+ml-dsa-65>`. A workflow certificate may sign nothing but `wolos/provenance/1`; `woltrust::verify` refuses it as the signer of a package, an index or a list, the way it refuses a witness. It is published in `certs/workflows/` so the chain is in the log after the key is gone.

### `wolos/reproducible/1` body
```json
{"format":1,"subject":{...},"source":{"repo":"...","commit":"<40 hex>","recipe":"wol-build.toml","recipe_sha256":"..."},
 "toolchain":[{"name":"rust","version":"1.99.1","sha256":"..."}],"image":"<runner image digest>","runner":"hosted",
 "started":1793701000,"finished":1793701900,"result":"identical","rebuilt_sha256":"...","diff":{"sha512":"...","bytes":0,"members":0},
 "rebuilder":"wolos-hub","issued":1793701901}
```
The recipe is a file the publisher declares in the listing (`build.recipe`); the rebuild job runs it in a clean runner with no network after fetching the pinned inputs, sets a fixed build time and path, and compares the whole file. `differs` lists each differing member in `reproducible.diff` (`path expected_sha256 got_sha256`); the body carries `members` = how many.

### `wolos/contents/1` body
```json
{"format":1,"subject":{...},"members":{"count":41,"bytes":18234111,"sha512":"<of contents.members>"},
 "deps":{"count":3,"sha512":"<of contents.deps>"},"findings":{"count":0,"verdict":"pass","sha512":"<of contents.findings>"},
 "binaries":[{"path":"bin/notes","sha256":"...","format":"elf","arch":"x86_64","size":2201332}],
 "scripts":[{"path":"share/notes/main.lua","language":"lua","lines":412}],"runtimes":[],
 "permissions":["files","network"],"capabilities":["notifications"],"hubcheck":"<version>","issued":1793700200}
```
`contents.deps` lines: `ecosystem name version sha256|none source` where source is the lockfile path or `embedded:<path>` for a copy recognised by hash. Formats and languages are closed lists in hubcheck (`elf`, `pe`, `macho`, `wasm`; `lua`, `crystal`, `javascript`, `shell`, `other`).

### `wolos/scan/1` body and the lists (interface I11)
```json
{"format":1,"subject":{...},"lists":[{"name":"malware","serial":57,"sha512":"..."},{"name":"vulnerable","serial":212,"sha512":"..."},
 {"name":"capabilities","serial":9,"sha512":"..."},{"name":"secrets","serial":14,"sha512":"..."},{"name":"names","serial":3,"sha512":"..."}],
 "scope":{"files":12,"bytes":88211,"languages":["lua","javascript"],"skipped":[{"path":"bin/notes","why":"binary"}]},
 "result":"clean","counts":{"malware":0,"vulnerable":0,"capability":0,"secret":0,"name":0,"obfuscation":0},
 "findings":{"count":0,"sha512":"<of scan.findings>"},"hubcheck":"<version>","issued":1793700300}
```
`scan.findings` lines: `class rule entry_id path line column sha256(snippet) | snippet | sentence`, sorted by path then line. **The zero-false-positive rule**: a finding exists only when an entry of a list matches exactly; there is no score, no threshold, no "looks like". What "exactly" means per class:

| List | Entry | Matches when |
|---|---|---|
| `malware` | `{"id","family","class":"hash\|string\|indicator","value","sentence","source","added"}` | `hash`: SHA-256 of a whole member equals `value`; `string`: the member contains the exact byte string `value` (hex, at least 24 bytes); `indicator`: the exact host name, address, wallet id or key id appears as a whole token |
| `vulnerable` | `{"id","ecosystem","name","versions":[exact list],"fixed_in","advisory","sentence"}` and `{"id","embedded":{"name","version","sha256"}}` | a line of `contents.deps` has that ecosystem, name and a version in the set; or a member's SHA-256 equals an embedded copy's hash |
| `capabilities` | `{"id","language","token","capability","sentence"}` | the exact token appears as a code token (not inside a string or comment, decided by the language's tokenizer) and the capability is not among the declared permissions; also `dynamic-eval` tokens combined with a fetch token in the same file, each named by line |
| `secrets` | `{"id","name","prefix","length","checksum":"none\|<named algorithm>","sentence"}` | a token with that exact prefix and length, and the checksum holds when one is defined |
| `names` | `{"id","reserved":[...],"confusables":[...]}` | the id or display name equals a reserved word, or equals another listing's after the mapping of confusables (ADR 0049 O6) |
| `obfuscation` (part of `capabilities`) | `{"id","language","shape":"base64-literal>=N\|hex-literal>=N\|charcode-chain>=N","sentence"}` | a literal of that exact shape and at least that length |

The list file: `{"format":1,"name":"malware","serial":57,"issued":T,"entries":[...]}` + `.wsig` (label `wolos/list/1`, Index), at most 16 MiB and 200,000 entries, entries sorted by id, ids never reused, `serial` grows by one per publication, a signature line each. **Update path**: entries arrive as a pull request to `lists/` in the log repository; CI checks the schema, the sort, the sources, that every entry has a sentence, and runs the new lists over the **clean corpus** (the SHA-256 of every listed release whose last scan was clean is in `lists/corpus.txt`; the pipeline re-fetches the newest 500 and the full set weekly): a single new match on the corpus fails the check and a person must decide whether the corpus or the entry is wrong. A merged change is signed and published by the next `sign` run; a `rescan` job then re-scans every listed readable release within 24 hours with the new serials, writes new scan statements, and a new `malware` or `vulnerable` match on a listed version goes to the moderation queue as a trusted report (ADR 0049 R1), never to an automatic withdrawal. **Who maintains**: a `scanners` team of the organisation (moderators and trusted reporters; V5), with a script that drafts entries from public advisory feeds into a pull request and never merges. The hub's own release statements name list serials, so anyone can replay a scan with the exact lists it used.

**Easy publishing of plugins and extensions** (readable code only, binaries refused in these kinds: today's `content/*` finding becomes `block` for plugins and extensions): `wol publish FILE` packs if needed, runs the same exhaustive scan locally against the lists in `/var/lib/wolos/trust/lists/`, prints the checklist, signs with the account's origin certificate, and creates the tag and the release with the person's own GitHub login (device sign-in kept by the system, never by the hub); the hub registers the listing itself on the first release webhook when the id is in the owner's namespace, so there is no form; a template repository with the release workflow that emits provenance is one click in the Maker. Because the scan is complete for these kinds, a clean plugin or extension by a listed author is offered as an update after 24 hours instead of 72 (a change to ADR 0049 O4; V6).

## The command (interface I10)
```
wol trust verify FILE [--proof B.wolproof | --log DIR | --file-cert F --provenance P --contents C --scan S --reproducible R]
                      [--level origin|trusted] [--now UNIX] [--json] [--lines]
```
- Inputs: the file, and the statements from one of three places: a **proof bundle** (`<id>-<version>.wolproof`: a plain tar, at most 4 MiB, with `file-cert.wsig`, the `.wstmt` files, their side files, `lines.txt`, `certs/` for every signer, `revoked.rkdb` and the two checkpoints, and the index entry's bytes with `index.json.wsig` and the index header), a **log checkout** (`--log DIR`, the public log at any commit: the command finds everything by id and version), or single files. With none given, the command looks next to FILE for `FILE.wolproof`. The trust store is the machine's (`/usr/share/wolos/trust`, `/var/lib/wolos/trust`); `--roots DIR` for a clean machine. Nothing is fetched; the command works with the network off.
- Output: one header line, eleven numbered lines in the fixed order above, each `PASS`, `FAIL` or `NOT PRESENT` followed by the signature line when the claim has one and by one sentence of detail; a result line. Widths are fixed, so the lines align in a terminal and in the page.

```
notes-1.2.wol  sha256 ab12…9f  id ana.notes  version 1.2  app  level origin
 1 author signature     PASS         <ana> <github:ana/notes@v1.2 3f9c2a1> <wolos.org/trust/8c…> <wolos.org/terms/trust/v1> <sha512:4e…> <2026-11-03> <origin>
 2 file certificate     PASS         <WolOS Index> <wolos.org/hub/ana.notes/1.2> <author: 8c…> <sha512:4e…> <checked 2026-11-04> <pass> <rkdb:412>
 3 members              PASS         41 members hashed, none missing, none extra, canonical order
 4 origin               PASS         github.com/ana/notes is ana's; tag v1.2 at 3f9c2a1 on the default branch; asset hash is the file hash (hub-checked 2026-11-04)
 5 build provenance     NOT PRESENT  bonus at origin
 6 reproducible build   NOT PRESENT  bonus at origin
 7 contents             PASS         <WolOS Index> <wolos.org/hub/ana.notes/1.2> <members: 41> <deps: 3> <binaries: 0> <scripts: 2> <findings: 0> <sha512:4e…> <2026-11-04>
 8 code scan            PASS         checked, nothing to report  <WolOS Index> <wolos.org/hub/ana.notes/1.2> <lists: malware/57,vulnerable/212,capabilities/9,secrets/14,names/3> <clean> <sha512:4e…> <2026-11-04>
 9 chain and revocation PASS         ana (origin) <- WolOS Authors <- root A | root B; both halves; nothing withdrawn; rkdb 412 >= floor 409; checkpoints agree
10 hub statement        PASS         listed in index 1832 of 2026-11-04, fresh until 2026-11-11; not withdrawn, not yanked
11 signature lines      PASS         4 lines derived from the signed bytes equal the published lines
checked: 9 passed, 0 failed, 2 not present (bonus)
```
- Exit codes: **0** every claim present passed and every claim required at `--level` (default: the level of the Author's certificate) is present; **1** at least one `FAIL`; **2** usage, or FILE unreadable; **3** no `FAIL`, but a required claim is `NOT PRESENT` (incomplete). `--json` prints `{"file":{...},"level","claims":[{"n","name","state":"pass|fail|not-present","line","detail","evidence"}],"result":{"passed","failed","not_present","exit"}}` (interface I10, the same bytes the pipeline commits as `checklist.json` and the page renders). Words: `PASS` means "checked out"; the help text says so and never says "safe".

## The hub version page
- The same eleven rows in the same order, from `checklist.json` the pipeline committed, each row green (`PASS`), red (`FAIL`) or grey (`NOT PRESENT`, with "bonus" or "required" in the row); the signature line in a monospace block with a copy control; the detail sentence under it; a `FAIL` row names the code (`file-cert-other-file`, `rkdb-floor`, ...) in words.
- A "Verify it yourself" box: three links (the file on its GitHub release, the proof bundle on the hub, the public log with the commit that holds this version), the exact command `wol trust verify notes-1.2.wol --proof ana.notes-1.2.wolproof`, and the last line the person should see. A second command for people without WolOS: `woltrust check FILE --proof B` (the pipeline's tool, built from source), and a browser check that runs the same wasm on the downloaded file and bundle without uploading either.
- Binding words: "checked", "found nothing to report", "found 2 things to look at", "not present"; never "safe", "secure", "verified safe", "malware-free". A fixed paragraph under the list: "These lines say who signed what and that the hub's checks ran. They do not say the app is harmless. WolOS reads what a release declares and what changed; it does not run it for you."
- For `NOT PRESENT` bonus rows at Origin, one line for the publisher: "Add build provenance" linking the template workflow.

## The public log (interface I13)
The log is the `hub-index` repository as today (ADR 0047, option C; the lead may rename it `hub-log`, V8), append-only, mirrored byte for byte on the CDN under `hub/`:

```
signed/index.json  index.json.sig  index.json.wsig          every publication (git history = every index ever signed)
signed/revoked.rkdb  revoked.rkdb.checkpoint  revoked.json(+.wsig, legacy)
signed/checkpoints/<serial>.json (+ <serial>.json.wsig by a witness)
signed/lists/<name>.wlist (+ .wsig)                          the lists the scan uses, one serial each
certs/roots/  certs/intermediates/  certs/<id>.wolcert  certs/workflows/<id>.wolcert
statements/<id>/<version>/file-cert.wsig  provenance.wstmt  contents.wstmt  contents.members  contents.deps  contents.findings
                          reproducible.wstmt  reproducible.diff  scan.wstmt  scan.findings  lines.txt  checklist.json
                          <id>-<version>.wolproof            the bundle of all of the above
                          history/<kind>.<issued>.wstmt       earlier statements for the same subject (re-scans)
lists/<name>.json  lists/corpus.txt  lists/CHANGES.md         the sources the sign job signs (pull requests land here)
verdicts/<id>/<version>.json                                 the raw pipeline output, as today
revocations.json  requests/                                  as today
```
Rules: a file under `statements/` or `certs/` is written once and never changed (CI refuses a commit that modifies one; `history/` receives superseded statements); every `.wstmt`, `.wlist` and `.rkdb` has its `lines.txt` line next to it; the index entry carries the ids of the statements present at signing (`statements: {file_cert, provenance, contents, scan, reproducible}`) and the bundle's sha256; the CDN copy is checked against the log by the `sign` job before upload, and anyone can check the CDN against the log afterwards.

## Three options for provenance [fresh]
| | A. Author-signed | B. Build-identity-signed (short-lived workflow certificate on a platform identity token) | C. Both, with the hub's match recorded (favourite) |
|---|---|---|---|
| What it proves | the holder of the author's key vouches for these build facts | the statement was made inside that run of that workflow on that ref, in that repository | the author vouches for the release (claim 1), the run vouches for the build (claim 5), the hub says it asked the platform and the run was real (claim 2) |
| New pieces | none (same key, new label) | a Workflows CA, a `wcert` job in the log repository, Worker endpoints that verify identity tokens from publisher repositories, the `wol ci provenance` step | A + B, and `provenance_match` in the file certificate |
| Latency for the publisher | none | about 30 to 90 s while the certificate is issued (a dispatch of the `wcert` workflow) | the same, optional at Origin |
| Where it fails | a laptop build or a leaked author key signs any "provenance"; it is a claim, not a witness | a hijacked run (poisoned cache, lifted in-flight token) produces valid provenance from the real workflow, as in every researched case; a dishonest platform could mint tokens | the same hijacked-run limit; more code and two chains to show |
| Offline check | yes | yes, the certificate is in the bundle and in `certs/workflows/` | yes |

**Recommended: C**, because the three signers are independent (the author's key is never on the platform or the hub, the workflow key never leaves the run, the Index never sees either), so one compromise cannot forge all three lines; at Origin the workflow certificate is optional and a missing one reads "declared by the author", so nobody is blocked; at Trusted it is required (ADR 0049 T3).

**Premortem of C** (it is 2028 and it failed; why):
1. **A hijacked run signed real-looking provenance.** True and said on the page: provenance proves where, not whether the build was honest. Guards: claim 4 (commit reachable from the default branch), the expectation match (one workflow, one ref), claim 6 (a second build), claim 7 (a diff of members a reviewer sees, ADR 0049 T2), and the revocation path. The ADR never lets a `PASS` on claim 5 be read as more than "witnessed by the run".
2. **The `wcert` path was slow or down and publishers skipped it.** Guard: optional at Origin with a plain `NOT PRESENT (bonus)`; the publisher's step times out at 3 minutes and falls back to author-signed provenance (`declared`); the weekly numbers show the share of releases with witnessed provenance.
3. **Someone minted workflow certificates for a repository they did not own.** Guard: the Worker checks the identity token's repository id against the listing's registration before dispatching, one certificate per run id, 20 a day per owner; the Workflows CA issues `workflow` kind only, 2 hours, usage `sign-provenance`; a certificate issued wrongly can sign nothing that installs.
4. **Two chains confused people.** Guard: the page shows one line per claim and names the signer in words ("signed by the build job of github.com/ana/notes", "signed by Ana"); the detail is a click away.
5. **The identity token's claims changed shape.** Guard: the Worker pins the claim names it reads and refuses anything else; a change is a `wcert` outage (premortem 2), not a wrong certificate.
6. **The platform's own attestation format was what publishers already had, and ours was one more thing.** Guard: the converter in the pipeline reads the common attestation from the release and produces `wolos/provenance/1` as `declared` (hub-converted, named so in the line); the publisher adds the workflow step only when they want `witnessed`.

## What is added, per component
- **woltrust** (TRUST-core-6): `statement.rs` (envelope I9, kinds, ids, `verify_statement(store, bytes, subject) -> Result<Checked<Body>, StatementError>`, codes `statement-malformed|signer-unknown|wrong-signer|bad-signature|other-file|wrong-kind`), the four bodies and the list body, `lists.rs` (`.wlist` parse, verify, sorted search by hash, string, token, prefix), `checklist.rs` (the eleven claims as one pure function over bytes: `check(store, file, Proof) -> Checklist`, text and JSON renderers, exit code), `line.rs` lines for the five kinds and the workflow certificate, `cert.rs` kind `workflow` and usage `sign-provenance`, `filecert.rs` fields `origin_checks`, `provenance_match`, `statements`, the `wolos/proof` tar reader, `woltrust statement sign|verify`, `woltrust list sign|verify`, `woltrust check FILE --proof|--log`, vectors (`vectors/statements/`, `vectors/checklist/`), fuzz targets for every new parser; `wol trust verify` in the system (`wol_core::trust`) over the same function. Nothing with I/O enters the crate.
- **hubcheck** (HUB-wasm-4): `contents.rs` (members list, lockfile and manifest readers for the closed set of ecosystems, embedded-copy hashes, binaries by magic, scripts by extension and shebang, runtimes), `scan.rs` (tokenizers for Lua, Crystal and JavaScript that know strings and comments; the five match classes; findings with path, line, column), `provenance.rs` (compare a provenance body with the facts JSON the pipeline fetched: repository id, workflow, ref, run head commit, asset name and hash), `api.rs` and `glue.rs` exports `contents_json`, `scan_json`, `provenance_match_json`, `checklist_json` (the browser check), the native commands `hubcheck contents|scan|provenance`; plugins and extensions: a binary member is `block`.
- **pipeline** (HUB-pipe-3): `check.yml` fetches `<asset>.provenance.wstmt` (and the common attestation, converted) from the release, runs `hubcheck contents|scan` and `provenance` against the facts `verify-owners.mjs` fetched, writes statement bodies and side files; a `rebuild` job (no secrets, no network after fetch, the recipe, the toolchain digests, 60 minutes at most; Origin bonus first, P6); a `wcert.yml` workflow and the Workflows CA environment; `sign.yml` signs file certificates with the new fields, every `.wstmt`, the lists, builds `lines.txt`, `checklist.json` (`woltrust check`) and the `.wolproof` bundle, publishes to the CDN and commits; `lists/` with its schema, the corpus check and the `rescan` job; the README's "verify a release" section.
- **Worker** (HUB-worker-4): `POST /api/ci/workflow-cert` and `POST /api/ci/provenance` (identity tokens from publisher repositories: issuer pinned, audience `hub.wolos.org`, repository id must be a registered listing's, one per run id, rate limits); dispatch of `wcert.yml`; `GET /api/pipeline/provenance/<id>/<version>`; `POST /api/pipeline/statements` (ids, `checklist.json`); listing registration gains `build: {workflow, ref, recipe, reproducible}`; tables `provenance`, `statements`, `workflow_certs`; `GET /api/listings/<id>/<version>/checklist` for the page's live state (withdrawn since the static build).
- **site** (HUB-site-5): the version page's checklist and lines (`render.mjs` version section), the "Verify it yourself" box with the bundle link, the browser check at `/verify` (wasm, files never leave the browser), `/lists` (each list, its serial, line and change log), the publisher page "one command to publish a plugin", the `wol trust verify` help text; the word check (`check-doctrine.mjs`) extended with "safe", "secure", "malware-free" as forbidden on these pages.

## Threat model, per claim
| # | Stops | Does not stop |
|---|---|---|
| 1 | a changed member, a swapped asset, a release by someone without the author's key (a taken-over GitHub account alone publishes nothing signed) | a stolen author key until revocation reaches the machine (claim 9); a dishonest author |
| 2 | a file the hub never checked passing as checked; a hub statement moved to another file; a withdrawn version shown as current | a hub that was wrong: the verdict is "no rule broke" (ADR 0022) |
| 3 | an extra member slipped into the archive after signing; a member omitted from the hash list; path tricks | anything inside a member the author did sign |
| 4 | a release from a fork, an orphan commit or a side branch; a repository that is not the author's; an asset replaced under the tag; a tag moved later | a malicious commit merged to the default branch; a mutable tag between the hub's check and a later move (the pinned hash still fails the file) |
| 5 | a laptop build passed off as the pipeline's (at Trusted); a workflow other than the registered one; a non-protected ref; a run that never existed | a poisoned build inside the real workflow (cache poisoning, lifted token): valid provenance, bad build; this is said on the page |
| 6 | a build that does not come from its source (a planted binary, a poisoned toolchain on the publisher's runner), when the rebuild matches or differs | an attack in the source itself; a non-reproducible build (it reads `differs`, which at Origin is a flag, not a proof of harm) |
| 7 | hidden members, undeclared dependencies, bundled runtimes, a 2 MB file that appeared between versions (a reviewer sees the diff); a contents statement for another file | what a binary does |
| 8 | exact known malware families and indicators, known vulnerable dependency versions and embedded copies, capabilities used but not declared, leaked tokens, look-alike names, named obfuscation shapes, each at file and line | anything not on a list: a new family, a second stage fetched at runtime, a payload in a binary, a time bomb; the page says "checked against lists of <date>", never "clean of malware" |
| 9 | a signer outside the roots, a single-root intermediate, a revoked key, a rolled-back or frozen revocation database (floor, checkpoints), a withdrawn version | both roots lost; a machine that never updates past 37 days shows the stale line, not a refusal |
| 10 | a version the hub never listed, a yanked version, a stale or replayed index | a hub that lists spam until moderators act (ADR 0047) |
| 11 | a page or a sheet showing a line that does not match the signed bytes (a compromised site or CDN rewriting text) | nothing on its own; it is the honesty check of the display |

Shared limits, said once on the page: the hub, the index key and the lists are one organisation; a compromise of the Index key can sign claims 2, 6, 7, 8 and 10 until the roots revoke it (the author's key and the workflow key are not in that blast radius); offline verification is only as current as the machine's revocation database.

## Consequences
- Five new signed kinds, one new certificate kind and one new intermediate; the file certificate grows three optional fields (older ones keep verifying). One pure function decides the checklist for the system, the pipeline, the Worker and the browser.
- The log roughly triples in files per version (about 10 small files and one bundle of at most 4 MiB); at 5,000 versions that is under 20 GB of history, within a public repository's limits, and the CDN copy is the same bytes.
- Publishers get three tiers of effort: nothing new (Origin, claims 1-4, 7-11 all automatic), one workflow step (claim 5 witnessed), a recipe file (claim 6). Plugins and extensions become the easiest kinds to publish and the most completely checked, because they hold readable code only.
- The scan is exact and therefore blind to the new; the cooldown, the sandbox and the revocation path remain the answer to that, as ADR 0049 says.

## Lead decisions
- V1: approve the eleven claims, the required and bonus sets per level, and the exit codes (0, 1, 2, 3).
- V2: provenance option C (recommended), or A alone for the first release of the system.
- V3: the Workflows CA as a fourth intermediate (software key in the `wcert` environment, like the others, L3), issuing 2-hour `workflow` certificates automatically.
- V4: the statement envelope (I9) and the `.wolproof` bundle as the public shape; the SHA-512 ids rule for statements.
- V5: who sits on the `scanners` team, and whether advisory feeds are imported by script (recommended: yes, into pull requests only).
- V6: the 24-hour update cooldown for clean plugins and extensions by listed authors (a change to ADR 0049 O4), and `block` for a binary inside a plugin or extension.
- V7: whether independent rebuilders (other than the hub's job) may publish reproducible statements under a witness-class certificate, now or later.
- V8: rename the log repository `hub-log`, or keep `hub-index`.
- V9: the page's words above and the fixed paragraph "They do not say the app is harmless".

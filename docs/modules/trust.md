# Trust: author certificates, signatures and withdrawn keys (ADR 0046)

**Concept.** A `.wol`, a `.wolplug` or an extension can say who made it. WolOS issues certificates to
people and organisations; a file signed with such a certificate shows as "From Alice Doe" (origin)
or "Trusted developer Alice Doe" (trusted). The
system checks all of it offline, and a certificate WolOS has withdrawn stops working.

**Why.** Until now "signed" meant "signed by WolOS". There was no way to say "made by this person"
and no way to take a leaked key back without shipping a new image. The signatures must also hold when
quantum computers arrive, because the roots live for 20 years.

**How.** Two parts.
- `crates/woltrust` (BSD-2-Clause) is pure: bytes and a clock value in, a verdict out. No files, no
  network, no clock. The system, the hub Worker and the browser all use it, so there is one
  implementation to get right.
- `wol_core::trust` (`crates/core/src/trust.rs`) reads the store from disk through `Host`, reads the
  clock, and judges a `.wol` with `TrustStore::verify_wol`.

## The signature (interface I1)

Every signature in the system covers `label || 0x00 || SHA-512(content)`. The label says what the
signature is for, so one made for a package cannot be replayed as an index or a certificate:

| Label | For |
|---|---|
| `wolos/cert/1` | a certificate, signed by its issuer |
| `wolos/package/1` | the `SHA256` member of a `.wol` |
| `wolos/plugin/1` | the `SHA256` member of a `.wolplug` |
| `wolos/extension/1` | the `SHA256` member of an extension |
| `wolos/index/1` | the hub index |
| `wolos/revocation/1` | a revocation list |

A signature has two halves and **both must verify**. Below the roots the pair is Ed25519 plus
ML-DSA-65; the two roots use Ed25519 plus SLH-DSA-SHA2-128s. There is no "either" mode: the format
number fixes the pair, and a new pair would be format 2. Neither half uses a context string.

### `.wolcert` (format 1)

```json
{"format":1,"body":"<base64url>","sig":{"ed25519":"<base64url>","second":"<base64url>"}}
```

`body` is the exact signed bytes (UTF-8 JSON, at most 4 KiB), never re-serialised, so there is no
canonical-JSON step. The file is at most 24 KiB. Base64url has no padding and must be canonical.
Body fields: `serial` (16 hex), `kind` (`root`, `intermediate`, `person`, `org`, `signer`), `subject`
(`github_id`, `login`, `name` for people and organisations; `label` for the rest), `namespaces`,
`usage` (`sign-certs`, `sign-index`, `sign-revocations`, `sign-package`, `sign-plugin`,
`sign-extension`), `keys` (`ed25519` plus `ml-dsa-65`, or for a root `slh-dsa-sha2-128s`), `issuer`
(the issuer's serial; a root names itself), `not_before`, `not_after` (Unix seconds, end exclusive)
and `level` (`wolos` or `verified`). Unknown fields are refused. Each kind has fixed rules:

- a root signs certificates and revocations only, as `wolos`;
- an intermediate signs certificates (and carries the namespaces it may issue), or the index and
  revocations (no namespaces);
- a person or organisation has a GitHub id, login and name, signs packages, plugins and extensions,
  is `verified`, and its namespaces are ordinary prefixes like `alice.`; **no author can hold
  `org.wolos.`**, not even through the `*` of the Authors CA;
- a signer is WolOS's own package key, issued by the Packages CA, inside `org.wolos.`.

### In a `.wol`

After `SHA256.sig` come two optional members, `CHAIN` and `SHA256.wsig` (see `packages.md`). `CHAIN` is
a JSON array of `.wolcert` objects, leaf first, **at most 4 certificates and 64 KiB, a root never
included**. `SHA256.wsig` is `{"cert":"<leaf serial>","ed25519":"..","second":".."}` over the `SHA256`
member with label `wolos/package/1`. Neither is listed in `SHA256`, so a renewed chain keeps the
package hash. A file without them stays valid: a legacy `SHA256.sig` signed by a WolOS key counts as
made by WolOS, no signature counts as unsigned.

## The check

`woltrust::verify(store, input)` runs in this order and stops at the first failure:

1. Parse `SHA256.wsig` and `CHAIN` within their caps.
2. The leaf (the first certificate of `CHAIN`, or a store certificate with the serial named in the
   signature) must carry the usage for this kind of file, and its namespaces must cover the app id
   in `Info.json`.
3. Both halves of the file's signature verify with the leaf's keys.
4. Each certificate verifies with its issuer, found next in the chain, in the store, or among the
   pinned roots. The issuer must be allowed to issue that kind (a root issues intermediates; an
   intermediate issues people, organisations and signers) and every namespace of the subject must
   lie inside the issuer's. At most 5 links.
5. The top of the path is a **pinned root, by exact key bytes**.
6. Revocation (below). Then the clock against every certificate's validity. Then the freshness of
   the newest revocation list.

The result is one of seven trust values and a code (the code is the stable string the vectors
check):

| Trust | Meaning | Codes |
|---|---|---|
| `wolos` | made by WolOS (Packages CA chain) | `ok`, `list-stale`, `list-too-old` |
| `trusted` | a developer or organisation a WolOS Hub administrator approved (`class: trusted`) | `ok`, `list-stale`, `list-too-old` |
| `origin` | who published it is proven (GitHub sign-in and the CLA, `class: origin`), or a trusted certificate superseded | `ok`, `list-stale`, `list-too-old`, `trust-withdrawn` |
| `legacy` | signed only with WolOS's previous flat key (raw Ed25519 `SHA256.sig`, no post-quantum half) | `legacy-key` |
| `community` | nobody vouches; ask "Do you really want to open this?" | `unsigned`, `expired`, `not-yet-valid` |
| `withdrawn` | a certificate in the path, its origin certificate, or the legacy key was withdrawn; refused, no override | `revoked-key-compromise`, `revoked-abuse`, `revoked-superseded`, `revoked-ceased` |
| `invalid` | a signature is present and wrong, or only the previous key on a machine that holds roots | `bad-signature`, `bad-issuer`, `wrong-usage`, `wrong-namespace`, `signer-unknown`, `issuer-unknown`, `signature-malformed`, `chain-malformed`, `chain-too-long`, `chain-too-large`, `origin-mismatch`, `legacy-refused` |

### Two certificates per author (lead decision 2026-11-04)

A person or organisation body carries `class`: `origin` (issued automatically at GitHub sign-in
with the CLA signed; proves the publisher and the namespace) or `trusted` (issued after an
administrator approved them). A body without `class` is origin: trust must be written in signed
bytes. A trusted body must carry `origin_cert`, the serial of the same subject's origin
certificate, and uses its own keys. The I7 report's `cert` gains `class`.

Revocation, final rules: `key-compromise` and `abuse` on any certificate void everything its key
signed; withdrawing the origin certificate also withdraws what its trusted certificate signed;
`superseded` on a trusted certificate only downgrades its files to `origin` (`trust-withdrawn`);
`ceased` keeps files the index saw before it. `key-compromise` or `abuse` on an origin certificate
is the supply-chain emergency path: it kills both certificates of that author at once.

### The previous key (audit H1)

A `.wol` or `.wolplug` with only the raw Ed25519 `SHA256.sig` by a key in `/usr/share/wolos/keys/`
is `legacy` ("Signed with the previous key"), never `wolos`. Each such key can be withdrawn in
`revoked.json` by its key id (the first 8 bytes of SHA-256 of the public key, 16 hex; `wol trust
status` prints them). Once the machine holds WolOS roots, a file signed only that way is refused
(`legacy-refused`).

**Expired is not revoked.** A certificate past `not_after` makes the file count as unsigned
(`community`), unless the caller passes `first_seen`, the time the signed index first saw this exact
file, and it lies inside the certificate's validity. The index is the timestamp.

## Revocation (interface I2)

One signed list, `revoked.json` next to `revoked.json.wsig` (label `wolos/revocation/1`):

```json
{"format":1,"serial":7,"issued":1770000000,"expires":1770604800,"entries":[
  {"serial":"0123456789abcdef","at":1769990000,"reason":"key-compromise"}]}
```

At most 1 MiB and 10,000 entries. The Index intermediate signs lists that withdraw people,
organisations and signers. Only a **root** can withdraw an intermediate or a root, and a root can
withdraw the other root but never itself; an entry that the signer has no right to make is ignored.

- `key-compromise` and `abuse` void every signature by that certificate, whenever it was made (a thief
  can write any date).
- `superseded` and `ceased` only stop new signatures: a file whose `first_seen` is before `at` stays
  valid.
- **Revoked fails closed.** A certificate in any list the machine holds, fresh or stale, is refused.
- A list is **fresh** until its `expires` (issued + 7 days). Up to **30 days** after that it is
  **stale** (`list-stale`). Later, or with no list at all, it is **too old** (`list-too-old`). In
  both cases signatures keep verifying and keep their value, with the line "Not checked for
  withdrawn certificates since <date>" (`stale = true`, `list_expires` in the outcome): the maximum
  offline window before that line appears is 37 days from the list's issue, and nothing turns
  silently unsigned after it (audit M3).
- **Memory** (audit M1): every entry of every list that verified is appended to
  `/var/lib/wolos/trust/seen-revocations.json` (`wol_core::trust::SEEN_FILE`) by `wol trust
  update`, never shortened, and every check consults it. A new list that leaves out a
  `key-compromise` or `abuse` entry seen before, from a signer of the same rank, is refused.
- A list counts when its signer was valid **when it issued the list** (audit M2), so an Index that
  expired later keeps what it withdrew; a list dated more than 24 hours in the future is not
  loaded. An entry never takes effect later than its list's `issued` (audit A7).
- Clocks: a certificate whose `not_before` is up to 24 hours ahead of the machine's clock counts
  (`woltrust::verify::CLOCK_SKEW`, audit L3).
- A list whose signature does not verify is not loaded. The newest list is the one with the highest
  `serial`; a lower one never replaces it.

### On disk

| Path | Content | Written by |
|---|---|---|
| `/usr/share/wolos/trust/roots/*.wolcert` | pinned roots A and B | image build |
| `/usr/share/wolos/trust/intermediates/*.wolcert` | known intermediates | image build |
| `/usr/share/wolos/trust/revoked.json` + `.wsig` | the list of the image | image build |
| `/var/lib/wolos/trust/intermediates/*.wolcert` | newer intermediates and re-issued leaves | the update-check service, after verifying |
| `/var/lib/wolos/trust/revoked.json` + `.wsig` | a newer list | the update-check service, after verifying |

Loading caps each file (certificate 24 KiB, list 1 MiB, signature 16 KiB) and each folder (64
certificates); anything unreadable, oversize or not verifying is left out and noted, never trusted.
A root whose label starts with `test` is never loaded by the system, and the test
`the_image_trust_folder_holds_no_test_root` fails if one is placed in `files/usr/share/wolos/trust/`.

## File certificates (lead decision 2026-11-04)

One signed record per published version, what WolOS Hub checked about exactly that file
(`woltrust::filecert`; called "release statement" in the first order). The Index intermediate
signs it with label `wolos/release/1`; the index entry carries it (`filecert`, base64url, in the
plugin index); the Store verifies it offline beside the author's own signature and shows "Checked
by WolOS Hub on <date>" or refuses with "This version was withdrawn".

```json
{"format":1,"body":"<b64url>","cert":"<Index serial>","sig":{"ed25519":"..","second":".."}}
```

Body: `format`, `serial` (its own 16 hex), `file_sha256` (the whole file), `manifest_sha256` (the
`SHA256` member), `id`, `version`, `kind` (`package`, `plugin`, `extension`), `origin`
(`repo`, `tag`, `commit`, optional), `cert` (the author's serial), `class` (at the time),
`verdict`, `findings` (count), `checked_at`. A revocation entry with the file certificate's serial
and reason `withdrawn-version` withdraws that version alone; certificate checks ignore such
entries. Vectors: `vectors/filecert/` (7).

## Roots name who vouches

A root's `subject.text` (1 to 200 characters, no control or invisible characters, roots only) is
signed in the root. `woltrust init --subject TEXT` is required and has no default; `wol trust
status` prints it as "who vouches", and the Store's install page shows "Who vouches: <text>" under
the trust row.

## Certificate requests (audit H2)

The proof of possession (label `wolos/request/1`) signs the JSON array `[github_id, kind, login,
name, org, namespaces, ed25519, ml-dsa-65, slh-dsa-sha2-128s, x25519, ml-kem-768]`
(`woltrust::request::Claimed`), so the CA cannot issue a certificate that names another login,
name, organisation or namespace: `woltrust issue --proof` refuses it. The requester names the
organisation they publish for; the hub only checks the membership. Unknown top-level fields of the
request file (the hub's `id`, `approved_by`, `approved_at`, `verified_at`) are ignored. Vectors:
`vectors/request/` (16).

## The report (interface I7)

```json
{"verdict":"pass","findings":[{"rule":"trust/revoked-abuse","sentence":"...","evidence":"<serial>"}],
 "trust":"withdrawn","cert":{"serial":"..","subject":{"github_id":1001,"login":"alice","name":"Alice Doe"},"level":"verified"}}
```

`verdict` is the hub's automatic check (`pass`, `flag`, `block`, `not-checked`); `trust` is the table
above; `findings` carry the hub's lines and, when the file is not plainly trusted, one more line
`trust/<code>` whose sentence comes from `Code::sentence`, so the sheet and the hub print the same
words. `cert` is present when a signing certificate was found. `Outcome::report(verdict, findings)`
builds it; the vectors store its exact text.

## Test vectors (what the hub's wasm build replays)

`crates/woltrust/vectors/` holds `pki.json` (the TEST authority: two roots, three intermediates, a
person, an organisation and a WolOS signer, all from public seeds, so anyone can rebuild them) and
one file per case: valid person, organisation and WolOS package; a chain completed from the store;
unsigned; a bad half (Ed25519, ML-DSA, certificate, content); a replayed label; expired, and expired
with a timestamp; not yet valid; each revocation reason, with files before and after; the Index
trying to withdraw an intermediate; root B withdrawing root A; stale, too old and missing lists; a
list with a bad signature; a wrong namespace, the reserved one, wrong usage, a bad issuer; unknown
and test roots; and malformed, 5-certificate and oversize chains.

Each file gives the clock, the label, the signed bytes, the signature, the chain, the store and the
exact expected result, including the report text. A string starting with `@` names a blob in
`pki.json`. Regenerate or check with the development tool (it is not shipped):

```
cargo run -p woltrust --bin woltrust-vectors -- crates/woltrust/vectors
cargo run -p woltrust --bin woltrust-vectors -- --check crates/woltrust/vectors
```

Two runs are byte-identical (the signatures are deterministic), and the test
`the_generator_reproduces_the_files_byte_for_byte` holds that.

## Cost of one verify

Measured on a Xeon Platinum 8488C (x86-64 baseline, release build, thin LTO, 4 KiB message, median of
200 runs; `cargo test -p woltrust --release --test timing -- --ignored --nocapture`). Not measured on
a laptop or in wasm.

| Check | Time |
|---|---|
| Ed25519 alone | 56.6 us |
| Hybrid, Ed25519 + ML-DSA-65 | 257.5 us |
| Hybrid, Ed25519 + SLH-DSA-SHA2-128s (a root) | 413.5 us |
| A whole package: file signature, leaf by the Authors CA, Authors CA by the root, parsing the chain | 1.28 ms |

## Safety of the code

Every input is capped before parsing (see the caps above); no `unsafe`; no panic on bad input. The
tests include a bounded random-input test (`tests/robust.rs`: 3,000 random strings through every
parser and 160 mutated valid files, none of which may panic or turn into a trusted file), and a second
ML-DSA implementation, used only in tests, must agree with ours on real and damaged signatures
(`tests/crosscheck.rs`). Fuzz targets: see phase 3.

## Phase 2 (2026-11-04): encryption, keys, origin, the OS hooks

- **Sealed files** (`woltrust::seal`, format `wolos/seal/1`, ADR 0046 "Encryption"): X25519 + ML-KEM-768
  hybrid KEM, ChaCha20-Poly1305, the whole header as associated data. Vectors in `vectors/seal/`
  (13 cases). Measured on this box (release, median of 200, other jobs running): seal 190-260 us and
  open 170-225 us for up to 4 KiB, 1.3-1.7 ms for 1 MiB; overhead 1178 bytes.
- **Keys**: `woltrust::key` (one seed, its text form for CI), `woltrust::wolkey` (`.wolkey`,
  PBKDF2-HMAC-SHA512 x600 000 + ChaCha20-Poly1305, 410-500 ms to lock or unlock), `woltrust::init`
  (`woltrust init`: both roots, three intermediates, CI keys, one backup, five printed lines),
  `woltrust::request` (proof of possession, label `wolos/request/1`), `wol_core::authorkeys` (where an
  account keeps its keys and certificates).
- **Origin** (`woltrust::origin`): the repository, tag, commit and build date inside `SHA256.wsig`,
  signed, checked against the certificate's login or organisation; the I7 report gains `origin` and
  `cert.verified_at`. Vectors `valid-origin`, `valid-origin-organisation`, `origin-mismatch`,
  `origin-tampered`.
- **Command lines**: `woltrust init|sign|issue|revoke|verify|keygen|public|extract` (the hub pipeline,
  the lead) and `wol cert request|add|show|list`, `wol sign`, `wol encrypt --to`, `wol decrypt`,
  `wol trust status|update`.
- **OS**: wol-admind judges the staged `.wol` with `TrustStore::verify_wol_at` (withdrawn and invalid
  are refused, unsigned needs the unsigned approval); `TrustStore::verify_wolplug` judges `CHAIN` and
  `SHA256.wsig` in a `.wolplug`; the Store sheet has five rows and a "Made by ... from ..." line;
  update-check fetches the list hourly and `wol trust update` installs it only when it verifies and is
  newer; `files/usr/share/wolos/trust/` holds no root until `woltrust init` has been run.

## Phase 3 (2026-11-04)

- The Store's plugin downloads (`wopkg::index::plugins::check_download`) and `wol plugin add` judge
  `.wolplug` files with `TrustStore::verify_wolplug`; `wol sign` signs `.wolplug` files too (through
  the system's zstd, as `wol plugin pack`).
- Passphrases of author keys are read without echo on a terminal (`controller::secret`).
- The revocation list comes from `https://cdn.wolos.org/plugins/revoked.json`, then the mirror
  name of the same server (`wol_core::trust::revoked_urls`, from `wol_core::cdn::HOSTS`), in
  update-check and in `wol trust update` without files.
- Fuzz targets `wolcert`, `revoked`, `chain`, `seal`, `request`, `filecert` (`fuzz/fuzz_targets/`).
- `.wolkey` keeps PBKDF2-HMAC-SHA512 x600 000: a memory-hard KDF (Argon2id) would resist GPU
  guessing better, but there is no Argon2 crate in the tree yet; the format's `kdf` field allows
  adding it as a new value (audit M9, a lead call).

## Not here yet

The real roots and intermediates (the lead runs `woltrust init --subject`), derived 256-bit ids
(SHA-512/256 of the public keys) for serials, file certificates and seal recipients, the binary
`.rkdb` revocation store, and the `wolcrypt` crate (lead decisions of 02:41Z-02:45Z, next round).

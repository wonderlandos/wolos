# 0046 - Author certificates: who made a .wol, checked offline, withdrawn when a key leaks
Status: proposed (draft by HUB-arch 2026-11-03 for the lead order of 23:18Z; lead decisions L1-L9 at the end). Amended 2026-11-04 by TRUST-core for the lead orders of 00:56Z (encryption, post-quantum too) and the lead decisions of that night (no AWS key service, no ceremony, automatic issuance): sections "Encryption", "Keys and setup", "Key storage" and "Origin" below (the last for the lead addition of 01:33Z). Builds on 0011 and 0021 (.wol, trust per source), 0031 (.wolplug, signed plugin index), 0034 (root-staged installs), 0040 (security defaults). ADR 0047 (WolOS Hub) is the service that issues and publishes what this record defines.

Labels: [mem] = taken from project memory or the code as it is; [fresh] = reasoned or measured for this record. Every measured number names its box.

## Context
- [mem] Today a `.wol` is signed or not. `wol_core::package::verify` (`crates/core/src/package.rs:199`) checks the raw 64-byte Ed25519 `SHA256.sig` against flat keys in `/usr/share/wolos/keys/*.pub` (`shared::package::KEYS_DIR`, `crates/shared/src/package.rs:22`; today `wolos-2026.pub`, `wolos-2026b.pub`), minus `revoked.list` (`package::REVOKED_FILE`, `crates/core/src/package.rs:536`, applied by `read_keys` at :560). The Store sheet has three trust rows, Verified, Not signed, Can't be installed (`TRUST_ROWS`, `crates/shell/src/ui/store.rs:240`), and wol-admind verifies the staged copy again as root (`stage_and_install`, `crates/api/src/admind.rs:385`).
- [mem] `.wolplug` files and the plugin index use the same key form (`verify_wolplug` :310, `verify_index` :714); the index carries `serial` and `expires`, checked by `wopkg::release::check_fresh` (`crates/wopkg/src/release.rs:160`).
- [mem] So "signed" means "signed by WolOS". There is no way to say "made by this person", no way to trust a person without handing them a WolOS key, and revoking means shipping a new image or a new `revoked.list`.
- [fresh] The lead wants: certificates WolOS issues to people and organisations, checked by the system offline; signatures that hold in the post-quantum era; revocation that works and is understood offline, where a revoked certificate stops validating; anyone may still publish, but a certified author shows as different; an unsigned `.wol` asks "Do you really want to open this?".

## Decision

### Identities
- **Root**: two pinned WolOS roots, A (active) and B (standby), each a key pair held offline (Key ceremony). Roots sign only intermediates and revocations of intermediates or of the other root. Validity 20 years.
- **Intermediates** (signed by the active root, 3 years each, one purpose each):
  - Authors CA: issues person and organisation certificates.
  - Packages CA: issues the signer certificates of WolOS's own compiled packages (`org.wolos.*`).
  - Index: signs the hub index and the revocation list (ADR 0047).
- **Person**: one GitHub account, named by its numeric id (stable across renames), with the login and display name as shown text. **Organisation**: one GitHub organisation, by numeric id; the request comes from an owner of that organisation; the organisation holds its own signing key (in its CI or with its release manager). Members and roles stay in GitHub and are read by the hub when someone acts (ADR 0047); they are not in the certificate.
- **Namespace**: each person or organisation certificate lists the app-id prefixes it may sign (`alice.` for person `alice`, `examplelabs.` for an organisation, a verified reverse domain later). A signature on an id outside the list is refused. `org.wolos.` belongs to the Packages CA only.

### Certificate and signature format (format 1)
- A certificate (`.wolcert`) is JSON: `{"format":1,"body":"<base64url>","sig":{"ed25519":"<b64url>","second":"<b64url>"}}`. The body is the exact signed bytes, never re-serialised, so there is no canonical-JSON step. Body (UTF-8 JSON, at most 8 KiB, no floats; 4 KiB before the encryption keys): `serial` (16 hex, random), `kind` (`root|intermediate|person|org|signer`), `subject` (`github_id`, `login`, `name`, for intermediates a label), `namespaces` (prefixes), `usage` (subset of `sign-certs`, `sign-index`, `sign-revocations`, `sign-package`, `sign-plugin`, `sign-extension`), `keys` (`ed25519`, and `ml-dsa-65` or for roots `slh-dsa-sha2-128s`, base64url; optionally the encryption keys `x25519` and `ml-kem-768`, both or neither, see "Encryption"), `issuer` (issuer serial), `not_before`, `not_after` (Unix seconds), `level` (`wolos|verified`).
- What is signed: every signature in this record is over `label || 0x00 || SHA-512(content)`, with one label per purpose: `wolos/cert/1`, `wolos/package/1`, `wolos/plugin/1`, `wolos/extension/1`, `wolos/index/1`, `wolos/revocation/1`. The label stops a signature made for one purpose from being replayed as another; hashing first keeps the signed message under 100 bytes, so any signer (a CI job, a phone, a future hardware key) signs the same short bytes. The second algorithm's own context string stays empty for the same reason.
- **Hybrid, both must verify.** A signature is valid only when the Ed25519 half and the second half both verify. There is no "either" mode and no downgrade: the format number fixes the pair (Ed25519 + ML-DSA-65 below the root; Ed25519 + SLH-DSA-SHA2-128s for the roots). A new pair is a new format.
- **In a `.wol`** (format 2 of ADR 0011): after the existing members come two optional members, `CHAIN` (JSON array of `.wolcert`, leaf first, at most 4 certificates and 64 KiB, root never included) and `SHA256.wsig` (`{"cert":"<leaf serial>","ed25519":"..","second":".."}` over the `SHA256` member with label `wolos/package/1`). `SHA256` keeps hashing the payload members only, so a renewed chain does not change the package hash. The legacy `SHA256.sig` stays for WolOS's own keys. `.wolplug` and extensions take the same two members with their labels. A 0.12 system refuses the new members ("unexpected member"), so format-2 files declare `min_wolos` 0.13.

### Origin (lead addition 2026-11-04 01:33Z: a certificate shows where the app comes from)
- **In the certificate** (additive to format 1; anyone can read a body, it is plain base64url JSON): `subject.github_id` (permanent), `subject.login`, `subject.name`, optionally `subject.org` = `{"github_id","login"}` for a person who publishes for a GitHub organisation (set by the hub from the organisation's membership, not claimed by the requester), `namespaces`, `issuer`, and `verified_at` (Unix seconds, when the hub verified the account and the request; before `not_after`). An organisation certificate is itself the organisation (`kind: org`).
- **In the release signature**: `SHA256.wsig` gains an optional `"origin"`, the base64url of the exact bytes of `{"repo":"https://github.com/OWNER/NAME","tag":"..","commit":"<40 hex>","built":<unix>}` (at most 1 KiB). With an origin both halves sign `label || 0x00 || SHA-512(SHA256 member) || SHA-512(origin)`; without one the message is unchanged, so every older signature still verifies. The pipeline signs it with `woltrust sign --origin ORIGIN.json`, an author with `wol sign FILE --as SERIAL REPO TAG COMMIT`.
- **Checked offline**: the origin parses (GitHub HTTPS form, a tag, a full commit id), it is covered by the signature (else `bad-signature`), and the repository owner is the certificate's login or its organisation's login, compared without case (WolOS's own signers: the `wonderlandos` organisation); else the file is refused with `origin-mismatch`. The id still has to lie in the certificate's namespaces.
- **I7 report**: `cert` gains `verified_at` and the subject's `org`; the report gains `origin` (`repo`, `tag`, `commit`, `built`), each omitted when absent, so the reports of the older vectors are byte for byte the same. Vectors: `valid-origin`, `valid-origin-organisation`, `origin-mismatch`, `origin-tampered` (TEST person `test-ana`, organisation `anaco`).

### Offline verification
- **Trust store in the image**: `/usr/share/wolos/trust/roots/` (A and B), `/usr/share/wolos/trust/intermediates/`, `/usr/share/wolos/trust/revoked.json` + `.wsig`, all fixed at build. Newer intermediates and lists arrive through the signed index (ADR 0047) and are kept, after verification, in `/var/lib/wolos/trust/` (root's, readable by all, written only by the update-check service). A newer list (higher `serial`) replaces an older one, never the reverse. The existing `/usr/share/wolos/keys/` stays as the legacy path for WolOS release files.
- **Check order** (in `wol_core`, again as root in wol-admind on the staged copy, the same way `stage_and_install` does today): bounded parse of `CHAIN`; leaf usage covers the file kind and a namespace covers the id in `Info.json`; `SHA256.wsig` verifies with the leaf's keys; each certificate verifies with its issuer (next in the chain, or a store certificate with that serial), the issuer may issue that kind (root issues intermediates; Authors CA issues person and org; Packages CA issues signer), path at most 3; the top issuer is a pinned root by exact key bytes; no certificate in the chain is in any revocation list held; the clock is inside every certificate's validity.
- **Expired is not revoked.** A certificate past `not_after` makes the file count as unsigned (the question sheet), unless the signed index lists this exact file (sha256) with a `first_seen` inside the certificate's validity: the index is the timestamp. Revoked is refused.
- **Repair**: when only an intermediate was replaced (not a leaked leaf), the hub re-issues the leaf with the same keys under the new intermediate and publishes it; the verifier may build the chain from store certificates for the same leaf keys instead of the embedded chain.
- [fresh, measured] Cost of one check on this box (Xeon Platinum 8488C, x86-64 baseline, release build, LTO, 4 KiB message, one run): Ed25519 verify 99.6 us, ML-DSA-65 verify 218 us (137.7 us with the second crate below), SLH-DSA-SHA2-128s verify 327.5 us. A full chain (package, leaf, intermediate) is 3 Ed25519 + 2 ML-DSA-65 + 1 SLH-DSA, about 1.1 ms. Not measured on a laptop or in wasm.

### Revocation
- **One signed list**, `revoked.json` + `revoked.json.wsig` (label `wolos/revocation/1`, signed by the Index intermediate; revocations of intermediates or of a root are signed by a root): `{"format":1,"serial":N,"issued":T,"expires":T+7d,"entries":[{"serial":"<16 hex>","at":T,"reason":"key-compromise|abuse|superseded|ceased"}]}`. It travels with every index publication (at least daily, ADR 0047), sits in each image, and the update-check timer that already runs hourly (ADR 0040) refreshes it.
- **Reasons**: `key-compromise` and `abuse` invalidate every signature by that certificate, whenever made (a thief can sign with any date). `superseded` and `ceased` keep files the index saw before `at` valid; new signatures fail.
- **Revoked fails closed.** A certificate found in any list the machine holds, fresh or stale, is refused with no override. On a newly arrived revocation, installed apps signed by that certificate get one notification ("The certificate “Name” was signed with has been withdrawn. ...") and their next launch and every update ask the unsigned question (L5 decides whether launch is blocked instead).
- **Stale offline**: list fresh (before `expires`): normal. Stale up to 30 days past `expires`: verified files still show as verified with one more line, "Not checked for withdrawn certificates since 3 November". Older than that: verified files count as unsigned (the question), never refused. The window bounds how long a leaked key keeps working on a machine that never goes online; a machine that is online gets a new list within an hour.
- A root is revoked only by the other root: a list signed by B may revoke A (and B becomes active); clients pin both, so this works without an image update. If both leak, only an image update recovers.

### What the person sees
Store install page and the open sheet (`TRUST_ROWS` grows from three to five rows):
- **WolOS**: "Made by WolOS." (Packages CA chain.)
- **Verified author**: "Made by Ana Lima (ana on GitHub), from github.com/ana/notes, version 1.2." (without an origin: "Made by Ana Lima (ana on GitHub), version 1.2."); the row title is "Verified author". For an organisation the same line with its name and login. When the list is stale, the extra line above. Implemented as `copy::trust_made_by` and the five `TRUST_ROWS` of `crates/shell/src/ui/store.rs` (WolOS, Verified author, Not signed, Withdrawn, Can't be installed).
- **Not signed**, as a sheet before anything is installed or opened:
  - Title: "Do you really want to open “Name”?"
  - Text: "Nobody vouches for this app. WolOS can't tell who made it or whether someone changed it on the way. Once open, it can use everything your account can: your files, your browser's saved passwords, your network."
  - When the hub lists this exact file: "WolOS Hub lists this file. Its automatic check found nothing to report." or "... found 2 things to look at:" with the lines.
  - Buttons: "Don't Open" (default) and "Open Anyway…", which then asks for the administrator password, never cached (ADR 0011's fresh-password rule; plugins keep their typed `yes`). No "always allow" for unsigned files.
- **Withdrawn**: "WolOS no longer trusts the certificate this app was signed with, because its key was exposed. It won't be installed." Buttons "OK" and "Move to Trash".
- **Can't be installed**: unchanged.

### Signature scheme: options [fresh]
| | A. Ed25519 only | B. Hybrid Ed25519 + ML-DSA-65 (favourite), roots Ed25519 + SLH-DSA-SHA2-128s | C. ML-DSA-65 only | D. SLH-DSA everywhere |
|---|---|---|---|---|
| Survives a quantum computer | no | yes (second half) | yes | yes |
| Survives a flaw in the new lattice scheme or its young code | yes | yes (Ed25519 half; root on hash-only assumptions) | no | yes |
| Public key / signature (bytes, measured) | 32 / 64 | 1984 / 3373; root 64 / 7920 | 1952 / 3309 | 32 / 7856 (128s) |
| Verify, this box | 99.6 us | ~318 us per signature | 218 us | 327.5 us; signing 336 ms (128s) |
| Code added to a binary (measured, stripped, LTO, verify only) | 0 (in tree) | +73 KB (ML-DSA-65) and +37 KB (SLH-DSA-128s) over the existing Ed25519 | +73 KB | +37 KB |
| Remote signing service supports it | Ed25519 yes | yes for intermediates; the root stays offline | yes | no |

- A is today's code and smallest, but a 20-year root on Ed25519 alone is the exact risk the lead named.
- C drops the one algorithm with decades of analysis and stands on young code.
- D has the most conservative assumptions, but 7.8 KB per signature and 336 ms per signature make every certificate and index publication slow and large, and it cannot sign remotely.
- **B is recommended**: a forger must break both halves; the root, which is the hardest key to replace, rests on hash functions only.

**Premortem of B** (it is 2028 and it failed; why):
1. The ML-DSA crate accepted a bad signature. Hurts only if Ed25519 also fails; tests hold the two ML-DSA crates and the NIST vectors to the same answers; the parse is fuzzed (the fuzz list rule exists).
2. (Withdrawn 2026-11-04: there is no remote signer. Intermediates are software keys, see "Keys and setup"; the risk moved to item 7.)
3. Two algorithms double the parsing and key handling; more code is more bugs. Mitigation: one pure crate, no I/O, bounded inputs, used by the OS, the hub Worker and the browser alike, so there is one implementation to get right.
4. Sizes grow. About 25 KB per signed package (chain and signature in base64); irrelevant next to payloads and inside the 2 MiB `.wolplug` cap. Leaf certificates do not go into the index, only intermediates do.
5. Nobody runs the ceremony and the test root ships. The image build refuses a test root (TRUST-core test), and no index is signed before L1-L3.
6. A future standard change or a better pair. The format number carries the pair; format 2 can add one without breaking format 1.
7. A CI secret leaks (the Authors CA or Index key file and its passphrase). Both live only in their GitHub environment ("ca", "sign"), the key file is locked, and the passphrase is a separate secret; a leak means root A revokes that intermediate and `woltrust issue` makes a new one from the backup (Repair). The roots are never in CI.

### Rust crates (dependency table; names appear only here)
| Crate | Version | Use | Notes |
|---|---|---|---|
| ed25519-dalek | 2.2.0 | Ed25519 (in tree) | `verify_strict` as today |
| fips204 | 0.5.0 | ML-DSA-65, favourite | pure Rust, `no_std`, no heap; README claims constant-time signing and a WASM build; MIT OR Apache-2.0; no independent audit named |
| ml-dsa | 0.1.2 | second ML-DSA, tests only | pure Rust; README: "never been independently audited"; Apache-2.0 OR MIT |
| fips205 | 0.5.0 | SLH-DSA-SHA2-128s (roots) | pure Rust, `no_std`; MIT OR Apache-2.0 |
| slh-dsa | 0.2.0-rc.5 | none | pre-release |
| aws-lc-rs | 1.16.3 | none | ML-DSA only behind its `unstable` feature, C and assembly, no wasm |
| fips203 | =0.5.0 | ML-KEM-768 (sealed messages), added 2026-11-04 | pure Rust, `no_std`, same author and family as fips204/fips205; MIT OR Apache-2.0; its dependencies (rand_core 0.6, sha3, subtle, zeroize) were already in the tree |
| curve25519-dalek | 4.1 | X25519 (Montgomery ladder) | already in the tree under ed25519-dalek; now also direct |
| ring | 0.17 | none since Phase 4 | was ChaCha20-Poly1305 and PBKDF2 until crates/wolcrypt replaced it (still in the tree under the TLS client) |
| chacha20poly1305 | 0.10.1 | ChaCha20-Poly1305 (sealed messages, `.wolkey`), added 2026-11-04 Phase 4 | RustCrypto, pure Rust, `no_std`; MIT OR Apache-2.0; in crates/wolcrypt only |
| pbkdf2 | 0.12.2 | PBKDF2-HMAC-SHA512 (`.wolkey`), added 2026-11-04 Phase 4 | RustCrypto, pure Rust, `no_std`; MIT OR Apache-2.0; in crates/wolcrypt only |
| zeroize | 1.8 | wipes seeds and passphrases | already in the tree |

All four candidates built here with rust 1.99.1 in 28 s (scratch crate, `/tmp/hubarch-bench`, outside every repo). Adding them to the workspace is a CTO decision (`deny.toml`). `cargo deny check` passes with the 2026-11-04 additions and no change to `deny.toml` (all MIT, Apache-2.0, BSD or ISC, all from crates.io).

### Keys and setup (lead decision 2026-11-04: no AWS key service, no ceremony)
- **One command.** The lead runs `woltrust init --out DIR` once, on their own machine. It generates root A and root B (Ed25519 + SLH-DSA-SHA2-128s, 20 years) and the three intermediates (Authors CA, Packages CA, Index; Ed25519 + ML-DSA-65, 3 years, signed by root A) from fresh random seeds, asks for one backup passphrase (at least 8 characters, typed twice), and writes: `DIR/public/` (the five certificates, for `/usr/share/wolos/trust/`), `DIR/ci/authors-ca.wolkey` and `DIR/ci/index.wolkey` (locked with a random CI passphrase in `DIR/ci/passphrase`), and `DIR/wolos-trust.wolbackup` (all five keys, each locked with the backup passphrase). Private files are created mode 0600 and never over an existing file. It prints five lines and nothing else:
  1. `gh secret set WOLOS_AUTHORS_CA_KEY --env ca --repo wonderlandos/hub-index < DIR/ci/authors-ca.wolkey`
  2. `gh secret set WOLOS_INDEX_SIGNING_KEY --env sign --repo wonderlandos/hub-index < DIR/ci/index.wolkey`
  3. `for e in ca sign; do gh secret set WOLOS_KEY_PASSPHRASE --env $e --repo wonderlandos/hub-index < DIR/ci/passphrase; done`
  4. Copy `DIR/wolos-trust.wolbackup` to your phone; it holds all five keys and opens only with the passphrase you typed.
  5. Give `DIR/public/` to the WolOS repository (`files/usr/share/wolos/trust/`), then delete `DIR`.
  That is the whole procedure. The roots and the Packages CA exist only in the backup; `woltrust extract --backup F --role root-a` gives one locked key back when a root must sign (a new intermediate, a revocation of an intermediate).
- **Intermediates are software keys** held as GitHub environment secrets: `WOLOS_AUTHORS_CA_KEY` in "ca", `WOLOS_INDEX_SIGNING_KEY` in "sign", `WOLOS_KEY_PASSPHRASE` in both. `woltrust sign|issue --key-env NAME --passphrase-env WOLOS_KEY_PASSPHRASE` is non-interactive.
- **Issuance is automatic** after an approval in the hub admin panel: the ca workflow runs `woltrust issue --body BODY.json --proof PROOF.json --key-env WOLOS_AUTHORS_CA_KEY --passphrase-env WOLOS_KEY_PASSPHRASE`, which refuses a person or organisation body without a proof of possession that verifies (label `wolos/request/1`, see "Requests"), and publishes the certificate under `certs/` in hub-index and in the index. The author downloads it from the hub; `wol cert add FILE` installs it into the account's store.
- **WolOS's own packages**: the Packages CA (from the backup) issues signer certificates when WolOS packages move to certificates; until then `SHA256.sig` with today's keys stays valid next to it.
- **Leak**: of an author key, revoke the leaf (`key-compromise`); the next list goes out within the hour. Of an intermediate, root A (from the backup) revokes it and signs a new one, leaves are re-issued (Repair), the CI secrets are replaced. Of root A, root B revokes it and the next image drops A.

### Requests
`wol cert request GITHUB_ID LOGIN NAMESPACE NAME... [--org]` makes the keys on the author's machine (one random 32-byte seed; the signing and the encryption keys derive from it with distinct labels), locks them at once (Key storage) and prints the hub-index request (`schema/request.schema.json` shape, `keys` with all four public keys). The proof is a hybrid signature with the new keys, label `wolos/request/1`, over the text `"{github_id}\n{kind}\n{ed25519}\n{ml-dsa-65}\n{slh-dsa-sha2-128s}\n{x25519}\n{ml-kem-768}\n"` (the base64url strings of the body, empty when absent). `woltrust issue` recomputes it from the body the CA built, so the request file is not needed there. The encryption keys cannot sign, so their possession is not proven: a requester who names someone else's encryption key only sends their own mail to that someone.

### Key storage
- An author's keys: `~/.local/share/wolos/trust/keys/<name>.wolkey` (`$XDG_DATA_HOME` when set), `<name>` the key id (first 8 bytes of SHA-256 of the Ed25519 key, hex) until the certificate comes, then its serial (`wol cert add` re-locks it under the serial). Certificates kept by the account: `.../trust/certs/<serial>.wolcert`. A key is never written in the clear; the passphrase is read from stdin, never from an argument.
- `.wolkey` (format 1, JSON, at most 2 KiB, 315 bytes measured): `{"format":1,"name":"<16 hex>","kdf":"pbkdf2-hmac-sha512","rounds":600000,"salt":"<16 B>","aead":"chacha20-poly1305","nonce":"<12 B>","sealed":"<key material + tag>"}`. Associated data `"wolos/wolkey/1" 0x00 name 0x00 rounds 0x00 salt`: a file renamed to another serial or with its rounds changed does not open; fewer than 210 000 or more than 10 000 000 rounds are refused. The same format holds the CI intermediates and the backup's keys.
- KDF [fresh]: Argon2id is the better choice against GPU guessing, but no Argon2 or scrypt crate is in the dependency tree; PBKDF2-HMAC-SHA512 and ChaCha20-Poly1305 come from ring, which is (under the TLS client). 600 000 rounds cost 410-500 ms per lock or unlock on this box (Xeon Platinum 8488C, release, one run each, while other jobs ran). Moving to Argon2id later is a new `kdf` value, not a new format.

### Encryption (lead order 2026-11-04 00:56Z: "a certificate system with encryption and keys for the sign; post-quantum encryption too")
- **What**: every certificate may carry an encryption key beside its signing keys (`keys.x25519`, `keys.ml-kem-768`, both or neither). The field is additive inside format 1: bodies without it parse and verify as before, and the 41 signature vectors made before it are byte for byte unchanged. `wol cert request` always includes it; certificates issued before it simply cannot receive sealed files ("This certificate has no encryption key; ask its owner to renew it."). Roots and the CI intermediates made by `woltrust init` carry none.
- **Sealed message, format `wolos/seal/1`** (frozen; `woltrust::seal`): `"wolos/seal/1" 0x00` (13 B) | label length (1 B, 1-64) | label (ASCII `[a-z0-9/._-]`) | recipient serial (16 hex) | X25519 ephemeral public key (32 B) | ML-KEM-768 ciphertext (1088 B) | ChaCha20-Poly1305 ciphertext and 16-byte tag. The associated data is the whole header, so the label, the recipient and both KEM ciphertexts are bound. Key schedule: `okm = SHA-512("wolos/seal/1/kdf" 0x00 || ss_mlkem || ss_x25519 || x25519_ephemeral || x25519_recipient || SHA-512(mlkem_recipient) || SHA-512(header))`, key = okm[0..32], nonce = okm[32..44]; every key is used once. An all-zero X25519 result (a low-order point) is refused. `wol encrypt FILE --to SERIAL` seals under the label `wolos/file/1` to `FILE.wolsealed`; `wol decrypt FILE.wolsealed` finds the key by the serial in the header. Plaintext at most 64 MiB in one piece.
- **Sizes and cost** [fresh, measured on this box, release, median of 200, other jobs running so ranges are given]: overhead 1178 bytes for `wolos/file/1` (an empty file seals to 1178 B, 4 KiB to 5274 B, 1 MiB to 1 049 754 B); seal 190-260 us and open 170-225 us for up to 4 KiB, about 1.3-1.7 ms for 1 MiB; key pair from a seed 112-120 us; the encryption keys add about 1.6 KB of base64 to a body (hence 8 KiB).
- **Vectors**: `crates/woltrust/vectors/seal/` (keys.json with two TEST recipients `test-alice-2`, `test-examplelabs-2` issued by the TEST Authors CA, and 13 vectors: sealed to a person, to an organisation, empty, 4 KiB, wrong recipient, wrong keys, wrong label, relabelled, tampered X25519 key, ML-KEM ciphertext, ciphertext, tag and recipient). Each holds the 64 random bytes it was sealed with, so another implementation reproduces the sealed bytes.

| | A. X25519 only | B. Hybrid X25519 + ML-KEM-768 (chosen) | C. ML-KEM-768 only |
|---|---|---|---|
| A recording read later by a quantum computer | readable | not readable (ML-KEM half) | not readable |
| A flaw in ML-KEM or its young code | n/a | X25519 half holds | readable |
| Public key / ciphertext overhead | 32 / 32 B | 1216 / 1120 B | 1184 / 1088 B |
| Seal + open, this box | about 0.1 ms | about 0.4 ms | about 0.3 ms |
| New code | none (in tree) | fips203 (one crate) | fips203 |

- A is smallest but is exactly what "post-quantum encryption" rules out: unlike signatures, encryption has a record-now-read-later risk, so the urgency is today, not in 20 years. C rests the secrecy on one young scheme and one young crate. **B** needs both broken; its cost (1.1 KB per message, 0.4 ms) is irrelevant next to files. It follows the X-Wing construction (draft-connolly-cfrg-xwing-kem) with SHA-512 in place of SHA3-256 and both ciphertexts and both public keys in the hash.
- **Checked against two references the lead sent (2026-11-04 01:42Z), read that night** [fresh]:
  - OpenSSH, "Post-Quantum Cryptography" (https://www.openssh.org/pq.html): `mlkem768x25519-sha256`, the default since OpenSSH 10.0, is the same pair as ours (ML-KEM-768 with X25519), chosen as a hybrid so it "is no worse than the previous best classical algorithm, even if the post-quantum algorithm turns out to be completely broken"; the page gives "store now, decrypt later" as the reason key agreement moves first and says signatures have no such urgency, only the retirement of classical keys before a quantum computer exists. Both match option B and the order of this record (encryption hybrid now; signatures hybrid with long-lived roots on hash-based SLH-DSA). Divergences: SSH hashes its two secrets with SHA-256 inside an interactive key exchange, both sides contributing; a sealed file is one-way and stored, so ours is a KEM to a static recipient key with SHA-512 over both secrets, both public halves and the whole header, and an AEAD key used once. OpenSSH has no post-quantum signatures yet; we have them because certificates and roots live for years.
  - LACNet, "Generation of Post-Quantum Certificates" (https://lnet.global/generation-of-post-quantum-certificates/): extends X.509 through a modified OpenSSL with a Falcon-512 key next to the ECDSA one, a CSR signed by each private key as proof of possession, and the subject bound to an existing identity (a traditional X.509 the CA checks), with the keys published in a public registry. Same as ours: a classical and a post-quantum key side by side in one certificate, proof of possession by signing the request (our `wolos/request/1`), the identity taken from an account that already exists (GitHub, not a prior X.509), the public parts published (hub-index `certs/` and the index). Divergences, and why: (1) Falcon-512 is smaller, but it was not among the August 2024 NIST standards (FIPS 203 to 205: ML-KEM, ML-DSA, SLH-DSA), and its floating-point signing is hard to make constant-time, so we keep ML-DSA-65 and SLH-DSA; (2) they use one post-quantum algorithm for both signing and encryption; FIPS separates them (ML-DSA signs, ML-KEM encapsulates), and we derive the two key sets from one seed with distinct labels so one backup covers both; (3) they rely on X.509 and a forked OpenSSL; our format is small JSON with exact signed bytes and one pure Rust crate, because every verifier is ours (system, hub Worker, browser) and a fork of a TLS library is the larger risk; (4) their keys are generated with quantum entropy from a service; ours come from the OS generator (`/dev/urandom`), which is sufficient for keys whose security rests on the algorithms, not on the entropy source.
- **Premortem of B** (2028, it failed; why): (1) fips203 had a bug that leaked the shared secret: the X25519 half still protects, and the vectors pin the exact bytes, so a crate swap is checked; (2) a combiner mistake (a field left out) made the two halves separable: the KDF hashes both secrets, both public halves and the whole header, and a test flips each part; (3) authors lose their passphrase and with it every sealed file: the key file is the only copy by design, `wol cert request` says so, and a lost key means a new certificate, not a recovery; (4) the same seed signs and decrypts, so one leak exposes both: the labels keep the keys independent, but a leak of the `.wolkey` and its passphrase is still total, and revocation covers both at once; (5) a sealed file says who it is for (the serial in the header): accepted, the recipient must be found without trying every key; (6) the hub's wasm cannot seal or open: by design the wasm build takes `default-features = false` (no ring); sealing happens on the machine that holds the key.
### Threat model
| Threat | Result |
|---|---|
| A file changed after signing, or a swapped release asset | hash or signature fails: refused |
| A stolen author key | works until the revocation reaches the machine (an hour online, at most 30 days past `expires` offline), then refused |
| An attacker controls an author's GitHub account but not the key | can publish only unsigned (the question sheet); verified updates fail |
| The hub's Worker or database is taken over | cannot sign anything; can list spam until moderators see it (ADR 0047) |
| The intermediate's signing role is abused | logged; the root revokes the intermediate |
| A frozen or rolled-back index or list | `serial` never goes back; `expires` makes a freeze visible within 7 days |
| An id that looks like another author's | namespaces bind ids to GitHub ids; the sheet shows the GitHub login |
| A quantum computer, in the roots' lifetime | the ML-DSA and SLH-DSA halves hold |
| A flaw in ML-DSA or its code | the Ed25519 half holds; roots rest on hash functions only |

What "post-quantum era" buys: the certificates and the roots stay unforgeable if quantum computers that break elliptic curves appear while they are still trusted (the roots have 20 years). Unlike encryption, signatures have no "record now, read later" risk, so the urgency is in the long-lived keys. What it costs: about 25 KB per package, about 1 ms per check, about 110 KB of code, and two algorithms to maintain. Not covered: the transport (TLS) and the pacman repositories, which keep their own trust (ADR 0021).

### Phase 3 amendments (TRUST-core-4, 2026-11-04, lead decisions 02:18Z-02:23Z and TRUST audit 1)

- **Two certificates per author.** Body field `class`: `origin` (automatic at GitHub sign-in with
  the CLA) or `trusted` (after an administrator's approval); absent means origin. A trusted body
  names `origin_cert`, the serial of the same subject's origin certificate, and has its own keys.
  Trust values: `wolos`, `trusted`, `origin`, `legacy`, `community`, `withdrawn`, `invalid`
  ("verified" is gone); the I7 `cert` carries `class`.
- **Revocation, final.** `key-compromise` and `abuse` on any certificate void everything its key
  signed (on an origin certificate: the supply-chain emergency path, it also voids the trusted
  one); `superseded` on a trusted certificate downgrades its files to origin (`trust-withdrawn`);
  `ceased` keeps files seen before it; `withdrawn-version` names a file certificate only. An
  entry never takes effect after its list's `issued`.
- **File certificates** (`woltrust::filecert`, label `wolos/release/1`, signed by the Index): one
  per published version, binding the whole file's SHA-256, the `SHA256` member's SHA-256, id,
  version, kind, origin, the author's serial and class, the verdict, the count of findings and the
  date. Withdrawn alone by `withdrawn-version`.
- **Roots name who vouches**: `subject.text`, signed, roots only, required by `woltrust init
  --subject`.
- **Audit 1.** H1: a raw Ed25519 `SHA256.sig` is `legacy`, revocable by its key id, refused once
  the machine holds roots. H2: the request proof covers every field the CA copies. M1: a machine
  keeps every withdrawal it saw (`seen-revocations.json`) and refuses a list that drops a hard
  one. M2: a list counts if its signer was valid when it issued it. M3: past the stale window, or
  with no list, signatures keep their value with the "not checked since" line (37 days from the
  list's issue). L1: invisible and bidi characters are refused in names. L3: 24 hours of clock
  skew on `not_before`.
- **Key files (audit M9, D note).** `.wolkey` keeps PBKDF2-HMAC-SHA512 at 600 000 rounds:
  standard (SP 800-132) and pure Rust, but not memory-hard, so a short passphrase falls to GPU
  guessing far sooner than with Argon2id. Argon2id would be a new `kdf` value; no Argon2 crate is
  in the tree yet (a dependency decision for the lead).
- **Not yet (next round):** done in Phase 4 below.

### Phase 4 amendments (TRUST-core-5, 2026-11-04, lead decisions 02:41Z-03:27Z)

Byte layouts and the runbook are in docs/modules/trust.md ("Phase 4"); the crate in
docs/modules/wolcrypt.md.
- **Ids.** Every identifier is derived: a certificate serial is SHA-512 over its public keys
  (`ed25519 || ml-dsa-65 || x25519 || ml-kem-768`, roots `ed25519 || slh-dsa-sha2-128s`), 128 hex,
  recomputed on every parse; a file certificate's id is SHA-512 of the `SHA256` member; a sealed
  message names the recipient's id (`wolos/seal/2`; `wolos/seal/1` still opens). Format stays 1 (the
  longer serial is additive); 16-hex serials are "legacy-length", accepted only under a TEST root.
  Second preimage 2^512 classical / 2^256 quantum; collision 2^256.
- **Two root signatures on every intermediate** (`sigs`, additive): both must verify and both roots
  join the path; one root alone signs the emergency withdrawal of the other or of an intermediate.
- **Witness keys.** Two intermediates with usage `witness` alone sign published files (detached
  `<file>.wsig`, label `wolos/witness/1`); never an app.
- **Revocation database `revoked.rkdb`** (binary, sorted, Index-signed, a root-signed section for
  intermediates and roots), deltas, merge, the JSON list as export. Lists live 24 hours, republished
  hourly. Freshness: the image's database is a floor; every file certificate (and origin) carries
  `rkdb_serial` and raises it; two checkpoints (CDN, public log) must agree, else warn and keep the
  higher. A connected machine with a stale or behind list holds installs of new apps.
- **Emergency** (`woltrust emergency`, `woltrust init --rotate-intermediates`): runbook of ten steps;
  both roots gone means a new image.
- **Signature lines** (`woltrust::line`), one per signed body, and `terms {version, sha256}` in
  requests, certificates and file certificates.
- **wolcrypt.** All cryptography in `crates/wolcrypt` (pure, `no_std`, no ring), a C ABI
  (`crates/wolcrypt-ffi`, libwolcrypt.so.1) and a wasm build; same bytes as before.

## Consequences
- `.wol`, `.wolplug` and extension format 2 needs WolOS 0.13; 0.12 keeps working with today's keys and refuses format-2 files with a sentence.
- One pure crate holds formats and checks, without I/O; `wol_core`, wol-admind, the Store, the hub Worker (wasm) and the browser (wasm) share it.
- No online status check is ever made when a file is opened: the list comes with the index, so opening an app tells nobody anything.
- New CLI: `wol cert request|add|show|list`, `wol sign FILE.wol --as SERIAL`, `wol encrypt FILE --to SERIAL`, `wol decrypt FILE.wolsealed`, `wol trust status`, `wol trust update LIST SIG` (root, called by update-check). The hub never sees a private key. `woltrust init|sign|issue|revoke|verify|keygen|public|extract` is the pipeline's and the lead's tool (crate woltrust, binary `woltrust`).
- The update-check service downloads `revoked.json` and its `.wsig` from the hub CDN every hour (HTTPS only, capped), and `wol trust update` installs them in `/var/lib/wolos/trust/` only when they verify against the image's roots and are newer.

## Lead decisions
- L1: approve scheme B (hybrid, SLH-DSA roots).
- L2: decided 2026-11-04: no ceremony; the lead runs `woltrust init` once (Keys and setup).
- L3: decided 2026-11-04: no AWS key service; intermediates are locked software keys in GitHub environment secrets.
- L4: who approves certificate issuance (two people, or the lead alone).
- L5: a revoked certificate on an installed app: ask at launch (recommended) or block launch.
- L6: the stale window (30 days recommended) and list lifetime (7 days).
- L7: what a person must show for a certificate (GitHub two-factor on, account at least 90 days old, a listed release that passed its checks, CLA signed: recommended), and whether any human identity check is added.
- L8: certificate lifetimes (person and organisation 1 year, intermediates 3, roots 20).
- L9: the sheet's words (above).

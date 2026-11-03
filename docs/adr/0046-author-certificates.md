# 0046 - Author certificates: who made a .wol, checked offline, withdrawn when a key leaks
Status: proposed (draft by HUB-arch 2026-11-03 for the lead order of 23:18Z; lead decisions L1-L9 at the end). Builds on 0011 and 0021 (.wol, trust per source), 0031 (.wolplug, signed plugin index), 0034 (root-staged installs), 0040 (security defaults). ADR 0047 (WolOS Hub) is the service that issues and publishes what this record defines.

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
- A certificate (`.wolcert`) is JSON: `{"format":1,"body":"<base64url>","sig":{"ed25519":"<b64url>","second":"<b64url>"}}`. The body is the exact signed bytes, never re-serialised, so there is no canonical-JSON step. Body (UTF-8 JSON, at most 4 KiB, no floats): `serial` (16 hex, random), `kind` (`root|intermediate|person|org|signer`), `subject` (`github_id`, `login`, `name`, for intermediates a label), `namespaces` (prefixes), `usage` (subset of `sign-certs`, `sign-index`, `sign-revocations`, `sign-package`, `sign-plugin`, `sign-extension`), `keys` (`ed25519`, and `ml-dsa-65` or for roots `slh-dsa-sha2-128s`, base64url), `issuer` (issuer serial), `not_before`, `not_after` (Unix seconds), `level` (`wolos|verified`).
- What is signed: every signature in this record is over `label || 0x00 || SHA-512(content)`, with one label per purpose: `wolos/cert/1`, `wolos/package/1`, `wolos/plugin/1`, `wolos/extension/1`, `wolos/index/1`, `wolos/revocation/1`. The label stops a signature made for one purpose from being replayed as another; hashing first keeps the signed message under 100 bytes, which a remote signing service accepts unchanged (Key ceremony). The second algorithm's own context string stays empty for the same reason.
- **Hybrid, both must verify.** A signature is valid only when the Ed25519 half and the second half both verify. There is no "either" mode and no downgrade: the format number fixes the pair (Ed25519 + ML-DSA-65 below the root; Ed25519 + SLH-DSA-SHA2-128s for the roots). A new pair is a new format.
- **In a `.wol`** (format 2 of ADR 0011): after the existing members come two optional members, `CHAIN` (JSON array of `.wolcert`, leaf first, at most 4 certificates and 64 KiB, root never included) and `SHA256.wsig` (`{"cert":"<leaf serial>","ed25519":"..","second":".."}` over the `SHA256` member with label `wolos/package/1`). `SHA256` keeps hashing the payload members only, so a renewed chain does not change the package hash. The legacy `SHA256.sig` stays for WolOS's own keys. `.wolplug` and extensions take the same two members with their labels. A 0.12 system refuses the new members ("unexpected member"), so format-2 files declare `min_wolos` 0.13.

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
- **Verified author**: "Made by Alice Doe (alice on GitHub). Verified author since March 2026." For an organisation: "Made by Example Labs (examplelabs on GitHub). Verified organisation." When the list is stale, the extra line above.
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
2. The remote signer returns ML-DSA in another encoding (wrapped, pre-hashed, or with a context). TRUST-core's first task is a spike: sign through the service, verify with the crate. If they differ, intermediates fall back to keys in a signing-only CI environment, as release keys are today.
3. Two algorithms double the parsing and key handling; more code is more bugs. Mitigation: one pure crate, no I/O, bounded inputs, used by the OS, the hub Worker and the browser alike, so there is one implementation to get right.
4. Sizes grow. About 25 KB per signed package (chain and signature in base64); irrelevant next to payloads and inside the 2 MiB `.wolplug` cap. Leaf certificates do not go into the index, only intermediates do.
5. Nobody runs the ceremony and the test root ships. The image build refuses a test root (TRUST-core test), and no index is signed before L1-L3.
6. A future standard change or a better pair. The format number carries the pair; format 2 can add one without breaking format 1.

### Rust crates (dependency table; names appear only here)
| Crate | Version | Use | Notes |
|---|---|---|---|
| ed25519-dalek | 2.2.0 | Ed25519 (in tree) | `verify_strict` as today |
| fips204 | 0.5.0 | ML-DSA-65, favourite | pure Rust, `no_std`, no heap; README claims constant-time signing and a WASM build; MIT OR Apache-2.0; no independent audit named |
| ml-dsa | 0.1.2 | second ML-DSA, tests only | pure Rust; README: "never been independently audited"; Apache-2.0 OR MIT |
| fips205 | 0.5.0 | SLH-DSA-SHA2-128s (roots) | pure Rust, `no_std`; MIT OR Apache-2.0 |
| slh-dsa | 0.2.0-rc.5 | none | pre-release |
| aws-lc-rs | 1.16.3 | none | ML-DSA only behind its `unstable` feature, C and assembly, no wasm |

All four candidates built here with rust 1.99.1 in 28 s (scratch crate, `/tmp/hubarch-bench`, outside every repo). Adding them to the workspace is a CTO decision (`deny.toml`).

### Key ceremony and storage (no hardware module bought)
- **Roots A and B**: generated at a ceremony on a computer with no network, booted from a WolOS live medium, with the lead present. Each private key is written encrypted with a passphrase to two USB drives, and the seeds are written on paper; A and B are kept in different places. Roots sign about once every three years and at an emergency. The public halves go into the image.
- **Intermediates**: non-exportable keys in the AWS key service of the build account (the installed CLI, 2.39.3, lists `ECC_NIST_EDWARDS25519` and `ML_DSA_65` key specs; no key was created for this record). The root signs each intermediate's certificate at the ceremony. Only one role may call `Sign`; it trusts only the hub's signing workflow in its own GitHub environment, pinned by workflow commit (the pattern of the release signing workflows). The Authors CA environment also needs a human approval (L4). Every signature is logged by the service.
- **WolOS's own packages**: the Packages CA signer replaces the flat keys over time; until then, `SHA256.sig` with today's keys stays valid next to it.
- **Leak**: of an author key, revoke the leaf (`key-compromise`); the next list goes out at once. Of an intermediate, root A revokes it, a new one is made at an emergency ceremony, leaves are re-issued (Repair). Of root A, root B revokes it and the next image drops A.

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

## Consequences
- `.wol`, `.wolplug` and extension format 2 needs WolOS 0.13; 0.12 keeps working with today's keys and refuses format-2 files with a sentence.
- One pure crate holds formats and checks, without I/O; `wol_core`, wol-admind, the Store, the hub Worker (wasm) and the browser (wasm) share it.
- No online status check is ever made when a file is opened: the list comes with the index, so opening an app tells nobody anything.
- New CLI: `wol cert request` (keys made and kept on the author's machine, a request with a proof of possession), `wol cert show`, `wol sign <file>`. The hub never sees a private key.

## Lead decisions
- L1: approve scheme B (hybrid, SLH-DSA roots).
- L2: the root ceremony (who attends, where A and B are kept, a date).
- L3: intermediates in the AWS key service (about 6 keys at about $1 a month each, from credits).
- L4: who approves certificate issuance (two people, or the lead alone).
- L5: a revoked certificate on an installed app: ask at launch (recommended) or block launch.
- L6: the stale window (30 days recommended) and list lifetime (7 days).
- L7: what a person must show for a certificate (GitHub two-factor on, account at least 90 days old, a listed release that passed its checks, CLA signed: recommended), and whether any human identity check is added.
- L8: certificate lifetimes (person and organisation 1 year, intermediates 3, roots 20).
- L9: the sheet's words (above).

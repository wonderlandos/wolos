# wolcrypt: every cryptographic operation in one crate (ADR 0046, lead decision 2026-11-04 "WOLCRYPT")

Decision record: ADR 0046 (lead decision 2026-11-04 "WOLCRYPT"). Code: `crates/wolcrypt` (BSD-2-Clause), the one crate every other crate calls for cryptography.

**Concept.** WolOS signs, verifies, seals, opens, hashes, derives keys and names things by hash in
one place: `crates/wolcrypt` (BSD-2-Clause, like libwol). The trust formats (the `woltrust` crate of the trust-core repository), the
system, the hub Worker (wasm), the browser and C programs all call it.

**Why.** Before, `woltrust` called five crypto crates directly and ring for ChaCha20-Poly1305 and
PBKDF2, and the hub's wasm build had to leave sealing out because ring is not pure Rust. One small
crate with one API is easier to review, to fuzz and to swap a primitive in, and it builds for wasm.

**How.** A std crate written no_std-friendly (only `core` and the `alloc` types; a `no_std` build
is `#![no_std]` and `extern crate alloc` away, which rule RR-044 forbids today), `#![forbid(unsafe_code)]`, no I/O, no clock, no random
generator: callers pass fresh random bytes. Pure-Rust dependencies only, no ring: ed25519-dalek,
fips204 (ML-DSA-65), fips205 (SLH-DSA-SHA2-128s), fips203 (ML-KEM-768), curve25519-dalek (X25519),
sha2, chacha20poly1305 0.10.1 and pbkdf2 0.12.2 (RustCrypto, MIT/Apache-2.0, new with this crate),
subtle, zeroize, serde/serde_json (alloc only, for the detached check).

| Module | What |
|---|---|
| `hash` | SHA-512, SHA-512/256 (FIPS 180-4), SHA-256; lowercase hex |
| `id` | derived ids: `of_keys` (SHA-512 over `ed25519 \|\| ml-dsa-65 \|\| x25519 \|\| ml-kem-768`; roots `ed25519 \|\| slh-dsa-sha2-128s`; absent keys omitted), `of_file` (SHA-512 of the `SHA256` member), `of_ed25519` |
| `sign` | `SigningKey::from_seed(seed, Scheme::Signing \| Scheme::Root)`, `sign(label, content) -> Result<Sig, Fail>` (a post-quantum half that cannot be made is `Fail::Second`, never an empty half), `verify(keys, label, content, sig)`: hybrid, both halves, label-bound; no classical-only call |
| `kem` | `EncSecret::from_seed`, `seal(format, to, prefix, plaintext, rnd)`, `open(format, keys, sealed, prefix_len)`: X25519 + ML-KEM-768, bound key schedule, ChaCha20-Poly1305 |
| `aead` | ChaCha20-Poly1305 seal/open with associated data (RFC 8439) |
| `kdf` | `pbkdf2_sha512(pass, salt, rounds)`: PBKDF2-HMAC-SHA512, 210 000 to 10 000 000 rounds, new files 600 000 |
| `tls` (feature `tls`, ADR 0052) | `X25519MlKem768 { rng }`: a rustls `SupportedKxGroup` for the hybrid TLS 1.3 group X25519MLKEM768 (0x11EC, draft-ietf-tls-ecdhe-mlkem), client side. Shares are ML-KEM-768 first (client 1184 + 32 bytes, server 1088 + 32), secret ML-KEM (32) then X25519 (32); built from the `kem` X25519 and fips203; an all-zero X25519 result or a malformed share is refused. Randomness comes from the caller's `rng` (the TLS provider's), never from wolcrypt. The only module that depends on rustls (the trait crate, no provider), so off by default and absent from the wasm and C builds. Use: `provider.kx_groups.insert(0, &X25519MLKEM768)` on any rustls provider. Tests: independent-server agreement, known answer, malformed and low-order shares, and `crates/federate/tests/live.rs` (negotiates the group with federate.network) |
| `ct` | constant-time equality (subtle) |
| `detached` | a witness's detached signature on a published file, checked with nothing but wolcrypt |
| `text` | the UTC date every signature line uses |

Key derivation from the one 32-byte seed keeps the labels the first keys were made with:
`derive(seed, what) = SHA-512("woltrust/" || what || 0x00 || seed)` for `ed25519`, `ml-dsa-65`,
`slh-dsa-sha2-128s`, `x25519`, `ml-kem-768`; ML-DSA-65 signs with 32 bytes of
`SHA-512("woltrust/rnd" 0x00 || seed || msg)`, SLH-DSA unhedged. So every key and every vector made
before the move derives and signs to the same bytes (woltrust's `vectors/` unchanged;
`vectors/wolkey/ring-made.wolkey`, written with ring, opens and re-locks byte for byte:
woltrust's `tests/wolkey_compat.rs`).

Secret types wipe themselves: `SigningKey` (seed and Ed25519 key; the post-quantum secret is derived
per signature and wiped), `EncSecret` (X25519 scalar and ML-KEM decapsulation key), every
intermediate secret in `Zeroizing`: the three SLH-DSA seeds, the ML-KEM `d`/`z` seeds and
encapsulation message, the AEAD key and nonce, and in the C ABI each seed and `rnd` copied out of
the caller's buffer (TRUST audit 2 L3; `wolcrypt::Zeroizing` is re-exported for that).

## Ids: why they cannot be forged or collide

An id is SHA-512 over public keys, 64 bytes, written as 128 lowercase hex. Anyone can compute it;
only the holder of the private keys can sign with them, and only an authority's certificate makes the
id trusted. To pass off other keys under a given id is a second preimage of SHA-512: about 2^512
operations classically, about 2^256 with Grover's algorithm. Two different key sets with one id is a
collision: about 2^256 classically (BHT would trade it to about 2^171 quantum operations with as much
memory, which no machine has; 2^256 is the planning figure). The parts have fixed lengths and each key
set has its own total length (64 bytes for a root, 1984 for signing keys, 3200 with encryption keys,
32 for a bare Ed25519 key), so no concatenation of one kind equals another's. The id covers the public
keys only, never private material: an id must be checkable by everyone.

## The C ABI (libwolcrypt.so.1)

`wolcrypt-ffi` (BSD-2-Clause, a crate of the trust-core repository) is the pointer boundary: `include/wolcrypt.h`, symbol versions
in `abi/wolcrypt.map` (node `WOLCRYPT_1.0`, frozen once shipped; new symbols go in a new node),
`abi/wolcrypt-1.0.symbols`, `link.sh` (from the staticlib, as libwolkit). Thirteen functions:
`wolcrypt_version`, `_sha512`, `_sha512_256`, `_id`, `_public`, `_sign`, `_verify`, `_kem_public`,
`_kem_seal`, `_kem_open`, `_pbkdf2_sha512`, `_ct_eq`, `_verify_detached`. Status 0 success, 1 does not
verify, negative a bad argument; nothing is allocated for the caller. `tests/abi.rs` checks the map,
the symbols file, the header and the source name the same set, round-trips every call through raw
pointers, and links the library and compares `nm -D` with the symbols file. The unsafe count lives in
that crate only (`unsafe_ratchet`: wolcrypt 0, wolcrypt-ffi 74).

## wasm and the command line

`cargo build -p wolcrypt --target wasm32-unknown-unknown` builds (checked on the CTO box, rust
1.99.1). `wolcrypt verify-detached FILE FILE.wsig WITNESS.wolcert ROOT_A.wolcert ROOT_B.wolcert`
prints the witness line and exits 0 when the signature verifies (the same line `woltrust witness
verify` prints: woltrust's `tests/witness.rs`). `wolcrypt sha512 FILE` prints the hash.

## Vectors

Per operation, in the unit tests: FIPS 180-4 `abc` for SHA-512, SHA-512/256 and SHA-256; RFC 8439
section 2.8.2 for ChaCha20-Poly1305; PBKDF2-HMAC-SHA512 known answers for 1 and 2 rounds; RFC 7748
section 6.1 for X25519; signatures deterministic per scheme with each half's failure named; ids in
key order; KEM seal/open with every byte bound. The format vectors in woltrust's `vectors/`
exercise all of it end to end.

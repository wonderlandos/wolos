# Federate Network: a proposal for post-quantum signatures (draft, not sent)

WolOS reads the Federate Network natively (ADR 0052). This is the text we would open as an issue on
https://github.com/federatenetwork/federate when the lead decides to; nothing has been posted.

## Issue text

**Title:** Optional ML-DSA-65 signatures beside Ed25519, announced by the root zone

Every layer of the chain of trust (root zone, TLD records, TLD registries, domain records,
manifests) is signed with Ed25519 (`signatures.md` in the upstream repository's English docs). A cryptographically relevant
quantum computer would forge Ed25519 signatures, and a zone forged under the root key would let it
rewrite every name. NIST's post-quantum signature standard is final (FIPS 204, ML-DSA, August 2024)
and NIST IR 8547 plans to deprecate classical signatures in 2030. Content addresses (BLAKE3, 256
bits) already keep a 128-bit margin and need no change.

Proposal, backward compatible, in three steps:

1. **Keys.** Each signer (root, TLD operator, domain owner) gets an ML-DSA-65 key beside its
   Ed25519 key. Records that name a signer key gain the matching field:
   `root_public_key_pq`, `operator_public_key_pq`, `owner_public_key_pq` (hex of the 1952-byte
   ML-DSA-65 public key). The Ed25519 fields keep their meaning.
2. **Signatures.** Every signed object may carry `signature_pq` (hex of the 3309-byte ML-DSA-65
   signature, FIPS 204 `ML-DSA.Sign` with an empty context string, hedged) over the same canonical
   bytes as today's Ed25519 signature, computed with BOTH `signature` and `signature_pq` set to
   null. `signature_algorithm` stays `"ed25519"`; a new `signature_algorithm_pq: "ml-dsa-65"`
   names the second one. A client that does not know the fields ignores them (they are inside the
   signed bytes, so they cannot be stripped without breaking the Ed25519 signature).
3. **Announcement.** The root zone gains `pq_required: true` once every TLD record, every
   root-managed domain record and every manifest it points to carries `signature_pq`. From the
   first zone a client has verified with `pq_required: true`, it requires both signatures on every
   layer, refuses any later zone without the flag (rollback), and pins the root's ML-DSA key the way
   it pins the Ed25519 one. Before that, a present `signature_pq` is verified and a wrong one is a
   failure, never ignored.

Both signatures must verify (hybrid AND): a break of either scheme alone forges nothing. Clients
verify ML-DSA with FIPS 204 `ML-DSA.Verify`. Reference crates exist in Rust (`fips204`,
`ml-dsa`). Key rollover for the root (planned in signatures.md) can introduce the PQ root key: the
transition record is signed by the old Ed25519 root key and the new ML-DSA key.

Size: about 3.3 kB more per signed object; the live root zone (22 kB, 23 TLD records, 3 names)
would grow by about 90 kB, manifests by 3.3 kB each.

WolOS would ship verification on the day the fields are specified, and the test vectors we
use (the live zone of 2026-11-04) are available on request.

## What WolOS does meanwhile

See ADR 0052, "Post-quantum status": the verifier is classical because the protocol is; the
transport and local state are hardened where WolOS controls both ends.

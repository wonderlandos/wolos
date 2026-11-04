# KMS spike: can the key service sign what our code verifies?

**Concept.** ADR 0046 wants the intermediates (Authors CA, Packages CA, Index) to be keys that never
leave AWS KMS. A KMS signature must verify with the `woltrust` crate, byte for byte, with no wrapper.
This page records what the installed CLI says (read-only, nothing was created) and gives the exact
test to run once the lead approves creating a key.

**Why.** Premortem 2 of ADR 0046: if the service wraps, pre-hashes or adds a context to ML-DSA, the
intermediates would have to fall back to keys held in a signing-only CI environment. Better to find
out with one throw-away key than after the ceremony.

## What the CLI says (checked 2026-11-04, aws-cli 2.39.3, no key created, no call made)

- `aws kms create-key --key-spec` lists `ML_DSA_44`, `ML_DSA_65`, `ML_DSA_87` and
  `ECC_NIST_EDWARDS25519` (Ed25519). The help example creates an `ML_DSA_65` key with
  `--key-usage SIGN_VERIFY`.
- `aws kms sign --signing-algorithm` offers `ML_DSA_SHAKE_256` and, for Edwards keys, `ED25519_SHA_512`
  and `ED25519_PH_SHA_512`.
- `--message` is 1 to 4096 bytes. Our signed message is `label || 0x00 || SHA-512(content)`, at most
  about 80 bytes, so it is sent whole; the service never sees the package.
- `--message-type`:
  - `RAW` (the default): the service hashes as part of the algorithm. This is the mode to use.
  - `DIGEST`: the message is already a digest. Do not use for ML-DSA.
  - `EXTERNAL_MU`: a 64-byte ML-DSA representative (FIPS 204 section 6.2). Fallback only.
  - For Edwards keys the help says `ED25519_SHA_512` requires `RAW` (that is pure Ed25519, which is
    what `verify_strict` checks) and `ED25519_PH_SHA_512` requires `DIGEST` (a different algorithm,
    Ed25519ph; never use it, our verifier does not accept it).
- The Sign call has no context parameter, so the ML-DSA context is the empty string, which is what
  `woltrust` verifies with.
- `aws kms get-public-key` returns a DER SubjectPublicKeyInfo; the raw key is the last 1952 bytes
  (ML-DSA-65) or the last 32 bytes (Ed25519).

Not answered by the help text, and the reason for the test below: whether the 3309-byte signature
that comes back for `RAW` is the plain FIPS 204 signature over the message with an empty context,
with no extra wrapping. The help says only that RAW "begins with a hash function".

## The exact test (needs one key; costs about one dollar a month, so the lead decides)

Keys: one `ML_DSA_65` and one `ECC_NIST_EDWARDS25519`, both `SIGN_VERIFY`, scheduled for deletion
(7 days) right after the test.

```sh
export AWS_REGION=<region>
ML=$(aws kms create-key --key-spec ML_DSA_65 --key-usage SIGN_VERIFY --query KeyMetadata.KeyId --output text)
ED=$(aws kms create-key --key-spec ECC_NIST_EDWARDS25519 --key-usage SIGN_VERIFY --query KeyMetadata.KeyId --output text)
D=$(mktemp -d)
# msg.bin = "wolos/package/1" || 0x00 || SHA-512("spike")
printf 'wolos/package/1\0' > $D/msg.bin; printf 'spike' | openssl dgst -sha512 -binary >> $D/msg.bin
aws kms sign --key-id $ML --message fileb://$D/msg.bin --message-type RAW --signing-algorithm ML_DSA_SHAKE_256 \
  --query Signature --output text | base64 -d > $D/ml.sig
aws kms sign --key-id $ED --message fileb://$D/msg.bin --message-type RAW --signing-algorithm ED25519_SHA_512 \
  --query Signature --output text | base64 -d > $D/ed.sig
aws kms get-public-key --key-id $ML --query PublicKey --output text | base64 -d > $D/ml.pub.der
aws kms get-public-key --key-id $ED --query PublicKey --output text | base64 -d > $D/ed.pub.der
WOLTRUST_KMS_DIR=$D cargo test -p woltrust --test kms -- --ignored --nocapture
aws kms schedule-key-deletion --key-id $ML --pending-window-in-days 7
aws kms schedule-key-deletion --key-id $ED --pending-window-in-days 7
```

The test (`crates/woltrust/tests/kms.rs`) passes when the signature is exactly 3309 bytes, verifies with
the workspace's ML-DSA-65 code with an empty context, and the 64-byte Ed25519 signature verifies with
`verify_strict`.

Results and what follows:

| Outcome | Meaning | Next step |
|---|---|---|
| Both pass | KMS signs exactly what we verify | Intermediates go in KMS (L3) |
| ML-DSA length is not 3309, or fails | The service wraps or adds context | Retry with `EXTERNAL_MU`: compute mu per FIPS 204 section 6.2 from the public key hash and the message, send the 64 bytes, verify with the same test |
| Still fails | Not interoperable today | Fall back to signing-only CI keys, as release keys are now |

Status: NOT RUN. No key exists. Ask the lead (L3) before creating one.

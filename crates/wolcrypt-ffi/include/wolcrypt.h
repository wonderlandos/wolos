/* SPDX-License-Identifier: BSD-2-Clause */
/* libwolcrypt 1.0: WolOS's cryptography behind a C ABI (crate wolcrypt-ffi, docs/modules/wolcrypt.md).
 * Status: 0 success, 1 does not verify / not equal, negative a bad argument (-1 NULL, -2 length,
 * -3 output buffer too small: *out_len holds the size needed, -4 bad key). Nothing is allocated
 * for the caller; every output goes into a buffer the caller sized. Signatures are hybrid and
 * label-bound: label || 0x00 || SHA-512(content), both halves always (no classical-only call). */
#ifndef WOLCRYPT_H
#define WOLCRYPT_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

#define WOLCRYPT_OK 0
#define WOLCRYPT_NO 1
#define WOLCRYPT_ENULL (-1)
#define WOLCRYPT_ELEN (-2)
#define WOLCRYPT_ESPACE (-3)
#define WOLCRYPT_EKEY (-4)

/* major << 16 | minor. */
uint32_t wolcrypt_version(void);
/* SHA-512 into out[64]; SHA-512/256 into out[32]. */
int wolcrypt_sha512(const uint8_t *data, size_t len, uint8_t *out);
int wolcrypt_sha512_256(const uint8_t *data, size_t len, uint8_t *out);
/* Id of a key set: SHA-512 over the keys in id order (32, 64, 1984 or 3200 bytes) into out[64]. */
int wolcrypt_id(const uint8_t *keys, size_t len, uint8_t *out);
/* Public signing keys of seed[32]: Ed25519 then ML-DSA-65 (root == 0) or SLH-DSA-SHA2-128s. */
int wolcrypt_public(const uint8_t *seed, int root, uint8_t *out, size_t cap, size_t *out_len);
/* Hybrid signature (Ed25519 64 bytes, then the post-quantum half) over label || 0 || SHA-512(content). */
int wolcrypt_sign(const uint8_t *seed, int root, const char *label, const uint8_t *content,
                  size_t len, uint8_t *out, size_t cap, size_t *out_len);
/* 0 when both halves verify with keys (as wolcrypt_public writes them), 1 when not. */
int wolcrypt_verify(const uint8_t *keys, size_t keys_len, const char *label, const uint8_t *content,
                    size_t len, const uint8_t *sig, size_t sig_len);
/* Encryption keys of seed[32]: X25519 then ML-KEM-768 into out[1216]. */
int wolcrypt_kem_public(const uint8_t *seed, uint8_t *out);
/* Hybrid KEM + ChaCha20-Poly1305: prefix || ephemeral(32) || ML-KEM ct(1088) || aead || tag(16). */
int wolcrypt_kem_seal(const char *format, const uint8_t *to, const uint8_t *prefix, size_t prefix_len,
                      const uint8_t *plaintext, size_t len, const uint8_t *rnd, uint8_t *out,
                      size_t cap, size_t *out_len);
int wolcrypt_kem_open(const char *format, const uint8_t *seed, const uint8_t *sealed, size_t len,
                      size_t prefix_len, uint8_t *out, size_t cap, size_t *out_len);
/* PBKDF2-HMAC-SHA512, 210000 to 10000000 rounds, into out[32]. */
int wolcrypt_pbkdf2_sha512(const uint8_t *pass, size_t pass_len, const uint8_t *salt, size_t salt_len,
                           uint32_t rounds, uint8_t *out);
/* Constant-time comparison: 0 equal, 1 not. */
int wolcrypt_ct_eq(const uint8_t *a, const uint8_t *b, size_t len);
/* A witness's detached signature on a published file: 0 and the signature line (no NUL). */
int wolcrypt_verify_detached(const uint8_t *file, size_t file_len, const uint8_t *wsig, size_t wsig_len,
                             const uint8_t *witness, size_t witness_len, const uint8_t *root_a,
                             size_t root_a_len, const uint8_t *root_b, size_t root_b_len, uint8_t *line,
                             size_t cap, size_t *line_len);

#ifdef __cplusplus
}
#endif
#endif

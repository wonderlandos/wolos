/* SPDX-License-Identifier: GPL-2.0-or-later */
/* libxguard: scan a file or bytes, and what is packed inside, in memory (ADR 0022).
 * Results are JSON documents; new fields are only added:
 * {"verdict":"clean|found|incomplete","findings":[{"name","how","path"}],"skipped":[{"path","why"}],
 *  "files":n,"bytes":n,"sha256":"hex","signatures_built":unix,"signatures_age_days":n|null,
 *  "signatures_versions":[n]}
 * "incomplete": nothing known was found, but part of the input could not be read. */
#ifndef XGUARD_H
#define XGUARD_H
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define XGUARD_OK 0
#define XGUARD_INVALID_ARGUMENT 1
#define XGUARD_UNAVAILABLE 3
#define XGUARD_INTERNAL 4

/* Run the signature rules too: spawns wol-xguard; without it, hashes and patterns only. */
#define XGUARD_SCAN_DEEP 1u

typedef struct xguard_engine xguard_engine;

uint32_t xguard_abi_version(void);
int32_t xguard_engine_create(const char *dir, size_t dir_len, xguard_engine **out);
void xguard_engine_release(xguard_engine *e);
int32_t xguard_scan_path(const xguard_engine *e, const char *path, size_t path_len, uint32_t flags,
                         char **json, size_t *json_len);
int32_t xguard_scan_memory(const xguard_engine *e, const uint8_t *data, size_t len, const char *name,
                           size_t name_len, char **json, size_t *json_len);
void xguard_string_release(char *s, size_t len);

#ifdef __cplusplus
}
#endif
#endif

/* SPDX-License-Identifier: BSD-2-Clause */
/* libwopkg: WolOS's package core (ADR 0021, archicture/modules/wopkg.md).
 * Rules (ADR 0020): handles are opaque; strings are UTF-8 with a length; every call that can fail
 * returns a wopkg_status and, if `error` is not NULL, a wopkg_error the caller releases; results
 * are wopkg_string JSON documents the caller releases; progress callbacks run on the calling
 * thread, during the call. New fields are only ever added to requests and results. */
#ifndef WOPKG_H
#define WOPKG_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define WOPKG_VERSION_MAJOR 1
#define WOPKG_VERSION_MINOR 0
#define WOPKG_VERSION_PATCH 0

typedef int32_t wopkg_status;
#define WOPKG_OK 0
#define WOPKG_ERR_INVALID_ARGUMENT 1
#define WOPKG_ERR_PERMISSION_DENIED 2
#define WOPKG_ERR_UNAVAILABLE 3
#define WOPKG_ERR_INTERNAL 4
#define WOPKG_ERR_UNTRUSTED 5
#define WOPKG_ERR_NOT_FOUND 6

typedef struct wopkg_context wopkg_context;
typedef struct wopkg_error wopkg_error;
typedef struct wopkg_string wopkg_string;

/* One JSON progress event (`{"event":"bytes",...}`), valid only during the callback. */
typedef void (*wopkg_progress_fn)(const char *json, size_t len, void *user_data);

uint32_t wopkg_version(void);
const char *wopkg_version_string(void);

const char *wopkg_string_get_bytes(const wopkg_string *string, size_t *len);
void wopkg_string_release(wopkg_string *string);

wopkg_status wopkg_error_get_code(const wopkg_error *error);
const char *wopkg_error_get_message(const wopkg_error *error, size_t *len);
void wopkg_error_release(wopkg_error *error);

/* config: JSON (`{}` or NULL for the person's defaults). */
wopkg_status wopkg_context_create(const char *config, size_t len, wopkg_context **out, wopkg_error **error);
void wopkg_context_release(wopkg_context *context);

/* Index */
wopkg_status wopkg_index_update(const wopkg_context *context, const char *request, size_t len, wopkg_progress_fn progress, void *user_data, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_search(const wopkg_context *context, const char *query, size_t len, uint32_t limit, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_info(const wopkg_context *context, const char *name, size_t len, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_complete(const wopkg_context *context, const char *prefix, size_t len, uint32_t limit, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_installed(const wopkg_context *context, wopkg_string **out, wopkg_error **error);

/* Downloads and trust */
wopkg_status wopkg_fetch(const wopkg_context *context, const char *request, size_t len, wopkg_progress_fn progress, void *user_data, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_verify(const wopkg_context *context, const char *request, size_t len, wopkg_string **out, wopkg_error **error);

/* AppImages */
wopkg_status wopkg_appimage_inspect(const wopkg_context *context, const char *path, size_t len, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_appimage_install(const wopkg_context *context, const char *request, size_t len, wopkg_progress_fn progress, void *user_data, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_appimage_remove(const wopkg_context *context, const char *name, size_t len, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_appimage_update(const wopkg_context *context, const char *request, size_t len, wopkg_progress_fn progress, void *user_data, wopkg_string **out, wopkg_error **error);

/* History */
wopkg_status wopkg_history(const wopkg_context *context, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_history_record(const wopkg_context *context, const char *entry, size_t len, wopkg_string **out, wopkg_error **error);
wopkg_status wopkg_rollback(const wopkg_context *context, const char *id, size_t len, wopkg_string **out, wopkg_error **error);

/* System (through wold) and releases */
wopkg_status wopkg_system_call(const wopkg_context *context, const char *request, size_t len, wopkg_error **error);
wopkg_status wopkg_release_plan(const wopkg_context *context, const char *request, size_t len, wopkg_string **out, wopkg_error **error);

#ifdef __cplusplus
}
#endif

#endif

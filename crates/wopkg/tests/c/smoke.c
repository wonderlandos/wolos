/* SPDX-License-Identifier: BSD-2-Clause */
/* smoke: libwopkg from C (ADR 0021). Built and run by tests/c_smoke.rs with argv[1] = a config
 * JSON and argv[2] = an index update request JSON. Prints one line per step. */
#include <stdio.h>
#include <string.h>

#include "wopkg.h"

#define S(x) (x), strlen(x)

static int events = 0;

static void on_progress(const char *json, size_t len, void *user_data) {
    (void)json;
    (void)len;
    int *n = user_data;
    ++*n;
}

static void print(const char *label, wopkg_string *s) {
    size_t n = 0;
    const char *b = wopkg_string_get_bytes(s, &n);
    printf("%s: %.*s\n", label, (int)n, b);
    wopkg_string_release(s);
}

static int fail(const char *what, wopkg_error *e) {
    size_t n = 0;
    const char *m = wopkg_error_get_message(e, &n);
    printf("%s failed: %d %.*s\n", what, (int)wopkg_error_get_code(e), (int)n, m);
    wopkg_error_release(e);
    return 1;
}

int main(int argc, char **argv) {
    if (argc != 3) return 2;
    printf("version: %s %u\n", wopkg_version_string(), wopkg_version());
    wopkg_context *ctx = NULL;
    wopkg_error *err = NULL;
    if (wopkg_context_create(S(argv[1]), &ctx, &err) != WOPKG_OK) return fail("context", err);
    wopkg_string *out = NULL;
    if (wopkg_index_update(ctx, S(argv[2]), on_progress, &events, &out, &err) != WOPKG_OK) return fail("update", err);
    print("update", out);
    printf("events: %d\n", events);
    if (wopkg_search(ctx, S("firefox"), 5, &out, &err) != WOPKG_OK) return fail("search", err);
    print("search", out);
    if (wopkg_complete(ctx, S("fi"), 10, &out, &err) != WOPKG_OK) return fail("complete", err);
    print("complete", out);
    wopkg_status st = wopkg_info(ctx, S("no-such-app"), &out, &err);
    printf("info status: %d\n", (int)st);
    fail("info", err);
    st = wopkg_search(NULL, S("x"), 5, &out, NULL);
    printf("null context: %d\n", (int)st);
    wopkg_context_release(ctx);
    wopkg_context_release(NULL);
    wopkg_error_release(NULL);
    return 0;
}

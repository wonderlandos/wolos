/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* prefs: libwol 1.4 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static const char *text(const wol_string *s) { return s ? wol_string_get_bytes(s, NULL) : "null"; }

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.prefs"), &ctx, &err) != WOL_OK)
        return 1;
    if (wol_prefs_set_string(ctx, S("width"), S("640"), &err) != WOL_OK)
        return 1;
    wol_string *s = NULL;
    if (wol_prefs_copy_string(ctx, S("width"), &s, &err) != WOL_OK)
        return 1;
    printf("width: %s\n", text(s));
    wol_string_release(s);
    s = NULL;
    if (wol_prefs_copy_string(ctx, S("nothing"), &s, &err) != WOL_OK)
        return 1;
    printf("missing: %s\n", text(s));
    wol_info *keys = NULL;
    if (wol_prefs_copy_keys(ctx, &keys, &err) != WOL_OK)
        return 1;
    printf("keys: %zu %s %s %s\n", wol_info_get_count(keys),
           text(wol_info_get_text(wol_info_get_item(keys, 0))),
           text(wol_info_get_text(wol_info_get_item(keys, 1))), text(wol_info_get_json(keys)));
    wol_info_release(keys);
    if (wol_prefs_remove(ctx, S("width"), &err) != WOL_OK)
        return 1;

    wol_status st = wol_prefs_set_string(ctx, S("a/b"), S("x"), &err);
    printf("bad key: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;
    static char big[4097];
    memset(big, 'x', sizeof big);
    st = wol_prefs_set_string(ctx, S("k"), big, sizeof big, &err);
    printf("long value: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

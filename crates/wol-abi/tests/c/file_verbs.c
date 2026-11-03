/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* file_verbs: libwol 1.15 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.files"), &ctx, &err) != WOL_OK)
        return 1;
    wol_string *s = NULL;
    if (wol_file_trash(ctx, S("/home/a/a.jpg"), &s, &err) != WOL_OK)
        return 2;
    printf("trashed: %s\n", wol_string_get_bytes(s, NULL));
    wol_string_release(s);
    s = NULL;
    if (wol_file_trash(ctx, S("/media/x/away.jpg"), &s, &err) != WOL_OK)
        return 3;
    printf("other drive: %s\n", s ? "named" : "null");
    if (wol_share_file(ctx, S("/home/a/a.jpg"), &s, &err) != WOL_OK)
        return 4;
    printf("shared: %s\n", wol_string_get_bytes(s, NULL));
    wol_string_release(s);
    wol_status st = wol_open_with(ctx, S("/home/a/a.jpg"), &s, &err);
    printf("open with: %d %s\n", (int)st, wol_error_get_message(err, NULL));
    wol_error_release(err);
    err = NULL;
    st = wol_file_trash(ctx, S("a.jpg"), &s, &err);
    printf("relative: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

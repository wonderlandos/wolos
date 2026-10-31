/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* share: libwol 1.13 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.share"), &ctx, &err) != WOL_OK)
        return 1;
    wol_string *done = NULL;
    if (wol_share_text(ctx, S("A note"), S("two\nlines"), &done, &err) != WOL_OK)
        return 2;
    printf("done: %s\n", wol_string_get_bytes(done, NULL));
    wol_string_release(done);
    wol_status st = wol_share_text(ctx, S(""), S("x"), &done, &err);
    printf("no title: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

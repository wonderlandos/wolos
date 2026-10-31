/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* service: libwol 1.14 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.service"), &ctx, &err) != WOL_OK)
        return 1;
    if (wol_service_start(ctx, S("sync"), &err) != WOL_OK)
        return 2;
    wol_info *i = NULL;
    if (wol_service_copy_status(ctx, S("sync"), &i, &err) != WOL_OK)
        return 3;
    printf("state: %s\n", wol_string_get_bytes(wol_info_get_text(wol_info_get(i, S("state"))), NULL));
    wol_info_release(i);
    if (wol_service_stop(ctx, S("sync"), &err) != WOL_OK)
        return 4;
    wol_status st = wol_service_start(ctx, S("../evil"), &err);
    printf("bad name: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* account: libwol 1.10 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.account"), &ctx, &err) != WOL_OK)
        return 1;
    wol_info *i = NULL;
    if (wol_account_copy_info(ctx, &i, &err) != WOL_OK)
        return 2;
    printf("%s (%s)\n", wol_string_get_bytes(wol_info_get_text(wol_info_get(i, S("full_name"))), NULL),
           wol_string_get_bytes(wol_info_get_text(wol_info_get(i, S("name"))), NULL));
    wol_info_release(i);
    wol_status st = wol_account_copy_info(ctx, NULL, &err);
    printf("null out: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

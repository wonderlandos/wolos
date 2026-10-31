/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* network: libwol 1.7 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.net"), &ctx, &err) != WOL_OK)
        return 1;
    wol_info *i = NULL;
    if (wol_network_copy_info(ctx, &i, &err) != WOL_OK)
        return 2;
    const wol_info *list = wol_info_get(i, S("interfaces"));
    printf("online: %d %zu\n", wol_info_get_bool(wol_info_get(i, S("online"))), wol_info_get_count(list));
    const wol_info *first = wol_info_get_item(list, 0);
    printf("%s %s\n", wol_string_get_bytes(wol_info_get_text(wol_info_get(first, S("name"))), NULL),
           wol_string_get_bytes(wol_info_get_text(wol_info_get(first, S("kind"))), NULL));
    wol_info_release(i);
    wol_status st = wol_network_copy_info(ctx, NULL, &err);
    printf("null out: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

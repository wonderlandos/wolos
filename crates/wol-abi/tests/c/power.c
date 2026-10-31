/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* power: libwol 1.6 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.power"), &ctx, &err) != WOL_OK)
        return 1;
    wol_info *i = NULL;
    if (wol_power_copy_info(ctx, &i, &err) != WOL_OK)
        return 2;
    double percent = 0;
    const wol_info *b = wol_info_get(i, S("battery"));
    int have = wol_info_get_number(wol_info_get(b, S("percent")), &percent);
    printf("battery: %d %.0f %d\n", have, percent, wol_info_get_bool(wol_info_get(i, S("on_battery"))));
    wol_info_release(i);

    wol_assertion *a = NULL;
    if (wol_power_keep_awake(ctx, S("Exporting a video"), &a, &err) != WOL_OK)
        return 3;
    printf("held: %d\n", a != NULL);
    wol_assertion_release(a);

    wol_status st = wol_power_keep_awake(ctx, S(""), &a, &err);
    printf("no reason: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

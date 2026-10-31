/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* audio: libwol 1.8 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.audio"), &ctx, &err) != WOL_OK)
        return 1;
    wol_info *i = NULL;
    if (wol_audio_copy_info(ctx, &i, &err) != WOL_OK)
        return 2;
    double v = 0;
    wol_info_get_number(wol_info_get(i, S("volume")), &v);
    printf("volume: %.0f %d %d\n", v, wol_info_get_bool(wol_info_get(i, S("muted"))),
           wol_info_get(i, S("now_playing")) != NULL);
    wol_info_release(i);
    if (wol_audio_set_volume(ctx, 20, &err) != WOL_OK || wol_audio_set_muted(ctx, 0, &err) != WOL_OK)
        return 3;
    printf("set\n");
    wol_status st = wol_audio_set_volume(ctx, 101, &err);
    printf("too loud: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}

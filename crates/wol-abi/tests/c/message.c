/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* message: libwol 1.11 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static wol_message *kept = NULL;

static void on_message(wol_endpoint *e, wol_message *m, void *ud) {
    (void)e;
    (void)ud;
    printf("got %s from %s\n", wol_string_get_bytes(wol_message_get_body(m), NULL),
           wol_string_get_bytes(wol_message_get_from(m), NULL));
    kept = wol_message_retain(m);
}

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.message"), &ctx, &err) != WOL_OK)
        return 1;
    wol_endpoint *e = NULL;
    if (wol_endpoint_create(ctx, S("greet"), on_message, NULL, &e, &err) != WOL_OK)
        return 2;
    wol_string *reply = NULL;
    if (wol_message_send(ctx, S("/usr/bin/peer"), S("greet"), S("hello"), &reply, &err) != WOL_OK)
        return 3;
    printf("reply: %s\n", wol_string_get_bytes(reply, NULL));
    wol_string_release(reply);
    for (int n = 0; n < 100 && !kept; n++)
        if (wol_context_dispatch(ctx, 100, &err) != WOL_OK)
            return 4;
    if (wol_message_reply(kept, S("pong"), &err) != WOL_OK)
        return 5;
    wol_message_release(kept);
    if (wol_publish(ctx, S("news"), S("item"), &err) != WOL_OK)
        return 6;
    wol_endpoint_release(e);
    wol_context_release(ctx);
    return 0;
}

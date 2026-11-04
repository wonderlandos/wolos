/* SPDX-License-Identifier: BSD-2-Clause */
/* A stand-in for libwolwebengine.so.1 (ABI 1) that does no browsing: it lets tests/shim_loader_*.rs
 * drive crates/webengine's loader and view against the C boundary. Built with `cc -shared -fPIC`;
 * with -DCHANNEL it also has the WWE_1.1 host channel, which echoes each posted message back to the
 * host's callback as "echo:<text>" (the real library hands it to the page). With -DPRESENT it has the
 * WWE_1.3 wwe_view_present: it paints one frame when the view is new or resized and counts each
 * present into <profile_dir>/presented. */
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <stdio.h>

typedef void (*msg_fn)(void *, const uint8_t *, size_t);
typedef struct { msg_fn fn; void *ctx; int set; char dir[512]; uint64_t frames, presents; } engine;
typedef struct { engine *e; } view;
/* The head of wwe_config: what the stand-in reads of it. */
typedef struct { uint32_t abi, flags, features, heap_mb; const char *profile_dir; } config_head;
typedef struct { uint32_t w, h; uint8_t *rgba; } image;

uint32_t wwe_abi_version(void) { return 1; }
const char *wwe_engine_version(void) { return "mock"; }
void *wwe_engine_new(const void *config, char *err, size_t n) {
    static engine e;
    memset(&e, 0, sizeof e);
    (void)err; (void)n;
#ifdef PRESENT
    const config_head *c = config;
    strncpy(e.dir, c->profile_dir ? c->profile_dir : "", sizeof e.dir - 1);
#else
    (void)config;
#endif
    return &e;
}
void wwe_engine_free(void *e) { (void)e; }
#ifdef PRESENT
/* A frame is painted when the view is new or resized (it is dirty), never twice for one change, as the
 * engine's frame counter works: wwe_state.frames counts the frames painted. */
static int dirty;
void wwe_engine_spin(void *e) { if (dirty) { ((engine *)e)->frames++; dirty = 0; } }
#else
void wwe_engine_spin(void *e) { (void)e; }
#endif
void *wwe_view_new(void *e, const void *t, const char *url, char *err, size_t n) {
    static view v;
    (void)t; (void)url; (void)err; (void)n;
    v.e = e;
#ifdef PRESENT
    dirty = 1;
#endif
    return &v;
}
void wwe_view_free(void *v) { (void)v; }
#ifdef PRESENT
void wwe_view_state(const void *v, void *out) {
    memset(out, 0, 16);
    memcpy((uint8_t *)out + 8, &((const view *)v)->e->frames, 8);
}
void wwe_view_resize(void *v, uint32_t w, uint32_t h, float s) { (void)v; (void)w; (void)h; (void)s; dirty = 1; }
/* WWE_1.3: counts the presents into <profile_dir>/presented so the test can read how many the host made. */
void wwe_view_present(void *v) {
    engine *en = ((view *)v)->e;
    char path[600];
    en->presents++;
    snprintf(path, sizeof path, "%s/presented", en->dir);
    FILE *f = fopen(path, "w");
    if (f) { fprintf(f, "%llu frames %llu\n", (unsigned long long)en->presents, (unsigned long long)en->frames); fclose(f); }
}
#else
void wwe_view_state(const void *v, void *out) { (void)v; memset(out, 0, 16); }
void wwe_view_resize(void *v, uint32_t w, uint32_t h, float s) { (void)v; (void)w; (void)h; (void)s; }
#endif
void wwe_view_input(void *v, const void *ev) { (void)v; (void)ev; }
void wwe_view_focus(void *v, bool f) { (void)v; (void)f; }
void wwe_view_visible(void *v, bool f) { (void)v; (void)f; }
void wwe_view_theme(void *v, bool d) { (void)v; (void)d; }
int wwe_view_screenshot(void *v, uint32_t ms, image *out, char *err, size_t n) { (void)v; (void)ms; (void)out; (void)err; (void)n; return -1; }
void wwe_image_free(image *i) { (void)i; }
void wwe_run_content_process(const char *token) { (void)token; }

#ifdef CHANNEL
int wwe_engine_set_host_channel(void *e, const char *origin, const char *init, msg_fn fn, void *ctx, char *err, size_t n) {
    engine *en = e;
    if (strncmp(origin, "file:///", 8) != 0 || !fn || !init) {
        const char *m = "bad origin";
        size_t k = strlen(m) < n - 1 ? strlen(m) : n - 1;
        memcpy(err, m, k);
        err[k] = 0;
        return -1;
    }
    en->fn = fn;
    en->ctx = ctx;
    en->set = 1;
    return 0;
}
int wwe_view_post_message(void *v, const uint8_t *text, size_t len) {
    engine *en = ((view *)v)->e;
    uint8_t buf[256];
    if (!en->set || len > 200) return -1;
    memcpy(buf, "echo:", 5);
    memcpy(buf + 5, text, len);
    en->fn(en->ctx, buf, len + 5);
    return 0;
}
#endif

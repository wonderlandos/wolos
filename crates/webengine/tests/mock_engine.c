/* SPDX-License-Identifier: BSD-2-Clause */
/* A stand-in for libwolwebengine.so.1 (ABI 1) that does no browsing: it lets tests/shim_loader_*.rs
 * drive crates/webengine's loader and view against the C boundary. Built with `cc -shared -fPIC`;
 * with -DCHANNEL it also has the WWE_1.1 host channel, which echoes each posted message back to the
 * host's callback as "echo:<text>" (the real library hands it to the page). With -DSCHEME it also has
 * WWE_1.2, wwe_engine_set_scheme, and two test hooks that do what the real library does with the
 * host's handler: mock_fed_load (one fed: load, the body collected) and mock_fed_map. With -DPRESENT
 * it has the WWE_1.3 wwe_view_present: it paints one frame when the view is new or resized and counts
 * each present into <profile_dir>/presented. */
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>

typedef void (*msg_fn)(void *, const uint8_t *, size_t);
typedef void (*wake_fn)(void *);
/* The first fields of wwe_config (crates/webengine/src/ffi.rs Config), up to the waker and ctx. */
typedef struct {
    uint32_t abi, flags, features, heap_mb;
    const char *profile_dir, *user_agent;
    void *allow;
    wake_fn wake;
    void *clip_get, *clip_set;
    void *ctx;
} wwe_config_t;
/* One engine per wwe_engine_new: tests run in parallel threads of one process. dir, frames, presents and
 * dirty are the WWE_1.3 (-DPRESENT) stand-in's: the profile directory, frames painted, presents made. */
typedef struct { msg_fn fn; void *ctx; int set; wake_fn wake; void *wake_ctx; char dir[512]; uint64_t frames, presents; int dirty; } engine;
typedef struct { engine *e; } view;
typedef struct { uint32_t w, h; uint8_t *rgba; } image;

uint32_t wwe_abi_version(void) { return 1; }
const char *wwe_engine_version(void) { return "mock"; }
void *wwe_engine_new(const void *config, char *err, size_t n) {
    engine *e = calloc(1, sizeof *e);
    const wwe_config_t *c = config;
    (void)err; (void)n;
    e->wake = c->wake;
    e->wake_ctx = c->ctx;
#ifdef PRESENT
    strncpy(e->dir, c->profile_dir ? c->profile_dir : "", sizeof e->dir - 1);
#endif
    return e;
}
void wwe_engine_free(void *e) { free(e); }
#ifdef PRESENT
/* A frame is painted when the view is new or resized (it is dirty), never twice for one change, as the
 * engine's frame counter works: wwe_state.frames counts the frames painted. */
void wwe_engine_spin(void *e) { engine *en = e; if (en->dirty) { en->frames++; en->dirty = 0; } }
#else
void wwe_engine_spin(void *e) { (void)e; }
#endif
void *wwe_view_new(void *e, const void *t, const char *url, char *err, size_t n) {
    view *v = calloc(1, sizeof *v);
    (void)t; (void)url; (void)err; (void)n;
    v->e = e;
#ifdef PRESENT
    ((engine *)e)->dirty = 1;
#endif
    return v;
}
void wwe_view_free(void *v) { free(v); }
#ifdef PRESENT
void wwe_view_state(const void *v, void *out) {
    memset(out, 0, 16);
    memcpy((uint8_t *)out + 8, &((const view *)v)->e->frames, 8);
}
void wwe_view_resize(void *v, uint32_t w, uint32_t h, float s) { (void)w; (void)h; (void)s; ((view *)v)->e->dirty = 1; }
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
/* The real engine wakes the host from its own threads (script, timers, the compositor): so does an input. */
static void *wake_thread(void *e) {
    engine *en = e;
    if (en->wake) en->wake(en->wake_ctx);
    return NULL;
}
void wwe_view_input(void *v, const void *ev) {
    pthread_t t;
    (void)ev;
    if (pthread_create(&t, NULL, wake_thread, ((view *)v)->e) == 0) pthread_join(t, NULL);
}
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

#ifdef SCHEME
typedef void (*body_fn)(void *, const uint8_t *, size_t);
typedef uint32_t (*load_fn)(void *, const uint8_t *, size_t, uint32_t, char *, size_t, body_fn, void *);
typedef intptr_t (*map_fn)(void *, const uint8_t *, size_t, char *, size_t);
static load_fn the_load;
static map_fn the_map;
static void *the_ctx;
int wwe_engine_set_scheme(void *e, const char *scheme, load_fn load, map_fn map, void *ctx, char *err, size_t n) {
    (void)e;
    if (!scheme || strcmp(scheme, "fed") != 0 || !load) {
        const char *m = "fed only";
        size_t k = strlen(m) < n - 1 ? strlen(m) : n - 1;
        memcpy(err, m, k);
        err[k] = 0;
        return -1;
    }
    the_load = load;
    the_map = map;
    the_ctx = ctx;
    return 0;
}
typedef struct { uint8_t *buf; size_t cap, len; } sink;
static void collect(void *s, const uint8_t *b, size_t len) {
    sink *k = s;
    size_t room = k->cap - k->len;
    size_t m = len < room ? len : room;
    memcpy(k->buf + k->len, b, m);
    k->len += m;
}
uint32_t mock_fed_load(const char *url, uint32_t kind, char *mime, size_t mimecap, uint8_t *buf, size_t cap, size_t *len) {
    sink k = { buf, cap, 0 };
    uint32_t status;
    if (!the_load) return 0;
    status = the_load(the_ctx, (const uint8_t *)url, strlen(url), kind, mime, mimecap, collect, &k);
    *len = k.len;
    return status;
}
intptr_t mock_fed_map(const char *url, char *out, size_t cap) {
    return the_map ? the_map(the_ctx, (const uint8_t *)url, strlen(url), out, cap) : -1;
}
#endif

/* SPDX-License-Identifier: BSD-2-Clause */
/* A C program on libWolkit (ADR 0039): a headless engine, a window with a button, one click,
   one event. Builds the patch by hand (the format is in wolkit.h) and prints what it sees. */
#include <stdio.h>
#include <string.h>
#include <wolkit.h>

static size_t at;
static uint8_t buf[512];

static void u16(uint16_t v) { buf[at++] = (uint8_t)v; buf[at++] = (uint8_t)(v >> 8); }
static void u32(uint32_t v) { u16((uint16_t)v); u16((uint16_t)(v >> 16)); }
static void op(uint16_t code, uint16_t words) { u16(code); u16((uint16_t)(0x8000u | (words + 1u))); }

/* header, ops, pool; returns the length */
static size_t patch_begin(uint32_t seq, uint32_t ops, uint32_t pool_len) {
  at = 0;
  u32(WOLKIT_PATCH_MAGIC); u16(WOLKIT_PATCH_FORMAT); u16(0); u32(seq); u32(ops); u32(pool_len); u32(0);
  return at;
}

static void fail(const char *what, wolkit_status s, wolkit_error *e) {
  size_t n = 0;
  const char *m = e ? wolkit_error_get_message(e, &n) : "";
  printf("%s failed: %d %s\n", what, (int)s, m);
  wolkit_error_release(e);
}

#define CHECK(call, what) do { wolkit_error *e_ = NULL; wolkit_status s_ = (call); \
  if (s_ != WOLKIT_OK) { fail(what, s_, e_); return 1; } wolkit_error_release(e_); } while (0)

int main(void) {
  printf("version: %u.%u.%u %u %s\n", WOLKIT_VERSION_MAJOR, WOLKIT_VERSION_MINOR, WOLKIT_VERSION_PATCH,
         wolkit_version(), wolkit_version_string());

  wolkit_engine *engine = NULL;
  CHECK(wolkit_engine_create("org.example.hello", 17, WOLKIT_ENGINE_HEADLESS, &engine, NULL), "engine");
  printf("backend: %llu\n", (unsigned long long)wolkit_engine_get_info(engine, WOLKIT_INFO_BACKEND));
  printf("fd: %s\n", wolkit_engine_get_fd(engine) >= 0 ? "yes" : "no");

  /* an app id that is not reverse-DNS is refused with a sentence */
  wolkit_engine *bad = NULL; wolkit_error *err = NULL;
  wolkit_status s = wolkit_engine_create("nope", 4, 0, &bad, &err);
  size_t n = 0;
  printf("bad id: %d %s\n", (int)s, wolkit_error_get_message(err, &n));
  wolkit_error_release(err);

  wolkit_window *win = NULL;
  CHECK(wolkit_window_create(engine, "Hello", 5, 320, 200, 0, &win, NULL), "window");
  CHECK(wolkit_window_show(win, NULL), "show");

  /* box 1 under the window; button 2 inside it with the label "Add"; the app wants clicks on 2 */
  const uint32_t pool = 4 + 4; /* "Add" is 3 bytes: length, bytes, one byte of padding */
  size_t o = patch_begin(1, 7, pool);
  op(WOLKIT_OP_CREATE, 2); u32(1); u32(WOLKIT_KIND_BOX);
  op(WOLKIT_OP_CREATE, 2); u32(2); u32(WOLKIT_KIND_BUTTON);
  op(WOLKIT_OP_INSERT, 3); u32(0); u32(1); u32(WOLKIT_APPEND);
  op(WOLKIT_OP_INSERT, 3); u32(1); u32(2); u32(WOLKIT_APPEND);
  op(WOLKIT_OP_SET_TEXT, 2); u32(2); u32(0);
  op(WOLKIT_OP_BIND_EVENT, 2); u32(2); u32(WOLKIT_BIND_CLICK);
  op(WOLKIT_OP_SET_A11Y, 4); u32(2); u32(WOLKIT_ROLE_BUTTON); u32(0); u32(WOLKIT_NO_STR);
  (void)o;
  u32(3); buf[at++] = 'A'; buf[at++] = 'd'; buf[at++] = 'd'; buf[at++] = 0;
  CHECK(wolkit_window_apply(win, buf, at, NULL), "apply");

  /* a broken patch (a part that does not exist) changes nothing and says why */
  patch_begin(2, 1, 0);
  op(WOLKIT_OP_SET_STATE, 2); u32(99); u32(WOLKIT_STATE_DISABLED);
  err = NULL;
  s = wolkit_window_apply(win, buf, at, &err);
  printf("broken patch: %d %s\n", (int)s, wolkit_error_get_message(err, &n));
  wolkit_error_release(err);

  char text[16]; size_t need = 0;
  CHECK(wolkit_window_read_text(win, 2, (uint8_t *)text, sizeof text, &need, NULL), "read_text");
  printf("button text: %.*s\n", (int)need, text);

  float r[4];
  CHECK(wolkit_window_node_rect(win, 2, r, NULL), "node_rect");
  float cx = r[0] + r[2] / 2, cy = r[1] + r[3] / 2;
  uint32_t hit = 0;
  CHECK(wolkit_window_hit_test(win, cx, cy, &hit), "hit_test");
  printf("hit: %u\n", hit);

  CHECK(wolkit_window_inject_pointer(win, WOLKIT_POINTER_MOVE, cx, cy, 0, 0, NULL), "move");
  CHECK(wolkit_window_inject_pointer(win, WOLKIT_POINTER_DOWN, cx, cy, 1, 0, NULL), "down");
  CHECK(wolkit_window_inject_pointer(win, WOLKIT_POINTER_UP, cx, cy, 1, 0, NULL), "up");

  uint32_t flags = 0;
  CHECK(wolkit_engine_dispatch(engine, 0, &flags, NULL), "dispatch");
  printf("events pending: %s\n", (flags & WOLKIT_DISPATCH_EVENTS) ? "yes" : "no");

  uint8_t ev[4096];
  size_t got = wolkit_engine_read_events(engine, ev, sizeof ev);
  size_t off = 0;
  int clicks = 0;
  while (off + WOLKIT_EVENT_HEADER <= got) {
    uint32_t len, window, node, seq; uint16_t kind;
    memcpy(&len, ev + off, 4); memcpy(&kind, ev + off + 4, 2);
    memcpy(&window, ev + off + 8, 4); memcpy(&node, ev + off + 12, 4); memcpy(&seq, ev + off + 16, 4);
    if (kind == WOLKIT_EVENT_CLICK) {
      printf("click: node %u window %s seq %u\n", node, window == wolkit_window_get_id(win) ? "ok" : "wrong", seq);
      clicks++;
    }
    off += len;
  }
  printf("clicks: %d\n", clicks);

  char dump[1024]; need = 0;
  CHECK(wolkit_window_dump(win, WOLKIT_DUMP_A11Y, (uint8_t *)dump, sizeof dump, &need, NULL), "dump");
  printf("a11y: %.*s", (int)need, dump);

  char stats[1024];
  CHECK(wolkit_engine_stats_json(engine, (uint8_t *)stats, sizeof stats, &need, NULL), "stats");
  printf("stats: %s\n", strstr(stats, "\"patches\":1") ? "one patch" : "unexpected");

  /* a handle of the wrong type is refused, not used */
  err = NULL;
  s = wolkit_window_show((wolkit_window *)engine, &err);
  printf("wrong handle: %d\n", (int)s);
  wolkit_error_release(err);

  wolkit_window_release(win);
  wolkit_engine_release(engine);
  wolkit_engine_release(NULL);
  printf("done\n");
  return 0;
}

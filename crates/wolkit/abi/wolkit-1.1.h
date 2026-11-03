/* SPDX-License-Identifier: BSD-2-Clause */
/*
 * wolkit.h: libWolkit, the WolOS UI engine, as a stable C ABI (ADR 0039, libwolkit.so.1).
 *
 * An app builds a scene by sending binary patch streams (wolkit_window_apply) and learns what
 * the person did by draining binary event records (wolkit_engine_read_events). There are no
 * callbacks and no per-node calls. Format of both streams: docs/modules/libwolkit.md.
 *
 * Rules (the same as wol.h):
 *  - Handles are opaque. _create returns a reference the caller releases with _release;
 *    _retain adds one; _get_ results are borrowed from their argument. _release(NULL) is fine.
 *  - Strings are UTF-8 with a length (no NUL needed). Buffers belong to the caller and are
 *    borrowed for the call; the engine copies what it keeps.
 *  - A call that can fail returns a wolkit_status and, when error is not NULL, fills it with a
 *    wolkit_error holding one sentence for a person. Release it with wolkit_error_release.
 *  - One thread at a time per engine. retain and release are safe from any thread.
 *  - "needed" buffers: *needed is always set; a buffer that is too small gets WOLKIT_ERR_LIMIT.
 *  - A panic never crosses the boundary: it becomes WOLKIT_ERR_INTERNAL.
 *
 * Symbols are versioned (abi/wolkit.map, node WOLKIT_1.0); a shipped symbol never changes. The
 * patch stream is the evolution surface: ids below are append-only.
 */
#ifndef WOLKIT_H
#define WOLKIT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- version, status, errors ---- */

#define WOLKIT_VERSION_MAJOR  1
#define WOLKIT_VERSION_MINOR  1
#define WOLKIT_VERSION_PATCH  0

uint32_t wolkit_version(void);
const char *wolkit_version_string(void);

typedef int32_t wolkit_status;
/* 0 OK, 1 INVALID_ARGUMENT, 2 PERMISSION_DENIED, 3 UNAVAILABLE, 4 INTERNAL, 5 LIMIT (a documented
   limit), 6 DEVICE_LOST */
#define WOLKIT_OK                     0
#define WOLKIT_ERR_INVALID_ARGUMENT   1
#define WOLKIT_ERR_PERMISSION_DENIED  2
#define WOLKIT_ERR_UNAVAILABLE        3
#define WOLKIT_ERR_INTERNAL           4
#define WOLKIT_ERR_LIMIT              5
#define WOLKIT_ERR_DEVICE_LOST        6

typedef struct wolkit_error wolkit_error;
wolkit_status wolkit_error_get_code(const wolkit_error *);
const char *wolkit_error_get_message(const wolkit_error *, size_t *len);
void wolkit_error_release(wolkit_error *);

/* ---- engine ---- */

typedef struct wolkit_engine wolkit_engine;
#define WOLKIT_ENGINE_HEADLESS   1u
#define WOLKIT_ENGINE_FORCE_CPU  2u
#define WOLKIT_ENGINE_FORCE_KIT  4u
wolkit_status wolkit_engine_create(const char *app_id, size_t len, uint32_t flags,
                                   wolkit_engine **out, wolkit_error **error);
wolkit_engine *wolkit_engine_retain(wolkit_engine *);
void wolkit_engine_release(wolkit_engine *);
int wolkit_engine_get_fd(const wolkit_engine *);          /* readable when dispatch has work */
#define WOLKIT_DISPATCH_EVENTS  1u
#define WOLKIT_DISPATCH_QUIT    2u
wolkit_status wolkit_engine_dispatch(wolkit_engine *, int32_t timeout_ms, uint32_t *flags,
                                     wolkit_error **error);
size_t wolkit_engine_read_events(wolkit_engine *, uint8_t *buf, size_t cap);   /* whole records */
/* BACKEND, PATCH_FORMAT, MAX_NODES, A11Y_ACTIVE, ... */
uint64_t wolkit_engine_get_info(const wolkit_engine *, uint32_t key);
#define WOLKIT_INFO_BACKEND           1
#define WOLKIT_INFO_PATCH_FORMAT      2
#define WOLKIT_INFO_MAX_NODES         3
#define WOLKIT_INFO_A11Y_ACTIVE       4
#define WOLKIT_INFO_MAX_PATCH         5
#define WOLKIT_INFO_MAX_WINDOWS       6
#define WOLKIT_INFO_MAX_EVENT_RECORD  7
#define WOLKIT_INFO_GPU               8
#define WOLKIT_BACKEND_HEADLESS  1
#define WOLKIT_BACKEND_KIT       2
#define WOLKIT_BACKEND_NATIVE    3
void wolkit_engine_trim(wolkit_engine *, uint32_t level);              /* Memory Saver */
#define WOLKIT_TRIM_LIGHT     0
#define WOLKIT_TRIM_HIDDEN    1
#define WOLKIT_TRIM_CRITICAL  2
wolkit_status wolkit_engine_stats_json(wolkit_engine *, uint8_t *buf, size_t cap, size_t *needed,
                                       wolkit_error **error);
/* same device for the AI framework */
wolkit_status wolkit_engine_get_gpu_uuid(const wolkit_engine *, uint8_t *out16);
/* headless only */
wolkit_status wolkit_engine_set_environment(wolkit_engine *, uint32_t dark, uint32_t accent_rgb,
                                            uint32_t flags, wolkit_error **error);
#define WOLKIT_ENV_REDUCE_MOTION        1u
#define WOLKIT_ENV_REDUCE_TRANSPARENCY  2u
#define WOLKIT_ENV_CONTRAST             4u

/* ---- windows ---- */

typedef struct wolkit_window wolkit_window;
#define WOLKIT_WINDOW_FIXED_SIZE  1u
wolkit_status wolkit_window_create(wolkit_engine *, const char *title, size_t title_len,
                                   uint32_t width, uint32_t height, uint32_t flags,
                                   wolkit_window **out, wolkit_error **error);
wolkit_window *wolkit_window_retain(wolkit_window *);
void wolkit_window_release(wolkit_window *);
uint32_t wolkit_window_get_id(const wolkit_window *);
wolkit_status wolkit_window_set_title(wolkit_window *, const char *, size_t, wolkit_error **);
wolkit_status wolkit_window_set_size_limits(wolkit_window *, uint32_t min_w, uint32_t min_h,
                                            uint32_t max_w, uint32_t max_h, wolkit_error **);
wolkit_status wolkit_window_show(wolkit_window *, wolkit_error **);
wolkit_status wolkit_window_close(wolkit_window *, wolkit_error **);
/* atomic; the batching verb */
wolkit_status wolkit_window_apply(wolkit_window *, const uint8_t *patch, size_t len,
                                  wolkit_error **error);
wolkit_status wolkit_window_read_text(const wolkit_window *, uint32_t node, uint8_t *buf,
                                      size_t cap, size_t *needed, wolkit_error **);
wolkit_status wolkit_window_set_focus(wolkit_window *, uint32_t node, wolkit_error **);
wolkit_status wolkit_window_get_size(const wolkit_window *, uint32_t *w, uint32_t *h, float *scale);
wolkit_status wolkit_window_node_rect(const wolkit_window *, uint32_t node, float *xywh, wolkit_error **);
wolkit_status wolkit_window_hit_test(const wolkit_window *, float x, float y, uint32_t *node);
/* SCENE, LAYOUT, A11Y */
wolkit_status wolkit_window_dump(wolkit_window *, uint32_t what, uint8_t *buf, size_t cap,
                                 size_t *needed, wolkit_error **);
#define WOLKIT_DUMP_SCENE   1
#define WOLKIT_DUMP_LAYOUT  2
#define WOLKIT_DUMP_A11Y    3

/* headless / test only: UNAVAILABLE on a display engine */
wolkit_status wolkit_window_render_rgba(wolkit_window *, uint8_t *pixels, size_t cap,
                                        uint32_t *w, uint32_t *h, wolkit_error **);
wolkit_status wolkit_window_inject_pointer(wolkit_window *, uint32_t kind, float x, float y,
                                           uint32_t button, uint32_t mods, wolkit_error **);
wolkit_status wolkit_window_inject_key(wolkit_window *, uint32_t keysym, uint32_t pressed,
                                       uint32_t mods, wolkit_error **);
wolkit_status wolkit_window_inject_text(wolkit_window *, const char *, size_t, wolkit_error **);
wolkit_status wolkit_window_advance_time(wolkit_window *, uint64_t ns, wolkit_error **);
#define WOLKIT_POINTER_MOVE    0
#define WOLKIT_POINTER_DOWN    1
#define WOLKIT_POINTER_UP      2
#define WOLKIT_POINTER_SCROLL  3
#define WOLKIT_POINTER_LEAVE   4
#define WOLKIT_MOD_SHIFT  1u
#define WOLKIT_MOD_CTRL   2u
#define WOLKIT_MOD_ALT    4u
#define WOLKIT_MOD_SUPER  8u

/* ---- style tables ---- */

typedef struct wolkit_style_table wolkit_style_table;
wolkit_status wolkit_style_table_load(wolkit_engine *, const uint8_t *bytes, size_t len,
                                      wolkit_style_table **out, wolkit_error **);
wolkit_style_table *wolkit_style_table_retain(wolkit_style_table *);
void wolkit_style_table_release(wolkit_style_table *);
wolkit_status wolkit_window_set_style_table(wolkit_window *, wolkit_style_table *, wolkit_error **);
/* dev reload and the CLI */
wolkit_status wolkit_style_compile(const char *wolcss, size_t len, uint8_t *out, size_t cap,
                                   size_t *needed, wolkit_error **error);

/* ---- text catalogs (version 1.1, ADR 0045) ---- */

/* One unit's compiled text tables (.wtx, made by `wol-i18n compile`): English, which defines the
   keys, and any translations. A lookup reads a hash slot and returns a slice of the table: no
   parsing, no allocation. The text is UTF-8, NOT NUL-terminated (use *len), and valid until the
   catalog is released. A language without the key falls back to English. */
typedef struct wolkit_catalog wolkit_catalog;
/* Language ids (wolkit_lang_*): the ten shipped languages and the pseudo-locale xx, which shows
   every string 40 percent longer and accented, to test layouts. */
#define WOLKIT_LANG_EN       0
#define WOLKIT_LANG_PT_BR    1
#define WOLKIT_LANG_ES       2
#define WOLKIT_LANG_FR       3
#define WOLKIT_LANG_DE       4
#define WOLKIT_LANG_JA       5
#define WOLKIT_LANG_KO       6
#define WOLKIT_LANG_ZH_HANS  7
#define WOLKIT_LANG_ID       8
#define WOLKIT_LANG_TL       9
#define WOLKIT_LANG_XX       10
/* in a lookup: the language wolkit_lang_set chose */
#define WOLKIT_LANG_CURRENT  -1
/* wolkit_catalog_find: no such key */
#define WOLKIT_NO_KEY        4294967295u
/* From `count` tables in memory (English among them) */
wolkit_status wolkit_catalog_load(const uint8_t *const *tables, const size_t *lens, size_t count,
                                  wolkit_catalog **out, wolkit_error **);
/* From <dir>/<unit>.<lang>.wtx files; a language pack can add a file without a rebuild */
wolkit_status wolkit_catalog_load_dir(const char *dir, size_t dir_len, const char *unit,
                                      size_t unit_len, wolkit_catalog **out, wolkit_error **);
wolkit_catalog *wolkit_catalog_retain(wolkit_catalog *);
void wolkit_catalog_release(wolkit_catalog *);
uint32_t wolkit_catalog_count(const wolkit_catalog *);
/* the key named `name` ("settings.sound_louder"); do it once per string and keep the key */
uint32_t wolkit_catalog_find(const wolkit_catalog *, const char *name, size_t len);
/* text and len are always written: "" with an error status for an unknown key or language */
wolkit_status wolkit_catalog_text(const wolkit_catalog *, uint32_t key, int32_t lang,
                                  const char **text, size_t *len);
/* the plural form of a key for `count` (CLDR rules of the language) */
wolkit_status wolkit_catalog_plural(const wolkit_catalog *, uint32_t key, uint64_t count,
                                    int32_t lang, const char **text, size_t *len);
/* a tag ("pt-BR") or a POSIX locale ("pt_BR.UTF-8") as a language id; -1: no translation, show English */
int32_t wolkit_lang_parse(const char *name, size_t len);
/* the BCP 47 tag of a language id, static NUL-terminated text; NULL for an unknown id */
const char *wolkit_lang_tag(int32_t lang);
/* the language of this process (WOLKIT_LANG_EN until set), shared by every catalog lookup */
int32_t wolkit_lang_get(void);
wolkit_status wolkit_lang_set(int32_t lang);

/* ---- images ---- */

typedef struct wolkit_image wolkit_image;
/* png, jpeg, svg; decoded on a worker */
wolkit_status wolkit_image_create(wolkit_engine *, const uint8_t *encoded, size_t len,
                                  wolkit_image **out, wolkit_error **);
wolkit_status wolkit_image_create_rgba(wolkit_engine *, uint32_t w, uint32_t h, uint32_t stride,
                                       const uint8_t *pixels, size_t len, wolkit_image **out,
                                       wolkit_error **);
uint32_t wolkit_image_get_id(const wolkit_image *);
wolkit_image *wolkit_image_retain(wolkit_image *);
void wolkit_image_release(wolkit_image *);

/* ---- canvas and GPU surface (native backend only) ---- */

wolkit_status wolkit_window_canvas_submit(wolkit_window *, uint32_t node, const uint8_t *ops,
                                          size_t len, wolkit_error **);
typedef struct wolkit_surface wolkit_surface;
wolkit_status wolkit_surface_create(wolkit_window *, uint32_t node, uint32_t width, uint32_t height,
                                    wolkit_surface **out, wolkit_error **);
wolkit_surface *wolkit_surface_retain(wolkit_surface *);
void wolkit_surface_release(wolkit_surface *);
wolkit_status wolkit_surface_get_size(const wolkit_surface *, uint32_t *w, uint32_t *h, float *scale);
/* static strings */
const char *const *wolkit_surface_get_vk_instance_extensions(uint32_t *count);
wolkit_status wolkit_surface_create_vk(wolkit_surface *, void *vk_instance, uint64_t *vk_surface,
                                       wolkit_error **error);

/* ---- patch stream v1 (frozen) ----
 * header (24 bytes, little-endian): u32 magic, u16 format, u16 0, u32 seq, u32 op_count,
 * u32 pool_len, u32 0. Then ops: u16 opcode, u16 (bit 15 must-understand | length in 4-byte words,
 * header included), payload words. Then the string pool: entries of u32 length, bytes, zero
 * padding to 4; a string reference is the byte offset of an entry (WOLKIT_NO_STR is empty).
 * Node ids: slot in bits 0..23 (< 65536), generation in bits 24..31; 0 is the window itself.
 * Op payloads (words): CREATE node kind | INSERT parent child index (WOLKIT_APPEND) | REMOVE node |
 * MOVE node parent index | SET_PROP node prop value... (arity by property type) | SET_TEXT node
 * strref | SET_STYLE node n class... (n <= 8) | SET_STATE node mask | SET_KEY node lo hi |
 * BIND_EVENT node mask | SET_A11Y node role name_strref desc_strref | ANIM node prop target_f32
 * mode param | LIST_RANGE node count height_f32 overscan | FOCUS node | FREE node. */
#define WOLKIT_PATCH_MAGIC            827345751u
#define WOLKIT_PATCH_FORMAT           1
#define WOLKIT_PATCH_HEADER           24
#define WOLKIT_PATCH_MUST_UNDERSTAND  32768u
#define WOLKIT_APPEND                 4294967295u
#define WOLKIT_NO_STR                 4294967295u
#define WOLKIT_ANIM_TWEEN             0
#define WOLKIT_ANIM_SPRING            1
#define WOLKIT_DIM_AUTO               0
#define WOLKIT_DIM_PX                 1
#define WOLKIT_DIM_PERCENT            2

/* op codes */
#define WOLKIT_OP_CREATE      1
#define WOLKIT_OP_INSERT      2
#define WOLKIT_OP_REMOVE      3
#define WOLKIT_OP_MOVE        4
#define WOLKIT_OP_SET_PROP    5
#define WOLKIT_OP_SET_TEXT    6
#define WOLKIT_OP_SET_STYLE   7
#define WOLKIT_OP_SET_STATE   8
#define WOLKIT_OP_SET_KEY     9
#define WOLKIT_OP_BIND_EVENT  10
#define WOLKIT_OP_SET_A11Y    11
#define WOLKIT_OP_ANIM        12
#define WOLKIT_OP_LIST_RANGE  13
#define WOLKIT_OP_FOCUS       14
#define WOLKIT_OP_FREE        15

/* node kinds */
#define WOLKIT_KIND_BOX         1
#define WOLKIT_KIND_TEXT        2
#define WOLKIT_KIND_IMAGE       3
#define WOLKIT_KIND_SPACER      4
#define WOLKIT_KIND_DIVIDER     5
#define WOLKIT_KIND_BUTTON      6
#define WOLKIT_KIND_TOGGLE      7
#define WOLKIT_KIND_SLIDER      8
#define WOLKIT_KIND_TEXT_INPUT  9
#define WOLKIT_KIND_SCROLL      10
#define WOLKIT_KIND_VLIST       11
#define WOLKIT_KIND_CANVAS      12
#define WOLKIT_KIND_SURFACE     13
#define WOLKIT_KIND_OVERLAY     14

/* accessibility roles */
#define WOLKIT_ROLE_NONE        0
#define WOLKIT_ROLE_GROUP       1
#define WOLKIT_ROLE_LABEL       2
#define WOLKIT_ROLE_BUTTON      3
#define WOLKIT_ROLE_TOGGLE      4
#define WOLKIT_ROLE_CHECKBOX    5
#define WOLKIT_ROLE_SLIDER      6
#define WOLKIT_ROLE_TEXTBOX     7
#define WOLKIT_ROLE_IMAGE       8
#define WOLKIT_ROLE_LIST        9
#define WOLKIT_ROLE_LISTITEM    10
#define WOLKIT_ROLE_HEADING     11
#define WOLKIT_ROLE_TAB         12
#define WOLKIT_ROLE_TABLIST     13
#define WOLKIT_ROLE_TABPANEL    14
#define WOLKIT_ROLE_MENU        15
#define WOLKIT_ROLE_MENUITEM    16
#define WOLKIT_ROLE_DIALOG      17
#define WOLKIT_ROLE_ALERT       18
#define WOLKIT_ROLE_SEPARATOR   19
#define WOLKIT_ROLE_SCROLLVIEW  20
#define WOLKIT_ROLE_LINK        21
#define WOLKIT_ROLE_PROGRESS    22

/* state bits: SET_STATE writes the app-owned ones; the engine owns HOVER, ACTIVE, FOCUS,
   FOCUS_VISIBLE and EMPTY */
#define WOLKIT_STATE_HOVER          1u
#define WOLKIT_STATE_ACTIVE         2u
#define WOLKIT_STATE_FOCUS          4u
#define WOLKIT_STATE_FOCUS_VISIBLE  8u
#define WOLKIT_STATE_DISABLED       16u
#define WOLKIT_STATE_SELECTED       32u
#define WOLKIT_STATE_CHECKED        64u
#define WOLKIT_STATE_EXPANDED       128u
#define WOLKIT_STATE_EMPTY          256u
#define WOLKIT_STATE_BUSY           512u

/* ---- event record v1 (frozen) ----
 * u32 len (whole record, multiple of 8), u16 kind, u16 flags, u32 window, u32 node, u32 seq,
 * payload from byte 20 (read it unaligned). seq is the last patch the engine had applied. */
#define WOLKIT_EVENT_HEADER         20
#define WOLKIT_EVENT_MAX_RECORD     65792
#define WOLKIT_EVENT_FLAG_OVERFLOW  1

/* event kinds */
#define WOLKIT_EVENT_CLICK            1
#define WOLKIT_EVENT_POINTER_ENTER    2
#define WOLKIT_EVENT_POINTER_LEAVE    3
#define WOLKIT_EVENT_KEY              4
#define WOLKIT_EVENT_TEXT_CHANGED     5
#define WOLKIT_EVENT_TEXT_SUBMIT      6
#define WOLKIT_EVENT_TOGGLE           7
#define WOLKIT_EVENT_SLIDER           8
#define WOLKIT_EVENT_SELECT           9
#define WOLKIT_EVENT_SCROLL           10
#define WOLKIT_EVENT_FOCUS            11
#define WOLKIT_EVENT_RESIZE           12
#define WOLKIT_EVENT_CLOSE_REQUEST    13
#define WOLKIT_EVENT_DROP             14
#define WOLKIT_EVENT_DRAG_START       15
#define WOLKIT_EVENT_THEME_CHANGED    16
#define WOLKIT_EVENT_FRAME_TICK       17
#define WOLKIT_EVENT_ANIMATION_DONE   18
#define WOLKIT_EVENT_LIST_RANGE       19
#define WOLKIT_EVENT_SURFACE_RESIZED  20
#define WOLKIT_EVENT_A11Y_ACTION      21
#define WOLKIT_EVENT_VISIBILITY       22
#define WOLKIT_EVENT_CONTEXT_MENU     23
#define WOLKIT_EVENT_SHORTCUT         24
#define WOLKIT_EVENT_DIAGNOSTIC       25

/* BIND_EVENT mask bits: 1 << (event kind - 1) */
#define WOLKIT_BIND_CLICK            1u
#define WOLKIT_BIND_POINTER_ENTER    2u
#define WOLKIT_BIND_POINTER_LEAVE    4u
#define WOLKIT_BIND_KEY              8u
#define WOLKIT_BIND_TEXT_CHANGED     16u
#define WOLKIT_BIND_TEXT_SUBMIT      32u
#define WOLKIT_BIND_TOGGLE           64u
#define WOLKIT_BIND_SLIDER           128u
#define WOLKIT_BIND_SELECT           256u
#define WOLKIT_BIND_SCROLL           512u
#define WOLKIT_BIND_FOCUS            1024u
#define WOLKIT_BIND_RESIZE           2048u
#define WOLKIT_BIND_CLOSE_REQUEST    4096u
#define WOLKIT_BIND_DROP             8192u
#define WOLKIT_BIND_DRAG_START       16384u
#define WOLKIT_BIND_THEME_CHANGED    32768u
#define WOLKIT_BIND_FRAME_TICK       65536u
#define WOLKIT_BIND_ANIMATION_DONE   131072u
#define WOLKIT_BIND_LIST_RANGE       262144u
#define WOLKIT_BIND_SURFACE_RESIZED  524288u
#define WOLKIT_BIND_A11Y_ACTION      1048576u
#define WOLKIT_BIND_VISIBILITY       2097152u
#define WOLKIT_BIND_CONTEXT_MENU     4194304u
#define WOLKIT_BIND_SHORTCUT         8388608u
#define WOLKIT_BIND_DIAGNOSTIC       16777216u

/* ---- properties (SET_PROP) ----
 * Value words by type: f32/i32/bool/enum/token/str/u32 one word; dim two (unit, f32); pair two
 * f32; border (width f32, token); gradient five; shadow four; transition three. */
#define WOLKIT_PROP_DISPLAY              1
#define WOLKIT_PROP_FLEX_DIRECTION       2
#define WOLKIT_PROP_FLEX_WRAP            3
#define WOLKIT_PROP_GROW                 4
#define WOLKIT_PROP_SHRINK               5
#define WOLKIT_PROP_BASIS                6
#define WOLKIT_PROP_GAP                  7
#define WOLKIT_PROP_ROW_GAP              8
#define WOLKIT_PROP_COLUMN_GAP           9
#define WOLKIT_PROP_ALIGN_ITEMS          10
#define WOLKIT_PROP_ALIGN_SELF           11
#define WOLKIT_PROP_ALIGN_CONTENT        12
#define WOLKIT_PROP_JUSTIFY_CONTENT      13
#define WOLKIT_PROP_PADDING              14
#define WOLKIT_PROP_PADDING_TOP          15
#define WOLKIT_PROP_PADDING_RIGHT        16
#define WOLKIT_PROP_PADDING_BOTTOM       17
#define WOLKIT_PROP_PADDING_LEFT         18
#define WOLKIT_PROP_MARGIN               19
#define WOLKIT_PROP_MARGIN_TOP           20
#define WOLKIT_PROP_MARGIN_RIGHT         21
#define WOLKIT_PROP_MARGIN_BOTTOM        22
#define WOLKIT_PROP_MARGIN_LEFT          23
#define WOLKIT_PROP_WIDTH                24
#define WOLKIT_PROP_HEIGHT               25
#define WOLKIT_PROP_MIN_WIDTH            26
#define WOLKIT_PROP_MIN_HEIGHT           27
#define WOLKIT_PROP_MAX_WIDTH            28
#define WOLKIT_PROP_MAX_HEIGHT           29
#define WOLKIT_PROP_ASPECT_RATIO         30
#define WOLKIT_PROP_POSITION             31
#define WOLKIT_PROP_INSET_TOP            32
#define WOLKIT_PROP_INSET_RIGHT          33
#define WOLKIT_PROP_INSET_BOTTOM         34
#define WOLKIT_PROP_INSET_LEFT           35
#define WOLKIT_PROP_OVERFLOW             36
#define WOLKIT_PROP_Z_INDEX              37
#define WOLKIT_PROP_TAB_INDEX            38
#define WOLKIT_PROP_BACKGROUND           50
#define WOLKIT_PROP_BACKGROUND_GRADIENT  51
#define WOLKIT_PROP_BORDER               52
#define WOLKIT_PROP_BORDER_RADIUS        53
#define WOLKIT_PROP_RADIUS_TL            54
#define WOLKIT_PROP_RADIUS_TR            55
#define WOLKIT_PROP_RADIUS_BR            56
#define WOLKIT_PROP_RADIUS_BL            57
#define WOLKIT_PROP_SHADOW_1             58
#define WOLKIT_PROP_SHADOW_2             59
#define WOLKIT_PROP_OPACITY              60
#define WOLKIT_PROP_OUTLINE              61
#define WOLKIT_PROP_TRANSLATE            62
#define WOLKIT_PROP_SCALE                63
#define WOLKIT_PROP_ROTATE               64
#define WOLKIT_PROP_CURSOR               65
#define WOLKIT_PROP_POINTER_EVENTS       66
#define WOLKIT_PROP_VISIBILITY           67
#define WOLKIT_PROP_BACKDROP             68
#define WOLKIT_PROP_COLOR                100
#define WOLKIT_PROP_FONT_FAMILY          101
#define WOLKIT_PROP_FONT_SIZE            102
#define WOLKIT_PROP_FONT_WEIGHT          103
#define WOLKIT_PROP_FONT_STYLE           104
#define WOLKIT_PROP_LINE_HEIGHT          105
#define WOLKIT_PROP_LETTER_SPACING       106
#define WOLKIT_PROP_TEXT_ALIGN           107
#define WOLKIT_PROP_TEXT_DECORATION      108
#define WOLKIT_PROP_TEXT_OVERFLOW        109
#define WOLKIT_PROP_WHITE_SPACE          110
#define WOLKIT_PROP_MAX_LINES            111
#define WOLKIT_PROP_FONT_FEATURE         112
#define WOLKIT_PROP_USER_SELECT          113
#define WOLKIT_PROP_TRANSITION           140
#define WOLKIT_PROP_SPRING               141
#define WOLKIT_PROP_SLIDER_MIN           150
#define WOLKIT_PROP_SLIDER_MAX           151
#define WOLKIT_PROP_SLIDER_VALUE         152
#define WOLKIT_PROP_SLIDER_STEP          153
#define WOLKIT_PROP_PLACEHOLDER          154
#define WOLKIT_PROP_MAX_LENGTH           155
#define WOLKIT_PROP_SECURE               156
#define WOLKIT_PROP_READONLY             157
#define WOLKIT_PROP_IMAGE_SOURCE         158
#define WOLKIT_PROP_IMAGE_ID             159
#define WOLKIT_PROP_IMAGE_FIT            160
#define WOLKIT_PROP_SCROLL_OFFSET        161
#define WOLKIT_PROP_TOOLTIP              162
#define WOLKIT_PROP_FOCUSABLE            163
#define WOLKIT_PROP_ANCHOR               164
#define WOLKIT_PROP_ANCHOR_SIDE          165
#define WOLKIT_PROP_ANCHOR_ALIGN         166
#define WOLKIT_PROP_MODAL                167
#define WOLKIT_PROP_LIST_COUNT           190
#define WOLKIT_PROP_LIST_HEIGHT          191
#define WOLKIT_PROP_LIST_OVERSCAN        192

#ifdef __cplusplus
}
#endif

#endif /* WOLKIT_H */

# SPDX-License-Identifier: BSD-2-Clause
# Generated from crates/wolkit/include/wolkit.h by crates/wol-abi/tests/bindings.rs; do not edit.
# Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings
@[Link("wolkit")]
lib LibWolkit
  WOLKIT_VERSION_MAJOR            =              1
  WOLKIT_VERSION_MINOR            =              1
  WOLKIT_VERSION_PATCH            =              0
  WOLKIT_OK                       =              0
  WOLKIT_ERR_INVALID_ARGUMENT     =              1
  WOLKIT_ERR_PERMISSION_DENIED    =              2
  WOLKIT_ERR_UNAVAILABLE          =              3
  WOLKIT_ERR_INTERNAL             =              4
  WOLKIT_ERR_LIMIT                =              5
  WOLKIT_ERR_DEVICE_LOST          =              6
  WOLKIT_ENGINE_HEADLESS          =          1_u32
  WOLKIT_ENGINE_FORCE_CPU         =          2_u32
  WOLKIT_ENGINE_FORCE_KIT         =          4_u32
  WOLKIT_DISPATCH_EVENTS          =          1_u32
  WOLKIT_DISPATCH_QUIT            =          2_u32
  WOLKIT_INFO_BACKEND             =              1
  WOLKIT_INFO_PATCH_FORMAT        =              2
  WOLKIT_INFO_MAX_NODES           =              3
  WOLKIT_INFO_A11Y_ACTIVE         =              4
  WOLKIT_INFO_MAX_PATCH           =              5
  WOLKIT_INFO_MAX_WINDOWS         =              6
  WOLKIT_INFO_MAX_EVENT_RECORD    =              7
  WOLKIT_INFO_GPU                 =              8
  WOLKIT_BACKEND_HEADLESS         =              1
  WOLKIT_BACKEND_KIT              =              2
  WOLKIT_BACKEND_NATIVE           =              3
  WOLKIT_TRIM_LIGHT               =              0
  WOLKIT_TRIM_HIDDEN              =              1
  WOLKIT_TRIM_CRITICAL            =              2
  WOLKIT_ENV_REDUCE_MOTION        =          1_u32
  WOLKIT_ENV_REDUCE_TRANSPARENCY  =          2_u32
  WOLKIT_ENV_CONTRAST             =          4_u32
  WOLKIT_WINDOW_FIXED_SIZE        =          1_u32
  WOLKIT_DUMP_SCENE               =              1
  WOLKIT_DUMP_LAYOUT              =              2
  WOLKIT_DUMP_A11Y                =              3
  WOLKIT_POINTER_MOVE             =              0
  WOLKIT_POINTER_DOWN             =              1
  WOLKIT_POINTER_UP               =              2
  WOLKIT_POINTER_SCROLL           =              3
  WOLKIT_POINTER_LEAVE            =              4
  WOLKIT_MOD_SHIFT                =          1_u32
  WOLKIT_MOD_CTRL                 =          2_u32
  WOLKIT_MOD_ALT                  =          4_u32
  WOLKIT_MOD_SUPER                =          8_u32
  WOLKIT_IMAGE_PENDING            =              0
  WOLKIT_IMAGE_READY              =              1
  WOLKIT_IMAGE_FAILED             =              2
  WOLKIT_PATCH_MAGIC              =  827345751_u32
  WOLKIT_PATCH_FORMAT             =              1
  WOLKIT_PATCH_HEADER             =             24
  WOLKIT_PATCH_MUST_UNDERSTAND    =      32768_u32
  WOLKIT_APPEND                   = 4294967295_u32
  WOLKIT_NO_STR                   = 4294967295_u32
  WOLKIT_ANIM_TWEEN               =              0
  WOLKIT_ANIM_SPRING              =              1
  WOLKIT_DIM_AUTO                 =              0
  WOLKIT_DIM_PX                   =              1
  WOLKIT_DIM_PERCENT              =              2
  WOLKIT_OP_CREATE                =              1
  WOLKIT_OP_INSERT                =              2
  WOLKIT_OP_REMOVE                =              3
  WOLKIT_OP_MOVE                  =              4
  WOLKIT_OP_SET_PROP              =              5
  WOLKIT_OP_SET_TEXT              =              6
  WOLKIT_OP_SET_STYLE             =              7
  WOLKIT_OP_SET_STATE             =              8
  WOLKIT_OP_SET_KEY               =              9
  WOLKIT_OP_BIND_EVENT            =             10
  WOLKIT_OP_SET_A11Y              =             11
  WOLKIT_OP_ANIM                  =             12
  WOLKIT_OP_LIST_RANGE            =             13
  WOLKIT_OP_FOCUS                 =             14
  WOLKIT_OP_FREE                  =             15
  WOLKIT_KIND_BOX                 =              1
  WOLKIT_KIND_TEXT                =              2
  WOLKIT_KIND_IMAGE               =              3
  WOLKIT_KIND_SPACER              =              4
  WOLKIT_KIND_DIVIDER             =              5
  WOLKIT_KIND_BUTTON              =              6
  WOLKIT_KIND_TOGGLE              =              7
  WOLKIT_KIND_SLIDER              =              8
  WOLKIT_KIND_TEXT_INPUT          =              9
  WOLKIT_KIND_SCROLL              =             10
  WOLKIT_KIND_VLIST               =             11
  WOLKIT_KIND_CANVAS              =             12
  WOLKIT_KIND_SURFACE             =             13
  WOLKIT_KIND_OVERLAY             =             14
  WOLKIT_ROLE_NONE                =              0
  WOLKIT_ROLE_GROUP               =              1
  WOLKIT_ROLE_LABEL               =              2
  WOLKIT_ROLE_BUTTON              =              3
  WOLKIT_ROLE_TOGGLE              =              4
  WOLKIT_ROLE_CHECKBOX            =              5
  WOLKIT_ROLE_SLIDER              =              6
  WOLKIT_ROLE_TEXTBOX             =              7
  WOLKIT_ROLE_IMAGE               =              8
  WOLKIT_ROLE_LIST                =              9
  WOLKIT_ROLE_LISTITEM            =             10
  WOLKIT_ROLE_HEADING             =             11
  WOLKIT_ROLE_TAB                 =             12
  WOLKIT_ROLE_TABLIST             =             13
  WOLKIT_ROLE_TABPANEL            =             14
  WOLKIT_ROLE_MENU                =             15
  WOLKIT_ROLE_MENUITEM            =             16
  WOLKIT_ROLE_DIALOG              =             17
  WOLKIT_ROLE_ALERT               =             18
  WOLKIT_ROLE_SEPARATOR           =             19
  WOLKIT_ROLE_SCROLLVIEW          =             20
  WOLKIT_ROLE_LINK                =             21
  WOLKIT_ROLE_PROGRESS            =             22
  WOLKIT_STATE_HOVER              =          1_u32
  WOLKIT_STATE_ACTIVE             =          2_u32
  WOLKIT_STATE_FOCUS              =          4_u32
  WOLKIT_STATE_FOCUS_VISIBLE      =          8_u32
  WOLKIT_STATE_DISABLED           =         16_u32
  WOLKIT_STATE_SELECTED           =         32_u32
  WOLKIT_STATE_CHECKED            =         64_u32
  WOLKIT_STATE_EXPANDED           =        128_u32
  WOLKIT_STATE_EMPTY              =        256_u32
  WOLKIT_STATE_BUSY               =        512_u32
  WOLKIT_EVENT_HEADER             =             20
  WOLKIT_EVENT_MAX_RECORD         =          65792
  WOLKIT_EVENT_FLAG_OVERFLOW      =              1
  WOLKIT_EVENT_CLICK              =              1
  WOLKIT_EVENT_POINTER_ENTER      =              2
  WOLKIT_EVENT_POINTER_LEAVE      =              3
  WOLKIT_EVENT_KEY                =              4
  WOLKIT_EVENT_TEXT_CHANGED       =              5
  WOLKIT_EVENT_TEXT_SUBMIT        =              6
  WOLKIT_EVENT_TOGGLE             =              7
  WOLKIT_EVENT_SLIDER             =              8
  WOLKIT_EVENT_SELECT             =              9
  WOLKIT_EVENT_SCROLL             =             10
  WOLKIT_EVENT_FOCUS              =             11
  WOLKIT_EVENT_RESIZE             =             12
  WOLKIT_EVENT_CLOSE_REQUEST      =             13
  WOLKIT_EVENT_DROP               =             14
  WOLKIT_EVENT_DRAG_START         =             15
  WOLKIT_EVENT_THEME_CHANGED      =             16
  WOLKIT_EVENT_FRAME_TICK         =             17
  WOLKIT_EVENT_ANIMATION_DONE     =             18
  WOLKIT_EVENT_LIST_RANGE         =             19
  WOLKIT_EVENT_SURFACE_RESIZED    =             20
  WOLKIT_EVENT_A11Y_ACTION        =             21
  WOLKIT_EVENT_VISIBILITY         =             22
  WOLKIT_EVENT_CONTEXT_MENU       =             23
  WOLKIT_EVENT_SHORTCUT           =             24
  WOLKIT_EVENT_DIAGNOSTIC         =             25
  WOLKIT_BIND_CLICK               =          1_u32
  WOLKIT_BIND_POINTER_ENTER       =          2_u32
  WOLKIT_BIND_POINTER_LEAVE       =          4_u32
  WOLKIT_BIND_KEY                 =          8_u32
  WOLKIT_BIND_TEXT_CHANGED        =         16_u32
  WOLKIT_BIND_TEXT_SUBMIT         =         32_u32
  WOLKIT_BIND_TOGGLE              =         64_u32
  WOLKIT_BIND_SLIDER              =        128_u32
  WOLKIT_BIND_SELECT              =        256_u32
  WOLKIT_BIND_SCROLL              =        512_u32
  WOLKIT_BIND_FOCUS               =       1024_u32
  WOLKIT_BIND_RESIZE              =       2048_u32
  WOLKIT_BIND_CLOSE_REQUEST       =       4096_u32
  WOLKIT_BIND_DROP                =       8192_u32
  WOLKIT_BIND_DRAG_START          =      16384_u32
  WOLKIT_BIND_THEME_CHANGED       =      32768_u32
  WOLKIT_BIND_FRAME_TICK          =      65536_u32
  WOLKIT_BIND_ANIMATION_DONE      =     131072_u32
  WOLKIT_BIND_LIST_RANGE          =     262144_u32
  WOLKIT_BIND_SURFACE_RESIZED     =     524288_u32
  WOLKIT_BIND_A11Y_ACTION         =    1048576_u32
  WOLKIT_BIND_VISIBILITY          =    2097152_u32
  WOLKIT_BIND_CONTEXT_MENU        =    4194304_u32
  WOLKIT_BIND_SHORTCUT            =    8388608_u32
  WOLKIT_BIND_DIAGNOSTIC          =   16777216_u32
  WOLKIT_PROP_DISPLAY             =              1
  WOLKIT_PROP_FLEX_DIRECTION      =              2
  WOLKIT_PROP_FLEX_WRAP           =              3
  WOLKIT_PROP_GROW                =              4
  WOLKIT_PROP_SHRINK              =              5
  WOLKIT_PROP_BASIS               =              6
  WOLKIT_PROP_GAP                 =              7
  WOLKIT_PROP_ROW_GAP             =              8
  WOLKIT_PROP_COLUMN_GAP          =              9
  WOLKIT_PROP_ALIGN_ITEMS         =             10
  WOLKIT_PROP_ALIGN_SELF          =             11
  WOLKIT_PROP_ALIGN_CONTENT       =             12
  WOLKIT_PROP_JUSTIFY_CONTENT     =             13
  WOLKIT_PROP_PADDING             =             14
  WOLKIT_PROP_PADDING_TOP         =             15
  WOLKIT_PROP_PADDING_RIGHT       =             16
  WOLKIT_PROP_PADDING_BOTTOM      =             17
  WOLKIT_PROP_PADDING_LEFT        =             18
  WOLKIT_PROP_MARGIN              =             19
  WOLKIT_PROP_MARGIN_TOP          =             20
  WOLKIT_PROP_MARGIN_RIGHT        =             21
  WOLKIT_PROP_MARGIN_BOTTOM       =             22
  WOLKIT_PROP_MARGIN_LEFT         =             23
  WOLKIT_PROP_WIDTH               =             24
  WOLKIT_PROP_HEIGHT              =             25
  WOLKIT_PROP_MIN_WIDTH           =             26
  WOLKIT_PROP_MIN_HEIGHT          =             27
  WOLKIT_PROP_MAX_WIDTH           =             28
  WOLKIT_PROP_MAX_HEIGHT          =             29
  WOLKIT_PROP_ASPECT_RATIO        =             30
  WOLKIT_PROP_POSITION            =             31
  WOLKIT_PROP_INSET_TOP           =             32
  WOLKIT_PROP_INSET_RIGHT         =             33
  WOLKIT_PROP_INSET_BOTTOM        =             34
  WOLKIT_PROP_INSET_LEFT          =             35
  WOLKIT_PROP_OVERFLOW            =             36
  WOLKIT_PROP_Z_INDEX             =             37
  WOLKIT_PROP_TAB_INDEX           =             38
  WOLKIT_PROP_BACKGROUND          =             50
  WOLKIT_PROP_BACKGROUND_GRADIENT =             51
  WOLKIT_PROP_BORDER              =             52
  WOLKIT_PROP_BORDER_RADIUS       =             53
  WOLKIT_PROP_RADIUS_TL           =             54
  WOLKIT_PROP_RADIUS_TR           =             55
  WOLKIT_PROP_RADIUS_BR           =             56
  WOLKIT_PROP_RADIUS_BL           =             57
  WOLKIT_PROP_SHADOW_1            =             58
  WOLKIT_PROP_SHADOW_2            =             59
  WOLKIT_PROP_OPACITY             =             60
  WOLKIT_PROP_OUTLINE             =             61
  WOLKIT_PROP_TRANSLATE           =             62
  WOLKIT_PROP_SCALE               =             63
  WOLKIT_PROP_ROTATE              =             64
  WOLKIT_PROP_CURSOR              =             65
  WOLKIT_PROP_POINTER_EVENTS      =             66
  WOLKIT_PROP_VISIBILITY          =             67
  WOLKIT_PROP_BACKDROP            =             68
  WOLKIT_PROP_COLOR               =            100
  WOLKIT_PROP_FONT_FAMILY         =            101
  WOLKIT_PROP_FONT_SIZE           =            102
  WOLKIT_PROP_FONT_WEIGHT         =            103
  WOLKIT_PROP_FONT_STYLE          =            104
  WOLKIT_PROP_LINE_HEIGHT         =            105
  WOLKIT_PROP_LETTER_SPACING      =            106
  WOLKIT_PROP_TEXT_ALIGN          =            107
  WOLKIT_PROP_TEXT_DECORATION     =            108
  WOLKIT_PROP_TEXT_OVERFLOW       =            109
  WOLKIT_PROP_WHITE_SPACE         =            110
  WOLKIT_PROP_MAX_LINES           =            111
  WOLKIT_PROP_FONT_FEATURE        =            112
  WOLKIT_PROP_USER_SELECT         =            113
  WOLKIT_PROP_TRANSITION          =            140
  WOLKIT_PROP_SPRING              =            141
  WOLKIT_PROP_SLIDER_MIN          =            150
  WOLKIT_PROP_SLIDER_MAX          =            151
  WOLKIT_PROP_SLIDER_VALUE        =            152
  WOLKIT_PROP_SLIDER_STEP         =            153
  WOLKIT_PROP_PLACEHOLDER         =            154
  WOLKIT_PROP_MAX_LENGTH          =            155
  WOLKIT_PROP_SECURE              =            156
  WOLKIT_PROP_READONLY            =            157
  WOLKIT_PROP_IMAGE_SOURCE        =            158
  WOLKIT_PROP_IMAGE_ID            =            159
  WOLKIT_PROP_IMAGE_FIT           =            160
  WOLKIT_PROP_SCROLL_OFFSET       =            161
  WOLKIT_PROP_TOOLTIP             =            162
  WOLKIT_PROP_FOCUSABLE           =            163
  WOLKIT_PROP_ANCHOR              =            164
  WOLKIT_PROP_ANCHOR_SIDE         =            165
  WOLKIT_PROP_ANCHOR_ALIGN        =            166
  WOLKIT_PROP_MODAL               =            167
  WOLKIT_PROP_LIST_COUNT          =            190
  WOLKIT_PROP_LIST_HEIGHT         =            191
  WOLKIT_PROP_LIST_OVERSCAN       =            192

  alias Status = Int32

  # Handles are opaque: the field exists only because a Crystal struct cannot be empty.
  struct Error
    opaque : UInt8
  end

  struct Engine
    opaque : UInt8
  end

  struct Window
    opaque : UInt8
  end

  struct StyleTable
    opaque : UInt8
  end

  struct Image
    opaque : UInt8
  end

  struct Surface
    opaque : UInt8
  end

  fun wolkit_version : UInt32
  fun wolkit_version_string : UInt8*
  fun wolkit_error_get_code(arg0 : Error*) : Status
  fun wolkit_error_get_message(arg0 : Error*, len : LibC::SizeT*) : UInt8*
  fun wolkit_error_release(arg0 : Error*)
  fun wolkit_engine_create(app_id : UInt8*, len : LibC::SizeT, flags : UInt32, out_ : Engine**, error : Error**) : Status
  fun wolkit_engine_retain(arg0 : Engine*) : Engine*
  fun wolkit_engine_release(arg0 : Engine*)
  fun wolkit_engine_get_fd(arg0 : Engine*) : Int32
  fun wolkit_engine_dispatch(arg0 : Engine*, timeout_ms : Int32, flags : UInt32*, error : Error**) : Status
  fun wolkit_engine_read_events(arg0 : Engine*, buf : UInt8*, cap : LibC::SizeT) : LibC::SizeT
  fun wolkit_engine_get_info(arg0 : Engine*, key : UInt32) : UInt64
  fun wolkit_engine_trim(arg0 : Engine*, level : UInt32)
  fun wolkit_engine_stats_json(arg0 : Engine*, buf : UInt8*, cap : LibC::SizeT, needed : LibC::SizeT*, error : Error**) : Status
  fun wolkit_engine_get_gpu_uuid(arg0 : Engine*, out16 : UInt8*) : Status
  fun wolkit_engine_set_environment(arg0 : Engine*, dark : UInt32, accent_rgb : UInt32, flags : UInt32, error : Error**) : Status
  fun wolkit_window_create(arg0 : Engine*, title : UInt8*, title_len : LibC::SizeT, width : UInt32, height : UInt32, flags : UInt32, out_ : Window**, error : Error**) : Status
  fun wolkit_window_retain(arg0 : Window*) : Window*
  fun wolkit_window_release(arg0 : Window*)
  fun wolkit_window_get_id(arg0 : Window*) : UInt32
  fun wolkit_window_set_title(arg0 : Window*, arg1 : UInt8*, arg2 : LibC::SizeT, arg3 : Error**) : Status
  fun wolkit_window_set_size_limits(arg0 : Window*, min_w : UInt32, min_h : UInt32, max_w : UInt32, max_h : UInt32, arg5 : Error**) : Status
  fun wolkit_window_show(arg0 : Window*, arg1 : Error**) : Status
  fun wolkit_window_close(arg0 : Window*, arg1 : Error**) : Status
  fun wolkit_window_apply(arg0 : Window*, patch : UInt8*, len : LibC::SizeT, error : Error**) : Status
  fun wolkit_window_read_text(arg0 : Window*, node : UInt32, buf : UInt8*, cap : LibC::SizeT, needed : LibC::SizeT*, arg5 : Error**) : Status
  fun wolkit_window_set_focus(arg0 : Window*, node : UInt32, arg2 : Error**) : Status
  fun wolkit_window_get_size(arg0 : Window*, w : UInt32*, h : UInt32*, scale : Float32*) : Status
  fun wolkit_window_node_rect(arg0 : Window*, node : UInt32, xywh : Float32*, arg3 : Error**) : Status
  fun wolkit_window_hit_test(arg0 : Window*, x : Float32, y : Float32, node : UInt32*) : Status
  fun wolkit_window_dump(arg0 : Window*, what : UInt32, buf : UInt8*, cap : LibC::SizeT, needed : LibC::SizeT*, arg5 : Error**) : Status
  fun wolkit_window_render_rgba(arg0 : Window*, pixels : UInt8*, cap : LibC::SizeT, w : UInt32*, h : UInt32*, arg5 : Error**) : Status
  fun wolkit_window_inject_pointer(arg0 : Window*, kind : UInt32, x : Float32, y : Float32, button : UInt32, mods : UInt32, arg6 : Error**) : Status
  fun wolkit_window_inject_key(arg0 : Window*, keysym : UInt32, pressed : UInt32, mods : UInt32, arg4 : Error**) : Status
  fun wolkit_window_inject_text(arg0 : Window*, arg1 : UInt8*, arg2 : LibC::SizeT, arg3 : Error**) : Status
  fun wolkit_window_advance_time(arg0 : Window*, ns : UInt64, arg2 : Error**) : Status
  fun wolkit_style_table_load(arg0 : Engine*, bytes : UInt8*, len : LibC::SizeT, out_ : StyleTable**, arg4 : Error**) : Status
  fun wolkit_style_table_retain(arg0 : StyleTable*) : StyleTable*
  fun wolkit_style_table_release(arg0 : StyleTable*)
  fun wolkit_window_set_style_table(arg0 : Window*, arg1 : StyleTable*, arg2 : Error**) : Status
  fun wolkit_style_compile(wolcss : UInt8*, len : LibC::SizeT, out_ : UInt8*, cap : LibC::SizeT, needed : LibC::SizeT*, error : Error**) : Status
  fun wolkit_image_create(arg0 : Engine*, encoded : UInt8*, len : LibC::SizeT, out_ : Image**, arg4 : Error**) : Status
  fun wolkit_image_create_rgba(arg0 : Engine*, w : UInt32, h : UInt32, stride : UInt32, pixels : UInt8*, len : LibC::SizeT, out_ : Image**, arg7 : Error**) : Status
  fun wolkit_image_get_id(arg0 : Image*) : UInt32
  fun wolkit_image_retain(arg0 : Image*) : Image*
  fun wolkit_image_release(arg0 : Image*)
  fun wolkit_image_create_encoded(arg0 : Engine*, encoded : UInt8*, len : LibC::SizeT, max_side : UInt32, out_ : Image**, arg5 : Error**) : Status
  fun wolkit_image_create_file(arg0 : Engine*, path : UInt8*, len : LibC::SizeT, max_side : UInt32, out_ : Image**, arg5 : Error**) : Status
  fun wolkit_image_get_size(arg0 : Image*, w : UInt32*, h : UInt32*) : Status
  fun wolkit_image_get_state(arg0 : Image*) : UInt32
  fun wolkit_window_canvas_submit(arg0 : Window*, node : UInt32, ops : UInt8*, len : LibC::SizeT, arg4 : Error**) : Status
  fun wolkit_surface_create(arg0 : Window*, node : UInt32, width : UInt32, height : UInt32, out_ : Surface**, arg5 : Error**) : Status
  fun wolkit_surface_retain(arg0 : Surface*) : Surface*
  fun wolkit_surface_release(arg0 : Surface*)
  fun wolkit_surface_get_size(arg0 : Surface*, w : UInt32*, h : UInt32*, scale : Float32*) : Status
  fun wolkit_surface_get_vk_instance_extensions(count : UInt32*) : UInt8**
  fun wolkit_surface_create_vk(arg0 : Surface*, vk_instance : Void*, vk_surface : UInt64*, error : Error**) : Status
end

# SPDX-License-Identifier: BSD-2-Clause
# The wire ids of patch stream v1 and event record v1, mirrored from libwolkit's append-only tables
# (crates/wolkit/src/scene/{prop,kind,patch}.rs, crates/wolkit/src/event/mod.rs and wolkit.h). The
# ids never change; spec/wire_spec.cr compares this file with the engine's snapshot so a drift
# fails a test.
module Wolkit
  module Wire
    # Patch stream header and limits.
    MAGIC           = 0x3150_4b57_u32 # "WKP1", little-endian
    FORMAT          =           1_u16
    HEADER          =              24
    MAX_PATCH       = 1 << 20
    MAX_STRING      = 64 * 1024
    MAX_CLASSES     =               8
    APPEND          = 0xffff_ffff_u32
    NO_STR          = 0xffff_ffff_u32
    MUST_UNDERSTAND =      0x8000_u16

    # Opcodes.
    module Op
      CREATE     =  1_u16
      INSERT     =  2_u16
      REMOVE     =  3_u16
      MOVE       =  4_u16
      SET_PROP   =  5_u16
      SET_TEXT   =  6_u16
      SET_STYLE  =  7_u16
      SET_STATE  =  8_u16
      SET_KEY    =  9_u16
      BIND_EVENT = 10_u16
      SET_A11Y   = 11_u16
      ANIM       = 12_u16
      LIST_RANGE = 13_u16
      FOCUS      = 14_u16
      FREE       = 15_u16
    end

    # Node kinds.
    module Kind
      BOX        =  1_u32
      TEXT       =  2_u32
      IMAGE      =  3_u32
      SPACER     =  4_u32
      DIVIDER    =  5_u32
      BUTTON     =  6_u32
      TOGGLE     =  7_u32
      SLIDER     =  8_u32
      TEXT_INPUT =  9_u32
      SCROLL     = 10_u32
      VLIST      = 11_u32
      CANVAS     = 12_u32
      SURFACE    = 13_u32
      OVERLAY    = 14_u32
    end

    # Accessibility roles.
    module Role
      NONE       =  0_u32
      GROUP      =  1_u32
      LABEL      =  2_u32
      BUTTON     =  3_u32
      TOGGLE     =  4_u32
      CHECKBOX   =  5_u32
      SLIDER     =  6_u32
      TEXTBOX    =  7_u32
      IMAGE      =  8_u32
      LIST       =  9_u32
      LISTITEM   = 10_u32
      HEADING    = 11_u32
      TAB        = 12_u32
      TABLIST    = 13_u32
      TABPANEL   = 14_u32
      MENU       = 15_u32
      MENUITEM   = 16_u32
      DIALOG     = 17_u32
      ALERT      = 18_u32
      SEPARATOR  = 19_u32
      SCROLLVIEW = 20_u32
      LINK       = 21_u32
      PROGRESS   = 22_u32
    end

    # State bits; the engine owns the first four.
    module State
      HOVER         =   1_u32
      ACTIVE        =   2_u32
      FOCUS         =   4_u32
      FOCUS_VISIBLE =   8_u32
      DISABLED      =  16_u32
      SELECTED      =  32_u32
      CHECKED       =  64_u32
      EXPANDED      = 128_u32
      EMPTY         = 256_u32
      BUSY          = 512_u32
    end

    # Event kinds.
    module Event
      CLICK           =  1_u16
      POINTER_ENTER   =  2_u16
      POINTER_LEAVE   =  3_u16
      KEY             =  4_u16
      TEXT_CHANGED    =  5_u16
      TEXT_SUBMIT     =  6_u16
      TOGGLE          =  7_u16
      SLIDER          =  8_u16
      SELECT          =  9_u16
      SCROLL          = 10_u16
      FOCUS           = 11_u16
      RESIZE          = 12_u16
      CLOSE_REQUEST   = 13_u16
      DROP            = 14_u16
      DRAG_START      = 15_u16
      THEME_CHANGED   = 16_u16
      FRAME_TICK      = 17_u16
      ANIMATION_DONE  = 18_u16
      LIST_RANGE      = 19_u16
      SURFACE_RESIZED = 20_u16
      A11Y_ACTION     = 21_u16
      VISIBILITY      = 22_u16
      CONTEXT_MENU    = 23_u16
      SHORTCUT        = 24_u16
      DIAGNOSTIC      = 25_u16
    end

    # Property ids.
    module Prop
      DISPLAY             =   1_u16
      FLEX_DIRECTION      =   2_u16
      FLEX_WRAP           =   3_u16
      GROW                =   4_u16
      SHRINK              =   5_u16
      BASIS               =   6_u16
      GAP                 =   7_u16
      ROW_GAP             =   8_u16
      COLUMN_GAP          =   9_u16
      ALIGN_ITEMS         =  10_u16
      ALIGN_SELF          =  11_u16
      ALIGN_CONTENT       =  12_u16
      JUSTIFY_CONTENT     =  13_u16
      PADDING             =  14_u16
      PADDING_TOP         =  15_u16
      PADDING_RIGHT       =  16_u16
      PADDING_BOTTOM      =  17_u16
      PADDING_LEFT        =  18_u16
      MARGIN              =  19_u16
      MARGIN_TOP          =  20_u16
      MARGIN_RIGHT        =  21_u16
      MARGIN_BOTTOM       =  22_u16
      MARGIN_LEFT         =  23_u16
      WIDTH               =  24_u16
      HEIGHT              =  25_u16
      MIN_WIDTH           =  26_u16
      MIN_HEIGHT          =  27_u16
      MAX_WIDTH           =  28_u16
      MAX_HEIGHT          =  29_u16
      ASPECT_RATIO        =  30_u16
      POSITION            =  31_u16
      INSET_TOP           =  32_u16
      INSET_RIGHT         =  33_u16
      INSET_BOTTOM        =  34_u16
      INSET_LEFT          =  35_u16
      OVERFLOW            =  36_u16
      Z_INDEX             =  37_u16
      TAB_INDEX           =  38_u16
      BACKGROUND          =  50_u16
      BACKGROUND_GRADIENT =  51_u16
      BORDER              =  52_u16
      BORDER_RADIUS       =  53_u16
      RADIUS_TL           =  54_u16
      RADIUS_TR           =  55_u16
      RADIUS_BR           =  56_u16
      RADIUS_BL           =  57_u16
      SHADOW_1            =  58_u16
      SHADOW_2            =  59_u16
      OPACITY             =  60_u16
      OUTLINE             =  61_u16
      TRANSLATE           =  62_u16
      SCALE               =  63_u16
      ROTATE              =  64_u16
      CURSOR              =  65_u16
      POINTER_EVENTS      =  66_u16
      VISIBILITY          =  67_u16
      BACKDROP            =  68_u16
      COLOR               = 100_u16
      FONT_FAMILY         = 101_u16
      FONT_SIZE           = 102_u16
      FONT_WEIGHT         = 103_u16
      FONT_STYLE          = 104_u16
      LINE_HEIGHT         = 105_u16
      LETTER_SPACING      = 106_u16
      TEXT_ALIGN          = 107_u16
      TEXT_DECORATION     = 108_u16
      TEXT_OVERFLOW       = 109_u16
      WHITE_SPACE         = 110_u16
      MAX_LINES           = 111_u16
      FONT_FEATURE        = 112_u16
      USER_SELECT         = 113_u16
      TRANSITION          = 140_u16
      SPRING              = 141_u16
      SLIDER_MIN          = 150_u16
      SLIDER_MAX          = 151_u16
      SLIDER_VALUE        = 152_u16
      SLIDER_STEP         = 153_u16
      PLACEHOLDER         = 154_u16
      MAX_LENGTH          = 155_u16
      SECURE              = 156_u16
      READONLY            = 157_u16
      IMAGE_SOURCE        = 158_u16
      IMAGE_ID            = 159_u16
      IMAGE_FIT           = 160_u16
      SCROLL_OFFSET       = 161_u16
      TOOLTIP             = 162_u16
      FOCUSABLE           = 163_u16
      ANCHOR              = 164_u16
      ANCHOR_SIDE         = 165_u16
      ANCHOR_ALIGN        = 166_u16
      MODAL               = 167_u16
      LIST_COUNT          = 190_u16
      LIST_HEIGHT         = 191_u16
      LIST_OVERSCAN       = 192_u16
    end

    # Property wire types, by property name: {id, type}. `enum` types name their table in ENUMS.
    PROPS = {
      "display"             => {id: 1, ty: "enum", en: "display"},
      "flex-direction"      => {id: 2, ty: "enum", en: "direction"},
      "flex-wrap"           => {id: 3, ty: "enum", en: "wrap"},
      "grow"                => {id: 4, ty: "f32", en: nil},
      "shrink"              => {id: 5, ty: "f32", en: nil},
      "basis"               => {id: 6, ty: "dim", en: nil},
      "gap"                 => {id: 7, ty: "f32", en: nil},
      "row-gap"             => {id: 8, ty: "f32", en: nil},
      "column-gap"          => {id: 9, ty: "f32", en: nil},
      "align-items"         => {id: 10, ty: "enum", en: "align"},
      "align-self"          => {id: 11, ty: "enum", en: "self_align"},
      "align-content"       => {id: 12, ty: "enum", en: "content"},
      "justify-content"     => {id: 13, ty: "enum", en: "justify"},
      "padding"             => {id: 14, ty: "f32", en: nil},
      "padding-top"         => {id: 15, ty: "f32", en: nil},
      "padding-right"       => {id: 16, ty: "f32", en: nil},
      "padding-bottom"      => {id: 17, ty: "f32", en: nil},
      "padding-left"        => {id: 18, ty: "f32", en: nil},
      "margin"              => {id: 19, ty: "dim", en: nil},
      "margin-top"          => {id: 20, ty: "dim", en: nil},
      "margin-right"        => {id: 21, ty: "dim", en: nil},
      "margin-bottom"       => {id: 22, ty: "dim", en: nil},
      "margin-left"         => {id: 23, ty: "dim", en: nil},
      "width"               => {id: 24, ty: "dim", en: nil},
      "height"              => {id: 25, ty: "dim", en: nil},
      "min-width"           => {id: 26, ty: "dim", en: nil},
      "min-height"          => {id: 27, ty: "dim", en: nil},
      "max-width"           => {id: 28, ty: "dim", en: nil},
      "max-height"          => {id: 29, ty: "dim", en: nil},
      "aspect-ratio"        => {id: 30, ty: "f32", en: nil},
      "position"            => {id: 31, ty: "enum", en: "position"},
      "inset-top"           => {id: 32, ty: "dim", en: nil},
      "inset-right"         => {id: 33, ty: "dim", en: nil},
      "inset-bottom"        => {id: 34, ty: "dim", en: nil},
      "inset-left"          => {id: 35, ty: "dim", en: nil},
      "overflow"            => {id: 36, ty: "enum", en: "overflow"},
      "z-index"             => {id: 37, ty: "i32", en: nil},
      "tab-index"           => {id: 38, ty: "i32", en: nil},
      "background"          => {id: 50, ty: "token", en: nil},
      "background-gradient" => {id: 51, ty: "gradient", en: nil},
      "border"              => {id: 52, ty: "border", en: nil},
      "border-radius"       => {id: 53, ty: "f32", en: nil},
      "radius-top-left"     => {id: 54, ty: "f32", en: nil},
      "radius-top-right"    => {id: 55, ty: "f32", en: nil},
      "radius-bottom-right" => {id: 56, ty: "f32", en: nil},
      "radius-bottom-left"  => {id: 57, ty: "f32", en: nil},
      "box-shadow-1"        => {id: 58, ty: "shadow", en: nil},
      "box-shadow-2"        => {id: 59, ty: "shadow", en: nil},
      "opacity"             => {id: 60, ty: "f32", en: nil},
      "outline"             => {id: 61, ty: "border", en: nil},
      "transform-translate" => {id: 62, ty: "pair", en: nil},
      "transform-scale"     => {id: 63, ty: "pair", en: nil},
      "transform-rotate"    => {id: 64, ty: "f32", en: nil},
      "cursor"              => {id: 65, ty: "enum", en: "cursor"},
      "pointer-events"      => {id: 66, ty: "enum", en: "pointer"},
      "visibility"          => {id: 67, ty: "enum", en: "visibility"},
      "backdrop"            => {id: 68, ty: "enum", en: "backdrop"},
      "color"               => {id: 100, ty: "token", en: nil},
      "font-family"         => {id: 101, ty: "enum", en: "family"},
      "font-size"           => {id: 102, ty: "enum", en: "size"},
      "font-weight"         => {id: 103, ty: "enum", en: "weight"},
      "font-style"          => {id: 104, ty: "enum", en: "fstyle"},
      "line-height"         => {id: 105, ty: "f32", en: nil},
      "letter-spacing"      => {id: 106, ty: "f32", en: nil},
      "text-align"          => {id: 107, ty: "enum", en: "talign"},
      "text-decoration"     => {id: 108, ty: "enum", en: "decoration"},
      "text-overflow"       => {id: 109, ty: "enum", en: "toverflow"},
      "white-space"         => {id: 110, ty: "enum", en: "whitespace"},
      "max-lines"           => {id: 111, ty: "i32", en: nil},
      "font-feature"        => {id: 112, ty: "enum", en: "feature"},
      "user-select"         => {id: 113, ty: "enum", en: "select"},
      "transition"          => {id: 140, ty: "transition", en: nil},
      "spring"              => {id: 141, ty: "enum", en: "spring"},
      "slider-min"          => {id: 150, ty: "f32", en: nil},
      "slider-max"          => {id: 151, ty: "f32", en: nil},
      "slider-value"        => {id: 152, ty: "f32", en: nil},
      "slider-step"         => {id: 153, ty: "f32", en: nil},
      "placeholder"         => {id: 154, ty: "str", en: nil},
      "max-length"          => {id: 155, ty: "i32", en: nil},
      "secure"              => {id: 156, ty: "bool", en: nil},
      "readonly"            => {id: 157, ty: "bool", en: nil},
      "image-source"        => {id: 158, ty: "str", en: nil},
      "image-id"            => {id: 159, ty: "u32", en: nil},
      "image-fit"           => {id: 160, ty: "enum", en: "fit"},
      "scroll-offset"       => {id: 161, ty: "pair", en: nil},
      "tooltip"             => {id: 162, ty: "str", en: nil},
      "focusable"           => {id: 163, ty: "bool", en: nil},
      "anchor"              => {id: 164, ty: "u32", en: nil},
      "anchor-side"         => {id: 165, ty: "enum", en: "side"},
      "anchor-align"        => {id: 166, ty: "enum", en: "anchor_align"},
      "modal"               => {id: 167, ty: "bool", en: nil},
      "list-count"          => {id: 190, ty: "u32", en: nil},
      "list-height"         => {id: 191, ty: "f32", en: nil},
      "list-overscan"       => {id: 192, ty: "u32", en: nil},
    }

    # The values of the enum properties, in wire order.
    ENUMS = {
      "display"      => ["flex", "none"],
      "direction"    => ["row", "column", "row-reverse", "column-reverse"],
      "wrap"         => ["nowrap", "wrap"],
      "align"        => ["start", "center", "end", "stretch", "baseline"],
      "self_align"   => ["auto", "start", "center", "end", "stretch", "baseline"],
      "content"      => ["start", "center", "end", "stretch", "space-between", "space-around"],
      "justify"      => ["start", "center", "end", "space-between", "space-around", "space-evenly"],
      "position"     => ["relative", "absolute"],
      "overflow"     => ["visible", "hidden", "scroll", "auto"],
      "cursor"       => ["default", "pointer", "text", "grab", "grabbing", "not-allowed"],
      "pointer"      => ["auto", "none"],
      "visibility"   => ["visible", "hidden"],
      "backdrop"     => ["none", "glass"],
      "family"       => ["ui", "mono"],
      "size"         => ["display", "large", "title", "body", "secondary", "caption"],
      "weight"       => ["400", "500", "600", "700"],
      "fstyle"       => ["normal", "italic"],
      "talign"       => ["start", "center", "end", "justify"],
      "decoration"   => ["none", "underline", "strikethrough"],
      "toverflow"    => ["clip", "ellipsis"],
      "whitespace"   => ["normal", "nowrap", "pre-wrap"],
      "feature"      => ["none", "tnum"],
      "select"       => ["none", "text"],
      "spring"       => ["gentle", "snappy", "bouncy"],
      "fit"          => ["contain", "cover", "fill"],
      "side"         => ["bottom", "top", "right", "left"],
      "anchor_align" => ["start", "center", "end"],
    }
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# The element and property tables the DSL macros read at compile time. A macro sees only syntax, so
# what a property is called, what it takes and which engine property it writes is data here, kept in
# constants of literals. Adding a property is one row (plus a row in the engine's table if it is
# new); adding an element is one row and its name in ELEMENT_NAMES.
module Wolkit
  module DSL
    # How a DSL property becomes a wire value. `wire` names the engine property (Wire::PROPS key);
    # `conv` is the conversion: space, radius, f32, i32, u32, bool, str, enum, dim, token, pair,
    # border, weight.
    PROPS = {
      # layout
      "direction"     => {wire: "flex-direction", conv: "enum"},
      "wrap"          => {wire: "flex-wrap", conv: "enum"},
      "grow"          => {wire: "grow", conv: "f32"},
      "shrink"        => {wire: "shrink", conv: "f32"},
      "basis"         => {wire: "basis", conv: "dim"},
      "gap"           => {wire: "gap", conv: "space"},
      "row_gap"       => {wire: "row-gap", conv: "space"},
      "column_gap"    => {wire: "column-gap", conv: "space"},
      "align"         => {wire: "align-items", conv: "enum"},
      "align_self"    => {wire: "align-self", conv: "enum"},
      "align_content" => {wire: "align-content", conv: "enum"},
      "justify"       => {wire: "justify-content", conv: "enum"},
      "pad"           => {wire: "padding", conv: "space"},
      "pad_top"       => {wire: "padding-top", conv: "space"},
      "pad_right"     => {wire: "padding-right", conv: "space"},
      "pad_bottom"    => {wire: "padding-bottom", conv: "space"},
      "pad_left"      => {wire: "padding-left", conv: "space"},
      "margin"        => {wire: "margin", conv: "dim"},
      "margin_top"    => {wire: "margin-top", conv: "dim"},
      "margin_right"  => {wire: "margin-right", conv: "dim"},
      "margin_bottom" => {wire: "margin-bottom", conv: "dim"},
      "margin_left"   => {wire: "margin-left", conv: "dim"},
      "width"         => {wire: "width", conv: "dim"},
      "height"        => {wire: "height", conv: "dim"},
      "min_width"     => {wire: "min-width", conv: "dim"},
      "min_height"    => {wire: "min-height", conv: "dim"},
      "max_width"     => {wire: "max-width", conv: "dim"},
      "max_height"    => {wire: "max-height", conv: "dim"},
      "aspect"        => {wire: "aspect-ratio", conv: "f32"},
      "position"      => {wire: "position", conv: "enum"},
      "top"           => {wire: "inset-top", conv: "dim"},
      "right"         => {wire: "inset-right", conv: "dim"},
      "bottom"        => {wire: "inset-bottom", conv: "dim"},
      "left"          => {wire: "inset-left", conv: "dim"},
      "overflow"      => {wire: "overflow", conv: "enum"},
      "z"             => {wire: "z-index", conv: "i32"},
      "tab_index"     => {wire: "tab-index", conv: "i32"},
      # paint
      "bg"             => {wire: "background", conv: "token"},
      "border"         => {wire: "border", conv: "border"},
      "radius"         => {wire: "border-radius", conv: "radius"},
      "opacity"        => {wire: "opacity", conv: "f32"},
      "outline"        => {wire: "outline", conv: "border"},
      "translate"      => {wire: "transform-translate", conv: "pair"},
      "scale"          => {wire: "transform-scale", conv: "pair"},
      "rotate"         => {wire: "transform-rotate", conv: "f32"},
      "cursor"         => {wire: "cursor", conv: "enum"},
      "pointer_events" => {wire: "pointer-events", conv: "enum"},
      "visibility"     => {wire: "visibility", conv: "enum"},
      "backdrop"       => {wire: "backdrop", conv: "enum"},
      # text
      "color"          => {wire: "color", conv: "token"},
      "font"           => {wire: "font-family", conv: "enum"},
      "size"           => {wire: "font-size", conv: "enum"},
      "weight"         => {wire: "font-weight", conv: "weight"},
      "line_height"    => {wire: "line-height", conv: "f32"},
      "letter_spacing" => {wire: "letter-spacing", conv: "f32"},
      "text_align"     => {wire: "text-align", conv: "enum"},
      "decoration"     => {wire: "text-decoration", conv: "enum"},
      "text_overflow"  => {wire: "text-overflow", conv: "enum"},
      "white_space"    => {wire: "white-space", conv: "enum"},
      "max_lines"      => {wire: "max-lines", conv: "i32"},
      "tnum"           => {wire: "font-feature", conv: "bool_tnum"},
      "italic"         => {wire: "font-style", conv: "bool_italic"},
      "selectable"     => {wire: "user-select", conv: "bool_text"},
      # motion
      "spring" => {wire: "spring", conv: "enum"},
      "role"   => {wire: "font-size", conv: "enum"},
      # part specific
      "placeholder"  => {wire: "placeholder", conv: "str"},
      "max_length"   => {wire: "max-length", conv: "i32"},
      "secure"       => {wire: "secure", conv: "bool"},
      "readonly"     => {wire: "readonly", conv: "bool"},
      "source"       => {wire: "image-source", conv: "str"},
      "fit"          => {wire: "image-fit", conv: "enum"},
      "tooltip"      => {wire: "tooltip", conv: "str"},
      "focusable"    => {wire: "focusable", conv: "bool"},
      "anchor"       => {wire: "anchor", conv: "u32"},
      "side"         => {wire: "anchor-side", conv: "enum"},
      "anchor_align" => {wire: "anchor-align", conv: "enum"},
      "modal"        => {wire: "modal", conv: "bool"},
      "min"          => {wire: "slider-min", conv: "f32"},
      "max"          => {wire: "slider-max", conv: "f32"},
      "step"         => {wire: "slider-step", conv: "f32"},
    }

    # Properties every element takes.
    COMMON = %w(direction wrap grow shrink basis gap row_gap column_gap align align_self align_content justify
      pad pad_top pad_right pad_bottom pad_left margin margin_top margin_right margin_bottom margin_left
      width height min_width min_height max_width max_height aspect position top right bottom left overflow z
      tab_index bg border radius opacity outline translate scale rotate cursor pointer_events visibility
      backdrop color size weight line_height letter_spacing text_align decoration text_overflow white_space
      max_lines tnum italic selectable spring tooltip focusable)

    # Events every element takes: `on_key: ->(key : Wolkit::KeyPress) { }` hears the keys pressed
    # while it or a part inside it has focus (dsl/keys.cr); `on_focus: ->(focused : Bool) { }` hears
    # focus arrive in it or leave it.
    COMMON_EVENTS = {"on_key" => {ev: "KEY", arg: "key"}, "on_focus" => {ev: "FOCUS", arg: "flag"}}

    # The state flags: one SET_STATE mask (the app-owned bits of the engine's state).
    STATES = {"disabled" => 16, "selected" => 32, "checked" => 64, "expanded" => 128, "busy" => 512}

    # Accessibility roles by DSL name (a11y_role:).
    ROLES = %w(none group label button toggle checkbox slider textbox image list listitem heading tab tablist
      tabpanel menu menuitem dialog alert separator scrollview link progress)

    # Elements. `kind` is the node kind; `a11y` the default accessibility role (NONE: none unless `label:` is given); `dir` the flex direction to set (0 row, 1 column) or -1;
    # `pos` what the first positional argument is (text, icon, signal, none); `props` the extra
    # properties; `events` the events it takes (DSL name => engine event and the payload accessor a one-argument proc gets);
    # `label` says a name for assistive technology is required.
    ELEMENTS = {
      "box"         => {kind: "BOX", a11y: "NONE", dir: -1, pos: "none", props: %w(), events: {"on_click" => {ev: "CLICK", arg: nil}, "on_hover" => {ev: "POINTER_ENTER", arg: nil}}, label: false},
      "column"      => {kind: "BOX", a11y: "NONE", dir: 1, pos: "none", props: %w(), events: {"on_click" => {ev: "CLICK", arg: nil}, "on_hover" => {ev: "POINTER_ENTER", arg: nil}}, label: false},
      "row"         => {kind: "BOX", a11y: "NONE", dir: 0, pos: "none", props: %w(), events: {"on_click" => {ev: "CLICK", arg: nil}, "on_hover" => {ev: "POINTER_ENTER", arg: nil}}, label: false},
      "scroll"      => {kind: "SCROLL", a11y: "SCROLLVIEW", dir: 1, pos: "none", props: %w(), events: {"on_scroll" => {ev: "SCROLL", arg: "scroll"}}, label: false},
      "spacer"      => {kind: "SPACER", a11y: "NONE", dir: -1, pos: "none", props: %w(), events: {"" => nil}, label: false},
      "divider"     => {kind: "DIVIDER", a11y: "SEPARATOR", dir: -1, pos: "none", props: %w(), events: {"" => nil}, label: false},
      "text"        => {kind: "TEXT", a11y: "NONE", dir: -1, pos: "text", props: %w(role), events: {"" => nil}, label: false},
      "image"       => {kind: "IMAGE", a11y: "IMAGE", dir: -1, pos: "none", props: %w(source fit), events: {"" => nil}, label: true},
      "button"      => {kind: "BUTTON", a11y: "BUTTON", dir: 0, pos: "text", props: %w(primary danger flat), events: {"on_click" => {ev: "CLICK", arg: nil}, "on_hover" => {ev: "POINTER_ENTER", arg: nil}}, label: false},
      "icon_button" => {kind: "BUTTON", a11y: "BUTTON", dir: 0, pos: "icon", props: %w(primary danger flat), events: {"on_click" => {ev: "CLICK", arg: nil}, "on_hover" => {ev: "POINTER_ENTER", arg: nil}}, label: true},
      "toggle"      => {kind: "TOGGLE", a11y: "TOGGLE", dir: -1, pos: "bool", props: %w(), events: {"on_toggle" => {ev: "TOGGLE", arg: "flag"}}, label: true},
      "slider"      => {kind: "SLIDER", a11y: "SLIDER", dir: -1, pos: "number", props: %w(min max step), events: {"on_change" => {ev: "SLIDER", arg: "number"}}, label: true},
      "text_field"  => {kind: "TEXT_INPUT", a11y: "TEXTBOX", dir: -1, pos: "string", props: %w(placeholder max_length secure readonly), events: {"on_change" => {ev: "TEXT_CHANGED", arg: "text"}, "on_submit" => {ev: "TEXT_SUBMIT", arg: "text"}}, label: false},
      "vlist"       => {kind: "VLIST", a11y: "LIST", dir: 1, pos: "none", props: %w(), events: {"on_range" => {ev: "LIST_RANGE", arg: "range"}, "on_scroll" => {ev: "SCROLL", arg: "scroll"}}, label: false},
      "canvas"      => {kind: "CANVAS", a11y: "IMAGE", dir: -1, pos: "none", props: %w(), events: {"on_frame" => {ev: "FRAME_TICK", arg: nil}}, label: true},
      "overlay"     => {kind: "OVERLAY", a11y: "NONE", dir: 1, pos: "none", props: %w(anchor side anchor_align modal), events: {"" => nil}, label: false},
    }

    # The Kit style variants a button takes (`primary: true`): DSL name => class constant of
    # Wolkit::KitClass.
    VARIANTS = {"primary" => "PRIMARY", "danger" => "DANGER", "flat" => "FLAT"}
  end
end

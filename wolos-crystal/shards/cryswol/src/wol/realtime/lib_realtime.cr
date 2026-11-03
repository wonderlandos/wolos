# SPDX-License-Identifier: GPL-2.0-or-later
# Generated from crates/realtime/include/wol_realtime.h by crates/wol-abi/tests/bindings.rs; do not edit.
# Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings
# No @[Link]: the library is a static archive; the program passes its own --link-flags.
lib LibRealtime
  WOL_ANIM_OPEN               =         0
  WOL_ANIM_CLOSE              =         1
  WOL_PLACE_SKIP              =         0
  WOL_PLACE_CLAMP_ONLY        =         1
  WOL_PLACE                   =         2
  WOL_FIXED_W                 =     1_u32
  WOL_FIXED_H                 =     2_u32
  WOL_KEY_SUPER               =         0
  WOL_KEY_TAB                 =         1
  WOL_KEY_ESC                 =         2
  WOL_KEY_OTHER               =         3
  WOL_KEYF_PRESSED            =     1_u32
  WOL_KEYF_SUPER_HELD         =     2_u32
  WOL_KEYF_SHIFT              =     4_u32
  WOL_KEYF_OTHER_MODS         =     8_u32
  WOL_KEYF_LOCKED             =    16_u32
  WOL_KEYF_SWITCHER_MAPPED    =    32_u32
  WOL_KEYF_SHELL_LAYER_MAPPED =    64_u32
  WOL_LINE_NONE               =         0
  WOL_ACTION_CONSUME          = 0x100_u32

  struct WolVisual
    scale : Float32
    opacity : Float32
  end

  fun wol_anim_sample(kind : UInt32, start_ms : UInt64, now_ms : UInt64, out_ : WolVisual*) : Int32
  fun wol_anim_duration_ms(kind : UInt32) : UInt64
  fun wol_titlebar_hit(width : Int32, x : Int32, y : Int32) : Int32
  fun wol_titlebar_height : Int32
  fun wol_lights_at(count : Int32, x : Int32, y : Int32) : Int32
  fun wol_lights_over(count : Int32, x : Int32, y : Int32) : Int32
  fun wol_lights_at_w(width : Int32, count : Int32, x : Int32, y : Int32) : Int32
  fun wol_lights_over_w(width : Int32, count : Int32, x : Int32, y : Int32) : Int32
  fun wol_lights_slot_w(slot : Int32) : Int32
  fun wol_lights_hover(state : UInt32, over : Int32, held : Int32) : UInt32
  fun wol_lights_glyphs(state : UInt32) : Int32
  fun wol_window_lights(class_ : UInt8*, decoration : Int32, transient : Int32, fixed_size : Int32) : Int32
  fun wol_titlebar_button_origin(button : Int32, x : Int32*, y : Int32*) : Int32
  fun wol_place(w : Int32, h : Int32, ax : Int32, ay : Int32, aw : Int32, ah : Int32, bar : Int32, has_prev : Int32, px : Int32, py : Int32, x : Int32*, y : Int32*) : Int32
  fun wol_fit(w : Int32, h : Int32, min_w : Int32, min_h : Int32, max_w : Int32, max_h : Int32, ax : Int32, ay : Int32, aw : Int32, ah : Int32, bar : Int32, out_w : Int32*, out_h : Int32*) : Int32
  fun wol_centre(w : Int32, h : Int32, px : Int32, py : Int32, pw : Int32, ph : Int32, ax : Int32, ay : Int32, aw : Int32, ah : Int32, bar : Int32, x : Int32*, y : Int32*) : Int32
  fun wol_refit(min_w : Int32, min_h : Int32, max_w : Int32, max_h : Int32, ax : Int32, ay : Int32, aw : Int32, ah : Int32, bar : Int32, x : Int32*, y : Int32*, w : Int32*, h : Int32*) : Int32
  fun wol_x11_placement(x11 : Int32, override_redirect : Int32, no_borders : Int32, size_hint_flags : UInt32) : Int32
  fun wol_x11_managed(x11 : Int32, override_redirect : Int32, no_borders : Int32) : Int32
  fun wol_fixed_axes(min_w : Int32, min_h : Int32, max_w : Int32, max_h : Int32) : UInt32
  fun wol_placement_limits(cur_w : Int32, cur_h : Int32, min_w : Int32*, min_h : Int32*, max_w : Int32*, max_h : Int32*) : Int32
  fun wol_keys_step(state : UInt32, key : Int32, flags : UInt32, action : UInt32*) : UInt32
  fun wol_keys_line(code : Int32) : UInt8*
  fun wol_qos_foreground(pid : Int32) : Int32
  fun wol_frame_record(begin_ns : UInt64, end_ns : UInt64, damage_px : UInt64, screen_px : UInt64, passes : UInt32)
  fun wol_frame_stats_json(buf : UInt8*, len : LibC::SizeT) : Int32
end

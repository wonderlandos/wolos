# SPDX-License-Identifier: GPL-2.0-or-later
# Generated from include/wol.h by crates/wol-abi/tests/bindings.rs; do not edit.
# Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings
@[Link("wol")]
lib LibWol
  WOL_VERSION_MAJOR = 1
  WOL_VERSION_MINOR = 1
  WOL_VERSION_PATCH = 0
  WOL_OK = 0
  WOL_ERR_INVALID_ARGUMENT = 1
  WOL_ERR_PERMISSION_DENIED = 2
  WOL_ERR_UNAVAILABLE = 3
  WOL_ERR_INTERNAL = 4
  WOL_APPEARANCE_LIGHT = 0
  WOL_APPEARANCE_DARK = 1

  alias Status = Int32
  alias Appearance = Int32
  # Handles are opaque: the field exists only because a Crystal struct cannot be empty.
  struct Error; opaque : UInt8; end
  struct String; opaque : UInt8; end
  struct Context; opaque : UInt8; end
  struct Event; opaque : UInt8; end
  struct Subscription; opaque : UInt8; end
  struct SystemInfo; opaque : UInt8; end
  struct Window; opaque : UInt8; end
  struct Widget; opaque : UInt8; end
  struct Notification; opaque : UInt8; end

  alias EventFn = Event*, Void* -> Void
  alias ClickFn = Widget*, Void* -> Void
  alias ChangeFn = Widget*, Void* -> Void
  alias WindowFn = Window*, Void* -> Void
  alias ActionFn = Notification*, String*, Void* -> Void

  fun wol_version() : UInt32
  fun wol_version_string() : UInt8*
  fun wol_error_get_code(error : Error*) : Status
  fun wol_error_get_message(error : Error*, len : LibC::SizeT*) : UInt8*
  fun wol_error_release(error : Error*)
  fun wol_string_create(utf8 : UInt8*, len : LibC::SizeT, out_ : String**, error : Error**) : Status
  fun wol_string_get_bytes(string : String*, len : LibC::SizeT*) : UInt8*
  fun wol_string_retain(string : String*) : String*
  fun wol_string_release(string : String*)
  fun wol_context_create(app_id : UInt8*, len : LibC::SizeT, out_ : Context**, error : Error**) : Status
  fun wol_context_retain(context : Context*) : Context*
  fun wol_context_release(context : Context*)
  fun wol_context_get_fd(context : Context*) : Int32
  fun wol_context_dispatch(context : Context*, timeout_ms : Int32, error : Error**) : Status
  fun wol_subscribe(context : Context*, topic : UInt8*, len : LibC::SizeT, callback : EventFn, user_data : Void*, out_ : Subscription**, error : Error**) : Status
  fun wol_subscription_cancel(subscription : Subscription*)
  fun wol_subscription_release(subscription : Subscription*)
  fun wol_event_get_topic(event : Event*) : String*
  fun wol_event_get_json(event : Event*) : String*
  fun wol_event_retain(event : Event*) : Event*
  fun wol_event_release(event : Event*)
  fun wol_system_info_copy(context : Context*, out_ : SystemInfo**, error : Error**) : Status
  fun wol_system_info_get_os_name(info : SystemInfo*) : String*
  fun wol_system_info_get_os_version(info : SystemInfo*) : String*
  fun wol_system_info_get_kernel(info : SystemInfo*) : String*
  fun wol_system_info_get_appearance(info : SystemInfo*) : Appearance
  fun wol_system_info_get_accent_name(info : SystemInfo*) : String*
  fun wol_system_info_get_accent_rgb(info : SystemInfo*) : UInt32
  fun wol_system_info_retain(info : SystemInfo*) : SystemInfo*
  fun wol_system_info_release(info : SystemInfo*)
  fun wol_notification_post(context : Context*, title : UInt8*, title_len : LibC::SizeT, body : UInt8*, body_len : LibC::SizeT, error : Error**) : Status
  fun wol_window_create(context : Context*, title : UInt8*, len : LibC::SizeT, out_ : Window**, error : Error**) : Status
  fun wol_window_set_content(window : Window*, content : Widget*, error : Error**) : Status
  fun wol_window_show(window : Window*, error : Error**) : Status
  fun wol_window_retain(window : Window*) : Window*
  fun wol_window_release(window : Window*)
  fun wol_stack_create(context : Context*, spacing : UInt32, out_ : Widget**, error : Error**) : Status
  fun wol_label_create(context : Context*, text : UInt8*, len : LibC::SizeT, out_ : Widget**, error : Error**) : Status
  fun wol_button_create(context : Context*, label : UInt8*, len : LibC::SizeT, on_click : ClickFn, user_data : Void*, out_ : Widget**, error : Error**) : Status
  fun wol_widget_add_child(parent : Widget*, child : Widget*, error : Error**) : Status
  fun wol_widget_retain(widget : Widget*) : Widget*
  fun wol_widget_release(widget : Widget*)
  fun wol_row_create(context : Context*, spacing : UInt32, out_ : Widget**, error : Error**) : Status
  fun wol_list_create(context : Context*, out_ : Widget**, error : Error**) : Status
  fun wol_spacer_create(context : Context*, out_ : Widget**, error : Error**) : Status
  fun wol_text_field_create(context : Context*, placeholder : UInt8*, len : LibC::SizeT, on_change : ChangeFn, user_data : Void*, out_ : Widget**, error : Error**) : Status
  fun wol_toggle_create(context : Context*, label : UInt8*, len : LibC::SizeT, on : Int32, on_change : ChangeFn, user_data : Void*, out_ : Widget**, error : Error**) : Status
  fun wol_slider_create(context : Context*, min : Float64, max : Float64, value : Float64, on_change : ChangeFn, user_data : Void*, out_ : Widget**, error : Error**) : Status
  fun wol_image_create(context : Context*, source : UInt8*, len : LibC::SizeT, size : UInt32, out_ : Widget**, error : Error**) : Status
  fun wol_widget_set_text(widget : Widget*, text : UInt8*, len : LibC::SizeT, error : Error**) : Status
  fun wol_widget_copy_text(widget : Widget*, out_ : String**, error : Error**) : Status
  fun wol_toggle_get_on(widget : Widget*) : Int32
  fun wol_toggle_set_on(widget : Widget*, on : Int32, error : Error**) : Status
  fun wol_slider_get_value(widget : Widget*) : Float64
  fun wol_slider_set_value(widget : Widget*, value : Float64, error : Error**) : Status
  fun wol_window_set_close_handler(window : Window*, on_close : WindowFn, user_data : Void*, error : Error**) : Status
  fun wol_window_close(window : Window*, error : Error**) : Status
  fun wol_notification_create(context : Context*, title : UInt8*, title_len : LibC::SizeT, body : UInt8*, body_len : LibC::SizeT, out_ : Notification**, error : Error**) : Status
  fun wol_notification_add_action(notification : Notification*, key : UInt8*, key_len : LibC::SizeT, label : UInt8*, label_len : LibC::SizeT, error : Error**) : Status
  fun wol_notification_set_action_handler(notification : Notification*, on_action : ActionFn, user_data : Void*, error : Error**) : Status
  fun wol_notification_send(notification : Notification*, error : Error**) : Status
  fun wol_notification_retain(notification : Notification*) : Notification*
  fun wol_notification_release(notification : Notification*)
end

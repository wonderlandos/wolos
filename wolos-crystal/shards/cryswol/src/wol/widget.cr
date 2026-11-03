# SPDX-License-Identifier: BSD-2-Clause
# Kit parts (wol.h "Windows and Kit parts" and "1.1: more Kit parts"; ADR 0020, ADR 0036).
#
# Concept: a part is data. libwol keeps the tree and sends it to the Kit host, which draws it and
# sends back what the person did. Why the rooting rules below: libwol calls a part's callback with
# a raw `user_data` pointer the GC cannot see, so a part with a callback is kept alive by its
# context (`Handle#root!`), and once it sits inside a stack or a window it stays rooted even
# after `close`, because the parent in libwol still holds it and can still deliver to it. How:
# each part class wraps its `wol_*_create` call; the callback ones use a class-level trampoline
# that captures nothing (`Callbacks`).
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"
require "./context"

module Wol
  # One Kit part. Make one with a subclass (`Label`, `Button`, `Stack`, ...) and put it in a
  # container with `Container#add` or in a window with `Window#content=`. A part goes in one place.
  abstract class Widget < Handle(LibWol::Widget)
    @placed = false

    # True once the part was put in a container or a window.
    def placed? : Bool
      @placed
    end

    # :nodoc:
    # Called by `Container#add` and `Window#content=` after libwol accepted the part.
    def placed! : Nil
      @placed = true
    end

    # Release the part. A part with a callback that is inside a container or a window stays
    # rooted: libwol's parent still holds it and may still call it.
    def close : Nil
      keep = !closed? && @placed && callback?
      super
      root! if keep
    end

    # True when this part registered a callback with libwol.
    protected def callback? : Bool
      false
    end

    protected def release(ptr : LibWol::Widget*) : Nil
      LibWol.wol_widget_release(ptr)
    end
  end

  # Text a part shows: a label's text, a button's label, a text field's text.
  module WidgetText
    # The current text (for a text field, what the person typed).
    def text : ::String
      Strings.take(Wol.out(LibWol::String) { |slot, err| LibWol.wol_widget_copy_text(self, slot, err) }) || ""
    end

    # Replace the text. Redraws the window the part is in, if it is shown.
    def text=(text : ::String) : ::String
      Wol.call { |err| LibWol.wol_widget_set_text(self, Strings.ptr(text), Strings.size(text), err) }
      text
    end
  end

  # A part that holds other parts in order.
  abstract class Container < Widget
    @children = [] of Widget

    # The parts added so far, in order.
    def children : Array(Widget)
      @children.dup
    end

    # Add `child` at the end. Raises `InvalidArgument` when the part is already placed.
    def add(child : Widget) : self
      Wol.call { |err| LibWol.wol_widget_add_child(self, child, err) }
      @children << child
      child.placed!
      self
    end

    # Same as `add`.
    def <<(child : Widget) : self
      add(child)
    end
  end

  # Parts one above the other. Spacing in points: 0, 4, 8, 12, 16 or 24.
  class Stack < Container
    # Raises `InvalidArgument` for another spacing.
    def initialize(context : Context, spacing : Int = 0)
      super(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_stack_create(context, spacing.to_u32!, slot, err)
      end, context)
    end
  end

  # Parts side by side. Spacing in points: 0, 4, 8, 12, 16 or 24.
  class Row < Container
    # Raises `InvalidArgument` for another spacing.
    def initialize(context : Context, spacing : Int = 0)
      super(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_row_create(context, spacing.to_u32!, slot, err)
      end, context)
    end
  end

  # Rows with separators that scroll when long.
  class List < Container
    def initialize(context : Context)
      super(Wol.out(LibWol::Widget) { |slot, err| LibWol.wol_list_create(context, slot, err) }, context)
    end
  end

  # Takes the free space in a row or a stack.
  class Spacer < Widget
    def initialize(context : Context)
      super(Wol.out(LibWol::Widget) { |slot, err| LibWol.wol_spacer_create(context, slot, err) }, context)
    end
  end

  # Text to read.
  class Label < Widget
    include WidgetText

    def initialize(context : Context, text : ::String)
      super(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_label_create(context, Strings.ptr(text), Strings.size(text), slot, err)
      end, context)
    end
  end

  # An icon by name ("weather-clear-symbolic") or an absolute path to a .png, .svg or .jpg.
  # Size in pixels: 16, 24, 32, 48, 64, 96 or 128.
  class Image < Widget
    # Raises `InvalidArgument` for a bad source or size.
    def initialize(context : Context, source : ::String, size : Int = 24)
      super(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_image_create(context, Strings.ptr(source), Strings.size(source), size.to_u32!, slot, err)
      end, context)
    end
  end

  # A push button. The block runs inside dispatch each time the person clicks it.
  class Button < Widget
    include WidgetText

    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    TRAMPOLINE = ->(_widget : LibWol::Widget*, data : Void*) do
      Callbacks.invoke(Button, data, &.fire)
      nil
    end

    @on_click : (Button -> Nil)? = nil

    # A button with no click handler.
    def initialize(context : Context, label : ::String)
      @context = context
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_button_create(context, Strings.ptr(label), Strings.size(label), nil, nil, slot, err)
      end)
    end

    # A button that calls the block on every click.
    def initialize(context : Context, label : ::String, &on_click : Button -> Nil)
      @context = context
      @on_click = on_click
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_button_create(context, Strings.ptr(label), Strings.size(label), TRAMPOLINE,
          Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    protected def callback? : Bool
      !@on_click.nil?
    end

    protected def fire : Nil
      @on_click.try &.call(self)
    end
  end

  # One line of text the person types; starts empty. The block runs after each edit.
  class TextField < Widget
    include WidgetText

    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    TRAMPOLINE = ->(_widget : LibWol::Widget*, data : Void*) do
      Callbacks.invoke(TextField, data, &.fire)
      nil
    end

    @on_change : (TextField -> Nil)? = nil

    # A field with grey `placeholder` text and no change handler.
    def initialize(context : Context, placeholder : ::String = "")
      @context = context
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_text_field_create(context, Strings.ptr(placeholder), Strings.size(placeholder), nil, nil, slot, err)
      end)
    end

    # A field that calls the block after the person changed the text (read it with `text`).
    def initialize(context : Context, placeholder : ::String = "", &on_change : TextField -> Nil)
      @context = context
      @on_change = on_change
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_text_field_create(context, Strings.ptr(placeholder), Strings.size(placeholder), TRAMPOLINE,
          Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    protected def callback? : Bool
      !@on_change.nil?
    end

    protected def fire : Nil
      @on_change.try &.call(self)
    end
  end

  # An on/off switch with an optional label. The block runs after the person flips it.
  class Toggle < Widget
    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    TRAMPOLINE = ->(_widget : LibWol::Widget*, data : Void*) do
      Callbacks.invoke(Toggle, data, &.fire)
      nil
    end

    @on_change : (Toggle -> Nil)? = nil

    # A toggle with no change handler.
    def initialize(context : Context, label : ::String = "", on : Bool = false)
      @context = context
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_toggle_create(context, Strings.ptr(label), Strings.size(label), on ? 1 : 0, nil, nil, slot, err)
      end)
    end

    # A toggle that calls the block after the person flipped it (read it with `on?`).
    def initialize(context : Context, label : ::String = "", on : Bool = false, &on_change : Toggle -> Nil)
      @context = context
      @on_change = on_change
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_toggle_create(context, Strings.ptr(label), Strings.size(label), on ? 1 : 0, TRAMPOLINE,
          Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    # True when on.
    def on? : Bool
      LibWol.wol_toggle_get_on(self) == 1
    end

    # Switch it. Redraws the window the toggle is in, if it is shown.
    def on=(on : Bool) : Bool
      Wol.call { |err| LibWol.wol_toggle_set_on(self, on ? 1 : 0, err) }
      on
    end

    protected def callback? : Bool
      !@on_change.nil?
    end

    protected def fire : Nil
      @on_change.try &.call(self)
    end
  end

  # A value between `min` and `max` the person drags. The block runs after each change.
  class Slider < Widget
    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    TRAMPOLINE = ->(_widget : LibWol::Widget*, data : Void*) do
      Callbacks.invoke(Slider, data, &.fire)
      nil
    end

    @on_change : (Slider -> Nil)? = nil

    # A slider with no change handler. Needs min < max and min <= value <= max, or raises
    # `InvalidArgument`.
    def initialize(context : Context, min : Number, max : Number, value : Number)
      @context = context
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_slider_create(context, min.to_f64, max.to_f64, value.to_f64, nil, nil, slot, err)
      end)
    end

    # A slider that calls the block after the person moved it (read it with `value`).
    def initialize(context : Context, min : Number, max : Number, value : Number, &on_change : Slider -> Nil)
      @context = context
      @on_change = on_change
      adopt(Wol.out(LibWol::Widget) do |slot, err|
        LibWol.wol_slider_create(context, min.to_f64, max.to_f64, value.to_f64, TRAMPOLINE, Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    # The current value.
    def value : Float64
      LibWol.wol_slider_get_value(self)
    end

    # Move it. Raises `InvalidArgument` outside min..max. Redraws the window, if it is shown.
    def value=(value : Number) : Number
      Wol.call { |err| LibWol.wol_slider_set_value(self, value.to_f64, err) }
      value
    end

    protected def callback? : Bool
      !@on_change.nil?
    end

    protected def fire : Nil
      @on_change.try &.call(self)
    end
  end
end

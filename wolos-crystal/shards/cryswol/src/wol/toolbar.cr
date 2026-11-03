# SPDX-License-Identifier: BSD-2-Clause
# Toolbars (wol.h "1.2: menus and toolbars"; ADR 0020). A toolbar is a row of buttons and
# spacers above a window's content; a press comes back as the button's key inside dispatch.
# Keys: 1-32 of [a-z0-9_-], unique in the toolbar. Up to 16 buttons and spacers. The key is
# borrowed from libwol for the callback only, so it is copied before the block sees it.
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"
require "./context"

module Wol
  # A window's toolbar (`Window#toolbar=`).
  class Toolbar < Handle(LibWol::Toolbar)
    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    TRAMPOLINE = ->(_toolbar : LibWol::Toolbar*, key : LibWol::String*, data : Void*) do
      Callbacks.invoke(Toolbar, data) { |toolbar| toolbar.fire(Strings.borrow(key) || "") }
      nil
    end

    @on_choose : (Toolbar, ::String -> Nil)? = nil
    @placed = false

    # An empty toolbar; the block gets the toolbar and the pressed button's key.
    def initialize(context : Context, &on_choose : Toolbar, ::String -> Nil)
      @context = context
      @on_choose = on_choose
      adopt(Wol.out(LibWol::Toolbar) do |slot, err|
        LibWol.wol_toolbar_create(context, TRAMPOLINE, Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    # Add a button. `icon` is an icon name, or nil to show the label; the label is the tooltip
    # either way. Raises `InvalidArgument` for a bad or repeated key, a bad label, or a full bar.
    def add_button(key : ::String, label : ::String, icon : ::String? = nil) : self
      Wol.call do |err|
        LibWol.wol_toolbar_add_button(self, Strings.ptr(key), Strings.size(key), Strings.ptr(label),
          Strings.size(label), Strings.ptr(icon), Strings.size(icon), err)
      end
      self
    end

    # Push what follows to the far side.
    def add_spacer : self
      Wol.call { |err| LibWol.wol_toolbar_add_spacer(self, err) }
      self
    end

    # Grey out (false) or allow (true) the button `key`. A greyed-out button is never delivered.
    # Raises `InvalidArgument` for an unknown key.
    def set_enabled(key : ::String, enabled : Bool) : self
      Wol.call do |err|
        LibWol.wol_toolbar_set_enabled(self, Strings.ptr(key), Strings.size(key), enabled ? 1 : 0, err)
      end
      self
    end

    # :nodoc:
    # Called by `Window#toolbar=` after libwol accepted (or, with false,
    # let go of) the toolbar.
    def placed!(placed : Bool = true) : Nil
      @placed = placed
    end

    # True once the toolbar was set on a window.
    def placed? : Bool
      @placed
    end

    # Release the toolbar. A toolbar on a window stays rooted: the window in libwol still holds
    # it and may still call the trampoline, which must find a live object. The block is dropped.
    def close : Nil
      keep = !closed? && @placed
      super
      root! if keep
    end

    protected def fire(key : ::String) : Nil
      @on_choose.try &.call(self, key)
    end

    # Drop the block, so a block that captured this toolbar does not keep a cycle alive.
    protected def before_release(_ptr : LibWol::Toolbar*) : Nil
      @on_choose = nil
    end

    protected def release(ptr : LibWol::Toolbar*) : Nil
      LibWol.wol_toolbar_release(ptr)
    end
  end
end

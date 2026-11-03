# SPDX-License-Identifier: BSD-2-Clause
# Windows (wol.h "Windows and Kit parts", "1.1" close handler, "1.2" menus and toolbar; ADR 0020).
#
# Concept: a window is a title, one content part, menus and a toolbar, drawn by the Kit host. Why
# a shown window is rooted: libwol's context only holds windows weakly, so in a GC language a
# shown window nobody references would vanish from the screen whenever the collector runs; a
# shown window therefore stays alive until `close` (or its context closes). How: `show` sends the
# tree, any later change redraws it, and the person's close comes back through `on_close`.
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"
require "./context"
require "./widget"
require "./menu"
require "./toolbar"

module Wol
  # A window of the app. Nothing is on screen until `show`.
  class Window < Handle(LibWol::Window)
    # The trampoline libwol calls when the person closes the window; captures nothing.
    TRAMPOLINE = ->(_window : LibWol::Window*, data : Void*) do
      Callbacks.invoke(Window, data, &.fire_close)
      nil
    end

    # The title given at creation (one line, up to 256 bytes).
    getter title : ::String

    # The content part, or nil.
    getter content : Widget? = nil

    @menus = [] of Menu
    @toolbar : Toolbar? = nil
    @on_close : (Window -> Nil)? = nil

    # Raises `InvalidArgument` for a title that is not one line of at most 256 bytes.
    def initialize(context : Context, @title : ::String)
      super(Wol.out(LibWol::Window) do |slot, err|
        LibWol.wol_window_create(context, Strings.ptr(@title), Strings.size(@title), slot, err)
      end, context)
    end

    # Set the content part; nil clears it. Raises `InvalidArgument` when the part is already in a
    # stack or a window. Redraws the window, if it is shown.
    def content=(content : Widget?) : Widget?
      if content
        Wol.call { |err| LibWol.wol_window_set_content(self, content, err) }
        content.placed!
      else
        Wol.call { |err| LibWol.wol_window_set_content(self, nil, err) }
      end
      # libwol let go of the old part: it may go somewhere else now.
      @content.try &.placed!(false) unless @content.same?(content)
      @content = content
    end

    # Show the window, or bring the screen up to date with its parts. Raises `Unavailable` when
    # the Kit host is not there, `InvalidArgument` when the tree breaks a Kit limit.
    def show : self
      Wol.call { |err| LibWol.wol_window_show(self, err) }
      root!
      self
    end

    # Take the window off the screen (wol_window_close); `show` brings it back. The object stays
    # usable; `close` releases it.
    def hide : self
      Wol.call { |err| LibWol.wol_window_close(self, err) }
      self
    end

    # Call the block when the person closes the window. Replaces an earlier handler.
    def on_close(&block : Window -> Nil) : self
      @on_close = block
      Wol.call { |err| LibWol.wol_window_set_close_handler(self, TRAMPOLINE, Callbacks.user_data(self), err) }
      root!
      self
    end

    # Remove the close handler.
    def clear_close_handler : self
      Wol.call { |err| LibWol.wol_window_set_close_handler(self, nil, nil, err) }
      @on_close = nil
      self
    end

    # Add `menu` to the menu bar, after the others. A menu goes on one window; up to 8.
    def add_menu(menu : Menu) : self
      Wol.call { |err| LibWol.wol_window_add_menu(self, menu, err) }
      @menus << menu
      menu.placed!
      self
    end

    # The menus added so far, in order.
    def menus : Array(Menu)
      @menus.dup
    end

    # The toolbar, or nil.
    def toolbar : Toolbar?
      @toolbar
    end

    # Set the toolbar; nil removes it. A window has one, a toolbar goes on one window.
    def toolbar=(toolbar : Toolbar?) : Toolbar?
      if toolbar
        Wol.call { |err| LibWol.wol_window_set_toolbar(self, toolbar, err) }
        toolbar.placed!
      else
        Wol.call { |err| LibWol.wol_window_set_toolbar(self, nil, err) }
      end
      @toolbar.try &.placed!(false) unless @toolbar.same?(toolbar)
      @toolbar = toolbar
    end

    protected def fire_close : Nil
      @on_close.try &.call(self)
    end

    # Drop the block and the parts, so nothing this window held keeps a cycle alive.
    protected def before_release(_ptr : LibWol::Window*) : Nil
      @on_close = nil
      @content = nil
      @menus.clear
      @toolbar = nil
    end

    protected def release(ptr : LibWol::Window*) : Nil
      LibWol.wol_window_release(ptr)
    end
  end
end

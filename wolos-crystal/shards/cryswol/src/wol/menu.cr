# SPDX-License-Identifier: BSD-2-Clause
# Menus (wol.h "1.2: menus and toolbars"; ADR 0020). A menu is a title and a list of items and
# separators; what the person chooses comes back as the item's key inside dispatch. Keys: 1-32 of
# [a-z0-9_-], unique in their menu. Labels: one line, up to 64 bytes. Up to 48 entries. A change
# redraws the window, if it is shown. The key is borrowed from libwol for the callback only, so
# it is copied into a Crystal String before the block sees it (plan-cr1 §3 rule 6).
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"
require "./context"

module Wol
  # A menu for a window's menu bar (`Window#add_menu`).
  class Menu < Handle(LibWol::Menu)
    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    TRAMPOLINE = ->(_menu : LibWol::Menu*, key : LibWol::String*, data : Void*) do
      Callbacks.invoke(Menu, data) { |menu| menu.fire(Strings.borrow(key) || "") }
      nil
    end

    # The title shown in the menu bar.
    getter title : ::String

    @on_choose : (Menu, ::String -> Nil)? = nil
    @placed = false

    # A menu titled `title`; the block gets the menu and the chosen item's key.
    def initialize(context : Context, @title : ::String, &on_choose : Menu, ::String -> Nil)
      @context = context
      @on_choose = on_choose
      adopt(Wol.out(LibWol::Menu) do |slot, err|
        LibWol.wol_menu_create(context, Strings.ptr(@title), Strings.size(@title), TRAMPOLINE,
          Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    # Add an item. `shortcut` is like "cmd+s", "cmd+shift+n" or "alt+f4"; nil for none.
    # Raises `InvalidArgument` for a bad or repeated key, a bad label or shortcut, or a full menu.
    def add_item(key : ::String, label : ::String, shortcut : ::String? = nil) : self
      Wol.call do |err|
        LibWol.wol_menu_add_item(self, Strings.ptr(key), Strings.size(key), Strings.ptr(label), Strings.size(label),
          Strings.ptr(shortcut), Strings.size(shortcut), err)
      end
      self
    end

    # Add a separator line.
    def add_separator : self
      Wol.call { |err| LibWol.wol_menu_add_separator(self, err) }
      self
    end

    # Grey out (false) or allow (true) the item `key`. A greyed-out item is never delivered.
    # Raises `InvalidArgument` for an unknown key.
    def set_enabled(key : ::String, enabled : Bool) : self
      Wol.call { |err| LibWol.wol_menu_set_enabled(self, Strings.ptr(key), Strings.size(key), enabled ? 1 : 0, err) }
      self
    end

    # Show (true) or hide (false) a check mark on the item `key`. Raises `InvalidArgument` for an
    # unknown key.
    def set_checked(key : ::String, checked : Bool) : self
      Wol.call { |err| LibWol.wol_menu_set_checked(self, Strings.ptr(key), Strings.size(key), checked ? 1 : 0, err) }
      self
    end

    # :nodoc:
    # Called by `Window#add_menu` after libwol accepted (or, with false,
    # let go of) the menu.
    def placed!(placed : Bool = true) : Nil
      @placed = placed
    end

    # True once the menu was added to a window.
    def placed? : Bool
      @placed
    end

    # Release the menu. A menu on a window stays rooted: the window in libwol still holds it and
    # may still call the trampoline, which must find a live object. The block is dropped.
    def close : Nil
      keep = !closed? && @placed
      super
      root! if keep
    end

    protected def fire(key : ::String) : Nil
      @on_choose.try &.call(self, key)
    end

    # Drop the block, so a block that captured this menu does not keep a cycle alive.
    protected def before_release(_ptr : LibWol::Menu*) : Nil
      @on_choose = nil
    end

    protected def release(ptr : LibWol::Menu*) : Nil
      LibWol.wol_menu_release(ptr)
    end
  end
end

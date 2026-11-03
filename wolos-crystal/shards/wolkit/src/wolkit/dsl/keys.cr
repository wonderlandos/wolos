# SPDX-License-Identifier: BSD-2-Clause
# Keys: the `on_key:` argument every element takes, and the `on_key { }` and `shortcut "cmd+s" { }`
# statements that bind the part the view is building into (the window's root at the top of a view).
#
# Concept: the engine sends every key to the app first, as KEY, to the nearest part with a key
# handler above the focused one (or the window's root when nothing has focus), then does what the key
# means (edit a field, move focus, press a button). One part gets the event; Wolkit then carries it
# up through the parts above, so a list's arrow keys and the window's shortcuts both work while a
# field inside them has focus. A shortcut is a key with Control (where Command arrives), Alt or
# Super, or F1 to F12 alone, named the way the engine names them (`cmd+shift+s`); the innermost part
# with that shortcut runs it. Why on KEY and not on the engine's SHORTCUT event: KEY reaches the app
# on every backend, the headless one included, so shortcuts are tested the way they run.
require "../event"
require "../mount"

module Wolkit
  # One key press or release: the KEY event's payload (`u32 keysym`, `u32 pressed`, `u32 mods`,
  # `u32 repeat`).
  struct KeyPress
    SHIFT = 1_u32
    CTRL  = 2_u32
    ALT   = 4_u32
    SUPER = 8_u32

    # Names of the keys that are not a character (the XKB keysyms the engine uses).
    NAMES = {
      0xff08_u32 => "backspace", 0xff09_u32 => "tab", 0xfe20_u32 => "tab", 0xff0d_u32 => "enter",
      0xff8d_u32 => "enter", 0xff1b_u32 => "escape", 0xff50_u32 => "home", 0xff51_u32 => "left",
      0xff52_u32 => "up", 0xff53_u32 => "right", 0xff54_u32 => "down", 0xff55_u32 => "page_up",
      0xff56_u32 => "page_down", 0xff57_u32 => "end", 0xff67_u32 => "menu", 0xffff_u32 => "delete",
      0x20_u32 => "space",
    }

    getter keysym : UInt32
    getter mods : UInt32
    getter? pressed : Bool
    getter? repeat : Bool

    def initialize(@keysym : UInt32, @pressed : Bool = true, @mods : UInt32 = 0_u32, @repeat : Bool = false)
    end

    def self.from(event : Event) : KeyPress
      new(event.u32(0), event.u32(4) != 0, event.u32(8), event.u32(12) != 0)
    end

    def shift? : Bool
      @mods & SHIFT != 0
    end

    # Control, which is also where Command arrives.
    def ctrl? : Bool
      @mods & CTRL != 0
    end

    def alt? : Bool
      @mods & ALT != 0
    end

    def super? : Bool
      @mods & SUPER != 0
    end

    # The key's name: "escape", "enter", "left", "f5", "a" (letters lower case), "7"; "" for a key
    # with no name here (a modifier alone, a dead key).
    def name : String
      if named = NAMES[@keysym]?
        return named
      end
      return "f#{@keysym - 0xffbe_u32 + 1}" if @keysym >= 0xffbe_u32 && @keysym <= 0xffc9_u32
      return (@keysym + 0x20).chr.to_s if @keysym >= 0x41 && @keysym <= 0x5a
      return @keysym.chr.to_s if @keysym > 0x20 && @keysym < 0x7f
      ""
    end

    # True for a press (or repeat) of the key called `name`, with exactly the modifiers given.
    def is?(name : String, shift : Bool = false, ctrl : Bool = false, alt : Bool = false) : Bool
      pressed? && self.name == name && shift? == shift && ctrl? == ctrl && alt? == alt && !super?
    end
  end

  struct Event
    # The key of a KEY event.
    def key : KeyPress
      KeyPress.from(self)
    end

    # The name of a SHORTCUT event (`cmd+s`): payload `u32 keysym`, `u32 mods`, then the name.
    def shortcut : String
      str(8)
    end
  end

  # The key listeners and shortcut tables of parts, per mount, and the walk that delivers a key.
  module Keys
    alias Slot = {UInt64, UInt32}

    @@listeners = {} of Slot => Array(Proc(KeyPress, Nil))
    @@shortcuts = {} of Slot => Hash(String, Proc(Nil))
    @@bound = Set(Slot).new

    # Normalise a name the way the engine writes it: `cmd`, `shift`, `alt` in that order, then a
    # letter, a digit or f1 to f12; `ctrl`, `control`, `command` and `super` all mean `cmd`.
    def self.normalise(name : String) : String
      parts = name.downcase.split('+').map(&.strip)
      raise UsageError.new("A shortcut is keys joined by +, like \"cmd+s\"; this is #{name.inspect}.") if parts.any?(&.empty?)
      key = parts.pop
      mods = parts.map { |m| %w(ctrl control command super).includes?(m) ? "cmd" : m }
      mods.each do |m|
        raise UsageError.new("A shortcut's modifiers are cmd, shift and alt; this has #{m.inspect}.") unless %w(cmd shift alt).includes?(m)
      end
      unless key =~ /\A([a-z0-9]|f([1-9]|1[0-2]))\z/
        raise UsageError.new("A shortcut ends with a letter, a digit or f1 to f12; this is #{name.inspect}.")
      end
      if !mods.includes?("cmd") && !mods.includes?("alt") && key.size == 1
        raise UsageError.new("A shortcut needs cmd or alt (or is f1 to f12 alone); this is #{name.inspect}.")
      end
      (%w(cmd shift alt).select { |m| mods.includes?(m) } << key).join('+')
    end

    # The shortcut a key press is, in the engine's form, or nil (the rule of the engine's keys.rs).
    def self.shortcut_name(key : KeyPress) : String?
      sym = key.keysym
      name = if sym >= 0xffbe_u32 && sym <= 0xffc9_u32
               "f#{sym - 0xffbe_u32 + 1}"
             elsif (sym >= 0x30 && sym <= 0x39) || (sym >= 0x61 && sym <= 0x7a)
               sym.chr.to_s
             elsif sym >= 0x41 && sym <= 0x5a
               (sym + 0x20).chr.to_s
             end
      return unless name
      cmd = key.mods & (KeyPress::CTRL | KeyPress::SUPER) != 0
      return unless cmd || key.alt? || name.size > 1
      String.build do |io|
        io << "cmd+" if cmd
        io << "shift+" if key.shift?
        io << "alt+" if key.alt?
        io << name
      end
    end

    # Call `handler` with every key that reaches `part` or a part inside it.
    def self.listen(mount : Mount, part : UInt32, handler : Proc(KeyPress, Nil)) : Nil
      slot = {mount.object_id, part}
      (@@listeners[slot] ||= [] of Proc(KeyPress, Nil)) << handler
      bind(mount, part, slot)
      Wolkit.on_cleanup do
        @@listeners[slot]?.try &.delete(handler)
        forget(slot)
        nil
      end
    end

    # Run `action` when the shortcut `name` is pressed inside `part`.
    def self.shortcut(mount : Mount, part : UInt32, name : String, action : Proc(Nil)) : Nil
      slot = {mount.object_id, part}
      key = normalise(name)
      (@@shortcuts[slot] ||= {} of String => Proc(Nil))[key] = action
      bind(mount, part, slot)
      Wolkit.on_cleanup do
        @@shortcuts[slot]?.try &.delete(key)
        forget(slot)
        nil
      end
    end

    # Parts with key listeners or shortcuts (for specs).
    def self.size : Int32
      @@bound.size
    end

    private def self.bind(mount : Mount, part : UInt32, slot : Slot) : Nil
      return if @@bound.includes?(slot)
      @@bound << slot
      mount.on(part, Wire::Event::KEY) do |event|
        deliver(mount, event)
        nil
      end
    end

    private def self.forget(slot : Slot) : Nil
      listeners = @@listeners[slot]?
      table = @@shortcuts[slot]?
      @@listeners.delete(slot) if listeners && listeners.empty?
      @@shortcuts.delete(slot) if table && table.empty?
      @@bound.delete(slot) unless @@listeners.has_key?(slot) || @@shortcuts.has_key?(slot)
    end

    # From the part the engine chose up to the root: every listener hears the key, the innermost
    # shortcut with its name runs.
    def self.deliver(mount : Mount, event : Event) : Nil
      key = event.key
      name = key.pressed? ? shortcut_name(key) : nil
      done = false
      node = event.node
      id = mount.object_id
      while node != Mount::NONE
        slot = {id, node}
        @@listeners[slot]?.try &.dup.each(&.call(key))
        if name && !done && (action = @@shortcuts[slot]?.try(&.[name]?))
          action.call
          done = true
        end
        break if node == Mount::ROOT
        node = mount.parent(node)
      end
    end
  end

  module DSL
    # Call the block with every key that reaches the part being built (the window's root at the
    # top of a view): `on_key { |key| close if key.is?("escape") }`.
    def on_key(&block : KeyPress -> _) : Nil
      mount = Mount.current
      Keys.listen(mount, mount.current_parent, ->(key : KeyPress) { block.call(key); nil })
    end

    # Run the block when the shortcut is pressed inside the part being built: `shortcut "cmd+s" { save }`.
    def shortcut(name : String, &block : -> _) : Nil
      mount = Mount.current
      Keys.shortcut(mount, mount.current_parent, name, -> { block.call; nil })
    end
  end
end

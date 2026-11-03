# SPDX-License-Identifier: BSD-2-Clause
# Mount: everything one window's UI needs on the Crystal side. It allocates node ids (a 24-bit slot
# and an 8-bit generation, chosen here so creating a part needs no round trip), writes patch ops into
# a PatchWriter, keeps a mirror of who is whose child (so a region can insert before its marker), the
# handler table indexed by node slot, and the insertion point the element macros use.
#
# Concept: compile-time mounting (plan-wolkit section 3.2). The element macros expand to calls on the
# current Mount: `create`, `set_*`, `insert`, `within`; there is no runtime tree of view objects. Why a
# mirror: INSERT takes an index, and a region's rows go before its marker, whose index moves. How:
# one `Array(UInt32)` of children per container, searched from the end.
require "./patch/writer"
require "./event"
require "./reactive/scope"

module Wolkit
  # What a mount calls when it has ops to send: the engine window, or a recorder in specs.
  alias PatchSink = Proc(Bytes, Nil)

  # An event handler: it gets the event and returns nothing.
  alias Handler = Proc(Event, Nil)

  class Mount
    # The slot of the window's root node.
    ROOT = 0_u32
    # No node (the insertion point has no "before").
    NONE = 0xffff_ffff_u32

    @@current : Mount? = nil

    # The mount the element macros write to.
    def self.current : Mount
      @@current || raise UsageError.new(
        "An element was created outside a window. Create elements inside `view`, inside a `window` block, or in a block given to `each`, `show` or `sheet`.")
    end

    def self.current? : Mount?
      @@current
    end

    # Run the block with `mount` as the current mount.
    def self.with(mount : Mount, & : -> T) : T forall T
      previous = @@current
      @@current = mount
      begin
        yield
      ensure
        @@current = previous
      end
    end

    private struct Point
      getter parent : UInt32
      getter before : UInt32

      def initialize(@parent : UInt32, @before : UInt32)
      end
    end

    # The handlers of one node: a few (kind, proc) pairs.
    private class NodeHandlers
      getter kinds = [] of UInt16
      getter procs = [] of Handler

      def set(kind : UInt16, handler : Handler) : Nil
        if index = @kinds.index(kind)
          @procs[index] = handler
        else
          @kinds << kind
          @procs << handler
        end
      end

      def find(kind : UInt16) : Handler?
        index = @kinds.index(kind)
        index ? @procs[index] : nil
      end
    end

    getter writer : PatchWriter
    # The sequence number of the last patch sent.
    getter seq : UInt32 = 0_u32
    # Parts alive (for the limit and for specs).
    getter live : Int32 = 0
    # Parts a window may hold (plan-wolkit section 2.8).
    MAX_NODES = 65_536

    @gens = Array(UInt8).new(64, 0_u8)
    @free = [] of UInt32
    @pending_free = [] of UInt32
    @next_slot = 1_u32
    @kids = Array(Array(UInt32)?).new(64, nil)
    @parent_of = Array(UInt32).new(64, NONE)
    @handlers = Array(NodeHandlers?).new(64, nil)
    @masks = Array(UInt32).new(64, 0_u32)
    @text_seq = Array(UInt32).new(64, 0_u32)
    @points = [Point.new(ROOT, NONE)]
    @collect : Array(UInt32)? = nil
    @collect_depth = 0
    @sink : PatchSink
    @dirty = false

    def initialize(@sink : PatchSink, @writer : PatchWriter = PatchWriter.new)
      @kids[0] = [] of UInt32
    end

    # --- ids

    def self.slot(id : UInt32) : Int32
      (id & 0x00ff_ffff_u32).to_i
    end

    # Make a node of `kind`: allocate its id, write CREATE, and remember it in the current scope so
    # its id comes back when the scope is disposed.
    def create(kind : UInt32) : UInt32
      if @live + 1 >= MAX_NODES
        raise LimitError.new("This window has #{MAX_NODES + 1} parts; the limit is #{MAX_NODES}. Use `list` for long lists.")
      end
      slot = take_slot
      id = slot | (@gens[slot.to_i].to_u32 << 24)
      @live += 1
      @writer.create(id, kind)
      Scope.current.nodes << id
      touch
      id
    end

    private def take_slot : UInt32
      if slot = @free.pop?
        return slot
      end
      slot = @next_slot
      @next_slot += 1
      grow(slot.to_i)
      slot
    end

    private def grow(slot : Int) : Nil
      while @gens.size <= slot
        @gens << 0_u8
        @kids << nil
        @parent_of << NONE
        @handlers << nil
        @masks << 0_u32
        @text_seq << 0_u32
      end
    end

    # --- structure

    # Put `child` at the insertion point: appended to the current parent, or before the marker a
    # region is inserting in front of.
    def insert(child : UInt32) : Nil
      point = @points.last
      @collect.try { |roots| roots << child if @points.size == @collect_depth }
      parent_slot = Mount.slot(point.parent)
      kids = (@kids[parent_slot] ||= [] of UInt32)
      if point.before == NONE
        @writer.insert(point.parent, child)
        kids << child
      else
        index = kids.rindex(point.before) || raise UsageError.new("A region lost its marker; this is a bug in Wolkit.")
        @writer.insert(point.parent, child, index.to_u32)
        kids.insert(index, child)
      end
      @parent_of[Mount.slot(child)] = point.parent
      touch
    end

    # New children go inside `parent` until the block ends.
    def within(parent : UInt32, & : ->) : Nil
      @points << Point.new(parent, NONE)
      begin
        yield
      ensure
        @points.pop
      end
    end

    # New children go inside `parent`, before `before` (a region's marker), until the block ends.
    def at(parent : UInt32, before : UInt32, & : ->) : Nil
      @points << Point.new(parent, before)
      begin
        yield
      ensure
        @points.pop
      end
    end

    # Run the block and return the parts it inserted at its own level (the roots of a row).
    def collect(& : ->) : Array(UInt32)
      roots = [] of UInt32
      previous = @collect
      previous_depth = @collect_depth
      @collect = roots
      @collect_depth = @points.size
      begin
        yield
      ensure
        @collect = previous
        @collect_depth = previous_depth
      end
      roots
    end

    # An invisible part that marks where a region's rows go. It takes no space (display: none).
    def marker : UInt32
      id = create(Wire::Kind::SPACER)
      @writer.set_u32(id, Wire::Prop::DISPLAY, 1_u32)
      insert(id)
      id
    end

    # Move a root to the position of `before` (or the end) inside its parent.
    def move(node : UInt32, before : UInt32) : Nil
      parent = @parent_of[Mount.slot(node)]
      kids = @kids[Mount.slot(parent)] || return
      kids.delete(node)
      if before == NONE
        @writer.move(node, parent)
        kids << node
      else
        index = kids.rindex(before) || raise UsageError.new("A region lost its marker; this is a bug in Wolkit.")
        @writer.move(node, parent, index.to_u32)
        kids.insert(index, node)
      end
      touch
    end

    # Take `roots` out of the window: FREE each root (the engine frees its subtree) when `send` is
    # true, and give every id of `nodes` back. `send` is false when a parent is being freed too.
    def release(roots : Array(UInt32), nodes : Array(UInt32), send : Bool) : Nil
      if send
        roots.each do |root|
          parent = @parent_of[Mount.slot(root)]
          @kids[Mount.slot(parent)].try &.delete(root)
          @writer.free(root)
        end
        touch
      end
      nodes.each { |id| forget(id) }
    end

    private def forget(id : UInt32) : Nil
      slot = Mount.slot(id)
      @kids[slot] = nil
      @handlers[slot] = nil
      @masks[slot] = 0_u32
      @text_seq[slot] = 0_u32
      @parent_of[slot] = NONE
      @gens[slot] = @gens[slot] &+ 1
      @pending_free << slot.to_u32
      @live -= 1
    end

    # --- properties (the element macros call these)

    def set_f32(id : UInt32, prop : UInt16, value : Float32) : Nil
      @writer.set_f32(id, prop, value)
      touch
    end

    def set_i32(id : UInt32, prop : UInt16, value : Int32) : Nil
      @writer.set_i32(id, prop, value)
      touch
    end

    def set_u32(id : UInt32, prop : UInt16, value : UInt32) : Nil
      @writer.set_u32(id, prop, value)
      touch
    end

    def set_bool(id : UInt32, prop : UInt16, value : Bool) : Nil
      @writer.set_bool(id, prop, value)
      touch
    end

    def set_str(id : UInt32, prop : UInt16, value : String) : Nil
      @writer.set_str(id, prop, value)
      touch
    end

    def set_dim(id : UInt32, prop : UInt16, unit : UInt32, value : Float32) : Nil
      @writer.set_dim(id, prop, unit, value)
      touch
    end

    def set_pair(id : UInt32, prop : UInt16, a : Float32, b : Float32) : Nil
      @writer.set_pair(id, prop, a, b)
      touch
    end

    def set_border(id : UInt32, prop : UInt16, width : Float32, token : UInt32) : Nil
      @writer.set_border(id, prop, width, token)
      touch
    end

    def set_shadow(id : UInt32, prop : UInt16, dx : Float32, dy : Float32, blur : Float32, token : UInt32) : Nil
      @writer.set_shadow(id, prop, dx, dy, blur, token)
      touch
    end

    def set_text(id : UInt32, text : String) : Nil
      @writer.set_text(id, text)
      @text_seq[Mount.slot(id)] = @seq + 1
      touch
    end

    def set_style(id : UInt32, classes : Array(UInt32)) : Nil
      @writer.set_style(id, classes)
      touch
    end

    def set_state(id : UInt32, mask : UInt32) : Nil
      @writer.set_state(id, mask)
      touch
    end

    def set_key(id : UInt32, key : UInt64) : Nil
      @writer.set_key(id, key)
      touch
    end

    def set_a11y(id : UInt32, role : UInt32, name : String, description : String = "") : Nil
      @writer.set_a11y(id, role, name, description)
      touch
    end

    def focus(id : UInt32) : Nil
      @writer.focus(id)
      touch
    end

    # Animate a number property on the engine clock.
    def anim(id : UInt32, prop : UInt16, target : Float32, spring : Bool, param : UInt32) : Nil
      @writer.anim(id, prop, target, spring ? 1_u32 : 0_u32, param)
      touch
    end

    def list_range(id : UInt32, count : UInt32, height : Float32, overscan : UInt32) : Nil
      @writer.list_range(id, count, height, overscan)
      touch
    end

    # --- events

    # Call `handler` when an event of `kind` reaches `id`. Handlers go when the node is freed.
    def on(id : UInt32, kind : UInt16, &handler : Event ->) : Nil
      slot = Mount.slot(id)
      (@handlers[slot] ||= NodeHandlers.new).set(kind, handler)
      @masks[slot] |= Event.bit(kind)
      @writer.bind_event(id, @masks[slot])
      touch
    end

    # Run the handler of the event, in a batch. An event for a part this mount no longer has is
    # ignored (it was queued before the part was freed).
    def dispatch(event : Event) : Nil
      slot = Mount.slot(event.node)
      return if slot >= @handlers.size
      handler = @handlers[slot].try &.find(event.kind)
      return unless handler
      Wolkit.batch { Wolkit.guard { Mount.with(self) { handler.call(event) } } }
    end

    # True when `event` was made before the app last wrote the text of its node: the person's edit
    # is stale and must not overwrite the app's value (the controlled-input race).
    def stale_text?(event : Event) : Bool
      slot = Mount.slot(event.node)
      slot < @text_seq.size && event.seq < @text_seq[slot]
    end

    # --- sending

    def pending? : Bool
      !@writer.empty?
    end

    # Send what was written as one patch. Returns true when a patch went out. The ids freed in it
    # become reusable only after the engine has applied it.
    def flush : Bool
      @dirty = false
      return false if @writer.empty?
      @seq += 1
      bytes = @writer.finish(@seq)
      begin
        @sink.call(bytes)
      ensure
        @writer.clear
      end
      @free.concat(@pending_free)
      @pending_free.clear
      true
    end

    # Send early when a patch is getting big (the engine takes 1 MiB per apply).
    def flush_if_full : Nil
      flush if @writer.full?
    end

    private def touch : Nil
      return if @dirty
      @dirty = true
      Frame.mark(self)
    end
  end

  # The mounts with something to send. The loop flushes them once per turn, so a turn with many
  # state changes makes one `wolkit_window_apply` per window (plan-wolkit section 3.2).
  module Frame
    @@dirty = [] of Mount
    @@requested = false

    def self.mark(mount : Mount) : Nil
      @@dirty << mount
      request
    end

    # Send every dirty mount's patch. Returns how many patches went out.
    def self.flush_all : Int32
      @@requested = false
      sent = 0
      until @@dirty.empty?
        mount = @@dirty.shift
        sent += 1 if mount.flush
      end
      sent
    end

    # Ask for a flush at the next scheduler turn, for changes made outside the loop's own turn
    # (a timer, a posted write). The loop clears the request when it flushes first.
    def self.request : Nil
      return if @@requested || !@@auto
      @@requested = true
      spawn(name: "wolkit-frame") { flush_all }
    end

    @@auto = false

    # Specs and the loop decide who flushes; `auto` lets a fiber do it.
    def self.auto=(value : Bool) : Bool
      @@auto = value
    end

    def self.pending : Int32
      @@dirty.size
    end
  end
end

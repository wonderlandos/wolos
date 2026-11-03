# SPDX-License-Identifier: BSD-2-Clause
# Regions: the parts of a view whose content comes and goes. `each` builds one row per list item and
# follows the list's change records (no diff); `show` mounts a subtree while a condition holds;
# `switch` rebuilds its content when a value changes. Each row is an ownership scope: removing it
# disposes its effects and handlers and frees its parts (plan-wolkit section 3.2 "Structure").
#
# Concept: a region owns an invisible marker part (display: none) inside its parent; its rows are
# inserted before the marker, so the region needs no wrapper box and does not disturb the layout of
# its siblings. Why a marker: INSERT takes an index, and the index of "the end of my rows" moves as
# siblings come and go; the mount finds the marker's index when it inserts.
require "./mount"
require "./reactive/list"
require "./reactive/effect"

module Wolkit
  # One built row: its scope and the parts it put directly inside the region's parent.
  class Row
    getter scope : Scope
    getter roots : Array(UInt32)

    def initialize(@scope : Scope, @roots : Array(UInt32))
    end
  end

  # Builds and removes rows for a region.
  abstract class Region
    @mount : Mount
    @parent : UInt32
    @marker : UInt32
    @scope : Scope

    def initialize(@mount : Mount)
      @parent = @mount.current_parent
      @scope = Scope.new
      @marker = @mount.marker
    end

    # Build the content of one row before `before` (a root of the next row, or the marker).
    protected def build_row(before : UInt32, key : UInt64? = nil, & : ->) : Row
      row_scope = Scope.new(@scope)
      roots = @mount.build(row_scope, @parent, before) { yield }
      raise UsageError.new("A row needs at least one element.") if roots.empty?
      key.try { |k| roots.each { |root| @mount.set_key(root, k) } }
      Row.new(row_scope, roots)
    end

    protected def drop_row(row : Row) : Nil
      row.scope.dispose
    end

    protected def before_of(row : Row?) : UInt32
      row ? row.roots.first : @marker
    end

    # Move all roots of `row` to just before `before`.
    protected def move_row(row : Row, before : UInt32) : Nil
      row.roots.each { |root| @mount.move(root, before) }
    end
  end

  # `each` over a List: rows follow the change records.
  class ListRegion(T) < Region
    @rows = [] of Row
    @list : List(T)
    @build : T -> Nil
    @key : Proc(T, UInt64)?
    @cursor : ListCursor(T)

    def initialize(mount : Mount, @list : List(T), @build : T -> Nil, @key : Proc(T, UInt64)? = nil)
      super(mount)
      @cursor = @list.open_cursor
      @scope.on_cleanup do
        @list.close_cursor(@cursor)
        @rows.each { |row| drop_row(row) }
        @rows.clear
        nil
      end
      @scope.run do
        @list.peek.each { |item| @rows << make_row(item, @marker) }
        @cursor.clear
        Wolkit.effect do
          @list.touch
          apply
        end
      end
    end

    private def make_row(item : T, before : UInt32) : Row
      key = @key
      build = @build
      build_row(before, key ? key.call(item) : nil) { build.call(item) }
    end

    private def apply : Nil
      changes = @cursor.changes
      return if changes.empty?
      changes.each do |change|
        case change.kind
        in .insert?
          before = before_of(@rows[change.index]?)
          @rows.insert(change.index, make_row(change.item.as(T), before))
        in .remove?
          drop_row(@rows.delete_at(change.index))
        in .move?
          row = @rows.delete_at(change.index)
          @rows.insert(change.to, row)
          move_row(row, before_of(@rows[change.to + 1]?))
        in .reset?
          @rows.each { |row| drop_row(row) }
          @rows.clear
          (change.items || [] of T).each { |item| @rows << make_row(item, @marker) }
        end
      end
      @cursor.clear
    end
  end

  # `each` over a value that is replaced as a whole (a signal holding an array): rows are matched
  # by key, so an unchanged key keeps its row.
  class KeyedRegion(T) < Region
    @rows = [] of Row
    @keys = [] of UInt64
    @source : Readable(Array(T))
    @build : T -> Nil
    @key : Proc(T, UInt64)

    def initialize(mount : Mount, @source : Readable(Array(T)), @build : T -> Nil, @key : Proc(T, UInt64))
      super(mount)
      @scope.on_cleanup do
        @rows.each { |row| drop_row(row) }
        @rows.clear
        nil
      end
      @scope.run do
        Wolkit.effect { apply(@source.value) }
      end
    end

    private def apply(items : Array(T)) : Nil
      keys = items.map { |item| @key.call(item) }
      # Rows whose key is gone.
      index = @rows.size - 1
      while index >= 0
        unless keys.includes?(@keys[index])
          drop_row(@rows.delete_at(index))
          @keys.delete_at(index)
        end
        index -= 1
      end
      items.each_with_index do |item, position|
        key = keys[position]
        at = @keys.index(key)
        if at.nil?
          build = @build
          before = before_of(@rows[position]?)
          @rows.insert(position, build_row(before, key) { build.call(item) })
          @keys.insert(position, key)
        elsif at != position
          row = @rows.delete_at(at)
          @keys.delete_at(at)
          @rows.insert(position, row)
          @keys.insert(position, key)
          move_row(row, before_of(@rows[position + 1]?))
        end
      end
    end
  end

  # `switch`: the content is rebuilt each time the value changes; `show` is a switch on a Bool.
  class SwitchRegion(T) < Region
    @row : Row? = nil
    @source : Readable(T)
    @build : T -> Nil

    def initialize(mount : Mount, @source : Readable(T), @build : T -> Nil)
      super(mount)
      @scope.on_cleanup do
        @row.try { |row| drop_row(row) }
        @row = nil
        nil
      end
      shown = nil
      @scope.run do
        Wolkit.effect do
          value = @source.value
          if shown.nil? || shown != value
            shown = value
            @row.try { |row| drop_row(row) }
            @row = nil
            build = @build
            @row = build_row(@marker) { build.call(value) }
          end
        end
      end
    end
  end

  # `show`: content while the condition is true.
  class ShowRegion < Region
    @row : Row? = nil

    def initialize(mount : Mount, @condition : Readable(Bool), @build : ->)
      super(mount)
      @scope.on_cleanup do
        @row.try { |row| drop_row(row) }
        @row = nil
        nil
      end
      @scope.run do
        Wolkit.effect do
          want = @condition.value
          if want && @row.nil?
            build = @build
            @row = build_row(@marker) { build.call }
          elsif !want && (row = @row)
            drop_row(row)
            @row = nil
          end
        end
      end
    end
  end
end

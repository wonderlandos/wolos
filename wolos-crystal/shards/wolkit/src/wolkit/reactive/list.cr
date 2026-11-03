# SPDX-License-Identifier: BSD-2-Clause
# List(T): a reactive array that records what changed. `each` regions read the change records
# (Insert, Remove, Move, Reset) instead of diffing, so adding a todo creates one row and removing
# one frees one row (plan-wolkit section 3.2 "Structure"). Reads (`size`, `[]`, `each`) are tracked.
module Wolkit
  # What happened to a list. `index` is where, `to` is the destination of a move.
  struct ListChange
    enum Kind
      Insert
      Remove
      Move
      Reset
    end

    getter kind : Kind
    getter index : Int32
    getter to : Int32

    def initialize(@kind : Kind, @index : Int32 = 0, @to : Int32 = 0)
    end
  end

  # One reader of the change records (an `each` region): it gets its own queue, so regions on the
  # same list do not share a cursor.
  class ListCursor
    getter changes = [] of ListChange

    def push(change : ListChange) : Nil
      # A reset makes every earlier record moot.
      @changes.clear if change.kind.reset?
      @changes << change
    end

    def clear : Nil
      @changes.clear
    end
  end

  class List(T) < Source
    include Indexable(T)

    @items = [] of T
    @cursors = [] of ListCursor

    def initialize(items : Enumerable(T)? = nil)
      items.try { |source| source.each { |item| @items << item } }
    end

    # The number of items; a tracked read.
    def size : Int32
      track
      @items.size
    end

    def unsafe_fetch(index : Int) : T
      track
      @items.unsafe_fetch(index)
    end

    # The items without recording a dependency (a snapshot copy).
    def peek : Array(T)
      @items.dup
    end

    # Open a reader of the change records. The region disposes it with its scope.
    def open_cursor : ListCursor
      cursor = ListCursor.new
      @cursors << cursor
      cursor
    end

    def close_cursor(cursor : ListCursor) : Nil
      @cursors.delete(cursor)
    end

    # Make the running effect depend on this list (the region does this each time it runs).
    def touch : Nil
      track
    end

    def <<(item : T) : self
      push(item)
    end

    def push(item : T) : self
      UI.check_context("`List#push`")
      @items << item
      changed(ListChange.new(:insert, @items.size - 1))
      self
    end

    def insert(index : Int, item : T) : self
      UI.check_context("`List#insert`")
      index += @items.size + 1 if index < 0
      raise IndexError.new("List#insert: index #{index} is outside 0..#{@items.size}") unless 0 <= index <= @items.size
      @items.insert(index, item)
      changed(ListChange.new(:insert, index))
      self
    end

    def delete_at(index : Int) : T
      UI.check_context("`List#delete_at`")
      index += @items.size if index < 0
      item = @items.delete_at(index)
      changed(ListChange.new(:remove, index))
      item
    end

    # Delete the first item equal to `item`; true when there was one.
    def delete(item : T) : Bool
      index = @items.index(item)
      return false unless index
      delete_at(index)
      true
    end

    def []=(index : Int, item : T) : T
      UI.check_context("`List#[]=`")
      index += @items.size if index < 0
      raise IndexError.new("List#[]=: index #{index} is outside 0...#{@items.size}") unless 0 <= index < @items.size
      return item if @items[index] == item
      @items[index] = item
      # A row shows its item: replace it (remove then insert), the region keeps the order.
      @cursors.each do |cursor|
        cursor.push(ListChange.new(:remove, index))
        cursor.push(ListChange.new(:insert, index))
      end
      notify
      item
    end

    # Move the item at `from` so that it ends up at `to`.
    def move(from : Int, to : Int) : self
      UI.check_context("`List#move`")
      return self if from == to
      @items.insert(to, @items.delete_at(from))
      changed(ListChange.new(:move, from, to))
      self
    end

    def clear : self
      UI.check_context("`List#clear`")
      return self if @items.empty?
      @items.clear
      changed(ListChange.new(:reset))
      self
    end

    # Replace everything (the regions rebuild their rows).
    def replace(items : Enumerable(T)) : self
      UI.check_context("`List#replace`")
      @items.clear
      items.each { |item| @items << item }
      changed(ListChange.new(:reset))
      self
    end

    # Sort in place (the regions rebuild their rows).
    def sort_by!(&block : T -> U) : self forall U
      UI.check_context("`List#sort_by!`")
      @items.sort_by!(&block)
      changed(ListChange.new(:reset))
      self
    end

    private def changed(change : ListChange) : Nil
      @cursors.each &.push(change)
      notify
    end
  end

  # A list: `@items = list(String)`.
  macro list(type)
    ::Wolkit::List({{ type }}).new
  end
end

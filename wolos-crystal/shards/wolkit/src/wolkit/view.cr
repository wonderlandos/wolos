# SPDX-License-Identifier: BSD-2-Clause
# View: the base of App and Component. It includes the DSL (the element macros, `each`, `show`,
# `signal`, `list`) so a `view` method reads as a declaration. A view method runs once, at mount;
# state lives in signals held as instance variables, and only the effects that read a changed signal
# run again (plan-wolkit section 3.4).
require "./dsl/elements"
require "./region"
require "./window"
require "./reactive/signal"
require "./reactive/list"
require "./reactive/computed"
require "./reactive/effect"

module Wolkit
  module DSL
    # A top-level window; the block is its content. `width:` and `height:` are in logical pixels.
    macro window(title, **props, &block)
      ::Wolkit::Window.open({{ title }}, {{ props.double_splat }}) do
        {{ block.body }}
      end
    end

    # One row per item of a list; the block builds a row (one or more elements). Adding an item
    # builds one row, removing one frees one row. `key:` gives each row a stable key.
    def each(list : List(T), key : Proc(T, U), &build : T -> _) : Nil forall T, U
      ListRegion(T).new(Mount.current, list, ->(item : T) { build.call(item); nil }, ->(item : T) { Key.of(key.call(item)) })
    end

    def each(list : List(T), &build : T -> _) : Nil forall T
      ListRegion(T).new(Mount.current, list, ->(item : T) { build.call(item); nil })
    end

    # Rows for an array held by a readable (a signal, a computed): rows are matched by key.
    def each(source : Readable(Array(T)), key : Proc(T, U), &build : T -> _) : Nil forall T, U
      KeyedRegion(T).new(Mount.current, source, ->(item : T) { build.call(item); nil }, ->(item : T) { Key.of(key.call(item)) })
    end

    # Rows for a plain array: built once.
    def each(items : Array(T), &build : T -> _) : Nil forall T
      items.each { |item| build.call(item) }
    end

    # The block's content is in the window while `condition` is true.
    def show(condition : Readable(Bool), &build : ->) : Nil
      ShowRegion.new(Mount.current, condition, -> { build.call; nil })
    end

    # The block's content is rebuilt each time `source` changes.
    def switch(source : Readable(T), &build : T -> _) : Nil forall T
      SwitchRegion(T).new(Mount.current, source, ->(value : T) { build.call(value); nil })
    end

    # Run the block now and again when a signal it read changes.
    def effect(&block : ->) : Effect
      Wolkit.effect(&block)
    end

    # A value derived from signals.
    def computed(&block : -> T) : Computed(T) forall T
      Wolkit.computed(&block)
    end

    # Group writes: the effects that depend on them run once, when the block ends.
    def batch(&block : ->) : Nil
      Wolkit.batch(&block)
    end

    # Read without depending.
    def untracked(&block : -> T) : T forall T
      Wolkit.untracked(&block)
    end

    # Run `block` when the enclosing view is torn down.
    def on_cleanup(&block : -> Nil) : Nil
      Wolkit.on_cleanup(&block)
    end
  end

  # The base of everything that has a `view`.
  abstract class View
    include DSL

    # Build the content. Runs once, at mount.
    abstract def view
  end
end

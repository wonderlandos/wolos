# SPDX-License-Identifier: BSD-2-Clause
# Scope: the owner of everything a mounted piece of UI creates (effects, handlers, timers, child
# scopes, cleanups, node ids). Disposing a scope frees all of it, so nothing leaks when a list row or
# a `show` region goes away (plan-wolkit section 3.2 "Structure").
#
# Concept: reactive code is a tree of scopes. Why: a row built by `each` must give back its effects
# and handlers when its item is removed. How: every constructor that makes something disposable
# registers it with `Scope.current`; `Scope#dispose` runs the cleanups in reverse order of creation.
module Wolkit
  # Anything a scope can dispose.
  module Disposable
    # Release it. Safe to call twice.
    abstract def dispose : Nil
  end

  # A cleanup block, so it sits in the scope's list in creation order with the effects.
  class CleanupItem
    include Disposable

    def initialize(@block : Proc(Nil))
    end

    def dispose : Nil
      block = @block
      Wolkit.guard { block.call }
    end
  end

  class Scope
    include Disposable

    @@current : Scope? = nil
    @@root = Scope.new(nil)

    # The scope that is collecting disposables right now (the root scope outside a mount).
    def self.current : Scope
      @@current || @@root
    end

    # The scope of the whole app.
    def self.root : Scope
      @@root
    end

    # Dispose everything in the root scope and start a new one (the end of a run, specs).
    def self.reset_root : Nil
      @@root.dispose
      @@root = Scope.new(nil)
    end

    getter parent : Scope?
    getter? disposed : Bool = false
    # Node ids created while this scope was current (for the mount to free).
    getter nodes = [] of UInt32
    # True once a mount has registered the cleanup that gives `nodes` back.
    property? tracks_nodes : Bool = false
    @children = [] of Scope
    @disposables = [] of Disposable

    def initialize(@parent : Scope? = Scope.current)
      @parent.try &.add_child(self)
    end

    # Run the block with this scope as the current one.
    def run(& : -> T) : T forall T
      raise Wolkit::UsageError.new("This scope was disposed; nothing can be created in it.") if @disposed
      previous = @@current
      @@current = self
      begin
        yield
      ensure
        @@current = previous
      end
    end

    # Register something to dispose with the scope.
    def add(item : Disposable) : Nil
      if @disposed
        item.dispose
      else
        @disposables << item
      end
    end

    # Run `block` when the scope is disposed (after the later registered ones).
    def on_cleanup(&block : -> Nil) : Nil
      add(CleanupItem.new(block))
    end

    protected def add_child(child : Scope) : Nil
      @children << child
    end

    protected def remove_child(child : Scope) : Nil
      @children.delete(child)
    end

    # Dispose children, then disposables and cleanups, newest first. Errors in one cleanup go to
    # `Wolkit.on_error` and do not stop the others.
    def dispose : Nil
      return if @disposed
      @disposed = true
      @parent.try &.remove_child(self)
      until @children.empty?
        @children.pop.dispose
      end
      until @disposables.empty?
        @disposables.pop.dispose
      end
    end
  end
end

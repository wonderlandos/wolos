# SPDX-License-Identifier: BSD-2-Clause
# Navigator(Route): a typed stack of screens (plan-wolkit section 3.4). `push` shows a new screen, `back`
# returns, `replace` swaps the top one. The routes are values of your own type (a record, an enum, a
# union of records), so a screen's arguments are checked by the compiler. The view shows the top
# route with `navigator`, which is a `switch` on the stack's top.
#
# ```
# record Detail, id : Int32
# alias Route = Symbol | Detail
# @nav = Wolkit::Navigator(Route).new(:list)
#
# navigator @nav do |route|
#   case route
#   in Symbol then list_screen
#   in Detail then detail_screen(route.id)
#   end
# end
# ```
#
# Back is Esc, `cmd+[` and the pointer back button (the window routes those keys to `back` through
# `Navigator#bind`). The screen change is an engine transition and snaps under reduced motion.
require "./reactive/signal"
require "./reactive/computed"
require "./view"

module Wolkit
  class Navigator(R)
    @stack : Signal(Array(R))
    @top : Computed(R)

    def initialize(root : R)
      @stack = Signal(Array(R)).new([root] of R)
      stack = @stack
      @top = Computed(R).new { stack.value.last }
    end

    # The route on top, as a readable the view depends on.
    def current : Readable(R)
      @top
    end

    # How deep the stack is (1 at the root).
    def depth : Int32
      @stack.value.size
    end

    # True when `back` would do something.
    def can_go_back? : Bool
      depth > 1
    end

    # Show `route` on top of the current screen.
    def push(route : R) : Nil
      @stack.set!(@stack.peek + [route] of R)
    end

    # Return to the previous screen. Does nothing at the root.
    def back : Nil
      return unless @stack.peek.size > 1
      @stack.set!(@stack.peek[0...-1])
    end

    # Swap the top screen for `route`.
    def replace(route : R) : Nil
      stack = @stack.peek.dup
      stack[-1] = route
      @stack.set!(stack)
    end

    # Return to the root screen.
    def reset : Nil
      @stack.set!([@stack.peek.first] of R)
    end
  end

  module DSL
    # Show the screen for the top route of `nav`; the block builds one screen. Moving to another
    # route frees the old screen and builds the new one.
    def navigator(nav : Navigator(R), &build : R -> _) : Nil forall R
      switch(nav.current) { |route| build.call(route) }
    end
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# Context values: `provide` puts a value where the views below it can `inject` it, by type, without
# passing it through every prop. The lookup walks the scope chain from the current scope up, so a
# screen can replace what its parent provided (plan-wolkit section 3.4).
require "./reactive/scope"

module Wolkit
  # Holds one provided value (the base lets scopes keep boxes of different types in one table).
  abstract class ContextBox
  end

  class ContextBoxOf(T) < ContextBox
    getter value : T

    def initialize(@value : T)
    end
  end

  class Scope
    @provided : Hash(String, ContextBox)? = nil

    # Provide `value` to this scope and the scopes below it.
    def provide(key : String, box : ContextBox) : Nil
      (@provided ||= {} of String => ContextBox)[key] = box
    end

    # The nearest provided box for `key`, here or in a parent scope.
    def find_provided(key : String) : ContextBox?
      @provided.try { |table| table[key]?.try { |box| return box } }
      @parent.try &.find_provided(key)
    end
  end

  # An injected value that nobody provided.
  class MissingContext < Exception
  end

  # Make `value` available to everything mounted below the current scope.
  def self.provide(value : T) : T forall T
    Scope.current.provide(T.name, ContextBoxOf(T).new(value))
    value
  end

  # The value of type `T` that an enclosing view provided.
  def self.inject(type : T.class) : T forall T
    box = Scope.current.find_provided(T.name)
    raise MissingContext.new("Nothing provides a #{T.name}. Call `provide(#{T.name}.new(...))` in a view above this one.") unless box
    box.as(ContextBoxOf(T)).value
  end

  module DSL
    # Make `value` available to the views mounted below this one.
    def provide(value : T) : T forall T
      Wolkit.provide(value)
    end

    # The provided value of type `T`.
    def inject(type : T.class) : T forall T
      Wolkit.inject(type)
    end
  end
end

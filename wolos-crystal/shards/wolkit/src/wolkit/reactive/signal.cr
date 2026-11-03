# SPDX-License-Identifier: BSD-2-Clause
# Signal(T): a reactive value. Reading `value` inside an effect records a dependency; writing it
# queues the dependent effects (plan-wolkit section 3.2). Unknown methods forward to the value
# through a tracked read, so `@draft.blank?` and `"#{@items.size}"` need no ceremony.
#
# Concurrency (ADR 0036): signals are read and written on the UI context only. A write from another
# thread raises `UsageError` in dev builds; use `set_later`, which posts the write to the UI context.
module Wolkit
  class Signal(T) < Source
    include Readable(T)
    include Writable(T)

    @value : T

    def initialize(@value : T)
    end

    # The value; inside an effect this records a dependency.
    def value : T
      track
      @value
    end

    # The value without recording a dependency.
    def peek : T
      @value
    end

    # Write the value. Dependents are queued only when it changed (`==`).
    def value=(value : T) : T
      UI.check_context("`#{self.class}#value=`")
      return value if @value == value
      @value = value
      notify
      value
    end

    # Write and notify even when equal (for a mutable value changed in place).
    def set!(value : T) : T
      UI.check_context("`#{self.class}#set!`")
      @value = value
      notify
      value
    end

    # Change the value with the block (`s.update { |n| n + 1 }`).
    def update(& : T -> T) : T
      self.value = yield @value
    end

    # Notify the dependents after the value was changed in place (`s.mutate(&.<<("x"))`).
    def mutate(& : T -> _) : T
      UI.check_context("`#{self.class}#mutate`")
      yield @value
      notify
      @value
    end

    # Write from any thread: the write runs on the UI context, in a batch, at the next loop turn.
    # ameba:disable Naming/AccessorMethodName
    def set_later(value : T) : Nil
      UI.post { self.value = value }
    end

    # The dependents read through the value: `@name.upcase`, `@items.empty?`.
    forward_missing_to value

    def ==(other : Signal) : Bool
      same?(other)
    end
  end

  # Make a signal. Written at class level (`@draft = signal("")`) the type argument is worked out
  # from the literal; for anything else give it: `signal(nil, of: String?)`.
  macro signal(value, of type = nil)
    {% if type %}
      ::Wolkit::Signal({{ type }}).new({{ value }})
    {% elsif value.is_a?(StringLiteral) %}
      ::Wolkit::Signal(::String).new({{ value }})
    {% elsif value.is_a?(BoolLiteral) %}
      ::Wolkit::Signal(::Bool).new({{ value }})
    {% elsif value.is_a?(NumberLiteral) %}
      ::Wolkit::Signal({{ {i8: "Int8", i16: "Int16", i32: "Int32", i64: "Int64", u8: "UInt8", u16: "UInt16", u32: "UInt32", u64: "UInt64", f32: "Float32", f64: "Float64"}[value.kind].id }}).new({{ value }})
    {% elsif value.is_a?(SymbolLiteral) %}
      ::Wolkit::Signal(::Symbol).new({{ value }})
    {% else %}
      ::Wolkit::Signal(typeof({{ value }})).new({{ value }})
    {% end %}
  end
end

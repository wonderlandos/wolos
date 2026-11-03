# SPDX-License-Identifier: BSD-2-Clause
# Component: a reusable piece of view with typed props (plan-wolkit section 3.4).
#
# ```
# class Counter < Wolkit::Component
#   prop start : Int32 = 0
#   @n = signal(0)
#
#   def view
#     @n.value = @start
#     row gap: :sm do
#       button "-", label: "Decrease", on_click: -> { @n.value -= 1 }
#       text "#{@n}"
#       button "+", label: "Increase", on_click: -> { @n.value += 1 }
#     end
#   end
# end
# # used as:  counter start: 5
# ```
#
# `view` runs once, at mount. `prop` gives a typed, documented argument; a prop typed
# `Readable(T)` also accepts a plain `T` and stays reactive. `slot` renders the block the caller gave
# (`section "Title" do ... end`). Naming the class `Counter` defines the element `counter`.
require "./view"
require "./context"

module Wolkit
  # A constant readable: a plain value in a place that takes a Readable.
  class Const(T)
    include Readable(T)

    def initialize(@value : T)
    end

    def value : T
      @value
    end

    def peek : T
      @value
    end
  end

  # A plain value or a readable, as a readable.
  def self.readable(value : Readable(T)) : Readable(T) forall T
    value
  end

  def self.readable(value : T) : Readable(T) forall T
    Const(T).new(value)
  end

  # The elements defined by component classes live here; every view includes it.
  module Components
  end

  abstract class View
    include Components
  end

  abstract class Component < View
    @content : Proc(Nil)? = nil
    @mounted = [] of Proc(Nil)

    # Mount a component where the caller is: its view runs once inside a new scope, with `content`
    # as the block for `slot`. Returns the id of the first part it made (0 for a component that opens windows).
    def self.mount(component : Component, content : Proc(Nil)? = nil) : UInt32
      component.content = content
      scope = Scope.new
      mount = Mount.current?
      unless mount
        # No window yet: this component opens its own (AppWindow).
        scope.run { component.build }
        component.finish_mount
        return 0_u32
      end
      roots = scope.run { mount.collect { component.build } }
      raise UsageError.new("#{component.class} made no element; a component's view must create at least one.") if roots.empty?
      component.finish_mount
      roots.first
    end

    protected def content=(content : Proc(Nil)?) : Nil
      @content = content
    end

    # Run the view with the lifecycle bookkeeping.
    def build : Nil
      view
    end

    def finish_mount : Nil
      @mounted.each { |hook| Wolkit.guard { hook.call } }
      @mounted.clear
    end

    # Render the block the caller passed to this component (nothing if none).
    def slot : Nil
      @content.try &.call
    end

    # Run `block` after the view is mounted, with the parts in place.
    def on_mount(&block : ->) : Nil
      @mounted << block
    end

    # Run `block` when the component goes away.
    def on_unmount(&block : -> Nil) : Nil
      Wolkit.on_cleanup(&block)
    end

    # A typed argument of the component. `prop name : Type` is required; `prop name : Type = default`
    # is optional. The initializer is written by the `finished` hook below.
    macro prop(decl)
      getter {{ decl.var }} : {{ decl.type }}
      PROP_{{ decl.var.id.upcase }} = { {{ decl.var.id.stringify }}, {{ decl.type.stringify }}, {{ decl.value.is_a?(Nop) ? nil : decl.value.stringify }} }
    end

    # Defines the element for the component class: `class Counter` becomes `counter start: 5`.
    macro inherited
      module ::Wolkit::Components
        macro {{ @type.name.split("::").last.underscore.id }}(*args, **props, &block)
          ::Wolkit::Component.mount(::{{ @type.name.id }}.new(\{{ args.splat }}\{% if args.size > 0 && props.size > 0 %}, \{% end %}\{{ props.double_splat }}){% verbatim do %}{% if block %}, -> { {{ block.body }} }{% end %}{% end %})
        end
      end
    end

    macro finished
      {% for sub in Wolkit::Component.all_subclasses %}
        {% props = sub.constants.select(&.stringify.starts_with?("PROP_")) %}
        {% unless props.empty? %}
          class ::{{ sub.name.id }}
            def initialize(
              {% for c in props %}{% spec = sub.constant(c) %}{% unless spec[2] %}{% if spec[1].starts_with?("Readable(") %}{{ spec[0].id }} : {{ spec[1][9..-2].id }} | {{ spec[1].id }}, {% else %}@{{ spec[0].id }} : {{ spec[1].id }}, {% end %}{% end %}{% end %}
              {% for c in props %}{% spec = sub.constant(c) %}{% if spec[2] %}{% if spec[1].starts_with?("Readable(") %}{{ spec[0].id }} : {{ spec[1][9..-2].id }} | {{ spec[1].id }} = {{ spec[2].id }}, {% else %}@{{ spec[0].id }} : {{ spec[1].id }} = {{ spec[2].id }}, {% end %}{% end %}{% end %}
            )
              {% for c in props %}{% spec = sub.constant(c) %}{% if spec[1].starts_with?("Readable(") %}
                @{{ spec[0].id }} = ::Wolkit.readable({{ spec[0].id }})
              {% end %}{% end %}
            end
          end
        {% end %}
      {% end %}
    end
  end
end

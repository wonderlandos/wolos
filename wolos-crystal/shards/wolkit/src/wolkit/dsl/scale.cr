# SPDX-License-Identifier: BSD-2-Clause
# The design scale as run-time conversions: what a DSL value (a symbol or a number) becomes on the
# wire. Literals are converted by the element macros at compile time with the same tables
# (dsl/tables.cr); these methods serve values that are only known at run time (`gap: @gap`). A value
# off the scale raises `UsageError` with the sentence the compile-time check uses.
require "../patch/wire"
require "../guard"

module Wolkit
  module Scale
    # Spacing in px (HIG section 2, the scale of the style language): 0, 4, 8, 12, 16, 24.
    SPACING = {"none" => 0, "xs" => 4, "sm" => 8, "md" => 12, "lg" => 16, "xl" => 24}
    # Corner radius in px (shared::theme::SCALE).
    RADIUS = {"none" => 0, "s" => 6, "m" => 10, "l" => 14, "xl" => 20}
    # The colour tokens, in the order of shared::theme::TOKENS (the wire id is the index).
    TOKENS = {"surface", "sidebar", "card", "line", "text", "dim", "accent", "accentText", "glass", "control", "success", "warning", "danger"}

    def self.space(name : Symbol) : Float32
      SPACING[name.to_s]?.try(&.to_f32) || raise UsageError.new("Spacing is one of #{SPACING.keys.map { |k| ":#{k}" }.join(", ")}; this is :#{name}.")
    end

    def self.space(px : Int | Float) : Float32
      return px.to_f32 if SPACING.values.includes?(px)
      raise UsageError.new("Spacing is one of 0, 4, 8, 12, 16 or 24; this is #{px}.")
    end

    def self.radius(name : Symbol) : Float32
      RADIUS[name.to_s]?.try(&.to_f32) || raise UsageError.new("A radius is one of #{RADIUS.keys.map { |k| ":#{k}" }.join(", ")}; this is :#{name}.")
    end

    def self.radius(px : Int | Float) : Float32
      return px.to_f32 if RADIUS.values.includes?(px)
      raise UsageError.new("A radius is one of 0, 6, 10, 14 or 20; this is #{px}.")
    end

    def self.token(name : Symbol) : UInt32
      index = TOKENS.index(name.to_s)
      raise UsageError.new("Colours are theme tokens: #{TOKENS.join(", ")}; this is :#{name}.") unless index
      index.to_u32
    end

    # The index of `name` in the engine's value list for an enum property (`:center` in `align`).
    def self.enum_index(table : String, name : Symbol | String, prop : String) : UInt32
      values = Wire::ENUMS[table]
      wanted = name.to_s.tr("_", "-")
      index = values.index(wanted)
      raise UsageError.new("`#{prop}:` takes #{values.map { |v| ":#{v.tr("-", "_")}" }.join(", ")}; this is :#{name}.") unless index
      index.to_u32
    end

    # A size: a number is pixels, `:auto` is auto, "50%" is a percentage.
    def self.dim(value : Int | Float) : {UInt32, Float32}
      {1_u32, value.to_f32}
    end

    def self.dim(value : Symbol) : {UInt32, Float32}
      raise UsageError.new("A size is a number of pixels, :auto or a percentage like \"50%\"; this is :#{value}.") unless value == :auto
      {0_u32, 0.0_f32}
    end

    def self.dim(value : String) : {UInt32, Float32}
      percent = value.chomp('%').to_f32?
      raise UsageError.new("A size is a number of pixels, :auto or a percentage like \"50%\"; this is #{value.inspect}.") unless value.ends_with?('%') && percent
      {2_u32, percent}
    end

    # A pair of numbers (`translate:`, `scale:`): one number is both, `{x, y}` is each.
    def self.pair(value : Int | Float) : {Float32, Float32}
      {value.to_f32, value.to_f32}
    end

    def self.pair(value : Tuple(A, B)) : {Float32, Float32} forall A, B
      x, y = value
      raise UsageError.new("A pair is one number or `{x, y}`; this is #{value.inspect}.") unless x.is_a?(Number) && y.is_a?(Number)
      {x.to_f32, y.to_f32}
    end

    def self.pair(value) : {Float32, Float32}
      raise UsageError.new("A pair is one number or `{x, y}`; this is #{value.inspect}.")
    end

    # A border or outline: `{width, :token}`, a width alone (in the `line` colour), or `:none`.
    def self.border(value : Int | Float) : {Float32, UInt32}
      border_width(value, value)
      {value.to_f32, token(:line)}
    end

    def self.border(value : Symbol) : {Float32, UInt32}
      raise UsageError.new("A border is a width, `{width, :token}` or :none; this is :#{value}.") unless value == :none
      {0.0_f32, token(:line)}
    end

    def self.border(value : Tuple(A, B)) : {Float32, UInt32} forall A, B
      width, colour = value
      unless width.is_a?(Number) && colour.is_a?(Symbol)
        raise UsageError.new("A border is `{width, :token}`, for example `{1, :line}`; this is #{value.inspect}.")
      end
      border_width(width, value)
      {width.to_f32, token(colour)}
    end

    def self.border(value) : {Float32, UInt32}
      raise UsageError.new("A border is a width, `{width, :token}` or :none; this is #{value.inspect}.")
    end

    private def self.border_width(width : Number, whole) : Nil
      raise UsageError.new("A border width is 0 or more; this is #{whole.inspect}.") if width < 0
    end

    # font-weight: 400, 500, 600 or 700.
    def self.weight(value : Int) : UInt32
      case value
      when 400 then 0_u32
      when 500 then 1_u32
      when 600 then 2_u32
      when 700 then 3_u32
      else          raise UsageError.new("A weight is 400, 500, 600 or 700; this is #{value}.")
      end
    end
  end
end

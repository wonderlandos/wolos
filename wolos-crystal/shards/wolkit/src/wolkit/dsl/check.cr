# SPDX-License-Identifier: BSD-2-Clause
# Typed gates for DSL values. The element macros pass every property value through one of these, so a
# wrong type is an ordinary overload error that points at the argument in the user's file
# ("expected argument #1 to 'Wolkit::Check.bool' to be Bool, not Wolkit::Signal(String)"). Only the
# plain types are accepted: a Readable is unwrapped by `Wolkit.read` before the gate.
module Wolkit
  module Check
    def self.bool(value : Bool) : Bool
      value
    end

    def self.str(value : String) : String
      value
    end

    def self.str(value : Nil) : String
      ""
    end

    # A value shown as text: a string, a symbol or a number.
    def self.text(value : String | Symbol | Number?) : String
      value.to_s
    end
  end
end

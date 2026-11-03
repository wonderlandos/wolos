# SPDX-License-Identifier: GPL-2.0-or-later
# A desktop that only remembers what it was asked, for the pure specs and the engine specs.
require "../../src/looks/core"

# What a desktop that only remembers answers.
class FakeApplier < Looks::Applier
  getter applied = [] of {String, String}
  getter switched = [] of String
  getter? timer_on : Bool = false
  property refuse : String? = nil
  property theme : String? = "wol-dark"

  def apply(look : Hue::Look, id : String) : String?
    return @refuse if @refuse
    @applied << {look.name, id}
    @theme = id
    nil
  end

  def current : String?
    @theme
  end

  def timer(on : Bool) : Nil
    @timer_on = on
  end

  def switch(id : String) : String?
    @switched << id
    @theme = id
    nil
  end
end

# The value of something that must be there.
def must(value : T?) : T forall T
  value || raise "expected a value, got nil"
end

# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-005
# Scanner fixture: formatted code that ameba refuses (not_nil! crashes on nil, the value is unused).
module BadAmeba
  def self.home : String
    unused = 1
    ENV["HOME"]?.not_nil!
  end
end

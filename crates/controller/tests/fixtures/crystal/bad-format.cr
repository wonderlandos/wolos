# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-004
# Scanner fixture: valid Crystal that `crystal tool format` would change.
module BadFormat
  def self.add(a : Int32,b : Int32) : Int32
      a+b
  end
end

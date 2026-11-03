# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-033
# Scanner fixture: a token from the predictable generator.
module BadRandom
  def self.token : String
    Random.new.hex(16)
  end
end

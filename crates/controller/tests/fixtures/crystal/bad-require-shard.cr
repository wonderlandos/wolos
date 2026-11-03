# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-043
# Scanner fixture: a require that is not the standard library, so a shard nobody counted.
require "kemal"

module BadRequire
  def self.start
    Kemal.run
  end
end

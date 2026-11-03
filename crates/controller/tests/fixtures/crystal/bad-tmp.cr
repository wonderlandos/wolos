# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-030
# Scanner fixture: a fixed, predictable file name in /tmp.
module BadTmp
  def self.save(text : String)
    File.write("/tmp/fixture-state", text)
  end
end

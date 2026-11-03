# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-026
# Scanner fixture: a bare rescue hides every failure behind one answer.
module BadCatchAll
  def self.read(path : String) : String?
    File.read(path)
  rescue
    nil
  end
end

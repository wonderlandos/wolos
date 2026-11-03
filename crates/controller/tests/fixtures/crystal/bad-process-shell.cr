# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-020
# Scanner fixture: a shell command string instead of a program and an argument array.
module BadProcess
  def self.list(dir : String) : Bool
    Process.run("sh", ["-c", "ls #{dir} | wc -l"]).success?
  end
end

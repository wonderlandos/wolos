# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-025
# Scanner fixture: rescuing Exception also catches signals-as-exceptions and out-of-memory.
module BadRescueException
  def self.read(path : String) : String
    File.read(path)
  rescue ex : Exception
    STDERR.puts ex.message
    ""
  end
end

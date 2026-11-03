# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-031
# Scanner fixture: a file anyone can write.
module BadWorldWritable
  def self.open_up(path : String)
    File.chmod(path, 0o777)
  end
end

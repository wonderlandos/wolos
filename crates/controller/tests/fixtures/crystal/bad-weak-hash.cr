# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-032
# Scanner fixture: MD5 as an integrity check.
require "digest/md5"

module BadWeakHash
  def self.sum(data : String) : String
    Digest::MD5.hexdigest(data)
  end
end

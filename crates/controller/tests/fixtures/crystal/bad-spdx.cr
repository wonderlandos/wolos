# trips: RCR-001
# SPDX-License-Identifier: GPL-2.0-or-later
# Scanner fixture: the licence line is on line 2, so line 1 is not the SPDX identifier (the licence
# check of tools/license still finds it within the head of the file).
module BadSpdx
  def self.hello : String
    "hello"
  end
end

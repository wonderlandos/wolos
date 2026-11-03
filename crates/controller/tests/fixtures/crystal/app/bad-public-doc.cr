# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-052
# A public method with no doc comment above it: the reader of the API docs gets nothing.
module Bad
  def self.undocumented : Int32
    1
  end
end

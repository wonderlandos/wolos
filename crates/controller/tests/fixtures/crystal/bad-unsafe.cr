# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-010
# Scanner fixture: raw memory outside an allowlisted FFI module.
module BadUnsafe
  def self.buffer : UInt8
    ptr = Pointer(UInt8).malloc(8)
    ptr[9] = 1_u8
    ptr[9]
  end
end

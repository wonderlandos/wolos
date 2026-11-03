# SPDX-License-Identifier: GPL-2.0-or-later
# trips: RCR-011
# Scanner fixture: a C binding declared outside an allowlisted FFI module.
@[Link("m")]
lib LibMath
  fun cos(x : Float64) : Float64
end

# SPDX-License-Identifier: BSD-2-Clause
# Versions: the shard's own version, the libwol ABI it was generated from, and the libwol that is
# actually loaded (wol.h "Version"). A newer call is used only when `Wol.version` says the loaded
# libwol has it, because the ABI only ever grows (ADR 0020).
require "./lib_wol"

module Wol
  # The Cryswol shard version (shard.yml).
  VERSION = "0.1.0"

  # The libwol ABI the bindings were generated from, as `{major, minor, patch}`.
  ABI_VERSION = {LibWol::WOL_VERSION_MAJOR, LibWol::WOL_VERSION_MINOR, LibWol::WOL_VERSION_PATCH}

  # The loaded libwol as `major << 16 | minor << 8 | patch`.
  def self.version : UInt32
    LibWol.wol_version
  end

  # The loaded libwol as text, for example "1.14.0".
  def self.version_string : ::String
    ::String.new(LibWol.wol_version_string)
  end

  # True when the loaded libwol is at least `major.minor` (check before calling a newer verb).
  def self.at_least?(major : Int, minor : Int) : Bool
    version >= ((major.to_u32 << 16) | (minor.to_u32 << 8))
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# System information (wol.h "System info", WOL_1.0): the OS name and version, the kernel, and the
# person's appearance and accent. `Wol::System.info` copies every field out at once and releases
# the libwol object, so the answer is a plain value that never holds a handle.
require "./lib_wol"
require "./error"
require "./string"

module Wol
  # Light or dark, as the person chose in System Settings.
  enum Appearance
    Light = 0
    Dark  = 1
  end

  # A snapshot of the system, read once by `System.info`.
  struct SystemInfo
    # "WolOS".
    getter os_name : ::String
    # The release, for example "0.12.0".
    getter os_version : ::String
    # The kernel release.
    getter kernel : ::String
    # The person's appearance. A value a newer libwol adds reads as `Light`.
    getter appearance : Appearance
    # The accent's name, for example "blue".
    getter accent_name : ::String
    # The accent as 0xRRGGBB.
    getter accent_rgb : UInt32

    def initialize(@os_name, @os_version, @kernel, @appearance, @accent_name, @accent_rgb)
    end

    # The accent as "#rrggbb".
    def accent_hex : ::String
      "#%06x" % accent_rgb
    end

    # True when the person chose dark.
    def dark? : Bool
      appearance.dark?
    end
  end

  module System
    # Read the system information. Needs no permission and no system service: libwol reads the
    # OS release files and the person's own appearance setting.
    def self.info(context : Context) : SystemInfo
      ptr = Wol.out(LibWol::SystemInfo) { |slot, err| LibWol.wol_system_info_copy(context, slot, err) }
      begin
        raw = LibWol.wol_system_info_get_appearance(ptr)
        SystemInfo.new(
          Strings.borrow(LibWol.wol_system_info_get_os_name(ptr)) || "",
          Strings.borrow(LibWol.wol_system_info_get_os_version(ptr)) || "",
          Strings.borrow(LibWol.wol_system_info_get_kernel(ptr)) || "",
          Appearance.from_value?(raw) || Appearance::Light,
          Strings.borrow(LibWol.wol_system_info_get_accent_name(ptr)) || "",
          LibWol.wol_system_info_get_accent_rgb(ptr),
        )
      ensure
        LibWol.wol_system_info_release(ptr)
      end
    end
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# Connected devices (wol.h "1.12: devices", WOL_1.12). The list never carries a serial number or
# an address. Opening hands this app one USB device as usbfs, for the grant device:<vendor>:<product>
# only; keyboards and mice, disks and hubs are never handed out.
require "./lib_wol"
require "./error"
require "./info"
require "./file"

module Wol
  module Device
    # Key: devices, a list of { kind ("usb", "iphone" or "bluetooth"), name, can_open; USB also
    # vendor, product; Bluetooth also connected }. Permission: devices.
    def self.list(context : Context) : Info
      Info.copy { |slot, err| LibWol.wol_device_copy_list(context, slot, err) }
    end

    # Open the USB device `vendor`:`product` (the numbers of the list) for reading and writing as
    # usbfs. The file's name is the device's. Needs the grant device:<vendor>:<product>.
    def self.open(context : Context, vendor : Int, product : Int) : OpenFile
      vend = vendor.in?(0..0xffff) ? vendor.to_u32 : UInt32::MAX # libwol refuses out of range
      prod = product.in?(0..0xffff) ? product.to_u32 : UInt32::MAX
      ptr = Wol.out(LibWol::File) { |slot, err| LibWol.wol_device_open(context, vend, prod, slot, err) }
      OpenFile.take(ptr) || raise Internal.new(LibWol::WOL_ERR_INTERNAL, FALLBACK_MESSAGE)
    end
  end
end

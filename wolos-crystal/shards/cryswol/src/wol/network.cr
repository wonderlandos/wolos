# SPDX-License-Identifier: BSD-2-Clause
# Network state (wol.h "1.7: network", WOL_1.7). On purpose there is no address, hardware address
# or network name in the answer: an app does not learn where the person is.
require "./lib_wol"
require "./error"
require "./info"

module Wol
  module Network
    # Keys: online (bool), interfaces (list of { name, kind: "ethernet", "wifi" or "other",
    # up (bool) }). Permission: network-info.
    def self.info(context : Context) : Info
      Info.copy { |slot, err| LibWol.wol_network_copy_info(context, slot, err) }
    end

    # True when the computer is online.
    def self.online?(context : Context) : Bool
      info = info(context)
      begin
        info["online"]?.try(&.as_bool) || false
      ensure
        info.close
      end
    end
  end
end

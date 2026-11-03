# SPDX-License-Identifier: BSD-2-Clause
# Power (wol.h "1.6: power", WOL_1.6): what the computer runs on, and keeping it awake while the
# app does something the person waits for. An assertion lasts until it is closed or the process
# ends; an account holds at most 16.
require "./lib_wol"
require "./error"
require "./handle"
require "./string"
require "./info"

module Wol
  # Keeps the computer from sleeping until `close` (or the GC, or the end of the process).
  class Assertion < Handle(LibWol::Assertion)
    protected def release(ptr : LibWol::Assertion*) : Nil
      LibWol.wol_assertion_release(ptr)
    end
  end

  module Power
    # The power state. Keys: on_battery (bool), low_power (bool), and with a battery: battery
    # { percent, charging, full, minutes_left (when known) }. Permission: power-info.
    def self.info(context : Context) : Info
      Info.copy { |slot, err| LibWol.wol_power_copy_info(context, slot, err) }
    end

    # Keep the computer awake; `reason` is one line the person can see. Close the assertion when
    # done. Permission: keep-awake.
    def self.keep_awake(context : Context, reason : ::String) : Assertion
      ptr = Wol.out(LibWol::Assertion) do |slot, err|
        LibWol.wol_power_keep_awake(context, Strings.ptr(reason), Strings.size(reason), slot, err)
      end
      Assertion.new(ptr)
    end

    # Keep the computer awake while the block runs, then let it sleep again.
    def self.keep_awake(context : Context, reason : ::String, & : -> T) : T forall T
      assertion = keep_awake(context, reason)
      begin
        yield
      ensure
        assertion.close
      end
    end
  end
end

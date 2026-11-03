# SPDX-License-Identifier: BSD-2-Clause
# Background services (wol.h "1.14: services", WOL_1.14). The system runs this app's own
# executable with the single argument --wol-service=<name>, a clean environment and background
# priority, and restarts it within 10 seconds when it exits, until it is stopped or the computer
# restarts. An account runs at most 8; only the app that started a service sees or stops it.
require "./lib_wol"
require "./error"
require "./string"
require "./info"

module Wol
  module Service
    # The argument a service process is started with (`--wol-service=<name>`).
    ARG_PREFIX = "--wol-service="

    # Start service `name` (letters, digits, '.', '_', '-'); starting one that runs does nothing.
    # Permission: background.
    def self.start(context : Context, name : ::String) : Nil
      Wol.call { |err| LibWol.wol_service_start(context, Strings.ptr(name), Strings.size(name), err) }
    end

    # Stop service `name`. Permission: background.
    def self.stop(context : Context, name : ::String) : Nil
      Wol.call { |err| LibWol.wol_service_stop(context, Strings.ptr(name), Strings.size(name), err) }
    end

    # Keys: state ("running", "waiting" or "stopped"), starts, last_exit. Permission: background.
    def self.status(context : Context, name : ::String) : Info
      Info.copy do |slot, err|
        LibWol.wol_service_copy_status(context, Strings.ptr(name), Strings.size(name), slot, err)
      end
    end

    # The service name this process was started as, from `args` (ARGV by default); nil when it
    # runs as the app.
    def self.current(args : Array(::String) = ARGV) : ::String?
      args.size == 1 && args[0].starts_with?(ARG_PREFIX) ? args[0][ARG_PREFIX.size..] : nil
    end
  end
end

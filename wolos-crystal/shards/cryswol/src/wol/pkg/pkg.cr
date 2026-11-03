# SPDX-License-Identifier: GPL-2.0-or-later
# Wol::Pkg: libwopkg, the package core behind `wopkg` (crates/wopkg/include/wopkg.h, ADR 0021,
# docs/modules/wopkg.md), for Crystal apps. Opt-in and GPL: `require "cryswol/pkg"` links
# libwopkg.so.1; plain `require "cryswol"` never does, so apps of any license can use Cryswol.
#
# Requests and results are JSON documents (their keys are listed in docs/modules/wopkg.md). They
# stay text here: decode a result into your own `JSON::Serializable` type (RCR-024), and ignore
# keys you do not know, because libwopkg only ever adds them. Failures raise the `Wol::Error`
# classes, plus `Pkg::Untrusted` and `Pkg::NotFound` for the two statuses only libwopkg has. The
# pointers stay in ffi.cr.
require "./lib_wopkg"
require "./ffi"
require "../error"
require "../string"

module Wol
  module Pkg
    # The library's version text, for example "1.0.0".
    def self.version : ::String
      ::String.new(LibWopkg.wopkg_version_string)
    end

    # The library's version as (major << 16) | (minor << 8) | patch, to compare with the header
    # this was built against (`LibWopkg::WOPKG_VERSION_*`).
    def self.abi_version : UInt32
      LibWopkg.wopkg_version
    end

    # One libwopkg context: the person's package state (or the dirs `config` names). Every
    # method returns the result document as JSON text (`complete` and `history_record`: plain
    # text) or raises.
    class Context
      @ptr : LibWopkg::Context*
      # True until the context exists, so a failed `initialize` leaves nothing for `finalize`.
      @closed = true

      # Open a context. `config` is the JSON of docs/modules/wopkg.md "JSON documents" (`{}`: the
      # person's defaults). Unknown keys are refused with `InvalidArgument`.
      def initialize(config : ::String = "{}")
        @ptr = FFI.context(config)
        @closed = false
      end

      # Release the context now. Later calls raise `UsageError`. Safe to call twice.
      def close : Nil
        return if @closed
        @closed = true
        LibWopkg.wopkg_context_release(@ptr)
      end

      def finalize
        LibWopkg.wopkg_context_release(@ptr) unless @closed
      end

      # True after `close`.
      def closed? : Bool
        @closed
      end

      # Search every source; one result per app (`limit` at most).
      def search(query : ::String, limit : Int = 20) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_search(live, Strings.ptr(query), Strings.size(query), limit.to_u32, slot, err) }
      end

      # Everything known about one package or app. Raises `Pkg::NotFound` for an unknown name.
      def info(name : ::String) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_info(live, Strings.ptr(name), Strings.size(name), slot, err) }
      end

      # Names starting with `prefix`, for shell completion: plain text, one name per line.
      def complete(prefix : ::String, limit : Int = 200) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_complete(live, Strings.ptr(prefix), Strings.size(prefix), limit.to_u32, slot, err) }
      end

      # What is installed, from every source.
      def installed : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_installed(live, slot, err) }
      end

      # Rebuild the index from `request`; the block gets each progress event (JSON text).
      def index_update(request : ::String, &progress : ::String ->) : ::String
        FFI.with_progress(progress) do |cb, data|
          FFI.result { |slot, err| LibWopkg.wopkg_index_update(live, Strings.ptr(request), Strings.size(request), cb, data, slot, err) }
        end
      end

      # Download what `request` names, checked against its hash; the block gets progress events.
      def fetch(request : ::String, &progress : ::String ->) : ::String
        FFI.with_progress(progress) do |cb, data|
          FFI.result { |slot, err| LibWopkg.wopkg_fetch(live, Strings.ptr(request), Strings.size(request), cb, data, slot, err) }
        end
      end

      # Check a signature or checksum. Raises `Pkg::Untrusted` when it does not hold.
      def verify(request : ::String) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_verify(live, Strings.ptr(request), Strings.size(request), slot, err) }
      end

      # Read an AppImage's name, version and update information without running it.
      def appimage_inspect(path : ::String) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_appimage_inspect(live, Strings.ptr(path), Strings.size(path), slot, err) }
      end

      # Install an AppImage; the block gets progress events.
      def appimage_install(request : ::String, &progress : ::String ->) : ::String
        FFI.with_progress(progress) do |cb, data|
          FFI.result { |slot, err| LibWopkg.wopkg_appimage_install(live, Strings.ptr(request), Strings.size(request), cb, data, slot, err) }
        end
      end

      # Remove the AppImage `name`.
      def appimage_remove(name : ::String) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_appimage_remove(live, Strings.ptr(name), Strings.size(name), slot, err) }
      end

      # Update AppImages (a delta when it can); the block gets progress events.
      def appimage_update(request : ::String, &progress : ::String ->) : ::String
        FFI.with_progress(progress) do |cb, data|
          FFI.result { |slot, err| LibWopkg.wopkg_appimage_update(live, Strings.ptr(request), Strings.size(request), cb, data, slot, err) }
        end
      end

      # Every recorded change, oldest first.
      def history : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_history(live, slot, err) }
      end

      # Record one change (`entry`: docs/modules/wopkg.md "History entry"); returns its id (plain
      # text, for `rollback`).
      def history_record(entry : ::String) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_history_record(live, Strings.ptr(entry), Strings.size(entry), slot, err) }
      end

      # Undo the recorded change `id`.
      def rollback(id : ::String) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_rollback(live, Strings.ptr(id), Strings.size(id), slot, err) }
      end

      # Ask the system service for a system change; it checks the person's consent.
      def system_call(request : ::String) : Nil
        FFI.call { |err| LibWopkg.wopkg_system_call(live, Strings.ptr(request), Strings.size(request), err) }
      end

      # What a system update to the release in `request` would change.
      def release_plan(request : ::String) : ::String
        FFI.result { |slot, err| LibWopkg.wopkg_release_plan(live, Strings.ptr(request), Strings.size(request), slot, err) }
      end

      private def live : LibWopkg::Context*
        raise UsageError.new("Wol::Pkg::Context is closed") if @closed
        @ptr
      end
    end
  end
end

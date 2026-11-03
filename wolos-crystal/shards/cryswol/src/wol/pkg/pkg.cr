# SPDX-License-Identifier: GPL-2.0-or-later
# Wol::Pkg: libwopkg, the package core behind `wopkg` (crates/wopkg/include/wopkg.h, ADR 0021,
# docs/modules/wopkg.md), for Crystal apps. Opt-in and GPL: `require "cryswol/pkg"` links
# libwopkg.so.1; plain `require "cryswol"` never does, so apps of any license can use Cryswol.
#
# Requests and results are JSON documents (their keys are listed in docs/modules/wopkg.md). They
# stay text here: decode a result into your own `JSON::Serializable` type (RCR-024), and ignore
# keys you do not know, because libwopkg only ever adds them. Failures raise the `Wol::Error`
# classes, plus `Pkg::Untrusted` and `Pkg::NotFound` for the two statuses only libwopkg has.
require "json"
require "./lib_wopkg"
require "../error"
require "../callbacks"

module Wol
  module Pkg
    # A download or package whose signature or checksum does not hold (`WOPKG_ERR_UNTRUSTED`).
    class Untrusted < Wol::Error
    end

    # No package, record or history entry by that name (`WOPKG_ERR_NOT_FOUND`).
    class NotFound < Wol::Error
    end

    # The error class for a libwopkg `code`: the two of its own, else the `Wol::Error` mapping
    # (the numbers 1 to 4 are the same in wol.h and wopkg.h).
    def self.error(code : Int32, message : ::String) : Wol::Error
      case code
      when LibWopkg::WOPKG_ERR_UNTRUSTED then Untrusted.new(code, message)
      when LibWopkg::WOPKG_ERR_NOT_FOUND then NotFound.new(code, message)
      else                                    Wol::Error.from(code, message)
      end
    end

    # The library's version text, for example "1.0.0".
    def self.version : ::String
      ::String.new(LibWopkg.wopkg_version_string)
    end

    # The library's version as (major << 16) | (minor << 8) | patch, to compare with the header
    # this was built against (`LibWopkg::WOPKG_VERSION_*`).
    def self.abi_version : UInt32
      LibWopkg.wopkg_version
    end

    # Raise for a failed `status`, with libwopkg's sentence, releasing `err`.
    protected def self.check(status : Int32, err : LibWopkg::Error*) : Nil
      return if status == LibWopkg::WOPKG_OK
      raise Internal.new(status, FALLBACK_MESSAGE) if err.null?
      code = LibWopkg.wopkg_error_get_code(err)
      len = LibC::SizeT.new(0)
      bytes = LibWopkg.wopkg_error_get_message(err, pointerof(len))
      message = bytes.null? ? FALLBACK_MESSAGE : ::String.new(bytes, len)
      LibWopkg.wopkg_error_release(err)
      raise error(code, message)
    end

    # Copy an owned result string and release it.
    protected def self.take(s : LibWopkg::String*) : ::String
      return "" if s.null?
      begin
        len = LibC::SizeT.new(0)
        bytes = LibWopkg.wopkg_string_get_bytes(s, pointerof(len))
        bytes.null? ? "" : ::String.new(bytes, len)
      ensure
        LibWopkg.wopkg_string_release(s)
      end
    end

    # One libwopkg context: the person's package state (or the dirs `config` names). Every
    # method returns the result document as JSON text (`complete` and `history_record`: plain
    # text) or raises.
    class Context
      # The progress trampoline: hands each event (JSON text, valid during the call) to the block
      # boxed in `user_data`. Exceptions go to `Wol::Loop.on_error`; nothing unwinds into C.
      PROGRESS = ->(json : UInt8*, len : LibC::SizeT, data : Void*) do
        Callbacks.guard do
          block = Box(Proc(::String, Nil)).unbox(data)
          block.call(::String.new(json, len)) unless json.null?
        end
        nil
      end

      @ptr : LibWopkg::Context*

      # Open a context. `config` is the JSON of docs/modules/wopkg.md "JSON documents" (`{}`: the
      # person's defaults). Unknown keys are refused with `InvalidArgument`.
      def initialize(config : ::String = "{}")
        ptr = Pointer(LibWopkg::Context).null
        err = Pointer(LibWopkg::Error).null
        Pkg.check(LibWopkg.wopkg_context_create(config.to_unsafe, config.bytesize, pointerof(ptr), pointerof(err)), err)
        @ptr = ptr
      end

      # Release the context now. Later calls raise `UsageError`. Safe to call twice.
      def close : Nil
        ptr = @ptr
        @ptr = Pointer(LibWopkg::Context).null
        LibWopkg.wopkg_context_release(ptr) unless ptr.null?
      end

      def finalize
        LibWopkg.wopkg_context_release(@ptr) unless @ptr.null?
      end

      # True after `close`.
      def closed? : Bool
        @ptr.null?
      end

      # Search every source; one result per app (`limit` at most).
      def search(query : ::String, limit : Int = 20) : ::String
        result { |slot, err| LibWopkg.wopkg_search(live, query.to_unsafe, query.bytesize, limit.to_u32, slot, err) }
      end

      # Everything known about one package or app.
      def info(name : ::String) : ::String
        result { |slot, err| LibWopkg.wopkg_info(live, name.to_unsafe, name.bytesize, slot, err) }
      end

      # Names starting with `prefix`, for shell completion: plain text, one name per line.
      def complete(prefix : ::String, limit : Int = 200) : ::String
        result { |slot, err| LibWopkg.wopkg_complete(live, prefix.to_unsafe, prefix.bytesize, limit.to_u32, slot, err) }
      end

      # What is installed, from every source.
      def installed : ::String
        result { |slot, err| LibWopkg.wopkg_installed(live, slot, err) }
      end

      # Rebuild the index from `request`; the block gets each progress event.
      def index_update(request : ::String, &progress : ::String ->) : ::String
        with_progress(progress) do |cb, data|
          result { |slot, err| LibWopkg.wopkg_index_update(live, request.to_unsafe, request.bytesize, cb, data, slot, err) }
        end
      end

      # Download what `request` names, checked against its hash; the block gets progress events.
      def fetch(request : ::String, &progress : ::String ->) : ::String
        with_progress(progress) do |cb, data|
          result { |slot, err| LibWopkg.wopkg_fetch(live, request.to_unsafe, request.bytesize, cb, data, slot, err) }
        end
      end

      # Check a signature or checksum. Raises `Pkg::Untrusted` when it does not hold.
      def verify(request : ::String) : ::String
        result { |slot, err| LibWopkg.wopkg_verify(live, request.to_unsafe, request.bytesize, slot, err) }
      end

      # Read an AppImage's name, version and update information without running it.
      def appimage_inspect(path : ::String) : ::String
        result { |slot, err| LibWopkg.wopkg_appimage_inspect(live, path.to_unsafe, path.bytesize, slot, err) }
      end

      # Install an AppImage; the block gets progress events.
      def appimage_install(request : ::String, &progress : ::String ->) : ::String
        with_progress(progress) do |cb, data|
          result { |slot, err| LibWopkg.wopkg_appimage_install(live, request.to_unsafe, request.bytesize, cb, data, slot, err) }
        end
      end

      # Remove the AppImage `name`.
      def appimage_remove(name : ::String) : ::String
        result { |slot, err| LibWopkg.wopkg_appimage_remove(live, name.to_unsafe, name.bytesize, slot, err) }
      end

      # Update AppImages (a delta when it can); the block gets progress events.
      def appimage_update(request : ::String, &progress : ::String ->) : ::String
        with_progress(progress) do |cb, data|
          result { |slot, err| LibWopkg.wopkg_appimage_update(live, request.to_unsafe, request.bytesize, cb, data, slot, err) }
        end
      end

      # Every recorded change, oldest first.
      def history : ::String
        result { |slot, err| LibWopkg.wopkg_history(live, slot, err) }
      end

      # Record one change (`entry`: docs/modules/wopkg.md "History entry"); returns its id (plain
      # text, for `rollback`).
      def history_record(entry : ::String) : ::String
        result { |slot, err| LibWopkg.wopkg_history_record(live, entry.to_unsafe, entry.bytesize, slot, err) }
      end

      # Undo the recorded change `id`.
      def rollback(id : ::String) : ::String
        result { |slot, err| LibWopkg.wopkg_rollback(live, id.to_unsafe, id.bytesize, slot, err) }
      end

      # Ask the system service for a system change; it checks the person's consent.
      def system_call(request : ::String) : Nil
        err = Pointer(LibWopkg::Error).null
        Pkg.check(LibWopkg.wopkg_system_call(live, request.to_unsafe, request.bytesize, pointerof(err)), err)
      end

      # What a system update to the release in `request` would change.
      def release_plan(request : ::String) : ::String
        result { |slot, err| LibWopkg.wopkg_release_plan(live, request.to_unsafe, request.bytesize, slot, err) }
      end

      private def live : LibWopkg::Context*
        raise UsageError.new("Wol::Pkg::Context is closed") if @ptr.null?
        @ptr
      end

      private def result(& : LibWopkg::String**, LibWopkg::Error** -> Int32) : ::String
        slot = Pointer(LibWopkg::String).null
        err = Pointer(LibWopkg::Error).null
        status = yield pointerof(slot), pointerof(err)
        Pkg.check(status, err)
        Pkg.take(slot)
      end

      # Progress runs only during the call (wopkg.h), so the box lives in this frame for exactly
      # as long as libwopkg may call it.
      private def with_progress(progress : ::String ->, & : LibWopkg::ProgressFn, Void* -> T) : T forall T
        box = Box(Proc(::String, Nil)).box(progress)
        yield PROGRESS, box
      end
    end
  end
end

# SPDX-License-Identifier: GPL-2.0-or-later
require "json"
require "./lib_wopkg"

# Wopkg::Core: libwopkg (crates/wopkg/include/wopkg.h, ADR 0021) as parsed JSON and exceptions, over the
# generated `lib LibWopkg` (lib_wopkg.cr, never hand-edited: ADR 0036). Each call owns its result
# string and error and releases them before returning.

module Wopkg
  # A libwopkg failure: the status code and the sentence to show.
  class Failure < Exception
    getter code : Int32

    STATUS = {
      LibWopkg::WOPKG_ERR_INVALID_ARGUMENT  => "invalid",
      LibWopkg::WOPKG_ERR_PERMISSION_DENIED => "denied",
      LibWopkg::WOPKG_ERR_UNAVAILABLE       => "unavailable",
      LibWopkg::WOPKG_ERR_INTERNAL          => "internal",
      LibWopkg::WOPKG_ERR_UNTRUSTED         => "untrusted",
      LibWopkg::WOPKG_ERR_NOT_FOUND         => "not-found",
    }

    # The prefix of the one stderr line that carries what the system service said about a change
    # (wold, and through it wol-admind: Update Now's restore point, or why none could be kept).
    # Settings reads the sentence after it, so it is stable: never reworded.
    SYSTEM_PREFIX = "wopkg: system: "

    # The failure came from the system service (a `system_call`), not from wopkg itself.
    property? system = false

    def initialize(@code : Int32, message : String)
      super(message)
    end

    def kind : String
      STATUS[@code]? || "internal"
    end

    # The stderr line: a system failure on one line under SYSTEM_PREFIX, others as they are.
    def line : String
      m = message || ""
      system? ? "#{SYSTEM_PREFIX}#{m.gsub(/[[:cntrl:]]+/, " ").strip}" : m
    end
  end

  # Owns one libwopkg context; every call returns parsed JSON or raises Failure.
  class Core
    alias Progress = Proc(JSON::Any, Nil)
    @ctx : LibWopkg::Context*

    def initialize(config : String = "{}")
      ctx = Pointer(LibWopkg::Context).null
      err = Pointer(LibWopkg::Error).null
      check LibWopkg.wopkg_context_create(config, config.bytesize, pointerof(ctx), pointerof(err)), err
      @ctx = ctx
    end

    # A failed initialize leaves @ctx null; there is nothing to release then.
    def finalize
      LibWopkg.wopkg_context_release(@ctx) unless @ctx.null?
    end

    # The library's version text, for `wopkg version`.
    def self.version : String
      String.new(LibWopkg.wopkg_version_string)
    end

    # The library's version as (major << 16) | (minor << 8) | patch, to compare with the header
    # this was built against (`LibWopkg::WOPKG_VERSION_*`).
    def self.abi_version : UInt32
      LibWopkg.wopkg_version
    end

    private def check(status : LibWopkg::Status, err : LibWopkg::Error*)
      return if status == LibWopkg::WOPKG_OK
      len = LibC::SizeT.new(0)
      ptr = LibWopkg.wopkg_error_get_message(err, pointerof(len))
      msg = String.new(ptr, len)
      LibWopkg.wopkg_error_release(err)
      raise Failure.new(status, msg)
    end

    private def take(s : LibWopkg::String*) : String
      len = LibC::SizeT.new(0)
      ptr = LibWopkg.wopkg_string_get_bytes(s, pointerof(len))
      text = String.new(ptr, len)
      LibWopkg.wopkg_string_release(s)
      text
    end

    private def json(&) : JSON::Any
      res = Pointer(LibWopkg::String).null
      err = Pointer(LibWopkg::Error).null
      check yield(pointerof(res), pointerof(err)), err
      JSON.parse(take(res))
    end

    # The call's result as libwopkg's JSON text, for callers that decode it into their own types.
    private def text(&) : String
      res = Pointer(LibWopkg::String).null
      err = Pointer(LibWopkg::Error).null
      check yield(pointerof(res), pointerof(err)), err
      take(res)
    end

    # Runs a progress-reporting call; `progress` gets each JSON event on this thread.
    private def with_progress(progress : Progress?, &)
      box = Box.box(progress || Progress.new { |_| nil })
      cb = ->(p : UInt8*, n : LibC::SizeT, ud : Void*) {
        Box(Progress).unbox(ud).call(JSON.parse(String.new(p, n)))
        nil
      }
      yield cb, box
    end

    def index_update(req : String, progress : Progress? = nil) : JSON::Any
      with_progress(progress) do |cb, ud|
        json { |o, e| LibWopkg.wopkg_index_update(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def search(q : String, limit = 20) : JSON::Any
      json { |o, e| LibWopkg.wopkg_search(@ctx, q, q.bytesize, limit.to_u32, o, e) }
    end

    def info(name : String) : JSON::Any
      json { |o, e| LibWopkg.wopkg_info(@ctx, name, name.bytesize, o, e) }
    end

    def complete(prefix : String, limit = 200) : String
      res = Pointer(LibWopkg::String).null
      err = Pointer(LibWopkg::Error).null
      check LibWopkg.wopkg_complete(@ctx, prefix, prefix.bytesize, limit.to_u32, pointerof(res), pointerof(err)), err
      take(res)
    end

    def installed : JSON::Any
      json { |o, e| LibWopkg.wopkg_installed(@ctx, o, e) }
    end

    def fetch(req : String, progress : Progress? = nil) : JSON::Any
      with_progress(progress) do |cb, ud|
        json { |o, e| LibWopkg.wopkg_fetch(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def verify(req : String) : JSON::Any
      json { |o, e| LibWopkg.wopkg_verify(@ctx, req, req.bytesize, o, e) }
    end

    def appimage_inspect(path : String) : JSON::Any
      json { |o, e| LibWopkg.wopkg_appimage_inspect(@ctx, path, path.bytesize, o, e) }
    end

    def appimage_install(req : String, progress : Progress? = nil) : JSON::Any
      with_progress(progress) do |cb, ud|
        json { |o, e| LibWopkg.wopkg_appimage_install(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def appimage_remove(name : String) : String
      res = Pointer(LibWopkg::String).null
      err = Pointer(LibWopkg::Error).null
      check LibWopkg.wopkg_appimage_remove(@ctx, name, name.bytesize, pointerof(res), pointerof(err)), err
      take(res)
    end

    def appimage_update(req : String, progress : Progress? = nil) : JSON::Any
      with_progress(progress) do |cb, ud|
        json { |o, e| LibWopkg.wopkg_appimage_update(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def history : JSON::Any
      json { |o, e| LibWopkg.wopkg_history(@ctx, o, e) }
    end

    def record(entry : String) : String
      res = Pointer(LibWopkg::String).null
      err = Pointer(LibWopkg::Error).null
      check LibWopkg.wopkg_history_record(@ctx, entry, entry.bytesize, pointerof(res), pointerof(err)), err
      take(res)
    end

    def rollback(id : String) : JSON::Any
      json { |o, e| LibWopkg.wopkg_rollback(@ctx, id, id.bytesize, o, e) }
    end

    def system_call(req : String) : Nil
      err = Pointer(LibWopkg::Error).null
      check LibWopkg.wopkg_system_call(@ctx, req, req.bytesize, pointerof(err)), err
    rescue e : Failure
      e.system = true
      raise e
    end

    def release_plan(req : String) : JSON::Any
      json { |o, e| LibWopkg.wopkg_release_plan(@ctx, req, req.bytesize, o, e) }
    end

    # The signed curated shard list (libwopkg 1.1), as JSON text.
    def shard_index(req : String) : String
      text { |o, e| LibWopkg.wopkg_shard_index(@ctx, req, req.bytesize, o, e) }
    end

    # Pins shards by commit in a project's shard.yml and writes its shard.lock, as JSON text.
    def shard_pin(req : String) : String
      text { |o, e| LibWopkg.wopkg_shard_pin(@ctx, req, req.bytesize, o, e) }
    end

    # Checks a project's shard.yml and shard.lock (RCR-040, RCR-041, the list), as JSON text.
    def shard_check(req : String) : String
      text { |o, e| LibWopkg.wopkg_shard_check(@ctx, req, req.bytesize, o, e) }
    end
  end
end

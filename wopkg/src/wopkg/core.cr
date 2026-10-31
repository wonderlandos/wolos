# SPDX-License-Identifier: BSD-2-Clause
require "json"
require "./lib_wopkg"

module Wopkg
  # A libwopkg failure: the status code and the sentence to show.
  class Failure < Exception
    getter code : Int32

    STATUS = {1 => "invalid", 2 => "denied", 3 => "unavailable", 4 => "internal", 5 => "untrusted", 6 => "not-found"}

    def initialize(@code : Int32, message : String)
      super(message)
    end

    def kind : String
      STATUS[@code]? || "internal"
    end
  end

  # Owns one libwopkg context; every call returns parsed JSON or raises Failure.
  class Core
    alias Progress = Proc(JSON::Any, Nil)
    @ctx : LibWopkg::Context

    def initialize(config : String = "{}")
      ctx = uninitialized LibWopkg::Context
      err = Pointer(Void).null
      check LibWopkg.context_create(config, config.bytesize, pointerof(ctx), pointerof(err)), err
      @ctx = ctx
    end

    def finalize
      LibWopkg.context_release(@ctx)
    end

    def self.version : String
      String.new(LibWopkg.version_string)
    end

    private def check(status : Int32, err : LibWopkg::Error)
      return if status == 0
      len = LibC::SizeT.new(0)
      ptr = LibWopkg.error_get_message(err, pointerof(len))
      msg = String.new(ptr, len)
      LibWopkg.error_release(err)
      raise Failure.new(status, msg)
    end

    private def take(s : LibWopkg::WString) : String
      len = LibC::SizeT.new(0)
      ptr = LibWopkg.string_get_bytes(s, pointerof(len))
      text = String.new(ptr, len)
      LibWopkg.string_release(s)
      text
    end

    private def json(&) : JSON::Any
      res = Pointer(Void).null
      err = Pointer(Void).null
      check yield(pointerof(res), pointerof(err)), err
      JSON.parse(take(res))
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
        json { |o, e| LibWopkg.index_update(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def search(q : String, limit = 20) : JSON::Any
      json { |o, e| LibWopkg.search(@ctx, q, q.bytesize, limit.to_u32, o, e) }
    end

    def info(name : String) : JSON::Any
      json { |o, e| LibWopkg.info(@ctx, name, name.bytesize, o, e) }
    end

    def complete(prefix : String, limit = 200) : String
      res = Pointer(Void).null
      err = Pointer(Void).null
      check LibWopkg.complete(@ctx, prefix, prefix.bytesize, limit.to_u32, pointerof(res), pointerof(err)), err
      take(res)
    end

    def installed : JSON::Any
      json { |o, e| LibWopkg.installed(@ctx, o, e) }
    end

    def fetch(req : String, progress : Progress? = nil) : JSON::Any
      with_progress(progress) do |cb, ud|
        json { |o, e| LibWopkg.fetch(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def verify(req : String) : JSON::Any
      json { |o, e| LibWopkg.verify(@ctx, req, req.bytesize, o, e) }
    end

    def appimage_inspect(path : String) : JSON::Any
      json { |o, e| LibWopkg.appimage_inspect(@ctx, path, path.bytesize, o, e) }
    end

    def appimage_install(req : String, progress : Progress? = nil) : JSON::Any
      with_progress(progress) do |cb, ud|
        json { |o, e| LibWopkg.appimage_install(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def appimage_remove(name : String) : String
      res = Pointer(Void).null
      err = Pointer(Void).null
      check LibWopkg.appimage_remove(@ctx, name, name.bytesize, pointerof(res), pointerof(err)), err
      take(res)
    end

    def appimage_update(req : String, progress : Progress? = nil) : JSON::Any
      with_progress(progress) do |cb, ud|
        json { |o, e| LibWopkg.appimage_update(@ctx, req, req.bytesize, cb, ud, o, e) }
      end
    end

    def history : JSON::Any
      json { |o, e| LibWopkg.history(@ctx, o, e) }
    end

    def record(entry : String) : String
      res = Pointer(Void).null
      err = Pointer(Void).null
      check LibWopkg.history_record(@ctx, entry, entry.bytesize, pointerof(res), pointerof(err)), err
      take(res)
    end

    def rollback(id : String) : JSON::Any
      json { |o, e| LibWopkg.rollback(@ctx, id, id.bytesize, o, e) }
    end

    def system_call(req : String) : Nil
      err = Pointer(Void).null
      check LibWopkg.system_call(@ctx, req, req.bytesize, pointerof(err)), err
    end

    def release_plan(req : String) : JSON::Any
      json { |o, e| LibWopkg.release_plan(@ctx, req, req.bytesize, o, e) }
    end
  end
end

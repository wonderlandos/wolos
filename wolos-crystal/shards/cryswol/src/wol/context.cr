# SPDX-License-Identifier: BSD-2-Clause
# Context: one app's connection to the system (wol.h "Context and event loop"). Everything else is
# made from a context, and callbacks run only inside `Context#dispatch`, which only `Wol::Loop` (or
# a spec) calls. The context also roots every object that has a callback (plan-cr1 §3 rule 3):
# libwol keeps raw `user_data` pointers the GC cannot see.
require "./lib_wol"
require "./error"
require "./handle"
require "./string"

module Wol
  class Context < Handle(LibWol::Context)
    private F_DUPFD_CLOEXEC = 1030

    # The reverse-DNS app id the context was opened with.
    getter app_id : ::String

    @live = {} of UInt64 => Rootable
    @dispatching = false
    @wake : IO::FileDescriptor? = nil
    @running = false

    # Open a context for `app_id` (reverse-DNS, for example "org.example.hello"). Raises
    # `InvalidArgument` for a bad id. Opening does not need the system service; the first call
    # that does raises `Unavailable` when it is not there.
    def self.open(app_id : ::String) : Context
      new(app_id)
    end

    # Open a context, yield it, close it afterwards.
    def self.open(app_id : ::String, & : Context -> T) : T forall T
      ctx = new(app_id)
      begin
        yield ctx
      ensure
        ctx.close
      end
    end

    # Same as `Context.open`.
    def initialize(@app_id : ::String)
      ptr = Wol.out(LibWol::Context) do |slot, err|
        LibWol.wol_context_create(Strings.ptr(@app_id), Strings.size(@app_id), slot, err)
      end
      super(ptr)
    end

    # Run the callbacks of everything that has arrived, waiting up to `timeout_ms` first
    # (-1 waits until something arrives, 0 not at all). Raises `UsageError` when called from inside
    # a callback (libwol would deadlock or reorder). Callbacks run inline: keep them short.
    def dispatch(timeout_ms : Int32 = 0) : Nil
      raise UsageError.new("Context#dispatch called from inside a callback") if @dispatching
      @dispatching = true
      begin
        Wol.call { |err| LibWol.wol_context_dispatch(self, timeout_ms, err) }
      ensure
        @dispatching = false
      end
    end

    # True while `dispatch` is running callbacks.
    def dispatching? : Bool
      @dispatching
    end

    # Call `block` with every event on `topic` ("active-window", "open-window", "close-window",
    # "workspace", "theme", "job"). The event is valid only inside the block (`Event#keep` to hold
    # it). Delivery stops with `Subscription#cancel` or when the context closes.
    def subscribe(topic : ::String, &block : Event ->) : Subscription
      Subscription.new(self, topic, block)
    end

    # Close every rooted object, then release the context. Safe to call twice.
    def close : Nil
      return if closed?
      @running = false
      @live.values.each(&.close)
      @live.clear
      close_wake
      super
    end

    # Keep `obj` alive until `unroot` (used by `Handle#root!`).
    def root(obj : Rootable) : Nil
      @live[obj.object_id] = obj
    end

    # Stop keeping `obj` alive (used by `Handle#close`).
    def unroot(obj : Rootable) : Nil
      @live.delete(obj.object_id)
    end

    # How many objects this context keeps alive for their callbacks (for specs and leak checks).
    def rooted_count : Int32
      @live.size
    end

    # A Crystal-owned duplicate of libwol's wake fd. The loop reads this one and Crystal closes
    # it, because closing libwol's own fd behind the event loop leaves a stale registration that
    # hangs the next context given the same fd number (plan-cr1 "S0 results").
    # :nodoc:
    def wake_io : IO::FileDescriptor
      @wake ||= begin
        fd = LibWol.wol_context_get_fd(self)
        raise Internal.new(LibWol::WOL_ERR_INTERNAL, FALLBACK_MESSAGE) if fd < 0
        # F_DUPFD_CLOEXEC (1030 on Linux): the dup must not leak into child processes.
        dup = LibC.fcntl(fd, F_DUPFD_CLOEXEC, 0)
        raise Internal.new(LibWol::WOL_ERR_INTERNAL, FALLBACK_MESSAGE) if dup < 0
        IO::FileDescriptor.new(dup)
      end
    end

    # :nodoc:
    property? running : Bool

    # :nodoc:
    # Close the wake dup; a loop blocked reading it wakes with `IO::Error`.
    def close_wake : Nil
      @wake.try &.close
      @wake = nil
    end

    protected def release(ptr : LibWol::Context*) : Nil
      LibWol.wol_context_release(ptr)
    end
  end
end

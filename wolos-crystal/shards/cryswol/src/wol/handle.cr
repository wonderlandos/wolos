# SPDX-License-Identifier: BSD-2-Clause
# Handle: the one base class for every libwol object the app owns (plan-cr1 §3 rule 1). It holds
# the pointer, releases it once (explicit `close` or the GC finalizer, whichever comes first), and
# refuses use after close with `UsageError`. libwol also refuses stale handles (its tag check in
# crates/wol-abi/src/handle.rs), so this is the second line of defence, not the only one.
#
# A handle that registered a callback is "rooted": its context keeps it alive (libwol's heap is
# invisible to the GC, so `user_data` alone does not keep it alive; rule 3) until `close` or until
# the context closes.
require "./lib_wol"
require "./error"

module Wol
  # Anything a context keeps alive for its callbacks: it must be closable, because closing the
  # context closes it. Every `Handle` is one.
  module Rootable
    abstract def close : Nil
  end

  # Base of every owned libwol object: `T` is the opaque `LibWol` struct.
  #
  # A subclass implements `release(ptr)` with the matching `wol_*_release`:
  #
  # ```
  # class Window < Wol::Handle(LibWol::Window)
  #   protected def release(ptr : LibWol::Window*) : Nil
  #     LibWol.wol_window_release(ptr)
  #   end
  # end
  # ```
  #
  # A subclass whose create call needs `self` (a callback's `user_data`) cannot call `super(ptr)`
  # first, so it sets `@context`, makes the call, then calls `adopt(ptr)` and `root!`:
  #
  # ```
  # def initialize(context : Context, @block : Event ->)
  #   @context = context
  #   adopt(Wol.out(LibWol::Subscription) { |slot, err| LibWol.wol_subscribe(..., Callbacks.user_data(self), slot, err) })
  #   root!
  # end
  # ```
  abstract class Handle(T)
    include Rootable

    @ptr : Pointer(T) = Pointer(T).null

    # The context this object was made from, or nil (the context itself, a free-standing object).
    getter context : Context?

    # Wrap `ptr`, which must be a reference the caller owns (a `*_create`/`*_copy` result).
    def initialize(ptr : Pointer(T), @context : Context? = nil)
      adopt(ptr)
    end

    # Take ownership of `ptr` (see the class doc). Raises `UsageError` for NULL or a second adopt.
    protected def adopt(ptr : Pointer(T)) : Nil
      raise UsageError.new("#{self.class.name}: libwol returned no object") if ptr.null?
      raise UsageError.new("#{self.class.name}: already holds an object") unless @ptr.null?
      @ptr = ptr
    end

    # The raw pointer, for passing to `LibWol` (Crystal calls this for `fun` arguments).
    # Raises `UsageError` once the handle is closed.
    def to_unsafe : Pointer(T)
      ptr = @ptr
      raise UsageError.new("#{self.class.name} is closed") if ptr.null?
      ptr
    end

    # True after `close`.
    def closed? : Bool
      @ptr.null?
    end

    # Release the libwol reference now and stop any callback. Safe to call twice.
    def close : Nil
      ptr = @ptr
      return if ptr.null?
      @ptr = Pointer(T).null
      before_release(ptr)
      release(ptr)
      @context.try &.unroot(self)
    end

    # The GC finalizer: the same release as `close`, without touching other Crystal objects
    # (their finalizers may already have run).
    def finalize
      ptr = @ptr
      return if ptr.null?
      @ptr = Pointer(T).null
      release(ptr)
    end

    # Keep this object alive while libwol holds `user_data` pointing at it. Call this right after
    # handing `Callbacks.user_data(self)` to libwol. Needs a context.
    protected def root! : Nil
      ctx = @context
      raise UsageError.new("#{self.class.name}: callbacks need a context") unless ctx
      ctx.root(self)
    end

    # Hook run by `close` just before the release, with the pointer still valid (for example to
    # cancel a subscription first).
    protected def before_release(ptr : Pointer(T)) : Nil
    end

    # Drop one libwol reference with the matching `wol_*_release`.
    protected abstract def release(ptr : Pointer(T)) : Nil
  end
end

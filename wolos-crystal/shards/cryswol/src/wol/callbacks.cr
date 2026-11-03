# SPDX-License-Identifier: BSD-2-Clause
# Callbacks from libwol into Crystal (plan-cr1 §3 rules 3 and 4).
#
# Concept: C gets a plain function pointer and a `user_data` pointer. Crystal cannot hand C a
# closure, so every callback is a class-level proc literal (a "trampoline") that captures nothing,
# turns `user_data` back into the wrapper object and calls it.
#
# Why: wol.h says callbacks "must not unwind", and libwol's heap is invisible to the GC.
#
# How: `Callbacks.user_data(obj)` gives the pointer to pass; the wrapper calls `root!` so its
# context keeps it alive; the trampoline calls `Callbacks.invoke`, which rescues every exception
# and hands it to `Wol::Loop.on_error`. Objects valid only during a callback include `Scoped` and
# are expired by `Callbacks.scoped` when the callback returns.
#
# ```
# CLICK = ->(w : LibWol::Widget*, data : Void*) do
#   Wol::Callbacks.invoke(Button, data) { |button| button.fire_click }
#   nil
# end
# ```
require "./lib_wol"
require "./error"

module Wol
  module Callbacks
    # The `user_data` pointer for `obj`. The caller must keep `obj` alive (`Handle#root!`).
    def self.user_data(obj : Reference) : Void*
      obj.as(Void*)
    end

    # Turn `data` back into the `T` it was made from and yield it, rescuing everything: nothing
    # raised in Crystal ever unwinds into libwol. A NULL `data` is ignored.
    def self.invoke(type : T.class, data : Void*, & : T ->) : Nil forall T
      return if data.null?
      guard { yield data.as(T) }
    end

    # Run the block; any exception goes to `Wol::Loop.on_error` instead of propagating. This and
    # the two methods below are the only places Cryswol catches every exception: each is a
    # boundary nothing may cross (C below a callback, the error handler, another thread).
    def self.guard(&) : Nil
      yield
    rescue ex
      report(ex)
    end

    # Hand `ex` to `Wol::Loop.on_error`. Never raises: a raising handler is reported on STDERR.
    def self.report(ex : Exception) : Nil
      Loop.on_error.call(ex)
    rescue error
      STDERR.puts "cryswol: the error handler raised #{error.class} while handling #{ex.class}"
    end

    # The block's value, or the exception it raised, so it can be raised again on another thread.
    def self.capture(& : -> T) : T | Exception forall T
      yield
    rescue ex
      ex
    end

    # Yield `obj`, then expire it whatever happens: it was valid only during this callback.
    def self.scoped(obj : Scoped, &) : Nil
      yield obj
    ensure
      obj.expire!
    end
  end

  # An object libwol lends only for the length of one callback (an event, a message, a search
  # reply; wol.h "valid only inside the callback"). After the callback every use raises
  # `UsageError`; call `keep` where the type offers it to hold on longer.
  module Scoped
    @valid = true

    # True while the callback that received this object is running.
    def valid? : Bool
      @valid
    end

    # Mark the object dead; done by `Callbacks.scoped` when the callback returns.
    def expire! : Nil
      @valid = false
    end

    # Raise `UsageError` unless still valid. Call first in every method that touches libwol.
    private def check_valid! : Nil
      raise UsageError.new("#{self.class.name} was used after its callback returned") unless @valid
    end
  end
end

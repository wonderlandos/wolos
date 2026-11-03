# SPDX-License-Identifier: BSD-2-Clause
# Events and subscriptions (wol.h "Events"). An `Event` is lent for one callback only (wol.h: keep
# it past its callback with retain); `Event#keep` returns a copy that stays valid until closed.
require "json"
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"

module Wol
  # One system event: its topic and its JSON (one object with an "event" key).
  class Event
    include Scoped

    @ptr : LibWol::Event*
    @kept : Bool

    # Wrap a borrowed event (inside a callback) or, with `kept: true`, a retained one.
    def initialize(@ptr : LibWol::Event*, @kept : Bool = false)
    end

    # The topic, for example "theme".
    def topic : ::String
      Strings.borrow(LibWol.wol_event_get_topic(live_ptr)) || ""
    end

    # The event as JSON text.
    def json : ::String
      Strings.borrow(LibWol.wol_event_get_json(live_ptr)) || ""
    end

    # The event decoded into `type` (a `JSON::Serializable`), for example
    # `event.decode(ThemeEvent)`. Raises `JSON::SerializableError` when it does not fit.
    def decode(type : T.class) : T forall T
      T.from_json(json)
    end

    # A copy that stays valid after the callback returns (until `close` or the GC).
    def keep : Event
      Event.new(LibWol.wol_event_retain(live_ptr), kept: true)
    end

    # Release a kept event. No effect on a borrowed one.
    def close : Nil
      return unless @kept && valid?
      expire!
      LibWol.wol_event_release(@ptr)
    end

    def finalize
      LibWol.wol_event_release(@ptr) if @kept && valid?
    end

    private def live_ptr : LibWol::Event*
      check_valid!
      @ptr
    end
  end

  # A running subscription; the block stays registered until `cancel`/`close` or the context closes.
  class Subscription < Handle(LibWol::Subscription)
    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    TRAMPOLINE = ->(event : LibWol::Event*, data : Void*) do
      Callbacks.invoke(Subscription, data, &.deliver(event))
      nil
    end

    # The topic this subscription listens to.
    getter topic : ::String
    @block : Event ->

    # Use `Context#subscribe`.
    def initialize(context : Context, @topic : ::String, @block : Event ->)
      @context = context
      adopt(Wol.out(LibWol::Subscription) do |slot, err|
        LibWol.wol_subscribe(context, Strings.ptr(@topic), Strings.size(@topic), TRAMPOLINE,
          Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    # Stop delivery at once and release the subscription. Same as `close`.
    def cancel : Nil
      close
    end

    protected def deliver(raw : LibWol::Event*) : Nil
      return if closed?
      Callbacks.scoped(Event.new(raw)) { |event| @block.call(event) }
    end

    protected def before_release(ptr : LibWol::Subscription*) : Nil
      LibWol.wol_subscription_cancel(ptr)
    end

    protected def release(ptr : LibWol::Subscription*) : Nil
      LibWol.wol_subscription_release(ptr)
    end
  end
end

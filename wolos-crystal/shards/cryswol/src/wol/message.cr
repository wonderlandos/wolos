# SPDX-License-Identifier: BSD-2-Clause
# Messages between apps (wol.h "1.11: messages between apps", WOL_1.11). An app is addressed by its
# full path, which no other app can take, and an endpoint name; the system routes every message and
# tells the receiver who sent it. Only apps of one account talk. A received message is lent to the
# block (plan-cr1 §3 rule 4); `Message#keep` holds it to reply later, within 3 seconds.
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"

module Wol
  # One message from another app: who sent it and what it says.
  class Message
    include Scoped

    @ptr : LibWol::Message*
    @kept : Bool

    # Wrap a borrowed message (inside the endpoint's block) or, with `kept: true`, a retained one.
    def initialize(@ptr : LibWol::Message*, @kept : Bool = false)
    end

    # The sender's executable, as the kernel saw it.
    def from : ::String
      Strings.borrow(LibWol.wol_message_get_from(live_ptr)) || ""
    end

    # The text.
    def body : ::String
      Strings.borrow(LibWol.wol_message_get_body(live_ptr)) || ""
    end

    # Answer the sender (up to 24 KiB). A published message takes no reply.
    def reply(body : ::String) : Nil
      ptr = live_ptr
      Wol.call { |err| LibWol.wol_message_reply(ptr, Strings.ptr(body), Strings.size(body), err) }
    end

    # A copy that stays valid after the block returns, to reply later (until `close` or the GC).
    def keep : Message
      Message.new(LibWol.wol_message_retain(live_ptr), kept: true)
    end

    # Release a kept message. No effect on a borrowed one.
    def close : Nil
      return unless @kept
      ptr = @ptr
      @ptr = Pointer(LibWol::Message).null
      expire!
      LibWol.wol_message_release(ptr) unless ptr.null?
    end

    def finalize
      LibWol.wol_message_release(@ptr) if @kept && !@ptr.null?
    end

    # Send `body` (up to 24 KiB) to endpoint `name` of the app at path `peer` and return its
    # reply. Needs the grant ipc:<peer>.
    # Blocks: up to 3 seconds for the reply; never send to this app from the dispatching fiber.
    def self.send(context : Context, peer : ::String, name : ::String, body : ::String) : ::String
      str = Wol.out(LibWol::String) do |o, err|
        LibWol.wol_message_send(context, Strings.ptr(peer), Strings.size(peer),
          Strings.ptr(name), Strings.size(name), Strings.ptr(body), Strings.size(body), o, err)
      end
      Strings.take(str) || ""
    end

    # Hand `body` to the endpoints named `topic` of every app holding the grant ipc:<this app>.
    # They take no reply, and this app is not told who heard. Permission: ipc.
    def self.publish(context : Context, topic : ::String, body : ::String) : Nil
      Wol.call do |err|
        LibWol.wol_publish(context, Strings.ptr(topic), Strings.size(topic),
          Strings.ptr(body), Strings.size(body), err)
      end
    end

    private def live_ptr : LibWol::Message*
      check_valid!
      raise UsageError.new("Wol::Message is closed") if @ptr.null?
      @ptr
    end
  end

  # Listening under one name: the block gets every message sent to it. One process of an app per
  # name; it listens until `close` or the context closes. Permission: ipc.
  class Endpoint < Handle(LibWol::Endpoint)
    # The trampoline libwol calls; captures nothing (see `Callbacks`).
    ON_MESSAGE = ->(_endpoint : LibWol::Endpoint*, message : LibWol::Message*, data : Void*) do
      Callbacks.invoke(Endpoint, data) { |endpoint| endpoint.deliver(message) }
      nil
    end

    # The name it listens under.
    getter name : ::String
    @block : Message ->

    # Listen under `name` (letters, digits, '.', '_', '-').
    #
    # ```
    # Wol::Endpoint.new(ctx, "ping") { |m| m.reply("pong") }
    # ```
    def initialize(context : Context, @name : ::String, &@block : Message ->)
      @context = context
      adopt(Wol.out(LibWol::Endpoint) do |slot, err|
        LibWol.wol_endpoint_create(context, Strings.ptr(@name), Strings.size(@name), ON_MESSAGE,
          Callbacks.user_data(self), slot, err)
      end)
      root!
    end

    protected def deliver(raw : LibWol::Message*) : Nil
      return if closed?
      Callbacks.scoped(Message.new(raw)) { |message| @block.call(message) }
    end

    protected def release(ptr : LibWol::Endpoint*) : Nil
      LibWol.wol_endpoint_release(ptr)
    end
  end
end

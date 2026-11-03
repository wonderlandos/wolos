# SPDX-License-Identifier: BSD-2-Clause
# Notifications (wol.h "Notifications", "1.1: notifications with buttons", "1.2: replace and
# withdraw"; permission: notifications). libwol only checks and asks; wold decides and shows.
#
# Why the action handler roots the object: a pressed button reaches the handler only while the
# notification object is alive (libwol keeps it weakly by wold's id), and libwol calls the
# handler with a raw `user_data` pointer the GC cannot see. So `on_action` roots it in its
# context until `close` (or the context closes). The button key is borrowed for the callback, so
# it is copied into a Crystal String before the block sees it.
require "./lib_wol"
require "./error"
require "./handle"
require "./callbacks"
require "./string"
require "./context"

module Wol
  # A notification with up to 3 buttons. Nothing is shown until `send`.
  class Notification < Handle(LibWol::Notification)
    # The trampoline libwol calls when a button is pressed; captures nothing.
    TRAMPOLINE = ->(_note : LibWol::Notification*, key : LibWol::String*, data : Void*) do
      Callbacks.invoke(Notification, data) { |note| note.fire(Strings.borrow(key) || "") }
      nil
    end

    # The title, as last set by `new` or `replace`.
    getter title : ::String
    # The body, as last set by `new` or `replace`.
    getter body : ::String

    @on_action : (Notification, ::String -> Nil)? = nil

    # Show a notification at once, without buttons. Raises `PermissionDenied` when the person
    # turned notifications off for this app, `Unavailable` when the system service is not there.
    def self.post(context : Context, title : ::String, body : ::String = "") : Nil
      Wol.call do |err|
        LibWol.wol_notification_post(context, Strings.ptr(title), Strings.size(title), Strings.ptr(body),
          Strings.size(body), err)
      end
    end

    # Prepare a notification. Raises `InvalidArgument` for a title or body over the limits.
    def initialize(context : Context, @title : ::String, @body : ::String = "")
      super(Wol.out(LibWol::Notification) do |slot, err|
        LibWol.wol_notification_create(context, Strings.ptr(@title), Strings.size(@title), Strings.ptr(@body),
          Strings.size(@body), slot, err)
      end, context)
    end

    # Add a button. `key`: 1-32 of [a-z0-9_-], unique; the key "default" stands for a click on
    # the notification itself. Up to 3. Raises `InvalidArgument` otherwise.
    def add_action(key : ::String, label : ::String) : self
      Wol.call do |err|
        LibWol.wol_notification_add_action(self, Strings.ptr(key), Strings.size(key), Strings.ptr(label),
          Strings.size(label), err)
      end
      self
    end

    # Call the block with the key of each pressed button (inside dispatch). Replaces an earlier
    # handler. Keeps this object alive until `close`.
    def on_action(&block : Notification, ::String -> Nil) : self
      @on_action = block
      Wol.call do |err|
        LibWol.wol_notification_set_action_handler(self, TRAMPOLINE, Callbacks.user_data(self), err)
      end
      root!
      self
    end

    # Remove the action handler.
    def clear_action_handler : self
      Wol.call { |err| LibWol.wol_notification_set_action_handler(self, nil, nil, err) }
      @on_action = nil
      self
    end

    # Show it. With buttons, libwol listens for the answer first, so a press is never lost.
    # Raises `PermissionDenied` or `Unavailable` as `post` does.
    def send : self
      Wol.call { |err| LibWol.wol_notification_send(self, err) }
      self
    end

    # New text. Already sent: it changes on screen, buttons kept. Not sent yet: used when sent.
    def replace(title : ::String, body : ::String = "") : self
      Wol.call do |err|
        LibWol.wol_notification_replace(self, Strings.ptr(title), Strings.size(title), Strings.ptr(body),
          Strings.size(body), err)
      end
      @title = title
      @body = body
      self
    end

    # Take it off the screen and out of Notification Center. Nothing happens if it was never sent.
    def withdraw : self
      Wol.call { |err| LibWol.wol_notification_withdraw(self, err) }
      self
    end

    protected def fire(key : ::String) : Nil
      @on_action.try &.call(self, key)
    end

    # Drop the block, so a block that captured this notification does not keep a cycle alive.
    protected def before_release(_ptr : LibWol::Notification*) : Nil
      @on_action = nil
    end

    protected def release(ptr : LibWol::Notification*) : Nil
      LibWol.wol_notification_release(ptr)
    end
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# The recorder backend: windows that keep the patches they were sent instead of drawing them. DSL
# specs use it to pin what a view emits, with no engine and no display (`Wolkit::Patch.describe`
# turns the bytes into lines). `Wolkit::Test::Recorder` is also the backend of the headless harness
# when the engine is not linked.
require "../window"
require "../app"
require "../patch/describe"

module Wolkit
  module Test
    class RecordedWindow < WindowBackend
      getter id : UInt32
      getter title : String
      getter patches = [] of Bytes
      # How many patches were sent, kept even when the bytes are not (a Recorder made with keep: false).
      getter sent = 0
      getter? shown : Bool = false
      getter? closed : Bool = false

      def initialize(@id : UInt32, @title : String, @width : Int32, @height : Int32, @keep : Bool = true)
      end

      def apply(bytes : Bytes) : Nil
        @sent += 1
        @patches << bytes.dup if @keep
      end

      def show : Nil
        @shown = true
      end

      def close : Nil
        @closed = true
      end

      def title=(title : String) : String
        @title = title
      end

      def size : {UInt32, UInt32, Float32}
        {@width.to_u32, @height.to_u32, 1.0_f32}
      end

      # Every op of every patch sent so far, one line each (the PATCH header lines left out).
      def ops : Array(String)
        @patches.flat_map { |patch| Patch.describe(patch).reject(&.starts_with?("PATCH ")) }
      end

      # The id of the part whose accessible name or text is `name` (the first one made).
      def node(name : String) : UInt32
        quoted = name.inspect
        ops.each do |line|
          parts = line.split(' ', 4)
          if parts[0] == "SET_A11Y" && line.includes?(" #{quoted} ")
            return parts[1].to_u32
          elsif parts[0] == "SET_TEXT" && parts[2, 2].join(' ') == quoted
            return parts[1].to_u32
          end
        end
        raise ArgumentError.new("No part is named #{quoted}.")
      end

      # The ops of the patch sent last.
      def last_ops : Array(String)
        patch = @patches.last? || return [] of String
        Patch.describe(patch).reject(&.starts_with?("PATCH "))
      end
    end

    class Recorder < Backend
      getter windows = [] of RecordedWindow
      @next_id = 1_u32

      # `keep: false` drops the bytes of each patch (allocation specs measure the app, not the recorder).
      def initialize(@keep : Bool = true)
      end

      def open_window(title : String, width : Int32, height : Int32, limits : {Int32, Int32, Int32, Int32}?) : WindowBackend
        window = RecordedWindow.new(@next_id, title, width, height, @keep)
        @next_id += 1
        @windows << window
        window
      end
    end

    # Mount `app` against a recorder and return the recorder. Everything the app's view wrote has
    # been sent when this returns. Call `Wolkit::Test.reset` afterwards.
    def self.record(app : App, keep : Bool = true) : Recorder
      recorder = Recorder.new(keep)
      Runtime.backend = recorder
      Frame.flush_all
      app.start
      Frame.flush_all
      recorder
    end

    # An event record as the engine would make it, for a handler to receive.
    def self.event(kind : UInt16, window : UInt32, node : UInt32, seq : UInt32 = 0_u32, payload : Bytes = Bytes.empty) : Event
      Event.new(kind, 0_u16, window, node, seq, payload)
    end

    # The payload of a text event: caret, anchor, `u32 length`, bytes.
    def self.text_payload(text : String) : Bytes
      io = IO::Memory.new
      io.write_bytes(text.bytesize.to_u32, IO::ByteFormat::LittleEndian)
      io.write_bytes(text.bytesize.to_u32, IO::ByteFormat::LittleEndian)
      io.write_bytes(text.bytesize.to_u32, IO::ByteFormat::LittleEndian)
      io.write(text.to_slice)
      io.to_slice
    end

    # Click the part named `name` in `window`, then send what changed.
    def self.click(window : RecordedWindow, name : String) : Nil
      Runtime.dispatch(event(Wire::Event::CLICK, window.id, window.node(name)))
      settle
    end

    # Type `text` into the text field named `name` (the whole new content, as the engine reports it).
    def self.type(window : RecordedWindow, name : String, text : String) : Nil
      node = window.node(name)
      Runtime.dispatch(event(Wire::Event::TEXT_CHANGED, window.id, node, window.patches.size.to_u32, text_payload(text)))
      settle
    end

    # Forget the windows and the backend.
    def self.reset : Nil
      Runtime.reset
      Runtime.backend = nil
      Scope.reset_root
    end

    # Send what changed since the last flush (the loop does this each turn).
    def self.settle : Nil
      Frame.flush_all
    end
  end
end

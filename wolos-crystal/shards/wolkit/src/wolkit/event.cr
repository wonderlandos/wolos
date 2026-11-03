# SPDX-License-Identifier: BSD-2-Clause
# Event: one record of the engine's event stream (event record v1, crates/wolkit/src/event/mod.rs):
# `u32 len | u16 kind | u16 flags | u32 window | u32 node | u32 seq | payload`, 8-byte aligned,
# little-endian. `EventReader` splits a buffer filled by `wolkit_engine_read_events` into events
# without copying; an Event's payload is a view into that buffer and is valid only until the next
# drain, so a handler that wants the text keeps a String (`event.text`), not the event.
require "./patch/wire"

module Wolkit
  struct Event
    # Bytes before the payload.
    HEADER = 20
    # `flags`: events were dropped just before this record.
    FLAG_OVERFLOW = 1_u16

    getter kind : UInt16
    getter flags : UInt16
    getter window : UInt32
    getter node : UInt32
    # The sequence number of the last patch the engine had applied when it made the event.
    getter seq : UInt32
    getter payload : Bytes

    def initialize(@kind : UInt16, @flags : UInt16, @window : UInt32, @node : UInt32, @seq : UInt32, @payload : Bytes)
    end

    def overflow? : Bool
      @flags & FLAG_OVERFLOW != 0
    end

    # The `BIND_EVENT` mask bit of an event kind.
    def self.bit(kind : UInt16) : UInt32
      1_u32 << (kind - 1)
    end

    # A u32 of the payload at byte `at` (0 when the payload is shorter).
    def u32(at : Int32) : UInt32
      return 0_u32 if at + 4 > @payload.size
      IO::ByteFormat::LittleEndian.decode(UInt32, @payload[at, 4])
    end

    def i32(at : Int32) : Int32
      u32(at).to_i32!
    end

    def f32(at : Int32) : Float32
      return 0.0_f32 if at + 4 > @payload.size
      IO::ByteFormat::LittleEndian.decode(Float32, @payload[at, 4])
    end

    # A string stored as `u32 length` then the bytes, at byte `at` of the payload.
    def str(at : Int32) : String
      len = u32(at).to_i
      return "" if len <= 0 || at + 4 + len > @payload.size
      String.new(@payload[at + 4, len])
    end
  end

  # Splits the buffer `wolkit_engine_read_events` filled into events.
  class EventReader
    def initialize(@buffer : Bytes, @length : Int32)
      @at = 0
    end

    # Each whole record in order.
    def each(& : Event ->) : Nil
      while @at + Event::HEADER <= @length
        len = le32(@at).to_i
        break if len < Event::HEADER || @at + len > @length
        kind = le16(@at + 4)
        flags = le16(@at + 6)
        payload = @buffer[@at + Event::HEADER, len - Event::HEADER]
        yield Event.new(kind, flags, le32(@at + 8), le32(@at + 12), le32(@at + 16), payload)
        @at += len
      end
    end

    private def le16(at : Int32) : UInt16
      IO::ByteFormat::LittleEndian.decode(UInt16, @buffer[at, 2])
    end

    private def le32(at : Int32) : UInt32
      IO::ByteFormat::LittleEndian.decode(UInt32, @buffer[at, 4])
    end
  end
end

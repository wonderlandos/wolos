# SPDX-License-Identifier: BSD-2-Clause
# FilePanel: the system's open or save panel as a view element. While `open` is true the panel is
# up (it runs on a background thread, the window stays live); the chosen file goes to `on_file`, a
# refusal ("Allow this app to ... in System Settings") is shown under the button that opened it, and
# a cancel does nothing. `open` turns false again when the panel closes.
#
# Concept: the kit does not link libwol (pure specs build without it), so the panel is reached
# through `FilePanel.runner`; a program that requires "wolkit" gets `Wolkit::Services` there (the
# `finished` hook below), and specs put their own runner in.
require "../component"

module Wolkit
  module Kit
    # The file the person chose: its name and the open descriptor (read it, or write it for a save).
    record PickedFile, name : String, io : IO::FileDescriptor

    class FilePanel < Component
      # Runs a panel: (save as name or nil to open, title, done(file or nil, notice or nil)).
      alias Runner = Proc(String?, String, Proc(PickedFile?, String?, Nil), Nil)

      @@runner : Runner? = nil

      def self.runner=(runner : Runner?) : Runner?
        @@runner = runner
      end

      def self.runner : Runner?
        @@runner
      end

      prop open : Signal(Bool)
      prop on_file : Proc(PickedFile, Nil)
      prop save_as : String? = nil
      prop title : String = ""

      @notice = Wolkit::Signal(String).new("")

      getter notice

      def view
        notice = @notice
        open = @open
        text notice, role: :caption, color: :danger, visibility: (notice.value.empty? ? :hidden : :visible),
          a11y_role: :alert, label: notice.value
        save_as = @save_as
        title = @title
        on_file = @on_file
        Wolkit.effect do
          next unless open.value
          runner = FilePanel.runner
          unless runner
            Wolkit.batch do
              notice.value = "File panels need the system services; build the app with require \"wolkit\"."
              open.value = false
            end
            next
          end
          Wolkit.untracked { notice.value = "" }
          runner.call(save_as, title, ->(file : PickedFile?, refusal : String?) do
            Wolkit.batch do
              open.value = false
              notice.value = refusal || ""
            end
            file.try { |f| on_file.call(f) }
            nil
          end)
        end
      end
    end

    macro finished
      {% if ::Wolkit.has_constant?("Services") %}
        ::Wolkit::Kit::FilePanel.runner ||= ->(save_as : ::String?, title : ::String, done : ::Proc(::Wolkit::Kit::PickedFile?, ::String?, Nil)) do
          handle = ->(outcome : ::Wolkit::Outcome(::Wol::OpenFile)) do
            file = outcome.value.try { |f| ::Wolkit::Kit::PickedFile.new(f.name, f.to_io) }
            done.call(file, outcome.notice)
            nil
          end
          if name = save_as
            ::Wolkit::Services.save_file(name, title) { |outcome| handle.call(outcome) }
          else
            ::Wolkit::Services.open_file(title) { |outcome| handle.call(outcome) }
          end
          nil
        end
      {% end %}
    end
  end
end

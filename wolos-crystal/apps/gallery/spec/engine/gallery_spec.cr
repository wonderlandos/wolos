# SPDX-License-Identifier: BSD-2-Clause
# Every state of the gallery through the headless engine, with golden screenshots and accessibility
# trees, and the keyboard map driven with key events.
require "./engine_helper_spec"

include GallerySpec

# The name of the picture the viewer shows, or "".
def name_of(app : Gallery::App) : String
  app.photo.try(&.name) || ""
end

describe "the gallery" do
  it "says it is reading while the library loads" do
    GallerySpec.run(nil) do |t, _app|
      t.text("Reading your pictures…").should eq "Reading your pictures…"
      GallerySpec.pin(t, "loading")
    end
  end

  it "shows the empty state with a way to read again" do
    GallerySpec.run(Gallery::ScanResult.new([] of Gallery::Photo, 0, false, "/home/ada/Pictures")) do |t, app|
      t.text("No pictures yet").should eq "No pictures yet"
      GallerySpec.pin(t, "empty")
      t.click "Read again"
      app.phase.value.empty?.should be_true
    end
  end

  it "says when there is no Pictures folder" do
    GallerySpec.run(Gallery::ScanResult.new([] of Gallery::Photo, 0, false, "/x", missing: true)) do |t, _app|
      t.text("There is no Pictures folder").should eq "There is no Pictures folder"
      GallerySpec.pin(t, "missing")
    end
  end

  it "says what went wrong, in one sentence" do
    error = Gallery::ScanError.new("The gallery may not read /home/ada/Pictures. Allow it in System Settings, or choose another folder.")
    GallerySpec.run(error) do |t, app|
      t.text("The library could not be read").should eq "The library could not be read"
      app.error.value.should contain("Allow it in System Settings")
      GallerySpec.pin(t, "error")
    end
  end

  it "groups the library by date" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      app.sections.map(&.title).should eq ["Today", "Yesterday", "Saturday", "September 2026", "August 2026"]
      t.find("IMG_0412.jpg, 3 November 2026")
      t.text("9 pictures").should eq "9 pictures"
      GallerySpec.pin(t, "library-date")
    end
  end

  it "groups the library by place" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.click "Place"
      app.sections.map(&.title).should eq ["41.2° N, 8.6° W", "38.7° N, 9.1° W", "No place"]
      GallerySpec.pin(t, "library-place")
    end
  end

  it "fits a narrow window" do
    GallerySpec.run(GallerySpec.ready, {480, 360}) do |t, _app|
      GallerySpec.pin(t, "library-narrow")
    end
  end

  it "follows dark mode" do
    GallerySpec.run(GallerySpec.ready) do |t, _app|
      t.engine.set_environment(true, 0x7c5cff_u32)
      t.settle
      GallerySpec.pin(t, "library-dark")
    end
  end

  it "moves the selection with the arrow keys and opens with Return" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      app.selected.value.should eq 0
      t.press Gallery::Keys::RIGHT
      t.press Gallery::Keys::RIGHT
      app.selected.value.should eq 2
      t.press Gallery::Keys::DOWN
      app.selected.value.should eq 2 + app.columns
      t.press Gallery::Keys::HOME
      app.selected.value.should eq 0
      t.press Gallery::Keys::END_
      app.selected.value.should eq 8
      t.press Gallery::Keys::LEFT
      t.press Gallery::Keys::RETURN
      app.screen.value.viewer?.should be_true
      name_of(app).should eq "Screenshot.png"
    end
  end

  it "opens a picture with a click and shows its place in the list" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.click "IMG_0377.jpg, 31 October 2026"
      app.screen.value.viewer?.should be_true
      t.text("4 of 9").should eq "4 of 9"
      app.zoom.value.label.should eq "Fit"
      GallerySpec.pin(t, "viewer")
    end
  end

  it "steps through the pictures and back to the library" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.press Gallery::Keys::RETURN
      t.press Gallery::Keys::RIGHT
      name_of(app).should eq "IMG_0411.jpg"
      t.press Gallery::Keys::PAGE_DOWN
      t.press Gallery::Keys::END_
      name_of(app).should eq "IMG_0102.jpg"
      t.press Gallery::Keys::RIGHT
      name_of(app).should eq "IMG_0102.jpg"
      t.click "IMG_0201.jpg"
      name_of(app).should eq "IMG_0201.jpg"
      t.press Gallery::Keys::ESCAPE
      app.screen.value.library?.should be_true
      app.selected.value.should eq 6
    end
  end

  it "zooms with the keys and pans when the picture is larger than the stage" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.press Gallery::Keys::RETURN
      t.press Gallery::Keys::ONE
      app.zoom.value.label.should eq "100 %"
      before = app.zoom.value.offset
      t.press Gallery::Keys::RIGHT
      app.zoom.value.offset.should_not eq before
      name_of(app).should eq "IMG_0412.jpg"
      GallerySpec.pin(t, "viewer-zoomed")
      t.press Gallery::Keys::PLUS
      app.zoom.value.label.should eq "200 %"
      t.press Gallery::Keys::ZERO
      app.zoom.value.label.should eq "Fit"
      t.click "Zoom in"
      app.zoom.value.fitted?.should be_false
      t.click "Zoom out"
      t.click "Zoom out"
      app.zoom.value.label.should eq "Fit"
    end
  end

  it "shows the quiet info panel" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.click "IMG_0377.jpg, 31 October 2026"
      t.press Gallery::Keys::I
      app.info.value.should be_true
      t.text("Saturday 31 October 2026 at 18:04").should eq "Saturday 31 October 2026 at 18:04"
      t.text("Wolcam One").should eq "Wolcam One"
      t.text("38.7° N, 9.1° W").should eq "38.7° N, 9.1° W"
      GallerySpec.pin(t, "viewer-info")
      t.click "Info"
      app.info.value.should be_false
    end
  end

  it "plays a slideshow that goes round" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.press Gallery::Keys::END_
      t.press Gallery::Keys::RETURN
      t.press Gallery::Keys::SPACE
      app.playing.value.should be_true
      t.find("Pause slideshow")
      app.advance_slide
      name_of(app).should eq "IMG_0412.jpg"
      GallerySpec.pin(t, "slideshow")
      t.press Gallery::Keys::ESCAPE
      app.playing.value.should be_false
      app.screen.value.viewer?.should be_true
    end
  end

  it "moves a picture to the Trash and brings it back with Undo" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.press Gallery::Keys::RETURN
      t.press Gallery::Keys::DELETE
      GallerySpec::App.system.trashed.should eq ["/home/ada/Pictures/IMG_0412.jpg"]
      app.photos.value.size.should eq 8
      name_of(app).should eq "IMG_0411.jpg"
      app.notice.value.should eq "“IMG_0412.jpg” is in the Trash."
      t.find("“IMG_0412.jpg” is in the Trash.")
      GallerySpec.pin(t, "trashed")
      t.click "Undo"
      GallerySpec::App.system.restored.should eq ["/home/ada/Pictures/IMG_0412.jpg"]
      app.photos.value.size.should eq 9
      app.notice.value.should eq "“IMG_0412.jpg” is back."
    end
  end

  it "shows the system's sentence when the Trash refuses" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      GallerySpec::App.system.refuse = "This picture is on another drive or read-only, so it cannot go to the Trash."
      t.press Gallery::Keys::BACKSPACE
      app.photos.value.size.should eq 9
      app.notice.value.should contain("another drive")
      t.click "Dismiss"
      app.notice.value.should eq ""
    end
  end

  it "empties the library when the last picture goes" do
    one = Gallery::ScanResult.new([GallerySpec.photo("a.jpg", GallerySpec::NOW)], 0, false, "/p")
    GallerySpec.run(one) do |t, app|
      t.press Gallery::Keys::RETURN
      t.press Gallery::Keys::DELETE
      app.phase.value.empty?.should be_true
      app.screen.value.library?.should be_true
      t.text("No pictures yet").should eq "No pictures yet"
    end
  end

  it "shares through the system and says Open with is not there yet" do
    GallerySpec.run(GallerySpec.ready) do |t, app|
      t.press Gallery::Keys::RETURN
      t.click "Share"
      GallerySpec::App.system.shared.should eq ["/home/ada/Pictures/IMG_0412.jpg"]
      app.open_with
      app.notice.value.should eq Gallery::OPEN_WITH_MISSING
    end
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# The doctor: each check of the session gives the sentence that says what to do, and the check of an
# app's view finds the usual reasons a window does not show.
require "./spec_helper"
require "../src/wolkit/dev/doctor"

class NoWindowApp < Wolkit::App
  def view
  end
end

class EmptyWindowApp < Wolkit::App
  def view
    window "Empty" do
    end
  end
end

class DeepApp < Wolkit::App
  def view
    window "Deep" do
      column { column { column { column { column { column { column { column { column { column { column { column { column { text "deep" } } } } } } } } } } } } }
    end
  end
end

class BigApp < Wolkit::App
  def view
    window "Big" do
      column do
        each((1..250).to_a) { |i| text "row #{i}" }
      end
    end
  end
end

class RaisingApp < Wolkit::App
  def view
    raise "no data folder"
  end
end

def session(extra = {} of String => String) : Hash(String, String)
  {"XDG_RUNTIME_DIR" => "/run/user/1000", "WAYLAND_DISPLAY" => "wayland-1"}.merge(extra)
end

EXISTING = Set{"/run/user/1000", "/run/user/1000/wayland-1", "/usr/lib/libwolkit.so.1", "/usr/lib/libwol.so.1"}

def check(env, present = EXISTING, mode = 0o700)
  Wolkit::Doctor.environment(env, ->(p : String) { present.includes?(p) }, ->(_p : String) { mode.as(Int32?) })
end

describe Wolkit::Doctor do
  it "passes a working session" do
    found = check(session)
    Wolkit::Doctor.healthy?(found).should be_true
    Wolkit::Doctor.report(found).should contain("ok    display: Wayland display wayland-1")
  end

  it "says what to do when there is no runtime folder or display" do
    found = check({} of String => String)
    Wolkit::Doctor.healthy?(found).should be_false
    report = Wolkit::Doctor.report(found)
    report.should contain("FAIL  runtime folder: XDG_RUNTIME_DIR is not set")
    report.should contain("Neither WAYLAND_DISPLAY nor WOL_KIT_SOCKET is set")
    report.should contain("use Wolkit::Test.run")
  end

  it "finds a dead compositor, a missing Kit host, headless mode and open permissions" do
    Wolkit::Doctor.report(check(session, Set{"/run/user/1000", "/usr/lib/libwolkit.so.1", "/usr/lib/libwol.so.1"})).should contain("/run/user/1000/wayland-1 does not exist")
    Wolkit::Doctor.report(check(session({"WOL_KIT_SOCKET" => "/run/user/1000/kit.sock"}))).should contain("the shell is not running")
    Wolkit::Doctor.report(check(session({"WOLKIT_ENGINE_HEADLESS" => "1"}))).should contain("never shows a window")
    Wolkit::Doctor.report(check(session, EXISTING, 0o755)).should contain("warn  runtime folder")
  end

  it "looks for the libraries where the loader does" do
    report = Wolkit::Doctor.report(check(session({"WOLKIT_BUILD" => "/home/me/build"}), Set{"/run/user/1000", "/run/user/1000/wayland-1"}))
    report.should contain("FAIL  libwolkit.so.1: libwolkit.so.1 is not in /home/me/build, /usr/lib")
    Wolkit::Doctor.library_dirs({"LD_LIBRARY_PATH" => "/a:/b"}).first(2).should eq ["/a", "/b"]
  end

  it "checks what the app's view made" do
    Wolkit::Doctor.report(Wolkit::Doctor.check_app(NoWindowApp.new)).should contain("The view made no window")
    Wolkit::Doctor.report(Wolkit::Doctor.check_app(EmptyWindowApp.new)).should contain("The window is empty")
    Wolkit::Doctor.report(Wolkit::Doctor.check_app(DeepApp.new)).should contain("levels of nesting")
    Wolkit::Doctor.report(Wolkit::Doctor.check_app(BigApp.new)).should contain("Use `table` or `vlist`")
    Wolkit::Doctor.report(Wolkit::Doctor.check_app(RaisingApp.new)).should contain("The view raised Exception: no data folder")
    Wolkit::Doctor.report(Wolkit::Doctor.check_app(SpecDoctorTodos.new)).should contain("ok    window \"Todos\"")
  end
end

class SpecDoctorTodos < Wolkit::App
  @draft = signal("")

  def view
    window "Todos" do
      column { text_field @draft, placeholder: "What needs doing?" }
    end
  end
end

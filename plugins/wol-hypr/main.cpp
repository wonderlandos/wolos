// SPDX-License-Identifier: GPL-2.0-or-later
// wol-hypr: thin compositor plugin. All policy lives in Rust (crates/realtime, C ABI wol_realtime.h).
#include <filesystem>
#include <hyprland/src/plugins/PluginAPI.hpp>
// Targets the compositor 0.57 (window facts behind backend(), metadata() and presentation()).
#include <hyprland/src/desktop/view/window/Window.hpp>
#include <hyprland/src/desktop/view/window/WaylandBackend.hpp>
#include <hyprland/src/desktop/view/window/X11Backend.hpp>
#include <hyprland/src/config/shared/actions/ConfigActions.hpp>
#include <hyprland/src/ipc/s1/S1.hpp>
#include <hyprland/src/render/Context.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/Compositor.hpp>
// Built with -fno-access-control: the decoration map and toplevel resource are private.
#include <hyprland/src/protocols/XDGDecoration.hpp>
#include <hyprland/src/protocols/XDGShell.hpp>
#include <hyprland/src/layout/LayoutManager.hpp>
#include <hyprland/src/layout/target/Target.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/xwayland/XSurface.hpp>
#include <hyprland/src/desktop/state/FocusState.hpp>
#include <hyprland/src/desktop/state/WindowState.hpp>
#include <hyprland/src/managers/fullscreen/FullscreenController.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopManager.hpp>
#include <hyprland/src/desktop/view/LayerSurface.hpp>
#include <hyprland/src/managers/SessionLockManager.hpp>
#include <hyprland/src/managers/input/InputManager.hpp>
#include <hyprland/src/state/MonitorState.hpp>
#include <hyprland/src/layout/supplementary/DragController.hpp>
#include <hyprland/src/config/values/types/BoolValue.hpp>
#include <hyprland/src/managers/eventLoop/EventLoopTimer.hpp>
#include <hyprland/src/desktop/state/ViewState.hpp>
#include <algorithm>
#include <array>
#include <condition_variable>
#include <cstdlib>
#include <cstring>
#include <format>
#include <functional>
#include <linux/input-event-codes.h>
#include <mutex>
#include <pthread.h>
extern "C" {
#include <lauxlib.h>
#include <lua.h>
}
#include <sstream>
#include <string>
#include <sys/socket.h>
#include <sys/un.h>
#include <system_error>
#include <thread>
#include <time.h>
#include <unistd.h>
#include <unordered_map>
#include <utility>
#include "../../crates/realtime/include/wol_realtime.h"

inline HANDLE PHANDLE = nullptr;
// Set by PLUGIN_INIT, run by PLUGIN_EXIT: releases the event listeners.
static std::function<void()> g_unhook;
static SP<IPC::Socket1::SCommand> g_wolstats;
static SP<IPC::Socket1::SCommand> g_wolsnap;

APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

// The window's xdg toplevel (Wayland) or Xwayland surface (X11); the backends keep them private
// (built with -fno-access-control).
static SP<CXDGToplevelResource> toplevelOf(PHLWINDOW w) {
    if (w->backend().isX11())
        return nullptr;
    const auto xdg = static_cast<const Desktop::View::CWaylandBackend&>(w->backend()).m_resource.lock();
    return xdg ? xdg->m_toplevel.lock() : nullptr;
}

static SP<CXWaylandSurface> xsurfaceOf(PHLWINDOW w) {
    if (!w->backend().isX11())
        return nullptr;
    return static_cast<const Desktop::View::CX11Backend&>(w->backend()).m_xwaylandSurface.lock();
}

static bool noBorders(PHLWINDOW w) {
    return w->backend().traits().suggestsNoBorder;
}

// What the window told us about its frame, as wol_window_lights reads it (crates/realtime chrome.rs).
// Kit windows are GTK with no decorations and may carry any app id (hosted apps): the program tells.
static bool isWolProgram(PHLWINDOW w) {
    std::error_code ec;
    const auto      exe = std::filesystem::read_symlink(std::format("/proc/{}/exe", w->backend().pid()), ec);
    return !ec && exe.filename().string().starts_with("wol-");
}

static int32_t decorationOf(PHLWINDOW w) {
    if (w->backend().isX11())
        return noBorders(w) ? 4 : 3;
    const auto top = toplevelOf(w);
    if (!top)
        return 1;
    // GTK 3/4 and libadwaita never create an xdg-decoration object.
    const auto it = PROTO::xdgDecoration->m_decorations.find(top->m_resource->resource());
    if (it == PROTO::xdgDecoration->m_decorations.end())
        return isWolProgram(w) ? 5 : 0;
    return it->second->mostRecentlyRequested == ZXDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE ? 2 : 1;
}

static bool isTransient(PHLWINDOW w) {
    if (w->backend().isX11()) {
        const auto xs = xsurfaceOf(w);
        return xs && (xs->m_parent || xs->m_transient || w->backend().traits().modal);
    }
    const auto top = toplevelOf(w);
    return top && top->m_parent;
}

static uint32_t fixedAxesOf(PHLWINDOW w);

// Close-only lights when either axis is fixed (palettes, fixed-size dialogs).
static int32_t lightsOf(PHLWINDOW w) {
    return wol_window_lights(w->metadata().appID().c_str(), decorationOf(w), isTransient(w), fixedAxesOf(w) != 0);
}

// X11 override-redirect menus, tooltips and combo lists, and borderless X11 popups, are not windows
// to place, cascade from, refit or tag (audit4 S5-2, audit-x11 X1-1): they position themselves at
// the pointer. The rule is realtime's (wm_gates.rs).
static bool x11OverrideRedirect(PHLWINDOW w) {
    return w->backend().isX11() && w->backend().traits().overrideRedirect;
}

static bool managedWindow(PHLWINDOW w) {
    return wol_x11_managed(w->backend().isX11(), x11OverrideRedirect(w), noBorders(w)) != 0;
}

static int32_t placementOf(PHLWINDOW w) {
    const auto  xs = xsurfaceOf(w);
    const auto* h  = xs ? xs->m_sizeHints.get() : nullptr;
    return wol_x11_placement(w->backend().isX11(), x11OverrideRedirect(w), noBorders(w), h ? h->flags : 0);
}

// `wol-lights-*` tags (hyprland.conf windowrules): no strip on windows that draw their own buttons,
// close only on fixed-size dialogs. Popups get no tag (one hyprctl call per tooltip was waste).
static void onOpen(PHLWINDOW w) {
    if (!w || !managedWindow(w)) return;
    const auto lights = lightsOf(w);
    const char* tag   = lights == 0 ? "wol-lights-none" : lights == 1 ? "wol-lights-close" : nullptr;
    if (tag)
        (void)Config::Actions::tag(std::format("+{}", tag), w);
}

// A client-supplied size as int32: NaN, negative and huge values (a double past INT32_MAX is UB
// to cast) become 0 (unset) or the cap; realtime clamps again (placement.rs MAX_COORD).
static int32_t clientDim(double v) {
    constexpr double CAP = 1 << 20;
    if (!(v > 0.0))
        return 0;
    return v >= CAP ? (int32_t)CAP : (int32_t)v;
}

static int32_t clientDim(int v) {
    return clientDim((double)v);
}

// xdg min/max size of the toplevel (X11: WM_NORMAL_HINTS); 0 = unset, as wol_fit reads it.
static void sizeLimits(PHLWINDOW w, int32_t& minW, int32_t& minH, int32_t& maxW, int32_t& maxH) {
    minW = minH = maxW = maxH = 0;
    if (w->backend().isX11()) {
        const auto xs = xsurfaceOf(w);
        const auto h  = xs ? xs->m_sizeHints.get() : nullptr;
        if (!h)
            return;
        minW = clientDim(h->min_width), minH = clientDim(h->min_height);
        maxW = clientDim(h->max_width), maxH = clientDim(h->max_height);
        return;
    }
    const auto top = toplevelOf(w);
    if (!top)
        return;
    const auto& s = top->m_current;
    minW = clientDim(s.minSize.x), minH = clientDim(s.minSize.y);
    maxW = clientDim(s.maxSize.x), maxH = clientDim(s.maxSize.y);
}

static uint32_t fixedAxesOf(PHLWINDOW w) {
    int32_t minW, minH, maxW, maxH;
    sizeLimits(w, minW, minH, maxW, maxH);
    return wol_fixed_axes(minW, minH, maxW, maxH);
}

// Limits for wol_fit/wol_refit: the client's, with a fixed axis pinned to the current size (audit4
// S5-11: a fixed-width tall window may still be cut to the band vertically).
static void placementLimits(PHLWINDOW w, int32_t curW, int32_t curH, int32_t& minW, int32_t& minH, int32_t& maxW, int32_t& maxH) {
    sizeLimits(w, minW, minH, maxW, maxH);
    (void)wol_placement_limits(curW, curH, &minW, &minH, &maxW, &maxH); // non-null: never -1
}

// The window a transient belongs to (xdg parent toplevel, X11 WM_TRANSIENT_FOR), if it is open.
static PHLWINDOW parentOf(PHLWINDOW w) {
    return w->backend().parent(); // mapped or null, both backends
}

// Topmost other floating window of the same app on the same desktop: the cascade origin.
// windowState()->windows() is z-ordered bottom to top, so the last match is the frontmost.
static PHLWINDOW previousOfClass(PHLWINDOW w) {
    PHLWINDOW best;
    for (const auto& o : Desktop::windowState()->windows()) {
        if (o == w || !o->mapped() || !o->isFloating() || o->metadata().appID() != w->metadata().appID() || o->m_workspace != w->m_workspace || !managedWindow(o))
            continue;
        if (Fullscreen::controller()->isFullscreen(o))
            continue;
        best = o;
    }
    return best;
}

// After mapWindow's layout recalc (window.open is too early: the layout re-centres the window).
// WolOS placement rules (realtime placement.rs): a new window keeps the size it asked for, cut to the usable
// area; a dialog opens centred on its parent; the next window of an app cascades from the previous
// one; a fixed-size window is never resized. The conf no longer forces a size (the rule hit
// dialogs and fixed-size windows, which then flashed at the wrong size).
static bool refitWindow(PHLWINDOW w, const CBox& zone);

// The box is final: no `windowsMove` glide from the layout's centred spot while the popin runs
// (audit4 S5-4; the compositor's X11 configure path warps the same way).
static void setGeomNow(PHLWINDOW w, const CBox& box) {
    const auto target = w->layoutTarget();
    if (!target)
        return;
    g_layoutManager->setTargetGeom(box, target);
    target->warpPositionSize();
}

static void onOpenLate(PHLWINDOW w) {
    if (!w || !w->isFloating() || !w->layoutTarget()) return;
    if (Fullscreen::controller()->isFullscreen(w)) return;
    auto mon = w->m_monitor.lock();
    if (!mon) return;
    const auto placement = placementOf(w);
    if (placement == WOL_PLACE_SKIP)
        return;
    const auto zone = mon->logicalBoxMinusReserved(); // below the menu bar, above the dock
    if (placement == WOL_PLACE_CLAMP_ONLY) {
        // An X11 window that asked for its position keeps it, only pulled back into the zone.
        if (refitWindow(w, zone))
            w->layoutTarget()->warpPositionSize();
        return;
    }
    const int32_t bar = lightsOf(w) != 0 ? wol_titlebar_height() : 0;
    const auto    sz  = w->m_realSize->goal();
    int32_t       fw = (int32_t)sz.x, fh = (int32_t)sz.y;
    int32_t       minW, minH, maxW, maxH;
    placementLimits(w, fw, fh, minW, minH, maxW, maxH);
    wol_fit((int32_t)sz.x, (int32_t)sz.y, minW, minH, maxW, maxH, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h, bar, &fw, &fh);
    int32_t x = 0, y = 0;
    if (const auto parent = isTransient(w) ? parentOf(w) : nullptr) {
        const auto pp = parent->m_realPosition->goal(), ps = parent->m_realSize->goal();
        wol_centre(fw, fh, (int32_t)pp.x, (int32_t)pp.y, (int32_t)ps.x, (int32_t)ps.y, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h, bar, &x, &y);
    } else {
        const auto prev = previousOfClass(w);
        const auto pp   = prev ? prev->m_realPosition->goal() : Vector2D{};
        wol_place(fw, fh, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h, bar, prev ? 1 : 0, (int32_t)pp.x, (int32_t)pp.y, &x, &y);
    }
    // Through the layout, which restores this box after zoom; moving the target directly is forgotten.
    setGeomNow(w, CBox{Vector2D(x, y), Vector2D(fw, fh)});
}

// Raise on a focus the user asked for. Not on focus that merely follows the desktop: the pointer
// (FFM), a desktop or special-workspace switch, the fallback after a window closed (the next window
// is wherever it was, WolOS leaves the order alone), activation (CWindow::activate raises a floating
// window itself) and other state changes.
static bool raisesOnFocus(Desktop::eFocusReason reason) {
    using namespace Desktop;
    switch (reason) {
        case FOCUS_REASON_UNKNOWN:
        case FOCUS_REASON_KEYBIND:
        case FOCUS_REASON_DISPATCH_FOCUSWINDOW:
        case FOCUS_REASON_CLICK:
        case FOCUS_REASON_SWITCH_TO_WINDOW_SOFT:
        case FOCUS_REASON_SWITCH_TO_WINDOW_HARD:
        case FOCUS_REASON_NEW_WINDOW: return true;
        default: return false;
    }
}

// Frontmost QoS hand-off on a helper thread (audit4 S5-1): wol_qos_foreground connects to wold and
// waits for its hello, which a stopped or wedged wold can stretch to seconds. The compositor thread
// only stores the pid and wakes the helper; a burst of focus changes coalesces to the last pid.
namespace {
    struct SQosWorker {
        std::mutex              m;
        std::condition_variable cv;
        int32_t                 pid  = 0;
        bool                    stop = false;
        std::thread             thread;
    };
}
static SQosWorker* g_qos = nullptr;

static void qosStart() {
    auto* q = new SQosWorker;
    try {
        q->thread = std::thread([q] {
            pthread_setname_np(pthread_self(), "wol-hypr-qos");
            std::unique_lock lk(q->m);
            for (;;) {
                q->cv.wait(lk, [q] { return q->stop || q->pid > 0; });
                if (q->stop)
                    return;
                const int32_t pid = std::exchange(q->pid, 0);
                lk.unlock();
                wol_qos_foreground(pid);
                lk.lock();
            }
        });
    } catch (const std::system_error&) {
        delete q; // no thread: no QoS boost, never a blocking call on the compositor thread
        return;
    }
    g_qos = q;
}

static void qosForeground(int32_t pid) {
    if (!g_qos || pid <= 0)
        return;
    {
        std::lock_guard lk(g_qos->m);
        g_qos->pid = pid;
    }
    g_qos->cv.notify_one();
}

// Unload: the helper must be gone before dlclose (its code lives in this .so). Waits for a hand-off
// in flight (bounded by realtime's own timeouts).
static void qosStop() {
    if (!g_qos)
        return;
    {
        std::lock_guard lk(g_qos->m);
        g_qos->stop = true;
    }
    g_qos->cv.notify_one();
    if (g_qos->thread.joinable())
        g_qos->thread.join();
    delete g_qos;
    g_qos = nullptr;
}

// WolOS: the active window is frontmost. focuswindow alone leaves floating windows behind. Raised
// directly once the focus change is done (audit4 S5-6): no hyprctl round trip, no simulated pointer
// motion handing the focus back to a tiled window under the pointer, no reordering inside
// rawWindowFocus.
static void raiseLater(PHLWINDOW w) {
    g_pEventLoopManager->doLater([ref = PHLWINDOWREF{w}] {
        const auto w = ref.lock();
        if (!w || !w->mapped() || !w->isFloating())
            return;
        Desktop::windowState()->raise(w);
        if (const auto target = w->layoutTarget())
            g_layoutManager->bringTargetToTop(target);
    });
}

static void onActive(PHLWINDOW w, Desktop::eFocusReason reason) {
    if (!w) return;
    qosForeground((int32_t)w->backend().pid());
    if (w->isFloating() && raisesOnFocus(reason))
        raiseLater(w);
}

// One floating window pulled back into `zone` and cut to fit (realtime placement::refit). True
// when it moved.
static bool refitWindow(PHLWINDOW w, const CBox& zone) {
    const auto    pos = w->m_realPosition->goal(), sz = w->m_realSize->goal();
    const int32_t bar = lightsOf(w) != 0 ? wol_titlebar_height() : 0;
    int32_t       minW, minH, maxW, maxH;
    placementLimits(w, clientDim(sz.x), clientDim(sz.y), minW, minH, maxW, maxH);
    int32_t x = (int32_t)pos.x, y = (int32_t)pos.y, fw = (int32_t)sz.x, fh = (int32_t)sz.y;
    if (!wol_refit(minW, minH, maxW, maxH, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h, bar, &x, &y, &fw, &fh))
        return false;
    g_layoutManager->setTargetGeom(CBox{Vector2D(x, y), Vector2D(fw, fh)}, w->layoutTarget());
    return true;
}

// The usable area of an output changed (VBoxClient resized the guest display, the Dock hid or
// came back, the menu bar moved): open floating windows are pulled back in and cut to fit
// (realtime placement::refit); zoomed windows get their band back from the layout.
static void refitMonitor(PHLMONITOR mon) {
    const auto zone = mon->logicalBoxMinusReserved();
    for (const auto& w : Desktop::windowState()->windows()) {
        if (!w->mapped() || !w->isFloating() || w->m_monitor.lock() != mon || !w->layoutTarget() || !managedWindow(w))
            continue;
        if (Fullscreen::controller()->isFullscreen(w))
            continue;
        refitWindow(w, zone);
    }
    g_layoutManager->recalculateMonitor(mon, Layout::CLayoutManager::RECALCULATE_MONITOR_REASON_UNKNOWN);
}

// A window left full screen (audit3 R2A-12): refitMonitor skipped it while it was fullscreen, so a
// display resize or a Dock change meanwhile left its old box under the menu bar or past the edge.
// Once the layout restored the box (doLater), the floating window is refitted like the others.
static void onFullscreen(PHLWINDOW w) {
    if (!w || Fullscreen::controller()->getFullscreenModes(w).internal != Fullscreen::FSMODE_NONE)
        return;
    g_pEventLoopManager->doLater([ref = PHLWINDOWREF{w}] {
        const auto w   = ref.lock();
        const auto mon = w ? w->m_monitor.lock() : nullptr;
        if (!w || !mon || !w->mapped() || !w->isFloating() || !w->layoutTarget() || !managedWindow(w) || Fullscreen::controller()->isFullscreen(w))
            return;
        refitWindow(w, mon->logicalBoxMinusReserved());
    });
}

// Shell layers on screen (`wol-panel-*`, the click-catcher), by surface: Esc and the Super release
// are forwarded to the shell only while one is mapped (audit3 R2A-10). A `wol` process per Esc
// in every app, games included, was a fork+exec per key press.
static std::unordered_map<uintptr_t, std::string> g_shellLayers;
static uint32_t                                   g_keys = 0; // wol_keys_step state

static bool isShellLayer(const std::string& ns) {
    return ns.starts_with("wol-panel-") || ns == "wol-catcher";
}

static void onLayerOpened(PHLLS l) {
    if (l && isShellLayer(l->m_namespace))
        g_shellLayers[(uintptr_t)l.get()] = l->m_namespace;
}

static void onLayerClosed(PHLLS l) {
    if (l)
        g_shellLayers.erase((uintptr_t)l.get());
}

static bool shellLayerMapped(const char* only) {
    return std::ranges::any_of(g_shellLayers, [only](const auto& e) { return !only || e.second == only; });
}

// A plugin (re)load while the shell is up (hyprctl plugin load, hyprpm) saw no layer.opened for
// the panels already on screen (audit4 S5-14).
static void seedShellLayers() {
    for (const auto& mon : State::monitorState()->monitors()) {
        for (const auto& level : mon->m_layerSurfaceLayers) {
            for (const auto& ref : level) {
                const auto l = ref.lock();
                if (l && l->m_mapped && isShellLayer(l->m_namespace))
                    g_shellLayers[(uintptr_t)l.get()] = l->m_namespace;
            }
        }
    }
}

// One line to `$XDG_RUNTIME_DIR/wol-shell.sock` (shell::protocol), from the compositor: no fork,
// never blocks (a busy or absent shell loses the line, as a lost key press would). send() with
// MSG_NOSIGNAL: a shell that closed the connection first must not raise SIGPIPE in the compositor
// (audit4 S5-5); MSG_DONTWAIT: a full socket buffer drops the line instead of waiting.
static void tellShell(const char* line) {
    const char* rt = std::getenv("XDG_RUNTIME_DIR");
    if (!rt)
        return;
    sockaddr_un addr{};
    addr.sun_family = AF_UNIX;
    const int n     = std::snprintf(addr.sun_path, sizeof(addr.sun_path), "%s/wol-shell.sock", rt);
    if (n <= 0 || (size_t)n >= sizeof(addr.sun_path))
        return;
    const int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (fd < 0)
        return;
    if (connect(fd, (const sockaddr*)&addr, sizeof(addr)) == 0 && send(fd, line, std::strlen(line), MSG_NOSIGNAL | MSG_DONTWAIT) < 0) {
        // Lost: nothing to do.
    }
    close(fd);
}

// Keys are not consumed (the app still gets its Esc, the binds still run), except the Tab of ⌘Tab:
// the plugin sends `show switcher` itself so the show and the Super release travel in order over one
// socket (audit4 S5-3), and the conf's exec bind (the fallback without the plugin) does not run too.
static void onKey(const IKeyboard::SKeyEvent& e, Event::SCallbackInfo& info) {
    int32_t key = WOL_KEY_OTHER;
    switch (e.keycode) {
        case KEY_LEFTMETA:
        case KEY_RIGHTMETA: key = WOL_KEY_SUPER; break;
        case KEY_TAB: key = WOL_KEY_TAB; break;
        case KEY_ESC: key = WOL_KEY_ESC; break;
        default: return;
    }
    const uint32_t mods  = g_pInputManager ? (uint32_t)g_pInputManager->getModsFromAllKBs() : 0;
    uint32_t       flags = 0;
    if (e.state == WL_KEYBOARD_KEY_STATE_PRESSED)
        flags |= WOL_KEYF_PRESSED;
    if (mods & (uint32_t)Input::HL_MODIFIER_META)
        flags |= WOL_KEYF_SUPER_HELD;
    if (mods & (uint32_t)Input::HL_MODIFIER_SHIFT)
        flags |= WOL_KEYF_SHIFT;
    if (mods & ((uint32_t)Input::HL_MODIFIER_CTRL | (uint32_t)Input::HL_MODIFIER_ALT))
        flags |= WOL_KEYF_OTHER_MODS;
    if (g_pSessionLockManager && g_pSessionLockManager->isSessionLocked())
        flags |= WOL_KEYF_LOCKED;
    if (shellLayerMapped("wol-panel-switcher"))
        flags |= WOL_KEYF_SWITCHER_MAPPED;
    if (shellLayerMapped(nullptr))
        flags |= WOL_KEYF_SHELL_LAYER_MAPPED;
    uint32_t action = 0;
    g_keys          = wol_keys_step(g_keys, key, flags, &action);
    const int32_t line = (int32_t)(action & 0xffu);
    if (line != WOL_LINE_NONE)
        tellShell(wol_keys_line(line)); // static NUL-terminated text, never null
    if (action & WOL_ACTION_CONSUME)
        info.cancelled = true;
}

// ---------------------------------------------------------------- window snapping (realtime snap.rs)
// Drag a floating window by its bar to a side of the screen for a half, a corner for a quarter, the
// top for zoom; Super+arrows (`hyprctl wolsnap left|right|up|down`, hyprland.lua) walk the same
// places; the shell draws the drop preview (`snap-preview`) and the snap layouts chooser
// (`snap-layouts`, after a rest on the zoom button). Off with plugin:wol:edge_snap = false
// (System Settings > Desktop & Dock > Snap windows to screen edges); the keys work either way.
static SP<Config::Values::CBoolValue> g_edgeSnap;

static bool edgeSnapOn() {
    return !g_edgeSnap || g_edgeSnap->value();
}

// A window that snapped: where it went and the frame to give back (the frame before the first snap).
struct SSnapped {
    PHLWINDOWREF window;
    int32_t      zone = WOL_SNAP_NONE;
    CBox         frame;
    CBox         restore;
};
static std::vector<SSnapped> g_snapped;

static SSnapped* snappedOf(PHLWINDOW w) {
    std::erase_if(g_snapped, [](const SSnapped& s) { return s.window.expired(); });
    for (auto& s : g_snapped)
        if (s.window.lock() == w)
            return &s;
    return nullptr;
}

static int32_t barOf(PHLWINDOW w) {
    return lightsOf(w) != 0 ? wol_titlebar_height() : 0;
}

// Frame = client box with the compositor's title bar above it (placement.rs: positions are the client area).
static CBox frameOf(PHLWINDOW w) {
    const auto    pos = w->m_realPosition->goal(), sz = w->m_realSize->goal();
    const int32_t bar = barOf(w);
    return CBox{pos.x, pos.y - bar, sz.x, sz.y + bar};
}

static bool stillAt(const CBox& a, const CBox& b) {
    return wol_snap_still((int32_t)a.x, (int32_t)a.y, (int32_t)a.w, (int32_t)a.h, (int32_t)b.x, (int32_t)b.y, (int32_t)b.w, (int32_t)b.h) != 0;
}

// The zone the window is in now: its record, while nobody moved or resized it since.
static int32_t zoneOf(PHLWINDOW w) {
    const auto s = snappedOf(w);
    return s && stillAt(frameOf(w), s->frame) ? s->zone : WOL_SNAP_NONE;
}

static PHLMONITOR monitorAt(const Vector2D& p) {
    for (const auto& m : State::monitorState()->monitors())
        if (m->m_enabled && m->logicalBox().containsPoint(p))
            return m;
    return nullptr;
}

// Outer edges of an output: no other output 2 px beyond them (a shared edge is a way across).
static uint32_t openEdges(PHLMONITOR mon) {
    const auto b    = mon->logicalBox();
    const auto mid  = b.middle();
    uint32_t   open = 0;
    const std::array<std::pair<Vector2D, uint32_t>, 4> probes = {{
        {{b.x - 2, mid.y}, WOL_SNAP_OPEN_LEFT},
        {{b.x + b.w + 2, mid.y}, WOL_SNAP_OPEN_RIGHT},
        {{mid.x, b.y - 2}, WOL_SNAP_OPEN_TOP},
        {{mid.x, b.y + b.h + 2}, WOL_SNAP_OPEN_BOTTOM},
    }};
    for (const auto& [p, bit] : probes)
        if (!monitorAt(p))
            open |= bit;
    return open;
}

static std::pair<int32_t, int32_t> tilingGaps() {
    static auto PIN  = CConfigValue<Config::IComplexConfigValue>("general:gaps_in");
    static auto POUT = CConfigValue<Config::IComplexConfigValue>("general:gaps_out");
    const auto* in   = sc<Config::CCssGapData*>(PIN.ptr());
    const auto* out  = sc<Config::CCssGapData*>(POUT.ptr());
    return {out ? (int32_t)out->m_top : 0, in ? (int32_t)in->m_top : 0};
}

// The frame of `zone` on `mon` (realtime snap::zone_box), in global layout coordinates.
static std::optional<CBox> zoneFrame(PHLMONITOR mon, int32_t zone) {
    const auto area         = mon->logicalBoxMinusReserved();
    const auto [outer, inner] = tilingGaps();
    int32_t    x = 0, y = 0, w = 0, h = 0;
    if (wol_snap_box(zone, (int32_t)area.x, (int32_t)area.y, (int32_t)area.w, (int32_t)area.h, outer, inner, &x, &y, &w, &h) != 0)
        return std::nullopt;
    return CBox{(double)x, (double)y, (double)w, (double)h};
}

static void hidePreview();

// Moves a floating window into `zone` on `mon` (zoom: the compositor's maximise, like the zoom button).
static void snapTo(PHLWINDOW w, PHLMONITOR mon, int32_t zone) {
    if (!w || !mon || !w->layoutTarget() || !w->isFloating())
        return;
    const bool zoomed = Fullscreen::controller()->isFullscreen(w);
    if (zone == WOL_SNAP_ZOOM) {
        if (!zoomed)
            (void)Config::Actions::fullscreenWindow(Fullscreen::FSMODE_MAXIMIZED, true, w);
        return;
    }
    const auto frame = zoneFrame(mon, zone);
    if (!frame)
        return;
    if (zoomed)
        Fullscreen::controller()->setFullscreenMode(w, Fullscreen::FSMODE_NONE);
    auto* rec = snappedOf(w);
    if (!rec) {
        g_snapped.push_back(SSnapped{.window = w, .zone = WOL_SNAP_NONE, .frame = CBox{}, .restore = frameOf(w)});
        rec = &g_snapped.back();
    } else if (!stillAt(frameOf(w), rec->frame) && !zoomed)
        rec->restore = frameOf(w); // moved or resized since: that is the size to come back to
    rec->zone      = zone;
    rec->frame     = *frame;
    const int32_t bar = barOf(w);
    g_layoutManager->setTargetGeom(CBox{frame->x, frame->y + bar, frame->w, std::max(1.0, frame->h - bar)}, w->layoutTarget());
}

static void restoreWindow(PHLWINDOW w) {
    auto* rec = snappedOf(w);
    if (Fullscreen::controller()->isFullscreen(w)) {
        (void)Config::Actions::fullscreenWindow(Fullscreen::FSMODE_MAXIMIZED, true, w);
        return;
    }
    if (!rec || !w->layoutTarget())
        return;
    const auto    r   = rec->restore;
    const int32_t bar = barOf(w);
    std::erase_if(g_snapped, [&](const SSnapped& s) { return s.window.lock() == w; });
    g_layoutManager->setTargetGeom(CBox{r.x, r.y + bar, r.w, std::max(1.0, r.h - bar)}, w->layoutTarget());
}

static int32_t zoneFromName(const std::string& s) {
    for (int32_t z = 0; const char* n = wol_snap_zone_name(z); ++z)
        if (s == n)
            return z;
    return -1;
}

static int32_t keyFromName(const std::string& s) {
    static const std::array<const char*, 4> names = {"left", "right", "up", "down"};
    for (int32_t k = 0; k < 4; ++k)
        if (s == names[k])
            return k;
    return -1;
}

// One Super+arrow on the focused window (realtime snap::key_step).
static std::string snapKey(PHLWINDOW w, int32_t key) {
    if (!w || !w->mapped() || !managedWindow(w))
        return "no window";
    const auto mon = w->m_monitor.lock();
    if (!mon)
        return "no display";
    const bool    tiled  = !w->isFloating();
    const bool    zoomed = Fullscreen::controller()->isFullscreen(w);
    const int32_t step   = wol_snap_key(tiled ? WOL_SNAP_NONE : zoneOf(w), key, tiled, zoomed);
    if (step < 0)
        return "bad key";
    // Fixed-size windows (palettes, dialogs) never resize: they only zoom off, minimise or stay.
    const bool fixed = fixedAxesOf(w) != 0;
    switch (step & 0xff) {
        case WOL_SNAP_TO:
            if (!fixed)
                snapTo(w, mon, (step >> 8) & 0xff);
            break;
        case WOL_SNAP_RESTORE: restoreWindow(w); break;
        case WOL_SNAP_MINIMISE: (void)Config::Actions::moveToWorkspace("special:hidden", true, w); break;
        case WOL_SNAP_MOVE_TILED: {
            static const std::array<Math::eDirection, 4> dirs = {Math::DIRECTION_LEFT, Math::DIRECTION_RIGHT, Math::DIRECTION_UP, Math::DIRECTION_DOWN};
            (void)Config::Actions::moveInDirection(dirs[(step >> 8) & 3], w);
            break;
        }
        case WOL_SNAP_ZOOM_TILED: (void)Config::Actions::fullscreenWindow(Fullscreen::FSMODE_MAXIMIZED, true, w); break;
        default: break;
    }
    return "ok";
}

static PHLWINDOW windowByAddress(const std::string& arg) {
    const auto hex = arg.starts_with("address:") ? arg.substr(8) : arg;
    for (const auto& w : Desktop::windowState()->windows())
        if (w->mapped() && std::format("0x{:x}", (uintptr_t)w.get()) == hex)
            return w;
    return nullptr;
}

// `hyprctl wolsnap <left|right|up|down> [address:0x...]`: a Super+arrow; `hyprctl wolsnap zone <ZONE>
// [address:0x...]`: straight to a place (the snap layouts chooser; `left` and `right` are keys without
// `zone`, so a window in the right half asked for the left half is not restored instead). The
// focused window unless an address is given.
static IPC::Socket1::SResponse wolSnap(const IPC::Socket1::SRequest& req) {
    static const char* usage = "usage: wolsnap left|right|up|down | zone left|right|top-left|top-right|bottom-left|bottom-right|zoom|none [address:0x...]";
    std::istringstream in(req.command);
    std::string        verb, what, where;
    in >> verb >> what;
    const bool toZone = what == "zone";
    if (toZone)
        in >> what;
    in >> where;
    const int32_t key  = toZone ? -1 : keyFromName(what);
    const int32_t zone = toZone ? zoneFromName(what) : -1;
    if (key < 0 && zone < 0)
        return std::string(usage);
    const auto w = where.empty() ? Desktop::focusState()->window() : windowByAddress(where);
    if (!w)
        return std::string("no window");
    if (key >= 0)
        return snapKey(w, key);
    if (!w->isFloating() || fixedAxesOf(w) != 0)
        return std::string("not a floating window");
    if (zone == WOL_SNAP_NONE)
        restoreWindow(w);
    else
        snapTo(w, w->m_monitor.lock(), zone);
    return std::string("ok");
}

// `hl.plugin.wol.snap("left"|"right"|"up"|"down")` in hyprland.lua: the same as `hyprctl wolsnap`, no
// process. Its namespace also tells the configuration the plugin is loaded (`if hl.plugin.wol then`
// guards the plugin:wol:* keys, as hl.plugin.hyprbars does for the strip).
static int luaSnap(lua_State* L) {
    const int32_t key = keyFromName(luaL_checkstring(L, 1));
    if (key < 0)
        return luaL_error(L, "hl.plugin.wol.snap: left, right, up or down");
    (void)snapKey(Desktop::focusState()->window(), key);
    return 0;
}

// Drop preview: one line to the shell per change of zone (never per motion).
static int32_t       g_previewZone = WOL_SNAP_NONE;
static PHLMONITORREF g_previewMonitor; // the output the shown preview is on: a drag across outputs moves it
static PHLWINDOWREF  g_dragWindow;
static PHLMONITORREF g_dragMonitor;

static void hidePreview() {
    if (g_previewZone != WOL_SNAP_NONE)
        tellShell("snap-preview off\n");
    g_previewZone = WOL_SNAP_NONE;
    g_previewMonitor.reset();
}

static void showPreview(PHLMONITOR mon, int32_t zone) {
    if (zone == g_previewZone && g_previewMonitor.lock() == mon)
        return;
    const auto frame = zoneFrame(mon, zone);
    if (!frame)
        return hidePreview();
    g_previewZone    = zone;
    g_previewMonitor = mon;
    const auto origin = mon->logicalBox().pos();
    tellShell(std::format("snap-preview {} {} {} {} {} {}\n", mon->m_name, wol_snap_zone_name(zone), (int)(frame->x - origin.x), (int)(frame->y - origin.y), (int)frame->w,
                          (int)frame->h)
                  .c_str());
}

// A snapped window that is dragged away gets the size it had before it snapped, under the pointer.
static void unsnapForDrag(PHLWINDOW w, const Vector2D& p) {
    auto* rec = snappedOf(w);
    if (!rec || Fullscreen::controller()->isFullscreen(w))
        return;
    const auto cur = frameOf(w);
    if (!stillAt(cur, rec->frame)) {
        std::erase_if(g_snapped, [&](const SSnapped& s) { return s.window.lock() == w; });
        return;
    }
    const auto r = rec->restore;
    int32_t    x = 0, y = 0;
    wol_snap_unsnap((int32_t)cur.x, (int32_t)cur.y, (int32_t)cur.w, (int32_t)cur.h, (int32_t)r.w, (int32_t)r.h, (int32_t)p.x, (int32_t)p.y, &x, &y);
    std::erase_if(g_snapped, [&](const SSnapped& s) { return s.window.lock() == w; });
    const int32_t bar    = barOf(w);
    const auto    client = CBox{(double)x, (double)(y + bar), r.w, std::max(1.0, r.h - bar)};
    setGeomNow(w, client);
    // The drag goes on from the new box: the grab point moves with it (built with -fno-access-control).
    auto& dc           = g_layoutManager->dragController();
    dc->m_dragHotspot  = p - client.pos();
    dc->m_beginDragXY  = p;
    dc->m_beginDragPositionXY = client.pos();
    dc->m_beginDragSizeXY     = client.size();
}

static void onDragMotion() {
    const auto& dc     = g_layoutManager->dragController();
    const auto  target = dc->target();
    const auto  w      = target ? target->window() : nullptr;
    if (!w || dc->mode() != MBIND_MOVE || dc->draggingTiled() || !w->isFloating() || !managedWindow(w) || fixedAxesOf(w) != 0 || !edgeSnapOn()) {
        g_dragWindow.reset();
        return hidePreview();
    }
    const auto p = g_pInputManager->getMouseCoordsInternal();
    if (g_dragWindow.lock() != w) {
        g_dragWindow = w;
        unsnapForDrag(w, p);
    }
    const auto mon = monitorAt(p);
    if (!mon)
        return hidePreview();
    const auto b    = mon->logicalBox();
    const auto zone = wol_snap_zone_at((int32_t)b.x, (int32_t)b.y, (int32_t)b.w, (int32_t)b.h, openEdges(mon), (int32_t)p.x, (int32_t)p.y);
    g_dragMonitor   = mon;
    if (zone == WOL_SNAP_NONE)
        return hidePreview();
    showPreview(mon, zone);
}

static void onDragEnded() {
    const auto w    = g_dragWindow.lock();
    const auto mon  = g_dragMonitor.lock();
    const auto zone = g_previewZone;
    g_dragWindow.reset();
    hidePreview();
    if (!w || !mon || zone == WOL_SNAP_NONE)
        return;
    // After dragEnd committed the dropped box (and refocused the window).
    g_pEventLoopManager->doLater([ref = PHLWINDOWREF{w}, m = PHLMONITORREF{mon}, zone] {
        const auto w = ref.lock();
        if (w && w->mapped())
            snapTo(w, m.lock(), zone);
    });
}

// Snap layouts chooser: the pointer rests on a window's zoom button; the shell shows the six places
// next to it (`snap-layouts <output> <address> <x> <y>`, the button's centre in output coordinates)
// and sends `wolsnap zone <zone> address:...` for the one clicked.
static SP<CEventLoopTimer>                      g_hoverTimer;
static PHLWINDOWREF                             g_hoverWindow;
// lightsOf per window (reads /proc once). Keyed by address but kept with a ref: an entry whose window
// is gone is never trusted, so a new window at the same address is read afresh.
struct SLights {
    PHLWINDOWREF window;
    int32_t      lights = 0;
};
static std::unordered_map<uintptr_t, SLights> g_lights;

static int32_t cachedLights(PHLWINDOW w) {
    const auto key = (uintptr_t)w.get();
    if (const auto it = g_lights.find(key); it != g_lights.end() && it->second.window.lock() == w)
        return it->second.lights;
    const auto n = lightsOf(w);
    g_lights[key] = SLights{.window = w, .lights = n};
    return n;
}

static void hoverDisarm() {
    g_hoverWindow.reset();
    if (g_hoverTimer)
        g_hoverTimer->updateTimeout(std::nullopt);
}

static void onHoverFire() {
    const auto w = g_hoverWindow.lock();
    g_hoverWindow.reset();
    if (!w || !w->mapped() || !w->isFloating())
        return;
    const auto mon = w->m_monitor.lock();
    if (!mon)
        return;
    const auto p      = g_pInputManager->getMouseCoordsInternal();
    const auto origin = mon->logicalBox().pos();
    tellShell(std::format("snap-layouts {} 0x{:x} {} {}\n", mon->m_name, (uintptr_t)w.get(), (int)(p.x - origin.x), (int)(p.y - origin.y)).c_str());
}

// Runs on every pointer motion: one bounded hit test (the windows under the pointer, no allocation
// beyond the result) and, over a window's title bar only, one cached /proc read per window. Nothing
// else happens until the pointer rests on a zoom button for the dwell.
static void onMouseMove(const Vector2D& p) {
    if (!edgeSnapOn() || g_layoutManager->dragController()->target())
        return hoverDisarm();
    const auto w = Desktop::viewState()->hitTest().windowAt(p, Desktop::View::RESERVED_EXTENTS | Desktop::View::INPUT_EXTENTS | Desktop::View::ALLOW_FLOATING);
    if (!w || !w->isFloating() || Fullscreen::controller()->isFullscreen(w) || fixedAxesOf(w) != 0 || !managedWindow(w))
        return hoverDisarm();
    const auto lights = cachedLights(w);
    if (lights < 3)
        return hoverDisarm();
    const auto pos = w->m_realPosition->goal(), sz = w->m_realSize->goal();
    const int  bar = wol_titlebar_height();
    if (wol_lights_at_w((int32_t)sz.x, 3, (int32_t)(p.x - pos.x), (int32_t)(p.y - (pos.y - bar))) != 2)
        return hoverDisarm();
    if (g_hoverWindow.lock() == w)
        return; // still resting on the same button: the timer runs
    g_hoverWindow = w;
    if (!g_hoverTimer) {
        g_hoverTimer = makeShared<CEventLoopTimer>(std::nullopt, [](SP<CEventLoopTimer>, void*) { onHoverFire(); }, nullptr);
        g_pEventLoopManager->addTimer(g_hoverTimer);
    }
    g_hoverTimer->updateTimeout(std::chrono::milliseconds(wol_snap_layouts_dwell_ms()));
}

// A window is destroyed: drop its cache entry, whether or not its ref still locks (the entries of
// every window that is gone go, so the map cannot grow past the open windows).
static void onWindowGone(PHLWINDOWREF gone) {
    if (const auto w = gone.lock())
        g_lights.erase((uintptr_t)w.get());
    std::erase_if(g_lights, [](const auto& e) { return e.second.window.expired(); });
}

// Zone per output as last seen; a change (mode, scale, position or reserved area) refits once the
// frame is out of the renderer (doLater), not inside render.pre.
static std::unordered_map<MONITORID, std::array<double, 4>> g_zones;
static void onRenderPre(PHLMONITOR mon) {
    if (!mon) return;
    const auto                  z    = mon->logicalBoxMinusReserved();
    const std::array<double, 4> zone = {z.x, z.y, z.w, z.h};
    const auto                  it   = g_zones.find(mon->m_id);
    if (it != g_zones.end() && it->second == zone)
        return;
    const bool first   = it == g_zones.end();
    g_zones[mon->m_id] = zone;
    if (first)
        return; // first sight of this output: windows were placed for it
    g_pEventLoopManager->doLater([m = PHLMONITORREF{mon}] {
        if (const auto mon = m.lock())
            refitMonitor(mon);
    });
}

// Frame ring (ADR 0007): begin at RENDER_BEGIN (damage already final), end at RENDER_POST.
// passes = 1: a lower bound on full read+write passes over the damage until passes are counted.
static uint64_t monoNs() {
    timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec;
}
static uint64_t g_frameBegin = 0, g_frameDamage = 0, g_frameScreen = 0;
static void onRenderStage(const Event::SRenderStageEvent& e) {
    if (e.stage == RENDER_BEGIN && e.context) {
        const auto& rd = e.context->get().m_data;
        uint64_t     px = 0;
        for (const auto& r : rd.damage.getRects())
            px += (uint64_t)(r.x2 - r.x1) * (uint64_t)(r.y2 - r.y1);
        const auto mon = rd.pMonitor.lock();
        g_frameDamage  = px;
        g_frameScreen  = mon ? (uint64_t)mon->m_pixelSize.x * (uint64_t)mon->m_pixelSize.y : 0;
        g_frameBegin   = monoNs();
    } else if (e.stage == RENDER_POST && g_frameBegin) {
        wol_frame_record(g_frameBegin, monoNs(), g_frameDamage, g_frameScreen, 1);
        g_frameBegin = 0;
    }
}

static IPC::Socket1::SResponse wolStats(const IPC::Socket1::SRequest&) {
    char buf[1024]; // audit4 S5-17: a JSON past 256 bytes came back as `{}`
    return wol_frame_stats_json((uint8_t*)buf, sizeof(buf)) < 0 ? std::string("{}") : std::string(buf);
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;
    // Animations are not set here: hyprland.lua carries the motion table (crates/realtime/src/anim.rs, anim::lua()); a curve is read by name with wol_anim_curve.
    static auto open = Event::bus()->m_events.window.open.listen([](PHLWINDOW w) { onOpen(w); });
    static auto openLate = Event::bus()->m_events.window.openLate.listen([](PHLWINDOW w) { onOpenLate(w); });
    static auto active = Event::bus()->m_events.window.active.listen([](PHLWINDOW w, Desktop::eFocusReason r) { onActive(w, r); });
    static auto render = Event::bus()->m_events.render.stage.listen([](const Event::SRenderStageEvent& s) { onRenderStage(s); });
    static auto pre    = Event::bus()->m_events.render.pre.listen([](PHLMONITOR m) { onRenderPre(m); });
    static auto fs     = Event::bus()->m_events.window.fullscreen.listen([](PHLWINDOW w) { onFullscreen(w); });
    static auto lsOpen = Event::bus()->m_events.layer.opened.listen([](PHLLS l) { onLayerOpened(l); });
    static auto lsShut = Event::bus()->m_events.layer.closed.listen([](PHLLS l) { onLayerClosed(l); });
    static auto key    = Event::bus()->m_events.input.keyboard.key.listen([](IKeyboard::SKeyEvent e, Event::SCallbackInfo& info) { onKey(e, info); });
    static auto mouse  = Event::bus()->m_events.input.mouse.move.listen([](Vector2D p, Event::SCallbackInfo&) { onMouseMove(p); });
    static auto gone   = Event::bus()->m_events.window.destroy.listen([](PHLWINDOWREF w) { onWindowGone(w); });
    static auto dragMv = g_layoutManager->dragController()->m_events.motion.listen([] { onDragMotion(); });
    static auto dragEd = g_layoutManager->dragController()->m_events.ended.listen([] { onDragEnded(); });
    g_edgeSnap = makeShared<Config::Values::CBoolValue>("plugin:wol:edge_snap", "snap a dragged window to the screen edges (half, quarter, zoom)", true);
    HyprlandAPI::addConfigValueV2(PHANDLE, g_edgeSnap);
    seedShellLayers();
    qosStart();
    g_wolstats = HyprlandAPI::registerHyprCtlCommand(PHANDLE, IPC::Socket1::SCommand{.name = "wolstats", .match = IPC::Socket1::COMMAND_MATCH_EXACT, .handler = wolStats});
    HyprlandAPI::addLuaFunction(PHANDLE, "wol", "snap", luaSnap);
    g_wolsnap  = HyprlandAPI::registerHyprCtlCommand(PHANDLE, IPC::Socket1::SCommand{.name = "wolsnap", .match = IPC::Socket1::COMMAND_MATCH_PREFIX, .handler = wolSnap});
    // Unloading must drop every listener: a callback left on the bus would jump into the
    // unmapped .so on the next window event.
    g_unhook = [] {
        open     = {};
        openLate = {};
        active   = {};
        render   = {};
        pre      = {};
        fs       = {};
        lsOpen   = {};
        lsShut   = {};
        key      = {};
        mouse    = {};
        gone     = {};
        dragMv   = {};
        dragEd   = {};
        if (g_hoverTimer && g_pEventLoopManager)
            g_pEventLoopManager->removeTimer(g_hoverTimer);
        g_hoverTimer.reset();
        g_snapped.clear();
        g_lights.clear();
        g_edgeSnap.reset();
        g_shellLayers.clear();
        g_keys = 0;
        qosStop();
    };
    return {"wol-hypr", "WolOS window policy (placement, refit, snapping, QoS, frame stats, shell keys)", "WolOS", "0.13.0"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    if (g_unhook)
        g_unhook();
    g_unhook = nullptr;
    if (g_wolstats)
        HyprlandAPI::unregisterHyprCtlCommand(PHANDLE, g_wolstats);
    HyprlandAPI::removeLuaFunction(PHANDLE, "wol", "snap");
    if (g_wolsnap)
        HyprlandAPI::unregisterHyprCtlCommand(PHANDLE, g_wolsnap);
    g_wolstats = nullptr;
    g_wolsnap  = nullptr;
    PHANDLE    = nullptr;
}

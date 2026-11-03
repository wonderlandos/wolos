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
    // Close curve is sampled by the animation config generated from wol_anim_duration_ms.
    static auto open = Event::bus()->m_events.window.open.listen([](PHLWINDOW w) { onOpen(w); });
    static auto openLate = Event::bus()->m_events.window.openLate.listen([](PHLWINDOW w) { onOpenLate(w); });
    static auto active = Event::bus()->m_events.window.active.listen([](PHLWINDOW w, Desktop::eFocusReason r) { onActive(w, r); });
    static auto render = Event::bus()->m_events.render.stage.listen([](const Event::SRenderStageEvent& s) { onRenderStage(s); });
    static auto pre    = Event::bus()->m_events.render.pre.listen([](PHLMONITOR m) { onRenderPre(m); });
    static auto fs     = Event::bus()->m_events.window.fullscreen.listen([](PHLWINDOW w) { onFullscreen(w); });
    static auto lsOpen = Event::bus()->m_events.layer.opened.listen([](PHLLS l) { onLayerOpened(l); });
    static auto lsShut = Event::bus()->m_events.layer.closed.listen([](PHLLS l) { onLayerClosed(l); });
    static auto key    = Event::bus()->m_events.input.keyboard.key.listen([](IKeyboard::SKeyEvent e, Event::SCallbackInfo& info) { onKey(e, info); });
    seedShellLayers();
    qosStart();
    g_wolstats = HyprlandAPI::registerHyprCtlCommand(PHANDLE, IPC::Socket1::SCommand{.name = "wolstats", .match = IPC::Socket1::COMMAND_MATCH_EXACT, .handler = wolStats});
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
        g_shellLayers.clear();
        g_keys = 0;
        qosStop();
    };
    return {"wol-hypr", "WolOS window policy (placement, refit, QoS, frame stats, shell keys)", "WolOS", "0.12.0"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    if (g_unhook)
        g_unhook();
    g_unhook = nullptr;
    if (g_wolstats)
        HyprlandAPI::unregisterHyprCtlCommand(PHANDLE, g_wolstats);
    g_wolstats = nullptr;
    PHANDLE    = nullptr;
}

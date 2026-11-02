// SPDX-License-Identifier: GPL-2.0-or-later
// wol-hypr: thin Hyprland plugin. All policy lives in Rust (crates/realtime, C ABI wol_realtime.h).
#include <filesystem>
#include <hyprland/src/plugins/PluginAPI.hpp>
// Targets Hyprland 0.56 (Event::bus(), desktop/view/Window.hpp, positionAnimation()).
#include <hyprland/src/desktop/view/Window.hpp>
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
#include <algorithm>
#include <array>
#include <cstdlib>
#include <cstring>
#include <format>
#include <functional>
#include <linux/input-event-codes.h>
#include <string>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>
#include <unordered_map>
#include "../../crates/realtime/include/wol_realtime.h"

inline HANDLE PHANDLE = nullptr;
// Set by PLUGIN_INIT, run by PLUGIN_EXIT: releases the event listeners.
static std::function<void()> g_unhook;
static SP<SHyprCtlCommand>   g_wolstats;

APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

// What the window told us about its frame, as wol_window_lights reads it (crates/realtime chrome.rs).
// Kit windows are GTK with no decorations and may carry any app id (hosted apps): the program tells.
static bool isWolProgram(PHLWINDOW w) {
    std::error_code ec;
    const auto      exe = std::filesystem::read_symlink(std::format("/proc/{}/exe", w->getPID()), ec);
    return !ec && exe.filename().string().starts_with("wol-");
}

static int32_t decorationOf(PHLWINDOW w) {
    if (w->m_isX11)
        return w->m_X11DoesntWantBorders ? 4 : 3;
    const auto xdg = w->m_xdgSurface.lock();
    const auto top = xdg ? xdg->m_toplevel.lock() : nullptr;
    if (!top)
        return 1;
    // GTK 3/4 and libadwaita never create an xdg-decoration object.
    const auto it = PROTO::xdgDecoration->m_decorations.find(top->m_resource->resource());
    if (it == PROTO::xdgDecoration->m_decorations.end())
        return isWolProgram(w) ? 5 : 0;
    return it->second->mostRecentlyRequested == ZXDG_TOPLEVEL_DECORATION_V1_MODE_CLIENT_SIDE ? 2 : 1;
}

static bool isTransient(PHLWINDOW w) {
    if (w->m_isX11)
        return w->m_xwaylandSurface && (w->m_xwaylandSurface->m_parent || w->m_xwaylandSurface->m_transient || w->isModal());
    const auto xdg = w->m_xdgSurface.lock();
    const auto top = xdg ? xdg->m_toplevel.lock() : nullptr;
    return top && top->m_parent;
}

static bool isFixedSize(PHLWINDOW w) {
    if (w->m_isX11) {
        const auto h = w->m_xwaylandSurface ? w->m_xwaylandSurface->m_sizeHints.get() : nullptr;
        return h && h->min_width > 0 && h->min_height > 0 && (h->min_width == h->max_width || h->min_height == h->max_height);
    }
    const auto xdg = w->m_xdgSurface.lock();
    const auto top = xdg ? xdg->m_toplevel.lock() : nullptr;
    if (!top)
        return false;
    const auto& s = top->m_current;
    return s.minSize.x > 1 && s.minSize.y > 1 && (s.minSize.x == s.maxSize.x || s.minSize.y == s.maxSize.y);
}

static int32_t lightsOf(PHLWINDOW w) {
    return wol_window_lights(w->m_class.c_str(), decorationOf(w), isTransient(w), isFixedSize(w));
}

// `wol-lights-*` tags (hyprland.conf windowrules): no strip on windows that draw their own buttons,
// close only on fixed-size dialogs.
static void onOpen(PHLWINDOW w) {
    if (!w) return;
    const auto lights = lightsOf(w);
    const char* tag   = lights == 0 ? "wol-lights-none" : lights == 1 ? "wol-lights-close" : nullptr;
    if (tag)
        HyprlandAPI::invokeHyprctlCommand("dispatch", std::format("tagwindow +{} address:0x{:x}", tag, (uintptr_t)w.get()));
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
    if (w->m_isX11) {
        const auto h = w->m_xwaylandSurface ? w->m_xwaylandSurface->m_sizeHints.get() : nullptr;
        if (!h)
            return;
        minW = clientDim(h->min_width), minH = clientDim(h->min_height);
        maxW = clientDim(h->max_width), maxH = clientDim(h->max_height);
        return;
    }
    const auto xdg = w->m_xdgSurface.lock();
    const auto top = xdg ? xdg->m_toplevel.lock() : nullptr;
    if (!top)
        return;
    const auto& s = top->m_current;
    minW = clientDim(s.minSize.x), minH = clientDim(s.minSize.y);
    maxW = clientDim(s.maxSize.x), maxH = clientDim(s.maxSize.y);
}

// The window a transient belongs to (xdg parent toplevel, X11 WM_TRANSIENT_FOR), if it is open.
static PHLWINDOW parentOf(PHLWINDOW w) {
    if (w->m_isX11) {
        const auto xs     = w->m_xwaylandSurface.lock();
        const auto parent = xs ? xs->m_parent.lock() : nullptr;
        if (!parent)
            return nullptr;
        for (const auto& o : Desktop::windowState()->windows())
            if (o->m_isMapped && o->m_xwaylandSurface.lock() == parent)
                return o;
        return nullptr;
    }
    const auto xdg    = w->m_xdgSurface.lock();
    const auto top    = xdg ? xdg->m_toplevel.lock() : nullptr;
    const auto parent = top ? top->m_parent.lock() : nullptr;
    const auto pw     = parent ? parent->m_window.lock() : nullptr;
    return pw && pw->m_isMapped ? pw : nullptr;
}

// Topmost other floating window of the same app on the same desktop: the cascade origin.
// windowState()->windows() is z-ordered bottom to top, so the last match is the frontmost.
static PHLWINDOW previousOfClass(PHLWINDOW w) {
    PHLWINDOW best;
    for (const auto& o : Desktop::windowState()->windows()) {
        if (o == w || !o->m_isMapped || !o->m_isFloating || o->m_class != w->m_class || o->m_workspace != w->m_workspace)
            continue;
        if (Fullscreen::controller()->isFullscreen(o))
            continue;
        best = o;
    }
    return best;
}

// After mapWindow's layout recalc (window.open is too early: the layout re-centres the window).
// macOS rules (realtime placement.rs): a new window keeps the size it asked for, cut to the usable
// area; a dialog opens centred on its parent; the next window of an app cascades from the previous
// one; a fixed-size window is never resized. The conf no longer forces a size (the rule hit
// dialogs and fixed-size windows, which then flashed at the wrong size).
static void onOpenLate(PHLWINDOW w) {
    if (!w || !w->m_isFloating || !w->layoutTarget()) return;
    if (Fullscreen::controller()->isFullscreen(w)) return;
    auto mon = w->m_monitor.lock();
    if (!mon) return;
    const auto    zone = mon->logicalBoxMinusReserved(); // below the menu bar, above the dock
    const int32_t bar  = lightsOf(w) != 0 ? wol_titlebar_height() : 0;
    const auto    sz   = w->m_realSize->goal();
    int32_t       fw = (int32_t)sz.x, fh = (int32_t)sz.y;
    if (!isFixedSize(w)) {
        int32_t minW, minH, maxW, maxH;
        sizeLimits(w, minW, minH, maxW, maxH);
        wol_fit((int32_t)sz.x, (int32_t)sz.y, minW, minH, maxW, maxH, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h, bar, &fw, &fh);
    }
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
    g_layoutManager->setTargetGeom(CBox{Vector2D(x, y), Vector2D(fw, fh)}, w->layoutTarget());
}

// Raise on a focus the user asked for. Not on focus that merely follows the desktop: the pointer
// (FFM), a desktop or special-workspace switch, the fallback after a window closed (the next window
// is wherever it was, macOS leaves the order alone), activation (CWindow::activate raises a floating
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

static void onActive(PHLWINDOW w, Desktop::eFocusReason reason) {
    if (!w) return;
    wol_qos_foreground((int32_t)w->getPID());
    // macOS: the active window is frontmost. focuswindow alone leaves floating windows behind.
    if (w->m_isFloating && raisesOnFocus(reason))
        HyprlandAPI::invokeHyprctlCommand("dispatch", std::format("alterzorder top,address:0x{:x}", (uintptr_t)w.get()));
}

// One floating window pulled back into `zone` and cut to fit (realtime placement::refit).
static void refitWindow(PHLWINDOW w, const CBox& zone) {
    const auto    pos = w->m_realPosition->goal(), sz = w->m_realSize->goal();
    const int32_t bar = lightsOf(w) != 0 ? wol_titlebar_height() : 0;
    int32_t       minW, minH, maxW, maxH;
    sizeLimits(w, minW, minH, maxW, maxH);
    if (isFixedSize(w))
        minW = maxW = clientDim(sz.x), minH = maxH = clientDim(sz.y);
    int32_t x = (int32_t)pos.x, y = (int32_t)pos.y, fw = (int32_t)sz.x, fh = (int32_t)sz.y;
    if (wol_refit(minW, minH, maxW, maxH, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h, bar, &x, &y, &fw, &fh))
        g_layoutManager->setTargetGeom(CBox{Vector2D(x, y), Vector2D(fw, fh)}, w->layoutTarget());
}

// The usable area of an output changed (VBoxClient resized the guest display, the Dock hid or
// came back, the menu bar moved): open floating windows are pulled back in and cut to fit
// (realtime placement::refit); zoomed windows get their band back from the layout.
static void refitMonitor(PHLMONITOR mon) {
    const auto zone = mon->logicalBoxMinusReserved();
    for (const auto& w : Desktop::windowState()->windows()) {
        if (!w->m_isMapped || !w->m_isFloating || w->m_monitor.lock() != mon || !w->layoutTarget())
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
        if (!w || !mon || !w->m_isMapped || !w->m_isFloating || !w->layoutTarget() || Fullscreen::controller()->isFullscreen(w))
            return;
        refitWindow(w, mon->logicalBoxMinusReserved());
    });
}

// Shell layers on screen (`wol-panel-*`, the click-catcher), by surface: Esc and the Super release
// are forwarded to the shell only while one is mapped (audit3 R2A-10). A `wol` process per Esc
// in every app, games included, was a fork+exec per key press.
static std::unordered_map<uintptr_t, std::string> g_shellLayers;
static bool                                       g_superDown = false, g_switching = false;

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

// One line to `$XDG_RUNTIME_DIR/wol-shell.sock` (shell::protocol), from the compositor: no fork,
// never blocks (a busy or absent shell loses the line, as a lost key press would).
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
    if (connect(fd, (const sockaddr*)&addr, sizeof(addr)) == 0 && write(fd, line, std::strlen(line)) < 0) {
        // Lost: nothing to do.
    }
    close(fd);
}

// Keys are never consumed: the app still gets its Esc, the binds still run.
static void onKey(const IKeyboard::SKeyEvent& e) {
    const bool pressed = e.state == WL_KEYBOARD_KEY_STATE_PRESSED;
    switch (e.keycode) {
        case KEY_LEFTMETA:
        case KEY_RIGHTMETA:
            g_superDown = pressed;
            // Releasing Super finishes ⌘Tab: also while the strip still waits for its client list
            // (not mapped yet), hence the Tab seen since Super went down.
            if (!pressed && (g_switching || shellLayerMapped("wol-panel-switcher")))
                tellShell("hide switcher\n");
            if (!pressed)
                g_switching = false;
            break;
        case KEY_TAB:
            if (pressed && g_superDown)
                g_switching = true;
            break;
        case KEY_ESC:
            if (pressed && shellLayerMapped(nullptr))
                tellShell("hide transient\n");
            break;
        default: break;
    }
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
static void onRenderStage(eRenderStage stage) {
    if (stage == RENDER_BEGIN) {
        const auto& rd = g_pHyprRenderer->m_renderData;
        uint64_t     px = 0;
        for (const auto& r : rd.damage.getRects())
            px += (uint64_t)(r.x2 - r.x1) * (uint64_t)(r.y2 - r.y1);
        const auto mon = rd.pMonitor.lock();
        g_frameDamage  = px;
        g_frameScreen  = mon ? (uint64_t)mon->m_pixelSize.x * (uint64_t)mon->m_pixelSize.y : 0;
        g_frameBegin   = monoNs();
    } else if (stage == RENDER_POST && g_frameBegin) {
        wol_frame_record(g_frameBegin, monoNs(), g_frameDamage, g_frameScreen, 1);
        g_frameBegin = 0;
    }
}

static std::string wolStats(eHyprCtlOutputFormat, std::string) {
    char buf[256];
    return wol_frame_stats_json((uint8_t*)buf, sizeof(buf)) < 0 ? std::string("{}") : std::string(buf);
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;
    // Close curve is sampled by the animation config generated from wol_anim_duration_ms.
    static auto open = Event::bus()->m_events.window.open.listen([](PHLWINDOW w) { onOpen(w); });
    static auto openLate = Event::bus()->m_events.window.openLate.listen([](PHLWINDOW w) { onOpenLate(w); });
    static auto active = Event::bus()->m_events.window.active.listen([](PHLWINDOW w, Desktop::eFocusReason r) { onActive(w, r); });
    static auto render = Event::bus()->m_events.render.stage.listen([](eRenderStage s) { onRenderStage(s); });
    static auto pre    = Event::bus()->m_events.render.pre.listen([](PHLMONITOR m) { onRenderPre(m); });
    static auto fs     = Event::bus()->m_events.window.fullscreen.listen([](PHLWINDOW w) { onFullscreen(w); });
    static auto lsOpen = Event::bus()->m_events.layer.opened.listen([](PHLLS l) { onLayerOpened(l); });
    static auto lsShut = Event::bus()->m_events.layer.closed.listen([](PHLLS l) { onLayerClosed(l); });
    static auto key    = Event::bus()->m_events.input.keyboard.key.listen([](IKeyboard::SKeyEvent e, Event::SCallbackInfo&) { onKey(e); });
    g_wolstats = HyprlandAPI::registerHyprCtlCommand(PHANDLE, SHyprCtlCommand{.name = "wolstats", .exact = true, .fn = wolStats});
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

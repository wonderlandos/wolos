// wol-hypr: thin Hyprland plugin. All policy lives in Rust (crates/realtime, C ABI wol_realtime.h).
#include <hyprland/src/plugins/PluginAPI.hpp>
// Targets Hyprland 0.56 (Event::bus(), desktop/view/Window.hpp, positionAnimation()).
#include <hyprland/src/desktop/view/Window.hpp>
#include <hyprland/src/output/Monitor.hpp>
#include <hyprland/src/event/EventBus.hpp>
#include <hyprland/src/Compositor.hpp>
// Built with -fno-access-control: the decoration map and toplevel resource are private.
#include <hyprland/src/protocols/XDGDecoration.hpp>
#include <hyprland/src/protocols/XDGShell.hpp>
#include <hyprland/src/layout/target/Target.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprland/src/xwayland/XSurface.hpp>
#include <format>
#include <time.h>
#include "../../crates/realtime/include/wol_realtime.h"

inline HANDLE PHANDLE = nullptr;

APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

// What the window told us about its frame, as wol_window_lights reads it (crates/realtime chrome.rs).
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
        return 0;
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

// After mapWindow's layout recalc (window.open is too early: the layout re-centres the window).
static void onOpenLate(PHLWINDOW w) {
    if (!w || !w->m_isFloating || !w->layoutTarget()) return;
    const bool bar = lightsOf(w) != 0;
    auto mon = w->m_monitor.lock();
    if (!mon) return;
    int32_t x = 0, y = 0;
    const auto sz = w->m_realSize->goal();
    const auto zone = mon->logicalBoxMinusReserved(); // below the menu bar, above the dock
    wol_place((int32_t)sz.x, (int32_t)sz.y, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h,
              bar ? wol_titlebar_height() : 0, 0, 0, 0, &x, &y);
    w->layoutTarget()->setPositionGlobal(CBox{Vector2D(x, y), sz});
}

static void onActive(PHLWINDOW w) {
    if (!w) return;
    wol_qos_foreground((int32_t)w->getPID());
    // macOS: the active window is frontmost. focuswindow alone leaves floating windows behind.
    if (w->m_isFloating)
        HyprlandAPI::invokeHyprctlCommand("dispatch", std::format("alterzorder top,address:0x{:x}", (uintptr_t)w.get()));
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
    static auto active = Event::bus()->m_events.window.active.listen([](PHLWINDOW w, Desktop::eFocusReason) { onActive(w); });
    static auto render = Event::bus()->m_events.render.stage.listen([](eRenderStage s) { onRenderStage(s); });
    HyprlandAPI::registerHyprCtlCommand(PHANDLE, SHyprCtlCommand{.name = "wolstats", .exact = true, .fn = wolStats});
    return {"wol-hypr", "WolOS window policy (placement, curves, QoS, frame stats)", "WolOS", "0.10.0"};
}

APICALL EXPORT void PLUGIN_EXIT() {}

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
#include <format>
#include <time.h>
#include "../../crates/realtime/include/wol_realtime.h"

inline HANDLE PHANDLE = nullptr;

APICALL EXPORT std::string PLUGIN_API_VERSION() { return HYPRLAND_API_VERSION; }

// GTK4/libadwaita apps never create an xdg-decoration object: they draw their own title bar.
static bool drawsOwnTitlebar(PHLWINDOW w) {
    const auto xdg = w->m_xdgSurface.lock();
    const auto top = xdg ? xdg->m_toplevel.lock() : nullptr;
    return top && !PROTO::xdgDecoration->m_decorations.contains(top->m_resource->resource());
}

static void onOpen(PHLWINDOW w) {
    if (!w) return;
    // `windowrule = match:tag wol-csd, hyprbars:no_bar on` keeps hyprbars off them (no double bar).
    const bool csd = drawsOwnTitlebar(w);
    if (csd)
        HyprlandAPI::invokeHyprctlCommand("dispatch", std::format("tagwindow +wol-csd address:0x{:x}", (uintptr_t)w.get()));
}

// After mapWindow's layout recalc (window.open is too early: the layout re-centres the window).
static void onOpenLate(PHLWINDOW w) {
    if (!w || !w->m_isFloating || !w->layoutTarget()) return;
    const bool csd = drawsOwnTitlebar(w);
    auto mon = w->m_monitor.lock();
    if (!mon) return;
    int32_t x = 0, y = 0;
    const auto sz = w->m_realSize->goal();
    const auto zone = mon->logicalBoxMinusReserved(); // below the menu bar, above the dock
    wol_place((int32_t)sz.x, (int32_t)sz.y, (int32_t)zone.x, (int32_t)zone.y, (int32_t)zone.w, (int32_t)zone.h,
              csd ? 0 : wol_titlebar_height(), 0, 0, 0, &x, &y);
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

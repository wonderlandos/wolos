-- WolOS window buttons (hyprbars), loaded once by /usr/share/wolos/hypr/hyprland.lua.
-- One quiet neutral circle for all three (the Kit's idle window control, a mid violet grey that
-- reads on the light and the dark bar), ink glyphs close x, zoom [], minimise - shown while the
-- pointer is over the group. Right alignment: the first button sits at the right edge, so the order
-- here is realtime's right row (titlebar::Align::Right): close, zoom, minimise; a close-only
-- dialog (hyprbars:buttons 1) keeps close at the edge. The button under the pointer takes the
-- theme's accent (button_hover_color / col.button_hover_text in hypr/wolos/theme.lua), like the
-- client-side controls (GTK 3/4, Firefox, Kit): crates/core/src/theme/apps.rs.
-- close/minimize/zoom act on the bar's own window when released over the button.
-- The plugin's functions exist from the compositor's second pass of the config on (after it loads
-- the plugins); the first pass skips the buttons.
local bars = hl.plugin.hyprbars
if bars then
    bars.add_button({ bg_color = "rgb(8f8da8)", size = 12, icon = "×", action = "close", fg_color = "rgba(221b36ff)" })
    bars.add_button({ bg_color = "rgb(8f8da8)", size = 12, icon = "□", action = "zoom", fg_color = "rgba(221b36ff)" })
    bars.add_button({ bg_color = "rgb(8f8da8)", size = 12, icon = "−", action = "minimize", fg_color = "rgba(221b36ff)" })
end

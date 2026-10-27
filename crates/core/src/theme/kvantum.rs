//! Kvantum (Qt) theme from WolOS tokens: `WolOS.kvconfig` + `WolOS.svg`. Controls are 9-slice
//! SVG elements (`<element>-<state>[-<side>]`), 6 px rounded like the kit; flat tabs with an accent
//! underline; transient (overlay) scrollbars; accent selection, checks, sliders and progress.
use super::ResolvedTheme;
use shared::theme::{Color, Variant};

/// Corner radius and 9-slice frame width (px), as kit `--radius-s`.
pub const RADIUS: u32 = 6;
/// Keyboard focus ring width (px); also its frame size, so Kvantum draws it 1:1.
pub const FOCUS: u32 = 2;
const STATES: [&str; 5] = ["normal", "focused", "pressed", "toggled", "disabled"];
const SIDES: [&str; 8] = [
    "top",
    "bottom",
    "left",
    "right",
    "topleft",
    "topright",
    "bottomleft",
    "bottomright",
];

/// One state's look: fill, border (None = no border), border width.
#[derive(Clone, Copy)]
struct Look {
    fill: Option<Color>,
    border: Option<Color>,
}

struct Pal {
    dark: bool,
    bg: Color,
    card: Color,
    control: Color,
    line: Color,
    text: Color,
    dim: Color,
    accent: Color,
    accent_text: Color,
}

fn mix(a: Color, b: Color, t: f64) -> Color {
    let m = |x: u8, y: u8| (f64::from(x) * (1.0 - t) + f64::from(y) * t).round() as u8;
    Color::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

impl Pal {
    fn new(t: &ResolvedTheme) -> Self {
        let dark = t.variant == Variant::Dark;
        let bg = Color::parse(&t.tokens["surface"]).unwrap_or(Color::WHITE);
        let c = |k: &str| {
            t.tokens
                .get(k)
                .and_then(|v| Color::parse(v))
                .map_or(bg, |c| c.over(bg))
        };
        let black = Color::rgb(0, 0, 0);
        Pal {
            dark,
            bg,
            card: c("card"),
            control: c("control"),
            // `line` is a hairline alpha; Qt needs it a little firmer to read as an edge.
            line: mix(bg, if dark { Color::WHITE } else { black }, 0.16),
            text: c("text"),
            dim: c("dim"),
            accent: c("accent"),
            accent_text: c("accentText"),
        }
    }
    fn hover(&self, c: Color) -> Color {
        mix(c, self.text, 0.06)
    }
    fn press(&self, c: Color) -> Color {
        mix(c, self.text, 0.14)
    }
}

fn hex(c: Color) -> String {
    c.to_hex()
}

/// SVG writer: each element gets its own cell so ids never overlap.
struct Svg {
    body: String,
    x: u32,
    y: u32,
    /// Border width (px) for `frame`.
    bw: f64,
    /// Frame (9-slice edge) size and corner radius (px) for `frame`.
    fr: u32,
}

impl Svg {
    fn cell(&mut self, w: u32, _h: u32) -> (u32, u32) {
        if self.x + w > 2000 {
            self.x = 0;
            self.y += 80;
        }
        let at = (self.x, self.y);
        self.x += w + 4;
        at
    }

    /// 9-slice rounded frame (`<el>-<state>-<side>`) plus interior (`<el>-<state>`).
    fn frame(&mut self, el: &str, state: &str, look: Look, frame: bool, interior: bool) {
        let r = f64::from(self.fr);
        let fill = look.fill.map_or("none".into(), hex);
        let stroke = look.border.map_or("none".into(), hex);
        // Kvantum skips the bottom frame of a top tab (it joins the tab frame), so the flat-tab
        // underline lives in the interior: 1.5 of 20 px, about 2 px at a 28 px tab height.
        let underline = el == "tab" && matches!(state, "toggled" | "pressed");
        if interior {
            let (x, y) = self.cell(20, 20);
            let extra = match (underline, look.border) {
                (true, Some(c)) => format!(
                    r#"<rect x="{x}" y="{}" width="20" height="1.5" fill="{}"/>"#,
                    f64::from(y) + 18.5,
                    hex(c)
                ),
                _ => String::new(),
            };
            self.body += &format!(
                r#"<g id="{el}-{state}"><rect x="{x}" y="{y}" width="20" height="20" fill="{fill}" fill-opacity="{}"/>{extra}</g>"#,
                if look.fill.is_some() { 1 } else { 0 }
            );
        }
        if !frame {
            return;
        }
        for side in SIDES {
            let (w, h) = match side {
                "top" | "bottom" => (20, self.fr),
                "left" | "right" => (self.fr, 20),
                _ => (self.fr, self.fr),
            };
            let (x, y) = self.cell(w, h);
            let (xf, yf) = (x as f64, y as f64);
            let (wf, hf) = (w as f64, h as f64);
            // Fill and a 1 px border inset by half a pixel; corners are quarter circles.
            // Every piece fills exactly its cell (bounds = cell, no stroke overhang), the 1 px
            // border is a filled shape. A transparent cell rect pins the element bounds.
            let pin = format!(
                r#"<rect x="{xf}" y="{yf}" width="{wf}" height="{hf}" fill="black" fill-opacity="0"/>"#
            );
            let edge = |x: f64, y: f64, w: f64, h: f64| {
                format!(r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{stroke}"/>"#)
            };
            let has_border = look.border.is_some() && !underline;
            let shape = match side {
                "top" | "bottom" | "left" | "right" => {
                    let mut out = format!(
                        r#"{pin}<rect x="{xf}" y="{yf}" width="{wf}" height="{hf}" fill="{fill}"/>"#
                    );
                    if has_border {
                        out += &match side {
                            "top" => edge(xf, yf, wf, self.bw),
                            "bottom" => edge(xf, yf + hf - self.bw, wf, self.bw),
                            "left" => edge(xf, yf, self.bw, hf),
                            _ => edge(xf + wf - self.bw, yf, self.bw, hf),
                        };
                    }
                    out
                }
                corner => {
                    // Circle centre at the inner corner of the cell; s/e = arc ends at radius r.
                    let (cx, cy, sx, sy, ex, ey) = match corner {
                        "topleft" => (xf + r, yf + r, xf, yf + r, xf + r, yf),
                        "topright" => (xf, yf + r, xf, yf, xf + r, yf + r),
                        "bottomleft" => (xf + r, yf, xf + r, yf + r, xf, yf),
                        _ => (xf, yf, xf + r, yf, xf, yf + r),
                    };
                    // Cubic quarter circle of radius `rad` (QtSvg mis-centres some SVG arcs).
                    const K: f64 = 0.5523;
                    let q = |rad: f64| {
                        let f = rad / r;
                        let (s0, s1) = (cx + (sx - cx) * f, cy + (sy - cy) * f);
                        let (e0, e1) = (cx + (ex - cx) * f, cy + (ey - cy) * f);
                        let (o0, o1) = (s0 + e0 - cx, s1 + e1 - cy);
                        [
                            (s0, s1),
                            (s0 + (o0 - s0) * K, s1 + (o1 - s1) * K),
                            (e0 + (o0 - e0) * K, e1 + (o1 - e1) * K),
                            (e0, e1),
                        ]
                    };
                    let [a, b, c, d] = q(r);
                    let mut out = format!(
                        r#"{pin}<path d="M{cx} {cy} L{} {} C{} {} {} {} {} {} Z" fill="{fill}"/>"#,
                        a.0, a.1, b.0, b.1, c.0, c.1, d.0, d.1
                    );
                    if has_border {
                        let [ai, bi, ci, di] = q(r - self.bw);
                        out += &format!(
                            r#"<path d="M{} {} C{} {} {} {} {} {} L{} {} C{} {} {} {} {} {} Z" fill="{stroke}"/>"#,
                            a.0,
                            a.1,
                            b.0,
                            b.1,
                            c.0,
                            c.1,
                            d.0,
                            d.1,
                            di.0,
                            di.1,
                            ci.0,
                            ci.1,
                            bi.0,
                            bi.1,
                            ai.0,
                            ai.1
                        );
                    }
                    out
                }
            };
            self.body += &format!(r#"<g id="{el}-{state}-{side}">{shape}</g>"#);
        }
    }

    fn all_states(&mut self, el: &str, frame: bool, interior: bool, look: impl Fn(&str) -> Look) {
        for s in STATES {
            self.frame(el, s, look(s), frame, interior);
        }
    }

    /// Fixed-size indicator element (`<id>`): raw SVG content drawn in a 16x16 box at 0,0.
    fn indicator(&mut self, id: &str, content: &str) {
        let (x, y) = self.cell(16, 16);
        self.body += &format!(r#"<g id="{id}" transform="translate({x} {y})">{content}</g>"#);
    }
}

fn button_look(p: &Pal, s: &str) -> Look {
    let fill = match s {
        "focused" => p.hover(p.control),
        "pressed" => p.press(p.control),
        "toggled" => mix(p.control, p.accent, 0.25),
        "disabled" => mix(p.control, p.bg, 0.5),
        _ => p.control,
    };
    Look {
        fill: Some(fill),
        border: Some(if s == "disabled" {
            mix(p.line, p.bg, 0.5)
        } else {
            p.line
        }),
    }
}

/// `WolOS.svg`.
pub fn svg(t: &ResolvedTheme) -> String {
    let p = Pal::new(t);
    let mut g = Svg {
        body: String::new(),
        x: 0,
        y: 0,
        bw: 1.0,
        fr: RADIUS,
    };
    let none = Look {
        fill: None,
        border: None,
    };
    g.all_states("button", true, true, |s| button_look(&p, s));
    g.all_states("lineedit", true, true, |s| Look {
        fill: Some(if s == "disabled" { p.bg } else { p.card }),
        border: Some(if s == "focused" || s == "toggled" {
            p.accent
        } else {
            p.line
        }),
    });
    g.all_states("common", true, true, |_| Look {
        fill: Some(p.card),
        border: Some(p.line),
    });
    g.all_states("tabframe", true, true, |_| Look {
        fill: Some(p.card),
        border: Some(p.line),
    });
    g.all_states("menu", true, true, |_| Look {
        fill: Some(p.card),
        border: Some(p.line),
    });
    g.all_states("tooltip", true, true, |_| Look {
        fill: Some(p.card),
        border: Some(p.line),
    });
    // Selections, menu items: accent, rounded, no border.
    g.all_states("itemview", true, true, |s| match s {
        "focused" => Look {
            fill: Some(mix(p.card, p.accent, 0.12)),
            border: None,
        },
        "pressed" | "toggled" => Look {
            fill: Some(p.accent),
            border: None,
        },
        _ => none,
    });
    g.all_states("menuitem", true, true, |s| match s {
        "focused" | "pressed" | "toggled" => Look {
            fill: Some(p.accent),
            border: None,
        },
        _ => none,
    });
    // Flat tabs: only the current tab is drawn (card), hover is a faint tint.
    g.all_states("tab", true, true, |s| match s {
        // Flat tabs: the current tab is only an accent underline.
        "toggled" | "pressed" => Look {
            fill: None,
            border: Some(p.accent),
        },
        "focused" => Look {
            fill: Some(p.hover(p.bg)),
            border: None,
        },
        _ => none,
    });
    // Progress and slider grooves (control), filled part / chunk (accent).
    g.all_states("progress", true, true, |_| Look {
        fill: Some(p.control),
        border: None,
    });
    g.all_states("progress-pattern", true, true, |_| Look {
        fill: Some(p.accent),
        border: None,
    });
    g.all_states("slider", true, true, |s| Look {
        fill: Some(if s == "toggled" { p.accent } else { p.control }),
        border: None,
    });
    // Transient scrollbar: thin rounded thumb, no groove.
    g.all_states("scrollbarslider", true, true, |s| Look {
        fill: Some(mix(
            p.bg,
            p.text,
            match s {
                "focused" | "pressed" => 0.55,
                _ => 0.35,
            },
        )),
        border: None,
    });
    g.all_states("header", true, true, |s| Look {
        fill: Some(if s == "focused" { p.hover(p.bg) } else { p.bg }),
        border: None,
    });

    // Keyboard focus: 2 px accent ring at half opacity (libadwaita's focus ring), stateless
    // `focus-<side>` pieces as in the stock themes.
    g.bw = FOCUS as f64;
    g.fr = FOCUS;
    let start = g.body.len();
    g.frame(
        "focus",
        "ring",
        Look {
            fill: None,
            border: Some(mix(p.accent, p.bg, 0.5)),
        },
        true,
        false,
    );
    let ring = g.body.split_off(start).replace("focus-ring-", "focus-");
    g.body += &ring;
    g.bw = 1.0;
    g.fr = RADIUS;
    // Kvantum marks the default button with this element, or a built-in triangle when absent.
    let (x, y) = g.cell(8, 8);
    g.body += &format!(
        r#"<g id="button-default-indicator"><rect x="{x}" y="{y}" width="8" height="8" fill="black" fill-opacity="0"/></g>"#
    );
    let (ink, white) = (hex(p.text), hex(Color::WHITE));
    let (acc, acc_t, line, card, dim) = (
        hex(p.accent),
        hex(p.accent_text),
        hex(p.line),
        hex(p.card),
        hex(p.dim),
    );
    let check = format!(
        r#"<path d="M4 8.5l2.5 2.5 5.5-6" fill="none" stroke="{acc_t}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    for s in STATES {
        let a = if s == "disabled" { "0.45" } else { "1" };
        let edge = if s == "focused" { &acc } else { &line };
        g.indicator(
            &format!("checkbox-{s}"),
            &format!(r#"<rect x=".5" y=".5" width="15" height="15" rx="4" fill="{card}" stroke="{edge}" opacity="{a}"/>"#),
        );
        g.indicator(
            &format!("checkbox-checked-{s}"),
            &format!(
                r#"<g opacity="{a}"><rect width="16" height="16" rx="4" fill="{acc}"/>{check}</g>"#
            ),
        );
        g.indicator(
            &format!("checkbox-tristate-{s}"),
            &format!(r#"<g opacity="{a}"><rect width="16" height="16" rx="4" fill="{acc}"/><path d="M4.5 8h7" stroke="{acc_t}" stroke-width="2" stroke-linecap="round"/></g>"#),
        );
        g.indicator(
            &format!("radio-{s}"),
            &format!(
                r#"<circle cx="8" cy="8" r="7.5" fill="{card}" stroke="{edge}" opacity="{a}"/>"#
            ),
        );
        g.indicator(
            &format!("radio-checked-{s}"),
            &format!(r#"<g opacity="{a}"><circle cx="8" cy="8" r="8" fill="{acc}"/><circle cx="8" cy="8" r="3" fill="{acc_t}"/></g>"#),
        );
        g.indicator(
            &format!("slidercursor-{s}"),
            &format!(
                r#"<circle cx="8" cy="8" r="7.5" fill="{white}" stroke="{line}" opacity="{a}"/>"#
            ),
        );
        for (dir, d) in [
            ("down", "M4.5 6.5l3.5 3.5 3.5-3.5"),
            ("up", "M4.5 9.5l3.5-3.5 3.5 3.5"),
            ("left", "M9.5 4.5l-3.5 3.5 3.5 3.5"),
            ("right", "M6.5 4.5l3.5 3.5-3.5 3.5"),
        ] {
            let c = if s == "disabled" { &dim } else { &ink };
            g.indicator(
                &format!("arrow-{dir}-{s}"),
                &format!(r#"<path d="{d}" fill="none" stroke="{c}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#),
            );
        }
        for (k, d) in [("plus", "M8 4.5v7M4.5 8h7"), ("minus", "M4.5 8h7")] {
            g.indicator(
                &format!("tree-{k}-{s}"),
                &format!(
                    r#"<path d="{d}" stroke="{dim}" stroke-width="1.6" stroke-linecap="round"/>"#
                ),
            );
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2000\" height=\"{}\">\n<!-- WolOS {} - generated by `wol theme`; edits are overwritten. -->\n{}\n</svg>\n",
        g.y + 100,
        t.id,
        g.body
    )
}

/// Section: (name, frame element, interior element, indicator element, extra keys).
const SECTIONS: [(&str, &str, &str, &str, &str); 26] = [
    ("PanelButtonCommand", "button", "button", "arrow", ""),
    ("PanelButtonTool", "button", "button", "arrow", ""),
    (
        "ToolbarButton",
        "button",
        "button",
        "arrow",
        "frame=false\ninterior=false\n",
    ),
    // Combo arrows: the drop-down part uses the element name as is (`arrow-down-<state>`).
    ("DropDownButton", "button", "button", "arrow-down", ""),
    ("ComboBox", "button", "button", "none", ""),
    ("LineEdit", "lineedit", "lineedit", "", ""),
    ("IndicatorSpinBox", "button", "button", "arrow", ""),
    // Kvantum draws check and radio marks as the section's interior.
    ("RadioButton", "", "radio", "", "frame=false\n"),
    ("CheckBox", "", "checkbox", "", "frame=false\n"),
    ("GenericFrame", "common", "common", "", "interior=false\n"),
    ("GroupBox", "common", "common", "", ""),
    ("TabFrame", "tabframe", "tabframe", "", ""),
    ("Tab", "tab", "tab", "", ""),
    ("ItemView", "itemview", "itemview", "", ""),
    ("Menu", "menu", "menu", "", ""),
    ("MenuItem", "menuitem", "menuitem", "arrow", ""),
    ("MenuBarItem", "menuitem", "menuitem", "", ""),
    ("ToolTip", "tooltip", "tooltip", "", ""),
    ("Progressbar", "progress", "progress", "", ""),
    (
        "ProgressbarContents",
        "progress-pattern",
        "progress-pattern",
        "",
        "",
    ),
    ("Slider", "slider", "slider", "", ""),
    ("SliderCursor", "", "slidercursor", "", "frame=false\n"),
    (
        "ScrollbarSlider",
        "scrollbarslider",
        "scrollbarslider",
        "",
        "",
    ),
    ("HeaderSection", "header", "header", "", ""),
    ("Focus", "focus", "", "", ""),
    // Stock themes (KvFlat): combo/spin/menu arrows come from this section.
    ("IndicatorArrow", "", "", "arrow", ""),
];

/// `WolOS.kvconfig`.
pub fn kvconfig(t: &ResolvedTheme) -> String {
    let p = Pal::new(t);
    let mut s = format!(
        "# WolOS {} - generated by `wol theme`; edits are overwritten.\n\
[%General]\nauthor=WolOS\ncomment=Generated from WolOS theme tokens\n\
x11drag=menubar_and_primary_toolbar\nalt_mnemonic=true\nleft_tabs=false\nattach_active_tab=false\n\
group_toolbar_buttons=false\nspread_progressbar=true\nprogressbar_thickness=6\nscroll_width=8\n\
scroll_min_extent=40\ntransient_scrollbar=true\ntransient_groove=false\nscrollbar_in_view=true\n\
scroll_arrows=false\nslider_width=4\nslider_handle_width=16\nslider_handle_length=16\n\
check_size=16\ntooltip_delay=500\nsubmenu_overlap=0\nmenu_shadow_depth=0\ntooltip_shadow_depth=0\n\
composite=true\ntranslucent_windows=false\nblurring=false\npopup_blurring=false\nanimate_states=true\n\
button_contents_shift=false\ncombo_as_lineedit=false\ncombo_menu=true\nhide_combo_checkboxes=true\n\
layout_spacing=6\nlayout_margin=12\nsmall_icon_size=16\nlarge_icon_size=32\nbutton_icon_size=16\n\
toolbar_icon_size=22\nfill_rubberband=false\nrespect_DE=true\nno_inactiveness=true\n\
dark_titlebar={}\nmenubar_mouse_tracking=true\nmerge_menubar_with_toolbar=true\n\
inline_spin_indicators=true\nvertical_spin_indicators=false\ntree_branch_line=false\n\n",
        t.id, p.dark
    );
    let c = |c: Color| hex(c);
    s += &format!(
        "[GeneralColors]\nwindow.color={}\nbase.color={}\nalt.base.color={}\nbutton.color={}\n\
light.color={}\nmid.light.color={}\ndark.color={}\nmid.color={}\nhighlight.color={}\n\
inactive.highlight.color={}\ntext.color={}\nwindow.text.color={}\nbutton.text.color={}\n\
disabled.text.color={}\ntooltip.text.color={}\nhighlight.text.color={}\nlink.color={}\n\
link.visited.color={}\nprogress.indicator.text.color={}\n\n",
        c(p.bg),
        c(p.card),
        c(mix(p.card, p.bg, 0.5)),
        c(p.control),
        c(mix(p.control, Color::WHITE, 0.4)),
        c(mix(p.control, Color::WHITE, 0.2)),
        c(p.line),
        c(mix(p.control, p.text, 0.2)),
        c(p.accent),
        c(mix(p.accent, p.bg, 0.4)),
        c(p.text),
        c(p.text),
        c(p.text),
        c(p.dim),
        c(p.text),
        c(p.accent_text),
        c(p.accent),
        c(mix(p.accent, p.text, 0.3)),
        c(p.text),
    );
    for (name, frame, interior, indicator, extra) in SECTIONS {
        s += &format!("[{name}]\n");
        let r = if name == "Focus" { FOCUS } else { RADIUS };
        if !frame.is_empty() {
            s += &format!(
                "frame=true\nframe.element={frame}\nframe.top={r}\nframe.bottom={r}\nframe.left={r}\nframe.right={r}\n"
            );
        }
        if !interior.is_empty() {
            s += &format!("interior=true\ninterior.element={interior}\n");
        }
        if !indicator.is_empty() {
            s += &format!("indicator.element={indicator}\nindicator.size=9\n");
        }
        s += extra;
        let sel = matches!(name, "ItemView" | "MenuItem" | "MenuBarItem");
        let on = if sel { c(p.accent_text) } else { c(p.text) };
        s += &format!(
            "text.normal.color={}\ntext.focus.color={}\ntext.press.color={on}\ntext.toggle.color={on}\ntext.disabled.color={}\n\
text.margin.left=6\ntext.margin.right=6\ntext.margin.top=2\ntext.margin.bottom=2\n\n",
            c(p.text),
            if name == "MenuItem" {
                c(p.accent_text)
            } else {
                c(p.text)
            },
            c(p.dim)
        );
    }
    s += "[Hacks]\nrespect_darkness=true\ntransparent_dolphin_view=false\nlined_tabs=false\nsingle_top_toolbar=true\nno_selection_tint=true\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeCatalog;

    #[test]
    fn every_element_the_config_uses_is_drawn() {
        for id in ["wol-light", "wol-dark"] {
            let t = ThemeCatalog::builtin().resolve(id).unwrap();
            let (svg, conf) = (svg(&t), kvconfig(&t));
            let has = |el: &str| svg.contains(&format!("id=\"{el}\""));
            for (name, frame, interior, _indicator, extra) in SECTIONS {
                let on = |k: &str| !extra.contains(&format!("{k}=false"));
                for st in STATES {
                    if !frame.is_empty() && on("frame") {
                        for side in SIDES {
                            // The focus ring is stateless (`focus-<side>`), like the stock themes.
                            let el = if name == "Focus" {
                                format!("{frame}-{side}")
                            } else {
                                format!("{frame}-{st}-{side}")
                            };
                            assert!(has(&el), "{id} {name} {frame}-{st}-{side}");
                        }
                    }
                    if !interior.is_empty() && on("interior") {
                        assert!(
                            has(&format!("{interior}-{st}")),
                            "{id} {name} {interior}-{st}"
                        );
                    }
                }
                assert!(conf.contains(&format!("[{name}]")));
            }
            for el in [
                "checkbox-checked-normal",
                "radio-checked-normal",
                "slidercursor-normal",
                "arrow-down-normal",
            ] {
                assert!(has(el), "{id} {el}");
            }
            let accent = t.tokens["accent"].to_lowercase();
            assert!(svg.contains(&format!("fill=\"{accent}\"")), "{id} accent");
            assert!(conf.contains(&format!("highlight.color={accent}")));
            assert!(conf.contains("transient_scrollbar=true") && conf.contains("frame.top=6"));
            assert!(
                conf.contains("[Focus]\nframe=true\nframe.element=focus\nframe.top=2\n"),
                "focus ring frame is its 2 px width"
            );
            assert_eq!(
                svg.matches("<g ").count(),
                svg.matches("</g>").count(),
                "balanced"
            );
        }
    }
}

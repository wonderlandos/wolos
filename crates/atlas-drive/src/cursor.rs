// SPDX-License-Identifier: GPL-2.0-or-later
//! `cursor show`: a virtual mouse pointer drawn into the page so screenshots and recordings show
//! where the agent pointed, clicked or dragged. It is a fixed-position overlay (`pointer-events:none`),
//! removed by `cursor hide`; the page's own input events never see it.

#[derive(Default, Debug, Clone)]
pub struct Cursor {
    pub on: bool,
    pub x: f64,
    pub y: f64,
    pub trail: Vec<(f64, f64)>,
    /// Where the browser's mouse is, and in which document (`nav_gen`): a click at the same point
    /// needs no move event (a move is frame-aligned in the renderer, a press is not).
    pub at: Option<(u64, f64, f64)>,
}

impl Cursor {
    pub fn moved(&mut self, x: f64, y: f64) {
        self.x = x;
        self.y = y;
        if self.trail.len() >= 64 {
            self.trail.remove(0);
        }
        self.trail.push((x, y));
    }
}

/// The script that draws (or moves) the pointer; `ring` adds a click ring that fades out.
pub fn overlay_js(x: f64, y: f64, ring: bool, path: &[(f64, f64)]) -> String {
    let pts: Vec<String> = path
        .iter()
        .map(|(a, b)| format!("[{a:.0},{b:.0}]"))
        .collect();
    format!(
        r##"(()=>{{const R=document.documentElement;if(!R)return 0;
let c=document.getElementById('__atlas_drive_cursor');
if(!c){{c=document.createElement('div');c.id='__atlas_drive_cursor';
c.style.cssText='position:fixed;left:0;top:0;width:0;height:0;z-index:2147483647;pointer-events:none;';
c.innerHTML='<svg width="22" height="22" viewBox="0 0 22 22" style="position:absolute;left:0;top:0;filter:drop-shadow(0 1px 1px rgba(0,0,0,.5))"><path d="M2 2 L2 17 L6.5 13 L9.5 20 L12 19 L9 12 L15 12 Z" fill="#fff" stroke="#111" stroke-width="1.4" stroke-linejoin="round"/></svg>';
R.appendChild(c);}}
c.style.transform='translate({x:.0}px,{y:.0}px)';
let t=document.getElementById('__atlas_drive_trail');
if(!t){{t=document.createElementNS('http://www.w3.org/2000/svg','svg');t.id='__atlas_drive_trail';
t.setAttribute('style','position:fixed;left:0;top:0;width:100vw;height:100vh;z-index:2147483646;pointer-events:none');R.appendChild(t);}}
const P={pts};
if(P.length>1)t.innerHTML='<polyline points="'+P.map(p=>p.join(',')).join(' ')+'" fill="none" stroke="#e5484d" stroke-width="2" stroke-dasharray="4 3" opacity=".8"/>';
if({ring}){{const r=document.createElement('div');
r.style.cssText='position:fixed;left:{x:.0}px;top:{y:.0}px;width:30px;height:30px;margin:-15px 0 0 -15px;border:3px solid #e5484d;border-radius:50%;z-index:2147483647;pointer-events:none;opacity:.9;transition:transform .45s ease-out,opacity .45s ease-out';
R.appendChild(r);requestAnimationFrame(()=>{{r.style.transform='scale(2)';r.style.opacity='0';}});setTimeout(()=>r.remove(),600);}}
return 1;}})()"##,
        x = x,
        y = y,
        ring = ring,
        pts = if pts.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", pts.join(","))
        },
    )
}

pub const HIDE_JS: &str = "(()=>{for(const id of ['__atlas_drive_cursor','__atlas_drive_trail']){const e=document.getElementById(id);if(e)e.remove();}return 1})()";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trail_is_bounded() {
        let mut c = Cursor::default();
        for i in 0..200 {
            c.moved(i as f64, 0.0);
        }
        assert_eq!(c.trail.len(), 64);
        assert_eq!(c.x, 199.0);
    }

    #[test]
    fn overlay_mentions_position_and_ring() {
        let js = overlay_js(120.0, 45.0, true, &[(1.0, 2.0), (120.0, 45.0)]);
        assert!(js.contains("translate(120px,45px)"));
        assert!(js.contains("if(true)"));
        assert!(js.contains("[[1,2],[120,45]]"));
    }
}

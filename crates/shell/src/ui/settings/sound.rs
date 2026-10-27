//! Sound: output device, volume, mute.
use super::*;

// ---- Sound ----

fn fill_outputs(list: &gtk::Box, ctx: &Rc<Ctx>, st: &gtk::Label, outs: Vec<crate::system::Output>) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    if outs.is_empty() {
        list.append(&row("No output devices", &w::label("", "secondary")));
    }
    for o in &outs {
        let b = w::choice_row("audio-speakers-symbolic", &o.name, "", o.default);
        let (lc, ctx, st, id, all) = (
            list.clone(),
            ctx.clone(),
            st.clone(),
            o.id.clone(),
            outs.clone(),
        );
        b.connect_clicked(move |_| {
            let next: Vec<_> = all
                .iter()
                .cloned()
                .map(|mut x| {
                    x.default = x.id == id;
                    x
                })
                .collect();
            let (lc, c2, st) = (lc.clone(), ctx.clone(), st.clone());
            ctx.act(
                Action::Output(id.clone()),
                Rc::new(move |r| match r {
                    Ok(()) => {
                        say(&st, "");
                        fill_outputs(&lc, &c2, &st, next.clone());
                    }
                    Err(e) => say(&st, e.message()),
                }),
            );
        });
        list.append(&b);
    }
}

/// Volume while dragging: one action in flight, newest value queued.
#[derive(Default)]
struct Coalesce {
    busy: std::cell::Cell<bool>,
    latest: std::cell::Cell<Option<u8>>,
}

fn send_volume(ctx: &Rc<Ctx>, q: &Rc<Coalesce>, v: u8) {
    if q.busy.replace(true) {
        return q.latest.set(Some(v));
    }
    let (c2, q2) = (ctx.clone(), q.clone());
    ctx.act(
        Action::Volume(v),
        Rc::new(move |_| {
            q2.busy.set(false);
            if let Some(n) = q2.latest.take() {
                send_volume(&c2, &q2, n);
            }
        }),
    );
}

pub(super) fn page(ctx: &Rc<Ctx>) -> gtk::Box {
    let p = w::page("Sound");
    let status = status_label();
    let list = w::vbox(0);
    fill_outputs(&list, ctx, &status, ctx.sys.output_devices());
    p.append(&w::section("Output", &[list.upcast()]));
    let audio = ctx.sys.audio();
    let q = Rc::new(Coalesce::default());
    let slider = {
        let ctx = ctx.clone();
        w::row_slider(audio.volume.unwrap_or(0) as f64, move |v| {
            send_volume(&ctx, &q, v as u8)
        })
    };
    slider.set_sensitive(audio.volume.is_some());
    let mute = {
        let (ctx, st) = (ctx.clone(), status.clone());
        w::toggle(audio.muted, move |on| {
            let st = st.clone();
            ctx.act(
                Action::Muted(on),
                Rc::new(move |r| say(&st, r.err().map_or("", |e| e.message()))),
            );
        })
    };
    let vol = w::hbox(w::S3);
    vol.append(&w::icon("audio-volume-low-symbolic", w::ICON));
    vol.append(&slider);
    vol.append(&w::icon("audio-volume-high-symbolic", w::ICON));
    vol.set_hexpand(true);
    p.append(&w::section(
        "",
        &[row("Output volume", &vol), row("Mute", &mute)],
    ));
    p.append(&status);
    p
}

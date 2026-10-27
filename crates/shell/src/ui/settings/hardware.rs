//! Hardware: every device the hardware catalogue knows (`wol hw`, snapshot written at boot), what
//! drives it, and a Fix button for those that need a driver module or a firmware package
//! (wol-admind `hw-load`, App Store install; the Authenticate sheet asks first).
use super::machine::{Device, HardwareSnapshot, Kind, State};
use super::*;

pub(crate) fn kind_icon(k: Kind) -> &'static str {
    match k {
        Kind::Cpu | Kind::Other => "wolos-chip-symbolic",
        Kind::Gpu => "video-display-symbolic",
        Kind::Wifi => "network-wireless-symbolic",
        Kind::Ethernet => "network-wired-symbolic",
        Kind::Bluetooth => "bluetooth-active-symbolic",
        Kind::Audio => "audio-speakers-symbolic",
        Kind::Webcam => "camera-photo-symbolic",
        Kind::Input => "input-keyboard-symbolic",
        Kind::CardReader => "drive-harddisk-symbolic",
        Kind::Backlight => "display-brightness-symbolic",
    }
}

/// The steps Fix takes: modules to load, then packages to install. Empty = nothing to fix here.
pub(crate) fn fix_actions(d: &Device) -> Vec<Action> {
    let mut v: Vec<Action> = d
        .missing_klds
        .iter()
        .filter_map(|k| admin("hw-load", &[k]))
        .collect();
    v.extend(
        d.missing_packages
            .iter()
            .map(|p| Action::PkgInstall(p.clone())),
    );
    v
}

fn subtitle(d: &Device) -> String {
    match d.state {
        State::Ready => d
            .driver
            .as_ref()
            .map_or("Working".into(), |u| format!("Working · driver {u}")),
        _ if !d.hint.is_empty() => d.hint.clone(),
        _ if !d.notes.is_empty() => d.notes[0].clone(),
        State::NeedsDriver => "Needs a driver".into(),
        State::NeedsFirmware => "Needs firmware".into(),
        State::Unsupported => "Not supported yet".into(),
    }
}

/// Runs `steps` one after another; `done` gets the first error, or Ok after the last.
fn run_steps(ctx: &Rc<Ctx>, mut steps: Vec<Action>, done: Rc<dyn Fn(Result<(), ActionError>)>) {
    if steps.is_empty() {
        return done(Ok(()));
    }
    let first = steps.remove(0);
    let c = ctx.clone();
    ctx.act(
        first,
        Rc::new(move |r| match r {
            Ok(()) => run_steps(&c, steps.clone(), done.clone()),
            Err(e) => done(Err(e)),
        }),
    );
}

fn device_row(e: &Rc<Env>, d: &Device, reload: &Rc<dyn Fn()>, status: &gtk::Label) -> gtk::Widget {
    let steps = fix_actions(d);
    if d.state == State::Ready || steps.is_empty() {
        return w::list_row(kind_icon(d.kind), &d.name, &subtitle(d), None).upcast();
    }
    let (ctx, st, rl) = (e.ctx.clone(), status.clone(), reload.clone());
    let fix = w::button("Fix", w::Kind::Default, || {});
    fix.connect_clicked(move |b| {
        b.set_sensitive(false);
        b.set_label("Fixing…");
        let (st, rl, b) = (st.clone(), rl.clone(), b.clone());
        run_steps(
            &ctx,
            steps.clone(),
            Rc::new(move |r| match r {
                Ok(()) => rl(),
                Err(x) => {
                    b.set_sensitive(true);
                    b.set_label("Fix");
                    say(&st, x.message());
                }
            }),
        );
    });
    w::list_row(
        kind_icon(d.kind),
        &d.name,
        &subtitle(d),
        Some(fix.upcast_ref()),
    )
    .upcast()
}

fn render(
    e: &Rc<Env>,
    into: &gtk::Box,
    hw: HardwareSnapshot,
    reload: &Rc<dyn Fn()>,
    status: &gtk::Label,
) {
    if hw.devices.is_empty() {
        into.append(&w::empty_state(
            "wolos-chip-symbolic",
            "No Hardware Report Yet",
            "WolOS checks your devices each time it starts. Restart to see them here.",
            None,
        ));
        return;
    }
    let bad: Vec<&Device> = hw.problems().collect();
    let icon = if bad.is_empty() {
        "object-select-symbolic"
    } else {
        "dialog-warning-symbolic"
    };
    let line = about::hardware_summary(&hw);
    let summary = w::list_row(
        icon,
        &line,
        if bad.is_empty() {
            "Drivers and firmware are in place."
        } else if bad.iter().any(|d| !fix_actions(d).is_empty()) {
            "Fix installs what is missing. Nothing else changes."
        } else {
            "WolOS can’t make these work yet. Updates may add them."
        },
        None,
    );
    summary.add_css_class(if bad.is_empty() {
        "all-good"
    } else {
        "attention"
    });
    into.append(&w::section("", &[summary.upcast()]));
    if !bad.is_empty() {
        let rows: Vec<gtk::Widget> = bad
            .iter()
            .map(|d| device_row(e, d, reload, status))
            .collect();
        into.append(&w::section("Needs Attention", &rows));
    }
    let mut ready: Vec<&Device> = hw
        .devices
        .iter()
        .filter(|d| d.state == State::Ready)
        .collect();
    ready.sort_by_key(|d| d.kind);
    if !ready.is_empty() {
        let rows: Vec<gtk::Widget> = ready
            .iter()
            .map(|d| device_row(e, d, reload, status))
            .collect();
        into.append(&w::section("Devices", &rows));
    }
}

fn load(e: &Rc<Env>, into: &gtk::Box, status: &gtk::Label) {
    clear(into);
    let reload: Rc<dyn Fn()> = {
        let (e, into, st) = (e.clone(), into.downgrade(), status.clone());
        Rc::new(move || {
            if let Some(i) = into.upgrade() {
                load(&e, &i, &st)
            }
        })
    };
    let (m, e2, st) = (e.machine(), e.clone(), status.clone());
    later(
        into,
        move || m.hardware(),
        move |into, hw| render(&e2, into, hw, &reload, &st),
    );
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Hardware");
    let body = w::vbox(w::S6);
    p.append(&body);
    let status = status_label();
    p.append(&status);
    load(e, &body, &status);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_loads_modules_then_installs_packages() {
        let d = Device {
            missing_klds: vec!["if_iwlwifi".into()],
            missing_packages: vec!["wifi-firmware-iwlwifi-kmod".into()],
            state: State::NeedsFirmware,
            ..Default::default()
        };
        let steps = fix_actions(&d);
        assert_eq!(steps.len(), 2);
        assert!(matches!(&steps[0], Action::Admin(r) if r.args == ["if_iwlwifi"]));
        assert!(matches!(&steps[1], Action::PkgInstall(p) if p == "wifi-firmware-iwlwifi-kmod"));
        assert!(fix_actions(&Device::default()).is_empty());
        assert_eq!(subtitle(&Device::default()), "Working");
        let odd = Device {
            state: State::Unsupported,
            notes: vec!["This Wi-Fi card has no FreeBSD driver yet.".into()],
            ..Default::default()
        };
        assert_eq!(subtitle(&odd), "This Wi-Fi card has no FreeBSD driver yet.");
        assert_eq!(
            subtitle(&Device {
                notes: vec![],
                ..odd
            }),
            "Not supported yet"
        );
    }
}

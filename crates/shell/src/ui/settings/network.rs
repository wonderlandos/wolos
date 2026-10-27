//! Network: every connection with its addresses, router and DNS (read off the UI thread).
use super::*;
use wol_core::netinfo::{IfKind, Interface, NetInfo};

/// "192.168.1.24/24" -> ("192.168.1.24", "255.255.255.0").
pub(crate) fn split_cidr(a: &str) -> (String, String) {
    let (ip, bits) = a.split_once('/').unwrap_or((a, ""));
    let mask = bits.parse::<u32>().ok().filter(|b| *b <= 32).map(|b| {
        let m: u32 = if b == 0 { 0 } else { u32::MAX << (32 - b) };
        let o = m.to_be_bytes();
        format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
    });
    (ip.to_string(), mask.unwrap_or_default())
}

fn kind_label(i: &Interface) -> (&'static str, &'static str) {
    match i.kind {
        IfKind::WiFi => ("network-wireless-symbolic", "Wi-Fi"),
        IfKind::Ethernet => ("network-wired-symbolic", "Ethernet"),
        IfKind::Other => ("network-wired-symbolic", "Connection"),
    }
}

fn connected(i: &Interface) -> bool {
    i.up && !(i.ipv4.is_empty() && i.ipv6.is_empty())
}

fn render(p: &gtk::Box, net: NetInfo) {
    if net.interfaces.is_empty() {
        p.append(&w::empty_state(
            "network-offline-symbolic",
            "No Network Connections",
            "Plug in a network cable or turn on Wi-Fi.",
            None,
        ));
        return;
    }
    let mut ifs = net.interfaces.clone();
    ifs.sort_by_key(|i| !connected(i));
    for i in &ifs {
        let (icon, kind) = kind_label(i);
        let on = connected(i);
        let mut rows = vec![
            w::list_row(
                icon,
                &format!("{kind} ({})", i.name),
                if on { "Connected" } else { "Not connected" },
                None,
            )
            .upcast(),
        ];
        if on {
            for a in &i.ipv4 {
                let (ip, mask) = split_cidr(a);
                rows.push(info("IP address", &ip));
                if !mask.is_empty() {
                    rows.push(info("Subnet mask", &mask));
                }
            }
            if let Some(g) = &net.gateway {
                rows.push(info("Router", g));
            }
            if !net.dns.is_empty() {
                rows.push(info("DNS servers", &net.dns.join(", ")));
            }
            if !net.search.is_empty() {
                rows.push(info("Search domains", &net.search.join(", ")));
            }
            for a in &i.ipv6 {
                rows.push(info("IPv6 address", a));
            }
        }
        if !i.mac.is_empty() {
            rows.push(info("Hardware address", &i.mac));
        }
        p.append(&w::section(kind, &rows));
    }
}

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("Network");
    let body = w::vbox(w::S6);
    p.append(&body);
    let m = e.machine();
    later(&body, move || m.net(), render);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_to_mask() {
        for (a, ip, mask) in [
            ("192.168.1.24/24", "192.168.1.24", "255.255.255.0"),
            ("10.0.0.2/8", "10.0.0.2", "255.0.0.0"),
            ("10.0.0.2/30", "10.0.0.2", "255.255.255.252"),
            ("10.0.0.2/0", "10.0.0.2", "0.0.0.0"),
            ("10.0.0.2", "10.0.0.2", ""),
            ("10.0.0.2/99", "10.0.0.2", ""),
        ] {
            assert_eq!(split_cidr(a), (ip.to_string(), mask.to_string()), "{a}");
        }
    }
}

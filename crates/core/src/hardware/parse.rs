//! Pure parsers for Linux device sources: sysfs attribute files, `lspci -mm`, `/proc/modules`,
//! `modules.dep`, `/proc/cpuinfo`, ACPI modaliases, `pacman -Qq`.
use std::collections::HashSet;

/// `0x8086\n` (sysfs id file) or `8086` -> id.
pub fn hex16(s: &str) -> Option<u16> {
    u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

/// sysfs PCI `class` (`0x028000`) -> six hex digits (`028000`).
pub fn pci_class(s: &str) -> Option<String> {
    let h = s.trim().trim_start_matches("0x");
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| h.to_ascii_lowercase())
}

/// Module names use `_`; sysfs and modprobe accept `-` too.
pub fn module(name: &str) -> String {
    name.trim().replace('-', "_")
}

/// `/proc/modules`: first word of each line.
pub fn proc_modules(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(module)
        .collect()
}

/// `modules.dep` / `modules.builtin`: `kernel/drivers/.../i915.ko.zst: deps` -> `i915`.
pub fn module_paths(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|l| {
            let path = l.split(':').next()?.trim();
            let file = path.rsplit('/').next()?;
            let stem = file.split(".ko").next()?;
            (!stem.is_empty()).then(|| module(stem))
        })
        .collect()
}

/// `lspci -mm` -> (slot `00:14.3`, device name). Slots lack the PCI domain.
pub fn lspci_names(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('"').collect();
            (f.len() > 6).then(|| (f[0].trim().to_owned(), f[5].to_owned()))
        })
        .collect()
}

/// sysfs PCI address `0000:00:14.3` -> `lspci -mm` slot `00:14.3` (domain 0 only).
pub fn lspci_slot(addr: &str) -> &str {
    addr.strip_prefix("0000:").unwrap_or(addr)
}

/// ACPI `modalias` (`acpi:ELAN0678:PNP0C50:`) -> ids (`_HID` then `_CID`s).
pub fn acpi_ids(modalias: &str) -> Vec<String> {
    modalias
        .trim()
        .strip_prefix("acpi:")
        .map(|r| {
            r.split(':')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// `/proc/cpuinfo` -> (vendor_id, model name) of the first processor.
pub fn cpuinfo(text: &str) -> (Option<String>, Option<String>) {
    let mut vendor = None;
    let mut model = None;
    for l in text.lines() {
        if l.trim().is_empty() && (vendor.is_some() || model.is_some()) {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            let v = v.trim();
            match k.trim() {
                "vendor_id" if vendor.is_none() => vendor = Some(v.to_owned()),
                "model name" if model.is_none() => model = Some(v.to_owned()),
                _ => {}
            }
        }
    }
    (vendor, model)
}

/// PCI vendor id for a cpuinfo `vendor_id`.
pub fn cpu_vendor(vendor_id: &str) -> Option<u16> {
    match vendor_id.trim() {
        "GenuineIntel" => Some(0x8086),
        "AuthenticAMD" | "HygonGenuine" => Some(0x1022),
        _ => None,
    }
}

pub fn lines(text: &str) -> HashSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;

    #[test]
    fn ids_and_classes() {
        assert_eq!(hex16("0x8086\n"), Some(0x8086));
        assert_eq!(hex16("51f0"), Some(0x51f0));
        assert_eq!(hex16("zz"), None);
        assert_eq!(pci_class("0x028000\n").as_deref(), Some("028000"));
        assert_eq!(pci_class("0x0280"), None);
        assert_eq!(module("snd-hda-intel"), "snd_hda_intel");
    }

    #[test]
    fn module_lists() {
        let m = proc_modules(PROC_MODULES);
        assert!(m.contains("i915") && m.contains("iwlwifi") && m.contains("snd_sof_pci_intel_tgl"));
        let d = module_paths(MODULES_DEP);
        assert!(d.contains("i915") && d.contains("rtw88_8822ce") && d.contains("nvidia_drm"));
        assert!(!d.contains(""));
    }

    #[test]
    fn lspci_mm_names() {
        let n = lspci_names(LSPCI_MM_ADL);
        assert_eq!(
            n[1],
            (
                "00:02.0".to_string(),
                "Alder Lake-UP3 GT2 [Iris Xe Graphics]".to_string()
            )
        );
        assert_eq!(lspci_slot("0000:00:14.3"), "00:14.3");
        assert_eq!(lspci_slot("0001:00:00.0"), "0001:00:00.0");
    }

    #[test]
    fn acpi_and_cpu() {
        assert_eq!(
            acpi_ids("acpi:ELAN0678:PNP0C50:\n"),
            ["ELAN0678", "PNP0C50"]
        );
        assert!(acpi_ids("of:foo").is_empty());
        let (v, m) = cpuinfo(CPUINFO_ADL);
        assert_eq!(v.as_deref(), Some("GenuineIntel"));
        assert_eq!(m.as_deref(), Some("12th Gen Intel(R) Core(TM) i5-1235U"));
        assert_eq!(cpu_vendor("AuthenticAMD"), Some(0x1022));
        assert_eq!(cpu_vendor("QEMU"), None);
    }
}

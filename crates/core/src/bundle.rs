//! Bundle target resolver: freebsd-native > linux > proton. Proton targets run on the native
//! FreeBSD wine-proton build, so they need no Linux ABI.
use shared::bundle::{Abi, Info, Target};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCaps {
    pub arch: String,
    pub linux_compat: bool,
    pub proton: bool,
}

fn supports(h: &HostCaps, abi: Abi) -> bool {
    match abi {
        Abi::FreeBsdNative => true,
        Abi::Linux => h.linux_compat,
        Abi::Proton => h.proton,
    }
}

pub fn resolve<'a>(info: &'a Info, host: &HostCaps) -> Option<&'a Target> {
    info.targets
        .iter()
        .filter(|t| t.arch == host.arch && supports(host, t.abi))
        .min_by_key(|t| t.abi)
}

/// FreeBSD `uname -m` name for the build arch.
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(abi: Abi, entry: &str) -> Target {
        Target {
            abi,
            arch: "amd64".into(),
            entry: entry.into(),
        }
    }

    #[test]
    fn picks_most_native_supported() {
        let (n, l, w) = (
            t(Abi::FreeBsdNative, "bin/app"),
            t(Abi::Linux, "linux/app"),
            t(Abi::Proton, "win/app.exe"),
        );
        let h = |arch: &str, lc, p| HostCaps {
            arch: arch.into(),
            linux_compat: lc,
            proton: p,
        };
        let cases = [
            (
                vec![w.clone(), l.clone(), n.clone()],
                h("amd64", true, true),
                Some(&n),
            ),
            (vec![w.clone(), l.clone()], h("amd64", true, true), Some(&l)),
            (vec![w.clone()], h("amd64", true, true), Some(&w)),
            // wine-proton is a native FreeBSD build: Windows targets need no Linux ABI.
            (vec![w.clone()], h("amd64", false, true), Some(&w)),
            (
                vec![l.clone(), w.clone()],
                h("amd64", false, true),
                Some(&w),
            ),
            (vec![w.clone()], h("amd64", true, false), None),
            (vec![l.clone()], h("amd64", false, false), None),
            (vec![n.clone()], h("arm64", true, true), None),
        ];
        for (targets, host, want) in cases {
            // Through serde: compiles whatever optional fields Info carries.
            let mut info: Info =
                serde_json::from_str(r#"{"id":"org.example.app","version":"1.0","targets":[]}"#)
                    .unwrap();
            info.targets = targets;
            assert_eq!(resolve(&info, &host), want, "{host:?}");
        }
    }
}

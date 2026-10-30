//! Who may use privileged compositor protocols (ADR 0013). Pure: identity + grants in, decision out.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    ShellLayer,
    WindowList,
    ScreenCapture,
    InputInjection,
    ClipboardHistory,
}

impl Capability {
    pub fn name(self) -> &'static str {
        match self {
            Capability::ShellLayer => "shell-layer",
            Capability::WindowList => "window-list",
            Capability::ScreenCapture => "screen-capture",
            Capability::InputInjection => "input-injection",
            Capability::ClipboardHistory => "clipboard-history",
        }
    }
}

/// What the compositor knows about a client process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The code actually running: executable path, or for .NET apps the main assembly path.
    pub code: String,
}

/// WolOS's own components live in root-owned `<libexec>/wol-*` folders. Callers pass the canonical
/// libexec path (it may be a symlink, e.g. /usr -> /work/local) and canonical identities.
pub const LIBEXEC: &str = "/usr/libexec";

/// Capabilities system components get without asking (the shell needs its layers and the window list).
const SYSTEM_DEFAULTS: [Capability; 2] = [Capability::ShellLayer, Capability::WindowList];

pub fn is_system(id: &Identity, libexec: &str) -> bool {
    id.code.starts_with(&format!("{libexec}/wol-")) && !id.code.split('/').any(|seg| seg == "..")
}

/// `grants`: (code, capability name) pairs from the consent database.
pub fn allowed(id: &Identity, cap: Capability, grants: &[(String, String)], libexec: &str) -> bool {
    id.code != XWAYLAND
        && ((is_system(id, libexec) && SYSTEM_DEFAULTS.contains(&cap))
            || grants
                .iter()
                .any(|(code, c)| *code == id.code && c == cap.name()))
}

/// Identity of every X11 client: they all share one Xwayland connection, so none can be told apart
/// and none is trusted (ADR 0025). Not a path, so it can never match a system component or a grant.
pub const XWAYLAND: &str = "xwayland";

pub fn xwayland() -> Identity {
    Identity {
        code: XWAYLAND.to_owned(),
    }
}

/// For .NET apps the process is `dotnet`; the identity is the assembly it runs (argv[1]).
pub fn identity_from(exe: &str, args: &[String]) -> Identity {
    let is_dotnet = exe.rsplit('/').next().is_some_and(|n| n == "dotnet");
    match (is_dotnet, args.get(1)) {
        (true, Some(dll)) if dll.ends_with(".dll") => Identity { code: dll.clone() },
        _ => Identity {
            code: exe.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(code: &str) -> Identity {
        Identity { code: code.into() }
    }

    #[test]
    fn shell_gets_layers_and_window_list_without_asking() {
        let shell = id("/usr/libexec/wol-shell/Wol.Shell.dll");
        assert!(allowed(&shell, Capability::ShellLayer, &[], LIBEXEC));
        assert!(allowed(&shell, Capability::WindowList, &[], LIBEXEC));
    }

    #[test]
    fn even_system_components_need_consent_to_record_the_screen() {
        let shell = id("/usr/libexec/wol-shell/Wol.Shell.dll");
        assert!(!allowed(&shell, Capability::ScreenCapture, &[], LIBEXEC));
    }

    #[test]
    fn ordinary_apps_get_nothing_privileged_by_default() {
        let app = id("/usr/bin/helium");
        for cap in [
            Capability::ShellLayer,
            Capability::WindowList,
            Capability::ScreenCapture,
            Capability::InputInjection,
            Capability::ClipboardHistory,
        ] {
            assert!(!allowed(&app, cap, &[], LIBEXEC), "{cap:?}");
        }
    }

    #[test]
    fn consent_grants_exactly_one_capability_to_exactly_one_app() {
        let grants = vec![("/usr/bin/obs".to_string(), "screen-capture".to_string())];
        assert!(allowed(
            &id("/usr/bin/obs"),
            Capability::ScreenCapture,
            &grants,
            LIBEXEC
        ));
        assert!(!allowed(
            &id("/usr/bin/obs"),
            Capability::InputInjection,
            &grants,
            LIBEXEC
        ));
        assert!(!allowed(
            &id("/usr/bin/helium"),
            Capability::ScreenCapture,
            &grants,
            LIBEXEC
        ));
    }

    #[test]
    fn path_tricks_do_not_make_an_app_system() {
        assert!(!is_system(
            &id("/usr/libexec/wol-shell/../../../home/eve/evil.dll"),
            LIBEXEC
        ));
        assert!(!is_system(
            &id("/home/eve/usr/libexec/wol-shell/x.dll"),
            LIBEXEC
        ));
    }

    #[test]
    fn symlinked_install_root_is_honoured_when_identities_are_canonical() {
        // /usr -> /work/local: the kernel reports /work/local/...; the caller passes the canonical libexec.
        let shell = id("/work/local/libexec/wol-shell/Wol.Shell.dll");
        assert!(allowed(
            &shell,
            Capability::ShellLayer,
            &[],
            "/work/local/libexec"
        ));
        assert!(!allowed(&shell, Capability::ShellLayer, &[], LIBEXEC));
    }

    #[test]
    fn dotnet_apps_are_identified_by_their_assembly() {
        let args = vec![
            "dotnet".to_string(),
            "/usr/libexec/wol-shell/Wol.Shell.dll".to_string(),
        ];
        assert_eq!(
            identity_from("/usr/share/dotnet/dotnet", &args).code,
            "/usr/libexec/wol-shell/Wol.Shell.dll"
        );
        assert_eq!(
            identity_from("/usr/bin/foot", &["foot".into()]).code,
            "/usr/bin/foot"
        );
    }
}

/// Consent database (`/etc/wolos/consent.json`): `{"grants":[{"app": "<canonical path>", "capability": "<name>"}]}`.
#[derive(serde::Deserialize, Default)]
pub struct Consent {
    #[serde(default)]
    pub grants: Vec<Grant>,
}

#[derive(serde::Deserialize, Clone)]
pub struct Grant {
    pub app: String,
    pub capability: String,
}

impl Consent {
    /// Unreadable or malformed files grant nothing (fail closed).
    pub fn parse(json: &str) -> Consent {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn pairs(&self) -> Vec<(String, String)> {
        self.grants
            .iter()
            .map(|g| (g.app.clone(), g.capability.clone()))
            .collect()
    }
}

#[cfg(test)]
mod consent_tests {
    use super::*;

    #[test]
    fn xwayland_gets_nothing_privileged_even_with_grants() {
        let x = xwayland();
        assert_eq!(x.code, XWAYLAND);
        let all = [
            Capability::ShellLayer,
            Capability::WindowList,
            Capability::ScreenCapture,
            Capability::InputInjection,
            Capability::ClipboardHistory,
        ];
        let grants: Vec<_> = all
            .iter()
            .map(|c| (XWAYLAND.to_string(), c.name().to_string()))
            .collect();
        for cap in all {
            assert!(!allowed(&x, cap, &grants, LIBEXEC), "{cap:?}");
        }
    }

    #[test]
    fn grants_parse_into_pairs() {
        let c =
            Consent::parse(r#"{"grants":[{"app":"/usr/bin/wlrctl","capability":"window-list"}]}"#);
        assert_eq!(
            c.pairs(),
            vec![("/usr/bin/wlrctl".to_string(), "window-list".to_string())]
        );
        let id = Identity {
            code: "/usr/bin/wlrctl".into(),
        };
        assert!(allowed(&id, Capability::WindowList, &c.pairs(), LIBEXEC));
        assert!(!allowed(
            &id,
            Capability::ScreenCapture,
            &c.pairs(),
            LIBEXEC
        ));
    }

    #[test]
    fn malformed_consent_fails_closed() {
        assert!(Consent::parse("{not json").pairs().is_empty());
        assert!(Consent::parse("").pairs().is_empty());
    }

    #[test]
    fn clipboard_history_needs_an_explicit_grant_even_for_system_code() {
        let shell = Identity {
            code: format!("{LIBEXEC}/wol-shell/Wol.Shell.dll"),
        };
        let paste = Identity {
            code: "/usr/bin/wl-paste".into(),
        };
        assert!(!allowed(&shell, Capability::ClipboardHistory, &[], LIBEXEC));
        assert!(!allowed(&paste, Capability::ClipboardHistory, &[], LIBEXEC));
        let grants = [(paste.code.clone(), "clipboard-history".to_owned())];
        assert!(allowed(
            &paste,
            Capability::ClipboardHistory,
            &grants,
            LIBEXEC
        ));
    }

    /// Screenshots (wol-screenshot, Super+Shift+3/4/5) run grim, and slurp for a region.
    /// The shipped consent grants exactly those tools exactly what they need, nothing else.
    #[test]
    fn shipped_consent_lets_the_screenshot_tools_work_and_nothing_more() {
        let c = Consent::parse(include_str!("../config/consent.json"));
        let g = c.pairs();
        let grim = Identity {
            code: "/usr/bin/grim".into(),
        };
        let slurp = Identity {
            code: "/usr/bin/slurp".into(),
        };
        assert!(allowed(&grim, Capability::ScreenCapture, &g, LIBEXEC));
        assert!(allowed(&slurp, Capability::ShellLayer, &g, LIBEXEC));
        for cap in [
            Capability::ShellLayer,
            Capability::WindowList,
            Capability::InputInjection,
            Capability::ClipboardHistory,
        ] {
            assert!(!allowed(&grim, cap, &g, LIBEXEC), "grim {cap:?}");
        }
        for cap in [
            Capability::ScreenCapture,
            Capability::WindowList,
            Capability::InputInjection,
            Capability::ClipboardHistory,
        ] {
            assert!(!allowed(&slurp, cap, &g, LIBEXEC), "slurp {cap:?}");
        }
    }
}

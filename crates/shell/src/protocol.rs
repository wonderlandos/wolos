//! Shell socket (port of Iara.Core.Shell.ShellCommand): one line `<verb> [panel]`, e.g. `toggle switcher`.
//! Path `$XDG_RUNTIME_DIR/wol-shell.sock`; sent by `wol shell ...` and keybinds. `approve <json>`
//! comes from `wol mcp` (helper approvals, `ui::approval`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Toggle,
    Show,
    Hide,
    Reload,
    /// `approve <json>`: a helper asks for an administrator change ([`shared::shell::Approval`]);
    /// the target is the JSON, verbatim. Never logged (a Wi-Fi password may be in it).
    Approve,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub verb: Verb,
    pub target: String,
}

pub const SOCKET: &str = "wol-shell.sock";

pub fn socket_path(runtime_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(runtime_dir).join(SOCKET)
}

pub fn parse(line: &str) -> Option<Command> {
    if let Some(json) = line.trim_start().strip_prefix("approve ") {
        let json = json.trim();
        return (!json.is_empty()).then(|| Command {
            verb: Verb::Approve,
            target: json.into(),
        });
    }
    let mut w = line.split_whitespace();
    let verb = match w.next()?.to_ascii_lowercase().as_str() {
        "toggle" => Verb::Toggle,
        "show" => Verb::Show,
        "hide" => Verb::Hide,
        "reload" => Verb::Reload,
        _ => return None,
    };
    let target = w.next().map(str::to_ascii_lowercase);
    if w.next().is_some() {
        return None;
    }
    match (verb, target) {
        (Verb::Reload, None) => Some(Command {
            verb,
            target: String::new(),
        }),
        (Verb::Reload, Some(_)) | (_, None) => None,
        (_, Some(target)) => Some(Command { verb, target }),
    }
}

/// New visibility for a panel.
pub fn apply(verb: Verb, visible: bool) -> bool {
    match verb {
        Verb::Show => true,
        Verb::Hide => false,
        _ => !visible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines() {
        assert_eq!(
            parse("toggle Switcher"),
            Some(Command {
                verb: Verb::Toggle,
                target: "switcher".into()
            })
        );
        assert_eq!(parse(" show notifications \n").unwrap().verb, Verb::Show);
        assert_eq!(parse("reload").unwrap().target, "");
        for bad in ["", "toggle", "reload x", "fly osd", "show a b"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    /// `approve <json>`: a helper's admin request for the approval sheet; JSON kept verbatim.
    #[test]
    fn approve_keeps_the_request_verbatim() {
        let j = r#"{"helper":"Claude Desktop","tool":"wifi join","request":{"action":"wifi-join","ssid":"My Home"}}"#;
        let c = parse(&format!("approve {j}\n")).unwrap();
        assert_eq!((c.verb, c.target.as_str()), (Verb::Approve, j));
        assert_eq!(parse("approve"), None);
        assert_eq!(parse("approve   "), None);
    }

    #[test]
    fn applies_visibility() {
        assert!(apply(Verb::Toggle, false));
        assert!(!apply(Verb::Toggle, true));
        assert!(apply(Verb::Show, true));
        assert!(!apply(Verb::Hide, true));
    }
}

//! Helper gate (archicture/modules/automation.md), one for `wol mcp` and `WOL_HELPER` scripts:
//! policy first; reads run; a change asks on screen (wol-shell's approval sheet) unless the user
//! chose Always Allow for that helper; administrator changes ask for the password every time and
//! run in the shell, never in the helper. No session showing = refused. The caller writes the one
//! audit record per call.
use crate::catalog::{self, Access, HelperVerb};
use crate::cli::Invocation;
use crate::run::{Env, SHELL_SOCKET};
use shared::shell::{APPROVAL_TIMEOUT_SECS, APPROVAL_WAIT_SECS, Answer, Approval, ApprovalRequest};
use std::path::Path;
use wol_core::automation::{Outcome, Policy, policy};

pub const NO_SESSION: &str = "No WolOS session is showing (wol-shell is not running), so nobody \
can approve this change on screen. Nothing changed.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Run the verb here. `approved`: the user said yes on screen; `note` goes to the audit log.
    Run { approved: bool, note: &'static str },
    /// The shell ran the administrator change after the password; the sentence of what changed.
    Done(String),
    /// Nothing ran: the audit outcome and the message for the helper.
    Stop(Outcome, String),
}

/// Decides one helper call. `helper` is the policy and audit name (`Claude Desktop`,
/// `script:backup`); `secret` is the stdin secret of the verb (a Wi-Fi password).
pub fn decide(
    env: &mut dyn Env,
    helper: &str,
    tool: &str,
    rule: &HelperVerb,
    inv: &Invocation,
    secret: Option<String>,
) -> Verdict {
    let pol = Policy::parse(
        env.read_file(&env.config_home().join(policy::FILE))
            .as_deref(),
    );
    if let Err(r) = pol.check(rule.capability) {
        return Verdict::Stop(Outcome::Refused, r.to_string());
    }
    if !catalog::changes(inv) {
        return Verdict::Run {
            approved: false,
            note: "",
        };
    }
    let admin = rule.access == Access::Admin;
    if !admin && pol.always_allows(helper, rule.capability) {
        return Verdict::Run {
            approved: false,
            note: "always allowed",
        };
    }
    let request = if admin {
        match catalog::approval(inv, secret) {
            Ok(r) => r,
            Err(e) => return Verdict::Stop(Outcome::Failed, e),
        }
    } else {
        ApprovalRequest::Change {
            capability: rule.capability.into(),
            what: catalog::describe(inv),
        }
    };
    let what = request.describe();
    let a = Approval {
        helper: helper.into(),
        tool: tool.into(),
        request,
    };
    let Some(rt) = env.var("XDG_RUNTIME_DIR") else {
        return Verdict::Stop(Outcome::Refused, NO_SESSION.into());
    };
    let sock = Path::new(&rt).join(SHELL_SOCKET);
    let answer = match env.request_wait(&sock, a.line().as_bytes(), APPROVAL_WAIT_SECS) {
        Ok(reply) => Answer::parse(&reply),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Some(Answer::TimedOut)
        }
        Err(_) => return Verdict::Stop(Outcome::Refused, NO_SESSION.into()),
    };
    let nothing = "Nothing changed.";
    match (answer, admin) {
        (Some(Answer::AllowOnce), false) => Verdict::Run {
            approved: true,
            note: "allowed once",
        },
        (Some(Answer::AllowAlways), false) => Verdict::Run {
            approved: true,
            note: "always allowed from now on",
        },
        (Some(Answer::Done), true) => Verdict::Done(what),
        (Some(Answer::Failed { message }), true) => Verdict::Stop(
            Outcome::Failed,
            format!("The user allowed it, but it did not work: {message}. {nothing}"),
        ),
        (Some(Answer::Deny), _) => Verdict::Stop(
            Outcome::Declined,
            format!("The user declined: {what}. {nothing}"),
        ),
        (Some(Answer::TimedOut), _) => Verdict::Stop(
            Outcome::Declined,
            format!(
                "No answer on screen within {APPROVAL_TIMEOUT_SECS} seconds, so the request to {what} was turned down. {nothing}"
            ),
        ),
        (Some(Answer::Busy), _) => Verdict::Stop(
            Outcome::Refused,
            format!(
                "Too many requests are already waiting on screen. {nothing} Ask again after the user answers them."
            ),
        ),
        // No answer line (an older shell), or an answer that does not fit the request.
        _ => Verdict::Stop(
            Outcome::Refused,
            format!("WolOS could not ask on screen. {nothing}"),
        ),
    }
}

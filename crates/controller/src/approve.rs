// SPDX-License-Identifier: GPL-2.0-or-later
//! Helper gate (docs/modules/automation.md), one for `wol mcp` and `WOL_HELPER` scripts:
//! policy first; reads run; a change asks on screen (wol-shell's approval sheet) unless the user
//! chose Always Allow for that helper; administrator changes ask for the password every time and
//! run in the shell, never in the helper. No session showing = refused. The caller writes the one
//! audit record per call.
use crate::catalog::{self, Access, HelperVerb};
use crate::cli::Invocation;
use crate::run::{Env, SHELL_SOCKET};
use shared::patience::Patience;
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
    // C10-1: once-only changes ask every time, even with an `apps` grant from `open`.
    let once_only = policy::once_only(tool);
    if !admin && pol.always_allows_tool(helper, rule.capability, tool) {
        return Verdict::Run {
            approved: false,
            note: "always allowed",
        };
    }
    let request = if admin {
        match catalog::approval_in(env, inv, secret) {
            Ok(Some(r)) => r,
            // A Doctor fix of the person's own services: the consent sheet, every time, and the
            // verb runs here after Allow (it sends no wol-admind request for a helper).
            Ok(None) => {
                let user = HelperVerb {
                    access: Access::Change,
                    ..*rule
                };
                return decide(env, helper, tool, &user, inv, None);
            }
            Err(e) => return Verdict::Stop(Outcome::Failed, e),
        }
    } else {
        ApprovalRequest::Change {
            capability: rule.capability.into(),
            what: catalog::describe(inv),
        }
    };
    let what = request.describe();
    // Settings > Accessibility > Timeouts: the sheet stays open that much longer, so the wait for
    // its answer does too (the shell and wold read the same file).
    let patience = env.patience();
    let a = Approval {
        helper: helper.into(),
        tool: tool.into(),
        request,
    };
    let answer = if admin {
        // R2C-6: an administrator change is asked through wold, which shows it to this account's
        // shell naming this process by its kernel executable; the shell runs it after the
        // password. The shell takes administrator sheets from no same-account process directly.
        match via_wold(env, a, patience) {
            Ok(a) => a,
            Err(stop) => return stop,
        }
    } else {
        match on_shell_socket(env, &a, patience) {
            Ok(a) => a,
            Err(stop) => return stop,
        }
    };
    let nothing = "Nothing changed.";
    match (answer, admin) {
        (Some(Answer::AllowOnce), false) => Verdict::Run {
            approved: true,
            note: "allowed once",
        },
        // A shell that offered Always Allow anyway: it counts once (nothing honours the grant).
        (Some(Answer::AllowAlways), false) if once_only => Verdict::Run {
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
                "No answer on screen within {} seconds, so the request to {what} was turned down. {nothing}",
                patience.secs(APPROVAL_TIMEOUT_SECS)
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

/// wold's control socket (`WOL_JOBS_SOCKET` in tests), as `crate::run` reaches it.
fn wold_socket(env: &dyn Env) -> std::path::PathBuf {
    env.var("WOL_JOBS_SOCKET")
        .map_or_else(|| shared::jobs::SOCKET.into(), std::path::PathBuf::from)
}

/// A user-level consent sheet: straight to this account's shell (`approve` on its socket).
fn on_shell_socket(
    env: &mut dyn Env,
    a: &Approval,
    patience: Patience,
) -> Result<Option<Answer>, Verdict> {
    let Some(rt) = env.var("XDG_RUNTIME_DIR") else {
        return Err(Verdict::Stop(Outcome::Refused, NO_SESSION.into()));
    };
    let sock = Path::new(&rt).join(SHELL_SOCKET);
    let wait = approval_wait(patience);
    match env.request_wait(&sock, a.line().as_bytes(), wait) {
        Ok(reply) => Ok(Answer::parse(&reply)),
        Err(e) if timed_out(&e) => Ok(Some(Answer::TimedOut)),
        Err(_) => Err(Verdict::Stop(Outcome::Refused, NO_SESSION.into())),
    }
}

/// How long a requester waits for the sheet's answer: the sheet's own time, stretched by the
/// account's Timeouts setting, and the ten seconds for the answer to arrive.
pub fn approval_wait(patience: Patience) -> u32 {
    patience
        .secs(APPROVAL_TIMEOUT_SECS)
        .saturating_add(APPROVAL_WAIT_SECS - APPROVAL_TIMEOUT_SECS)
}

fn timed_out(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// How long wold may take to answer `call` (`api::packages::ask_wait`): the sheet, then the
/// package steps it runs at wol-admind's long deadline (Update Now, a Doctor fix).
fn sheet_wait(call: &shared::portal::Call, patience: Patience) -> u32 {
    use wol_core::admin::{LONG_STEP_SECS, REPLY_MARGIN_SECS};
    let shared::portal::Call::SessionApprove { approval } = call else {
        return approval_wait(patience);
    };
    let long = match &approval.request {
        ApprovalRequest::Packages { names, .. } => names.len().max(1),
        ApprovalRequest::DoctorFix { steps, .. } => steps.len().max(1),
        _ => 0,
    };
    match u32::try_from(long) {
        Ok(0) => approval_wait(patience),
        Ok(n) => approval_wait(patience)
            .saturating_add(n.saturating_mul(LONG_STEP_SECS))
            .saturating_add(REPLY_MARGIN_SECS),
        Err(_) => u32::MAX,
    }
}

/// An administrator sheet through wold (`session.approve`): the sheet's answer, as the shell gave
/// it to wold. wold's own refusals map to the same outcomes as the shell's.
fn via_wold(env: &mut dyn Env, a: Approval, patience: Patience) -> Result<Option<Answer>, Verdict> {
    use shared::portal::{Answer as Reply, BUSY, Call, NO_ANSWER, NOT_RUNNING};
    // No session in this environment: nobody could see a sheet.
    if env.var("XDG_RUNTIME_DIR").is_none() {
        return Err(Verdict::Stop(Outcome::Refused, NO_SESSION.into()));
    }
    let call = Call::SessionApprove { approval: a };
    if let Err(e) = call.validate() {
        return Err(Verdict::Stop(Outcome::Failed, e));
    }
    let line = format!("call {}\n", call.encode());
    let sock = wold_socket(env);
    // wold waits APPROVAL_WAIT_SECS for the shell, plus wol-admind's time for what the sheet runs;
    // give its answer a moment more to arrive.
    let reply = match env.request_wait(&sock, line.as_bytes(), sheet_wait(&call, patience) + 5) {
        Ok(r) => r,
        Err(e) if timed_out(&e) => return Ok(Some(Answer::TimedOut)),
        Err(_) => return Err(Verdict::Stop(Outcome::Refused, NO_SESSION.into())),
    };
    // Audit S4-12: wold's own reason (permission, validation, a newer wold) reaches the helper.
    let cannot = |why: &str| {
        Verdict::Stop(
            Outcome::Refused,
            format!("WolOS could not ask on screen ({why}). Nothing changed."),
        )
    };
    let Ok(r) = serde_json::from_str::<Reply>(reply.trim()) else {
        return Err(cannot("wold's answer could not be read"));
    };
    if r.ok {
        return match r.data.map(serde_json::from_value::<Answer>) {
            Some(Ok(a)) => Ok(Some(a)),
            _ => Err(cannot("wold's answer could not be read")),
        };
    }
    match r.message.as_deref() {
        Some(BUSY) => Ok(Some(Answer::Busy)),
        Some(NO_ANSWER) => Ok(Some(Answer::TimedOut)),
        Some(NOT_RUNNING) => Err(Verdict::Stop(Outcome::Refused, NO_SESSION.into())),
        Some(m) if !m.trim().is_empty() => Err(cannot(&reason(m))),
        _ => Err(cannot("wold gave no reason")),
    }
}

/// wold's refusal text for the helper: one line, control characters dropped, at most 160 chars.
fn reason(m: &str) -> String {
    m.chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C10-1: every state-changing `apps` verb besides plain `open` is once-only, so an Always
    /// Allow given on an `open` sheet never covers it.
    #[test]
    fn apps_changes_besides_open_are_once_only() {
        for v in catalog::HELPER_VERBS {
            let tool = v.path.join("_");
            if v.capability == "apps" && v.access == Access::Change {
                assert_eq!(policy::once_only(&tool), tool != "open", "{tool}");
            }
        }
        for t in policy::ONCE_ONLY {
            assert!(
                catalog::HELPER_VERBS.iter().any(|v| v.path.join("_") == t),
                "{t} is not a helper verb"
            );
        }
    }
}

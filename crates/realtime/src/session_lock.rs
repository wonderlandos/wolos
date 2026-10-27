//! Session lock policy (ext-session-lock-v1, ADR 0022). Pure: events in, phase out.
//!
//! Unlocked -> Locking (a locker asked) -> Locked (a cleared frame was shown) -> Unlocked
//! (only the owning locker's verified unlock). A dead locker never unlocks: the session stays
//! Locked with no owner (solid fallback) until a new locker takes over.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase<O> {
    Unlocked,
    /// Screen already hidden; waiting for one cleared frame before confirming `locked`.
    Locking {
        owner: O,
    },
    /// `owner: None` = the locker died; show the fallback and wait for a new locker.
    Locked {
        owner: Option<O>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy<O> {
    phase: Phase<O>,
}

impl<O> Default for Policy<O> {
    fn default() -> Self {
        Self {
            phase: Phase::Unlocked,
        }
    }
}

/// What the compositor must tell a locker that asked to lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockReply {
    /// Send `finished`: another locker holds the session.
    Refuse,
    /// Hide everything; confirm `locked` after the next drawn frame.
    ConfirmAfterFrame,
    /// Session is already hidden (abandoned lock): confirm `locked` now.
    ConfirmNow,
}

impl<O: Clone + PartialEq> Policy<O> {
    pub fn phase(&self) -> &Phase<O> {
        &self.phase
    }

    /// True whenever client content must be hidden and input withheld.
    pub fn is_locked(&self) -> bool {
        !matches!(self.phase, Phase::Unlocked)
    }

    pub fn owner(&self) -> Option<&O> {
        match &self.phase {
            Phase::Locking { owner } | Phase::Locked { owner: Some(owner) } => Some(owner),
            _ => None,
        }
    }

    pub fn request_lock(&mut self, who: O) -> LockReply {
        match self.phase {
            Phase::Unlocked => {
                self.phase = Phase::Locking { owner: who };
                LockReply::ConfirmAfterFrame
            }
            Phase::Locked { owner: None } => {
                self.phase = Phase::Locked { owner: Some(who) };
                LockReply::ConfirmNow
            }
            _ => LockReply::Refuse,
        }
    }

    /// A frame without client content reached the screen. Returns true to confirm `locked`.
    pub fn frame_drawn(&mut self) -> bool {
        let Phase::Locking { owner } = &self.phase else {
            return false;
        };
        self.phase = Phase::Locked {
            owner: Some(owner.clone()),
        };
        true
    }

    /// The owning locker disconnected or was killed.
    pub fn owner_gone(&mut self) {
        if self.is_locked() {
            self.phase = Phase::Locked { owner: None };
        }
    }

    /// `unlock_and_destroy` arrived. `verified`: it came from the owner and the owner is still
    /// connected. Returns true when the session is now unlocked.
    pub fn unlock(&mut self, verified: bool) -> bool {
        if verified && matches!(self.phase, Phase::Locked { owner: Some(_) }) {
            self.phase = Phase::Unlocked;
            return true;
        }
        false
    }

    /// Lock surfaces are shown only from the owning locker.
    pub fn accepts_surface_from(&self, who: &O) -> bool {
        self.owner() == Some(who)
    }

    /// Compositor shortcuts that would end the session (and so drop the lock) are off while locked.
    /// While locked, shortcuts may only run hardware keys (`wol sys set ...`: volume, brightness).
    pub fn allows_shortcut(&self, argv: &[String]) -> bool {
        !self.is_locked() || argv.starts_with(&["wol".into(), "sys".into(), "set".into()])
    }

    pub fn allows_session_exit(&self) -> bool {
        !self.is_locked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u32 = 1;
    const B: u32 = 2;

    fn locked_by(o: u32) -> Policy<u32> {
        let mut p = Policy::default();
        assert_eq!(p.request_lock(o), LockReply::ConfirmAfterFrame);
        assert!(p.frame_drawn());
        p
    }

    #[test]
    fn starts_unlocked_and_shows_everything() {
        let p = Policy::<u32>::default();
        assert!(!p.is_locked());
        assert!(p.allows_session_exit());
        assert_eq!(p.owner(), None);
    }

    #[test]
    fn lock_hides_immediately_and_confirms_after_one_frame() {
        let mut p = Policy::default();
        assert_eq!(p.request_lock(A), LockReply::ConfirmAfterFrame);
        assert!(
            p.is_locked(),
            "content hidden before the client hears `locked`"
        );
        assert_eq!(p.phase(), &Phase::Locking { owner: A });
        assert!(p.frame_drawn());
        assert_eq!(p.phase(), &Phase::Locked { owner: Some(A) });
        assert!(!p.frame_drawn(), "confirm only once");
    }

    #[test]
    fn verified_unlock_from_owner_unlocks() {
        let mut p = locked_by(A);
        assert!(p.unlock(true));
        assert_eq!(p.phase(), &Phase::Unlocked);
    }

    #[test]
    fn unverified_unlock_is_ignored() {
        let mut p = locked_by(A);
        assert!(!p.unlock(false));
        assert_eq!(p.phase(), &Phase::Locked { owner: Some(A) });
    }

    #[test]
    fn unlock_before_confirmed_lock_keeps_locked() {
        let mut p = Policy::default();
        p.request_lock(A);
        assert!(!p.unlock(true));
        assert!(p.is_locked());
    }

    #[test]
    fn second_locker_is_refused_while_owned() {
        for mut p in [locked_by(A), {
            let mut p = Policy::default();
            p.request_lock(A);
            p
        }] {
            assert_eq!(p.request_lock(B), LockReply::Refuse);
            assert_eq!(p.owner(), Some(&A));
            assert!(!p.accepts_surface_from(&B));
            assert!(p.accepts_surface_from(&A));
        }
    }

    #[test]
    fn locker_death_keeps_session_locked_without_owner() {
        for mut p in [locked_by(A), {
            let mut p = Policy::default();
            p.request_lock(A);
            p
        }] {
            p.owner_gone();
            assert_eq!(p.phase(), &Phase::Locked { owner: None });
            assert!(p.is_locked());
            assert!(!p.unlock(true), "nobody can unlock an abandoned lock");
            assert!(!p.accepts_surface_from(&A));
        }
    }

    #[test]
    fn new_locker_takes_over_abandoned_lock_and_can_unlock() {
        let mut p = locked_by(A);
        p.owner_gone();
        assert_eq!(p.request_lock(B), LockReply::ConfirmNow);
        assert_eq!(p.phase(), &Phase::Locked { owner: Some(B) });
        assert!(p.accepts_surface_from(&B));
        assert!(p.unlock(true));
        assert!(!p.is_locked());
    }

    #[test]
    fn owner_gone_while_unlocked_changes_nothing() {
        let mut p = Policy::<u32>::default();
        p.owner_gone();
        assert_eq!(p.phase(), &Phase::Unlocked);
        assert!(!p.unlock(true));
    }

    #[test]
    fn session_exit_shortcut_is_off_while_locked() {
        let mut p = locked_by(A);
        assert!(!p.allows_session_exit());
        p.owner_gone();
        assert!(!p.allows_session_exit());
    }

    #[test]
    fn only_hardware_key_shortcuts_run_while_locked() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cases: [(&[&str], bool, bool); 5] = [
            (&["wol", "sys", "set", "volume", "+5"], true, true),
            (&["wol-lock"], true, false),
            (&["wol", "sys"], true, false),
            (&["wol", "open", "Terminal"], true, false),
            (&[], true, false),
        ];
        let locked = locked_by(A);
        let unlocked = Policy::<u32>::default();
        for (argv, when_unlocked, when_locked) in cases {
            assert_eq!(
                unlocked.allows_shortcut(&v(argv)),
                when_unlocked,
                "{argv:?}"
            );
            assert_eq!(locked.allows_shortcut(&v(argv)), when_locked, "{argv:?}");
        }
    }
}

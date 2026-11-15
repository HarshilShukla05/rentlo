//! A pair of profiles that were shown to each other, and what each said.

use crate::ids::{MatchId, ProfileId};

string_enum!(Decision {
    Yes => "yes",
    Skip => "skip",
});

string_enum!(MatchStatus {
    /// At least one side has not answered, and nobody said skip.
    Suggested => "suggested",
    /// Both said yes. Each may now see the other's contact.
    Mutual => "mutual",
    /// One side said skip. Final.
    Declined => "declined",
});

/// One pair. Stored once: `a` is always the smaller profile id, so the same
/// two people can never have two rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub id: MatchId,
    pub a: ProfileId,
    pub b: ProfileId,
    pub score: u8,
    pub decision_a: Option<Decision>,
    pub decision_b: Option<Decision>,
}

/// What a decision changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decided {
    /// Recorded. The other side has not answered yet.
    Waiting,
    /// This decision completed a mutual yes.
    Mutual,
    /// The pair is closed.
    Declined,
    /// This side had already answered, or the pair was closed. Nothing changed.
    Unchanged,
}

/// The two ids in storage order.
pub fn ordered(x: ProfileId, y: ProfileId) -> (ProfileId, ProfileId) {
    if x <= y { (x, y) } else { (y, x) }
}

impl Match {
    pub fn status(&self) -> MatchStatus {
        match (self.decision_a, self.decision_b) {
            (Some(Decision::Skip), _) | (_, Some(Decision::Skip)) => MatchStatus::Declined,
            (Some(Decision::Yes), Some(Decision::Yes)) => MatchStatus::Mutual,
            _ => MatchStatus::Suggested,
        }
    }

    pub fn involves(&self, profile: ProfileId) -> bool {
        self.a == profile || self.b == profile
    }

    /// The other profile of the pair, seen from `profile`.
    pub fn other(&self, profile: ProfileId) -> ProfileId {
        if self.a == profile { self.b } else { self.a }
    }

    pub fn decision_of(&self, profile: ProfileId) -> Option<Decision> {
        if self.a == profile { self.decision_a } else { self.decision_b }
    }

    /// Records `profile`'s answer. A first answer is final: a later one for
    /// the same side changes nothing, so a repeated message cannot flip it.
    pub fn decide(&mut self, profile: ProfileId, decision: Decision) -> Decided {
        if !self.involves(profile) || self.decision_of(profile).is_some() || self.status() != MatchStatus::Suggested {
            return Decided::Unchanged;
        }
        if self.a == profile {
            self.decision_a = Some(decision);
        } else {
            self.decision_b = Some(decision);
        }
        match self.status() {
            MatchStatus::Mutual => Decided::Mutual,
            MatchStatus::Declined => Decided::Declined,
            MatchStatus::Suggested => Decided::Waiting,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn pair() -> (Match, ProfileId, ProfileId) {
        let (a, b) = (ProfileId(Uuid::from_u128(1)), ProfileId(Uuid::from_u128(2)));
        let m = Match { id: MatchId(Uuid::from_u128(9)), a, b, score: 80, decision_a: None, decision_b: None };
        (m, a, b)
    }

    #[test]
    fn two_yeses_make_it_mutual() {
        let (mut m, a, b) = pair();
        assert_eq!(m.decide(a, Decision::Yes), Decided::Waiting);
        assert_eq!(m.status(), MatchStatus::Suggested);
        assert_eq!(m.decide(b, Decision::Yes), Decided::Mutual);
        assert_eq!(m.status(), MatchStatus::Mutual);
    }

    #[test]
    fn one_skip_closes_the_pair_for_good() {
        let (mut m, a, b) = pair();
        assert_eq!(m.decide(a, Decision::Skip), Decided::Declined);
        assert_eq!(m.decide(b, Decision::Yes), Decided::Unchanged);
        assert_eq!(m.decision_of(b), None);
        assert_eq!(m.status(), MatchStatus::Declined);
    }

    #[test]
    fn a_first_answer_cannot_be_changed_or_repeated() {
        let (mut m, a, _) = pair();
        m.decide(a, Decision::Yes);
        assert_eq!(m.decide(a, Decision::Skip), Decided::Unchanged);
        assert_eq!(m.decide(a, Decision::Yes), Decided::Unchanged);
        assert_eq!(m.decision_of(a), Some(Decision::Yes));
    }

    #[test]
    fn an_outsider_cannot_decide() {
        let (mut m, a, _) = pair();
        let stranger = ProfileId(Uuid::from_u128(77));
        assert_eq!(m.decide(stranger, Decision::Yes), Decided::Unchanged);
        assert_eq!(m.other(a), m.b);
        assert_eq!(ordered(m.b, m.a), (m.a, m.b));
    }
}

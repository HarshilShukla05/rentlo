//! Where a chat stands, and what the bot should ask next.
//!
//! The state is small and saved after every message, so the bot can restart
//! at any point and pick the chat up where it was.

use crate::ids::{MatchId, PublicCode};
use crate::profile::{Field, Gender, GenderPref, ProfileData, ProfilePatch};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

string_enum!(Stage {
    /// Has not written before.
    New => "new",
    /// Giving the details a profile needs.
    Collecting => "collecting",
    /// Profile is live and being matched.
    Live => "live",
    /// Profile is hidden at the person's request.
    Paused => "paused",
});

/// A question is asked this many times. After that a field with a safe
/// default takes it, so nobody is stuck in a loop over an optional answer.
pub const MAX_ASKS: u8 = 2;

/// A profile with fewer stated habits than this gets one prompt for more.
const ENOUGH_HABITS: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conversation {
    pub stage: Stage,
    /// Everything the person has said about themselves so far.
    pub draft: ProfilePatch,
    /// The one field the last message asked for, so a bare "2" can be read.
    pub asked: Option<Field>,
    /// How often each field has been asked.
    pub asks: BTreeMap<Field, u8>,
    pub habits_asked: bool,
    /// The match cards last shown, in order: "1 yes" means the first.
    pub shown: Vec<MatchId>,
    /// Set by "delete"; the next "delete yes" erases the account.
    pub delete_armed: bool,
    /// A profile the person asked to meet before their own was live. Acted
    /// on the moment it goes live.
    pub interest: Option<PublicCode>,
}

impl Default for Conversation {
    fn default() -> Self {
        Self {
            stage: Stage::New,
            draft: ProfilePatch::default(),
            asked: None,
            asks: BTreeMap::new(),
            habits_asked: false,
            shown: Vec::new(),
            delete_armed: false,
            interest: None,
        }
    }
}

/// What the bot does after reading a message.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Ask for these fields, in this order, in one message.
    Ask(Vec<Field>),
    /// Everything required is known. Ask once how the person lives.
    AskHabits,
    /// The profile can go live.
    Ready(ProfileData),
}

impl Conversation {
    /// Takes in what a message said and decides the next step.
    pub fn absorb(&mut self, patch: ProfilePatch) -> Step {
        self.draft.merge(patch);
        self.asked = None;
        self.apply_defaults();

        let missing = self.draft.missing();
        if let Some(first) = missing.first().copied() {
            // City and budget are both free text and read well from one
            // reply ("pune, 15k"). A field with numbered options is asked
            // alone, so its numbers cannot be mistaken for another's.
            let fields = if first == Field::City && missing.contains(&Field::Budget) {
                vec![Field::City, Field::Budget]
            } else {
                vec![first]
            };
            for field in &fields {
                let count = self.asks.entry(*field).or_insert(0);
                *count = count.saturating_add(1);
            }
            self.asked = Some(first);
            return Step::Ask(fields);
        }

        if !self.habits_asked && self.draft.lifestyle.known() < ENOUGH_HABITS {
            self.habits_asked = true;
            return Step::AskHabits;
        }

        match self.draft.complete() {
            Ok(profile) => Step::Ready(profile),
            // `missing` was empty, so this cannot happen; ask again rather
            // than trust that.
            Err(fields) => Step::Ask(fields),
        }
    }

    /// Fills a field that was asked [`MAX_ASKS`] times without an answer,
    /// where a default harms nobody: an unstated gender, no preference.
    fn apply_defaults(&mut self) {
        let exhausted = |asks: &BTreeMap<Field, u8>, field| asks.get(&field).copied().unwrap_or(0) >= MAX_ASKS;
        if self.draft.gender.is_none() && exhausted(&self.asks, Field::Gender) {
            self.draft.gender = Some(Gender::Other);
        }
        if self.draft.gender_pref.is_none() && exhausted(&self.asks, Field::GenderPref) {
            self.draft.gender_pref = Some(GenderPref::Any);
        }
    }

    /// How many times `field` has been asked, this message included.
    pub fn times_asked(&self, field: Field) -> u8 {
        self.asks.get(&field).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::extract;
    use crate::profile::{Food, Intent};

    /// Feeds a reply through the rule extractor, as the bot does.
    fn say(chat: &mut Conversation, text: &str) -> Step {
        let patch = extract(text, chat.asked);
        chat.absorb(patch)
    }

    #[test]
    fn a_full_first_message_needs_only_the_missing_fields() {
        let mut chat = Conversation::default();
        let step = say(&mut chat, "need a room in koramangala blr, 18k max, sleep by 11, veg, no smoking, clean freak");
        assert_eq!(step, Step::Ask(vec![Field::Gender]));
        assert_eq!(say(&mut chat, "1"), Step::Ask(vec![Field::GenderPref]));
        let Step::Ready(profile) = say(&mut chat, "1") else { panic!("should be ready") };
        assert_eq!(profile.intent, Intent::NeedsRoom);
        assert_eq!(profile.budget.max(), 18_000);
        assert_eq!(profile.lifestyle.food, Some(Food::Veg));
    }

    #[test]
    fn numbered_replies_alone_complete_a_profile() {
        let mut chat = Conversation::default();
        assert_eq!(say(&mut chat, "hello there"), Step::Ask(vec![Field::Intent]));
        assert_eq!(say(&mut chat, "1"), Step::Ask(vec![Field::City, Field::Budget]));
        assert_eq!(say(&mut chat, "Jhansi 12k"), Step::Ask(vec![Field::Gender]));
        assert_eq!(say(&mut chat, "2"), Step::Ask(vec![Field::GenderPref]));
        assert_eq!(say(&mut chat, "2"), Step::AskHabits);
        let Step::Ready(profile) = say(&mut chat, "skip") else { panic!("should be ready") };
        assert_eq!(profile.intent, Intent::HasRoom);
        assert_eq!(profile.city.as_str(), "Jhansi");
        assert_eq!((profile.budget.min(), profile.budget.max()), (12_000, 12_000));
        assert_eq!(profile.gender, Gender::Female);
        assert_eq!(profile.gender_pref, GenderPref::Same);
    }

    #[test]
    fn city_and_budget_are_asked_together_but_answered_in_any_order() {
        let mut chat = Conversation::default();
        say(&mut chat, "hi");
        say(&mut chat, "2");
        assert_eq!(say(&mut chat, "20000"), Step::Ask(vec![Field::City]));
        assert_eq!(say(&mut chat, "Pune"), Step::Ask(vec![Field::Gender]));
    }

    #[test]
    fn habits_are_asked_once_and_never_block() {
        let mut chat = Conversation::default();
        let step = say(&mut chat, "have a room in pune, 15k, male, any gender");
        assert_eq!(step, Step::AskHabits);
        assert!(matches!(say(&mut chat, "whatever"), Step::Ready(_)));
        assert!(matches!(say(&mut chat, "ok"), Step::Ready(_)), "not asked again");
    }

    #[test]
    fn an_optional_answer_falls_back_to_a_default_after_two_asks() {
        let mut chat = Conversation::default();
        say(&mut chat, "need a room in pune, 15k, veg, tidy, non smoker");
        assert_eq!(chat.asked, Some(Field::Gender));
        assert_eq!(say(&mut chat, "why do you ask"), Step::Ask(vec![Field::Gender]));
        assert_eq!(chat.times_asked(Field::Gender), 2);
        // Third non-answer: gender defaults, the next question is asked.
        assert_eq!(say(&mut chat, "none of your business"), Step::Ask(vec![Field::GenderPref]));
        assert_eq!(chat.draft.gender, Some(Gender::Other));
        say(&mut chat, "hmm");
        let Step::Ready(profile) = say(&mut chat, "hmm") else { panic!("should be ready") };
        assert_eq!(profile.gender_pref, GenderPref::Any);
    }

    #[test]
    fn required_answers_have_no_default() {
        let mut chat = Conversation::default();
        for _ in 0..5 {
            assert_eq!(say(&mut chat, "..."), Step::Ask(vec![Field::Intent]));
        }
    }

    #[test]
    fn a_live_profile_takes_edits() {
        let mut chat = Conversation::default();
        say(&mut chat, "need a room in pune, 15k, male, any gender, veg, tidy, non smoker");
        let Step::Ready(edited) = say(&mut chat, "budget 20k now") else { panic!("should be ready") };
        assert_eq!(edited.budget.max(), 20_000);
        assert_eq!(edited.city.as_str(), "Pune");
    }

    #[test]
    fn the_state_survives_storage() {
        let mut chat = Conversation::default();
        say(&mut chat, "need a room in pune");
        let json = serde_json::to_string(&chat).unwrap();
        assert_eq!(serde_json::from_str::<Conversation>(&json).unwrap(), chat);
        // A state saved by an older version, missing newer fields, still loads.
        let old: Conversation = serde_json::from_str(r#"{"stage":"live"}"#).unwrap();
        assert_eq!(old.stage, Stage::Live);
    }
}

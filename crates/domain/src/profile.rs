//! What a person tells Rentlo about themselves.
//!
//! Two shapes: [`ProfilePatch`] is whatever is known so far, every field
//! optional; [`ProfileData`] is a profile that can go live, with every
//! required field present. Only [`ProfilePatch::complete`] turns one into the
//! other, so an incomplete profile cannot be matched by mistake.

use crate::city::City;
use crate::geo::GeoPoint;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

string_enum!(
    /// Why the person is here.
    Intent {
        HasRoom => "has_room",
        NeedsRoom => "needs_room",
        TeamUp => "team_up",
    }
);

string_enum!(ProfileStatus {
    /// Shown on the map and matched.
    Active => "active",
    /// Hidden at the person's request. Nothing is lost.
    Paused => "paused",
});

string_enum!(Gender {
    Male => "male",
    Female => "female",
    Other => "other",
});

string_enum!(
    /// Who the person is willing to live with.
    GenderPref {
        Any => "any",
        Same => "same",
    }
);

string_enum!(Sleep {
    Early => "early",
    Flexible => "flexible",
    Late => "late",
});

string_enum!(Cleanliness {
    Relaxed => "relaxed",
    Tidy => "tidy",
    VeryTidy => "very_tidy",
});

string_enum!(Food {
    Veg => "veg",
    Egg => "egg",
    NonVeg => "non_veg",
});

string_enum!(Smoking {
    No => "no",
    Outside => "outside",
    Yes => "yes",
});

string_enum!(Drinking {
    No => "no",
    Social => "social",
    Yes => "yes",
});

string_enum!(Pets {
    NoPets => "no_pets",
    FineWithPets => "fine_with_pets",
    HasPet => "has_pet",
});

string_enum!(Guests {
    Rarely => "rarely",
    Sometimes => "sometimes",
    Often => "often",
});

string_enum!(WorkMode {
    Home => "home",
    Hybrid => "hybrid",
    Office => "office",
});

string_enum!(
    /// A profile field the bot can ask for.
    Field {
        Intent => "intent",
        City => "city",
        Budget => "budget",
        Gender => "gender",
        GenderPref => "gender_pref",
    }
);

/// How a person lives. Every trait is optional: people say what matters to
/// them. Stored as JSON so new traits need no migration; a trait this version
/// does not know is dropped on read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lifestyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep: Option<Sleep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanliness: Option<Cleanliness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub food: Option<Food>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smoking: Option<Smoking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drinking: Option<Drinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pets: Option<Pets>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guests: Option<Guests>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work: Option<WorkMode>,
}

impl Lifestyle {
    /// How many traits are stated.
    pub fn known(&self) -> usize {
        [
            self.sleep.is_some(),
            self.cleanliness.is_some(),
            self.food.is_some(),
            self.smoking.is_some(),
            self.drinking.is_some(),
            self.pets.is_some(),
            self.guests.is_some(),
            self.work.is_some(),
        ]
        .into_iter()
        .filter(|known| *known)
        .count()
    }

    /// Takes every trait `other` states; keeps the rest.
    pub fn merge(&mut self, other: Lifestyle) {
        merge(&mut self.sleep, other.sleep);
        merge(&mut self.cleanliness, other.cleanliness);
        merge(&mut self.food, other.food);
        merge(&mut self.smoking, other.smoking);
        merge(&mut self.drinking, other.drinking);
        merge(&mut self.pets, other.pets);
        merge(&mut self.guests, other.guests);
        merge(&mut self.work, other.work);
    }
}

fn merge<T>(slot: &mut Option<T>, new: Option<T>) {
    if new.is_some() {
        *slot = new;
    }
}

/// A monthly amount in whole rupees: the rent of a room, or the most a seeker
/// will pay. `min` is 0 when only a ceiling was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    min: u32,
    max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BudgetError {
    #[error("a monthly budget is between ₹{} and ₹{}", Budget::FLOOR, Budget::CEIL)]
    OutOfRange,
    #[error("the lower amount is above the higher one")]
    Inverted,
}

impl Budget {
    pub const FLOOR: u32 = 1_000;
    pub const CEIL: u32 = 500_000;

    pub fn new(min: u32, max: u32) -> Result<Self, BudgetError> {
        if !(Self::FLOOR..=Self::CEIL).contains(&max) {
            return Err(BudgetError::OutOfRange);
        }
        if min > max {
            return Err(BudgetError::Inverted);
        }
        Ok(Self { min, max })
    }

    pub const fn min(self) -> u32 {
        self.min
    }

    pub const fn max(self) -> u32 {
        self.max
    }
}

/// What is known about a person so far. The output of reading one message,
/// and, merged over many messages, the draft of a profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfilePatch {
    pub intent: Option<Intent>,
    pub city: Option<City>,
    pub area: Option<String>,
    pub pin: Option<GeoPoint>,
    pub budget_min: Option<u32>,
    pub budget_max: Option<u32>,
    pub move_in: Option<NaiveDate>,
    pub gender: Option<Gender>,
    pub gender_pref: Option<GenderPref>,
    pub lifestyle: Lifestyle,
    pub note: Option<String>,
}

/// Longest area name and note kept, in characters.
pub const AREA_MAX_CHARS: usize = 60;
pub const NOTE_MAX_CHARS: usize = 280;

impl ProfilePatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Takes every field `other` states; keeps the rest. Free text is trimmed
    /// and cut to its limit here, the one place all sources pass through.
    pub fn merge(&mut self, other: ProfilePatch) {
        merge(&mut self.intent, other.intent);
        merge(&mut self.city, other.city);
        merge(&mut self.area, clean_text(other.area, AREA_MAX_CHARS).filter(|area| is_place_name(area)));
        merge(&mut self.pin, other.pin);
        merge(&mut self.budget_min, other.budget_min);
        merge(&mut self.budget_max, other.budget_max);
        merge(&mut self.move_in, other.move_in);
        merge(&mut self.gender, other.gender);
        merge(&mut self.gender_pref, other.gender_pref);
        self.lifestyle.merge(other.lifestyle);
        merge(&mut self.note, clean_text(other.note, NOTE_MAX_CHARS));
    }

    /// The budget this draft describes, if it states a valid one.
    ///
    /// A room has one price, so a lone amount is both ends for someone who
    /// has a room. For a seeker a lone amount is a ceiling.
    pub fn budget(&self) -> Option<Budget> {
        let max = self.budget_max?;
        let min = match (self.budget_min, self.intent) {
            (Some(min), _) => min,
            (None, Some(Intent::HasRoom)) => max,
            (None, _) => 0,
        };
        Budget::new(min.min(max), max).ok()
    }

    /// Required fields still missing, in the order the bot asks for them.
    pub fn missing(&self) -> Vec<Field> {
        let mut missing = Vec::new();
        if self.intent.is_none() {
            missing.push(Field::Intent);
        }
        if self.city.is_none() {
            missing.push(Field::City);
        }
        if self.budget().is_none() {
            missing.push(Field::Budget);
        }
        if self.gender.is_none() {
            missing.push(Field::Gender);
        }
        if self.gender_pref.is_none() {
            missing.push(Field::GenderPref);
        }
        missing
    }

    /// A profile that can go live, or the fields that stop it.
    pub fn complete(&self) -> Result<ProfileData, Vec<Field>> {
        match (self.intent, self.city.clone(), self.budget(), self.gender, self.gender_pref) {
            (Some(intent), Some(city), Some(budget), Some(gender), Some(gender_pref)) => Ok(ProfileData {
                intent,
                city,
                area: self.area.clone(),
                pin: self.pin,
                budget,
                move_in: self.move_in,
                gender,
                gender_pref,
                lifestyle: self.lifestyle.clone(),
                note: self.note.clone(),
            }),
            _ => Err(self.missing()),
        }
    }
}

/// An area is shown on the public map, so it must be a place name and
/// nothing else: "Sector 62" passes, a phone number or a link does not.
fn is_place_name(area: &str) -> bool {
    const MAX_DIGITS: usize = 3;
    area.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '.' | ',' | '\''))
        && area.chars().filter(char::is_ascii_digit).count() <= MAX_DIGITS
        && area.chars().any(char::is_alphabetic)
}

fn clean_text(text: Option<String>, max_chars: usize) -> Option<String> {
    let text = text?;
    let cleaned: String = text
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// A profile with everything matching needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileData {
    pub intent: Intent,
    pub city: City,
    pub area: Option<String>,
    pub pin: Option<GeoPoint>,
    pub budget: Budget,
    pub move_in: Option<NaiveDate>,
    pub gender: Gender,
    pub gender_pref: GenderPref,
    pub lifestyle: Lifestyle,
    pub note: Option<String>,
}

impl ProfileData {
    /// The same profile as a draft, for editing.
    pub fn to_patch(&self) -> ProfilePatch {
        ProfilePatch {
            intent: Some(self.intent),
            city: Some(self.city.clone()),
            area: self.area.clone(),
            pin: self.pin,
            budget_min: Some(self.budget.min()),
            budget_max: Some(self.budget.max()),
            move_in: self.move_in,
            gender: Some(self.gender),
            gender_pref: Some(self.gender_pref),
            lifestyle: self.lifestyle.clone(),
            note: self.note.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeker() -> ProfilePatch {
        ProfilePatch {
            intent: Some(Intent::NeedsRoom),
            city: City::parse("blr"),
            budget_max: Some(18_000),
            gender: Some(Gender::Male),
            gender_pref: Some(GenderPref::Any),
            ..ProfilePatch::default()
        }
    }

    #[test]
    fn a_draft_lists_what_is_missing_in_asking_order() {
        assert_eq!(
            ProfilePatch::default().missing(),
            [Field::Intent, Field::City, Field::Budget, Field::Gender, Field::GenderPref]
        );
        let mut draft = seeker();
        draft.gender = None;
        assert_eq!(draft.missing(), [Field::Gender]);
        assert_eq!(draft.complete(), Err(vec![Field::Gender]));
        assert!(seeker().complete().is_ok());
    }

    #[test]
    fn a_lone_amount_is_a_ceiling_for_a_seeker_and_a_price_for_a_room() {
        let seeker_budget = seeker().budget().unwrap();
        assert_eq!((seeker_budget.min(), seeker_budget.max()), (0, 18_000));

        let mut owner = seeker();
        owner.intent = Some(Intent::HasRoom);
        let rent = owner.budget().unwrap();
        assert_eq!((rent.min(), rent.max()), (18_000, 18_000));
    }

    #[test]
    fn an_impossible_budget_counts_as_missing() {
        for max in [0, 999, 500_001] {
            let mut draft = seeker();
            draft.budget_max = Some(max);
            assert_eq!(draft.missing(), [Field::Budget], "{max}");
        }
        assert_eq!(Budget::new(20_000, 10_000), Err(BudgetError::Inverted));
    }

    #[test]
    fn merging_keeps_old_answers_and_takes_new_ones() {
        let mut draft = seeker();
        draft.lifestyle.food = Some(Food::Veg);
        draft.merge(ProfilePatch {
            budget_max: Some(20_000),
            lifestyle: Lifestyle { sleep: Some(Sleep::Early), ..Lifestyle::default() },
            area: Some("  HSR \n Layout ".into()),
            ..ProfilePatch::default()
        });
        assert_eq!(draft.budget_max, Some(20_000));
        assert_eq!(draft.city, City::parse("bengaluru"));
        assert_eq!(draft.lifestyle.food, Some(Food::Veg));
        assert_eq!(draft.lifestyle.sleep, Some(Sleep::Early));
        assert_eq!(draft.area.as_deref(), Some("HSR Layout"));
        assert_eq!(draft.lifestyle.known(), 2);
    }

    #[test]
    fn free_text_is_cut_to_its_limit() {
        let mut draft = ProfilePatch::default();
        draft.merge(ProfilePatch { note: Some("x".repeat(1000)), area: Some("   ".into()), ..ProfilePatch::default() });
        assert_eq!(draft.note.map(|n| n.chars().count()), Some(NOTE_MAX_CHARS));
        assert_eq!(draft.area, None);
    }

    #[test]
    fn an_area_must_be_a_place_name() {
        let area_after = |text: &str| {
            let mut draft = ProfilePatch::default();
            draft.merge(ProfilePatch { area: Some(text.into()), ..ProfilePatch::default() });
            draft.area
        };
        assert_eq!(area_after("Sector 62").as_deref(), Some("Sector 62"));
        assert_eq!(area_after("HSR Layout, 2nd Stage").as_deref(), Some("HSR Layout, 2nd Stage"));
        assert_eq!(area_after("call 9876543210"), None);
        assert_eq!(area_after("see http://x.co/a"), None);
        assert_eq!(area_after("560034"), None);
    }

    #[test]
    fn lifestyle_json_ignores_traits_it_does_not_know() {
        let json = r#"{"sleep":"late","hobby":"chess"}"#;
        let lifestyle: Lifestyle = serde_json::from_str(json).unwrap();
        assert_eq!(lifestyle.sleep, Some(Sleep::Late));
        assert_eq!(serde_json::to_string(&lifestyle).unwrap(), r#"{"sleep":"late"}"#);
        assert!(serde_json::from_str::<Lifestyle>(r#"{"sleep":"sometimes"}"#).is_err());
    }

    #[test]
    fn a_profile_survives_the_round_trip_through_a_draft() {
        let profile = seeker().complete().unwrap();
        assert_eq!(profile.to_patch().complete().unwrap(), profile);
    }
}

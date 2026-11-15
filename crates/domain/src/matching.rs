//! Who can live with whom, and how well.
//!
//! [`eligible`] is the yes/no gate. [`score`] ranks the pairs that pass it.
//! [`explain`] says why, in words the viewer can act on. All three are plain
//! functions of two profiles, so a match can always be reproduced.

use crate::profile::{
    Cleanliness, Drinking, Food, Gender, GenderPref, Guests, Intent, Pets, ProfileData, Sleep, Smoking, WorkMode,
};

/// Pairs scoring below this are not shown to anyone.
pub const MIN_SCORE: u8 = 50;

/// How far over a seeker's ceiling a rent may be and still be shown.
const BUDGET_STRETCH: f64 = 1.10;
/// Two seekers pair up only if the smaller ceiling is this share of the larger.
const SEEKER_BUDGET_RATIO: f64 = 0.60;

/// Full marks within this distance; none beyond [`FAR_KM`].
const NEAR_KM: f64 = 2.0;
const FAR_KM: f64 = 15.0;

/// A trait one side did not state counts as half a fit: unknown is neither
/// good nor bad, and a thin profile must still be matchable.
const UNKNOWN: f64 = 0.5;

const W_SLEEP: f64 = 20.0;
const W_CLEANLINESS: f64 = 20.0;
const W_SMOKING: f64 = 15.0;
const W_FOOD: f64 = 10.0;
const W_BUDGET: f64 = 10.0;
const W_DISTANCE: f64 = 10.0;
const W_PETS: f64 = 5.0;
const W_GUESTS: f64 = 5.0;
const W_DRINKING: f64 = 3.0;
const W_WORK: f64 = 2.0;

/// Can these two be shown to each other at all?
pub fn eligible(a: &ProfileData, b: &ProfileData) -> bool {
    a.city == b.city && intents_fit(a.intent, b.intent) && genders_fit(a, b) && budgets_fit(a, b)
}

/// Someone with a room needs someone without one. Two people without a room
/// can look together, if at least one of them wants that.
pub fn intents_fit(a: Intent, b: Intent) -> bool {
    use Intent::{HasRoom, NeedsRoom, TeamUp};
    matches!(
        (a, b),
        (HasRoom, NeedsRoom) | (NeedsRoom, HasRoom) | (NeedsRoom, TeamUp) | (TeamUp, NeedsRoom) | (TeamUp, TeamUp)
    )
}

/// Each side's preference must hold for the other.
fn genders_fit(a: &ProfileData, b: &ProfileData) -> bool {
    let accepts = |pref: GenderPref, own: Gender, other: Gender| match pref {
        GenderPref::Any => true,
        GenderPref::Same => own == other,
    };
    accepts(a.gender_pref, a.gender, b.gender) && accepts(b.gender_pref, b.gender, a.gender)
}

fn budgets_fit(a: &ProfileData, b: &ProfileData) -> bool {
    match (a.intent, b.intent) {
        (Intent::HasRoom, _) => rent_in_reach(a, b),
        (_, Intent::HasRoom) => rent_in_reach(b, a),
        _ => {
            let (low, high) = ordered(a.budget.max(), b.budget.max());
            f64::from(low) >= f64::from(high) * SEEKER_BUDGET_RATIO
        }
    }
}

fn rent_in_reach(owner: &ProfileData, seeker: &ProfileData) -> bool {
    let rent = f64::from(owner.budget.min());
    rent <= f64::from(seeker.budget.max()) * BUDGET_STRETCH && rent >= f64::from(seeker.budget.min())
}

fn ordered(a: u32, b: u32) -> (u32, u32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// How well two eligible profiles fit, 0 to 100. Symmetric.
pub fn score(a: &ProfileData, b: &ProfileData) -> u8 {
    let (la, lb) = (&a.lifestyle, &b.lifestyle);
    let total = W_SLEEP * both(la.sleep, lb.sleep, sleep_fit)
        + W_CLEANLINESS * both(la.cleanliness, lb.cleanliness, cleanliness_fit)
        + W_SMOKING * both(la.smoking, lb.smoking, smoking_fit)
        + W_FOOD * both(la.food, lb.food, food_fit)
        + W_BUDGET * budget_fit(a, b)
        + W_DISTANCE * distance_fit(a, b)
        + W_PETS * both(la.pets, lb.pets, pets_fit)
        + W_GUESTS * both(la.guests, lb.guests, guests_fit)
        + W_DRINKING * both(la.drinking, lb.drinking, drinking_fit)
        + W_WORK * both(la.work, lb.work, work_fit);
    // The weights sum to 100 and every factor is in 0..=1.
    total.round().clamp(0.0, 100.0) as u8
}

fn both<T: Copy>(a: Option<T>, b: Option<T>, fit: fn(T, T) -> f64) -> f64 {
    match (a, b) {
        (Some(a), Some(b)) => fit(a, b),
        _ => UNKNOWN,
    }
}

/// Fit for a three-step scale: equal, one step apart, opposite ends.
fn ladder(gap: u8, adjacent: f64, opposite: f64) -> f64 {
    match gap {
        0 => 1.0,
        1 => adjacent,
        _ => opposite,
    }
}

fn gap<T: Copy + Into<u8>>(a: T, b: T) -> u8 {
    a.into().abs_diff(b.into())
}

macro_rules! rank {
    ($ty:ty { $($variant:ident = $rank:literal),+ }) => {
        impl From<$ty> for u8 {
            fn from(value: $ty) -> u8 {
                match value { $(<$ty>::$variant => $rank),+ }
            }
        }
    };
}

rank!(Sleep { Early = 0, Flexible = 1, Late = 2 });
rank!(Cleanliness { Relaxed = 0, Tidy = 1, VeryTidy = 2 });
rank!(Food { Veg = 0, Egg = 1, NonVeg = 2 });
rank!(Smoking { No = 0, Outside = 1, Yes = 2 });
rank!(Drinking { No = 0, Social = 1, Yes = 2 });
rank!(Guests { Rarely = 0, Sometimes = 1, Often = 2 });

fn sleep_fit(a: Sleep, b: Sleep) -> f64 {
    ladder(gap(a, b), 0.7, 0.0)
}

fn cleanliness_fit(a: Cleanliness, b: Cleanliness) -> f64 {
    ladder(gap(a, b), 0.6, 0.1)
}

fn smoking_fit(a: Smoking, b: Smoking) -> f64 {
    ladder(gap(a, b), 0.6, 0.0)
}

fn food_fit(a: Food, b: Food) -> f64 {
    ladder(gap(a, b), 0.75, 0.2)
}

fn drinking_fit(a: Drinking, b: Drinking) -> f64 {
    ladder(gap(a, b), 0.7, 0.3)
}

fn guests_fit(a: Guests, b: Guests) -> f64 {
    ladder(gap(a, b), 0.6, 0.2)
}

fn pets_fit(a: Pets, b: Pets) -> f64 {
    match (a, b) {
        (Pets::HasPet, Pets::NoPets) | (Pets::NoPets, Pets::HasPet) => 0.0,
        _ => 1.0,
    }
}

fn work_fit(a: WorkMode, b: WorkMode) -> f64 {
    if a == b { 1.0 } else { 0.7 }
}

fn budget_fit(a: &ProfileData, b: &ProfileData) -> f64 {
    match (a.intent, b.intent) {
        (Intent::HasRoom, _) => rent_fit(a, b),
        (_, Intent::HasRoom) => rent_fit(b, a),
        _ => {
            let (low, high) = ordered(a.budget.max(), b.budget.max());
            let ratio = f64::from(low) / f64::from(high);
            ((ratio - SEEKER_BUDGET_RATIO) / (1.0 - SEEKER_BUDGET_RATIO)).clamp(0.0, 1.0)
        }
    }
}

/// Full marks when the rent is within the seeker's ceiling, falling to zero
/// at the edge of the stretch.
fn rent_fit(owner: &ProfileData, seeker: &ProfileData) -> f64 {
    let rent = f64::from(owner.budget.min());
    let ceiling = f64::from(seeker.budget.max());
    if rent <= ceiling {
        return 1.0;
    }
    (1.0 - (rent - ceiling) / (ceiling * (BUDGET_STRETCH - 1.0))).clamp(0.0, 1.0)
}

fn distance_fit(a: &ProfileData, b: &ProfileData) -> f64 {
    match distance_km(a, b) {
        Some(km) => (1.0 - (km - NEAR_KM) / (FAR_KM - NEAR_KM)).clamp(0.0, 1.0),
        None if same_area(a, b) => 1.0,
        None => UNKNOWN,
    }
}

fn distance_km(a: &ProfileData, b: &ProfileData) -> Option<f64> {
    Some(a.pin?.km_to(b.pin?))
}

fn same_area(a: &ProfileData, b: &ProfileData) -> bool {
    match (&a.area, &b.area) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// Why a match is good and what to talk about first, written for `viewer`
/// about `other`. At most three fits and two cautions, strongest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Explanation {
    pub fits: Vec<String>,
    pub watch: Vec<String>,
}

const MAX_FITS: usize = 3;
const MAX_WATCH: usize = 2;

pub fn explain(viewer: &ProfileData, other: &ProfileData) -> Explanation {
    let (mine, theirs) = (&viewer.lifestyle, &other.lifestyle);
    // In weight order, so the strongest reasons are kept when the list is cut.
    let lines: [Option<Line>; 8] = [
        pair(mine.sleep, theirs.sleep, sleep_line),
        pair(mine.cleanliness, theirs.cleanliness, cleanliness_line),
        pair(mine.smoking, theirs.smoking, smoking_line),
        pair(mine.food, theirs.food, food_line),
        distance_line(viewer, other),
        pair(mine.pets, theirs.pets, pets_line),
        pair(mine.guests, theirs.guests, guests_line),
        pair(mine.drinking, theirs.drinking, drinking_line),
    ];
    let mut explanation = Explanation::default();
    for line in lines.into_iter().flatten() {
        match line {
            Line::Fit(text) if explanation.fits.len() < MAX_FITS => explanation.fits.push(text),
            Line::Watch(text) if explanation.watch.len() < MAX_WATCH => explanation.watch.push(text),
            _ => {}
        }
    }
    explanation
}

enum Line {
    Fit(String),
    Watch(String),
}

fn fit(text: &str) -> Option<Line> {
    Some(Line::Fit(text.to_owned()))
}

fn watch(text: &str) -> Option<Line> {
    Some(Line::Watch(text.to_owned()))
}

fn pair<T: Copy>(mine: Option<T>, theirs: Option<T>, line: fn(T, T) -> Option<Line>) -> Option<Line> {
    line(mine?, theirs?)
}

fn sleep_line(mine: Sleep, theirs: Sleep) -> Option<Line> {
    match (mine, theirs) {
        (Sleep::Early, Sleep::Early) => fit("both sleep early"),
        (Sleep::Late, Sleep::Late) => fit("both are night owls"),
        (Sleep::Flexible, Sleep::Flexible) => fit("both have flexible sleep hours"),
        (Sleep::Early, Sleep::Late) => watch("they sleep late, you sleep early"),
        (Sleep::Late, Sleep::Early) => watch("they sleep early, you sleep late"),
        _ => None,
    }
}

fn cleanliness_line(mine: Cleanliness, theirs: Cleanliness) -> Option<Line> {
    match (mine, theirs) {
        (Cleanliness::VeryTidy, Cleanliness::VeryTidy) => fit("both very tidy"),
        (Cleanliness::Tidy, Cleanliness::Tidy) => fit("both keep things tidy"),
        (Cleanliness::Relaxed, Cleanliness::Relaxed) => fit("both relaxed about cleaning"),
        (Cleanliness::VeryTidy, Cleanliness::Relaxed) => watch("they are relaxed about cleaning, you are very tidy"),
        (Cleanliness::Relaxed, Cleanliness::VeryTidy) => watch("they are very tidy, you are relaxed about cleaning"),
        _ => None,
    }
}

fn smoking_line(mine: Smoking, theirs: Smoking) -> Option<Line> {
    match (mine, theirs) {
        (Smoking::No, Smoking::No) => fit("both non-smokers"),
        (Smoking::No, Smoking::Yes) => watch("they smoke at home, you do not smoke"),
        (Smoking::Yes, Smoking::No) => watch("they do not smoke, you smoke at home"),
        (Smoking::No, Smoking::Outside) => watch("they smoke, but only outside"),
        _ => None,
    }
}

fn food_line(mine: Food, theirs: Food) -> Option<Line> {
    match (mine, theirs) {
        (Food::Veg, Food::Veg) => fit("both vegetarian"),
        (Food::NonVeg, Food::NonVeg) => fit("both eat non-veg"),
        (Food::Veg, Food::NonVeg) => watch("they cook non-veg, you are vegetarian"),
        (Food::NonVeg, Food::Veg) => watch("they are vegetarian, you cook non-veg"),
        _ => None,
    }
}

fn pets_line(mine: Pets, theirs: Pets) -> Option<Line> {
    match (mine, theirs) {
        (Pets::NoPets, Pets::HasPet) => watch("they have a pet, you prefer none"),
        (Pets::HasPet, Pets::NoPets) => watch("they prefer no pets, you have one"),
        (Pets::FineWithPets, Pets::HasPet) => fit("they have a pet, and you like pets"),
        (Pets::HasPet, Pets::FineWithPets) => fit("they are fine with your pet"),
        (Pets::HasPet, Pets::HasPet) => fit("both have pets"),
        _ => None,
    }
}

fn guests_line(mine: Guests, theirs: Guests) -> Option<Line> {
    match (mine, theirs) {
        (Guests::Rarely, Guests::Rarely) => fit("both rarely have guests"),
        (Guests::Often, Guests::Often) => fit("both like having friends over"),
        (Guests::Rarely, Guests::Often) => watch("they often have guests, you rarely do"),
        (Guests::Often, Guests::Rarely) => watch("they rarely have guests, you often do"),
        _ => None,
    }
}

fn drinking_line(mine: Drinking, theirs: Drinking) -> Option<Line> {
    match (mine, theirs) {
        (Drinking::No, Drinking::No) => fit("both do not drink"),
        (Drinking::No, Drinking::Yes) => watch("they drink, you do not"),
        (Drinking::Yes, Drinking::No) => watch("they do not drink, you do"),
        _ => None,
    }
}

fn distance_line(viewer: &ProfileData, other: &ProfileData) -> Option<Line> {
    match distance_km(viewer, other) {
        Some(km) if km <= NEAR_KM => fit("same neighbourhood"),
        Some(km) if km >= FAR_KM => Some(Line::Watch(format!("about {km:.0} km from your area"))),
        Some(_) => None,
        None if same_area(viewer, other) => fit("same area"),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::City;
    use crate::geo::GeoPoint;
    use crate::profile::{Budget, Lifestyle};

    fn person(intent: Intent, max: u32) -> ProfileData {
        let min = if intent == Intent::HasRoom { max } else { 0 };
        ProfileData {
            intent,
            city: City::parse("blr").unwrap(),
            area: None,
            pin: None,
            budget: Budget::new(min, max).unwrap(),
            move_in: None,
            gender: Gender::Male,
            gender_pref: GenderPref::Any,
            lifestyle: Lifestyle::default(),
            note: None,
        }
    }

    fn tidy_early_veg() -> Lifestyle {
        Lifestyle {
            sleep: Some(Sleep::Early),
            cleanliness: Some(Cleanliness::VeryTidy),
            food: Some(Food::Veg),
            smoking: Some(Smoking::No),
            ..Lifestyle::default()
        }
    }

    #[test]
    fn intents_pair_as_the_plan_says() {
        use Intent::{HasRoom, NeedsRoom, TeamUp};
        let fits: Vec<(Intent, Intent)> = Intent::ALL
            .iter()
            .flat_map(|a| Intent::ALL.iter().map(move |b| (*a, *b)))
            .filter(|(a, b)| intents_fit(*a, *b))
            .collect();
        assert_eq!(
            fits,
            [(HasRoom, NeedsRoom), (NeedsRoom, HasRoom), (NeedsRoom, TeamUp), (TeamUp, NeedsRoom), (TeamUp, TeamUp)]
        );
    }

    #[test]
    fn different_cities_never_match() {
        let owner = person(Intent::HasRoom, 15_000);
        let mut seeker = person(Intent::NeedsRoom, 18_000);
        assert!(eligible(&owner, &seeker));
        seeker.city = City::parse("pune").unwrap();
        assert!(!eligible(&owner, &seeker));
    }

    #[test]
    fn a_same_gender_preference_binds_both_sides() {
        let mut owner = person(Intent::HasRoom, 15_000);
        let mut seeker = person(Intent::NeedsRoom, 18_000);
        owner.gender = Gender::Female;
        owner.gender_pref = GenderPref::Same;
        assert!(!eligible(&owner, &seeker), "her preference excludes him");
        assert!(!eligible(&seeker, &owner), "and it holds whoever asks");
        seeker.gender = Gender::Female;
        assert!(eligible(&owner, &seeker));
    }

    #[test]
    fn rent_may_exceed_the_ceiling_by_a_tenth_and_no_more() {
        let seeker = person(Intent::NeedsRoom, 20_000);
        assert!(eligible(&person(Intent::HasRoom, 22_000), &seeker));
        assert!(!eligible(&person(Intent::HasRoom, 22_100), &seeker));
        // A seeker who gave a floor is not shown rooms below it.
        let mut picky = person(Intent::NeedsRoom, 20_000);
        picky.budget = Budget::new(12_000, 20_000).unwrap();
        assert!(!eligible(&person(Intent::HasRoom, 8_000), &picky));
    }

    #[test]
    fn two_seekers_need_budgets_in_the_same_league() {
        let a = person(Intent::TeamUp, 20_000);
        assert!(eligible(&a, &person(Intent::TeamUp, 12_000)));
        assert!(!eligible(&a, &person(Intent::NeedsRoom, 11_000)));
    }

    #[test]
    fn the_score_is_symmetric_and_bounded() {
        let mut a = person(Intent::HasRoom, 15_000);
        let mut b = person(Intent::NeedsRoom, 18_000);
        a.lifestyle = tidy_early_veg();
        b.lifestyle = Lifestyle { sleep: Some(Sleep::Late), pets: Some(Pets::HasPet), ..Lifestyle::default() };
        a.pin = GeoPoint::new(12.9352, 77.6245);
        b.pin = GeoPoint::new(12.9784, 77.6408);
        assert_eq!(score(&a, &b), score(&b, &a));
        assert!(score(&a, &b) <= 100);
    }

    #[test]
    fn identical_habits_score_far_above_opposite_ones() {
        let mut a = person(Intent::HasRoom, 15_000);
        let mut twin = person(Intent::NeedsRoom, 18_000);
        a.lifestyle = tidy_early_veg();
        twin.lifestyle = tidy_early_veg();
        let mut opposite = person(Intent::NeedsRoom, 18_000);
        opposite.lifestyle = Lifestyle {
            sleep: Some(Sleep::Late),
            cleanliness: Some(Cleanliness::Relaxed),
            food: Some(Food::NonVeg),
            smoking: Some(Smoking::Yes),
            ..Lifestyle::default()
        };
        assert!(score(&a, &twin) >= 85, "{}", score(&a, &twin));
        assert!(score(&a, &opposite) < MIN_SCORE, "{}", score(&a, &opposite));
    }

    #[test]
    fn two_profiles_that_say_nothing_are_still_shown() {
        let a = person(Intent::HasRoom, 15_000);
        let b = person(Intent::NeedsRoom, 18_000);
        assert!(score(&a, &b) >= MIN_SCORE, "{}", score(&a, &b));
    }

    #[test]
    fn nearer_scores_higher() {
        let mut a = person(Intent::TeamUp, 15_000);
        let mut near = person(Intent::TeamUp, 15_000);
        let mut far = person(Intent::TeamUp, 15_000);
        a.pin = GeoPoint::new(12.9352, 77.6245);
        near.pin = GeoPoint::new(12.9360, 77.6250);
        far.pin = GeoPoint::new(13.1000, 77.6000);
        assert!(score(&a, &near) > score(&a, &far));
    }

    #[test]
    fn the_explanation_is_written_for_the_viewer() {
        let mut me = person(Intent::NeedsRoom, 18_000);
        let mut them = person(Intent::HasRoom, 15_000);
        me.lifestyle = Lifestyle { pets: Some(Pets::NoPets), ..tidy_early_veg() };
        them.lifestyle = Lifestyle { pets: Some(Pets::HasPet), ..tidy_early_veg() };

        let mine = explain(&me, &them);
        assert_eq!(mine.fits, ["both sleep early", "both very tidy", "both non-smokers"]);
        assert_eq!(mine.watch, ["they have a pet, you prefer none"]);

        let theirs = explain(&them, &me);
        assert_eq!(theirs.watch, ["they prefer no pets, you have one"]);
    }

    #[test]
    fn the_explanation_keeps_only_the_strongest_lines() {
        let mut me = person(Intent::NeedsRoom, 18_000);
        let mut them = person(Intent::HasRoom, 15_000);
        me.lifestyle = Lifestyle {
            sleep: Some(Sleep::Early),
            cleanliness: Some(Cleanliness::VeryTidy),
            smoking: Some(Smoking::No),
            food: Some(Food::Veg),
            guests: Some(Guests::Rarely),
            ..Lifestyle::default()
        };
        them.lifestyle = Lifestyle {
            sleep: Some(Sleep::Late),
            cleanliness: Some(Cleanliness::Relaxed),
            smoking: Some(Smoking::Yes),
            food: Some(Food::NonVeg),
            guests: Some(Guests::Often),
            ..Lifestyle::default()
        };
        let explanation = explain(&me, &them);
        assert!(explanation.fits.is_empty());
        assert_eq!(
            explanation.watch,
            ["they sleep late, you sleep early", "they are relaxed about cleaning, you are very tidy"]
        );
    }

    #[test]
    fn nothing_stated_means_nothing_claimed() {
        let a = person(Intent::HasRoom, 15_000);
        let b = person(Intent::NeedsRoom, 18_000);
        assert_eq!(explain(&a, &b), Explanation::default());
    }
}

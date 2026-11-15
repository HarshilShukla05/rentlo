//! Reads a chat message into a [`ProfilePatch`] with fixed rules.
//!
//! This is the floor the product stands on: it needs no network, costs
//! nothing and cannot fail, so the bot still works when the language model
//! is slow, down or over budget. The model reads nuance; these rules read
//! the plain cases ("18k", "blr", "veg", "2") and every numbered reply.
//!
//! A rule fires only on an unambiguous phrase. A field it is unsure of stays
//! empty, and the bot asks for it: a wrong guess is worse than a question.

use crate::city::{City, MAX_ALIAS_WORDS, title_case};
use crate::profile::{
    Budget, Cleanliness, Drinking, Field, Food, Gender, GenderPref, Guests, Intent, Lifestyle, Pets, ProfilePatch,
    Sleep, Smoking, WorkMode,
};

/// A bare number is read as a monthly amount from here up. Below it sit
/// clock times, ages and years.
const BARE_AMOUNT_FLOOR: f64 = 3_000.0;

/// Reads `text`. `asked` is the field the bot's last message asked for, if
/// it asked for exactly one: it lets a bare reply like "2" or "pune" count.
pub fn extract(text: &str, asked: Option<Field>) -> ProfilePatch {
    let message = Message::new(text);
    let mut patch = ProfilePatch {
        intent: intent(&message),
        gender: gender(&message),
        gender_pref: gender_pref(&message),
        lifestyle: lifestyle(&message),
        ..ProfilePatch::default()
    };
    if let Some((city, at)) = city(&message) {
        patch.area = area_before(&message, at);
        patch.city = Some(city);
    }
    (patch.budget_min, patch.budget_max) = budget(&message, asked == Some(Field::Budget));
    if let Some(field) = asked {
        answer(&message, field, &mut patch);
    }
    patch
}

/// A message as lower-case words. Punctuation is gone, so a phrase can be
/// matched on word boundaries: "need a flat" never matches "need a flatmate".
struct Message {
    words: Vec<String>,
    /// The words joined by single spaces, with a space at each end.
    padded: String,
}

impl Message {
    fn new(text: &str) -> Self {
        let chars: Vec<char> = text.to_lowercase().chars().collect();
        let mut clean = String::with_capacity(chars.len());
        for (i, &c) in chars.iter().enumerate() {
            let digit_at = |j: Option<usize>| j.and_then(|j| chars.get(j)).is_some_and(char::is_ascii_digit);
            let between_digits = digit_at(i.checked_sub(1)) && digit_at(Some(i + 1));
            match c {
                // "18,000" is one number; "17.5k" keeps its point.
                ',' if between_digits => {}
                '.' if between_digits => clean.push('.'),
                // "don't" reads as "dont".
                '\'' | '’' => {}
                c if c.is_alphanumeric() || c == '₹' => clean.push(c),
                _ => clean.push(' '),
            }
        }
        let words: Vec<String> = clean.split_whitespace().map(str::to_owned).collect();
        let padded = format!(" {} ", words.join(" "));
        Self { words, padded }
    }

    fn has(&self, phrase: &str) -> bool {
        self.padded.contains(&format!(" {phrase} "))
    }

    fn has_any(&self, phrases: &[&str]) -> bool {
        phrases.iter().any(|phrase| self.has(phrase))
    }

    fn word(&self, index: usize) -> Option<&str> {
        self.words.get(index).map(String::as_str)
    }

    fn is_short(&self) -> bool {
        self.words.len() <= 3
    }
}

fn intent(m: &Message) -> Option<Intent> {
    // Checked in this order: "looking for a flat together" is a team-up,
    // not a plain search.
    if m.has_any(&[
        "team up",
        "teamup",
        "flat together",
        "place together",
        "house together",
        "search together",
        "hunt together",
        "look together",
        "find a flat with",
        "co hunt",
    ]) {
        return Some(Intent::TeamUp);
    }
    if m.has_any(&[
        "have a room",
        "have room",
        "has a room",
        "got a room",
        "room available",
        "room is available",
        "room vacant",
        "vacant room",
        "spare room",
        "empty room",
        "have a flat",
        "have a 1bhk",
        "have a 2bhk",
        "have a 3bhk",
        "my flat",
        "my 2bhk",
        "my 3bhk",
        "our flat",
        "our 2bhk",
        "our 3bhk",
        "need a flatmate",
        "need flatmate",
        "need a roommate",
        "need roommate",
        "want a flatmate",
        "want a roommate",
        "looking for a flatmate",
        "looking for flatmate",
        "looking for a roommate",
        "looking for roommate",
        "flatmate needed",
        "roommate needed",
        "flatmate required",
        "roommate required",
        "need a replacement",
        "replacement needed",
        "moving out",
    ]) {
        return Some(Intent::HasRoom);
    }
    if m.has_any(&[
        "need a room",
        "need room",
        "need a flat",
        "need flat",
        "need a place",
        "need accommodation",
        "need a pg",
        "want a room",
        "want a flat",
        "looking for a room",
        "looking for room",
        "looking for a flat",
        "looking for flat",
        "looking for a place",
        "looking for accommodation",
        "looking for a pg",
        "searching for a room",
        "searching for a flat",
        "search for a room",
        "find a room",
        "find me a room",
        "room needed",
        "room required",
        "shifting to",
        "moving to",
        "relocating to",
    ]) {
        return Some(Intent::NeedsRoom);
    }
    None
}

const MALE_WORDS: &[&str] = &["male", "man", "guy", "boy"];
const FEMALE_WORDS: &[&str] = &["female", "woman", "girl", "lady"];

fn gender_of(word: &str) -> Option<Gender> {
    if MALE_WORDS.contains(&word) {
        Some(Gender::Male)
    } else if FEMALE_WORDS.contains(&word) {
        Some(Gender::Female)
    } else {
        None
    }
}

/// The person's own gender. A gender word next to "flatmate" or "only" says
/// who they want, not who they are, and is left to [`gender_pref`].
fn gender(m: &Message) -> Option<Gender> {
    const ABOUT_OTHERS_AFTER: &[&str] = &[
        "flatmate",
        "flatmates",
        "roommate",
        "roommates",
        "tenant",
        "tenants",
        "only",
        "preferred",
        "candidate",
        "candidates",
    ];
    const ABOUT_OTHERS_BEFORE: &[&str] =
        &["only", "prefer", "preferably", "preferred", "for", "want", "need", "a", "any"];
    const SELF_BEFORE: &[&str] = &["am", "im", "m", "i"];

    for (i, word) in m.words.iter().enumerate() {
        // "25m", "24f": age and gender in one token.
        if let Some(found) = age_gender(word) {
            return Some(found);
        }
        let Some(found) = gender_of(word) else {
            continue;
        };
        let before = i.checked_sub(1).and_then(|j| m.word(j));
        let before2 = i.checked_sub(2).and_then(|j| m.word(j));
        let after = m.word(i + 1);
        if after.is_some_and(|w| ABOUT_OTHERS_AFTER.contains(&w)) {
            continue;
        }
        // "i am a female": the article belongs to a statement about oneself.
        let stated = before.is_some_and(|w| SELF_BEFORE.contains(&w))
            || (before == Some("a") && before2.is_some_and(|w| SELF_BEFORE.contains(&w)));
        if stated || !before.is_some_and(|w| ABOUT_OTHERS_BEFORE.contains(&w)) {
            return Some(found);
        }
    }
    None
}

fn age_gender(word: &str) -> Option<Gender> {
    let (age, letter) = word.split_at_checked(word.len().checked_sub(1)?)?;
    let plausible_age = age.len() == 2 && age.parse::<u8>().is_ok_and(|a| (16..=80).contains(&a));
    match (plausible_age, letter) {
        (true, "m") => Some(Gender::Male),
        (true, "f") => Some(Gender::Female),
        _ => None,
    }
}

fn gender_pref(m: &Message) -> Option<GenderPref> {
    if m.has_any(&[
        "any gender",
        "gender no bar",
        "no gender preference",
        "gender doesnt matter",
        "gender does not matter",
        "male or female",
        "boys or girls",
        "girls or boys",
    ]) {
        return Some(GenderPref::Any);
    }
    if m.has_any(&["same gender", "same sex"]) {
        return Some(GenderPref::Same);
    }
    let names_a_gender = m.words.iter().enumerate().any(|(i, word)| {
        let plural = word.strip_suffix('s').unwrap_or(word);
        let is_gender = gender_of(word).is_some()
            || gender_of(plural).is_some()
            || matches!(word.as_str(), "women" | "men" | "ladies");
        let next = m.word(i + 1);
        let prev = i.checked_sub(1).and_then(|j| m.word(j));
        is_gender
            && (matches!(next, Some("only" | "flatmate" | "flatmates" | "roommate" | "roommates" | "preferred"))
                || prev == Some("only"))
    });
    names_a_gender.then_some(GenderPref::Same)
}

fn lifestyle(m: &Message) -> Lifestyle {
    Lifestyle {
        sleep: sleep(m),
        cleanliness: cleanliness(m),
        food: food(m),
        smoking: smoking(m),
        drinking: drinking(m),
        pets: pets(m),
        guests: guests(m),
        work: work(m),
    }
}

fn sleep(m: &Message) -> Option<Sleep> {
    if m.has_any(&[
        "night owl",
        "sleep late",
        "sleeps late",
        "late sleeper",
        "late nights",
        "up late",
        "after midnight",
        "past midnight",
    ]) {
        return Some(Sleep::Late);
    }
    if m.has_any(&["early bird", "sleep early", "sleeps early", "early sleeper", "early riser", "morning person"]) {
        return Some(Sleep::Early);
    }
    if m.has_any(&["flexible sleep", "sleep flexible", "sleep whenever", "no fixed sleep"]) {
        return Some(Sleep::Flexible);
    }
    // "sleep by 11", "sleep at 1": the hour on a twelve-hour clock.
    m.words.iter().enumerate().find_map(|(i, word)| {
        if !matches!(word.as_str(), "sleep" | "sleeps" | "bed" | "asleep") {
            return None;
        }
        let hour_at = |j: usize| m.word(j).and_then(|w| w.trim_end_matches(['a', 'p', 'm']).parse::<u8>().ok());
        let hour = match m.word(i + 1) {
            Some("by" | "at" | "around" | "before" | "after") => hour_at(i + 2),
            _ => hour_at(i + 1),
        }?;
        match hour {
            9..=11 => Some(Sleep::Early),
            12 => Some(Sleep::Flexible),
            1..=4 => Some(Sleep::Late),
            _ => None,
        }
    })
}

fn cleanliness(m: &Message) -> Option<Cleanliness> {
    if m.has_any(&[
        "clean freak",
        "cleanliness freak",
        "neat freak",
        "very clean",
        "very tidy",
        "very neat",
        "super clean",
        "super tidy",
        "extremely clean",
        "spotless",
        "ocd",
    ]) {
        return Some(Cleanliness::VeryTidy);
    }
    if m.has_any(&[
        "messy",
        "not very clean",
        "not so clean",
        "not tidy",
        "untidy",
        "chill about cleaning",
        "relaxed about cleaning",
        "laid back about cleaning",
    ]) {
        return Some(Cleanliness::Relaxed);
    }
    m.has_any(&["clean", "tidy", "neat", "organised", "organized", "hygienic"]).then_some(Cleanliness::Tidy)
}

fn food(m: &Message) -> Option<Food> {
    // "non veg" before "veg": the longer phrase contains the shorter word.
    if m.has_any(&[
        "non veg",
        "nonveg",
        "non vegetarian",
        "nonvegetarian",
        "eat meat",
        "eats meat",
        "meat eater",
        "chicken",
        "eat everything",
    ]) {
        return Some(Food::NonVeg);
    }
    if m.has_any(&["eggetarian", "eggitarian", "eat eggs", "eats eggs", "veg and egg", "veg plus egg"]) {
        return Some(Food::Egg);
    }
    m.has_any(&["veg", "vegetarian", "pure veg", "vegan", "jain"]).then_some(Food::Veg)
}

fn smoking(m: &Message) -> Option<Smoking> {
    if m.has_any(&[
        "non smoker",
        "nonsmoker",
        "non smoking",
        "no smoking",
        "dont smoke",
        "do not smoke",
        "doesnt smoke",
        "never smoke",
        "not a smoker",
        "no smoke",
    ]) {
        return Some(Smoking::No);
    }
    if m.has_any(&[
        "smoke outside",
        "smokes outside",
        "smoke only outside",
        "smoke in the balcony",
        "smoke on the balcony",
        "smoke in balcony",
        "balcony smoker",
    ]) {
        return Some(Smoking::Outside);
    }
    m.has_any(&["smoker", "i smoke", "smoke", "smokes", "smoking"]).then_some(Smoking::Yes)
}

fn drinking(m: &Message) -> Option<Drinking> {
    if m.has_any(&[
        "non drinker",
        "nondrinker",
        "teetotaler",
        "teetotaller",
        "teetotal",
        "dont drink",
        "do not drink",
        "doesnt drink",
        "no alcohol",
        "no drinking",
        "non alcoholic",
        "never drink",
    ]) {
        return Some(Drinking::No);
    }
    if m.has_any(&[
        "social drinker",
        "drink socially",
        "drinks socially",
        "drink occasionally",
        "occasional drinker",
        "occasional drinks",
        "social drinking",
    ]) {
        return Some(Drinking::Social);
    }
    m.has_any(&["drinker", "i drink", "drinks regularly", "drink regularly"]).then_some(Drinking::Yes)
}

fn pets(m: &Message) -> Option<Pets> {
    if m.has_any(&[
        "no pets",
        "no pet",
        "not ok with pets",
        "not okay with pets",
        "dont like pets",
        "allergic to cats",
        "allergic to dogs",
        "allergic to pets",
        "pets not allowed",
    ]) {
        return Some(Pets::NoPets);
    }
    if m.has_any(&[
        "have a cat",
        "have a dog",
        "have a pet",
        "have cats",
        "have dogs",
        "have pets",
        "has a cat",
        "has a dog",
        "my cat",
        "my dog",
        "my pet",
        "my cats",
        "my dogs",
        "pet parent",
        "cat parent",
        "dog parent",
    ]) {
        return Some(Pets::HasPet);
    }
    m.has_any(&[
        "pet friendly",
        "ok with pets",
        "okay with pets",
        "fine with pets",
        "love pets",
        "love dogs",
        "love cats",
        "love animals",
        "like pets",
        "pets welcome",
        "pets are fine",
        "pets ok",
    ])
    .then_some(Pets::FineWithPets)
}

fn guests(m: &Message) -> Option<Guests> {
    if m.has_any(&["no guests", "rarely have guests", "guests rarely", "no visitors", "no parties", "dont party"]) {
        return Some(Guests::Rarely);
    }
    if m.has_any(&[
        "guests often",
        "often have guests",
        "friends over often",
        "love hosting",
        "host often",
        "house parties",
        "party often",
        "love parties",
    ]) {
        return Some(Guests::Often);
    }
    m.has_any(&["guests sometimes", "occasional guests", "guests occasionally", "friends over sometimes"])
        .then_some(Guests::Sometimes)
}

fn work(m: &Message) -> Option<WorkMode> {
    if m.has("hybrid") {
        return Some(WorkMode::Hybrid);
    }
    if m.has_any(&[
        "wfh",
        "work from home",
        "works from home",
        "working from home",
        "remote job",
        "work remotely",
        "remote work",
    ]) {
        return Some(WorkMode::Home);
    }
    m.has_any(&["wfo", "work from office", "works from office", "go to office", "office going", "office daily"])
        .then_some(WorkMode::Office)
}

/// The first known city named, with the index of its first word.
fn city(m: &Message) -> Option<(City, usize)> {
    (0..m.words.len()).find_map(|start| {
        (1..=MAX_ALIAS_WORDS)
            .rev()
            .filter(|len| start + len <= m.words.len())
            .find_map(|len| City::from_alias(&m.words[start..start + len].join(" ")))
            .map(|city| (city, start))
    })
}

/// The area in "in koramangala blr": the words between a preposition and the
/// city. Only when every word looks like part of a place name.
fn area_before(m: &Message, city_at: usize) -> Option<String> {
    const PREPOSITIONS: &[&str] = &["in", "near", "around", "at"];
    const NOT_A_PLACE: &[&str] = &[
        "a", "an", "the", "my", "room", "flat", "place", "pg", "house", "for", "and", "or", "of", "to", "area",
        "budget", "rent", "max", "need", "have", "looking", "with",
    ];
    const MAX_AREA_WORDS: usize = 3;

    let from = (city_at.saturating_sub(MAX_AREA_WORDS + 1)..city_at)
        .rev()
        .find(|&i| m.word(i).is_some_and(|w| PREPOSITIONS.contains(&w)))?;
    let words = &m.words[from + 1..city_at];
    let is_place = !words.is_empty()
        && words.iter().all(|w| w.chars().all(char::is_alphabetic) && !NOT_A_PLACE.contains(&w.as_str()));
    is_place.then(|| title_case(&words.join(" ")))
}

/// A number in the message that may be an amount of money.
struct Amount {
    at: usize,
    value: f64,
    /// Written with "k", "thousand", a currency mark or a money word.
    marked: bool,
    thousands: bool,
}

/// `(budget_min, budget_max)`. One amount is a ceiling; two are a range.
fn budget(m: &Message, asked_for_budget: bool) -> (Option<u32>, Option<u32>) {
    const MONEY_BEFORE: &[&str] =
        &["rs", "inr", "₹", "budget", "rent", "upto", "under", "max", "within", "below", "maximum"];
    const MONEY_AFTER: &[&str] = &["rs", "inr", "rupees", "max", "budget", "rent", "pm", "monthly"];

    let mut amounts: Vec<Amount> = Vec::new();
    for (at, word) in m.words.iter().enumerate() {
        let currency = word.starts_with('₹') || word.starts_with("rs");
        let digits = word.trim_start_matches('₹').trim_start_matches("rs");
        let (digits, suffix_k) = match digits.strip_suffix('k') {
            Some(rest) => (rest, true),
            None => (digits, false),
        };
        let Ok(value) = digits.parse::<f64>() else {
            continue;
        };
        if digits.is_empty() || !value.is_finite() {
            continue;
        }
        let before = at.checked_sub(1).and_then(|j| m.word(j));
        let after = m.word(at + 1);
        let thousands = suffix_k || matches!(after, Some("k" | "thousand"));
        let marked = currency
            || thousands
            || before.is_some_and(|w| MONEY_BEFORE.contains(&w))
            || after.is_some_and(|w| MONEY_AFTER.contains(&w));
        amounts.push(Amount { at, value, marked, thousands });
    }

    // "15-20k" and "15 to 20k": the "k" belongs to both ends.
    for i in 1..amounts.len() {
        let gap = amounts[i].at - amounts[i - 1].at;
        let joined = gap == 1 || (gap == 2 && matches!(m.word(amounts[i].at - 1), Some("to" | "and")));
        if joined && amounts[i].thousands && !amounts[i - 1].thousands && amounts[i - 1].value < 1_000.0 {
            amounts[i - 1].thousands = true;
            amounts[i - 1].marked = true;
        }
    }

    let range = f64::from(Budget::FLOOR)..=f64::from(Budget::CEIL);
    let mut rupees = amounts.iter().filter_map(|amount| {
        let value = if amount.thousands { amount.value * 1_000.0 } else { amount.value };
        // Asked "your budget?", a bare "18" can only mean eighteen thousand.
        let shorthand = asked_for_budget && m.is_short() && !amount.marked && amount.value < 1_000.0;
        let value = if shorthand { value * 1_000.0 } else { value };
        let credible = amount.marked || shorthand || value >= BARE_AMOUNT_FLOOR;
        (credible && range.contains(&value)).then_some(value.round() as u32)
    });

    match (rupees.next(), rupees.next()) {
        (Some(a), Some(b)) => (Some(a.min(b)), Some(a.max(b))),
        (Some(only), None) => (None, Some(only)),
        _ => (None, None),
    }
}

/// Reads a reply to a question about one field: a number from the options,
/// or a single word that would be too loose to trust in a longer message.
fn answer(m: &Message, field: Field, patch: &mut ProfilePatch) {
    if !m.is_short() {
        return;
    }
    let first = m.word(0).unwrap_or_default();
    let choice = first.parse::<usize>().ok().filter(|_| m.words.len() == 1);
    match field {
        Field::Intent if patch.intent.is_none() => {
            patch.intent = match (choice, first) {
                (Some(n), _) => n.checked_sub(1).and_then(|i| Intent::ALL.get(i)).copied(),
                (None, "have" | "has" | "owner") => Some(Intent::HasRoom),
                (None, "need" | "looking" | "searching" | "want") => Some(Intent::NeedsRoom),
                (None, "team" | "together" | "teamup") => Some(Intent::TeamUp),
                _ => None,
            };
        }
        Field::Gender if patch.gender.is_none() => {
            patch.gender = match (choice, first) {
                (Some(n), _) => n.checked_sub(1).and_then(|i| Gender::ALL.get(i)).copied(),
                (None, "m") => Some(Gender::Male),
                (None, "f") => Some(Gender::Female),
                (None, "other" | "nb" | "nonbinary" | "skip") => Some(Gender::Other),
                _ if m.has_any(&["non binary", "prefer not", "rather not"]) => Some(Gender::Other),
                _ => None,
            };
        }
        Field::GenderPref if patch.gender_pref.is_none() => {
            patch.gender_pref = match (choice, first) {
                (Some(n), _) => n.checked_sub(1).and_then(|i| GenderPref::ALL.get(i)).copied(),
                (None, "any" | "anyone" | "either" | "both" | "no" | "none" | "nope" | "skip") => Some(GenderPref::Any),
                (None, "same") => Some(GenderPref::Same),
                _ if m.has_any(&["doesnt matter", "does not matter", "dont care", "no preference", "not really"]) => {
                    Some(GenderPref::Any)
                }
                _ => None,
            };
        }
        Field::City if patch.city.is_none() => {
            // "jhansi 12k": the name is the words that are not an amount.
            let name: Vec<&str> =
                m.words.iter().map(String::as_str).filter(|w| w.chars().all(char::is_alphabetic)).collect();
            patch.city = City::parse(&name.join(" "));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> ProfilePatch {
        extract(text, None)
    }

    #[test]
    fn a_full_first_message_is_read_in_one_pass() {
        let patch = read("need a room in koramangala blr, 18k max, i sleep by 11, veg, no smoking, clean freak. 26M");
        assert_eq!(patch.intent, Some(Intent::NeedsRoom));
        assert_eq!(patch.city, City::parse("bengaluru"));
        assert_eq!(patch.area.as_deref(), Some("Koramangala"));
        assert_eq!((patch.budget_min, patch.budget_max), (None, Some(18_000)));
        assert_eq!(patch.gender, Some(Gender::Male));
        assert_eq!(
            patch.lifestyle,
            Lifestyle {
                sleep: Some(Sleep::Early),
                cleanliness: Some(Cleanliness::VeryTidy),
                food: Some(Food::Veg),
                smoking: Some(Smoking::No),
                ..Lifestyle::default()
            }
        );
    }

    #[test]
    fn intent_follows_what_the_person_lacks() {
        let cases = [
            ("I have a room in my 2bhk", Intent::HasRoom),
            ("Looking for a flatmate for our flat", Intent::HasRoom),
            ("need a flatmate", Intent::HasRoom),
            ("need a flat near office", Intent::NeedsRoom),
            ("looking for a room", Intent::NeedsRoom),
            ("moving to pune next month", Intent::NeedsRoom),
            ("want to team up and find a place", Intent::TeamUp),
            ("looking for a flat together with someone", Intent::TeamUp),
        ];
        for (text, expected) in cases {
            assert_eq!(read(text).intent, Some(expected), "{text}");
        }
        assert_eq!(read("hello").intent, None);
    }

    #[test]
    fn amounts_are_read_in_every_common_spelling() {
        type Range = (Option<u32>, Option<u32>);
        let cases: [(&str, Range); 12] = [
            ("budget 18k", (None, Some(18_000))),
            ("18,000", (None, Some(18_000))),
            ("₹18000 per month", (None, Some(18_000))),
            ("rs 9500", (None, Some(9_500))),
            ("17.5k", (None, Some(17_500))),
            ("15-20k", (Some(15_000), Some(20_000))),
            ("15k to 20k", (Some(15_000), Some(20_000))),
            ("between 12000 and 15000", (Some(12_000), Some(15_000))),
            ("upto 25 thousand", (None, Some(25_000))),
            ("rent is 22000", (None, Some(22_000))),
            ("max 8k", (None, Some(8_000))),
            ("20 K", (None, Some(20_000))),
        ];
        for (text, expected) in cases {
            let patch = read(text);
            assert_eq!((patch.budget_min, patch.budget_max), expected, "{text}");
        }
    }

    #[test]
    fn numbers_that_are_not_money_are_left_alone() {
        for text in [
            "i sleep by 11",
            "moving in 2026",
            "i am 26",
            "call me on 9876543210",
            "2 bhk",
            "from 1 december 2026",
            "budget 900",
            "10 lakh package",
        ] {
            let patch = read(text);
            assert_eq!((patch.budget_min, patch.budget_max), (None, None), "{text}");
        }
    }

    #[test]
    fn a_bare_number_is_thousands_only_when_budget_was_asked() {
        assert_eq!(extract("18", Some(Field::Budget)).budget_max, Some(18_000));
        assert_eq!(extract("18", None).budget_max, None);
        assert_eq!(extract("18", Some(Field::Gender)).budget_max, None);
        assert_eq!(extract("i sleep by 11 and wake at 7", Some(Field::Budget)).budget_max, None);
    }

    #[test]
    fn own_gender_and_wanted_gender_are_kept_apart() {
        let own = read("I am a female working in IT");
        assert_eq!((own.gender, own.gender_pref), (Some(Gender::Female), None));

        let wanted = read("looking for a female flatmate");
        assert_eq!((wanted.gender, wanted.gender_pref), (None, Some(GenderPref::Same)));

        let both = read("im a guy, male only please");
        assert_eq!((both.gender, both.gender_pref), (Some(Gender::Male), Some(GenderPref::Same)));

        assert_eq!(read("girls only").gender, None);
        assert_eq!(read("girls only").gender_pref, Some(GenderPref::Same));
        assert_eq!(read("any gender is fine").gender_pref, Some(GenderPref::Any));
        assert_eq!(read("24f").gender, Some(Gender::Female));
        assert_eq!(read("the 2f flat").gender, None);
    }

    #[test]
    fn habits_are_read_from_plain_phrases() {
        let patch =
            read("Night owl, non-veg, I smoke outside only on the balcony, have a cat, WFH, social drinker, messy");
        assert_eq!(
            patch.lifestyle,
            Lifestyle {
                sleep: Some(Sleep::Late),
                cleanliness: Some(Cleanliness::Relaxed),
                food: Some(Food::NonVeg),
                smoking: Some(Smoking::Outside),
                drinking: Some(Drinking::Social),
                pets: Some(Pets::HasPet),
                work: Some(WorkMode::Home),
                guests: None,
            }
        );
        assert_eq!(read("don't smoke, eggetarian, sleep at 1am").lifestyle.smoking, Some(Smoking::No));
        assert_eq!(read("eggetarian").lifestyle.food, Some(Food::Egg));
        assert_eq!(read("sleep at 1am").lifestyle.sleep, Some(Sleep::Late));
        assert_eq!(read("vegetable seller").lifestyle.food, None);
    }

    #[test]
    fn a_city_is_found_anywhere_and_an_area_only_before_it() {
        let patch = read("room near hsr layout bangalore for 15k");
        assert_eq!(patch.city, City::parse("blr"));
        assert_eq!(patch.area.as_deref(), Some("Hsr Layout"));

        assert_eq!(read("navi mumbai, 12k").city.unwrap().as_str(), "Navi Mumbai");
        assert_eq!(read("need a room in pune").area, None);
        assert_eq!(read("i work in a bank in mumbai").area, None, "'a bank' is not an area");
        assert_eq!(read("nothing here").city, None);
    }

    #[test]
    fn numbered_replies_answer_the_question_that_was_asked() {
        assert_eq!(extract("2", Some(Field::Intent)).intent, Some(Intent::NeedsRoom));
        assert_eq!(extract("3", Some(Field::Gender)).gender, Some(Gender::Other));
        assert_eq!(extract("1", Some(Field::GenderPref)).gender_pref, Some(GenderPref::Any));
        assert_eq!(extract("9", Some(Field::Gender)).gender, None);
        assert_eq!(extract("0", Some(Field::Gender)).gender, None);
        assert_eq!(extract("2", None), ProfilePatch::default());
    }

    #[test]
    fn one_word_replies_count_only_for_their_question() {
        assert_eq!(extract("f", Some(Field::Gender)).gender, Some(Gender::Female));
        assert_eq!(extract("no", Some(Field::GenderPref)).gender_pref, Some(GenderPref::Any));
        assert_eq!(extract("doesn't matter", Some(Field::GenderPref)).gender_pref, Some(GenderPref::Any));
        assert_eq!(extract("Jhansi", Some(Field::City)).city.unwrap().as_str(), "Jhansi");
        assert_eq!(extract("Jhansi", None).city, None, "an unknown name is a city only when asked");
        assert_eq!(extract("no", Some(Field::City)).gender_pref, None);
    }

    #[test]
    fn empty_and_odd_input_yields_an_empty_patch() {
        for text in ["", "   ", "👍", "???", "\n\t"] {
            assert!(read(text).is_empty(), "{text:?}");
        }
    }
}

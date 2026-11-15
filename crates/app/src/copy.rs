//! Every word the bot says, in one place.
//!
//! WhatsApp renders `*bold*` and `_italic_`. Messages are short, say one
//! thing, and always tell the person what to send next.

use crate::model::{Profile, Settings};
use chrono::Datelike;
use domain::matching::{self, Explanation};
use domain::{
    Cleanliness, Conversation, Drinking, Field, Food, Gender, GenderPref, Guests, Intent, Lifestyle, Pets, ProfileData,
    PublicCode, Sleep, Smoking, WorkMode,
};

pub fn welcome(settings: &Settings) -> String {
    format!(
        "Hi! I'm Rentlo. I find flatmates who live like you.\n\n\
         Tell me in one message:\n\
         • do you *have a room* or *need one*\n\
         • city and area\n\
         • monthly budget\n\
         • how you live: sleep time, cleanliness, food, smoking, pets\n\n\
         Example: _Need a room in Koramangala Bengaluru, 18k, sleep by 11, veg, non-smoker, very tidy_\n\n\
         By continuing you agree to {}",
        settings.privacy_url()
    )
}

/// The first reply to someone who came from the map asking for one profile.
pub fn welcome_for(code: &PublicCode, settings: &Settings) -> String {
    format!(
        "Hi! I'm Rentlo. I'll connect you with *{code}* as soon as I know a little about you, \
         so they can see you are a fit too.\n\n\
         Tell me in one message: do you *have a room* or *need one*, your city and area, \
         monthly budget, and how you live (sleep time, cleanliness, food, smoking, pets).\n\n\
         By continuing you agree to {}",
        settings.privacy_url()
    )
}

/// The question for the fields still missing.
pub fn ask(fields: &[Field], chat: &Conversation) -> String {
    let has_room = chat.draft.intent == Some(Intent::HasRoom);
    match fields {
        [Field::City, Field::Budget] => {
            if has_room {
                "Which city and area is the room in, and what is the monthly rent?\n\nExample: _Koramangala Bengaluru, 16k_".into()
            } else {
                "Which city and area do you want, and what is your monthly budget?\n\nExample: _Koramangala Bengaluru, 18k_".into()
            }
        }
        [field, ..] => {
            let question = question(*field, has_room);
            // A second ask means the first reply was not understood. Say so,
            // or the same question twice reads as a broken bot.
            if chat.times_asked(*field) > 1 {
                format!("Sorry, I did not get that.\n\n{question}")
            } else {
                question.into()
            }
        }
        [] => help(),
    }
}

fn question(field: Field, has_room: bool) -> &'static str {
    match field {
        Field::Intent => {
            "What brings you here?\n\n\
             *1.* I have a room and need a flatmate\n\
             *2.* I need a room\n\
             *3.* I want to team up with someone and find a flat\n\n\
             Reply with a number."
        }
        Field::City => "Which city, and which area?\n\nExample: _HSR Layout Bengaluru_",
        Field::Budget if has_room => "What is the monthly rent for the room?\n\nExample: _16k_",
        Field::Budget => "What is the most you can pay per month?\n\nExample: _18k_",
        Field::Gender => "Your gender?\n\n*1.* Male\n*2.* Female\n*3.* Other, or rather not say",
        Field::GenderPref => "Any preference for your flatmate's gender?\n\n*1.* Any\n*2.* Same as mine",
    }
}

pub fn ask_habits() -> String {
    "Last one, and it is what makes a match good. How do you live?\n\n\
     Sleep time, cleanliness, food, smoking, pets, guests. One line is enough.\n\n\
     Example: _night owl, tidy, non-veg, smoke outside, love dogs_\n\n\
     Or reply *skip*."
        .into()
}

pub fn live(profile: &Profile) -> String {
    format!(
        "You are live. ✅\n\n{}\n\nYour code is *{}*. To change anything, just tell me.",
        summary(&profile.data),
        profile.code
    )
}

pub fn updated(profile: &Profile) -> String {
    format!("Updated. ✅\n\n{}", summary(&profile.data))
}

pub fn own_profile(profile: &Profile, paused: bool) -> String {
    let state = if paused { "paused" } else { "live" };
    format!(
        "Your profile ({state}) · *{}*\n\n{}\n\nTo change anything, just tell me. Example: _budget 20k_",
        profile.code,
        summary(&profile.data)
    )
}

pub fn no_matches(city: &str) -> String {
    format!("No match in {city} yet. I will message you the moment one appears.")
}

pub fn no_more_matches() -> String {
    "That is everyone for now. I will message you when someone new fits.".into()
}

pub fn pin_hint() -> String {
    "Tip: send me a 📍 location pin of the area you prefer. You then appear on the live map, \
     and I can match you by distance. Only the area is shown, never the exact spot."
        .into()
}

pub fn pin_saved_before_live() -> String {
    "Got your area. 📍 Now, back to my last question.".into()
}

pub fn pin_outside_india() -> String {
    "That pin is outside India. Send a 📍 pin of the area where you want to live.".into()
}

/// The first reply to someone whose first message already described them.
pub fn hello(settings: &Settings) -> String {
    format!("Hi! I'm Rentlo. I find flatmates who live like you. By continuing you agree to {}", settings.privacy_url())
}

pub fn phone_saved() -> String {
    "Saved your number. Matches can reach you on it after you both say yes.".into()
}

/// The match cards for one reply, numbered from 1, with how to answer.
pub fn cards(viewer: &ProfileData, others: &[(u8, &Profile)]) -> String {
    let mut text = match others.len() {
        1 => "Your best match:\n\n".to_owned(),
        n => format!("Your top {n} matches:\n\n"),
    };
    for (index, (score, other)) in others.iter().enumerate() {
        text.push_str(&format!("*{}.* ", index + 1));
        text.push_str(&card(viewer, other, *score));
        text.push_str("\n\n");
    }
    text.push_str("Reply *1 yes* to connect, *1 skip* to pass. Nobody sees your number until you both say yes.");
    text
}

/// An unprompted message about one new match.
pub fn alert(viewer: &ProfileData, other: &Profile, score: u8) -> String {
    format!("New match for you:\n\n{}\n\n{}", card(viewer, other, score), answer_by_code(&other.code))
}

/// Tells someone that a match said yes to them first.
pub fn like(viewer: &ProfileData, other: &Profile, score: u8) -> String {
    format!("Someone wants to connect with you:\n\n{}\n\n{}", card(viewer, other, score), answer_by_code(&other.code))
}

fn answer_by_code(code: &PublicCode) -> String {
    format!("Reply *yes {code}* to connect, *skip {code}* to pass.")
}

/// One profile as seen by `viewer`. No name, no number.
fn card(viewer: &ProfileData, other: &Profile, score: u8) -> String {
    let Explanation { fits, watch } = matching::explain(viewer, &other.data);
    let mut lines = vec![format!("{score}% match · {}", other.code), headline(&other.data)];
    lines.extend(person_line(&other.data));
    if !fits.is_empty() {
        lines.push(format!("✅ {}", fits.join(", ")));
    }
    if !watch.is_empty() {
        lines.push(format!("⚠️ {}", watch.join(", ")));
    }
    lines.join("\n")
}

/// After a mutual yes: who they are and how to reach them.
pub fn intro(other_name: Option<&str>, other: &Profile, other_phone: Option<&str>) -> String {
    let name = other_name.unwrap_or("Your match");
    let contact = match other_phone {
        Some(phone) => format!("Chat with them: https://wa.me/{phone}"),
        None => "I have shared your number with them. They will message you.".to_owned(),
    };
    format!(
        "It's a match! 🎉\n\n*{name}* ({}) said yes too.\n{}\n\n{contact}\n\n\
         Stay safe: meet in a public place first, see the flat before you pay anything, \
         and send *report {}* if something feels wrong.",
        other.code,
        headline(&other.data),
        other.code
    )
}

pub fn liked() -> String {
    "Done. I have told them. If they say yes too, I will connect you both.".into()
}

pub fn skipped() -> String {
    "Skipped. They will not be told.".into()
}

pub fn already_decided() -> String {
    "You have already answered that one. Send *matches* to see who else fits.".into()
}

pub fn unknown_card() -> String {
    "I am not sure which match you mean. Send *matches* to see them again.".into()
}

pub fn finish_first() -> String {
    "Let's finish your profile first. It takes a minute.".into()
}

pub fn interest_unavailable(code: &PublicCode) -> String {
    format!("*{code}* is no longer available. Send *matches* to see who fits you now.")
}

pub fn interest_not_a_fit(code: &PublicCode) -> String {
    format!(
        "*{code}* is looking for something different (city, budget or who they want to live with), \
         so I cannot connect you. Send *matches* to see who fits."
    )
}

pub fn interest_own() -> String {
    "That is your own profile. 🙂".into()
}

pub fn paused() -> String {
    "Paused. You are hidden and will get no messages from me. Send *resume* to come back.".into()
}

pub fn is_paused() -> String {
    "Your profile is paused. Send *resume* to come back.".into()
}

pub fn resumed() -> String {
    "Welcome back. You are live again. ✅".into()
}

pub fn delete_confirm() -> String {
    "This erases your profile, your matches and your number from Rentlo. It cannot be undone.\n\n\
     Reply *delete yes* to confirm."
        .into()
}

pub fn deleted() -> String {
    "Everything is erased. If you need a flatmate again, just say hi.".into()
}

pub fn reported() -> String {
    "Thank you. I have recorded your report and hidden that profile from you.".into()
}

pub fn link(url: &str) -> String {
    format!("Your private link to the map and your matches. It works once, for 15 minutes:\n\n{url}")
}

pub fn unsupported() -> String {
    "I can read text and 📍 location pins only. Send *help* to see what I can do.".into()
}

pub fn not_understood() -> String {
    format!("I did not catch that.\n\n{}", help())
}

pub fn help() -> String {
    "What you can send me:\n\n\
     • *matches* to see who fits you\n\
     • *1 yes* or *1 skip* to answer a match\n\
     • a change in plain words, like _budget 20k_ or _I have a cat now_\n\
     • 📍 a location pin of your preferred area\n\
     • *profile* to see what I know about you\n\
     • *link* for the live map\n\
     • *pause* / *resume*\n\
     • *report CODE reason* if someone behaves badly\n\
     • *delete* to erase everything"
        .into()
}

/// A profile in two lines: the facts, then the habits.
pub fn summary(data: &ProfileData) -> String {
    let mut lines = vec![format!("• {}", headline(data))];
    lines.extend(person_line(data).map(|line| format!("• {line}")));
    lines.join("\n")
}

/// "Room available · Koramangala, Bengaluru · ₹16,000/month · from 1 Dec"
fn headline(data: &ProfileData) -> String {
    let intent = match data.intent {
        Intent::HasRoom => "Room available",
        Intent::NeedsRoom => "Looking for a room",
        Intent::TeamUp => "Looking to team up",
    };
    let place = match &data.area {
        Some(area) => format!("{area}, {}", data.city),
        None => data.city.to_string(),
    };
    let budget = match (data.intent, data.budget.min(), data.budget.max()) {
        (Intent::HasRoom, _, rent) => format!("{}/month", rupees(rent)),
        (_, 0, max) => format!("up to {}", rupees(max)),
        (_, min, max) => format!("{} to {}", rupees(min), rupees(max)),
    };
    let mut parts = vec![intent.to_owned(), place, budget];
    if let Some(date) = data.move_in {
        parts.push(format!("from {} {}", date.day(), date.format("%b")));
    }
    parts.join(" · ")
}

/// "Female, same gender only · sleeps early · very tidy · veg"
fn person_line(data: &ProfileData) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    match (data.gender, data.gender_pref) {
        (Gender::Male, GenderPref::Any) => parts.push("Male"),
        (Gender::Female, GenderPref::Any) => parts.push("Female"),
        (Gender::Male, GenderPref::Same) => parts.push("Male, male flatmates only"),
        (Gender::Female, GenderPref::Same) => parts.push("Female, female flatmates only"),
        (Gender::Other, GenderPref::Any) => {}
        (Gender::Other, GenderPref::Same) => parts.push("Same gender only"),
    }
    parts.extend(habits(&data.lifestyle));
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn habits(lifestyle: &Lifestyle) -> Vec<&'static str> {
    let Lifestyle { sleep, cleanliness, food, smoking, drinking, pets, guests, work } = lifestyle;
    [
        sleep.map(|v| match v {
            Sleep::Early => "sleeps early",
            Sleep::Flexible => "flexible sleep hours",
            Sleep::Late => "night owl",
        }),
        cleanliness.map(|v| match v {
            Cleanliness::Relaxed => "relaxed about cleaning",
            Cleanliness::Tidy => "tidy",
            Cleanliness::VeryTidy => "very tidy",
        }),
        food.map(|v| match v {
            Food::Veg => "veg",
            Food::Egg => "eggetarian",
            Food::NonVeg => "non-veg",
        }),
        smoking.map(|v| match v {
            Smoking::No => "non-smoker",
            Smoking::Outside => "smokes outside only",
            Smoking::Yes => "smoker",
        }),
        drinking.map(|v| match v {
            Drinking::No => "does not drink",
            Drinking::Social => "drinks socially",
            Drinking::Yes => "drinks",
        }),
        pets.map(|v| match v {
            Pets::NoPets => "no pets",
            Pets::FineWithPets => "fine with pets",
            Pets::HasPet => "has a pet",
        }),
        guests.map(|v| match v {
            Guests::Rarely => "rarely has guests",
            Guests::Sometimes => "sometimes has guests",
            Guests::Often => "often has guests",
        }),
        work.map(|v| match v {
            WorkMode::Home => "works from home",
            WorkMode::Hybrid => "hybrid work",
            WorkMode::Office => "goes to office",
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Rupees with Indian digit grouping: 18,000 and 1,20,000.
pub fn rupees(amount: u32) -> String {
    let digits = amount.to_string();
    let (head, last_three) = digits.split_at(digits.len().saturating_sub(3));
    let mut groups: Vec<String> = Vec::new();
    let head_chars: Vec<char> = head.chars().collect();
    for chunk in head_chars.rchunks(2) {
        groups.push(chunk.iter().collect());
    }
    groups.reverse();
    groups.push(last_three.to_owned());
    format!("₹{}", groups.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Budget, City, ProfileId, ProfileStatus, UserId};
    use uuid::Uuid;

    fn profile(intent: Intent, min: u32, max: u32) -> Profile {
        Profile {
            id: ProfileId(Uuid::from_u128(1)),
            user_id: UserId(Uuid::from_u128(2)),
            code: PublicCode::parse("R7K2Q9").unwrap(),
            status: ProfileStatus::Active,
            data: ProfileData {
                intent,
                city: City::parse("blr").unwrap(),
                area: Some("Koramangala".into()),
                pin: None,
                budget: Budget::new(min, max).unwrap(),
                move_in: chrono::NaiveDate::from_ymd_opt(2026, 12, 1),
                gender: Gender::Female,
                gender_pref: GenderPref::Same,
                lifestyle: Lifestyle {
                    sleep: Some(Sleep::Early),
                    cleanliness: Some(Cleanliness::VeryTidy),
                    pets: Some(Pets::HasPet),
                    ..Lifestyle::default()
                },
                note: None,
            },
        }
    }

    #[test]
    fn rupees_use_indian_grouping() {
        let cases = [
            (0, "₹0"),
            (950, "₹950"),
            (1_000, "₹1,000"),
            (18_000, "₹18,000"),
            (120_000, "₹1,20,000"),
            (12_345_678, "₹1,23,45,678"),
        ];
        for (amount, text) in cases {
            assert_eq!(rupees(amount), text);
        }
    }

    #[test]
    fn a_summary_states_facts_then_habits() {
        assert_eq!(
            summary(&profile(Intent::HasRoom, 16_000, 16_000).data),
            "• Room available · Koramangala, Bengaluru · ₹16,000/month · from 1 Dec\n\
             • Female, female flatmates only · sleeps early · very tidy · has a pet"
        );
        assert!(summary(&profile(Intent::NeedsRoom, 0, 18_000).data).contains("up to ₹18,000"));
        assert!(summary(&profile(Intent::TeamUp, 12_000, 18_000).data).contains("₹12,000 to ₹18,000"));
    }

    #[test]
    fn a_card_carries_the_code_and_reasons_but_no_identity() {
        let other = profile(Intent::HasRoom, 16_000, 16_000);
        let mut viewer = profile(Intent::NeedsRoom, 0, 18_000).data;
        viewer.lifestyle.pets = Some(Pets::NoPets);
        let text = cards(&viewer, &[(87, &other)]);
        assert!(text.contains("*1.* 87% match · R7K2Q9"), "{text}");
        assert!(text.contains("✅ both sleep early, both very tidy"), "{text}");
        assert!(text.contains("⚠️ they have a pet, you prefer none"), "{text}");
        assert!(text.contains("Reply *1 yes*"), "{text}");
        assert!(!text.contains("wa.me"), "{text}");
    }

    #[test]
    fn an_alert_is_answered_by_code_because_it_has_no_list() {
        let other = profile(Intent::HasRoom, 16_000, 16_000);
        let text = alert(&profile(Intent::NeedsRoom, 0, 18_000).data, &other, 80);
        assert!(text.contains("Reply *yes R7K2Q9*"), "{text}");
    }

    #[test]
    fn an_intro_gives_the_contact_and_the_safety_note() {
        let other = profile(Intent::HasRoom, 16_000, 16_000);
        let text = intro(Some("Asha"), &other, Some("919876543210"));
        assert!(text.contains("*Asha* (R7K2Q9)"), "{text}");
        assert!(text.contains("https://wa.me/919876543210"), "{text}");
        assert!(text.contains("report R7K2Q9"), "{text}");
        assert!(intro(None, &other, None).contains("They will message you"));
    }

    #[test]
    fn a_repeated_question_admits_the_miss() {
        let mut chat = Conversation::default();
        chat.absorb(Default::default());
        assert!(ask(&[Field::Intent], &chat).starts_with("What brings you here?"));
        chat.absorb(Default::default());
        assert!(ask(&[Field::Intent], &chat).starts_with("Sorry, I did not get that."));
    }
}

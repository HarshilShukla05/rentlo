//! The few fixed things a person can tell the bot to do. Anything that is not
//! a command is free text about themselves.

use crate::ids::PublicCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Greeting,
    Help,
    Pause,
    Resume,
    /// Asks to erase everything. Nothing happens until [`Command::DeleteConfirm`].
    Delete,
    DeleteConfirm,
    Matches,
    Profile,
    /// Asks for the private link to the web page.
    Link,
    Decision {
        target: Target,
        accept: bool,
    },
    /// "I like R7K2Q": sent by the map's button, or typed.
    Interest(PublicCode),
    Report {
        code: PublicCode,
        reason: String,
    },
}

/// Which match card a decision is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The card's number in the last list shown, starting at 1.
    Index(usize),
    Code(PublicCode),
    /// A bare "yes" or "skip": valid when exactly one card is on screen.
    Sole,
}

const GREETINGS: &[&str] = &["hi", "hii", "hiii", "hello", "hey", "heya", "hola", "namaste", "start", "yo"];
const ACCEPT: &[&str] = &["yes", "y", "ok", "accept", "like", "interested"];
const DECLINE: &[&str] = &["skip", "no", "n", "pass", "nope", "reject"];

/// The command in `text`, or `None` when it is free text.
pub fn parse(text: &str) -> Option<Command> {
    let lower = text.to_lowercase();
    let words: Vec<&str> =
        lower.split(|c: char| !(c.is_alphanumeric() || c == '?')).filter(|w| !w.is_empty()).collect();
    let code_at = |i: usize| words.get(i).and_then(|w| PublicCode::parse(w));

    match words.as_slice() {
        [] => None,
        ["report", code, ..] => PublicCode::parse(code).map(|code| Command::Report {
            code,
            reason: text.split_whitespace().skip(2).collect::<Vec<&str>>().join(" "),
        }),
        ["help" | "?" | "menu" | "commands" | "options"] => Some(Command::Help),
        ["pause" | "stop" | "unsubscribe"] => Some(Command::Pause),
        ["resume" | "unpause" | "restart"] => Some(Command::Resume),
        ["delete", "yes"] => Some(Command::DeleteConfirm),
        ["delete"] | ["delete", "my", "data" | "profile" | "account"] | ["delete", "account"] => Some(Command::Delete),
        ["matches" | "match" | "more"] | ["show", "matches"] | ["my", "matches"] => Some(Command::Matches),
        ["profile" | "me"] | ["my", "profile"] => Some(Command::Profile),
        ["link" | "web" | "map" | "edit"] => Some(Command::Link),
        ["yes"] => Some(Command::Decision { target: Target::Sole, accept: true }),
        ["skip"] => Some(Command::Decision { target: Target::Sole, accept: false }),
        [a, b] => decision(a, b).or_else(|| decision(b, a)).or_else(|| greeting(&words)),
        // "Hi Rentlo, I like R7K2Q": a code anywhere, with a word of interest.
        _ if words.iter().any(|w| matches!(*w, "like" | "interested" | "connect")) => {
            (0..words.len()).find_map(code_at).map(Command::Interest).or_else(|| greeting(&words))
        }
        _ => greeting(&words),
    }
}

fn decision(target: &str, verdict: &str) -> Option<Command> {
    let accept = if ACCEPT.contains(&verdict) {
        true
    } else if DECLINE.contains(&verdict) {
        false
    } else {
        return None;
    };
    let target = match target.parse::<usize>() {
        Ok(index) if (1..=9).contains(&index) => Target::Index(index),
        Ok(_) => return None,
        Err(_) => Target::Code(PublicCode::parse(target)?),
    };
    Some(Command::Decision { target, accept })
}

/// A greeting and nothing else: "hi", "hello there", "hey rentlo".
fn greeting(words: &[&str]) -> Option<Command> {
    let only_greeting = words.len() <= 2
        && words.first().is_some_and(|w| GREETINGS.contains(w))
        && words.get(1).is_none_or(|w| matches!(*w, "there" | "rentlo" | "bot" | "team") || GREETINGS.contains(w));
    only_greeting.then_some(Command::Greeting)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(text: &str) -> PublicCode {
        PublicCode::parse(text).unwrap()
    }

    #[test]
    fn greetings_are_short_and_carry_nothing_else() {
        for text in ["Hi", "hello!", "Hey there", "hi rentlo", "  HI  "] {
            assert_eq!(parse(text), Some(Command::Greeting), "{text}");
        }
        for text in ["hi i need a room in pune", "hello 18k", "high budget"] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn a_decision_names_its_card_by_number_or_code_in_either_order() {
        let yes = |target| Some(Command::Decision { target, accept: true });
        let no = |target| Some(Command::Decision { target, accept: false });
        assert_eq!(parse("1 yes"), yes(Target::Index(1)));
        assert_eq!(parse("Yes 2"), yes(Target::Index(2)));
        assert_eq!(parse("3 skip"), no(Target::Index(3)));
        assert_eq!(parse("no 1"), no(Target::Index(1)));
        assert_eq!(parse("r7k2q9 yes"), yes(Target::Code(code("R7K2Q9"))));
        assert_eq!(parse("yes"), yes(Target::Sole));
        assert_eq!(parse("skip"), no(Target::Sole));
        assert_eq!(parse("0 yes"), None);
        assert_eq!(parse("12 yes"), None);
    }

    #[test]
    fn a_bare_no_is_an_answer_not_a_command() {
        // "no" answers "any gender preference?"; it must reach the extractor.
        assert_eq!(parse("no"), None);
        assert_eq!(parse("2"), None);
        assert_eq!(parse("male, any"), None);
    }

    #[test]
    fn the_map_button_message_is_an_interest() {
        assert_eq!(parse("Hi Rentlo, I like R7K2Q9"), Some(Command::Interest(code("R7K2Q9"))));
        assert_eq!(parse("interested in r7k2q9 please connect"), Some(Command::Interest(code("R7K2Q9"))));
        assert_eq!(parse("i like tidy people and quiet evenings"), None);
    }

    #[test]
    fn delete_takes_two_steps() {
        assert_eq!(parse("delete"), Some(Command::Delete));
        assert_eq!(parse("Delete my data"), Some(Command::Delete));
        assert_eq!(parse("delete yes"), Some(Command::DeleteConfirm));
    }

    #[test]
    fn a_report_keeps_the_reason_as_typed() {
        assert_eq!(
            parse("report R7K2Q9 Asked for money upfront"),
            Some(Command::Report { code: code("R7K2Q9"), reason: "Asked for money upfront".into() })
        );
        assert_eq!(parse("report this person"), None);
    }

    #[test]
    fn single_word_commands_are_recognised() {
        assert_eq!(parse("help"), Some(Command::Help));
        assert_eq!(parse("?"), Some(Command::Help));
        assert_eq!(parse("PAUSE"), Some(Command::Pause));
        assert_eq!(parse("resume"), Some(Command::Resume));
        assert_eq!(parse("matches"), Some(Command::Matches));
        assert_eq!(parse("my profile"), Some(Command::Profile));
        assert_eq!(parse("link"), Some(Command::Link));
        assert_eq!(parse(""), None);
    }
}

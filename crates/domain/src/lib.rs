//! Rentlo's rules. Everything here is pure: no I/O, no clock, no randomness.
//! A function that needs the time or an id takes it as an argument, so every
//! rule can be tested with plain values.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[macro_use]
mod macros;

pub mod city;
pub mod command;
pub mod conversation;
pub mod extract;
pub mod geo;
pub mod ids;
pub mod matching;
pub mod pairing;
pub mod profile;

pub use city::City;
pub use command::Command;
pub use conversation::{Conversation, Stage, Step};
pub use geo::GeoPoint;
pub use ids::{MatchId, ProfileId, PublicCode, UserId};
pub use pairing::{Decided, Decision, Match, MatchStatus};
pub use profile::{
    Budget, Cleanliness, Drinking, Field, Food, Gender, GenderPref, Guests, Intent, Lifestyle, Pets, ProfileData,
    ProfilePatch, ProfileStatus, Sleep, Smoking, WorkMode,
};

/// A stored string that is not one of an enum's values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind}: {value:?}")]
pub struct UnknownValue {
    pub kind: &'static str,
    pub value: String,
}

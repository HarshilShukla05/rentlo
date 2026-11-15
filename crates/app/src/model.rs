//! The records use cases pass around.

use chrono::{DateTime, Utc};
use domain::{ProfileData, ProfileId, ProfileStatus, PublicCode, UserId};

/// A person, known by the WhatsApp address they wrote from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    /// Where replies go.
    pub jid: String,
    /// Digits with country code, when WhatsApp disclosed it. Shown to a
    /// match only after both said yes.
    pub phone: Option<String>,
    pub first_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub user_id: UserId,
    pub code: PublicCode,
    pub status: ProfileStatus,
    pub data: ProfileData,
}

/// Kinds of queued message. The kind decides nothing about delivery; it
/// exists so alerts can be counted against their daily cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    /// A reply to something the person just sent.
    Reply,
    /// A new match, sent unprompted. Capped per day.
    Alert,
    /// "Someone said yes to you". Not capped: it needs an answer.
    Like,
    /// Contact details after a mutual yes.
    Intro,
}

impl Topic {
    pub const fn as_str(self) -> &'static str {
        match self {
            Topic::Reply => "message.reply",
            Topic::Alert => "match.alert",
            Topic::Like => "match.like",
            Topic::Intro => "match.intro",
        }
    }
}

/// A message to put on the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub topic: Topic,
    /// Unique within the topic. Enqueueing the same key twice is a no-op,
    /// which is what makes a retried use case safe.
    pub dedupe_key: String,
    /// The recipient, for the daily cap and so that deleting an account
    /// drops its queued messages. `None` for the goodbye after a delete.
    pub user: Option<UserId>,
    pub jid: String,
    pub text: String,
    /// Not sent before this time.
    pub available_at: DateTime<Utc>,
}

/// A queued message claimed for sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub id: i64,
    pub jid: String,
    pub text: String,
    /// Sends already tried.
    pub attempts: u32,
}

/// Tunables that differ between production and tests.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Where the web app is served, without a trailing slash.
    pub public_url: String,
    /// The bot's own number in digits, for the web's "open WhatsApp" link.
    pub bot_phone: Option<String>,
}

impl Settings {
    pub fn privacy_url(&self) -> String {
        format!("{}/privacy", self.public_url)
    }
}

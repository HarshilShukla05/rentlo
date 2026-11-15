//! What the use cases need from the outside world.

use crate::model::{Outgoing, Profile, Queued, User};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    City, Conversation, Field, Match, MatchId, ProfileData, ProfileId, ProfilePatch, ProfileStatus, PublicCode, UserId,
};

/// The database failed. The cause is kept for the log; a use case cannot do
/// anything about it except stop, and the open transaction rolls back.
#[derive(Debug, thiserror::Error)]
#[error("store: {0}")]
pub struct StoreError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

pub type StoreResult<T> = Result<T, StoreError>;

/// Opens transactions. A use case does all its reads and writes in one, so
/// its state changes and the messages they cause commit together or not at all.
#[async_trait]
pub trait Store: Send + Sync {
    async fn begin(&self) -> StoreResult<Box<dyn Tx>>;
}

/// A rectangle on the map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

/// One open transaction. Dropping it without [`Tx::commit`] rolls it back.
#[async_trait]
pub trait Tx: Send {
    async fn commit(self: Box<Self>) -> StoreResult<()>;

    // People.

    /// The user for a WhatsApp address, created on first contact. A later
    /// call fills in a phone or name that was not known before.
    async fn upsert_user(
        &mut self,
        jid: &str,
        phone: Option<&str>,
        first_name: Option<&str>,
        now: DateTime<Utc>,
    ) -> StoreResult<User>;
    async fn user(&mut self, id: UserId) -> StoreResult<Option<User>>;
    /// Erases the user and everything that hangs off them.
    async fn delete_user(&mut self, id: UserId) -> StoreResult<()>;

    // Inbound messages.

    async fn inbound_seen(&mut self, message_id: &str) -> StoreResult<bool>;
    /// Records a message id. False when it was already recorded: WhatsApp
    /// delivered the message twice, and the caller must do nothing.
    async fn record_inbound(&mut self, message_id: &str, user: UserId, now: DateTime<Utc>) -> StoreResult<bool>;
    async fn inbound_count_since(&mut self, user: UserId, since: DateTime<Utc>) -> StoreResult<u32>;

    // Chat state.

    /// The chat's state; a fresh one for a new user. With `lock`, the row
    /// stays locked until the transaction ends.
    async fn conversation(&mut self, user: UserId, lock: bool) -> StoreResult<Conversation>;
    async fn save_conversation(&mut self, user: UserId, chat: &Conversation, now: DateTime<Utc>) -> StoreResult<()>;

    // Profiles.

    async fn profile_of(&mut self, user: UserId) -> StoreResult<Option<Profile>>;
    async fn profile(&mut self, id: ProfileId) -> StoreResult<Option<Profile>>;
    async fn profile_by_code(&mut self, code: &PublicCode) -> StoreResult<Option<Profile>>;
    /// Creates or replaces the user's profile and makes it active.
    async fn save_profile(&mut self, user: UserId, data: &ProfileData, now: DateTime<Utc>) -> StoreResult<Profile>;
    async fn set_profile_status(&mut self, user: UserId, status: ProfileStatus, now: DateTime<Utc>) -> StoreResult<()>;
    /// Active profiles in the same city, of other users, that have no pair
    /// with `profile` yet. Most recently updated first.
    async fn candidates(&mut self, profile: &Profile, limit: u32) -> StoreResult<Vec<Profile>>;
    /// Active profiles with a pin, by their blurred position.
    async fn pins(&mut self, bbox: Option<BBox>, limit: u32) -> StoreResult<Vec<Profile>>;
    async fn city_counts(&mut self) -> StoreResult<Vec<(City, u32)>>;

    // Pairs.

    /// Creates the pair. `None` when the two already have one.
    async fn insert_match(
        &mut self,
        x: ProfileId,
        y: ProfileId,
        score: u8,
        now: DateTime<Utc>,
    ) -> StoreResult<Option<Match>>;
    async fn match_by_id(&mut self, id: MatchId) -> StoreResult<Option<Match>>;
    async fn match_between(&mut self, x: ProfileId, y: ProfileId) -> StoreResult<Option<Match>>;
    async fn save_decisions(&mut self, pair: &Match, now: DateTime<Utc>) -> StoreResult<()>;
    /// Removes the pairs of `profile` that nobody answered, before they are
    /// recomputed from an edited profile. Returns the other side of each, so
    /// they are not alerted twice about the same person.
    async fn delete_undecided_matches(&mut self, profile: ProfileId) -> StoreResult<Vec<ProfileId>>;
    /// The profile's open and mutual pairs with the other side's active
    /// profile, best score first.
    async fn matches_of(&mut self, profile: ProfileId) -> StoreResult<Vec<(Match, Profile)>>;

    // Outbox.

    /// Queues a message. False when its dedupe key was already queued.
    async fn enqueue(&mut self, message: &Outgoing, now: DateTime<Utc>) -> StoreResult<bool>;
    async fn alerts_since(&mut self, user: UserId, since: DateTime<Utc>) -> StoreResult<u32>;
    /// Locks the oldest message that is due, skipping any another worker holds.
    async fn claim_due(&mut self, now: DateTime<Utc>) -> StoreResult<Option<Queued>>;
    async fn mark_sent(&mut self, id: i64, now: DateTime<Utc>) -> StoreResult<()>;
    /// Records a failed send. `retry_at` is `None` to give up.
    async fn mark_failed(&mut self, id: i64, error: &str, retry_at: Option<DateTime<Utc>>) -> StoreResult<()>;

    // Web sign-in. Tokens are stored as hashes; the caller hashes.

    async fn create_magic_link(
        &mut self,
        user: UserId,
        token_hash: &[u8],
        expires_at: DateTime<Utc>,
    ) -> StoreResult<()>;
    /// Uses up a link. `None` when it is unknown, expired or already used.
    async fn take_magic_link(&mut self, token_hash: &[u8], now: DateTime<Utc>) -> StoreResult<Option<UserId>>;
    async fn create_session(&mut self, user: UserId, token_hash: &[u8], expires_at: DateTime<Utc>) -> StoreResult<()>;
    async fn session_user(&mut self, token_hash: &[u8], now: DateTime<Utc>) -> StoreResult<Option<UserId>>;

    // Safety.

    async fn insert_report(
        &mut self,
        reporter: UserId,
        profile: ProfileId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<()>;
}

/// Sends WhatsApp messages.
#[async_trait]
pub trait Messenger: Send + Sync {
    async fn send_text(&self, jid: &str, text: &str) -> Result<(), SendError>;
}

#[derive(Debug, thiserror::Error)]
#[error("send failed: {0}")]
pub struct SendError(pub String);

/// Reads a free-text message into profile fields.
///
/// It cannot fail: an implementation that calls a model falls back to the
/// fixed rules when the model does not answer.
#[async_trait]
pub trait ProfileExtractor: Send + Sync {
    /// `asked` is the field the bot's last message asked for. `today` lets
    /// "next month" become a date.
    async fn extract(&self, text: &str, asked: Option<Field>, today: NaiveDate) -> ProfilePatch;
}

/// The extractor with no model behind it: the fixed rules alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct RulesExtractor;

#[async_trait]
impl ProfileExtractor for RulesExtractor {
    async fn extract(&self, text: &str, asked: Option<Field>, _today: NaiveDate) -> ProfilePatch {
        domain::extract::extract(text, asked)
    }
}

//! PostgreSQL behind the [`app::Store`] port.
//!
//! Queries are checked at run time, not compile time, so the crate builds
//! with no database. Every query is exercised by the tests in `tests/`,
//! which run against a real PostgreSQL.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use app::model::{Outgoing, Profile, Queued, User};
use app::ports::{BBox, StoreError, StoreResult, Tx};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    Budget, City, Conversation, Decision, GeoPoint, Intent, Lifestyle, Match, MatchId, ProfileData, ProfileId,
    ProfileStatus, PublicCode, UserId,
};
use sqlx::postgres::{PgListener, PgPool, PgPoolOptions, PgRow};
// Every query built at run time only splices in the fixed column lists
// below (or a name checked to be a plain identifier). Values always go
// through bind parameters, never into the text.
use sqlx::{AssertSqlSafe, Postgres, Row, Transaction};
use std::time::Duration;
use uuid::Uuid;

/// Channels the database notifies on.
pub const OUTBOX_CHANNEL: &str = "outbox";
pub const PINS_CHANNEL: &str = "pins";

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub async fn connect(url: &str, max_connections: u32) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(10))
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    /// Brings the schema up to date. Safe to run from several processes at
    /// once: the migrator holds an advisory lock while it works.
    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        MIGRATOR.run(&self.pool).await
    }

    /// A connection that receives notifications on `channels`.
    pub async fn listen(&self, channels: &[&str]) -> Result<PgListener, sqlx::Error> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.listen_all(channels.iter().copied()).await?;
        Ok(listener)
    }

    /// Creates an empty database on the same server. For tests and first
    /// setup; the name must be a plain identifier.
    pub async fn create_database(&self, name: &str) -> Result<(), sqlx::Error> {
        let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !plain {
            return Err(sqlx::Error::Protocol(format!("not a plain database name: {name:?}")));
        }
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&self.pool).await.map(|_| ())
    }

    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await.map(|_| ())
    }

    pub async fn active_profile_count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM profiles WHERE status = 'active'").fetch_one(&self.pool).await
    }

    /// Deletes rows nobody needs any more: used links, ended sessions, old
    /// inbound ids, sent messages. Returns how many rows went.
    pub async fn prune(&self, now: DateTime<Utc>) -> Result<u64, sqlx::Error> {
        let week_ago = now - chrono::Duration::days(7);
        let mut removed = 0;
        for (sql, at) in [
            ("DELETE FROM magic_links WHERE expires_at < $1", now),
            ("DELETE FROM web_sessions WHERE expires_at < $1", now),
            ("DELETE FROM inbound_messages WHERE received_at < $1", week_ago),
            ("DELETE FROM outbox WHERE (sent_at < $1 OR failed_at < $1) AND topic <> 'match.alert'", week_ago),
            ("DELETE FROM outbox WHERE topic = 'match.alert' AND created_at < $1 AND sent_at IS NOT NULL", week_ago),
        ] {
            removed += sqlx::query(sql).bind(at).execute(&self.pool).await?.rows_affected();
        }
        Ok(removed)
    }
}

#[async_trait]
impl app::Store for PgStore {
    async fn begin(&self) -> StoreResult<Box<dyn Tx>> {
        let tx = self.pool.begin().await.map_err(fail)?;
        Ok(Box::new(PgTx { tx }))
    }
}

fn fail(error: impl std::error::Error + Send + Sync + 'static) -> StoreError {
    StoreError(Box::new(error))
}

/// A row that the code cannot read: a bug or a hand edit, never user input.
#[derive(Debug, thiserror::Error)]
#[error("corrupt row: {0}")]
struct Corrupt(String);

fn corrupt(what: impl std::fmt::Display) -> StoreError {
    fail(Corrupt(what.to_string()))
}

struct PgTx {
    tx: Transaction<'static, Postgres>,
}

const PROFILE_COLUMNS: &str = "p.id, p.user_id, p.code, p.status, p.intent, p.city, p.area, p.lat, p.lng, \
     p.budget_min, p.budget_max, p.move_in, p.gender, p.gender_pref, p.lifestyle, p.note";

fn profile_from(row: &PgRow) -> StoreResult<Profile> {
    let parse = |name: &str| row.try_get::<String, _>(name).map_err(fail);
    let lat: Option<f64> = row.try_get("lat").map_err(fail)?;
    let lng: Option<f64> = row.try_get("lng").map_err(fail)?;
    let pin = match (lat, lng) {
        (Some(lat), Some(lng)) => Some(GeoPoint::new(lat, lng).ok_or_else(|| corrupt("pin outside India"))?),
        _ => None,
    };
    let budget_min: i32 = row.try_get("budget_min").map_err(fail)?;
    let budget_max: i32 = row.try_get("budget_max").map_err(fail)?;
    let budget = Budget::new(u32::try_from(budget_min).map_err(corrupt)?, u32::try_from(budget_max).map_err(corrupt)?)
        .map_err(corrupt)?;
    let city = parse("city")?;
    let lifestyle: serde_json::Value = row.try_get("lifestyle").map_err(fail)?;
    let code = parse("code")?;
    Ok(Profile {
        id: ProfileId(row.try_get("id").map_err(fail)?),
        user_id: UserId(row.try_get("user_id").map_err(fail)?),
        code: PublicCode::parse(&code).ok_or_else(|| corrupt(format!("code {code}")))?,
        status: parse("status")?.parse().map_err(corrupt)?,
        data: ProfileData {
            intent: parse("intent")?.parse().map_err(corrupt)?,
            city: City::parse(&city).ok_or_else(|| corrupt(format!("city {city}")))?,
            area: row.try_get("area").map_err(fail)?,
            pin,
            budget,
            move_in: row.try_get::<Option<NaiveDate>, _>("move_in").map_err(fail)?,
            gender: parse("gender")?.parse().map_err(corrupt)?,
            gender_pref: parse("gender_pref")?.parse().map_err(corrupt)?,
            lifestyle: serde_json::from_value::<Lifestyle>(lifestyle).map_err(corrupt)?,
            note: row.try_get("note").map_err(fail)?,
        },
    })
}

/// Aliased, because the match query joins `profiles`, whose `id` would
/// otherwise shadow the match's.
const MATCH_COLUMNS: &str = "m.id AS match_id, m.profile_a, m.profile_b, m.score, m.decision_a, m.decision_b";

fn match_from(row: &PgRow) -> StoreResult<Match> {
    let decision = |name: &str| -> StoreResult<Option<Decision>> {
        let text: Option<String> = row.try_get(name).map_err(fail)?;
        text.map(|t| t.parse().map_err(corrupt)).transpose()
    };
    let score: i16 = row.try_get("score").map_err(fail)?;
    Ok(Match {
        id: MatchId(row.try_get("match_id").map_err(fail)?),
        a: ProfileId(row.try_get("profile_a").map_err(fail)?),
        b: ProfileId(row.try_get("profile_b").map_err(fail)?),
        score: u8::try_from(score).map_err(corrupt)?,
        decision_a: decision("decision_a")?,
        decision_b: decision("decision_b")?,
    })
}

fn user_from(row: &PgRow) -> StoreResult<User> {
    Ok(User {
        id: UserId(row.try_get("id").map_err(fail)?),
        jid: row.try_get("jid").map_err(fail)?,
        phone: row.try_get("phone").map_err(fail)?,
        first_name: row.try_get("first_name").map_err(fail)?,
    })
}

/// Money and counts fit `integer`; a value that does not is a bug upstream.
fn int(value: u32) -> StoreResult<i32> {
    i32::try_from(value).map_err(corrupt)
}

fn count(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[async_trait]
impl Tx for PgTx {
    async fn commit(self: Box<Self>) -> StoreResult<()> {
        self.tx.commit().await.map_err(fail)
    }

    async fn upsert_user(
        &mut self,
        jid: &str,
        phone: Option<&str>,
        first_name: Option<&str>,
        now: DateTime<Utc>,
    ) -> StoreResult<User> {
        let row = sqlx::query(
            "INSERT INTO users (id, jid, phone, first_name, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) \
             ON CONFLICT (jid) DO UPDATE SET \
                 phone = COALESCE(users.phone, EXCLUDED.phone), \
                 first_name = COALESCE(EXCLUDED.first_name, users.first_name), \
                 updated_at = EXCLUDED.updated_at \
             RETURNING id, jid, phone, first_name",
        )
        .bind(Uuid::now_v7())
        .bind(jid)
        .bind(phone)
        .bind(first_name)
        .bind(now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(fail)?;
        user_from(&row)
    }

    async fn user(&mut self, id: UserId) -> StoreResult<Option<User>> {
        let row = sqlx::query("SELECT id, jid, phone, first_name FROM users WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(fail)?;
        row.as_ref().map(user_from).transpose()
    }

    async fn delete_user(&mut self, id: UserId) -> StoreResult<()> {
        sqlx::query("DELETE FROM users WHERE id = $1").bind(id.as_uuid()).execute(&mut *self.tx).await.map_err(fail)?;
        Ok(())
    }

    async fn inbound_seen(&mut self, message_id: &str) -> StoreResult<bool> {
        let row = sqlx::query("SELECT 1 FROM inbound_messages WHERE message_id = $1")
            .bind(message_id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(row.is_some())
    }

    async fn record_inbound(&mut self, message_id: &str, user: UserId, now: DateTime<Utc>) -> StoreResult<bool> {
        let done = sqlx::query(
            "INSERT INTO inbound_messages (message_id, user_id, received_at) VALUES ($1, $2, $3) \
             ON CONFLICT (message_id) DO NOTHING",
        )
        .bind(message_id)
        .bind(user.as_uuid())
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(fail)?;
        Ok(done.rows_affected() == 1)
    }

    async fn inbound_count_since(&mut self, user: UserId, since: DateTime<Utc>) -> StoreResult<u32> {
        let n: i64 =
            sqlx::query_scalar("SELECT count(*) FROM inbound_messages WHERE user_id = $1 AND received_at >= $2")
                .bind(user.as_uuid())
                .bind(since)
                .fetch_one(&mut *self.tx)
                .await
                .map_err(fail)?;
        Ok(count(n))
    }

    async fn conversation(&mut self, user: UserId, lock: bool) -> StoreResult<Conversation> {
        if lock {
            // The user row always exists, unlike the chat row on a first
            // message, so it is what serialises two messages from one person.
            sqlx::query("SELECT 1 FROM users WHERE id = $1 FOR UPDATE")
                .bind(user.as_uuid())
                .execute(&mut *self.tx)
                .await
                .map_err(fail)?;
        }
        let state: Option<serde_json::Value> = sqlx::query_scalar("SELECT state FROM conversations WHERE user_id = $1")
            .bind(user.as_uuid())
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(fail)?;
        match state {
            Some(state) => serde_json::from_value(state).map_err(corrupt),
            None => Ok(Conversation::default()),
        }
    }

    async fn save_conversation(&mut self, user: UserId, chat: &Conversation, now: DateTime<Utc>) -> StoreResult<()> {
        let state = serde_json::to_value(chat).map_err(corrupt)?;
        sqlx::query(
            "INSERT INTO conversations (user_id, state, updated_at) VALUES ($1, $2, $3) \
             ON CONFLICT (user_id) DO UPDATE SET state = EXCLUDED.state, updated_at = EXCLUDED.updated_at",
        )
        .bind(user.as_uuid())
        .bind(state)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(fail)?;
        Ok(())
    }

    async fn profile_of(&mut self, user: UserId) -> StoreResult<Option<Profile>> {
        let sql = format!("SELECT {PROFILE_COLUMNS} FROM profiles p WHERE p.user_id = $1");
        let row =
            sqlx::query(AssertSqlSafe(sql)).bind(user.as_uuid()).fetch_optional(&mut *self.tx).await.map_err(fail)?;
        row.as_ref().map(profile_from).transpose()
    }

    async fn profile(&mut self, id: ProfileId) -> StoreResult<Option<Profile>> {
        let sql = format!("SELECT {PROFILE_COLUMNS} FROM profiles p WHERE p.id = $1");
        let row =
            sqlx::query(AssertSqlSafe(sql)).bind(id.as_uuid()).fetch_optional(&mut *self.tx).await.map_err(fail)?;
        row.as_ref().map(profile_from).transpose()
    }

    async fn profile_by_code(&mut self, code: &PublicCode) -> StoreResult<Option<Profile>> {
        let sql = format!("SELECT {PROFILE_COLUMNS} FROM profiles p WHERE p.code = $1");
        let row =
            sqlx::query(AssertSqlSafe(sql)).bind(code.as_str()).fetch_optional(&mut *self.tx).await.map_err(fail)?;
        row.as_ref().map(profile_from).transpose()
    }

    async fn save_profile(&mut self, user: UserId, data: &ProfileData, now: DateTime<Utc>) -> StoreResult<Profile> {
        // A code is drawn once, on the first save, and kept for good. A
        // collision is checked first rather than caught, because a failed
        // statement would abort the whole transaction.
        let code = loop {
            let candidate = PublicCode::from_entropy(rand::random());
            let taken = sqlx::query("SELECT 1 FROM profiles WHERE code = $1")
                .bind(candidate.as_str())
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(fail)?;
            if taken.is_none() {
                break candidate;
            }
        };
        let lifestyle = serde_json::to_value(&data.lifestyle).map_err(corrupt)?;
        let sql = format!(
            "INSERT INTO profiles AS p (id, user_id, code, status, intent, city, area, lat, lng, budget_min, \
                 budget_max, move_in, gender, gender_pref, lifestyle, note, created_at, updated_at) \
             VALUES ($1, $2, $3, 'active', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $16) \
             ON CONFLICT (user_id) DO UPDATE SET \
                 status = 'active', intent = EXCLUDED.intent, city = EXCLUDED.city, area = EXCLUDED.area, \
                 lat = EXCLUDED.lat, lng = EXCLUDED.lng, budget_min = EXCLUDED.budget_min, \
                 budget_max = EXCLUDED.budget_max, move_in = EXCLUDED.move_in, gender = EXCLUDED.gender, \
                 gender_pref = EXCLUDED.gender_pref, lifestyle = EXCLUDED.lifestyle, note = EXCLUDED.note, \
                 version = p.version + 1, updated_at = EXCLUDED.updated_at \
             RETURNING {PROFILE_COLUMNS}"
        );
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(Uuid::now_v7())
            .bind(user.as_uuid())
            .bind(code.as_str())
            .bind(data.intent.as_str())
            .bind(data.city.as_str())
            .bind(data.area.as_deref())
            .bind(data.pin.map(GeoPoint::lat))
            .bind(data.pin.map(GeoPoint::lng))
            .bind(int(data.budget.min())?)
            .bind(int(data.budget.max())?)
            .bind(data.move_in)
            .bind(data.gender.as_str())
            .bind(data.gender_pref.as_str())
            .bind(lifestyle)
            .bind(data.note.as_deref())
            .bind(now)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(fail)?;
        profile_from(&row)
    }

    async fn set_profile_status(&mut self, user: UserId, status: ProfileStatus, now: DateTime<Utc>) -> StoreResult<()> {
        sqlx::query("UPDATE profiles SET status = $2, updated_at = $3 WHERE user_id = $1")
            .bind(user.as_uuid())
            .bind(status.as_str())
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(())
    }

    async fn candidates(&mut self, profile: &Profile, limit: u32) -> StoreResult<Vec<Profile>> {
        let sql = format!(
            "SELECT {PROFILE_COLUMNS} FROM profiles p \
             WHERE p.status = 'active' AND p.city = $1 AND p.user_id <> $2 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM matches m \
                   WHERE m.profile_a = LEAST(p.id, $3) AND m.profile_b = GREATEST(p.id, $3)) \
             ORDER BY p.updated_at DESC LIMIT $4"
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(profile.data.city.as_str())
            .bind(profile.user_id.as_uuid())
            .bind(profile.id.as_uuid())
            .bind(i64::from(limit))
            .fetch_all(&mut *self.tx)
            .await
            .map_err(fail)?;
        rows.iter().map(profile_from).collect()
    }

    async fn pins(&mut self, bbox: Option<BBox>, limit: u32) -> StoreResult<Vec<Profile>> {
        let bbox = bbox.unwrap_or(BBox { south: -90.0, west: -180.0, north: 90.0, east: 180.0 });
        let sql = format!(
            "SELECT {PROFILE_COLUMNS} FROM profiles p \
             WHERE p.status = 'active' AND p.lat IS NOT NULL \
               AND p.lat BETWEEN $1 AND $2 AND p.lng BETWEEN $3 AND $4 \
             ORDER BY p.updated_at DESC LIMIT $5"
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(bbox.south)
            .bind(bbox.north)
            .bind(bbox.west)
            .bind(bbox.east)
            .bind(i64::from(limit))
            .fetch_all(&mut *self.tx)
            .await
            .map_err(fail)?;
        rows.iter().map(profile_from).collect()
    }

    async fn area_counts(&mut self, city: &City) -> StoreResult<Vec<(Option<String>, Intent, u32)>> {
        let rows: Vec<(Option<String>, String, i64)> = sqlx::query_as(
            "SELECT area, intent, count(*) FROM profiles WHERE city = $1 AND status = 'active' GROUP BY area, intent",
        )
        .bind(city.as_str())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(fail)?;
        rows.into_iter().map(|(area, intent, n)| Ok((area, intent.parse().map_err(corrupt)?, count(n)))).collect()
    }

    async fn city_counts(&mut self) -> StoreResult<Vec<(City, u32)>> {
        let rows = sqlx::query(
            "SELECT city, count(*) AS n FROM profiles WHERE status = 'active' GROUP BY city ORDER BY n DESC, city",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(fail)?;
        rows.iter()
            .map(|row| {
                let name: String = row.try_get("city").map_err(fail)?;
                let n: i64 = row.try_get("n").map_err(fail)?;
                let city = City::parse(&name).ok_or_else(|| corrupt(format!("city {name}")))?;
                Ok((city, count(n)))
            })
            .collect()
    }

    async fn insert_match(
        &mut self,
        x: ProfileId,
        y: ProfileId,
        score: u8,
        now: DateTime<Utc>,
    ) -> StoreResult<Option<Match>> {
        let (a, b) = domain::pairing::ordered(x, y);
        let sql = format!(
            "INSERT INTO matches AS m (id, profile_a, profile_b, score, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (profile_a, profile_b) DO NOTHING \
             RETURNING {MATCH_COLUMNS}"
        );
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(Uuid::now_v7())
            .bind(a.as_uuid())
            .bind(b.as_uuid())
            .bind(i16::from(score))
            .bind(now)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(fail)?;
        row.as_ref().map(match_from).transpose()
    }

    async fn match_by_id(&mut self, id: MatchId) -> StoreResult<Option<Match>> {
        let sql = format!("SELECT {MATCH_COLUMNS} FROM matches m WHERE m.id = $1 FOR UPDATE");
        let row =
            sqlx::query(AssertSqlSafe(sql)).bind(id.as_uuid()).fetch_optional(&mut *self.tx).await.map_err(fail)?;
        row.as_ref().map(match_from).transpose()
    }

    async fn match_between(&mut self, x: ProfileId, y: ProfileId) -> StoreResult<Option<Match>> {
        let (a, b) = domain::pairing::ordered(x, y);
        let sql =
            format!("SELECT {MATCH_COLUMNS} FROM matches m WHERE m.profile_a = $1 AND m.profile_b = $2 FOR UPDATE");
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(a.as_uuid())
            .bind(b.as_uuid())
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(fail)?;
        row.as_ref().map(match_from).transpose()
    }

    async fn save_decisions(&mut self, pair: &Match, now: DateTime<Utc>) -> StoreResult<()> {
        sqlx::query("UPDATE matches SET decision_a = $2, decision_b = $3, updated_at = $4 WHERE id = $1")
            .bind(pair.id.as_uuid())
            .bind(pair.decision_a.map(Decision::as_str))
            .bind(pair.decision_b.map(Decision::as_str))
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(())
    }

    async fn delete_undecided_matches(&mut self, profile: ProfileId) -> StoreResult<Vec<ProfileId>> {
        let others: Vec<Uuid> = sqlx::query_scalar(
            "DELETE FROM matches \
             WHERE (profile_a = $1 OR profile_b = $1) AND decision_a IS NULL AND decision_b IS NULL \
             RETURNING CASE WHEN profile_a = $1 THEN profile_b ELSE profile_a END",
        )
        .bind(profile.as_uuid())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(fail)?;
        Ok(others.into_iter().map(ProfileId).collect())
    }

    async fn matches_of(&mut self, profile: ProfileId) -> StoreResult<Vec<(Match, Profile)>> {
        let sql = format!(
            "SELECT {MATCH_COLUMNS}, {PROFILE_COLUMNS} FROM matches m \
             JOIN profiles p ON p.id = CASE WHEN m.profile_a = $1 THEN m.profile_b ELSE m.profile_a END \
             WHERE (m.profile_a = $1 OR m.profile_b = $1) AND p.status = 'active' \
               AND m.decision_a IS DISTINCT FROM 'skip' AND m.decision_b IS DISTINCT FROM 'skip' \
             ORDER BY m.score DESC, m.created_at"
        );
        let rows =
            sqlx::query(AssertSqlSafe(sql)).bind(profile.as_uuid()).fetch_all(&mut *self.tx).await.map_err(fail)?;
        rows.iter().map(|row| Ok((match_from(row)?, profile_from(row)?))).collect()
    }

    async fn enqueue(&mut self, message: &Outgoing, now: DateTime<Utc>) -> StoreResult<bool> {
        let done = sqlx::query(
            "INSERT INTO outbox (topic, dedupe_key, user_id, jid, body, available_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (topic, dedupe_key) DO NOTHING",
        )
        .bind(message.topic.as_str())
        .bind(&message.dedupe_key)
        .bind(message.user.map(UserId::as_uuid))
        .bind(&message.jid)
        .bind(&message.text)
        .bind(message.available_at)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(fail)?;
        Ok(done.rows_affected() == 1)
    }

    async fn alerts_since(&mut self, user: UserId, since: DateTime<Utc>) -> StoreResult<u32> {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox WHERE user_id = $1 AND topic = 'match.alert' AND created_at >= $2",
        )
        .bind(user.as_uuid())
        .bind(since)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(fail)?;
        Ok(count(n))
    }

    async fn claim_due(&mut self, now: DateTime<Utc>) -> StoreResult<Option<Queued>> {
        let row = sqlx::query(
            "SELECT id, jid, body, attempts FROM outbox \
             WHERE sent_at IS NULL AND failed_at IS NULL AND available_at <= $1 \
             ORDER BY available_at, id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(fail)?;
        let Some(row) = row else { return Ok(None) };
        let attempts: i32 = row.try_get("attempts").map_err(fail)?;
        Ok(Some(Queued {
            id: row.try_get("id").map_err(fail)?,
            jid: row.try_get("jid").map_err(fail)?,
            text: row.try_get("body").map_err(fail)?,
            attempts: u32::try_from(attempts).unwrap_or(0),
        }))
    }

    async fn mark_sent(&mut self, id: i64, now: DateTime<Utc>) -> StoreResult<()> {
        sqlx::query("UPDATE outbox SET sent_at = $2, attempts = attempts + 1 WHERE id = $1")
            .bind(id)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(())
    }

    async fn mark_failed(&mut self, id: i64, error: &str, retry_at: Option<DateTime<Utc>>) -> StoreResult<()> {
        sqlx::query(
            "UPDATE outbox SET attempts = attempts + 1, last_error = $2, \
                 available_at = COALESCE($3, available_at), \
                 failed_at = CASE WHEN $3::timestamptz IS NULL THEN now() END \
             WHERE id = $1",
        )
        .bind(id)
        .bind(error)
        .bind(retry_at)
        .execute(&mut *self.tx)
        .await
        .map_err(fail)?;
        Ok(())
    }

    async fn create_magic_link(
        &mut self,
        user: UserId,
        token_hash: &[u8],
        expires_at: DateTime<Utc>,
    ) -> StoreResult<()> {
        sqlx::query("INSERT INTO magic_links (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
            .bind(token_hash)
            .bind(user.as_uuid())
            .bind(expires_at)
            .execute(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(())
    }

    async fn take_magic_link(&mut self, token_hash: &[u8], now: DateTime<Utc>) -> StoreResult<Option<UserId>> {
        let user: Option<Uuid> = sqlx::query_scalar(
            "UPDATE magic_links SET used_at = $2 \
             WHERE token_hash = $1 AND used_at IS NULL AND expires_at > $2 RETURNING user_id",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(fail)?;
        Ok(user.map(UserId))
    }

    async fn create_session(&mut self, user: UserId, token_hash: &[u8], expires_at: DateTime<Utc>) -> StoreResult<()> {
        sqlx::query("INSERT INTO web_sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
            .bind(token_hash)
            .bind(user.as_uuid())
            .bind(expires_at)
            .execute(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(())
    }

    async fn delete_session(&mut self, token_hash: &[u8]) -> StoreResult<()> {
        sqlx::query("DELETE FROM web_sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(())
    }

    async fn session_user(&mut self, token_hash: &[u8], now: DateTime<Utc>) -> StoreResult<Option<UserId>> {
        let user: Option<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM web_sessions WHERE token_hash = $1 AND expires_at > $2")
                .bind(token_hash)
                .bind(now)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(fail)?;
        Ok(user.map(UserId))
    }

    async fn insert_report(
        &mut self,
        reporter: UserId,
        profile: ProfileId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<()> {
        sqlx::query("INSERT INTO reports (reporter_id, profile_id, reason, created_at) VALUES ($1, $2, $3, $4)")
            .bind(reporter.as_uuid())
            .bind(profile.as_uuid())
            .bind(reason)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(fail)?;
        Ok(())
    }
}

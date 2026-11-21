//! What the web app may read and do.
//!
//! Member activity and matches require a session from a private WhatsApp link.

use crate::Error;
use crate::model::{Profile, Settings, User};
use crate::ops::{self, Outcome};
use crate::policy;
use crate::ports::{BBox, Store};
use chrono::{DateTime, NaiveDate, Utc};
use domain::conversation::Stage;
use domain::{
    City, Gender, GenderPref, GeoPoint, Intent, Lifestyle, MatchId, MatchStatus, ProfilePatch, ProfileStatus,
    PublicCode, area, geo,
};
use serde::Serialize;
use std::sync::Arc;

/// Pins returned for one map view at most.
pub const MAX_PINS: u32 = 500;
const MAX_WEB_MATCHES: usize = 30;

pub struct Web {
    store: Arc<dyn Store>,
    settings: Settings,
}

/// One profile on the public map.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pin {
    pub code: PublicCode,
    pub intent: Intent,
    /// Blurred: the same spot inside the profile's grid cell, always.
    pub lat: f64,
    pub lng: f64,
    pub budget_max: u32,
    pub gender_pref: GenderPref,
    pub gender: Gender,
}

#[derive(Debug, Clone, Default)]
pub struct PinFilter {
    pub intent: Option<Intent>,
    /// Only profiles whose budget starts at or below this.
    pub budget_max: Option<u32>,
    pub gender: Option<Gender>,
}

/// What anyone may see of a profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Card {
    pub code: PublicCode,
    pub intent: Intent,
    pub city: String,
    pub area: Option<String>,
    pub budget_min: u32,
    pub budget_max: u32,
    pub move_in: Option<NaiveDate>,
    pub gender: Gender,
    pub gender_pref: GenderPref,
    pub lifestyle: Lifestyle,
    pub note: Option<String>,
}

impl From<&Profile> for Card {
    fn from(profile: &Profile) -> Self {
        let data = &profile.data;
        Self {
            code: profile.code.clone(),
            intent: data.intent,
            city: data.city.to_string(),
            area: data.area.clone(),
            budget_min: data.budget.min(),
            budget_max: data.budget.max(),
            move_in: data.move_in,
            gender: data.gender,
            gender_pref: data.gender_pref,
            lifestyle: data.lifestyle.clone(),
            note: data.note.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CityCount {
    pub city: String,
    pub count: u32,
    /// The map centre for a known city.
    pub lat: Option<f64>,
    pub lng: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Me {
    /// The approximate point the owner sees on their private map.
    pub location: Option<MapLocation>,
    pub card: Option<Card>,
    pub paused: bool,
    pub has_pin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapLocation {
    pub point: GeoPoint,
    pub precision: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct AreaActivity {
    /// A catalogue neighbourhood, or the city for activity outside those neighbourhoods.
    pub area: Option<&'static str>,
    pub intent: Intent,
    /// Lower bound of a bucket. Exact counts and groups under three stay private.
    pub at_least: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchCard {
    pub intent: Intent,
    pub city: String,
    pub area: Option<&'static str>,
    pub budget_min: u32,
    pub budget_max: u32,
    pub move_in: Option<NaiveDate>,
}

impl From<&Profile> for MatchCard {
    fn from(profile: &Profile) -> Self {
        let data = &profile.data;
        Self {
            intent: data.intent,
            city: data.city.to_string(),
            area: data.area.as_deref().and_then(|name| area::find(&data.city, name)).map(|area| area.name),
            budget_min: data.budget.min(),
            budget_max: data.budget.max(),
            move_in: data.move_in,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WebMatch {
    pub id: MatchId,
    pub status: MatchStatus,
    /// The viewer's own answer, if given.
    pub decided: bool,
    pub card: MatchCard,
    pub fits: Vec<String>,
    pub watch: Vec<String>,
    /// After a mutual yes only.
    pub name: Option<String>,
    pub whatsapp: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionResult {
    Liked,
    Mutual,
    Skipped,
    AlreadyDecided,
}

impl Web {
    pub fn new(store: Arc<dyn Store>, settings: Settings) -> Self {
        Self { store, settings }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub async fn pins(&self, bbox: Option<BBox>, filter: &PinFilter) -> Result<Vec<Pin>, Error> {
        let mut tx = self.store.begin().await?;
        let profiles = tx.pins(bbox, MAX_PINS).await?;
        tx.commit().await?;
        Ok(profiles.iter().filter_map(pin).filter(|pin| keep(pin, filter)).collect())
    }

    /// The map form of one profile, for a live update. `None` when it should
    /// not be on the map.
    pub async fn pin(&self, code: &PublicCode) -> Result<Option<Pin>, Error> {
        let mut tx = self.store.begin().await?;
        let profile = tx.profile_by_code(code).await?;
        tx.commit().await?;
        Ok(profile.filter(|p| p.status == ProfileStatus::Active).as_ref().and_then(pin))
    }

    pub async fn card(&self, code: &PublicCode) -> Result<Card, Error> {
        let mut tx = self.store.begin().await?;
        let profile = tx.profile_by_code(code).await?;
        tx.commit().await?;
        profile.filter(|p| p.status == ProfileStatus::Active).map(|p| Card::from(&p)).ok_or(Error::NotFound)
    }

    pub async fn cities(&self) -> Result<Vec<CityCount>, Error> {
        let mut tx = self.store.begin().await?;
        let counts = tx.city_counts().await?;
        tx.commit().await?;
        Ok(counts
            .into_iter()
            .map(|(city, count)| {
                let centre = city.centre();
                CityCount { city: city.to_string(), count, lat: centre.map(|p| p.lat()), lng: centre.map(|p| p.lng()) }
            })
            .collect())
    }

    /// Trades a private link for a session. The link works once.
    pub async fn sign_in(&self, link_token: &str, previous: Option<&str>, now: DateTime<Utc>) -> Result<String, Error> {
        let mut tx = self.store.begin().await?;
        let user = tx.take_magic_link(&ops::hash_token(link_token), now).await?.ok_or(Error::Unauthorized)?;
        let session = ops::new_token();
        tx.create_session(user, &ops::hash_token(&session), now + policy::session_ttl()).await?;
        if let Some(previous) = previous {
            tx.delete_session(&ops::hash_token(previous)).await?;
        }
        tx.commit().await?;
        Ok(session)
    }

    pub async fn sign_out(&self, session: &str) -> Result<(), Error> {
        let mut tx = self.store.begin().await?;
        tx.delete_session(&ops::hash_token(session)).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn area_activity(
        &self,
        session: &str,
        city: &City,
        now: DateTime<Utc>,
    ) -> Result<Vec<AreaActivity>, Error> {
        let mut tx = self.store.begin().await?;
        let user = self.user(tx.as_mut(), session, now).await?;
        if tx.profile_of(user.id).await?.is_none() {
            return Err(Error::Unauthorized);
        }
        let counts = tx.area_counts(city).await?;
        tx.commit().await?;
        let mut grouped = std::collections::BTreeMap::new();
        for (name, intent, count) in counts {
            let name = name.as_deref().and_then(|name| area::find(city, name)).map(|area| area.name);
            let total = grouped.entry((name, intent)).or_insert(0_u32);
            *total = total.saturating_add(count);
        }
        Ok(grouped
            .into_iter()
            .filter_map(|((area, intent), count)| {
                let at_least = [25, 10, 5, 3].into_iter().find(|bound| count >= *bound)?;
                Some(AreaActivity { area, intent, at_least })
            })
            .collect())
    }

    async fn user(&self, tx: &mut dyn crate::Tx, session: &str, now: DateTime<Utc>) -> Result<User, Error> {
        let id = tx.session_user(&ops::hash_token(session), now).await?.ok_or(Error::Unauthorized)?;
        tx.user(id).await?.ok_or(Error::Unauthorized)
    }

    pub async fn me(&self, session: &str, now: DateTime<Utc>) -> Result<Me, Error> {
        let mut tx = self.store.begin().await?;
        let user = self.user(tx.as_mut(), session, now).await?;
        let profile = tx.profile_of(user.id).await?;
        tx.commit().await?;
        Ok(Me {
            location: profile.as_ref().and_then(map_location),
            paused: profile.as_ref().is_some_and(|p| p.status == ProfileStatus::Paused),
            has_pin: profile.as_ref().is_some_and(|p| p.data.pin.is_some()),
            card: profile.as_ref().map(Card::from),
        })
    }

    pub async fn matches(&self, session: &str, now: DateTime<Utc>) -> Result<Vec<WebMatch>, Error> {
        let mut tx = self.store.begin().await?;
        let user = self.user(tx.as_mut(), session, now).await?;
        let Some(own) = tx.profile_of(user.id).await? else { return Ok(Vec::new()) };
        if own.status != ProfileStatus::Active {
            return Ok(Vec::new());
        }
        let mut list = Vec::new();
        for (pair, other) in tx.matches_of(own.id).await? {
            if list.len() == MAX_WEB_MATCHES {
                break;
            }
            if pair.status() == MatchStatus::Suggested && !domain::matching::eligible(&own.data, &other.data) {
                continue;
            }
            let status = pair.status();
            let (name, whatsapp) = if status == MatchStatus::Mutual {
                let other_user = tx.user(other.user_id).await?;
                let name = other_user.as_ref().and_then(|u| u.first_name.clone());
                let link = other_user.and_then(|u| u.phone).map(|phone| format!("https://wa.me/{phone}"));
                (name, link)
            } else {
                (None, None)
            };
            let explanation = domain::matching::explain(&own.data, &other.data);
            list.push(WebMatch {
                id: pair.id,
                status,
                decided: pair.decision_of(own.id).is_some(),
                card: MatchCard::from(&other),
                fits: explanation.fits,
                watch: explanation.watch,
                name,
                whatsapp,
            });
        }
        tx.commit().await?;
        Ok(list)
    }

    pub async fn decide(
        &self,
        session: &str,
        id: MatchId,
        accept: bool,
        now: DateTime<Utc>,
    ) -> Result<DecisionResult, Error> {
        let mut tx = self.store.begin().await?;
        let user = self.user(tx.as_mut(), session, now).await?;
        let own = tx.profile_of(user.id).await?.ok_or(Error::NotFound)?;
        if own.status != ProfileStatus::Active {
            return Err(Error::Invalid("your profile is paused".into()));
        }
        let pair = tx.match_by_id(id).await?.filter(|p| p.involves(own.id)).ok_or(Error::NotFound)?;
        let result = match ops::decide(tx.as_mut(), &user, &own, pair, accept, now).await? {
            Outcome::Liked => DecisionResult::Liked,
            Outcome::Mutual { .. } => DecisionResult::Mutual,
            Outcome::Skipped => DecisionResult::Skipped,
            Outcome::AlreadyDecided => DecisionResult::AlreadyDecided,
            Outcome::Unavailable => return Err(Error::NotFound),
        };
        tx.commit().await?;
        Ok(result)
    }

    /// Changes participation without changing the profile or its WhatsApp identity.
    pub async fn set_paused(&self, session: &str, paused: bool, now: DateTime<Utc>) -> Result<(), Error> {
        let mut tx = self.store.begin().await?;
        let user = self.user(tx.as_mut(), session, now).await?;
        let mut chat = tx.conversation(user.id, true).await?;
        let profile = tx.profile_of(user.id).await?.ok_or(Error::NotFound)?;
        match (paused, profile.status) {
            (true, ProfileStatus::Active) => {
                tx.set_profile_status(user.id, ProfileStatus::Paused, now).await?;
                chat.stage = Stage::Paused;
            }
            (false, ProfileStatus::Paused) => {
                ops::publish(tx.as_mut(), &user, &profile.data, now).await?;
                chat.stage = Stage::Live;
            }
            _ => return Ok(()),
        }
        tx.save_conversation(user.id, &chat, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Edits the person's profile. Only fields present in `patch` change.
    pub async fn update(&self, session: &str, patch: ProfilePatch, now: DateTime<Utc>) -> Result<Card, Error> {
        let mut tx = self.store.begin().await?;
        let user = self.user(tx.as_mut(), session, now).await?;
        let mut chat = tx.conversation(user.id, true).await?;
        let mut draft = match tx.profile_of(user.id).await? {
            Some(profile) => profile.data.to_patch(),
            None => chat.draft.clone(),
        };
        draft.merge(patch);
        let data = draft.complete().map_err(|missing| {
            let names: Vec<&str> = missing.iter().map(|f| f.as_str()).collect();
            Error::Invalid(format!("missing: {}", names.join(", ")))
        })?;
        let published = ops::publish(tx.as_mut(), &user, &data, now).await?;
        chat.draft = draft;
        if chat.stage != Stage::Paused {
            chat.stage = Stage::Live;
        }
        tx.save_conversation(user.id, &chat, now).await?;
        tx.commit().await?;
        Ok(Card::from(&published.profile))
    }
}

fn map_location(profile: &Profile) -> Option<MapLocation> {
    let data = &profile.data;
    let (point, precision) = if let Some(pin) = data.pin {
        (geo::area_centre(pin), "approximate")
    } else if let Some(area) = data.area.as_deref().and_then(|name| area::find(&data.city, name)) {
        (area.centre, "neighbourhood")
    } else {
        (data.city.centre()?, "city")
    };
    Some(MapLocation { point, precision })
}

fn pin(profile: &Profile) -> Option<Pin> {
    let shown = geo::blur(profile.data.pin?, profile.id);
    Some(Pin {
        code: profile.code.clone(),
        intent: profile.data.intent,
        lat: shown.lat(),
        lng: shown.lng(),
        budget_max: profile.data.budget.max(),
        gender_pref: profile.data.gender_pref,
        gender: profile.data.gender,
    })
}

fn keep(pin: &Pin, filter: &PinFilter) -> bool {
    filter.intent.is_none_or(|intent| pin.intent == intent)
        && filter.budget_max.is_none_or(|max| pin.budget_max <= max)
        && filter.gender.is_none_or(|gender| pin.gender == gender)
}

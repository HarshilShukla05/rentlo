//! The WhatsApp conversation: one inbound message in, replies out.
//!
//! A message is handled in one transaction. The person's state, every
//! change it causes and every reply are committed together, then a separate
//! dispatcher sends the replies. A crash at any point either loses nothing
//! or repeats nothing: the message id is recorded in the same transaction,
//! so a redelivered message is dropped.

use crate::Error;
use crate::copy;
use crate::model::{Outgoing, Profile, Settings, Topic, User};
use crate::ops::{self, Outcome};
use crate::policy;
use crate::ports::{ProfileExtractor, Store, StoreResult, Tx};
use chrono::{DateTime, Duration, Utc};
use domain::command::{self, Command, Target};
use domain::conversation::{Conversation, Stage, Step};
use domain::{GeoPoint, ProfileData, ProfilePatch, ProfileStatus, PublicCode};
use std::sync::Arc;

/// One message from WhatsApp, already stripped of transport details.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// WhatsApp's id for the message. Unique per message.
    pub message_id: String,
    /// The sender's chat address. Replies go here.
    pub jid: String,
    /// The sender's number in digits with country code, when known.
    pub phone: Option<String>,
    /// The name the sender set in WhatsApp.
    pub name: Option<String>,
    pub body: Body,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum Body {
    Text(String),
    Location {
        lat: f64,
        lng: f64,
    },
    LiveLocation {
        lat: f64,
        lng: f64,
    },
    Area(String),
    UnresolvedMapLink,
    /// A photo, a voice note, a sticker: nothing the bot reads.
    Unsupported,
}

pub struct Bot {
    store: Arc<dyn Store>,
    extractor: Arc<dyn ProfileExtractor>,
    settings: Settings,
}

impl Bot {
    pub fn new(store: Arc<dyn Store>, extractor: Arc<dyn ProfileExtractor>, settings: Settings) -> Self {
        Self { store, extractor, settings }
    }

    pub async fn handle(&self, message: Inbound) -> Result<(), Error> {
        let command = match &message.body {
            Body::Text(text) => command::parse(text),
            _ => None,
        };
        // Free text is read before the transaction opens: a model call can
        // take seconds, and must not hold the person's row locked meanwhile.
        let patch = match (&message.body, &command) {
            (Body::Text(text), None) => match self.read(&message, text).await? {
                Some(patch) => Some(patch),
                None => return Ok(()),
            },
            _ => None,
        };

        let mut tx = self.store.begin().await?;
        let user = tx.upsert_user(&message.jid, message.phone.as_deref(), message.name.as_deref(), message.at).await?;
        if !tx.record_inbound(&message.message_id, user.id, message.at).await? {
            return Ok(());
        }
        if tx.inbound_count_since(user.id, message.at - Duration::minutes(1)).await? > policy::INBOUND_PER_MINUTE {
            // Someone flooding the bot gets silence, not a reply per message.
            tx.commit().await?;
            return Ok(());
        }
        let mut chat = tx.conversation(user.id, true).await?;
        let mut turn = Turn {
            tx: tx.as_mut(),
            user: user.clone(),
            chat: &mut chat,
            settings: &self.settings,
            now: message.at,
            replies: Vec::new(),
        };
        let erased = turn.run(message.body, command, patch.unwrap_or_default()).await?;
        let replies = std::mem::take(&mut turn.replies);
        drop(turn);

        if !erased {
            tx.save_conversation(user.id, &chat, message.at).await?;
        }
        for (index, text) in replies.into_iter().enumerate() {
            let reply = Outgoing {
                topic: Topic::Reply,
                dedupe_key: format!("{}:{index}", message.message_id),
                // After an erase the goodbye must not hang off the user row.
                user: (!erased).then_some(user.id),
                jid: user.jid.clone(),
                text,
                available_at: message.at,
            };
            tx.enqueue(&reply, message.at).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Reads free text into a patch. `None` when the message was already
    /// handled, so a redelivery costs no model call.
    async fn read(&self, message: &Inbound, text: &str) -> StoreResult<Option<ProfilePatch>> {
        let mut tx = self.store.begin().await?;
        if tx.inbound_seen(&message.message_id).await? {
            return Ok(None);
        }
        let user = tx.upsert_user(&message.jid, message.phone.as_deref(), message.name.as_deref(), message.at).await?;
        let asked = tx.conversation(user.id, false).await?.asked;
        tx.commit().await?;
        Ok(Some(self.extractor.extract(text, asked, message.at.date_naive()).await))
    }
}

/// One message being handled.
struct Turn<'a> {
    tx: &'a mut dyn Tx,
    user: User,
    chat: &'a mut Conversation,
    settings: &'a Settings,
    now: DateTime<Utc>,
    replies: Vec<String>,
}

impl Turn<'_> {
    fn say(&mut self, text: String) {
        self.replies.push(text);
    }

    /// Handles the message. True when the account was erased.
    async fn run(&mut self, body: Body, command: Option<Command>, patch: ProfilePatch) -> StoreResult<bool> {
        let armed = std::mem::take(&mut self.chat.delete_armed);
        match body {
            Body::Unsupported => self.say(copy::unsupported()),
            Body::Location { lat, lng } => self.location(lat, lng, false).await?,
            Body::LiveLocation { lat, lng } => self.location(lat, lng, true).await?,
            Body::Area(area) => self.area(area).await?,
            Body::UnresolvedMapLink => self.say(copy::map_link_needs_pin()),
            Body::Text(text) => {
                // While onboarding, "skip" and "yes" answer the bot's question.
                let onboarding = matches!(self.chat.stage, Stage::New | Stage::Collecting);
                let command = command.filter(|c| !(onboarding && matches!(c, Command::Decision { .. })));
                match command {
                    None => self.free_text(&text, patch).await?,
                    Some(Command::Greeting) => self.greeting().await?,
                    Some(Command::Help) => self.say(copy::help()),
                    Some(Command::Pause) => self.pause(true).await?,
                    Some(Command::Resume) => self.pause(false).await?,
                    Some(Command::DeleteConfirm) if armed => {
                        self.tx.delete_user(self.user.id).await?;
                        self.say(copy::deleted());
                        return Ok(true);
                    }
                    Some(Command::Delete | Command::DeleteConfirm) => {
                        self.chat.delete_armed = true;
                        self.say(copy::delete_confirm());
                    }
                    Some(Command::Matches) => self.matches(false).await?,
                    Some(Command::Profile) => self.profile().await?,
                    Some(Command::Link) => self.link().await?,
                    Some(Command::Decision { target, accept }) => self.decision(target, accept).await?,
                    Some(Command::Interest(code)) => self.interest(code).await?,
                    Some(Command::Report { code, reason }) => self.report(code, &reason).await?,
                }
            }
        }
        Ok(false)
    }

    async fn greeting(&mut self) -> StoreResult<()> {
        match self.chat.stage {
            Stage::New | Stage::Collecting => {
                self.chat.stage = Stage::Collecting;
                self.say(copy::welcome(self.settings));
            }
            Stage::Live => self.profile().await?,
            Stage::Paused => self.say(copy::is_paused()),
        }
        Ok(())
    }

    async fn free_text(&mut self, text: &str, patch: ProfilePatch) -> StoreResult<()> {
        // A bare phone number, when WhatsApp did not tell us the sender's.
        if self.user.phone.is_none()
            && !text.chars().any(char::is_alphabetic)
            && let Some(phone) = domain::extract::phone(text)
        {
            self.user = self.tx.upsert_user(&self.user.jid, Some(&phone), None, self.now).await?;
            self.say(copy::phone_saved());
            return Ok(());
        }
        match self.chat.stage {
            Stage::New => {
                self.chat.stage = Stage::Collecting;
                self.say(copy::hello(self.settings));
                self.absorb(patch).await
            }
            Stage::Collecting => self.absorb(patch).await,
            Stage::Paused => {
                self.say(copy::is_paused());
                Ok(())
            }
            Stage::Live if patch.is_empty() => {
                self.say(copy::not_understood());
                Ok(())
            }
            Stage::Live => {
                self.rebase_draft().await?;
                self.absorb(patch).await
            }
        }
    }

    /// For a live profile the draft is the stored profile: it may have been
    /// edited on the web since the chat last saw it.
    async fn rebase_draft(&mut self) -> StoreResult<()> {
        if let Some(profile) = self.tx.profile_of(self.user.id).await? {
            self.chat.draft = profile.data.to_patch();
        }
        Ok(())
    }

    async fn absorb(&mut self, patch: ProfilePatch) -> StoreResult<()> {
        match self.chat.absorb(patch) {
            Step::Ask(fields) => self.say(copy::ask(&fields, self.chat)),
            Step::AskHabits => self.say(copy::ask_habits()),
            Step::Ready(data) => self.go_live(&data).await?,
        }
        Ok(())
    }

    async fn go_live(&mut self, data: &ProfileData) -> StoreResult<()> {
        let published = ops::publish(self.tx, &self.user, data, self.now).await?;
        self.chat.stage = Stage::Live;
        let profile = published.profile;
        if published.fresh {
            self.say(copy::live(&profile));
        } else {
            self.say(copy::updated(&profile));
        }
        if let Some(code) = self.chat.interest.take() {
            self.interest(code).await?;
        }
        // After an edit, silence beats "no matches yet" on every change.
        self.matches(!published.fresh).await?;
        if published.fresh && profile.data.pin.is_none() {
            self.say(copy::pin_hint());
        }
        Ok(())
    }

    /// The person's profile if it is live; otherwise says why not.
    async fn live_profile(&mut self) -> StoreResult<Option<Profile>> {
        match self.tx.profile_of(self.user.id).await? {
            Some(profile) if profile.status == ProfileStatus::Active => Ok(Some(profile)),
            Some(_) => {
                self.say(copy::is_paused());
                Ok(None)
            }
            None => {
                self.say(copy::finish_first());
                self.say(self.next_question());
                Ok(None)
            }
        }
    }

    /// The question onboarding is waiting on, without counting it as asked again.
    fn next_question(&self) -> String {
        match self.chat.draft.missing().first() {
            Some(field) => copy::ask(&[*field], &Conversation::default()),
            None => copy::ask_habits(),
        }
    }

    async fn matches(&mut self, quiet_if_none: bool) -> StoreResult<()> {
        let Some(profile) = self.live_profile().await? else { return Ok(()) };
        let open = ops::open_matches(self.tx, &profile).await?;
        if open.is_empty() {
            if !quiet_if_none {
                let text = if self.chat.shown.is_empty() {
                    copy::no_matches(profile.data.city.as_str())
                } else {
                    copy::no_more_matches()
                };
                self.say(text);
            }
            self.chat.shown.clear();
            return Ok(());
        }
        let top: Vec<_> = open.into_iter().take(policy::CARDS_PER_REPLY).collect();
        self.chat.shown = top.iter().map(|(pair, _)| pair.id).collect();
        let cards: Vec<(u8, &Profile)> = top.iter().map(|(pair, other)| (pair.score, other)).collect();
        self.say(copy::cards(&profile.data, &cards));
        Ok(())
    }

    async fn decision(&mut self, target: Target, accept: bool) -> StoreResult<()> {
        let Some(profile) = self.live_profile().await? else { return Ok(()) };
        let shown = match target {
            Target::Index(index) => index.checked_sub(1).and_then(|i| self.chat.shown.get(i)).copied(),
            Target::Sole if self.chat.shown.len() == 1 => self.chat.shown.first().copied(),
            Target::Sole | Target::Code(_) => None,
        };
        let pair = match (shown, &target) {
            (Some(id), _) => self.tx.match_by_id(id).await?,
            (None, Target::Code(code)) => match self.tx.profile_by_code(code).await? {
                Some(other) => self.tx.match_between(profile.id, other.id).await?,
                None => None,
            },
            (None, _) => None,
        };
        match pair.filter(|pair| pair.involves(profile.id)) {
            Some(pair) => self.decide(&profile, pair, accept).await,
            None => {
                self.say(copy::unknown_card());
                Ok(())
            }
        }
    }

    async fn decide(&mut self, profile: &Profile, pair: domain::Match, accept: bool) -> StoreResult<()> {
        let text = match ops::decide(self.tx, &self.user, profile, pair, accept, self.now).await? {
            Outcome::Liked => copy::liked(),
            Outcome::Mutual { intro } => intro,
            Outcome::Skipped => copy::skipped(),
            Outcome::AlreadyDecided => copy::already_decided(),
            Outcome::Unavailable => copy::unknown_card(),
        };
        self.say(text);
        Ok(())
    }

    /// "I like R7K2Q9", usually from the map's button.
    async fn interest(&mut self, code: PublicCode) -> StoreResult<()> {
        let own = self.tx.profile_of(self.user.id).await?;
        let Some(own) = own else {
            // Remembered, and acted on the moment the profile goes live.
            if self.chat.stage == Stage::New {
                self.chat.stage = Stage::Collecting;
                self.say(copy::welcome_for(&code, self.settings));
            } else {
                self.say(copy::finish_first());
                self.say(self.next_question());
            }
            self.chat.interest = Some(code);
            return Ok(());
        };
        if own.status != ProfileStatus::Active {
            self.say(copy::is_paused());
            return Ok(());
        }
        let target = self.tx.profile_by_code(&code).await?.filter(|p| p.status == ProfileStatus::Active);
        let Some(target) = target else {
            self.say(copy::interest_unavailable(&code));
            return Ok(());
        };
        if target.user_id == self.user.id {
            self.say(copy::interest_own());
            return Ok(());
        }
        if !domain::matching::eligible(&own.data, &target.data) {
            self.say(copy::interest_not_a_fit(&code));
            return Ok(());
        }
        let pair = match self.tx.match_between(own.id, target.id).await? {
            Some(pair) => Some(pair),
            None => {
                let score = domain::matching::score(&own.data, &target.data);
                self.tx.insert_match(own.id, target.id, score, self.now).await?
            }
        };
        match pair {
            Some(pair) => self.decide(&own, pair, true).await,
            None => {
                self.say(copy::unknown_card());
                Ok(())
            }
        }
    }

    async fn report(&mut self, code: PublicCode, reason: &str) -> StoreResult<()> {
        let Some(target) = self.tx.profile_by_code(&code).await? else {
            self.say(copy::interest_unavailable(&code));
            return Ok(());
        };
        self.tx.insert_report(self.user.id, target.id, reason, self.now).await?;
        // The reporter never sees that profile again.
        if let Some(own) = self.tx.profile_of(self.user.id).await?
            && let Some(mut pair) = self.tx.match_between(own.id, target.id).await?
            && pair.decide(own.id, domain::Decision::Skip) != domain::Decided::Unchanged
        {
            self.tx.save_decisions(&pair, self.now).await?;
        }
        self.say(copy::reported());
        Ok(())
    }

    async fn pause(&mut self, pause: bool) -> StoreResult<()> {
        if self.tx.profile_of(self.user.id).await?.is_none() {
            self.say(copy::finish_first());
            self.say(self.next_question());
            return Ok(());
        }
        if pause {
            self.tx.set_profile_status(self.user.id, ProfileStatus::Paused, self.now).await?;
            self.chat.stage = Stage::Paused;
            self.say(copy::paused());
        } else {
            self.tx.set_profile_status(self.user.id, ProfileStatus::Active, self.now).await?;
            self.chat.stage = Stage::Live;
            self.say(copy::resumed());
            self.matches(true).await?;
        }
        Ok(())
    }

    async fn profile(&mut self) -> StoreResult<()> {
        match self.tx.profile_of(self.user.id).await? {
            Some(profile) => {
                let paused = profile.status == ProfileStatus::Paused;
                self.say(copy::own_profile(&profile, paused));
            }
            None => {
                self.say(copy::finish_first());
                self.say(self.next_question());
            }
        }
        Ok(())
    }

    async fn link(&mut self) -> StoreResult<()> {
        let token = ops::new_token();
        let expires = self.now + policy::magic_link_ttl();
        self.tx.create_magic_link(self.user.id, &ops::hash_token(&token), expires).await?;
        self.say(copy::link(&format!("{}/m/{token}", self.settings.public_url)));
        Ok(())
    }

    async fn location(&mut self, lat: f64, lng: f64, live: bool) -> StoreResult<()> {
        if self.chat.stage == Stage::Paused {
            self.say(copy::is_paused());
            return Ok(());
        }
        if self.chat.stage == Stage::Live {
            self.rebase_draft().await?;
        }
        if live && self.chat.live_location_seen {
            return Ok(());
        }
        if live {
            self.chat.live_location_seen = true;
            if self.chat.draft.pin.is_some() {
                self.say(copy::live_location_existing());
                return Ok(());
            }
        }
        let Some(pin) = GeoPoint::new(lat, lng) else {
            self.say(copy::pin_outside_india());
            return Ok(());
        };
        if live {
            self.say(copy::live_location_saved());
        }
        let patch = ProfilePatch { pin: Some(pin), ..ProfilePatch::default() };
        match self.chat.stage {
            Stage::New => {
                self.chat.stage = Stage::Collecting;
                self.chat.draft.merge(patch);
                self.say(copy::welcome(self.settings));
            }
            Stage::Collecting => {
                // Merged without a step: a pin is not an answer to the
                // question, and must not count as one more failed ask.
                self.chat.draft.merge(patch);
                self.say(copy::pin_saved_before_live());
                self.say(self.next_question());
            }
            Stage::Live => self.absorb(patch).await?,
            Stage::Paused => {}
        }
        Ok(())
    }

    async fn area(&mut self, area: String) -> StoreResult<()> {
        if self.chat.stage == Stage::Paused {
            self.say(copy::is_paused());
            return Ok(());
        }
        let mut patch = ProfilePatch::default();
        patch.merge(ProfilePatch { area: Some(area), ..ProfilePatch::default() });
        let Some(area) = patch.area.as_deref() else {
            self.say(copy::map_link_needs_pin());
            return Ok(());
        };
        if self.chat.stage == Stage::Live {
            self.rebase_draft().await?;
        }
        if self.chat.draft.area.as_deref().is_none_or(|old| !old.eq_ignore_ascii_case(area)) {
            self.chat.draft.pin = None;
        }
        self.say(copy::map_area_saved(area, self.chat.draft.pin.is_some()));
        match self.chat.stage {
            Stage::New => {
                self.chat.stage = Stage::Collecting;
                self.chat.draft.merge(patch);
                self.say(copy::welcome(self.settings));
            }
            Stage::Collecting => {
                self.chat.draft.merge(patch);
                self.say(self.next_question());
            }
            Stage::Live => self.absorb(patch).await?,
            Stage::Paused => {}
        }
        Ok(())
    }
}

//! End-to-end: the bot, the dispatcher and the web use cases against a real
//! PostgreSQL, with WhatsApp replaced by a recorder.
//!
//! Set RENTLO_TEST_DATABASE_URL to a server where the user may create
//! databases. Each test makes its own database. Without the variable the
//! tests are skipped, loudly.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use app::{Body, Bot, Dispatcher, Inbound, Messenger, RulesExtractor, SendError, Settings, Store, Web};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::sync::{Arc, Mutex};
use store::PgStore;

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<(String, String)>>,
    /// Sends to fail before succeeding.
    failures: Mutex<u32>,
}

#[async_trait]
impl Messenger for Recorder {
    async fn send_text(&self, jid: &str, text: &str) -> Result<(), SendError> {
        {
            let mut failures = self.failures.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err(SendError("socket closed".into()));
            }
        }
        self.sent.lock().unwrap().push((jid.to_owned(), text.to_owned()));
        Ok(())
    }
}

struct World {
    bot: Bot,
    dispatcher: Dispatcher,
    web: Web,
    recorder: Arc<Recorder>,
    now: Mutex<DateTime<Utc>>,
    next_id: Mutex<u32>,
}

/// 11:30 in India, so nothing waits for quiet hours.
const START: &str = "2026-11-16T06:00:00Z";

impl World {
    async fn new() -> Option<Self> {
        let Ok(admin_url) = std::env::var("RENTLO_TEST_DATABASE_URL") else {
            eprintln!("SKIPPED: set RENTLO_TEST_DATABASE_URL to run the PostgreSQL tests");
            return None;
        };
        let name = format!("rentlo_test_{}", uuid::Uuid::new_v4().simple());
        let admin = PgStore::connect(&admin_url, 1).await.expect("connect to the test server");
        admin.create_database(&name).await.expect("create the test database");
        let url = replace_database(&admin_url, &name);
        let store = PgStore::connect(&url, 5).await.unwrap();
        store.migrate().await.unwrap();

        let settings = Settings { public_url: "https://rentlo.test".into(), bot_phone: Some("910000000000".into()) };
        let shared: Arc<dyn Store> = Arc::new(store.clone());
        let recorder = Arc::new(Recorder::default());
        Some(Self {
            bot: Bot::new(shared.clone(), Arc::new(RulesExtractor), settings.clone()),
            dispatcher: Dispatcher::new(shared.clone(), recorder.clone()),
            web: Web::new(shared, settings),
            recorder,
            now: Mutex::new(START.parse().unwrap()),
            next_id: Mutex::new(0),
        })
    }

    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }

    fn advance(&self, by: Duration) {
        *self.now.lock().unwrap() += by;
    }

    async fn deliver(&self, who: &Person, body: Body) -> String {
        self.advance(Duration::seconds(5));
        let id = {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            format!("MSG{next}")
        };
        let message = Inbound {
            message_id: id.clone(),
            jid: who.jid(),
            phone: Some(who.phone.into()),
            name: Some(who.name.into()),
            body,
            at: self.now(),
        };
        self.bot.handle(message).await.unwrap();
        id
    }

    /// Sends `text` as `who` and returns every message the bot then sent to them.
    async fn say(&self, who: &Person, text: &str) -> String {
        self.deliver(who, Body::Text(text.into())).await;
        self.inbox(who).await
    }

    /// Delivers everything due and returns what `who` received, joined.
    async fn inbox(&self, who: &Person) -> String {
        let now = self.now();
        self.dispatcher.drain(|| now).await.unwrap();
        let mut sent = self.recorder.sent.lock().unwrap();
        let (mine, rest): (Vec<_>, Vec<_>) = sent.drain(..).partition(|(jid, _)| *jid == who.jid());
        *sent = rest;
        mine.into_iter().map(|(_, text)| text).collect::<Vec<_>>().join("\n---\n")
    }
}

fn replace_database(url: &str, name: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("a URL with a database name");
    format!("{base}/{name}")
}

struct Person {
    phone: &'static str,
    name: &'static str,
}

impl Person {
    fn jid(&self) -> String {
        format!("{}@s.whatsapp.net", self.phone)
    }
}

const ASHA: Person = Person { phone: "919800000001", name: "Asha" };
const RAVI: Person = Person { phone: "919800000002", name: "Ravi" };
const MEERA: Person = Person { phone: "919800000003", name: "Meera" };

/// The profile code the bot announced in `text`.
fn code_in(text: &str) -> String {
    let start = text.find("Your code is *").expect("a code announcement") + "Your code is *".len();
    text[start..start + 6].to_owned()
}

async fn onboard_asha(world: &World) -> String {
    let reply = world
        .say(
            &ASHA,
            "I have a room in my 2bhk in Koramangala Bangalore, rent 16k. I'm a female, any gender is fine. \
             Sleep early, very tidy, veg, non smoker",
        )
        .await;
    assert!(reply.contains("You are live"), "{reply}");
    code_in(&reply)
}

#[tokio::test]
async fn two_people_meet_from_first_message_to_contact() {
    let Some(world) = World::new().await else { return };

    let hello = world.say(&ASHA, "Hi").await;
    assert!(hello.contains("Tell me in one message"), "{hello}");
    assert!(hello.contains("https://rentlo.test/privacy"), "{hello}");

    let asha_code = onboard_asha(&world).await;
    let asha_live = world.inbox(&ASHA).await;
    assert_eq!(asha_live, "", "nothing more is pending");

    // Ravi says most things at once; the bot asks only for the rest.
    let first = world
        .say(&RAVI, "need a room in koramangala blr, 18k max, i sleep by 11, veg, no smoking, clean freak, 26M")
        .await;
    assert!(first.contains("Hi! I'm Rentlo"), "{first}");
    assert!(first.contains("preference for your flatmate's gender"), "{first}");
    let live = world.say(&RAVI, "1").await;
    assert!(live.contains("You are live"), "{live}");
    assert!(live.contains(&format!("· {asha_code}")), "Ravi sees Asha's card: {live}");
    assert!(live.contains("both sleep early"), "{live}");
    assert!(!live.contains("9198"), "no phone number before a mutual yes: {live}");
    assert!(live.contains("location pin"), "{live}");

    // Asha is told about Ravi, unprompted.
    let alert = world.inbox(&ASHA).await;
    assert!(alert.contains("New match for you"), "{alert}");
    let ravi_code = code_in(&live);
    assert!(alert.contains(&format!("Reply *yes {ravi_code}*")), "{alert}");

    // Ravi says yes; Asha is asked.
    let liked = world.say(&RAVI, "1 yes").await;
    assert!(liked.contains("I have told them"), "{liked}");
    let asked = world.inbox(&ASHA).await;
    assert!(asked.contains("Someone wants to connect with you"), "{asked}");

    // Asha says yes: both get the other's contact.
    let intro = world.say(&ASHA, &format!("yes {ravi_code}")).await;
    assert!(intro.contains("It's a match!"), "{intro}");
    assert!(intro.contains("*Ravi*"), "{intro}");
    assert!(intro.contains("https://wa.me/919800000002"), "{intro}");
    let ravi_intro = world.inbox(&RAVI).await;
    assert!(ravi_intro.contains("*Asha*"), "{ravi_intro}");
    assert!(ravi_intro.contains("https://wa.me/919800000001"), "{ravi_intro}");

    // A repeated answer changes nothing.
    let again = world.say(&ASHA, &format!("yes {ravi_code}")).await;
    assert!(again.contains("already answered"), "{again}");
}

#[tokio::test]
async fn a_redelivered_message_is_handled_once() {
    let Some(world) = World::new().await else { return };
    let message = Inbound {
        message_id: "SAME".into(),
        jid: ASHA.jid(),
        phone: None,
        name: None,
        body: Body::Text("hi".into()),
        at: world.now(),
    };
    world.bot.handle(message.clone()).await.unwrap();
    world.bot.handle(message).await.unwrap();
    let replies = world.inbox(&ASHA).await;
    assert_eq!(replies.matches("Tell me in one message").count(), 1, "{replies}");
}

#[tokio::test]
async fn a_skip_is_final_and_silent() {
    let Some(world) = World::new().await else { return };
    let asha_code = onboard_asha(&world).await;
    world.inbox(&ASHA).await;
    world.say(&RAVI, "need a room in koramangala blr 18k, male, any gender, veg, tidy, non smoker").await;
    world.inbox(&ASHA).await;
    let skipped = world.say(&RAVI, &format!("skip {asha_code}")).await;
    assert!(skipped.contains("They will not be told"), "{skipped}");
    assert_eq!(world.inbox(&ASHA).await, "");
    let matches = world.say(&RAVI, "matches").await;
    assert!(!matches.contains(&asha_code), "{matches}");
}

#[tokio::test]
async fn a_map_visitor_is_connected_once_their_profile_is_live() {
    let Some(world) = World::new().await else { return };
    let asha_code = onboard_asha(&world).await;
    world.inbox(&ASHA).await;

    let first = world.say(&RAVI, &format!("Hi Rentlo, I like {asha_code}")).await;
    assert!(first.contains(&format!("connect you with *{asha_code}*")), "{first}");
    let live =
        world.say(&RAVI, "need a room in koramangala bangalore 18k, male, any gender, veg, tidy, non smoker").await;
    assert!(live.contains("I have told them"), "the remembered interest is acted on: {live}");
    let asha = world.inbox(&ASHA).await;
    assert!(asha.contains("Someone wants to connect with you"), "{asha}");
}

#[tokio::test]
async fn people_who_do_not_fit_are_never_paired() {
    let Some(world) = World::new().await else { return };
    let asha_code = onboard_asha(&world).await;
    world.inbox(&ASHA).await;
    // Another city.
    let pune = world.say(&RAVI, "need a room in pune 18k, male, any gender, veg, tidy, non smoker").await;
    assert!(pune.contains("No match in Pune yet"), "{pune}");
    // Same city, but Meera wants women only and Ravi is a man: irrelevant
    // here; she has a budget far below the rent.
    let poor = world.say(&MEERA, "need a room in blr, budget 9k, female, any gender, veg, tidy, non smoker").await;
    assert!(!poor.contains(&asha_code), "{poor}");
    let interest = world.say(&RAVI, &format!("I like {asha_code}")).await;
    assert!(interest.contains("looking for something different"), "{interest}");
}

#[tokio::test]
async fn delete_erases_everything_after_a_confirmation() {
    let Some(world) = World::new().await else { return };
    let asha_code = onboard_asha(&world).await;
    let confirm = world.say(&ASHA, "delete").await;
    assert!(confirm.contains("delete yes"), "{confirm}");
    let gone = world.say(&ASHA, "delete yes").await;
    assert!(gone.contains("Everything is erased"), "{gone}");
    let code = domain::PublicCode::parse(&asha_code).unwrap();
    assert!(matches!(world.web.card(&code).await, Err(app::Error::NotFound)));
    // A new hello starts from nothing.
    let hello = world.say(&ASHA, "hi").await;
    assert!(hello.contains("Tell me in one message"), "{hello}");
}

#[tokio::test]
async fn delete_needs_the_confirmation_right_after() {
    let Some(world) = World::new().await else { return };
    onboard_asha(&world).await;
    world.say(&ASHA, "delete").await;
    world.say(&ASHA, "matches").await;
    let reply = world.say(&ASHA, "delete yes").await;
    assert!(reply.contains("Reply *delete yes*"), "the confirmation expired: {reply}");
}

#[tokio::test]
async fn a_failed_send_is_retried_later() {
    let Some(world) = World::new().await else { return };
    *world.recorder.failures.lock().unwrap() = 1;
    world.deliver(&ASHA, Body::Text("hi".into())).await;
    assert_eq!(world.inbox(&ASHA).await, "", "the first send failed");
    world.advance(Duration::seconds(6));
    let retried = world.inbox(&ASHA).await;
    assert!(retried.contains("Tell me in one message"), "{retried}");
}

#[tokio::test]
async fn alerts_wait_for_morning_in_india() {
    let Some(world) = World::new().await else { return };
    // 23:30 in India.
    *world.now.lock().unwrap() = "2026-11-16T18:00:00Z".parse().unwrap();
    onboard_asha(&world).await;
    world.inbox(&ASHA).await;
    let ravi = world.say(&RAVI, "need a room in koramangala blr 18k, male, any gender, veg, tidy, non smoker").await;
    assert!(ravi.contains("You are live"), "replies are never delayed: {ravi}");
    assert_eq!(world.inbox(&ASHA).await, "", "the alert waits");
    world.advance(Duration::hours(9));
    assert!(world.inbox(&ASHA).await.contains("New match for you"));
}

#[tokio::test]
async fn the_map_shows_blurred_pins_and_the_link_signs_in() {
    let Some(world) = World::new().await else { return };
    let asha_code = onboard_asha(&world).await;
    world.inbox(&ASHA).await;
    world.deliver(&ASHA, Body::Location { lat: 12.935_27, lng: 77.624_51 }).await;
    let updated = world.inbox(&ASHA).await;
    assert!(updated.contains("Updated"), "{updated}");

    let pins = world.web.pins(None, &app::web::PinFilter::default()).await.unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].code.as_str(), asha_code);
    assert!((pins[0].lat - 12.935_27).abs() < 0.01 && (pins[0].lat - 12.935_27).abs() > 0.0, "{}", pins[0].lat);
    let json = serde_json::to_string(&world.web.card(&pins[0].code).await.unwrap()).unwrap();
    assert!(!json.contains("12.935") && !json.contains("9198"), "{json}");

    let reply = world.say(&ASHA, "link").await;
    let token = reply.rsplit("/m/").next().unwrap().trim().to_owned();
    let session = world.web.sign_in(&token, world.now()).await.unwrap();
    assert!(world.web.sign_in(&token, world.now()).await.is_err(), "a link works once");
    let me = world.web.me(&session, world.now()).await.unwrap();
    assert_eq!(me.card.unwrap().code.as_str(), asha_code);
    assert!(me.has_pin);

    let patch = domain::ProfilePatch { budget_max: Some(17_000), budget_min: Some(17_000), ..Default::default() };
    let card = world.web.update(&session, patch, world.now()).await.unwrap();
    assert_eq!(card.budget_max, 17_000);
    assert!(world.web.me("not-a-session", world.now()).await.is_err());
}

#[tokio::test]
async fn unprompted_alerts_stop_at_the_daily_cap() {
    let Some(world) = World::new().await else { return };
    onboard_asha(&world).await;
    world.inbox(&ASHA).await;
    let people = ["919800000010", "919800000011", "919800000012", "919800000013", "919800000014"];
    for phone in people {
        let person = Person { phone: Box::leak(phone.to_owned().into_boxed_str()), name: "X" };
        world.say(&person, "need a room in koramangala blr 18k, male, any gender, veg, tidy, non smoker").await;
    }
    let alerts = world.inbox(&ASHA).await;
    assert_eq!(alerts.matches("New match for you").count(), app::policy::ALERTS_PER_DAY as usize, "{alerts}");
}

#[tokio::test]
async fn shared_locations_update_the_area_without_tracking_live_movement() {
    let Some(world) = World::new().await else { return };
    onboard_asha(&world).await;
    let filter = app::web::PinFilter::default();

    world.deliver(&ASHA, Body::LiveLocation { lat: 12.935_27, lng: 77.624_51 }).await;
    assert!(world.inbox(&ASHA).await.contains("saved one point"));
    let pins = world.web.pins(None, &filter).await.unwrap();
    assert_eq!(pins.len(), 1);

    world.deliver(&ASHA, Body::LiveLocation { lat: 13.01, lng: 77.62 }).await;
    assert_eq!(world.inbox(&ASHA).await, "");
    let unchanged = world.web.pins(None, &filter).await.unwrap();
    assert_eq!((pins[0].lat, pins[0].lng), (unchanged[0].lat, unchanged[0].lng));

    world.deliver(&ASHA, Body::UnresolvedMapLink).await;
    assert!(world.inbox(&ASHA).await.contains("couldn't get a location"));
    assert_eq!(world.web.pins(None, &filter).await.unwrap().len(), 1);

    world.deliver(&ASHA, Body::Area("Test District".into())).await;
    assert!(world.inbox(&ASHA).await.contains("Saved your preferred area as *Test District*"));
    assert!(world.web.pins(None, &filter).await.unwrap().is_empty());

    world.deliver(&ASHA, Body::LiveLocation { lat: 13.01, lng: 77.62 }).await;
    assert_eq!(world.inbox(&ASHA).await, "");
    assert!(world.web.pins(None, &filter).await.unwrap().is_empty());

    world.deliver(&ASHA, Body::Location { lat: 12.97, lng: 77.59 }).await;
    assert!(world.inbox(&ASHA).await.contains("Updated"));
    assert_eq!(world.web.pins(None, &filter).await.unwrap().len(), 1);
}

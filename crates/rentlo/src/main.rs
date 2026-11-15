//! The Rentlo server. One binary, one subcommand per service:
//!
//! - `rentlo bot`: the WhatsApp session, the conversation, the outbox.
//!   Exactly one may run: two on one number would both answer.
//! - `rentlo migrate`: brings the schema up to date and exits.
//!
//! The command comes from the first argument or `RENTLO_COMMAND`. The rest
//! of the configuration is by environment; see [`Env`].

use app::{Bot, Dispatcher, Inbound, RulesExtractor, Settings, Store};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use store::PgStore;
use tokio::sync::mpsc;

/// Messages waiting for the bot. More than this means the bot is far
/// behind; WhatsApp redelivers nothing we drop, so the limit is generous.
const INBOX: usize = 1024;
/// The dispatcher looks for due messages at least this often, for ones
/// that became due by time (quiet hours, retries) rather than by insert.
const OUTBOX_POLL: Duration = Duration::from_secs(15);
const PRUNE_EVERY: Duration = Duration::from_secs(3600);
/// Tries per inbound message when the database fails.
const HANDLE_ATTEMPTS: u32 = 3;

struct Env {
    database_url: String,
    public_url: String,
    bot_phone: Option<String>,
    session_path: String,
    pairing_number: Option<String>,
}

impl Env {
    fn read() -> Result<Self, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let digits = |v: String| v.chars().filter(char::is_ascii_digit).collect::<String>();
        Ok(Self {
            database_url: var("DATABASE_URL").ok_or("DATABASE_URL is not set")?,
            public_url: var("PUBLIC_URL")
                .unwrap_or_else(|| "https://rentlo.inframan.live".into())
                .trim_end_matches('/')
                .into(),
            bot_phone: var("BOT_PHONE").map(digits),
            session_path: var("WA_SESSION").unwrap_or_else(|| "/data/whatsapp.db".into()),
            pairing_number: var("WA_PAIRING_NUMBER").map(digits),
        })
    }

    fn settings(&self) -> Settings {
        Settings { public_url: self.public_url.clone(), bot_phone: self.bot_phone.clone() }
    }
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn,whatsapp_rust=warn".into()),
        )
        .init();
    // From the command line, or from RENTLO_COMMAND so one image serves
    // every service with only an environment variable between them.
    let command = std::env::args().nth(1).or_else(|| std::env::var("RENTLO_COMMAND").ok()).unwrap_or_default();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(async {
        let env = Env::read()?;
        match command.as_str() {
            "bot" => bot(env).await,
            "migrate" => connect(&env, 2).await.map(|_| ()),
            other => Err(format!("unknown command {other:?}; use `rentlo bot` or `rentlo migrate`")),
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Connects and migrates. Every service migrates at start: the migrator
/// takes an advisory lock, so two starting at once do not race.
async fn connect(env: &Env, connections: u32) -> Result<PgStore, String> {
    let store = PgStore::connect(&env.database_url, connections).await.map_err(|e| format!("database: {e}"))?;
    store.migrate().await.map_err(|e| format!("migration: {e}"))?;
    Ok(store)
}

async fn bot(env: Env) -> Result<(), String> {
    let store = connect(&env, 5).await?;
    let shared: Arc<dyn Store> = Arc::new(store.clone());
    let link = wa::Link::default();
    let bot = Arc::new(Bot::new(shared.clone(), Arc::new(RulesExtractor), env.settings()));
    let dispatcher = Dispatcher::new(shared, Arc::new(link.clone()));

    let (inbox_tx, inbox) = mpsc::channel(INBOX);
    let worker = tokio::spawn(handle_all(bot, inbox));
    let sender = tokio::spawn(send_all(store.clone(), dispatcher, link.clone()));
    let pruner = tokio::spawn(prune(store));

    let config = wa::Config { session_path: env.session_path, pairing_number: env.pairing_number };
    let result = wa::run(config, link, inbox_tx, shutdown_signal()).await;
    // The inbox sender is gone with `run`, so the worker finishes what it
    // holds and stops.
    let _ = worker.await;
    sender.abort();
    pruner.abort();
    result
}

/// Handles messages one at a time, in arrival order. One person's messages
/// must not be handled concurrently, and at this volume one at a time is
/// simpler than per-person ordering.
async fn handle_all(bot: Arc<Bot>, mut inbox: mpsc::Receiver<Inbound>) {
    while let Some(message) = inbox.recv().await {
        let id = message.message_id.clone();
        for attempt in 1..=HANDLE_ATTEMPTS {
            match bot.handle(message.clone()).await {
                Ok(()) => break,
                Err(e) if attempt < HANDLE_ATTEMPTS => {
                    tracing::warn!(message = %id, attempt, "handling failed, retrying: {e}");
                    tokio::time::sleep(Duration::from_secs(u64::from(attempt))).await;
                }
                Err(e) => tracing::error!(message = %id, "handling failed, giving up: {e}"),
            }
        }
    }
}

/// Sends due messages when the database says some were queued, and on a
/// timer for those that became due by time.
async fn send_all(store: PgStore, dispatcher: Dispatcher, link: wa::Link) {
    loop {
        let mut listener = match store.listen(&[store::OUTBOX_CHANNEL]).await {
            Ok(listener) => listener,
            Err(e) => {
                tracing::warn!("outbox listener: {e}");
                tokio::time::sleep(OUTBOX_POLL).await;
                continue;
            }
        };
        loop {
            // While WhatsApp is down every send would fail and burn a retry.
            if link.is_connected()
                && let Err(e) = dispatcher.drain(chrono::Utc::now).await
            {
                tracing::warn!("outbox: {e}");
            }
            if let Ok(Err(e)) = tokio::time::timeout(OUTBOX_POLL, listener.recv()).await {
                tracing::warn!("outbox listener lost: {e}");
                break;
            }
        }
    }
}

async fn prune(store: PgStore) {
    loop {
        tokio::time::sleep(PRUNE_EVERY).await;
        match store.prune(chrono::Utc::now()).await {
            Ok(removed) => tracing::info!(removed, "pruned old rows"),
            Err(e) => tracing::warn!("prune: {e}"),
        }
    }
}

/// Resolves on SIGINT or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    tracing::info!("shutting down");
}

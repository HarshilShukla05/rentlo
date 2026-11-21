//! WhatsApp, through the unofficial WhatsApp Web client `whatsapp-rust`.
//!
//! Two directions, kept apart:
//! - [`run`] keeps the linked session connected and turns each direct
//!   message into an [`app::Inbound`] on a channel. It never waits on the
//!   bot's work, so WhatsApp's event loop is never blocked.
//! - [`Link`] sends text through whichever client is connected now. It is
//!   the [`app::Messenger`] the dispatcher uses.
//!
//! The session (keys, device identity) lives in a SQLite file on a disk
//! that survives restarts. Losing it means linking the phone again.

use app::{Body, Inbound, Messenger, SendError};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use whatsapp_rust::http::UreqHttpClient;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::*;
use whatsapp_rust::transport::TokioWebSocketTransportFactory;
use whatsapp_rust::types::events::{Event, EventKind};
use whatsapp_rust::wacore_binary::JidExt;

/// Messages older than this when they arrive (queued while the bot was
/// down) are still answered; older ones are dropped as stale.
const MAX_AGE_HOURS: i64 = 24;
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct Config {
    /// The SQLite file that holds the linked session.
    pub session_path: String,
    /// The bot phone's number in digits, to link by an 8-character code
    /// instead of a QR. Without it, the QR is printed to the log.
    pub pairing_number: Option<String>,
}

/// Sends through the connected client. Cheap to clone.
#[derive(Clone, Default)]
pub struct Link {
    client: Arc<RwLock<Option<Arc<Client>>>>,
}

impl Link {
    fn set(&self, client: Option<Arc<Client>>) {
        if let Ok(mut slot) = self.client.write() {
            *slot = client;
        }
    }

    fn current(&self) -> Option<Arc<Client>> {
        self.client.read().ok().and_then(|slot| slot.clone())
    }

    pub fn is_connected(&self) -> bool {
        self.current().is_some()
    }
}

#[async_trait]
impl Messenger for Link {
    async fn send_text(&self, jid: &str, text: &str) -> Result<(), SendError> {
        let client = self.current().ok_or_else(|| SendError("WhatsApp is not connected".into()))?;
        let to = Jid::from_str(jid).map_err(|e| SendError(format!("bad address {jid}: {e}")))?;
        let message = whatsapp_rust::waproto::whatsapp::Message::text(text);
        match tokio::time::timeout(SEND_TIMEOUT, client.send_message(&to, message)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(SendError(e.to_string())),
            Err(_) => Err(SendError("timed out".into())),
        }
    }
}

/// Keeps the session connected until `shutdown` resolves, reconnecting with
/// backoff. Inbound direct messages go to `inbox`.
pub async fn run(
    config: Config,
    link: Link,
    inbox: mpsc::Sender<Inbound>,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), String> {
    const FIRST_RETRY: std::time::Duration = std::time::Duration::from_secs(2);
    const MAX_RETRY: std::time::Duration = std::time::Duration::from_secs(60);
    const HEALTHY: std::time::Duration = std::time::Duration::from_secs(300);

    tokio::pin!(shutdown);
    let mut retry = FIRST_RETRY;
    loop {
        let started = tokio::time::Instant::now();
        tracing::info!(session = %config.session_path, "connecting to WhatsApp");
        match connect(&config, link.clone(), inbox.clone()).await {
            Ok(bot) => {
                let mut handle = bot.spawn();
                link.set(Some(handle.client()));
                tokio::select! {
                    _ = &mut handle => tracing::warn!("WhatsApp client stopped"),
                    () = &mut shutdown => {
                        link.set(None);
                        handle.shutdown().await;
                        return Ok(());
                    }
                }
                link.set(None);
            }
            // A session file that cannot open will not open on a retry.
            Err(e) if e.starts_with("session store") => return Err(e),
            Err(e) => tracing::error!("{e}"),
        }
        if started.elapsed() > HEALTHY {
            retry = FIRST_RETRY;
        }
        tracing::info!("connecting to WhatsApp again in {} s", retry.as_secs());
        tokio::select! {
            () = tokio::time::sleep(retry) => {}
            () = &mut shutdown => return Ok(()),
        }
        retry = (retry * 2).min(MAX_RETRY);
    }
}

async fn connect(config: &Config, link: Link, inbox: mpsc::Sender<Inbound>) -> Result<Bot, String> {
    let store = SqliteStore::new(&config.session_path)
        .await
        .map_err(|e| format!("session store {}: {e}", config.session_path))?;
    let pairing = config.pairing_number.is_some();
    let mut builder = Bot::builder()
        .with_backend(store)
        .with_transport_factory(TokioWebSocketTransportFactory::new())
        .with_http_client(UreqHttpClient::new())
        .with_runtime(TokioRuntime)
        .skip_history_sync()
        .on_qr_code(move |code, timeout| async move {
            if !pairing {
                tracing::info!(
                    "link the bot phone: WhatsApp → Linked devices → Link a device, scan this QR (valid {} s):\n{code}",
                    timeout.as_secs()
                );
            }
        })
        .on_pair_code(|code, timeout| async move {
            tracing::info!(
                "link the bot phone: WhatsApp → Linked devices → Link a device → Link with phone number instead → enter {code} (valid {} s)",
                timeout.as_secs()
            );
        })
        .on_pair_code_error(|error, _| async move {
            tracing::warn!(
                rejection = ?error.rejection,
                retry_after_seconds = ?error.backoff.map(|delay| delay.as_secs()),
                "WhatsApp pairing code request failed"
            );
        })
        .on_event_for(&[EventKind::PairSuccess, EventKind::PairError, EventKind::StreamReplaced], |event, _| async move {
            match event.as_ref() {
                Event::PairSuccess(_) => tracing::info!("WhatsApp pairing confirmed"),
                Event::PairError(_) => tracing::warn!("WhatsApp pairing confirmation failed"),
                Event::StreamReplaced(_) => tracing::warn!("WhatsApp stream replaced by another session"),
                _ => {}
            }
        })
        .on_connected(|_| async { tracing::info!("WhatsApp connected") })
        .on_logged_out(move |event| {
            link.set(None);
            async move {
                // Logout stanzas can carry account-specific details. Log only the protocol status.
                let stream_code = event
                    .raw
                    .as_ref()
                    .filter(|node| node.tag == "stream:error")
                    .and_then(|node| node.attrs().optional_string("code"))
                    .and_then(|value| value.parse::<u16>().ok());
                let device_removed = event
                    .raw
                    .as_ref()
                    .and_then(|node| node.get_optional_child("conflict"))
                    .and_then(|conflict| conflict.attrs().optional_string("type"))
                    .is_some_and(|kind| kind == "device_removed");
                tracing::error!(
                    reason = ?event.reason,
                    on_connect = event.on_connect,
                    ?stream_code,
                    device_removed,
                    "WhatsApp logged this device out"
                );
            }
        })
        .on_message(move |ctx| {
            if let Some(message) = inbound(&ctx)
                && inbox.try_send(message).is_err()
            {
                tracing::warn!("inbox full: dropped a message");
            }
            async {}
        });
    if let Some(phone_number) = config.pairing_number.clone() {
        builder = builder.with_pair_code(PairCodeOptions { phone_number, ..Default::default() });
    }
    builder.build().await.map_err(|e| format!("WhatsApp: {e}"))
}

/// A direct message from a person, or `None` for everything else: groups,
/// status updates, channels, our own messages, stale backlog.
fn inbound(ctx: &MessageContext) -> Option<Inbound> {
    let info = &ctx.info;
    let source = &info.source;
    if source.is_from_me
        || source.is_group
        || source.chat.is_group()
        || source.chat.is_status_broadcast()
        || source.chat.is_newsletter()
    {
        return None;
    }
    if Utc::now() - info.timestamp > Duration::hours(MAX_AGE_HOURS) {
        return None;
    }
    let phone = [Some(&source.sender), source.sender_alt.as_ref()]
        .into_iter()
        .flatten()
        .find(|jid| jid.is_pn())
        .map(|jid| jid.user.to_string());
    let name = Some(info.push_name.trim()).filter(|n| !n.is_empty()).map(str::to_owned);
    Some(Inbound {
        message_id: info.id.to_string(),
        jid: source.chat.to_non_ad_string(),
        phone,
        name,
        body: body(&ctx.message),
        at: info.timestamp,
    })
}

fn body(message: &whatsapp_rust::waproto::whatsapp::Message) -> Body {
    if let Some(text) = message.text_content() {
        return Body::Text(text.to_owned());
    }
    if let Some(location) = message.location_message.as_option()
        && let (Some(lat), Some(lng)) = (location.degrees_latitude, location.degrees_longitude)
    {
        return Body::Location { lat, lng };
    }
    Body::Unsupported
}

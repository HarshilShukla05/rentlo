//! Sends what the use cases queued.
//!
//! Delivery is at least once: a message is marked sent only after WhatsApp
//! accepted it, so a crash between the two sends it again. The dedupe keys
//! upstream make sure each message was queued once.

use crate::policy;
use crate::ports::{Messenger, Store, StoreResult};
use chrono::{DateTime, Utc};
use std::sync::Arc;

pub struct Dispatcher {
    store: Arc<dyn Store>,
    messenger: Arc<dyn Messenger>,
}

impl Dispatcher {
    pub fn new(store: Arc<dyn Store>, messenger: Arc<dyn Messenger>) -> Self {
        Self { store, messenger }
    }

    /// Sends the oldest due message. False when nothing was due.
    pub async fn send_next(&self, now: DateTime<Utc>) -> StoreResult<bool> {
        let mut tx = self.store.begin().await?;
        let Some(message) = tx.claim_due(now).await? else {
            tx.commit().await?;
            return Ok(false);
        };
        match self.messenger.send_text(&message.jid, &message.text).await {
            Ok(()) => tx.mark_sent(message.id, now).await?,
            Err(error) => {
                let tried = message.attempts + 1;
                let retry_at = (tried < policy::MAX_SEND_ATTEMPTS).then(|| now + policy::retry_delay(message.attempts));
                tracing::warn!(id = message.id, tried, gave_up = retry_at.is_none(), "{error}");
                tx.mark_failed(message.id, &error.0, retry_at).await?;
            }
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Sends every due message. Returns how many were tried.
    pub async fn drain(&self, now: impl Fn() -> DateTime<Utc>) -> StoreResult<usize> {
        let mut count = 0;
        while self.send_next(now()).await? {
            count += 1;
        }
        Ok(count)
    }
}

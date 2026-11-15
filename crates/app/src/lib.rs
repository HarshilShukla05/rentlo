//! What Rentlo does, written against ports. The adapters behind the ports
//! (PostgreSQL, WhatsApp, a language model) live in other crates; nothing in
//! this one names them.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod bot;
pub mod copy;
pub mod dispatch;
pub mod model;
pub mod ops;
pub mod policy;
pub mod ports;
pub mod web;

pub use bot::{Body, Bot, Inbound};
pub use dispatch::Dispatcher;
pub use model::{Outgoing, Profile, Queued, Settings, Topic, User};
pub use ports::{Messenger, ProfileExtractor, RulesExtractor, SendError, Store, StoreError, Tx};
pub use web::Web;

/// Why a use case did not complete.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("not signed in")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
}

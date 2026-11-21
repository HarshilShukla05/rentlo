//! Drives a fresh QA deployment through WhatsApp using a dedicated linked account.

use chrono::Utc;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use whatsapp_rust::http::UreqHttpClient;
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::*;
use whatsapp_rust::transport::TokioWebSocketTransportFactory;
use whatsapp_rust::waproto::whatsapp::{Message, message};

const WAIT: Duration = Duration::from_secs(45);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let session = required("RENTLO_QA_TESTER_SESSION")?;
    let mode = std::env::args().nth(1);
    let pairing = mode.as_deref() == Some("pair");
    if pairing && std::env::var_os("GITHUB_ACTIONS").is_some() {
        return Err("pair the QA tester from a private terminal".into());
    }
    if !pairing && !Path::new(&session).is_file() {
        return Err("pair the dedicated QA tester before running this flow".into());
    }
    let store = SqliteStore::new(&session).await.map_err(|_| "could not open QA tester session")?;
    let builder = Bot::builder()
        .with_backend(store.clone())
        .with_transport_factory(TokioWebSocketTransportFactory::new())
        .with_http_client(UreqHttpClient::new())
        .with_runtime(TokioRuntime)
        .skip_history_sync();
    if pairing {
        let phone_number = required("RENTLO_QA_TESTER_PHONE")?;
        let (linked, mut ready) = mpsc::channel(1);
        let mut handle = builder
            .with_pair_code(PairCodeOptions { phone_number, ..Default::default() })
            .on_pair_code(|code, timeout| async move {
                eprintln!("Enter {code} on the QA tester phone within {} seconds.", timeout.as_secs());
            })
            .on_connected(move |_| {
                let _ = linked.try_send(());
                async {}
            })
            .build()
            .await
            .map_err(|_| "could not initialize QA pairing")?
            .spawn();
        let result = tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(240), ready.recv()) => {
                result.ok().flatten().ok_or("QA pairing did not finish")
            }
            _ = &mut handle => Err("QA pairing client disconnected"),
        };
        handle.shutdown().await;
        result?;
        println!("QA tester session ready");
        return Ok(());
    }
    let phone = test_number("RENTLO_QA_BOT_PHONE")?;
    let tester = test_number("RENTLO_QA_TESTER_PHONE")?;
    if tester == phone {
        return Err("the QA tester and bot must be different accounts".into());
    }
    verify_account(&store, &tester, "tester").await?;
    if mode.as_deref() == Some("check-sessions") {
        let bot_session = required("WA_SESSION")?;
        if !Path::new(&bot_session).is_file() {
            return Err("pair the dedicated QA bot before running this flow".into());
        }
        let bot_store = SqliteStore::new(&bot_session).await.map_err(|_| "could not open QA bot session")?;
        verify_account(&bot_store, &phone, "bot").await?;
        return Ok(());
    }
    let to = Jid::from_str(&format!("{phone}@s.whatsapp.net")).map_err(|_| "bad QA bot address")?;
    let (replies, mut inbox) = mpsc::channel(64);
    let (linked, mut ready) = mpsc::channel(1);
    let mut handle = builder
        .on_connected(move |_| {
            let _ = linked.try_send(());
            async {}
        })
        .on_message(move |ctx| {
            let source = &ctx.info.source;
            if !source.is_from_me
                && !source.is_group
                && Utc::now() - ctx.info.timestamp < chrono::Duration::minutes(1)
                && [&source.sender, &source.chat]
                    .into_iter()
                    .chain(source.sender_alt.as_ref())
                    .any(|jid| jid.is_pn() && jid.user == phone)
                && let Some(reply) = ctx.message.text_content()
            {
                let _ = replies.try_send(reply.to_owned());
            }
            async {}
        })
        .build()
        .await
        .map_err(|_| "could not initialize QA tester")?
        .spawn();
    let client = handle.client();
    let result = tokio::select! {
        result = run(&client, &to, &mut ready, &mut inbox) => result,
        _ = &mut handle => Err("QA tester client disconnected".into()),
    };
    handle.shutdown().await;
    result
}

async fn run(
    client: &Client,
    to: &Jid,
    ready: &mut mpsc::Receiver<()>,
    inbox: &mut mpsc::Receiver<String>,
) -> Result<(), String> {
    tokio::time::timeout(WAIT, ready.recv()).await.ok().flatten().ok_or("QA tester could not connect")?;
    let send = |message| async move {
        tokio::time::timeout(WAIT, client.send_message(to, message))
            .await
            .map_err(|_| "WhatsApp send timed out")?
            .map(|_| ())
            .map_err(|_| "WhatsApp send failed")
    };
    send(Message::text("Hi")).await?;
    expect(inbox, "Greeting", "Tell me in one message").await?;
    send(Message::text("Need a room in Hyderabad, 20k, female, any gender, sleeps early, tidy, veg, non smoker"))
        .await?;
    expect(inbox, "Publish a synthetic profile", "You are live").await?;

    send(Message {
        live_location_message: Some(message::LiveLocationMessage {
            degrees_latitude: Some(17.42),
            degrees_longitude: Some(78.36),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    })
    .await?;
    expect(inbox, "Native live location snapshot", "saved one point").await?;
    expect(inbox, "Persist the live location", "Updated").await?;

    send(Message {
        location_message: Some(message::LocationMessage {
            degrees_latitude: Some(17.44),
            degrees_longitude: Some(78.37),
            ..Default::default()
        })
        .into(),
        ..Default::default()
    })
    .await?;
    expect(inbox, "Native WhatsApp pin", "Updated").await?;

    send(Message::text("https://www.google.com/maps/search/?api=1&query=17.45%2C78.38")).await?;
    expect(inbox, "Google Maps coordinate link", "Updated").await?;
    send(Message::text("https://www.google.com/maps/place/Financial+District")).await?;
    expect(inbox, "Save a named Google Maps area", "Saved your preferred area as *Financial District*").await?;
    expect(inbox, "Persist the named area", "Updated").await?;

    send(Message::text("profile")).await?;
    expect(inbox, "Read the saved area", "Financial District").await?;
    send(Message::text("delete")).await?;
    expect(inbox, "Request erasure of the synthetic profile", "delete yes").await?;
    send(Message::text("delete yes")).await?;
    expect(inbox, "Erase the synthetic profile", "Everything is erased").await?;
    Ok(())
}

async fn expect(inbox: &mut mpsc::Receiver<String>, step: &str, expected: &str) -> Result<(), String> {
    let found = tokio::time::timeout(WAIT, async {
        while let Some(reply) = inbox.recv().await {
            if reply.contains(expected) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    if !found {
        println!("FAIL | {step}");
        return Err(format!("expected reply not received during {step}"));
    }
    // Report only the assertion label. Incoming payloads can contain account data or links.
    println!("PASS | {step}");
    Ok(())
}

async fn verify_account(store: &SqliteStore, expected: &str, role: &str) -> Result<(), String> {
    let matches = store
        .load_device_data_for_device(store.device_id())
        .await
        .map_err(|_| format!("could not read QA {role} session"))?
        .and_then(|device| device.pn)
        .is_some_and(|jid| jid.user == expected);
    if matches { Ok(()) } else { Err(format!("QA {role} session does not match its configured phone number")) }
}

fn test_number(name: &str) -> Result<String, String> {
    let number = required(name)?;
    if (8..=15).contains(&number.len()) && number.bytes().all(|value| value.is_ascii_digit()) {
        Ok(number)
    } else {
        Err(format!("{name} must contain country code and phone digits"))
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} must be set"))
}

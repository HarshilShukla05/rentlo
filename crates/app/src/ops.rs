//! Operations both the chat and the web perform. Each runs inside the
//! caller's transaction, so its writes commit with the caller's.

use crate::copy;
use crate::model::{Outgoing, Profile, Topic, User};
use crate::policy;
use crate::ports::{StoreResult, Tx};
use chrono::{DateTime, Duration, Utc};
use domain::matching::{self, MIN_SCORE};
use domain::{Decided, Decision, Match, MatchStatus, ProfileData, ProfileStatus};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use uuid::Uuid;

pub struct Published {
    pub profile: Profile,
    /// The profile did not exist before.
    pub fresh: bool,
    /// Pairs made by this publish.
    pub new_matches: usize,
}

/// Saves `data` as the user's live profile and pairs it with everyone it
/// fits. Each new counterpart gets an alert, within their daily cap.
pub async fn publish(tx: &mut dyn Tx, user: &User, data: &ProfileData, now: DateTime<Utc>) -> StoreResult<Published> {
    let fresh = tx.profile_of(user.id).await?.is_none();
    let profile = tx.save_profile(user.id, data, now).await?;
    // An edit recomputes the open pairs. People already told about this
    // profile are not told again.
    let already_alerted: HashSet<_> = tx.delete_undecided_matches(profile.id).await?.into_iter().collect();

    let mut found = Vec::new();
    for other in tx.candidates(&profile, policy::CANDIDATES_PER_PUBLISH).await? {
        if !matching::eligible(&profile.data, &other.data) {
            continue;
        }
        let score = matching::score(&profile.data, &other.data);
        if score < MIN_SCORE {
            continue;
        }
        if let Some(pair) = tx.insert_match(profile.id, other.id, score, now).await? {
            found.push((pair, other));
        }
    }
    found.sort_by_key(|(pair, _)| std::cmp::Reverse(pair.score));

    let to_alert = found.iter().filter(|(_, other)| !already_alerted.contains(&other.id));
    for (pair, other) in to_alert.take(policy::ALERTS_PER_PUBLISH) {
        alert(tx, &profile, other, pair, now).await?;
    }
    Ok(Published { profile, fresh, new_matches: found.len() })
}

/// Tells `other` about `profile`, unless they are paused or had enough
/// alerts today.
async fn alert(
    tx: &mut dyn Tx,
    profile: &Profile,
    other: &Profile,
    pair: &Match,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    if other.status != ProfileStatus::Active {
        return Ok(());
    }
    let Some(recipient) = tx.user(other.user_id).await? else { return Ok(()) };
    if tx.alerts_since(recipient.id, now - Duration::days(1)).await? >= policy::ALERTS_PER_DAY {
        return Ok(());
    }
    let message = Outgoing {
        topic: Topic::Alert,
        dedupe_key: format!("{}:{}", pair.id, other.id),
        user: Some(recipient.id),
        jid: recipient.jid,
        text: copy::alert(&other.data, profile, pair.score),
        available_at: policy::next_allowed(now),
    };
    tx.enqueue(&message, now).await?;
    Ok(())
}

/// Pairs waiting for `profile`'s answer, best first.
pub async fn open_matches(tx: &mut dyn Tx, profile: &Profile) -> StoreResult<Vec<(Match, Profile)>> {
    let all = tx.matches_of(profile.id).await?;
    Ok(all
        .into_iter()
        .filter(|(pair, _)| pair.status() == MatchStatus::Suggested && pair.decision_of(profile.id).is_none())
        .collect())
}

pub enum Outcome {
    /// Yes recorded; the other side is asked.
    Liked,
    /// Both said yes. `intro` is the viewer's message with the contact.
    Mutual {
        intro: String,
    },
    Skipped,
    /// The viewer had already answered, or the pair was closed.
    AlreadyDecided,
    /// The other profile is gone.
    Unavailable,
}

/// Records `viewer`'s answer on `pair` and tells the other side what they
/// need to know: a yes asks them, a mutual yes introduces them, a skip says
/// nothing.
pub async fn decide(
    tx: &mut dyn Tx,
    viewer_user: &User,
    viewer: &Profile,
    mut pair: Match,
    accept: bool,
    now: DateTime<Utc>,
) -> StoreResult<Outcome> {
    let Some(other) = tx.profile(pair.other(viewer.id)).await? else { return Ok(Outcome::Unavailable) };
    let Some(other_user) = tx.user(other.user_id).await? else { return Ok(Outcome::Unavailable) };
    let decision = if accept { Decision::Yes } else { Decision::Skip };
    let decided = pair.decide(viewer.id, decision);
    if decided == Decided::Unchanged {
        return Ok(Outcome::AlreadyDecided);
    }
    tx.save_decisions(&pair, now).await?;

    let to_other = |topic: Topic, text: String| Outgoing {
        topic,
        dedupe_key: format!("{}:{}", pair.id, other.id),
        user: Some(other_user.id),
        jid: other_user.jid.clone(),
        text,
        available_at: policy::next_allowed(now),
    };
    match decided {
        Decided::Waiting if accept => {
            if other.status == ProfileStatus::Active {
                tx.enqueue(&to_other(Topic::Like, copy::like(&other.data, viewer, pair.score)), now).await?;
            }
            Ok(Outcome::Liked)
        }
        Decided::Mutual => {
            let theirs = copy::intro(viewer_user.first_name.as_deref(), viewer, viewer_user.phone.as_deref());
            tx.enqueue(&to_other(Topic::Intro, theirs), now).await?;
            let intro = copy::intro(other_user.first_name.as_deref(), &other, other_user.phone.as_deref());
            Ok(Outcome::Mutual { intro })
        }
        _ => Ok(Outcome::Skipped),
    }
}

/// A secret for a link or a cookie: 244 random bits, URL-safe.
pub fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Tokens are stored only as this hash, so a database leak leaks no session.
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

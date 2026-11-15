//! Limits that protect people from the bot, and the bot's number from a ban.

use chrono::{DateTime, Duration, FixedOffset, NaiveTime, TimeZone, Utc};

/// Unprompted match alerts one person may get in 24 hours.
pub const ALERTS_PER_DAY: u32 = 3;
/// Counterparts alerted when one profile goes live, best matches first.
pub const ALERTS_PER_PUBLISH: usize = 20;
/// Messages read from one person per minute. Beyond it the bot stays silent.
pub const INBOUND_PER_MINUTE: u32 = 20;
/// Match cards sent in one reply.
pub const CARDS_PER_REPLY: usize = 3;
/// Candidates scored when a profile goes live, most recently active first.
pub const CANDIDATES_PER_PUBLISH: u32 = 500;

/// A private link works once, within this time.
pub fn magic_link_ttl() -> Duration {
    Duration::minutes(15)
}

/// A web session lasts this long.
pub fn session_ttl() -> Duration {
    Duration::days(30)
}

/// Unprompted messages wait for this window, India time.
const QUIET_FROM_HOUR: u32 = 22;
const QUIET_UNTIL_HOUR: u32 = 8;
/// India has one time zone and no daylight saving.
const INDIA_OFFSET_SECONDS: i32 = 5 * 3600 + 30 * 60;

/// The earliest moment an unprompted message may go out: now, or 08:00
/// India time if it is night there.
pub fn next_allowed(now: DateTime<Utc>) -> DateTime<Utc> {
    let Some(india) = FixedOffset::east_opt(INDIA_OFFSET_SECONDS) else { return now };
    let local = now.with_timezone(&india);
    let hour = chrono::Timelike::hour(&local);
    if (QUIET_UNTIL_HOUR..QUIET_FROM_HOUR).contains(&hour) {
        return now;
    }
    let morning_date =
        if hour >= QUIET_FROM_HOUR { local.date_naive() + Duration::days(1) } else { local.date_naive() };
    let Some(morning) = NaiveTime::from_hms_opt(QUIET_UNTIL_HOUR, 0, 0) else { return now };
    india.from_local_datetime(&morning_date.and_time(morning)).single().map_or(now, |t| t.with_timezone(&Utc))
}

/// How long to wait before sending attempt number `attempts + 1`:
/// 5 s, 10 s, 20 s ... capped at an hour.
pub fn retry_delay(attempts: u32) -> Duration {
    const BASE_SECONDS: i64 = 5;
    const CAP_SECONDS: i64 = 3600;
    Duration::seconds((BASE_SECONDS << attempts.min(20)).min(CAP_SECONDS))
}

/// Sends tried before a message is given up on.
pub const MAX_SEND_ATTEMPTS: u32 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn daytime_in_india_sends_at_once() {
        // 09:00 UTC is 14:30 in India.
        let now = utc("2026-11-15T09:00:00Z");
        assert_eq!(next_allowed(now), now);
        // 02:30 UTC is exactly 08:00 in India: the window has opened.
        let opening = utc("2026-11-15T02:30:00Z");
        assert_eq!(next_allowed(opening), opening);
    }

    #[test]
    fn night_in_india_waits_for_eight() {
        // 17:00 UTC is 22:30 in India: wait for 08:00, which is 02:30 UTC next day.
        assert_eq!(next_allowed(utc("2026-11-15T17:00:00Z")), utc("2026-11-16T02:30:00Z"));
        // 20:00 UTC is 01:30 in India, already the next local day.
        assert_eq!(next_allowed(utc("2026-11-15T20:00:00Z")), utc("2026-11-16T02:30:00Z"));
        // 02:29 UTC is 07:59 in India.
        assert_eq!(next_allowed(utc("2026-11-15T02:29:00Z")), utc("2026-11-15T02:30:00Z"));
    }

    #[test]
    fn retries_back_off_and_stop_growing() {
        assert_eq!(retry_delay(0), Duration::seconds(5));
        assert_eq!(retry_delay(1), Duration::seconds(10));
        assert_eq!(retry_delay(3), Duration::seconds(40));
        assert_eq!(retry_delay(30), Duration::hours(1));
        assert_eq!(retry_delay(u32::MAX), Duration::hours(1));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! The rolling trial window: pure, time-travellable lifecycle decisions.
//!
//! A registered trial is a rolling window over authenticated activity. ANY authenticated
//! activity (read, write, import) refreshes the window to a full 14 days from that moment.
//! 14 days with no activity puts the trial in READ-ONLY: reads still work, writes are refused
//! with a message naming the trial's end and how to start again. 21 days with no activity
//! freezes the trial: the per-trial database is archived (copy + checksum), the live copy is
//! removed, and the account is kept so the same email can start fresh.
//!
//! This module is pure arithmetic over seconds-since-the-epoch: it takes no clock of its own,
//! so every decision is testable by time-travelling two integers.

pub const ACTIVE_WINDOW_DAYS: i64 = 14;
pub const READ_ONLY_WINDOW_DAYS: i64 = 7;
pub const ACTIVE_WINDOW_SECS: i64 = ACTIVE_WINDOW_DAYS * 86_400;
pub const READ_ONLY_WINDOW_SECS: i64 = READ_ONLY_WINDOW_DAYS * 86_400;
pub const FROZEN_AFTER_SECS: i64 = ACTIVE_WINDOW_SECS + READ_ONLY_WINDOW_SECS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Active,
    ReadOnly,
    Frozen,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Active => "active",
            Phase::ReadOnly => "read_only",
            Phase::Frozen => "frozen",
        }
    }

    pub fn parse(raw: &str) -> Option<Phase> {
        match raw {
            "active" => Some(Phase::Active),
            "read_only" => Some(Phase::ReadOnly),
            "frozen" => Some(Phase::Frozen),
            _ => None,
        }
    }
}

pub fn phase(last_activity_at: i64, now: i64) -> Phase {
    let idle = now.saturating_sub(last_activity_at);
    if idle < ACTIVE_WINDOW_SECS {
        Phase::Active
    } else if idle < FROZEN_AFTER_SECS {
        Phase::ReadOnly
    } else {
        Phase::Frozen
    }
}

pub fn writable(last_activity_at: i64, now: i64) -> bool {
    phase(last_activity_at, now) == Phase::Active
}

pub fn read_only_refusal(last_activity_at: i64, now: i64) -> String {
    let freeze_at = last_activity_at.saturating_add(FROZEN_AFTER_SECS);
    let idle_days = now.saturating_sub(last_activity_at) / 86_400;
    format!("this trial is read-only after {} days without activity; it will be archived at {} (seconds since the epoch). Sign in and use it - any authenticated activity, even a read, restores the full 14-day window and writes are accepted again.", idle_days, freeze_at)
}

pub fn write_refusal(last_activity_at: i64, now: i64) -> Option<String> {
    match phase(last_activity_at, now) {
        Phase::Active => None,
        Phase::ReadOnly => Some(read_only_refusal(last_activity_at, now)),
        Phase::Frozen => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    #[test]
    fn a_fresh_trial_is_active_for_thirteen_days() {
        let start = 1_700_000_000;
        assert_eq!(phase(start, start), Phase::Active);
        assert_eq!(phase(start, start + 13 * DAY), Phase::Active);
        assert_eq!(phase(start, start + ACTIVE_WINDOW_SECS - 1), Phase::Active);
    }

    #[test]
    fn fourteen_days_idle_is_read_only_and_twenty_one_is_frozen() {
        let start = 1_700_000_000;
        assert_eq!(phase(start, start + ACTIVE_WINDOW_SECS), Phase::ReadOnly);
        assert_eq!(phase(start, start + FROZEN_AFTER_SECS - 1), Phase::ReadOnly);
        assert_eq!(phase(start, start + FROZEN_AFTER_SECS), Phase::Frozen);
    }

    #[test]
    fn activity_on_day_thirteen_refreshes_the_full_window() {
        let start = 1_700_000_000;
        let refreshed = start + 13 * DAY;
        assert_eq!(phase(start, refreshed), Phase::Active);
        assert_eq!(phase(refreshed, refreshed + 13 * DAY), Phase::Active);
        assert_eq!(phase(refreshed, refreshed + 14 * DAY), Phase::ReadOnly);
    }

    #[test]
    fn a_read_only_refusal_names_the_end_and_the_restart() {
        let start = 1_700_000_000;
        let now = start + ACTIVE_WINDOW_SECS;
        let message = read_only_refusal(start, now);
        assert!(message.contains("read-only"), "{}", message);
        assert!(
            message.contains(&(start + FROZEN_AFTER_SECS).to_string()),
            "{}",
            message
        );
        assert!(
            message.contains("restores the full 14-day window"),
            "{}",
            message
        );
    }

    #[test]
    fn write_refusal_is_none_when_active_and_some_when_read_only() {
        let start = 1_700_000_000;
        assert!(write_refusal(start, start).is_none());
        assert!(write_refusal(start, start + ACTIVE_WINDOW_SECS).is_some());
        assert!(write_refusal(start, start + FROZEN_AFTER_SECS).is_none());
    }
}

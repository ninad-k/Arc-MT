//! The US equity session clock, in New York time.
//!
//! New York is UTC-5, and UTC-4 from 02:00 local on the second Sunday of March to 02:00 local on
//! the first Sunday of November (the US rule since 2007). Computed here rather than with a time
//! zone database, so the crate needs no new dependency.
//!
//! Phases on a trading day (local time): pre-market 04:00-09:30, regular 09:30-16:00 (13:00 on
//! an early-close day), after-hours to 20:00, closed otherwise. Weekends and NYSE holidays are
//! closed all day. The built-in tables cover 2026 and 2027; config can add days.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};

/// NYSE full-day holidays, 2026 and 2027.
pub const NYSE_HOLIDAYS: &[(i32, u32, u32)] = &[
    (2026, 1, 1),
    (2026, 1, 19),
    (2026, 2, 16),
    (2026, 4, 3),
    (2026, 5, 25),
    (2026, 6, 19),
    (2026, 7, 3),
    (2026, 9, 7),
    (2026, 11, 26),
    (2026, 12, 25),
    (2027, 1, 1),
    (2027, 1, 18),
    (2027, 2, 15),
    (2027, 3, 26),
    (2027, 5, 31),
    (2027, 6, 18),
    (2027, 7, 5),
    (2027, 9, 6),
    (2027, 11, 25),
    (2027, 12, 24),
];

/// NYSE early closes (13:00 New York), 2026 and 2027.
pub const NYSE_EARLY_CLOSES: &[(i32, u32, u32)] = &[(2026, 11, 27), (2026, 12, 24), (2027, 11, 26)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    PreMarket,
    Open,
    AfterHours,
    Closed,
}

impl Phase {
    pub const fn label(self) -> &'static str {
        match self {
            Phase::PreMarket => "Pre-market",
            Phase::Open => "Open",
            Phase::AfterHours => "After-hours",
            Phase::Closed => "Closed",
        }
    }
}

/// The trading days the clock knows about. `Default` is the built-in tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub holidays: Vec<NaiveDate>,
    pub early_closes: Vec<NaiveDate>,
}

impl Default for Exchange {
    fn default() -> Self {
        let days = |t: &[(i32, u32, u32)]| {
            t.iter()
                .filter_map(|&(y, m, d)| NaiveDate::from_ymd_opt(y, m, d))
                .collect()
        };
        Self {
            holidays: days(NYSE_HOLIDAYS),
            early_closes: days(NYSE_EARLY_CLOSES),
        }
    }
}

/// The clock at one moment, as the HUD shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub phase: Phase,
    pub label: String,
    /// New York local time, `HH:MM`.
    pub ny_time: String,
    /// New York local date, `YYYY-MM-DD`.
    pub ny_date: String,
    /// `"EST"` or `"EDT"`.
    pub ny_zone: String,
    /// The next phase change (ISO 8601 UTC) and what it changes to. `None` only if none is found
    /// in the next two weeks.
    pub next_change: Option<String>,
    pub next_phase: Option<Phase>,
    pub seconds_to_change: Option<i64>,
    /// Seconds to the regular close while the market is open.
    pub seconds_to_close: Option<i64>,
    pub trading_day: bool,
    pub early_close: bool,
}

/// Start of daylight saving time in UTC for `year` (second Sunday of March, 02:00 EST = 07:00 UTC).
fn dst_start(year: i32) -> Option<DateTime<Utc>> {
    let day = nth_weekday(year, 3, Weekday::Sun, 2)?;
    Some(Utc.from_utc_datetime(&day.and_hms_opt(7, 0, 0)?))
}

/// End of daylight saving time in UTC (first Sunday of November, 02:00 EDT = 06:00 UTC).
fn dst_end(year: i32) -> Option<DateTime<Utc>> {
    let day = nth_weekday(year, 11, Weekday::Sun, 1)?;
    Some(Utc.from_utc_datetime(&day.and_hms_opt(6, 0, 0)?))
}

/// The `n`-th (1-based) `weekday` of a month.
pub fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u32) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let offset = (7 + weekday.num_days_from_monday() - first.weekday().num_days_from_monday()) % 7;
    let day = first + Duration::days(i64::from(offset + 7 * (n.checked_sub(1)?)));
    (day.month() == month).then_some(day)
}

/// New York's offset from UTC in hours at an instant: -4 in daylight saving time, else -5.
pub fn ny_offset_hours(at: DateTime<Utc>) -> i64 {
    let year = at.year();
    match (dst_start(year), dst_end(year)) {
        (Some(start), Some(end)) if at >= start && at < end => -4,
        _ => -5,
    }
}

/// New York local wall-clock time at an instant.
pub fn ny_local(at: DateTime<Utc>) -> chrono::NaiveDateTime {
    at.naive_utc() + Duration::hours(ny_offset_hours(at))
}

/// The UTC instant of a New York local time. Only used for times after 02:00 local, which are
/// never ambiguous.
fn ny_to_utc(date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    let local = date.and_time(time);
    // The offset at noon local of that day is the offset of every time from 03:00 on.
    let noon_guess = Utc.from_utc_datetime(&(date.and_time(NaiveTime::MIN) + Duration::hours(17)));
    let offset = ny_offset_hours(noon_guess);
    Utc.from_utc_datetime(&(local - Duration::hours(offset)))
}

impl Exchange {
    pub fn is_trading_day(&self, date: NaiveDate) -> bool {
        !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) && !self.holidays.contains(&date)
    }

    pub fn is_early_close(&self, date: NaiveDate) -> bool {
        self.early_closes.contains(&date)
    }

    /// The trading day on or before `date`.
    pub fn trading_day_on_or_before(&self, mut date: NaiveDate) -> NaiveDate {
        for _ in 0..10 {
            if self.is_trading_day(date) {
                return date;
            }
            date -= Duration::days(1);
        }
        date
    }

    /// The phase boundaries of one local day, in order: (instant, phase that starts).
    fn boundaries(&self, date: NaiveDate) -> Vec<(DateTime<Utc>, Phase)> {
        if !self.is_trading_day(date) {
            return Vec::new();
        }
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::MIN);
        let close = if self.is_early_close(date) {
            t(13, 0)
        } else {
            t(16, 0)
        };
        vec![
            (ny_to_utc(date, t(4, 0)), Phase::PreMarket),
            (ny_to_utc(date, t(9, 30)), Phase::Open),
            (ny_to_utc(date, close), Phase::AfterHours),
            (ny_to_utc(date, t(20, 0)), Phase::Closed),
        ]
    }

    pub fn phase_at(&self, at: DateTime<Utc>) -> Phase {
        let date = ny_local(at).date();
        let mut phase = Phase::Closed;
        for (when, p) in self.boundaries(date) {
            if at >= when {
                phase = p;
            }
        }
        phase
    }

    /// The regular close of the local day of `at`, if it is a trading day.
    pub fn close_of(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let date = ny_local(at).date();
        self.boundaries(date)
            .into_iter()
            .find(|(_, p)| *p == Phase::AfterHours)
            .map(|(t, _)| t)
    }

    pub fn status(&self, at: DateTime<Utc>) -> SessionStatus {
        let local = ny_local(at);
        let date = local.date();
        let phase = self.phase_at(at);
        let next = (0..15)
            .flat_map(|d| self.boundaries(date + Duration::days(d)))
            .find(|(when, p)| *when > at && *p != phase);
        let seconds_to_close = (phase == Phase::Open)
            .then(|| self.close_of(at).map(|c| (c - at).num_seconds()))
            .flatten();
        SessionStatus {
            phase,
            label: phase.label().to_owned(),
            ny_time: local.format("%H:%M").to_string(),
            ny_date: date.format("%Y-%m-%d").to_string(),
            ny_zone: if ny_offset_hours(at) == -4 {
                "EDT"
            } else {
                "EST"
            }
            .to_owned(),
            next_change: next.map(|(t, _)| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            next_phase: next.map(|(_, p)| p),
            seconds_to_change: next.map(|(t, _)| (t - at).num_seconds()),
            seconds_to_close,
            trading_day: self.is_trading_day(date),
            early_close: self.is_early_close(date),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn dst_switches_on_the_us_dates() {
        // 2026: DST from 8 March to 1 November.
        assert_eq!(ny_offset_hours(utc("2026-03-08T06:59:00Z")), -5);
        assert_eq!(ny_offset_hours(utc("2026-03-08T07:00:00Z")), -4);
        assert_eq!(ny_offset_hours(utc("2026-11-01T05:59:00Z")), -4);
        assert_eq!(ny_offset_hours(utc("2026-11-01T06:00:00Z")), -5);
        assert_eq!(ny_offset_hours(utc("2026-07-01T12:00:00Z")), -4);
        assert_eq!(ny_offset_hours(utc("2026-01-15T12:00:00Z")), -5);
    }

    #[test]
    fn open_and_close_follow_new_york_in_both_seasons() {
        let x = Exchange::default();
        // Summer: 09:30 EDT is 13:30 UTC.
        assert_eq!(x.phase_at(utc("2026-07-01T13:29:00Z")), Phase::PreMarket);
        assert_eq!(x.phase_at(utc("2026-07-01T13:30:00Z")), Phase::Open);
        assert_eq!(x.phase_at(utc("2026-07-01T20:00:00Z")), Phase::AfterHours);
        assert_eq!(x.phase_at(utc("2026-07-02T00:00:00Z")), Phase::Closed);
        // Winter: 09:30 EST is 14:30 UTC.
        assert_eq!(x.phase_at(utc("2026-11-24T14:29:00Z")), Phase::PreMarket);
        assert_eq!(x.phase_at(utc("2026-11-24T14:30:00Z")), Phase::Open);
        assert_eq!(x.phase_at(utc("2026-11-24T20:59:00Z")), Phase::Open);
        assert_eq!(x.phase_at(utc("2026-11-24T21:00:00Z")), Phase::AfterHours);
        assert_eq!(x.phase_at(utc("2026-11-24T08:59:00Z")), Phase::Closed);
        assert_eq!(x.phase_at(utc("2026-11-24T09:00:00Z")), Phase::PreMarket);
    }

    #[test]
    fn weekends_and_holidays_are_closed() {
        let x = Exchange::default();
        assert_eq!(x.phase_at(utc("2026-11-28T16:00:00Z")), Phase::Closed); // Saturday
        assert_eq!(x.phase_at(utc("2026-11-26T16:00:00Z")), Phase::Closed); // Thanksgiving
        assert_eq!(x.phase_at(utc("2026-06-19T15:00:00Z")), Phase::Closed); // Juneteenth
    }

    #[test]
    fn early_close_ends_the_session_at_one() {
        let x = Exchange::default();
        // 27 November 2026, 13:00 EST = 18:00 UTC.
        assert_eq!(x.phase_at(utc("2026-11-27T17:59:00Z")), Phase::Open);
        assert_eq!(x.phase_at(utc("2026-11-27T18:00:00Z")), Phase::AfterHours);
        let s = x.status(utc("2026-11-27T17:50:00Z"));
        assert!(s.early_close);
        assert_eq!(s.seconds_to_close, Some(600));
    }

    #[test]
    fn status_counts_down_to_the_next_change() {
        let x = Exchange::default();
        // Tuesday 24 November 2026, 10:00 EST.
        let s = x.status(utc("2026-11-24T15:00:00Z"));
        assert_eq!(s.phase, Phase::Open);
        assert_eq!(s.ny_time, "10:00");
        assert_eq!(s.ny_zone, "EST");
        assert_eq!(s.next_phase, Some(Phase::AfterHours));
        assert_eq!(s.seconds_to_change, Some(6 * 3600));
        assert_eq!(s.seconds_to_close, Some(6 * 3600));

        // Wednesday 25 November after 20:00: Thanksgiving is closed, so next is Friday's pre-market.
        let s = x.status(utc("2026-11-26T02:00:00Z"));
        assert_eq!(s.phase, Phase::Closed);
        assert_eq!(s.next_phase, Some(Phase::PreMarket));
        assert_eq!(s.next_change.as_deref(), Some("2026-11-27T09:00:00Z"));
    }

    #[test]
    fn nth_weekday_finds_the_right_day() {
        assert_eq!(
            nth_weekday(2026, 3, Weekday::Sun, 2),
            NaiveDate::from_ymd_opt(2026, 3, 8)
        );
        assert_eq!(
            nth_weekday(2026, 11, Weekday::Sun, 1),
            NaiveDate::from_ymd_opt(2026, 11, 1)
        );
        assert_eq!(
            nth_weekday(2026, 6, Weekday::Fri, 3),
            NaiveDate::from_ymd_opt(2026, 6, 19)
        );
        assert_eq!(nth_weekday(2026, 2, Weekday::Mon, 5), None);
    }
}

//! The desk calendar: FOMC decision days and monthly options expiry.
//!
//! - **FOMC:** the days of rate decisions found in the MT5 economic calendar (US events whose
//!   title names the FOMC or the Fed's rate decision), plus the Fed's published 2026 schedule
//!   and any days added in config, so a month view has them even beyond the calendar's horizon.
//! - **Options expiry:** the third Friday of each month, or the trading day before it when that
//!   Friday is an exchange holiday.

use chrono::{Datelike, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

use crate::session::{Exchange, nth_weekday};

/// The second day of each 2026 FOMC meeting (the decision day), from the Fed's schedule.
pub const FOMC_2026: &[(i32, u32, u32)] = &[
    (2026, 1, 28),
    (2026, 3, 18),
    (2026, 4, 29),
    (2026, 6, 17),
    (2026, 7, 29),
    (2026, 9, 16),
    (2026, 10, 28),
    (2026, 12, 9),
];

/// Does a calendar event title name a Fed rate decision?
pub fn is_fomc_title(title: &str) -> bool {
    let t = title.to_ascii_lowercase();
    // Minutes, speeches, projections and press conferences are not the decision itself.
    if [
        "minutes",
        "speak",
        "member",
        "testimon",
        "projection",
        "press conference",
    ]
    .iter()
    .any(|w| t.contains(w))
    {
        return false;
    }
    t.contains("fed interest rate decision")
        || t.contains("federal funds rate")
        || t.contains("fomc statement")
        || t.contains("fomc rate decision")
        || ((t.contains("fed") || t.contains("fomc")) && t.contains("rate decision"))
}

/// Monthly options expiry for a month.
pub fn opex(exchange: &Exchange, year: i32, month: u32) -> Option<NaiveDate> {
    let third_friday = nth_weekday(year, month, Weekday::Fri, 3)?;
    Some(exchange.trading_day_on_or_before(third_friday))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Fomc,
    Opex,
    Holiday,
    EarlyClose,
}

/// A dot on the calendar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    /// `YYYY-MM-DD`, New York date.
    pub date: String,
    pub kind: MarkKind,
    pub label: String,
    /// `"mt5 calendar"`, `"fed schedule"`, `"config"` or `"exchange"`.
    pub source: String,
}

/// FOMC days from the three sources, without duplicates, oldest first.
pub fn fomc_days(
    from_calendar: &[NaiveDate],
    from_config: &[NaiveDate],
) -> Vec<(NaiveDate, &'static str)> {
    let mut out: Vec<(NaiveDate, &'static str)> = Vec::new();
    let mut push = |d: NaiveDate, src: &'static str| {
        if !out.iter().any(|(x, _)| *x == d) {
            out.push((d, src));
        }
    };
    for d in from_calendar {
        push(*d, "mt5 calendar");
    }
    for d in from_config {
        push(*d, "config");
    }
    for &(y, m, d) in FOMC_2026 {
        if let Some(d) = NaiveDate::from_ymd_opt(y, m, d) {
            push(d, "fed schedule");
        }
    }
    out.sort();
    out
}

/// The marks of one month.
pub fn month_marks(
    exchange: &Exchange,
    year: i32,
    month: u32,
    fomc: &[(NaiveDate, &'static str)],
) -> Vec<Mark> {
    let in_month = |d: &NaiveDate| d.year() == year && d.month() == month;
    let mut marks = Vec::new();
    for (d, src) in fomc.iter().filter(|(d, _)| in_month(d)) {
        marks.push(Mark {
            date: d.format("%Y-%m-%d").to_string(),
            kind: MarkKind::Fomc,
            label: "FOMC decision".into(),
            source: (*src).into(),
        });
    }
    if let Some(d) = opex(exchange, year, month) {
        marks.push(Mark {
            date: d.format("%Y-%m-%d").to_string(),
            kind: MarkKind::Opex,
            label: "Monthly options expiry".into(),
            source: "exchange".into(),
        });
    }
    for d in exchange.holidays.iter().filter(|d| in_month(d)) {
        marks.push(Mark {
            date: d.format("%Y-%m-%d").to_string(),
            kind: MarkKind::Holiday,
            label: "Exchange holiday".into(),
            source: "exchange".into(),
        });
    }
    for d in exchange.early_closes.iter().filter(|d| in_month(d)) {
        marks.push(Mark {
            date: d.format("%Y-%m-%d").to_string(),
            kind: MarkKind::EarlyClose,
            label: "Early close (13:00)".into(),
            source: "exchange".into(),
        });
    }
    marks.sort_by(|a, b| a.date.cmp(&b.date));
    marks
}

/// The next FOMC day on or after `today`.
pub fn next_fomc(
    fomc: &[(NaiveDate, &'static str)],
    today: NaiveDate,
) -> Option<(NaiveDate, &'static str)> {
    fomc.iter().copied().find(|(d, _)| *d >= today)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn opex_is_the_third_friday_or_the_day_before_a_holiday() {
        let x = Exchange::default();
        assert_eq!(opex(&x, 2026, 11), Some(d(2026, 11, 20)));
        assert_eq!(opex(&x, 2026, 12), Some(d(2026, 12, 18)));
        // 19 June 2026 is Juneteenth: expiry moves to Thursday the 18th.
        assert_eq!(opex(&x, 2026, 6), Some(d(2026, 6, 18)));
    }

    #[test]
    fn fomc_titles_are_recognised() {
        assert!(is_fomc_title("Fed Interest Rate Decision"));
        assert!(is_fomc_title("FOMC Statement"));
        assert!(is_fomc_title("Federal Funds Rate"));
        assert!(!is_fomc_title("Nonfarm Payrolls"));
        assert!(!is_fomc_title("FOMC Meeting Minutes"));
        assert!(!is_fomc_title("FOMC Member Waller Speaks"));
        assert!(!is_fomc_title("FOMC Economic Projections"));
    }

    #[test]
    fn fomc_days_merge_without_duplicates() {
        let days = fomc_days(&[d(2026, 12, 9), d(2027, 1, 27)], &[d(2027, 3, 17)]);
        let dec: Vec<_> = days.iter().filter(|(x, _)| *x == d(2026, 12, 9)).collect();
        assert_eq!(dec.len(), 1);
        assert_eq!(dec[0].1, "mt5 calendar");
        assert_eq!(
            next_fomc(&days, d(2026, 11, 24)),
            Some((d(2026, 12, 9), "mt5 calendar"))
        );
        assert_eq!(
            next_fomc(&days, d(2026, 12, 10)).map(|x| x.0),
            Some(d(2027, 1, 27))
        );
    }

    #[test]
    fn a_month_has_its_marks_in_date_order() {
        let x = Exchange::default();
        let fomc = fomc_days(&[], &[]);
        let marks = month_marks(&x, 2026, 11, &fomc);
        let kinds: Vec<_> = marks.iter().map(|m| (m.date.as_str(), m.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("2026-11-20", MarkKind::Opex),
                ("2026-11-26", MarkKind::Holiday),
                ("2026-11-27", MarkKind::EarlyClose),
            ]
        );
        let dec = month_marks(&x, 2026, 12, &fomc);
        assert!(
            dec.iter()
                .any(|m| m.kind == MarkKind::Fomc && m.date == "2026-12-09")
        );
    }
}

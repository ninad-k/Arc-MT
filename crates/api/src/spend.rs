//! The Claude spending meter for Assistant mode (spec-assistant-mode, "Safety cap").
//!
//! Every Claude reply in Assistant mode reports its token usage. The meter turns that into
//! dollars with the prices from `[assistant]` and adds it to this month's total, kept in a small
//! JSON file so it survives restarts. Before every request, [`SpendMeter::check`] refuses once the
//! month's total has reached the cap: nothing is sent to Anthropic after that.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Datelike, Utc};
use jarvis_llm::Usage;
use serde::{Deserialize, Serialize};

/// The share of the cap at which a notice appears.
pub const WARN_AT: f64 = 0.8;

/// US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prices {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Prices {
    /// What one reply cost. `usage.input_tokens` includes the cache reads and writes, which are
    /// billed at their own prices.
    pub fn cost(&self, usage: &Usage) -> f64 {
        let cached = u64::from(usage.cache_read_tokens) + u64::from(usage.cache_write_tokens);
        let plain = u64::from(usage.input_tokens).saturating_sub(cached);
        let per = |tokens: u64, price: f64| tokens as f64 * price / 1_000_000.0;
        per(plain, self.input)
            + per(u64::from(usage.cache_read_tokens), self.cache_read)
            + per(u64::from(usage.cache_write_tokens), self.cache_write)
            + per(u64::from(usage.output_tokens), self.output)
    }
}

/// Where this month stands.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpendStatus {
    /// `YYYY-MM` (UTC).
    pub month: String,
    pub spent_usd: f64,
    pub budget_usd: f64,
    /// At or above [`WARN_AT`] of the cap.
    pub warn: bool,
    /// At or above the cap: no request is sent until next month or a higher cap.
    pub blocked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Ledger {
    month: String,
    spent_usd: f64,
}

/// Money set aside for one request before it is sent ([`SpendMeter::reserve`]). If the request
/// never comes back (a timeout, a dropped connection), the worst case stays counted.
#[derive(Debug)]
#[must_use]
pub struct Reservation {
    amount: f64,
}

impl Reservation {
    pub fn amount(&self) -> f64 {
        self.amount
    }
}

pub struct SpendMeter {
    path: Option<PathBuf>,
    budget_usd: f64,
    prices: Prices,
    ledger: Mutex<Ledger>,
}

fn month_of(now: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", now.year(), now.month())
}

impl SpendMeter {
    /// `path`: the JSON file that keeps the month's total (`None` keeps it in memory only).
    /// A missing or unreadable file starts from zero.
    pub fn new(path: Option<PathBuf>, budget_usd: f64, prices: Prices) -> Self {
        let ledger = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<Ledger>(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            budget_usd: budget_usd.max(0.0),
            prices,
            ledger: Mutex::new(ledger),
        }
    }

    pub fn prices(&self) -> Prices {
        self.prices
    }

    fn status_of(&self, ledger: &Ledger) -> SpendStatus {
        let budget = self.budget_usd;
        SpendStatus {
            month: ledger.month.clone(),
            spent_usd: ledger.spent_usd,
            budget_usd: budget,
            warn: ledger.spent_usd >= budget * WARN_AT,
            blocked: ledger.spent_usd >= budget,
        }
    }

    /// The ledger, rolled over to `now`'s month.
    fn current(&self, now: DateTime<Utc>) -> std::sync::MutexGuard<'_, Ledger> {
        let mut ledger = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        let month = month_of(now);
        if ledger.month != month {
            *ledger = Ledger {
                month,
                spent_usd: 0.0,
            };
        }
        ledger
    }

    pub fn status(&self, now: DateTime<Utc>) -> SpendStatus {
        let ledger = self.current(now);
        self.status_of(&ledger)
    }

    /// `Err` with the status when the cap is reached: do not send the request.
    pub fn check(&self, now: DateTime<Utc>) -> Result<SpendStatus, SpendStatus> {
        let status = self.status(now);
        if status.blocked {
            Err(status)
        } else {
            Ok(status)
        }
    }

    /// The most a request can cost: its estimated input at the full input price plus its whole
    /// output budget.
    pub fn worst_case(&self, input_tokens: u64, max_output_tokens: u32) -> f64 {
        (input_tokens as f64 * self.prices.input
            + f64::from(max_output_tokens) * self.prices.output)
            / 1_000_000.0
    }

    /// Set `worst_usd` aside before sending a request. `Err` with the status when that would
    /// take the month past the cap: do not send it. Settle the reservation with
    /// [`SpendMeter::settle`] when the reply arrives.
    pub fn reserve(&self, now: DateTime<Utc>, worst_usd: f64) -> Result<Reservation, SpendStatus> {
        let worst_usd = worst_usd.max(0.0);
        let mut ledger = self.current(now);
        if ledger.spent_usd + worst_usd > self.budget_usd {
            return Err(self.status_of(&ledger));
        }
        ledger.spent_usd += worst_usd;
        self.persist(&ledger);
        Ok(Reservation { amount: worst_usd })
    }

    /// Replace a reservation with what the reply really cost (`usage`), or keep the worst case
    /// when no usage is known. Returns the new status and the cost counted.
    pub fn settle(
        &self,
        now: DateTime<Utc>,
        reservation: Reservation,
        usage: Option<&Usage>,
    ) -> (SpendStatus, f64) {
        let cost = usage.map_or(reservation.amount, |u| self.prices.cost(u));
        let mut ledger = self.current(now);
        ledger.spent_usd = (ledger.spent_usd - reservation.amount + cost).max(0.0);
        self.persist(&ledger);
        (self.status_of(&ledger), cost)
    }

    /// Give a reservation back: the request was refused before anything was billed.
    pub fn release(&self, now: DateTime<Utc>, reservation: Reservation) -> SpendStatus {
        let mut ledger = self.current(now);
        ledger.spent_usd = (ledger.spent_usd - reservation.amount).max(0.0);
        self.persist(&ledger);
        self.status_of(&ledger)
    }

    /// Add one reply's cost and save the total. Returns the new status and the reply's cost.
    pub fn record(&self, now: DateTime<Utc>, usage: &Usage) -> (SpendStatus, f64) {
        let cost = self.prices.cost(usage);
        let mut ledger = self.current(now);
        ledger.spent_usd += cost;
        self.persist(&ledger);
        (self.status_of(&ledger), cost)
    }

    fn persist(&self, ledger: &Ledger) {
        if let Some(path) = &self.path
            && let Err(e) = save(path, ledger)
        {
            tracing::warn!(path = %path.display(), "could not save the Claude spend total: {e}");
        }
    }
}

/// Write through a temporary file, so a crash never leaves half a file.
fn save(path: &Path, ledger: &Ledger) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(ledger)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const OPUS: Prices = Prices {
        input: 4.0,
        output: 20.0,
        cache_read: 0.2,
        cache_write: 5.0,
    };

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn usage(input: u32, output: u32, read: u32, write: u32) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: read,
            cache_write_tokens: write,
        }
    }

    #[test]
    fn cost_prices_each_kind_of_token() {
        // 1M plain input, 1M output: $4 + $20.
        assert!((OPUS.cost(&usage(1_000_000, 1_000_000, 0, 0)) - 24.0).abs() < 1e-9);
        // 3,000 input of which 2,000 cache reads and 500 cache writes, 400 output.
        let c = OPUS.cost(&usage(3_000, 400, 2_000, 500));
        let expected =
            500.0 * 4.0 / 1e6 + 2_000.0 * 0.2 / 1e6 + 500.0 * 5.0 / 1e6 + 400.0 * 20.0 / 1e6;
        assert!((c - expected).abs() < 1e-12, "{c} vs {expected}");
    }

    #[test]
    fn warns_at_eighty_percent_and_blocks_at_the_cap() {
        let meter = SpendMeter::new(None, 1.0, OPUS);
        let now = at(2026, 11, 11);
        assert!(meter.check(now).is_ok());
        // $0.80: 40,000 output tokens.
        let (s, cost) = meter.record(now, &usage(0, 40_000, 0, 0));
        assert!((cost - 0.8).abs() < 1e-9);
        assert!(s.warn && !s.blocked);
        assert!(meter.check(now).is_ok());
        let (s, _) = meter.record(now, &usage(0, 10_000, 0, 0));
        assert!(s.blocked);
        let err = meter.check(now).unwrap_err();
        assert!(err.blocked && (err.spent_usd - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_reservation_holds_the_worst_case_until_settled() {
        let meter = SpendMeter::new(None, 1.0, OPUS);
        let now = at(2026, 11, 11);
        // 10,000 input tokens and a 16,000-token output budget: $0.04 + $0.32.
        let worst = meter.worst_case(10_000, 16_000);
        assert!((worst - 0.36).abs() < 1e-9);
        let r = meter.reserve(now, worst).unwrap();
        assert!((meter.status(now).spent_usd - 0.36).abs() < 1e-9);
        // The reply cost less: only that stays.
        let (s, cost) = meter.settle(now, r, Some(&usage(10_000, 1_000, 0, 0)));
        assert!((cost - 0.06).abs() < 1e-9 && (s.spent_usd - 0.06).abs() < 1e-9);
        // A request that never came back stays counted at its worst case.
        let lost = meter.reserve(now, 0.3).unwrap();
        drop(lost);
        assert!((meter.status(now).spent_usd - 0.36).abs() < 1e-9);
        // A refused request is given back.
        let r = meter.reserve(now, 0.1).unwrap();
        meter.release(now, r);
        assert!((meter.status(now).spent_usd - 0.36).abs() < 1e-9);
    }

    #[test]
    fn a_request_that_could_pass_the_cap_is_not_sent() {
        let meter = SpendMeter::new(None, 1.0, OPUS);
        let now = at(2026, 11, 11);
        meter.record(now, &usage(0, 40_000, 0, 0)); // $0.80
        let err = meter.reserve(now, 0.3).unwrap_err();
        assert!(
            !err.blocked,
            "not used up, but not enough left for this request"
        );
        assert!(meter.reserve(now, 0.15).is_ok());
    }

    #[test]
    fn a_new_month_starts_from_zero() {
        let meter = SpendMeter::new(None, 1.0, OPUS);
        meter.record(at(2026, 11, 30), &usage(0, 60_000, 0, 0));
        assert!(meter.check(at(2026, 11, 30)).is_err());
        let s = meter.check(at(2026, 12, 1)).unwrap();
        assert_eq!(s.month, "2026-12");
        assert_eq!(s.spent_usd, 0.0);
    }

    #[test]
    fn the_total_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("arc-spend-{}", uuid::Uuid::new_v4()));
        let path = dir.join("spend/assistant-spend.json");
        let now = at(2026, 11, 11);
        SpendMeter::new(Some(path.clone()), 20.0, OPUS).record(now, &usage(0, 50_000, 0, 0));
        let again = SpendMeter::new(Some(path.clone()), 20.0, OPUS);
        assert!((again.status(now).spent_usd - 1.0).abs() < 1e-9);
        // A damaged file starts from zero instead of failing.
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(
            SpendMeter::new(Some(path), 20.0, OPUS)
                .status(now)
                .spent_usd,
            0.0
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

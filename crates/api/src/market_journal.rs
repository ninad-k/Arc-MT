//! The market view journal: every view is stored and later scored against the price.
//!
//! [`score`] is a pure function of the stored view, the M5 bars since it was made and the
//! clock, so every rule is tested without a database or a bridge. [`Journal`] is the SQLite
//! side: insert, read, list, statistics and the update the evaluator makes.
//!
//! All times that meet bars are broker server time (unix seconds), the clock of the bars.

use anyhow::Context;
use chrono::{SecondsFormat, Utc};
use jarvis_market::{Bar, Bias, Side};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::market::MarketViewDto;

/// Length of one M5 bar, in seconds.
pub const M5_SECS: i64 = 300;
/// The bias counts as right when the price moved at least this many H1 ATRs its way (for a
/// neutral bias: when it moved less than this in either direction).
pub const BIAS_MOVE_ATR: f64 = 0.25;

/// Where a view stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Open,
    TpHit,
    SlHit,
    Expired,
    NoSetup,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Open => "open",
            Outcome::TpHit => "tp_hit",
            Outcome::SlHit => "sl_hit",
            Outcome::Expired => "expired",
            Outcome::NoSetup => "no_setup",
        }
    }

    pub fn parse(text: &str) -> Option<Outcome> {
        [
            Outcome::Open,
            Outcome::TpHit,
            Outcome::SlHit,
            Outcome::Expired,
            Outcome::NoSetup,
        ]
        .into_iter()
        .find(|o| o.as_str() == text)
    }
}

/// The levels of a setup that scoring needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackedSetup {
    pub side: Side,
    pub stop: f64,
    pub tp: f64,
}

/// What the evaluator knows about a stored view.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracked {
    pub id: String,
    pub created_server: i64,
    pub valid_server: i64,
    pub bias: Bias,
    /// Middle of bid and ask when the view was made.
    pub price: f64,
    pub atr_h1: Option<f64>,
    pub setup: Option<TrackedSetup>,
    pub outcome: Outcome,
    /// The bias has been judged already.
    pub bias_done: bool,
}

/// The result of scoring a view once.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub outcome: Outcome,
    /// True once `valid_until` has passed and the bias was judged (or could not be).
    pub bias_done: bool,
    /// Only set in the run that judged the bias.
    pub bias_correct: Option<bool>,
    pub move_atr: Option<f64>,
}

/// Score a view against M5 `bars` (oldest first) at server time `now`.
///
/// - Only bars that opened after the view was made and closed by `valid_until` count. A bar
///   that opened earlier cannot say what came first.
/// - Buy: the target is touched when a high reaches it, the stop when a low reaches it; sell is
///   the mirror. The first bar that touches either decides. A bar that touches both counts as
///   the stop (the order inside a bar is unknown, so the worse case is assumed).
/// - No touch and `valid_until` passed: `expired`, or `no_setup` for a view without a setup.
/// - At `valid_until` the close of the last counted bar is compared with the price at the time
///   of the view: a bullish or bearish bias is right when the price moved at least a quarter
///   of the H1 ATR its way, a neutral one when it moved less than that either way.
/// - If the bars do not reach back to the creation of the view (the server was off for days),
///   nothing can be said about the order of events: a setup ends `expired` and the bias stays
///   unscored.
pub fn score(t: &Tracked, bars: &[Bar], now: i64) -> Verdict {
    let past = now >= t.valid_server;
    let covered = bars
        .first()
        .is_some_and(|b| b.time <= t.created_server + M5_SECS);

    let mut outcome = t.outcome;
    if outcome == Outcome::Open {
        match &t.setup {
            Some(setup) => {
                if covered
                    && let Some(hit) = first_touch(setup, bars, t.created_server, t.valid_server)
                {
                    outcome = hit;
                }
                if outcome == Outcome::Open && past {
                    outcome = Outcome::Expired;
                }
            }
            None if past => outcome = Outcome::NoSetup,
            None => {}
        }
    }

    let mut verdict = Verdict {
        outcome,
        bias_done: t.bias_done,
        bias_correct: None,
        move_atr: None,
    };
    if past && !t.bias_done {
        verdict.bias_done = true;
        if covered
            && let Some(close) = close_at_expiry(bars, t.created_server, t.valid_server)
            && let Some(atr) = t.atr_h1.filter(|a| a.is_finite() && *a > 0.0)
        {
            let moved = (close - t.price) / atr;
            let (correct, signed) = match t.bias {
                Bias::Bullish => (moved >= BIAS_MOVE_ATR, moved),
                Bias::Bearish => (-moved >= BIAS_MOVE_ATR, -moved),
                Bias::Neutral => (moved.abs() < BIAS_MOVE_ATR, moved),
            };
            verdict.bias_correct = Some(correct);
            verdict.move_atr = Some((signed * 1000.0).round() / 1000.0);
        }
    }
    verdict
}

/// Bars that count for a view: opened at or after its creation, closed by its expiry.
fn counted(bars: &[Bar], created: i64, valid: i64) -> impl Iterator<Item = &Bar> {
    bars.iter()
        .filter(move |b| b.time >= created && b.time + M5_SECS <= valid)
}

fn first_touch(setup: &TrackedSetup, bars: &[Bar], created: i64, valid: i64) -> Option<Outcome> {
    for bar in counted(bars, created, valid) {
        let (tp_touched, sl_touched) = match setup.side {
            Side::Buy => (bar.high >= setup.tp, bar.low <= setup.stop),
            Side::Sell => (bar.low <= setup.tp, bar.high >= setup.stop),
        };
        if sl_touched {
            return Some(Outcome::SlHit);
        }
        if tp_touched {
            return Some(Outcome::TpHit);
        }
    }
    None
}

fn close_at_expiry(bars: &[Bar], created: i64, valid: i64) -> Option<f64> {
    counted(bars, created, valid).last().map(|b| b.close)
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// A view as it is written to the journal.
#[derive(Debug, Clone)]
pub struct JournalEntry<'a> {
    pub view: &'a MarketViewDto,
    pub created_server: i64,
    pub valid_server: i64,
}

/// A stored view with its server-time bounds.
#[derive(Debug, Clone)]
pub struct StoredView {
    pub view: MarketViewDto,
    pub created_server: i64,
    pub valid_server: i64,
}

/// An open view the evaluator should look at, with the symbol to fetch bars for.
#[derive(Debug, Clone)]
pub struct Pending {
    pub tracked: Tracked,
    pub broker_symbol: String,
}

/// One row of `GET /api/market/journal`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JournalRow {
    pub id: String,
    pub created_at: String,
    pub symbol: String,
    pub broker_symbol: String,
    pub bias: String,
    pub confidence: u32,
    pub score: f64,
    pub stand_aside: bool,
    pub side: Option<String>,
    /// Middle of the entry zone.
    pub entry: Option<f64>,
    pub stop: Option<f64>,
    /// The first target.
    pub tp: Option<f64>,
    /// Middle of bid and ask when the view was made.
    pub price: f64,
    pub valid_until: String,
    pub outcome: String,
    pub evaluated_at: Option<String>,
    pub bias_correct: Option<bool>,
    /// The move at `valid_until` in H1 ATRs, positive when the price went the way the bias
    /// said (neutral: positive when it went up).
    pub move_atr: Option<f64>,
}

/// Totals over the journal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JournalStats {
    pub total: u64,
    /// Views that had a setup.
    pub setups: u64,
    pub open: u64,
    pub tp_hit: u64,
    pub sl_hit: u64,
    pub expired: u64,
    pub no_setup: u64,
    pub bias_correct: u64,
    pub bias_wrong: u64,
    /// Target hits among closed setups (target, stop or expired); `null` when none closed.
    pub tp_rate: Option<f64>,
    /// Right biases among scored ones; `null` when none was scored.
    pub bias_accuracy: Option<f64>,
}

impl JournalStats {
    fn from_counts(c: &Counts) -> Self {
        let closed_setups = c.tp_hit + c.sl_hit + c.expired;
        let scored = c.bias_correct + c.bias_wrong;
        Self {
            total: c.total,
            setups: c.setups,
            open: c.open,
            tp_hit: c.tp_hit,
            sl_hit: c.sl_hit,
            expired: c.expired,
            no_setup: c.no_setup,
            bias_correct: c.bias_correct,
            bias_wrong: c.bias_wrong,
            tp_rate: rate(c.tp_hit, closed_setups),
            bias_accuracy: rate(c.bias_correct, scored),
        }
    }
}

fn rate(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| ((part as f64 / whole as f64) * 1000.0).round() / 1000.0)
}

#[derive(Default)]
struct Counts {
    total: u64,
    setups: u64,
    open: u64,
    tp_hit: u64,
    sl_hit: u64,
    expired: u64,
    no_setup: u64,
    bias_correct: u64,
    bias_wrong: u64,
}

/// The `market_views` table.
#[derive(Clone, Debug)]
pub struct Journal {
    pool: SqlitePool,
}

impl Journal {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, entry: &JournalEntry<'_>) -> anyhow::Result<()> {
        let v = entry.view;
        let setup = v.setup.as_ref();
        let atr_h1 = v
            .timeframes
            .iter()
            .find(|t| t.timeframe == jarvis_market::Timeframe::H1)
            .map(|t| t.atr)
            .filter(|a| a.is_finite() && *a > 0.0);
        sqlx::query(
            "INSERT INTO market_views (id, created_at, created_server, symbol, broker_symbol, \
             bias, confidence, score, stand_aside, side, entry, stop, tp, price, atr_h1, \
             valid_until, valid_server, view_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        )
        .bind(&v.id)
        .bind(&v.generated_at)
        .bind(entry.created_server)
        .bind(&v.symbol)
        .bind(&v.broker_symbol)
        .bind(bias_text(v.bias))
        .bind(i64::from(v.confidence))
        .bind(v.score)
        .bind(v.stand_aside)
        .bind(setup.map(|s| side_text(s.side)))
        .bind(setup.map(|s| (s.entry_low + s.entry_high) / 2.0))
        .bind(setup.map(|s| s.stop_loss))
        .bind(setup.map(|s| s.take_profit))
        .bind((v.price.bid + v.price.ask) / 2.0)
        .bind(atr_h1)
        .bind(&v.valid_until)
        .bind(entry.valid_server)
        .bind(serde_json::to_string(v)?)
        .execute(&self.pool)
        .await
        .context("storing a market view")?;
        Ok(())
    }

    /// A stored view by id.
    pub async fn get(&self, id: &str) -> anyhow::Result<Option<StoredView>> {
        let row = sqlx::query(
            "SELECT view_json, created_server, valid_server FROM market_views WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("reading a market view")?;
        row.map(|r| {
            let text: String = r.try_get("view_json")?;
            Ok(StoredView {
                view: serde_json::from_str(&text).context("a stored market view is not valid")?,
                created_server: r.try_get("created_server")?,
                valid_server: r.try_get("valid_server")?,
            })
        })
        .transpose()
    }

    /// Views still to be scored: open ones, and finished ones whose bias is not judged yet.
    pub async fn pending(&self) -> anyhow::Result<Vec<Pending>> {
        let rows = sqlx::query(
            "SELECT id, broker_symbol, created_server, valid_server, bias, price, atr_h1, \
             side, stop, tp, outcome, bias_done FROM market_views \
             WHERE outcome = 'open' OR bias_done = 0 ORDER BY created_server",
        )
        .fetch_all(&self.pool)
        .await
        .context("reading the open market views")?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let side: Option<String> = r.try_get("side")?;
            let stop: Option<f64> = r.try_get("stop")?;
            let tp: Option<f64> = r.try_get("tp")?;
            let setup = match (side.as_deref().and_then(parse_side), stop, tp) {
                (Some(side), Some(stop), Some(tp)) => Some(TrackedSetup { side, stop, tp }),
                _ => None,
            };
            let bias: String = r.try_get("bias")?;
            let outcome: String = r.try_get("outcome")?;
            let (Some(bias), Some(outcome)) = (parse_bias(&bias), Outcome::parse(&outcome)) else {
                tracing::error!("a stored market view has an unknown bias or outcome; skipped");
                continue;
            };
            out.push(Pending {
                broker_symbol: r.try_get("broker_symbol")?,
                tracked: Tracked {
                    id: r.try_get("id")?,
                    created_server: r.try_get("created_server")?,
                    valid_server: r.try_get("valid_server")?,
                    bias,
                    price: r.try_get("price")?,
                    atr_h1: r.try_get("atr_h1")?,
                    setup,
                    outcome,
                    bias_done: r.try_get::<i64, _>("bias_done")? != 0,
                },
            });
        }
        Ok(out)
    }

    /// Write what [`score`] decided. Bias fields are written once, in the run that judged them.
    pub async fn apply(&self, before: &Tracked, verdict: &Verdict) -> anyhow::Result<()> {
        let changed = verdict.outcome != before.outcome || verdict.bias_done != before.bias_done;
        if !changed {
            return Ok(());
        }
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        sqlx::query(
            "UPDATE market_views SET outcome = ?2, evaluated_at = ?3, bias_done = ?4, \
             bias_correct = COALESCE(?5, bias_correct), move_atr = COALESCE(?6, move_atr) \
             WHERE id = ?1",
        )
        .bind(&before.id)
        .bind(verdict.outcome.as_str())
        .bind(now)
        .bind(verdict.bias_done)
        .bind(verdict.bias_correct)
        .bind(verdict.move_atr)
        .execute(&self.pool)
        .await
        .context("updating a market view")?;
        Ok(())
    }

    /// The newest views first, optionally for one canonical symbol.
    pub async fn list(&self, symbol: Option<&str>, limit: u32) -> anyhow::Result<Vec<JournalRow>> {
        let rows = sqlx::query(
            "SELECT id, created_at, symbol, broker_symbol, bias, confidence, score, stand_aside, \
             side, entry, stop, tp, price, valid_until, outcome, evaluated_at, bias_correct, \
             move_atr FROM market_views WHERE (?1 IS NULL OR symbol = ?1) \
             ORDER BY created_server DESC, rowid DESC LIMIT ?2",
        )
        .bind(symbol)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("listing the market views")?;
        rows.into_iter()
            .map(|r| {
                Ok(JournalRow {
                    id: r.try_get("id")?,
                    created_at: r.try_get("created_at")?,
                    symbol: r.try_get("symbol")?,
                    broker_symbol: r.try_get("broker_symbol")?,
                    bias: r.try_get("bias")?,
                    confidence: u32::try_from(r.try_get::<i64, _>("confidence")?).unwrap_or(0),
                    score: r.try_get("score")?,
                    stand_aside: r.try_get::<i64, _>("stand_aside")? != 0,
                    side: r.try_get("side")?,
                    entry: r.try_get("entry")?,
                    stop: r.try_get("stop")?,
                    tp: r.try_get("tp")?,
                    price: r.try_get("price")?,
                    valid_until: r.try_get("valid_until")?,
                    outcome: r.try_get("outcome")?,
                    evaluated_at: r.try_get("evaluated_at")?,
                    bias_correct: r.try_get::<Option<i64>, _>("bias_correct")?.map(|v| v != 0),
                    move_atr: r.try_get("move_atr")?,
                })
            })
            .collect()
    }

    /// Totals over every stored view (or those of one canonical symbol).
    pub async fn stats(&self, symbol: Option<&str>) -> anyhow::Result<JournalStats> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS total, \
             COALESCE(SUM(side IS NOT NULL), 0) AS setups, \
             COALESCE(SUM(outcome = 'open'), 0) AS open, \
             COALESCE(SUM(outcome = 'tp_hit'), 0) AS tp_hit, \
             COALESCE(SUM(outcome = 'sl_hit'), 0) AS sl_hit, \
             COALESCE(SUM(outcome = 'expired'), 0) AS expired, \
             COALESCE(SUM(outcome = 'no_setup'), 0) AS no_setup, \
             COALESCE(SUM(bias_correct = 1), 0) AS bias_correct, \
             COALESCE(SUM(bias_correct = 0), 0) AS bias_wrong \
             FROM market_views WHERE (?1 IS NULL OR symbol = ?1)",
        )
        .bind(symbol)
        .fetch_one(&self.pool)
        .await
        .context("counting the market views")?;
        let count = |name: &str| -> anyhow::Result<u64> {
            Ok(u64::try_from(row.try_get::<i64, _>(name)?).unwrap_or(0))
        };
        Ok(JournalStats::from_counts(&Counts {
            total: count("total")?,
            setups: count("setups")?,
            open: count("open")?,
            tp_hit: count("tp_hit")?,
            sl_hit: count("sl_hit")?,
            expired: count("expired")?,
            no_setup: count("no_setup")?,
            bias_correct: count("bias_correct")?,
            bias_wrong: count("bias_wrong")?,
        }))
    }
}

fn bias_text(b: Bias) -> &'static str {
    match b {
        Bias::Bullish => "bullish",
        Bias::Bearish => "bearish",
        Bias::Neutral => "neutral",
    }
}

fn parse_bias(text: &str) -> Option<Bias> {
    match text {
        "bullish" => Some(Bias::Bullish),
        "bearish" => Some(Bias::Bearish),
        "neutral" => Some(Bias::Neutral),
        _ => None,
    }
}

fn side_text(s: Side) -> &'static str {
    match s {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn parse_side(text: &str) -> Option<Side> {
    match text {
        "buy" => Some(Side::Buy),
        "sell" => Some(Side::Sell),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREATED: i64 = 1_000_000;
    const VALID: i64 = CREATED + 240 * 60;

    fn bar(k: i64, low: f64, high: f64, close: f64) -> Bar {
        Bar {
            time: CREATED + k * M5_SECS,
            open: close,
            high,
            low,
            close,
            tick_volume: 1,
            spread: 0,
        }
    }

    /// Quiet bars around 100 from the creation to the expiry.
    fn quiet(n: i64) -> Vec<Bar> {
        (0..n).map(|k| bar(k, 99.5, 100.5, 100.0)).collect()
    }

    fn buy() -> Tracked {
        Tracked {
            id: "v".into(),
            created_server: CREATED,
            valid_server: VALID,
            bias: Bias::Bullish,
            price: 100.0,
            atr_h1: Some(4.0),
            setup: Some(TrackedSetup {
                side: Side::Buy,
                stop: 96.0,
                tp: 106.0,
            }),
            outcome: Outcome::Open,
            bias_done: false,
        }
    }

    fn sell() -> Tracked {
        Tracked {
            bias: Bias::Bearish,
            setup: Some(TrackedSetup {
                side: Side::Sell,
                stop: 104.0,
                tp: 94.0,
            }),
            ..buy()
        }
    }

    fn neutral() -> Tracked {
        Tracked {
            bias: Bias::Neutral,
            setup: None,
            ..buy()
        }
    }

    /// Bars up to (not including) the one that opens at `k`.
    fn then(k: i64, mut bars: Vec<Bar>, extra: Bar) -> Vec<Bar> {
        bars.truncate(usize::try_from(k).unwrap());
        bars.push(extra);
        bars
    }

    #[test]
    fn a_buy_reaching_its_target_first_is_a_tp_hit() {
        let bars = then(10, quiet(40), bar(10, 99.0, 106.5, 106.0));
        let v = score(&buy(), &bars, CREATED + 11 * M5_SECS);
        assert_eq!(v.outcome, Outcome::TpHit);
        assert!(!v.bias_done, "the bias waits for the expiry");
    }

    #[test]
    fn a_buy_reaching_its_stop_first_is_an_sl_hit() {
        let bars = then(10, quiet(40), bar(10, 95.5, 100.5, 96.0));
        let v = score(&buy(), &bars, CREATED + 11 * M5_SECS);
        assert_eq!(v.outcome, Outcome::SlHit);
    }

    #[test]
    fn the_order_of_bars_decides_when_both_levels_are_reached_later() {
        let mut bars = quiet(40);
        bars[5] = bar(5, 99.0, 106.5, 106.0);
        bars[20] = bar(20, 95.0, 100.0, 96.0);
        assert_eq!(
            score(&buy(), &bars, CREATED + 40 * M5_SECS).outcome,
            Outcome::TpHit
        );
        let mut bars = quiet(40);
        bars[5] = bar(5, 95.0, 100.0, 96.0);
        bars[20] = bar(20, 99.0, 106.5, 106.0);
        assert_eq!(
            score(&buy(), &bars, CREATED + 40 * M5_SECS).outcome,
            Outcome::SlHit
        );
    }

    #[test]
    fn both_levels_in_one_bar_count_as_the_stop() {
        let bars = then(3, quiet(40), bar(3, 95.0, 107.0, 100.0));
        for t in [buy(), sell()] {
            let wide = then(3, quiet(40), bar(3, 93.0, 107.0, 100.0));
            let v = score(&t, &wide, CREATED + 4 * M5_SECS);
            assert_eq!(v.outcome, Outcome::SlHit, "{:?}", t.setup);
        }
        // The buy's stop and target both sit inside this bar.
        let v = score(&buy(), &bars, CREATED + 4 * M5_SECS);
        assert_eq!(v.outcome, Outcome::SlHit);
    }

    #[test]
    fn a_sell_mirrors_the_buy() {
        let target = then(10, quiet(40), bar(10, 93.5, 100.5, 94.0));
        assert_eq!(
            score(&sell(), &target, CREATED + 11 * M5_SECS).outcome,
            Outcome::TpHit
        );
        let stopped = then(10, quiet(40), bar(10, 99.0, 104.5, 104.0));
        assert_eq!(
            score(&sell(), &stopped, CREATED + 11 * M5_SECS).outcome,
            Outcome::SlHit
        );
        // A rise that stays below the stop does not count against a sell.
        assert_eq!(
            score(&sell(), &quiet(40), CREATED + 11 * M5_SECS).outcome,
            Outcome::Open
        );
    }

    #[test]
    fn a_touch_exactly_at_a_level_counts() {
        let bars = then(2, quiet(40), bar(2, 99.0, 106.0, 105.0));
        assert_eq!(
            score(&buy(), &bars, CREATED + 3 * M5_SECS).outcome,
            Outcome::TpHit
        );
        let bars = then(2, quiet(40), bar(2, 96.0, 101.0, 97.0));
        assert_eq!(
            score(&buy(), &bars, CREATED + 3 * M5_SECS).outcome,
            Outcome::SlHit
        );
    }

    #[test]
    fn untouched_stays_open_and_then_expires() {
        let bars = quiet(30);
        let v = score(&buy(), &bars, CREATED + 30 * M5_SECS);
        assert_eq!(v.outcome, Outcome::Open);
        assert!(!v.bias_done);

        // The whole horizon passed.
        let all = quiet(48);
        let v = score(&buy(), &all, VALID + 10);
        assert_eq!(v.outcome, Outcome::Expired);
        assert!(v.bias_done);
    }

    #[test]
    fn a_view_without_a_setup_is_no_setup_at_the_expiry_only() {
        let bars = quiet(48);
        let v = score(&neutral(), &bars, CREATED + 20 * M5_SECS);
        assert_eq!(v.outcome, Outcome::Open);
        let v = score(&neutral(), &bars, VALID);
        assert_eq!(v.outcome, Outcome::NoSetup);
        assert!(v.bias_done);
    }

    #[test]
    fn bars_that_opened_before_the_view_or_close_after_expiry_do_not_count() {
        let mut bars = quiet(48);
        // A spike in the bar that opened before the view, and one that straddles the expiry.
        bars.insert(
            0,
            Bar {
                time: CREATED - M5_SECS,
                ..bar(0, 90.0, 120.0, 100.0)
            },
        );
        bars.push(Bar {
            time: VALID - 100,
            ..bar(0, 90.0, 120.0, 100.0)
        });
        bars.push(Bar {
            time: VALID + 200,
            ..bar(0, 90.0, 120.0, 100.0)
        });
        let v = score(&buy(), &bars, VALID + 600);
        assert_eq!(v.outcome, Outcome::Expired);
    }

    #[test]
    fn a_resolved_view_keeps_its_outcome_and_still_gets_a_bias_score() {
        let mut t = buy();
        t.outcome = Outcome::TpHit;
        let mut bars = quiet(48);
        bars[47] = bar(47, 99.0, 105.0, 102.0);
        let early = score(&t, &bars, CREATED + 30 * M5_SECS);
        assert_eq!(early.outcome, Outcome::TpHit);
        assert!(!early.bias_done);
        let late = score(&t, &bars, VALID + 5);
        assert_eq!(late.outcome, Outcome::TpHit);
        assert!(late.bias_done);
        assert_eq!(late.bias_correct, Some(true));
        assert_eq!(late.move_atr, Some(0.5));
    }

    fn at_expiry(mut t: Tracked, close: f64) -> Verdict {
        t.setup = None;
        let mut bars = quiet(48);
        bars[47] = bar(47, 99.0, 101.0, close);
        score(&t, &bars, VALID)
    }

    #[test]
    fn a_bullish_bias_needs_a_quarter_atr_up() {
        // ATR 4: a quarter is 1.0.
        let up = at_expiry(buy(), 101.0);
        assert_eq!(up.bias_correct, Some(true));
        assert_eq!(up.move_atr, Some(0.25));
        let small = at_expiry(buy(), 100.9);
        assert_eq!(small.bias_correct, Some(false));
        let down = at_expiry(buy(), 98.0);
        assert_eq!(down.bias_correct, Some(false));
        assert_eq!(down.move_atr, Some(-0.5));
    }

    #[test]
    fn a_bearish_bias_needs_a_quarter_atr_down_and_move_is_signed_its_way() {
        let down = at_expiry(sell(), 98.0);
        assert_eq!(down.bias_correct, Some(true));
        assert_eq!(down.move_atr, Some(0.5));
        let up = at_expiry(sell(), 102.0);
        assert_eq!(up.bias_correct, Some(false));
        assert_eq!(up.move_atr, Some(-0.5));
    }

    #[test]
    fn a_neutral_bias_is_right_when_the_price_stayed_put() {
        let flat = at_expiry(neutral(), 100.4);
        assert_eq!(flat.bias_correct, Some(true));
        let up = at_expiry(neutral(), 101.5);
        assert_eq!(up.bias_correct, Some(false));
        let down = at_expiry(neutral(), 98.5);
        assert_eq!(down.bias_correct, Some(false));
        assert_eq!(down.move_atr, Some(-0.375));
    }

    #[test]
    fn the_bias_is_not_judged_before_the_expiry() {
        let v = score(&neutral(), &quiet(30), CREATED + 30 * M5_SECS);
        assert!(!v.bias_done);
        assert_eq!(v.bias_correct, None);
    }

    #[test]
    fn the_bias_is_judged_only_once() {
        let mut t = neutral();
        t.bias_done = true;
        t.outcome = Outcome::NoSetup;
        let v = score(&t, &quiet(48), VALID + 1000);
        assert!(v.bias_done);
        assert_eq!(v.bias_correct, None);
        assert_eq!(v.outcome, Outcome::NoSetup);
    }

    #[test]
    fn without_an_atr_the_bias_is_closed_but_not_scored() {
        let mut t = neutral();
        t.atr_h1 = None;
        let v = score(&t, &quiet(48), VALID);
        assert!(v.bias_done);
        assert_eq!(v.bias_correct, None);
        assert_eq!(v.move_atr, None);
    }

    #[test]
    fn bars_that_do_not_reach_back_leave_a_setup_expired_and_the_bias_unscored() {
        // Only the last bars are available: the first one opened an hour after the view.
        let bars: Vec<Bar> = (12..48).map(|k| bar(k, 95.0, 107.0, 100.0)).collect();
        let v = score(&buy(), &bars, VALID + 60);
        assert_eq!(v.outcome, Outcome::Expired);
        assert!(v.bias_done);
        assert_eq!(v.bias_correct, None);
        // Not past the expiry yet: nothing is decided from a partial history.
        let v = score(&buy(), &bars, CREATED + 20 * M5_SECS);
        assert_eq!(v.outcome, Outcome::Open);
    }

    #[test]
    fn no_bars_at_all_decides_nothing_before_the_expiry() {
        let v = score(&buy(), &[], CREATED + 10);
        assert_eq!(v.outcome, Outcome::Open);
        assert!(!v.bias_done);
    }

    #[test]
    fn rates_are_none_without_a_denominator() {
        let none = JournalStats::from_counts(&Counts::default());
        assert_eq!(none.tp_rate, None);
        assert_eq!(none.bias_accuracy, None);
        let some = JournalStats::from_counts(&Counts {
            total: 9,
            setups: 6,
            open: 1,
            tp_hit: 2,
            sl_hit: 1,
            expired: 1,
            no_setup: 3,
            bias_correct: 2,
            bias_wrong: 1,
        });
        // Four closed setups, two target hits.
        assert_eq!(some.tp_rate, Some(0.5));
        assert_eq!(some.bias_accuracy, Some(0.667));
    }

    #[test]
    fn outcome_names_round_trip() {
        for o in [
            Outcome::Open,
            Outcome::TpHit,
            Outcome::SlHit,
            Outcome::Expired,
            Outcome::NoSetup,
        ] {
            assert_eq!(Outcome::parse(o.as_str()), Some(o));
        }
        assert_eq!(Outcome::parse("won"), None);
    }
}

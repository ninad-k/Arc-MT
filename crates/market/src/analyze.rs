//! `analyze`: bars, calendar and news in, a [`MarketView`] out.

use std::cmp::Ordering;

use crate::config::ViewConfig;
use crate::indicators;
use crate::types::{Bar, EventInput, Importance, Timeframe, ViewInput};
use crate::view::{
    Bias, DISCLAIMER, Factor, Levels, MarketView, NewsView, Price, Setup, Side, TimeframeSummary,
    Trend, ViewEvent,
};

// Indicator periods are part of the model (the detail texts and the docs say "EMA20"),
// so they are constants rather than configuration.
const EMA_FAST: usize = 20;
const EMA_SLOW: usize = 50;
const RSI_PERIOD: usize = 14;
const ATR_PERIOD: usize = 14;
const MACD_FAST: usize = 12;
const MACD_SLOW: usize = 26;
const MACD_SIGNAL: usize = 9;
const MOMENTUM_LOOKBACK: usize = 3;
const SECONDS_PER_DAY: i64 = 86_400;
const MAX_DIGITS: u32 = 10;

/// Which factor a computed entry is, for the summary wording.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    H4Trend,
    H1Trend,
    Other,
}

struct Computed {
    factor: Factor,
    kind: Kind,
    /// Short clause used in the summary, e.g. "price is above VWAP".
    phrase: String,
}

struct TrendCalc {
    trend: Trend,
    ema_fast: f64,
    ema_slow: f64,
    close: f64,
}

/// Computes the market view. Pure and deterministic: the same input and config always
/// give the same output. Never panics, whatever the input.
pub fn analyze(input: &ViewInput, cfg: &ViewConfig) -> MarketView {
    let digits = input.digits.min(MAX_DIGITS);
    let dp = digits as usize;
    let now = input.now;
    let mut notes: Vec<String> = Vec::new();

    let frame = |tf: Timeframe| closed_bars(input.frames.get(&tf).map(Vec::as_slice), tf, now);
    let (h4, h1, m15, m5, d1) = (
        frame(Timeframe::H4),
        frame(Timeframe::H1),
        frame(Timeframe::M15),
        frame(Timeframe::M5),
        frame(Timeframe::D1),
    );

    let price_ok =
        input.bid.is_finite() && input.ask.is_finite() && input.bid > 0.0 && input.ask >= input.bid;
    let mid = price_ok.then(|| (input.bid + input.ask) / 2.0);
    if !price_ok {
        notes.push(
            "No valid bid/ask price, so price-based factors and the setup are skipped.".into(),
        );
    }

    let mut factors: Vec<Computed> = Vec::new();
    let mut timeframes: Vec<TimeframeSummary> = Vec::new();

    // Trend per frame (H4 and H1 also score; M15 only gets a table row).
    for (tf, weight, kind) in [
        (Timeframe::H4, Some(cfg.weight_h4_trend), Kind::H4Trend),
        (Timeframe::H1, Some(cfg.weight_h1_trend), Kind::H1Trend),
        (Timeframe::M15, None, Kind::Other),
    ] {
        let bars = match tf {
            Timeframe::H4 => &h4,
            Timeframe::H1 => &h1,
            _ => &m15,
        };
        let Some(calc) = trend_calc(bars) else {
            let msg = match weight {
                Some(_) => format!(
                    "{tf} trend factor omitted: needs {EMA_SLOW} closed {tf} bars, have {}.",
                    bars.len()
                ),
                None => format!(
                    "{tf} trend row omitted: needs {EMA_SLOW} closed {tf} bars, have {}.",
                    bars.len()
                ),
            };
            notes.push(msg);
            continue;
        };
        if let Some(row) = summary_row(tf, bars, &calc, digits) {
            timeframes.push(row);
        }
        if let Some(weight) = weight {
            factors.push(trend_factor(tf, weight, &calc, kind));
        }
    }

    // M15 momentum and RSI.
    let m15_closes = closes(&m15);
    match momentum_factor(&m15_closes, cfg, dp + 1) {
        Some(c) => factors.push(c),
        None => notes.push(format!(
            "M15 momentum factor omitted: needs {} closed M15 bars, have {}.",
            MACD_SLOW + MACD_SIGNAL - 1,
            m15.len()
        )),
    }
    match rsi_factor(&m15_closes, cfg) {
        Some(c) => factors.push(c),
        None => notes.push(format!(
            "M15 RSI factor omitted: needs {} closed M15 bars, have {}.",
            RSI_PERIOD + 1,
            m15.len()
        )),
    }

    // VWAP of the current server day, from M5 bars.
    let day_start = now.div_euclid(SECONDS_PER_DAY) * SECONDS_PER_DAY;
    let today: Vec<Bar> = m5.iter().copied().filter(|b| b.time >= day_start).collect();
    let vwap = indicators::vwap(&today);
    match (vwap, mid) {
        (Some(v), Some(mid)) => factors.push(level_factor(
            "VWAP",
            cfg.weight_vwap,
            mid,
            v,
            "session VWAP",
            "price",
            "VWAP",
            dp,
        )),
        (None, _) => {
            notes.push("VWAP factor omitted: no closed M5 bars for the current server day.".into())
        }
        _ => {}
    }

    // Classic pivot from the previous completed day.
    let pivots = d1.last().map(indicators::pivots);
    match (&pivots, mid) {
        (Some(p), Some(mid)) => factors.push(level_factor(
            "Daily pivot",
            cfg.weight_pivot,
            mid,
            p.pivot,
            "daily pivot",
            "price",
            "the daily pivot",
            dp,
        )),
        (None, _) => notes.push("Daily pivot factor omitted: no completed D1 bar.".into()),
        _ => {}
    }

    // News.
    if let Some(n) = input.news {
        if n.score.is_finite() {
            factors.push(news_factor(n.score, n.count, cfg));
        } else {
            notes.push("News factor omitted: the score is not a finite number.".into());
        }
    }

    // ATR for sizing and the spread check.
    let atr_h1 = last_value(&indicators::atr(&h1, ATR_PERIOD));
    let atr_m15 = last_value(&indicators::atr(&m15, ATR_PERIOD));

    // Levels. Swing points use three H1 bars on each side: a seven hour window is wide
    // enough for a turn to matter within the day and still confirms within three hours.
    let ref_price = mid.unwrap_or_else(|| h1.last().map_or(0.0, |b| b.close));
    let swing_tol = atr_h1.map_or(0.0, |a| cfg.swing_merge_atr_h1 * a);
    let (supports, resistances) =
        indicators::swing_levels_around(&h1, cfg.swing_left, cfg.swing_right, swing_tol, ref_price);
    let px = |v: f64| round_to(v, digits + 1);
    let levels = Levels {
        pivot: pivots.map(|p| px(p.pivot)),
        r1: pivots.map(|p| px(p.r1)),
        r2: pivots.map(|p| px(p.r2)),
        s1: pivots.map(|p| px(p.s1)),
        s2: pivots.map(|p| px(p.s2)),
        prev_day_high: d1.last().map(|b| px(b.high)),
        prev_day_low: d1.last().map(|b| px(b.low)),
        vwap: vwap.map(px),
        support: supports
            .iter()
            .take(cfg.levels_per_side)
            .map(|v| px(*v))
            .collect(),
        resistance: resistances
            .iter()
            .take(cfg.levels_per_side)
            .map(|v| px(*v))
            .collect(),
    };

    // Events.
    let events = view_events(input, cfg);

    // Stand aside.
    let mut stand_reasons: Vec<String> = Vec::new();
    if let Some(e) = blackout_event(input, cfg) {
        stand_reasons.push(event_reason(e, now));
    }
    if price_ok {
        match atr_m15 {
            Some(m) => {
                let spread = input.ask - input.bid;
                if spread > cfg.max_spread_atr_fraction * m {
                    stand_reasons.push(spread_reason(spread, input.point, m, cfg, dp));
                }
            }
            None => notes.push(format!(
                "Spread check skipped: M15 ATR needs {} closed M15 bars, have {}.",
                ATR_PERIOD + 1,
                m15.len()
            )),
        }
    }
    let stand_aside = !stand_reasons.is_empty();
    let stand_aside_reason = stand_aside.then(|| stand_reasons.join("; "));

    // Score, bias, confidence.
    let technical = factors.iter().filter(|c| !is_news(c)).count();
    let insufficient = !price_ok || technical < cfg.min_factors;

    let (score, bias, confidence, factors_out) = if insufficient {
        if price_ok {
            notes.push(format!(
                "Not enough price history for a view: {technical} of the technical factors could be computed, {} are needed.",
                cfg.min_factors
            ));
        }
        (0.0, Bias::Neutral, 0, Vec::new())
    } else {
        let raw: f64 = factors.iter().map(|c| c.factor.points).sum();
        let score = round_to(raw.clamp(-100.0, 100.0), 2);
        let bias = if score >= cfg.bias_threshold {
            Bias::Bullish
        } else if score <= -cfg.bias_threshold {
            Bias::Bearish
        } else {
            Bias::Neutral
        };
        let mut conf = score.abs();
        if trends_disagree(&factors) {
            conf = conf.min(cfg.confidence_cap_on_disagreement);
        }
        let confidence = conf.round().clamp(0.0, 100.0) as u32;
        (
            score,
            bias,
            confidence,
            factors.iter().map(|c| c.factor.clone()).collect(),
        )
    };

    // Setup.
    let mut setup = None;
    if !insufficient && bias != Bias::Neutral && !stand_aside {
        match (atr_h1, atr_m15, price_ok) {
            (Some(a), Some(m), true) => {
                let built = build_setup(bias, input, cfg, a, m, &supports, &resistances, digits);
                if built.is_none() {
                    notes.push("No setup: the sized levels are not in a valid order (the ATR is zero or too small for the price precision, or the multipliers are inconsistent).".into());
                }
                setup = built;
            }
            _ => notes.push(format!(
                "No setup: sizing needs the ATR on H1 and M15 ({} closed bars each).",
                ATR_PERIOD + 1
            )),
        }
    }

    let news = match input.news {
        Some(n) if n.score.is_finite() => NewsView {
            configured: true,
            score: round_to(n.score.clamp(-1.0, 1.0), 3),
            label: news_label(n.score.clamp(-1.0, 1.0)).to_string(),
            count: n.count,
        },
        _ => NewsView {
            configured: input.news.is_some(),
            score: 0.0,
            label: if input.news.is_some() {
                "unavailable"
            } else {
                "not configured"
            }
            .to_string(),
            count: 0,
        },
    };

    let summary = build_summary(&SummaryParts {
        insufficient,
        bias,
        confidence,
        computed: &factors,
        setup: setup.as_ref(),
        stand_reason: stand_aside_reason.as_deref(),
        dp,
    });

    MarketView {
        symbol: input.symbol.clone(),
        valid_until: now.saturating_add(cfg.horizon_minutes.saturating_mul(60)),
        price: Price {
            bid: sane(input.bid, digits),
            ask: sane(input.ask, digits),
        },
        bias,
        confidence,
        score,
        stand_aside,
        stand_aside_reason,
        setup,
        timeframes,
        levels,
        factors: factors_out,
        events,
        news,
        summary,
        disclaimer: DISCLAIMER.to_string(),
        notes,
    }
}

fn is_news(c: &Computed) -> bool {
    c.factor.name == "News"
}

fn sane(v: f64, digits: u32) -> f64 {
    if v.is_finite() {
        round_to(v, digits)
    } else {
        0.0
    }
}

fn round_to(x: f64, decimals: u32) -> f64 {
    let f = 10f64.powi(decimals as i32);
    let r = (x * f).round() / f;
    if r.is_finite() { r + 0.0 } else { x }
}

fn last_value(series: &[Option<f64>]) -> Option<f64> {
    series.last().copied().flatten()
}

fn closes(bars: &[Bar]) -> Vec<f64> {
    bars.iter().map(|b| b.close).collect()
}

/// Cleans a frame and drops the bar that is still forming. Bars with non-finite or
/// inconsistent prices are skipped. A bar is forming when it has not ended by `now`;
/// when the market is closed the last bar has ended and is kept.
fn closed_bars(raw: Option<&[Bar]>, tf: Timeframe, now: i64) -> Vec<Bar> {
    let Some(raw) = raw else { return Vec::new() };
    let mut bars: Vec<Bar> = raw
        .iter()
        .copied()
        .filter(|b| {
            b.open.is_finite()
                && b.high.is_finite()
                && b.low.is_finite()
                && b.close.is_finite()
                && b.high >= b.low
        })
        .collect();
    bars.sort_by_key(|b| b.time);
    bars.dedup_by_key(|b| b.time);
    if bars
        .last()
        .is_some_and(|b| b.time.saturating_add(tf.seconds()) > now)
    {
        bars.pop();
    }
    bars
}

fn trend_calc(bars: &[Bar]) -> Option<TrendCalc> {
    let c = closes(bars);
    let ema_fast = last_value(&indicators::ema(&c, EMA_FAST))?;
    let ema_slow = last_value(&indicators::ema(&c, EMA_SLOW))?;
    let close = *c.last()?;
    let trend = if ema_fast > ema_slow && close > ema_slow {
        Trend::Up
    } else if ema_fast < ema_slow && close < ema_slow {
        Trend::Down
    } else {
        Trend::Mixed
    };
    Some(TrendCalc {
        trend,
        ema_fast,
        ema_slow,
        close,
    })
}

fn summary_row(
    tf: Timeframe,
    bars: &[Bar],
    calc: &TrendCalc,
    digits: u32,
) -> Option<TimeframeSummary> {
    let c = closes(bars);
    let rsi = last_value(&indicators::rsi(&c, RSI_PERIOD))?;
    let (_, _, hist) = indicators::macd(&c, MACD_FAST, MACD_SLOW, MACD_SIGNAL);
    let hist = last_value(&hist)?;
    let atr = last_value(&indicators::atr(bars, ATR_PERIOD))?;
    let px = |v: f64| round_to(v, digits + 1);
    Some(TimeframeSummary {
        timeframe: tf,
        trend: calc.trend,
        ema20: px(calc.ema_fast),
        ema50: px(calc.ema_slow),
        rsi: round_to(rsi, 2),
        macd_hist: px(hist),
        atr: px(atr),
    })
}

fn rel(a: f64, b: f64) -> &'static str {
    match a.partial_cmp(&b) {
        Some(Ordering::Greater) => "above",
        Some(Ordering::Less) => "below",
        _ => "level with",
    }
}

fn bias_of(points: f64) -> Bias {
    if points > 0.0 {
        Bias::Bullish
    } else if points < 0.0 {
        Bias::Bearish
    } else {
        Bias::Neutral
    }
}

fn trend_factor(tf: Timeframe, weight: f64, calc: &TrendCalc, kind: Kind) -> Computed {
    let signal = match calc.trend {
        Trend::Up => 1.0,
        Trend::Down => -1.0,
        Trend::Mixed => 0.0,
    };
    let points = round_to(signal * weight, 2);
    let word = match calc.trend {
        Trend::Up => "up",
        Trend::Down => "down",
        Trend::Mixed => "mixed",
    };
    Computed {
        factor: Factor {
            name: format!("{tf} trend"),
            signal: bias_of(points),
            points,
            detail: format!(
                "EMA{EMA_FAST} {} EMA{EMA_SLOW}, close {} EMA{EMA_SLOW}",
                rel(calc.ema_fast, calc.ema_slow),
                rel(calc.close, calc.ema_slow)
            ),
        },
        kind,
        phrase: format!("{tf} trend is {word}"),
    }
}

fn momentum_factor(closes: &[f64], cfg: &ViewConfig, dp: usize) -> Option<Computed> {
    let (_, _, hist) = indicators::macd(closes, MACD_FAST, MACD_SLOW, MACD_SIGNAL);
    let last = hist.len().checked_sub(1)?;
    let h = hist[last]?;
    let earlier = last.checked_sub(MOMENTUM_LOOKBACK).and_then(|i| hist[i]);
    let sign = if h > 0.0 {
        1.0
    } else if h < 0.0 {
        -1.0
    } else {
        0.0
    };
    let grew = earlier.is_some_and(|p| (h > 0.0 && h > p) || (h < 0.0 && h < p));
    let bonus = if grew { cfg.momentum_growth_bonus } else { 0.0 };
    let value = (sign * (cfg.momentum_base + bonus)).clamp(-1.0, 1.0);
    let points = round_to(value * cfg.weight_m15_momentum, 2);
    let direction = match sign as i32 {
        1 => "positive",
        -1 => "negative",
        _ => "flat",
    };
    let trend = if grew {
        format!(", strengthening over {MOMENTUM_LOOKBACK} bars")
    } else {
        String::new()
    };
    Some(Computed {
        factor: Factor {
            name: "M15 momentum".into(),
            signal: bias_of(points),
            points,
            detail: format!("MACD histogram {h:.dp$} ({direction}{trend})"),
        },
        kind: Kind::Other,
        phrase: format!("M15 momentum is {direction}"),
    })
}

fn rsi_factor(closes: &[f64], cfg: &ViewConfig) -> Option<Computed> {
    let r = last_value(&indicators::rsi(closes, RSI_PERIOD))?;
    let (value, zone, phrase) = if r > cfg.rsi_overbought {
        (-1.0, "stretched (overbought)", "M15 RSI is overbought")
    } else if r >= cfg.rsi_bull_zone {
        (0.5, "in the bullish zone", "M15 RSI is firm")
    } else if r >= cfg.rsi_bear_zone {
        (0.0, "neutral", "M15 RSI is neutral")
    } else if r >= cfg.rsi_oversold {
        (-0.5, "in the bearish zone", "M15 RSI is weak")
    } else {
        (1.0, "stretched (oversold)", "M15 RSI is oversold")
    };
    let points = round_to(value * cfg.weight_m15_rsi, 2);
    Some(Computed {
        factor: Factor {
            name: "M15 RSI".into(),
            signal: bias_of(points),
            points,
            detail: format!("RSI({RSI_PERIOD}) {r:.1} is {zone}"),
        },
        kind: Kind::Other,
        phrase: phrase.into(),
    })
}

#[allow(clippy::too_many_arguments)]
fn level_factor(
    name: &str,
    weight: f64,
    mid: f64,
    level: f64,
    level_name: &str,
    subject: &str,
    object: &str,
    dp: usize,
) -> Computed {
    let (value, word) = match mid.partial_cmp(&level) {
        Some(Ordering::Greater) => (1.0, "above"),
        Some(Ordering::Less) => (-1.0, "below"),
        _ => (0.0, "at"),
    };
    let points = round_to(value * weight, 2);
    Computed {
        factor: Factor {
            name: name.into(),
            signal: bias_of(points),
            points,
            detail: format!("Mid price {mid:.dp$} is {word} the {level_name} {level:.dp$}"),
        },
        kind: Kind::Other,
        phrase: format!("{subject} is {word} {object}"),
    }
}

fn news_factor(score: f64, count: u32, cfg: &ViewConfig) -> Computed {
    let s = score.clamp(-1.0, 1.0);
    let points = round_to(s * cfg.weight_news, 2);
    let label = news_label(s);
    Computed {
        factor: Factor {
            name: "News".into(),
            signal: bias_of(points),
            points,
            detail: format!("Headline tone {s:.2} ({label}) from {count} headlines"),
        },
        kind: Kind::Other,
        phrase: format!("headline tone is {label}"),
    }
}

/// Same bands as the news service uses for its label.
fn news_label(score: f64) -> &'static str {
    if score <= -0.35 {
        "negative"
    } else if score <= -0.1 {
        "slightly negative"
    } else if score < 0.1 {
        "neutral"
    } else if score < 0.35 {
        "slightly positive"
    } else {
        "positive"
    }
}

/// The H4 and H1 trends point in opposite directions.
fn trends_disagree(factors: &[Computed]) -> bool {
    let sign = |kind: Kind| {
        factors
            .iter()
            .find(|c| c.kind == kind)
            .map(|c| c.factor.points.signum() * f64::from(c.factor.points != 0.0))
    };
    match (sign(Kind::H4Trend), sign(Kind::H1Trend)) {
        (Some(a), Some(b)) => a * b < 0.0,
        _ => false,
    }
}

fn view_events(input: &ViewInput, cfg: &ViewConfig) -> Vec<ViewEvent> {
    let now = input.now;
    let from = now.saturating_sub(cfg.event_lookback_minutes.saturating_mul(60));
    let to = now.saturating_add(cfg.event_lookahead_minutes.saturating_mul(60));
    let mut events: Vec<&EventInput> = input
        .events
        .iter()
        .filter(|e| e.time >= from && e.time <= to)
        .collect();
    events.sort_by(|a, b| {
        (a.time, &a.currency, &a.title, a.importance).cmp(&(
            b.time,
            &b.currency,
            &b.title,
            b.importance,
        ))
    });
    events
        .into_iter()
        .map(|e| ViewEvent {
            time: e.time,
            minutes_away: e.time.saturating_sub(now) / 60,
            currency: e.currency.clone(),
            title: e.title.clone(),
            importance: e.importance,
        })
        .collect()
}

/// The closest high-impact event for one of the instrument's currencies inside the
/// blackout window.
fn blackout_event<'a>(input: &'a ViewInput, cfg: &ViewConfig) -> Option<&'a EventInput> {
    let window = cfg.event_blackout_minutes.saturating_mul(60);
    let distance = |e: &EventInput| e.time.saturating_sub(input.now).saturating_abs();
    input
        .events
        .iter()
        .filter(|e| {
            e.importance == Importance::High
                && input
                    .currencies
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&e.currency))
                && distance(e) <= window
        })
        .min_by(|a, b| {
            (distance(a), a.time, &a.currency, &a.title).cmp(&(
                distance(b),
                b.time,
                &b.currency,
                &b.title,
            ))
        })
}

fn event_reason(e: &EventInput, now: i64) -> String {
    let delta = e.time.saturating_sub(now);
    let minutes = delta.saturating_abs() / 60;
    let when = if minutes == 0 {
        "right now".to_string()
    } else {
        let unit = if minutes == 1 { "minute" } else { "minutes" };
        if delta > 0 {
            format!("in {minutes} {unit}")
        } else {
            format!("{minutes} {unit} ago")
        }
    };
    format!("High-impact {} event \"{}\" {when}", e.currency, e.title)
}

fn spread_reason(spread: f64, point: f64, atr_m15: f64, cfg: &ViewConfig, dp: usize) -> String {
    let points = if point.is_finite() && point > 0.0 {
        format!(" ({:.0} points)", spread / point)
    } else {
        String::new()
    };
    format!(
        "Spread {spread:.dp$}{points} is wider than {:.0}% of the M15 ATR ({atr_m15:.dp$})",
        cfg.max_spread_atr_fraction * 100.0
    )
}

/// Entry zone, stop and targets from the ATR rules. `None` when the rounded prices do
/// not keep their order (ATR too small for the price precision) or the ATR is not
/// positive.
#[allow(clippy::too_many_arguments)]
fn build_setup(
    bias: Bias,
    input: &ViewInput,
    cfg: &ViewConfig,
    a: f64,
    m: f64,
    supports: &[f64],
    resistances: &[f64],
    digits: u32,
) -> Option<Setup> {
    if !(a > 0.0 && a.is_finite() && m >= 0.0 && m.is_finite()) {
        return None;
    }
    let depth = cfg.entry_zone_atr_m15 * m;
    let (side, low, high) = match bias {
        Bias::Bullish => (Side::Buy, input.ask - depth, input.ask),
        Bias::Bearish => (Side::Sell, input.bid, input.bid + depth),
        Bias::Neutral => return None,
    };
    let mid = (low + high) / 2.0;
    let (stop, tp1, tp2) = match side {
        Side::Buy => {
            let tp1 = mid + cfg.tp1_atr_h1 * a;
            let tp2 = resistances
                .iter()
                .copied()
                .filter(|r| *r > tp1 && *r - mid <= cfg.tp2_search_atr_h1 * a)
                .min_by(f64::total_cmp)
                .unwrap_or(mid + cfg.tp2_fallback_atr_h1 * a);
            (mid - cfg.stop_atr_h1 * a, tp1, tp2)
        }
        Side::Sell => {
            let tp1 = mid - cfg.tp1_atr_h1 * a;
            let tp2 = supports
                .iter()
                .copied()
                .filter(|s| *s < tp1 && mid - *s <= cfg.tp2_search_atr_h1 * a)
                .max_by(f64::total_cmp)
                .unwrap_or(mid - cfg.tp2_fallback_atr_h1 * a);
            (mid + cfg.stop_atr_h1 * a, tp1, tp2)
        }
    };
    let stop_distance = (mid - stop).abs();
    if stop_distance.is_nan() || stop_distance <= 0.0 {
        return None;
    }
    let risk_reward = round_to((tp1 - mid).abs() / stop_distance, 2);
    let p = |v: f64| round_to(v, digits);
    let setup = Setup {
        side,
        entry_low: p(low),
        entry_high: p(high),
        stop_loss: p(stop),
        take_profit: p(tp1),
        take_profit_2: p(tp2),
        risk_reward,
        atr_h1: round_to(a, digits + 1),
        atr_m15: round_to(m, digits + 1),
    };
    let ordered = match side {
        Side::Buy => {
            setup.stop_loss < setup.entry_low
                && setup.entry_low <= setup.entry_high
                && setup.entry_high < setup.take_profit
                && setup.take_profit <= setup.take_profit_2
        }
        Side::Sell => {
            setup.take_profit_2 <= setup.take_profit
                && setup.take_profit < setup.entry_low
                && setup.entry_low <= setup.entry_high
                && setup.entry_high < setup.stop_loss
        }
    };
    // A level at or below zero means the ATR is out of proportion to the price.
    let positive = [
        setup.stop_loss,
        setup.entry_low,
        setup.take_profit,
        setup.take_profit_2,
    ]
    .iter()
    .all(|v| *v > 0.0);
    (ordered && positive).then_some(setup)
}

struct SummaryParts<'a> {
    insufficient: bool,
    bias: Bias,
    confidence: u32,
    computed: &'a [Computed],
    setup: Option<&'a Setup>,
    stand_reason: Option<&'a str>,
    dp: usize,
}

fn build_summary(p: &SummaryParts<'_>) -> String {
    let dp = p.dp;
    if p.insufficient {
        let mut s = String::from(
            "Not enough price data for a view: the bias is neutral and no setup is offered.",
        );
        if let Some(r) = p.stand_reason {
            s.push_str(&format!(" Standing aside: {r}."));
        }
        return s;
    }
    let lead = match p.bias {
        Bias::Bullish => "Bullish",
        Bias::Bearish => "Bearish",
        Bias::Neutral => "Neutral",
    };
    let mut s = format!("{lead} intraday bias, confidence {} of 100", p.confidence);
    if p.bias == Bias::Neutral {
        s.push_str(": the factors do not agree enough to favour a direction.");
    } else {
        let want = if p.bias == Bias::Bullish { 1.0 } else { -1.0 };
        let mut top: Vec<&Computed> = p
            .computed
            .iter()
            .filter(|c| c.factor.points * want > 0.0)
            .collect();
        // Stable sort keeps the factor order for equal strength.
        top.sort_by(|a, b| b.factor.points.abs().total_cmp(&a.factor.points.abs()));
        top.truncate(2);
        s.push_str(": ");
        s.push_str(&reasons(&top, p.bias));
        s.push('.');
    }
    if let Some(r) = p.stand_reason {
        s.push_str(&format!(" Standing aside: {r}. No setup is offered."));
    } else if let Some(t) = p.setup {
        let side = if t.side == Side::Buy { "Buy" } else { "Sell" };
        s.push_str(&format!(
            " {side} zone {:.dp$} to {:.dp$}, stop {:.dp$}, target {:.dp$}.",
            t.entry_low, t.entry_high, t.stop_loss, t.take_profit
        ));
    } else if p.bias == Bias::Neutral {
        s.push_str(" No setup is offered while the bias is neutral.");
    } else {
        s.push_str(" No setup could be sized from the available data.");
    }
    s
}

fn reasons(top: &[&Computed], bias: Bias) -> String {
    let both_trends = top.len() == 2
        && top.iter().any(|c| c.kind == Kind::H4Trend)
        && top.iter().any(|c| c.kind == Kind::H1Trend);
    if both_trends {
        let dir = if bias == Bias::Bullish { "up" } else { "down" };
        return format!("H4 and H1 trends are {dir}");
    }
    top.iter()
        .map(|c| c.phrase.as_str())
        .collect::<Vec<_>>()
        .join(" and ")
}

//! Technical indicators as pure functions.
//!
//! Conventions shared by all of them:
//!
//! - Series outputs have the same length as the input; entries before the indicator has
//!   enough history (the warm-up) are `None`.
//! - Input shorter than the warm-up gives all `None`; a zero period gives all `None`.
//! - Empty or NaN input never panics. A non-finite value that reaches a computation
//!   poisons it: that entry and every later entry that depends on it is `None`.

use serde::{Deserialize, Serialize};

use crate::types::Bar;

/// Three aligned series, e.g. `(macd, signal, histogram)`.
pub type Triple = (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>);

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// Simple moving average: the mean of the last `period` values.
///
/// Warm-up: the first `period - 1` entries are `None`.
pub fn sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = values.len();
    let mut out = vec![None; n];
    if period == 0 || n < period {
        return out;
    }
    for i in (period - 1)..n {
        let window = &values[i + 1 - period..=i];
        out[i] = finite(window.iter().sum::<f64>() / period as f64);
    }
    out
}

/// Exponential moving average with smoothing `k = 2 / (period + 1)`:
/// `ema[i] = ema[i-1] + k * (value[i] - ema[i-1])`.
///
/// Seeded with the SMA of the first `period` values, so the first value is at index
/// `period - 1` and the first `period - 1` entries are `None`.
pub fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = values.len();
    let mut out = vec![None; n];
    if period == 0 || n < period {
        return out;
    }
    let seed = values[..period].iter().sum::<f64>() / period as f64;
    if !seed.is_finite() {
        return out;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut prev = seed;
    out[period - 1] = Some(seed);
    for (slot, &v) in out.iter_mut().zip(values).skip(period) {
        let next = prev + k * (v - prev);
        if !next.is_finite() {
            break;
        }
        *slot = Some(next);
        prev = next;
    }
    out
}

/// EMA over a series that itself has a warm-up (used for the MACD signal line). The
/// EMA starts at the first defined entry.
fn ema_of_series(values: &[Option<f64>], period: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; values.len()];
    let Some(start) = values.iter().position(Option::is_some) else {
        return out;
    };
    let run: Vec<f64> = values[start..].iter().map_while(|v| *v).collect();
    for (slot, v) in out[start..].iter_mut().zip(ema(&run, period)) {
        *slot = v;
    }
    out
}

/// Relative strength index with Wilder smoothing.
///
/// The first average gain and loss are plain means over the first `period` changes;
/// after that `avg = (avg * (period - 1) + x) / period`. `RSI = 100 - 100 / (1 + RS)`
/// with `RS = avg_gain / avg_loss`.
///
/// Warm-up: `period` closes are needed for the first change set, so the first value is
/// at index `period` and the first `period` entries are `None`.
///
/// No losses in the window: the RSI is 100 when there are gains and 50 when the series
/// is completely flat (no gains either). 50 is the neutral reading; 100 or 0 for a
/// market that did not move would read as an extreme.
pub fn rsi(closes: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = closes.len();
    let mut out = vec![None; n];
    if period == 0 || n <= period {
        return out;
    }
    let p = period as f64;
    let split = |d: f64| if d > 0.0 { (d, 0.0) } else { (0.0, -d) };
    let (mut gain, mut loss) = (0.0, 0.0);
    for i in 1..=period {
        let (g, l) = split(closes[i] - closes[i - 1]);
        gain += g;
        loss += l;
    }
    let (mut avg_gain, mut avg_loss) = (gain / p, loss / p);
    out[period] = rsi_value(avg_gain, avg_loss);
    for i in (period + 1)..n {
        let (g, l) = split(closes[i] - closes[i - 1]);
        avg_gain = (avg_gain * (p - 1.0) + g) / p;
        avg_loss = (avg_loss * (p - 1.0) + l) / p;
        match rsi_value(avg_gain, avg_loss) {
            Some(v) => out[i] = Some(v),
            None => break,
        }
    }
    out
}

fn rsi_value(avg_gain: f64, avg_loss: f64) -> Option<f64> {
    if !avg_gain.is_finite() || !avg_loss.is_finite() {
        return None;
    }
    if avg_loss == 0.0 {
        return Some(if avg_gain == 0.0 { 50.0 } else { 100.0 });
    }
    Some(100.0 - 100.0 / (1.0 + avg_gain / avg_loss))
}

/// Moving average convergence divergence: `(macd, signal, histogram)`.
///
/// `macd = EMA(fast) - EMA(slow)` of the closes, `signal = EMA(macd, signal_period)`
/// (seeded with the SMA of the first `signal_period` MACD values) and
/// `histogram = macd - signal`.
///
/// Warm-up: the MACD line starts at index `slow - 1`; signal and histogram at
/// `slow + signal_period - 2`. `fast` must be smaller than `slow` for the usual meaning,
/// but any positive periods are accepted.
pub fn macd(closes: &[f64], fast: usize, slow: usize, signal_period: usize) -> Triple {
    let fast_ema = ema(closes, fast);
    let slow_ema = ema(closes, slow);
    let line: Vec<Option<f64>> = fast_ema
        .iter()
        .zip(&slow_ema)
        .map(|(f, s)| match (f, s) {
            (Some(f), Some(s)) => finite(f - s),
            _ => None,
        })
        .collect();
    let signal = ema_of_series(&line, signal_period);
    let hist = line
        .iter()
        .zip(&signal)
        .map(|(m, s)| match (m, s) {
            (Some(m), Some(s)) => finite(m - s),
            _ => None,
        })
        .collect();
    (line, signal, hist)
}

/// True range per bar: `max(high - low, |high - prev_close|, |low - prev_close|)`, so a
/// gap from the previous close counts as range.
///
/// Warm-up: the first bar has no previous close, so index 0 is `None`.
pub fn true_range(bars: &[Bar]) -> Vec<Option<f64>> {
    let mut out = vec![None; bars.len()];
    for i in 1..bars.len() {
        out[i] = finite(tr(&bars[i], bars[i - 1].close));
    }
    out
}

fn tr(bar: &Bar, prev_close: f64) -> f64 {
    // f64::max skips NaN, so a bad bar would silently shrink the range; poison it.
    if !(bar.high.is_finite() && bar.low.is_finite() && prev_close.is_finite()) {
        return f64::NAN;
    }
    (bar.high - bar.low)
        .max((bar.high - prev_close).abs())
        .max((bar.low - prev_close).abs())
}

/// Average true range with Wilder smoothing.
///
/// The first value is the mean of the first `period` true ranges (bars 1 to `period`);
/// after that `atr = (atr * (period - 1) + tr) / period`.
///
/// Warm-up: the first value is at index `period`; the first `period` entries are `None`.
pub fn atr(bars: &[Bar], period: usize) -> Vec<Option<f64>> {
    let n = bars.len();
    let mut out = vec![None; n];
    if period == 0 || n <= period {
        return out;
    }
    let p = period as f64;
    let seed = (1..=period)
        .map(|i| tr(&bars[i], bars[i - 1].close))
        .sum::<f64>()
        / p;
    if !seed.is_finite() {
        return out;
    }
    let mut prev = seed;
    out[period] = Some(seed);
    for i in (period + 1)..n {
        let next = (prev * (p - 1.0) + tr(&bars[i], bars[i - 1].close)) / p;
        if !next.is_finite() {
            break;
        }
        out[i] = Some(next);
        prev = next;
    }
    out
}

/// Bollinger bands: `(mid, upper, lower)` where `mid` is the SMA and the bands sit
/// `k` population standard deviations (divisor `period`) either side.
///
/// Warm-up: the first `period - 1` entries are `None`.
pub fn bollinger(closes: &[f64], period: usize, k: f64) -> Triple {
    let n = closes.len();
    let mid = sma(closes, period);
    let mut upper = vec![None; n];
    let mut lower = vec![None; n];
    for i in 0..n {
        let Some(m) = mid[i] else { continue };
        let window = &closes[i + 1 - period..=i];
        let var = window.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / period as f64;
        let sd = var.sqrt();
        upper[i] = finite(m + k * sd);
        lower[i] = finite(m - k * sd);
    }
    (mid, upper, lower)
}

/// Volume weighted average price over the bars given: the typical price
/// `(high + low + close) / 3` weighted by `tick_volume`. When every volume is 0 the
/// bars are weighted equally. `None` for empty or non-finite input.
///
/// The caller picks the bars (for a session VWAP, the bars since the session start).
pub fn vwap(bars: &[Bar]) -> Option<f64> {
    if bars.is_empty() {
        return None;
    }
    let total_volume: f64 = bars.iter().map(|b| b.tick_volume as f64).sum();
    let typical = |b: &Bar| (b.high + b.low + b.close) / 3.0;
    let value = if total_volume > 0.0 {
        bars.iter()
            .map(|b| typical(b) * b.tick_volume as f64)
            .sum::<f64>()
            / total_volume
    } else {
        bars.iter().map(typical).sum::<f64>() / bars.len() as f64
    };
    finite(value)
}

/// Classic floor pivots.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pivots {
    pub pivot: f64,
    pub r1: f64,
    pub r2: f64,
    pub s1: f64,
    pub s2: f64,
}

/// Classic floor pivots from the previous day's bar:
/// `P = (H + L + C) / 3`, `R1 = 2P - L`, `S1 = 2P - H`, `R2 = P + (H - L)`,
/// `S2 = P - (H - L)`.
pub fn pivots(prev_day: &Bar) -> Pivots {
    let (h, l, c) = (prev_day.high, prev_day.low, prev_day.close);
    let p = (h + l + c) / 3.0;
    Pivots {
        pivot: p,
        r1: 2.0 * p - l,
        r2: p + (h - l),
        s1: 2.0 * p - h,
        s2: p - (h - l),
    }
}

/// Swing support and resistance levels from fractal lows and highs.
///
/// A bar is a fractal high when its high is strictly above the highs of the `left` bars
/// before it and at least as high as the `right` bars after it (so a flat top counts
/// once, at its first bar); fractal lows mirror that. Levels closer together than a
/// tolerance of `0.25 x` the last ATR(14) of `bars` are merged into their mean (no
/// tolerance when the ATR is not available yet).
///
/// Supports are fractal lows below the last close, resistances fractal highs above it;
/// both are sorted by distance from the last close, nearest first.
pub fn swing_levels(bars: &[Bar], left: usize, right: usize) -> (Vec<f64>, Vec<f64>) {
    let tolerance = atr(bars, 14)
        .last()
        .copied()
        .flatten()
        .map_or(0.0, |a| 0.25 * a);
    match bars.last() {
        Some(last) => swing_levels_around(bars, left, right, tolerance, last.close),
        None => (Vec::new(), Vec::new()),
    }
}

/// Like [`swing_levels`] with an explicit merge `tolerance` (in price) and an explicit
/// `reference` price to split supports from resistances and to sort by distance.
pub fn swing_levels_around(
    bars: &[Bar],
    left: usize,
    right: usize,
    tolerance: f64,
    reference: f64,
) -> (Vec<f64>, Vec<f64>) {
    let n = bars.len();
    if left == 0 || right == 0 || n < left + right + 1 || !reference.is_finite() {
        return (Vec::new(), Vec::new());
    }
    let mut highs = Vec::new();
    let mut lows = Vec::new();
    for i in left..(n - right) {
        let before = &bars[i - left..i];
        let after = &bars[i + 1..=i + right];
        let (h, l) = (bars[i].high, bars[i].low);
        if before.iter().all(|b| h > b.high) && after.iter().all(|b| h >= b.high) {
            highs.push(h);
        }
        if before.iter().all(|b| l < b.low) && after.iter().all(|b| l <= b.low) {
            lows.push(l);
        }
    }
    let by_distance = |a: &f64, b: &f64| {
        (a - reference)
            .abs()
            .total_cmp(&(b - reference).abs())
            .then(a.total_cmp(b))
    };
    let mut supports: Vec<f64> = merge_close(lows, tolerance)
        .into_iter()
        .filter(|l| *l < reference)
        .collect();
    let mut resistances: Vec<f64> = merge_close(highs, tolerance)
        .into_iter()
        .filter(|h| *h > reference)
        .collect();
    supports.sort_by(by_distance);
    resistances.sort_by(by_distance);
    (supports, resistances)
}

/// Groups sorted prices whose distance to the running group mean is within `tolerance`
/// and replaces each group by its mean.
fn merge_close(mut prices: Vec<f64>, tolerance: f64) -> Vec<f64> {
    prices.retain(|p| p.is_finite());
    prices.sort_by(f64::total_cmp);
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 {
        tolerance
    } else {
        0.0
    };
    let mut merged = Vec::new();
    let (mut sum, mut count) = (0.0, 0usize);
    for p in prices {
        if count > 0 && p - sum / count as f64 <= tolerance {
            sum += p;
            count += 1;
        } else {
            if count > 0 {
                merged.push(sum / count as f64);
            }
            sum = p;
            count = 1;
        }
    }
    if count > 0 {
        merged.push(sum / count as f64);
    }
    merged
}

//! Oracle (Quant): answers in probabilities, from the symbol's own past returns.
//!
//! The model is empirical and deliberately simple (in place of TimesFM): take every window of
//! `horizon` bars in the history, apply each window's return to the current price, and count.
//! - P(close above `level` after `horizon` bars) = share of windows that end above it.
//! - P(up) = share of windows with a positive return.
//! - The expected range is the 10th to 90th percentile of the projected prices.
//!
//! It needs at least [`MIN_SAMPLES`] windows; with fewer, every figure is `null`. It is a
//! description of the past, not a prediction, and says so.

use jarvis_market::Bar;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AgentId, Finding, Source, round_to};

pub const MIN_SAMPLES: usize = 30;
pub const DISCLAIMER: &str = "Probabilities describe how this symbol moved over past windows of the same length. They are \
     not a prediction and not advice.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub symbol: String,
    pub timeframe: String,
    pub horizon_bars: usize,
    pub price: Option<f64>,
    pub level: Option<f64>,
    pub samples: usize,
    pub p_up: Option<f64>,
    pub p_above_level: Option<f64>,
    pub p_below_level: Option<f64>,
    pub range_low: Option<f64>,
    pub range_high: Option<f64>,
    pub median: Option<f64>,
}

fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted.get(idx).copied()
}

/// The numbers. `bars` oldest first; `price` the live price (else the last close).
pub fn compute(
    symbol: &str,
    timeframe: &str,
    bars: &[Bar],
    horizon: usize,
    price: Option<f64>,
    level: Option<f64>,
    digits: u32,
) -> Forecast {
    let closes: Vec<f64> = bars
        .iter()
        .map(|b| b.close)
        .filter(|c| *c > 0.0 && c.is_finite())
        .collect();
    let price = price.or(closes.last().copied());
    let horizon = horizon.max(1);
    let returns: Vec<f64> = if closes.len() > horizon {
        (0..closes.len() - horizon)
            .map(|i| closes[i + horizon] / closes[i])
            .collect()
    } else {
        Vec::new()
    };
    let empty = Forecast {
        symbol: symbol.to_owned(),
        timeframe: timeframe.to_owned(),
        horizon_bars: horizon,
        price,
        level,
        samples: returns.len(),
        p_up: None,
        p_above_level: None,
        p_below_level: None,
        range_low: None,
        range_high: None,
        median: None,
    };
    let Some(p0) = price else { return empty };
    if returns.len() < MIN_SAMPLES {
        return empty;
    }
    let n = returns.len() as f64;
    let mut projected: Vec<f64> = returns.iter().map(|r| p0 * r).collect();
    projected.sort_by(f64::total_cmp);
    let share = |pred: &dyn Fn(f64) -> bool| {
        round_to(projected.iter().filter(|p| pred(**p)).count() as f64 / n, 3)
    };
    let p_up = share(&|p| p > p0);
    let (p_above, p_below) = match level {
        Some(l) => (share(&|p| p > l), share(&|p| p < l)),
        None => (None, None),
    };
    Forecast {
        p_up,
        p_above_level: p_above,
        p_below_level: p_below,
        range_low: percentile(&projected, 0.10).and_then(|v| round_to(v, digits)),
        range_high: percentile(&projected, 0.90).and_then(|v| round_to(v, digits)),
        median: percentile(&projected, 0.50).and_then(|v| round_to(v, digits)),
        ..empty
    }
}

/// Oracle's finding.
pub fn forecast(f: Forecast, at: Option<String>) -> Finding {
    let summary = match (f.p_up, f.range_low, f.range_high) {
        (Some(up), Some(lo), Some(hi)) => {
            let mut s = format!(
                "Over {} {} bars, {} closed higher {:.0}% of the time in {} past windows; the middle 80% \
                 of outcomes from here is {} to {}.",
                f.horizon_bars,
                f.timeframe,
                f.symbol,
                up * 100.0,
                f.samples,
                lo,
                hi
            );
            if let (Some(l), Some(a)) = (f.level, f.p_above_level) {
                s.push_str(&format!(" Chance of ending above {l}: {:.0}%.", a * 100.0));
            }
            s
        }
        _ => format!(
            "Not enough history for {} on {} to give probabilities ({} windows, {} needed).",
            f.symbol, f.timeframe, f.samples, MIN_SAMPLES
        ),
    };
    let what = format!("{} {} bars", f.symbol, f.timeframe);
    Finding::new(
        AgentId::Oracle,
        summary,
        json!({ "forecast": f, "model": "empirical returns", "disclaimer": DISCLAIMER }),
    )
    .with_source(Source::new("mt5", what, at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(closes: &[f64]) -> Vec<Bar> {
        closes
            .iter()
            .map(|&c| Bar {
                time: 0,
                open: c,
                high: c,
                low: c,
                close: c,
                tick_volume: 0,
                spread: 0,
            })
            .collect()
    }

    #[test]
    fn a_steady_rise_is_always_up() {
        let closes: Vec<f64> = (0..60).map(|i| 100.0 + i as f64).collect();
        let f = compute("NVDA", "D1", &bars(&closes), 5, None, Some(1000.0), 2);
        assert_eq!(f.samples, 55);
        assert_eq!(f.p_up, Some(1.0));
        assert_eq!(f.p_above_level, Some(0.0));
        assert_eq!(f.p_below_level, Some(1.0));
        let (lo, hi) = (f.range_low.unwrap(), f.range_high.unwrap());
        assert!(lo > 159.0 && hi > lo);
    }

    #[test]
    fn alternating_moves_split_evenly() {
        let closes: Vec<f64> = (0..81)
            .map(|i| if i % 2 == 0 { 100.0 } else { 101.0 })
            .collect();
        let f = compute("X", "H1", &bars(&closes), 1, Some(100.0), None, 2);
        assert_eq!(f.samples, 80);
        assert_eq!(f.p_up, Some(0.5));
        assert_eq!(f.p_above_level, None);
    }

    #[test]
    fn short_history_gives_no_numbers() {
        let f = compute("X", "D1", &bars(&[1.0, 2.0, 3.0]), 1, None, None, 2);
        assert_eq!(f.p_up, None);
        assert_eq!(f.range_low, None);
        let finding = forecast(f, None);
        assert!(finding.summary.starts_with("Not enough history"));
        assert!(finding.data["forecast"]["p_up"].is_null());
    }
}

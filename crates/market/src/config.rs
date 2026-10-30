//! Tunable numbers of the scoring model. `Default` is the approved model.

use serde::{Deserialize, Serialize};

/// Weights, thresholds and multipliers. Every field can be overridden from a config
/// file; missing fields keep their default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewConfig {
    // Factor weights (points at a signal of +1 or -1).
    pub weight_h4_trend: f64,
    pub weight_h1_trend: f64,
    pub weight_m15_momentum: f64,
    pub weight_m15_rsi: f64,
    pub weight_vwap: f64,
    pub weight_pivot: f64,
    pub weight_news: f64,

    /// Absolute score at which the bias stops being neutral.
    pub bias_threshold: f64,
    /// Confidence ceiling when the H4 and H1 trends point in opposite directions.
    pub confidence_cap_on_disagreement: f64,

    // M15 RSI bands: above `overbought` and below `oversold` are "stretched" (contrarian).
    pub rsi_overbought: f64,
    pub rsi_bull_zone: f64,
    pub rsi_bear_zone: f64,
    pub rsi_oversold: f64,

    /// MACD histogram signal for its sign alone (before the growth bonus).
    pub momentum_base: f64,
    /// Added in the histogram's direction when it grew over the last 3 bars.
    pub momentum_growth_bonus: f64,

    /// A high-impact event this close (before or after `now`) means "stand aside".
    pub event_blackout_minutes: i64,
    /// Stand aside when the spread exceeds this fraction of the M15 ATR.
    pub max_spread_atr_fraction: f64,

    // Setup sizing. `m` is the M15 ATR and `a` the H1 ATR.
    /// Entry zone depth, in multiples of `m`.
    pub entry_zone_atr_m15: f64,
    /// Stop distance from the zone's middle, in multiples of `a`.
    pub stop_atr_h1: f64,
    /// First target distance, in multiples of `a`.
    pub tp1_atr_h1: f64,
    /// A swing level is a candidate second target within this many `a`.
    pub tp2_search_atr_h1: f64,
    /// Second target distance when no swing level qualifies, in multiples of `a`.
    pub tp2_fallback_atr_h1: f64,

    /// How long a view stays valid.
    pub horizon_minutes: i64,

    /// Calendar events shown: this far ahead and this far back.
    pub event_lookahead_minutes: i64,
    pub event_lookback_minutes: i64,

    /// Fractal window on H1 (bars on each side of a swing point).
    pub swing_left: usize,
    pub swing_right: usize,
    /// Swing levels closer than this many H1 ATRs are merged.
    pub swing_merge_atr_h1: f64,
    /// How many support and resistance levels the view lists.
    pub levels_per_side: usize,

    /// Fewer technical factors than this and the view is a neutral "not enough data".
    pub min_factors: usize,
}

impl Default for ViewConfig {
    fn default() -> Self {
        Self {
            weight_h4_trend: 25.0,
            weight_h1_trend: 20.0,
            weight_m15_momentum: 15.0,
            weight_m15_rsi: 10.0,
            weight_vwap: 10.0,
            weight_pivot: 10.0,
            weight_news: 10.0,
            bias_threshold: 20.0,
            confidence_cap_on_disagreement: 50.0,
            rsi_overbought: 70.0,
            rsi_bull_zone: 55.0,
            rsi_bear_zone: 45.0,
            rsi_oversold: 30.0,
            momentum_base: 0.5,
            momentum_growth_bonus: 0.5,
            event_blackout_minutes: 30,
            max_spread_atr_fraction: 0.25,
            entry_zone_atr_m15: 0.5,
            stop_atr_h1: 1.0,
            tp1_atr_h1: 1.5,
            tp2_search_atr_h1: 3.0,
            tp2_fallback_atr_h1: 2.5,
            horizon_minutes: 240,
            event_lookahead_minutes: 24 * 60,
            event_lookback_minutes: 60,
            swing_left: 3,
            swing_right: 3,
            swing_merge_atr_h1: 0.25,
            levels_per_side: 3,
            min_factors: 3,
        }
    }
}

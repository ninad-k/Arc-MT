//! The computed view. The JSON shape follows the API's `MarketViewDto` minus the fields
//! the API adds (id, broker symbol, ISO times, headlines).

use serde::{Deserialize, Serialize};

use crate::types::{Importance, Timeframe};

/// Shown with every view.
pub const DISCLAIMER: &str = "Computed from price data, indicators, the economic calendar and headlines. It is not financial advice and no outcome is guaranteed.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bias {
    Bullish,
    Bearish,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    Up,
    Down,
    /// The fast EMA and the close do not agree about the slow EMA.
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub bid: f64,
    pub ask: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub side: Side,
    pub entry_low: f64,
    pub entry_high: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub take_profit_2: f64,
    pub risk_reward: f64,
    pub atr_h1: f64,
    pub atr_m15: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeframeSummary {
    pub timeframe: Timeframe,
    pub trend: Trend,
    pub ema20: f64,
    pub ema50: f64,
    pub rsi: f64,
    pub macd_hist: f64,
    pub atr: f64,
}

/// Key levels. A field is `null` when its inputs were not available.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Levels {
    pub pivot: Option<f64>,
    pub r1: Option<f64>,
    pub r2: Option<f64>,
    pub s1: Option<f64>,
    pub s2: Option<f64>,
    pub prev_day_high: Option<f64>,
    pub prev_day_low: Option<f64>,
    pub vwap: Option<f64>,
    /// Nearest swing supports below the price, closest first.
    pub support: Vec<f64>,
    /// Nearest swing resistances above the price, closest first.
    pub resistance: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Factor {
    pub name: String,
    pub signal: Bias,
    pub points: f64,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewEvent {
    /// Unix seconds on the input clock.
    pub time: i64,
    /// Whole minutes from `now` (negative once the event has happened).
    pub minutes_away: i64,
    pub currency: String,
    pub title: String,
    pub importance: Importance,
}

/// The news section. The API adds the headlines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsView {
    pub configured: bool,
    pub score: f64,
    pub label: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketView {
    pub symbol: String,
    /// Unix seconds on the input clock.
    pub valid_until: i64,
    pub price: Price,
    pub bias: Bias,
    /// 0 to 100.
    pub confidence: u32,
    /// -100 to 100.
    pub score: f64,
    pub stand_aside: bool,
    pub stand_aside_reason: Option<String>,
    pub setup: Option<Setup>,
    pub timeframes: Vec<TimeframeSummary>,
    pub levels: Levels,
    pub factors: Vec<Factor>,
    pub events: Vec<ViewEvent>,
    pub news: NewsView,
    pub summary: String,
    pub disclaimer: String,
    /// What was missing or skipped, in plain words.
    pub notes: Vec<String>,
}

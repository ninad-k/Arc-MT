//! Input types: bars, timeframes, calendar events, news and the full view input.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// One OHLC bar. `time` is the bar open time in unix seconds on the input clock.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(default)]
    pub tick_volume: u64,
    #[serde(default)]
    pub spread: i64,
}

/// The frames a view uses. Ordered from the shortest to the longest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    M5,
    M15,
    H1,
    H4,
    D1,
}

impl Timeframe {
    /// Length of one bar in seconds.
    pub const fn seconds(self) -> i64 {
        match self {
            Timeframe::M5 => 300,
            Timeframe::M15 => 900,
            Timeframe::H1 => 3_600,
            Timeframe::H4 => 14_400,
            Timeframe::D1 => 86_400,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Timeframe::M5 => "M5",
            Timeframe::M15 => "M15",
            Timeframe::H1 => "H1",
            Timeframe::H4 => "H4",
            Timeframe::D1 => "D1",
        }
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Calendar importance, ordered `Low < Moderate < High`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    Low,
    Moderate,
    High,
}

impl Importance {
    pub const fn as_str(self) -> &'static str {
        match self {
            Importance::Low => "low",
            Importance::Moderate => "moderate",
            Importance::High => "high",
        }
    }
}

/// A calendar event, already filtered by the caller to what it wants considered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventInput {
    pub time: i64,
    pub currency: String,
    pub title: String,
    pub importance: Importance,
}

/// Headline sentiment, already oriented to the instrument's direction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NewsInput {
    /// -1 (bearish for the instrument) to +1 (bullish).
    pub score: f64,
    /// Number of headlines behind the score.
    pub count: u32,
}

/// Everything `analyze` needs. All times are unix seconds on one clock.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewInput {
    pub symbol: String,
    pub digits: u32,
    pub point: f64,
    pub bid: f64,
    pub ask: f64,
    pub now: i64,
    /// Oldest bar first; the last bar of each frame is the one still forming.
    pub frames: BTreeMap<Timeframe, Vec<Bar>>,
    /// Currencies that move the instrument, e.g. `["EUR", "USD"]` for EURUSD.
    pub currencies: Vec<String>,
    pub events: Vec<EventInput>,
    pub news: Option<NewsInput>,
}

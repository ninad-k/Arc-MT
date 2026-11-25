//! Chartist (Technician): drives the desk chart.
//!
//! It decides what the chart shows (symbol, timeframe, the last bars) and what is drawn on it:
//! levels (entry, stop, targets, zones, liquidity) and an optional forecast band from Oracle. The
//! web draws exactly this, so the chart never shows a line the trader was not told about.

use jarvis_market::Bar;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AgentId, Finding, Source};

/// Bars sent to the chart at most.
pub const MAX_BARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelKind {
    Entry,
    Stop,
    Target,
    Supply,
    Demand,
    Liquidity,
    Marker,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub kind: LevelKind,
    pub price: f64,
    /// The top of a zone; `None` for a line.
    pub to: Option<f64>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub low: f64,
    pub high: f64,
    pub bars_ahead: usize,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    pub symbol: String,
    pub timeframe: String,
    pub digits: u32,
    pub bars: Vec<Bar>,
    pub levels: Vec<Level>,
    pub band: Option<Band>,
}

/// Build the chart: the last [`MAX_BARS`] bars, levels sorted from high to low, with lines that
/// are not finite dropped.
pub fn chart(
    symbol: &str,
    timeframe: &str,
    digits: u32,
    bars: &[Bar],
    mut levels: Vec<Level>,
    band: Option<Band>,
) -> Chart {
    levels.retain(|l| l.price.is_finite() && l.to.is_none_or(f64::is_finite));
    levels.sort_by(|a, b| b.price.total_cmp(&a.price));
    let start = bars.len().saturating_sub(MAX_BARS);
    Chart {
        symbol: symbol.to_owned(),
        timeframe: timeframe.to_owned(),
        digits,
        bars: bars[start..].to_vec(),
        levels,
        band: band.filter(|b| b.low.is_finite() && b.high.is_finite() && b.low <= b.high),
    }
}

pub fn finding(c: &Chart, at: Option<String>) -> Finding {
    let summary = if c.bars.is_empty() {
        format!(
            "No {} bars for {}, so the chart is empty.",
            c.timeframe, c.symbol
        )
    } else {
        format!(
            "Chart switched to {} {} with {} levels drawn{}.",
            c.symbol,
            c.timeframe,
            c.levels.len(),
            if c.band.is_some() {
                " and a forecast band"
            } else {
                ""
            }
        )
    };
    // The model gets the levels, not the bars.
    Finding::new(
        AgentId::Chartist,
        summary,
        json!({ "symbol": c.symbol, "timeframe": c.timeframe, "bars": c.bars.len(),
                "levels": c.levels, "band": c.band }),
    )
    .with_source(Source::new(
        "mt5",
        format!("{} {} bars", c.symbol, c.timeframe),
        at,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lvl(kind: LevelKind, price: f64) -> Level {
        Level {
            kind,
            price,
            to: None,
            label: String::new(),
        }
    }

    #[test]
    fn keeps_the_last_bars_and_sorts_levels() {
        let bars: Vec<Bar> = (0..200)
            .map(|i| Bar {
                time: i,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                tick_volume: 0,
                spread: 0,
            })
            .collect();
        let c = chart(
            "NVDA",
            "H1",
            2,
            &bars,
            vec![
                lvl(LevelKind::Stop, 90.0),
                lvl(LevelKind::Target, 120.0),
                lvl(LevelKind::Entry, f64::NAN),
            ],
            Some(Band {
                low: 2.0,
                high: 1.0,
                bars_ahead: 5,
                label: "bad".into(),
            }),
        );
        assert_eq!(c.bars.len(), MAX_BARS);
        assert_eq!(c.bars[0].time, 80);
        assert_eq!(c.levels.len(), 2);
        assert_eq!(c.levels[0].price, 120.0);
        assert!(c.band.is_none());
        let f = finding(&c, None);
        assert_eq!(f.data["bars"], 120);
        assert!(f.summary.contains("2 levels"));
    }
}

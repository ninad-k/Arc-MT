//! Athena (Analyst): what a symbol is worth on paper and how its chart is built.
//!
//! - **Fundamentals**: a few Finnhub basic-financials figures and the latest filings, passed
//!   through as reported (no estimates).
//! - **Structure**: supply zones (around swing highs above the price) and demand zones (around
//!   swing lows below it), each a band of a quarter ATR either side; liquidity pools at equal
//!   highs or lows (two or more swings within a tenth of an ATR) and at the previous bar's high and
//!   low.

use jarvis_market::Bar;
use jarvis_market::indicators::{atr, swing_levels_around};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AgentId, Finding, Source, round_to};

/// Finnhub basic financials, as reported. `None` when the field is missing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Fundamentals {
    pub pe_ttm: Option<f64>,
    pub eps_ttm: Option<f64>,
    pub net_margin_ttm: Option<f64>,
    pub revenue_growth_ttm_yoy: Option<f64>,
    pub market_cap_millions: Option<f64>,
    pub beta: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filing {
    pub form: String,
    pub filed: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub low: f64,
    pub high: f64,
    /// The swing level the zone is built around.
    pub level: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Liquidity {
    pub price: f64,
    /// `"equal highs"`, `"equal lows"`, `"previous high"` or `"previous low"`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Structure {
    pub timeframe: String,
    pub price: Option<f64>,
    pub atr: Option<f64>,
    pub supply: Vec<Zone>,
    pub demand: Vec<Zone>,
    pub liquidity: Vec<Liquidity>,
}

/// Chart structure from bars (oldest first). Up to three zones each side, nearest first.
pub fn structure(timeframe: &str, bars: &[Bar], price: Option<f64>, digits: u32) -> Structure {
    let price = price.or(bars.last().map(|b| b.close));
    let a = atr(bars, 14).last().copied().flatten();
    let r = |v: f64| round_to(v, digits).unwrap_or(v);
    let mut out = Structure {
        timeframe: timeframe.to_owned(),
        price,
        atr: a.and_then(|v| round_to(v, digits)),
        supply: Vec::new(),
        demand: Vec::new(),
        liquidity: Vec::new(),
    };
    let (Some(p), Some(a)) = (price, a) else {
        return out;
    };
    let half = 0.25 * a;
    let (supports, resistances) = swing_levels_around(bars, 3, 3, 0.0, p);
    let zone = |l: f64| Zone {
        low: r(l - half),
        high: r(l + half),
        level: r(l),
    };
    out.supply = merge(&resistances, half)
        .into_iter()
        .take(3)
        .map(zone)
        .collect();
    out.demand = merge(&supports, half)
        .into_iter()
        .take(3)
        .map(zone)
        .collect();
    let (raw_highs, raw_lows) = raw_swings(bars, 3);
    let above: Vec<f64> = raw_highs.into_iter().filter(|h| *h > p).collect();
    let below: Vec<f64> = raw_lows.into_iter().filter(|l| *l < p).collect();
    for (levels, kind) in [(&above, "equal highs"), (&below, "equal lows")] {
        for cluster in clusters(levels, 0.1 * a) {
            out.liquidity.push(Liquidity {
                price: r(cluster),
                kind: kind.into(),
            });
        }
    }
    if let Some(prev) = bars.len().checked_sub(2).and_then(|i| bars.get(i)) {
        out.liquidity.push(Liquidity {
            price: r(prev.high),
            kind: "previous high".into(),
        });
        out.liquidity.push(Liquidity {
            price: r(prev.low),
            kind: "previous low".into(),
        });
    }
    out
}

/// Every fractal high and low (`k` bars either side), unmerged, so repeats stay visible.
fn raw_swings(bars: &[Bar], k: usize) -> (Vec<f64>, Vec<f64>) {
    let mut highs = Vec::new();
    let mut lows = Vec::new();
    if bars.len() < 2 * k + 1 {
        return (highs, lows);
    }
    for i in k..bars.len() - k {
        let around = bars[i - k..=i + k]
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != k);
        let (h, l) = (bars[i].high, bars[i].low);
        let mut is_high = true;
        let mut is_low = true;
        for (_, b) in around {
            is_high &= h >= b.high;
            is_low &= l <= b.low;
        }
        if is_high {
            highs.push(h);
        }
        if is_low {
            lows.push(l);
        }
    }
    (highs, lows)
}

/// Levels (sorted nearest first) merged when closer than `tol`; keeps the order.
fn merge(levels: &[f64], tol: f64) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for &l in levels {
        if !out.iter().any(|x| (x - l).abs() < tol) {
            out.push(l);
        }
    }
    out
}

/// Means of groups of two or more levels within `tol` of each other.
fn clusters(levels: &[f64], tol: f64) -> Vec<f64> {
    let mut sorted = levels.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mut out = Vec::new();
    let mut group: Vec<f64> = Vec::new();
    for l in sorted {
        if group.last().is_some_and(|g| l - g > tol) {
            if group.len() >= 2 {
                out.push(group.iter().sum::<f64>() / group.len() as f64);
            }
            group.clear();
        }
        group.push(l);
    }
    if group.len() >= 2 {
        out.push(group.iter().sum::<f64>() / group.len() as f64);
    }
    out
}

/// Athena's finding for a symbol. Each part is optional; a missing part is a note.
pub fn analyse(
    symbol: &str,
    fundamentals: Option<Fundamentals>,
    filings: Vec<Filing>,
    structure: Option<Structure>,
    at: Option<String>,
) -> Finding {
    let mut parts = Vec::new();
    if let Some(f) = &fundamentals {
        match (f.pe_ttm, f.week52_low, f.week52_high) {
            (Some(pe), Some(lo), Some(hi)) => {
                parts.push(format!("P/E {pe:.1}, 52-week range {lo} to {hi}"))
            }
            (Some(pe), _, _) => parts.push(format!("P/E {pe:.1}")),
            _ => {}
        }
    }
    if let Some(s) = &structure {
        if let Some(z) = s.supply.first() {
            parts.push(format!("nearest supply {} to {}", z.low, z.high));
        }
        if let Some(z) = s.demand.first() {
            parts.push(format!("nearest demand {} to {}", z.low, z.high));
        }
    }
    let summary = if parts.is_empty() {
        format!("No fundamentals or chart structure came back for {symbol}.")
    } else {
        format!("{symbol}: {}.", parts.join("; "))
    };
    let mut finding = Finding::new(
        AgentId::Athena,
        summary,
        json!({ "symbol": symbol, "fundamentals": fundamentals, "filings": filings,
                "structure": structure }),
    );
    if fundamentals.is_some() {
        finding = finding.with_source(Source::new("finnhub", "basic financials", at.clone()));
    } else {
        finding = finding.with_note("No fundamentals (no Finnhub key, or none for this symbol).");
    }
    if !filings.is_empty() {
        finding = finding.with_source(Source::new("finnhub", "filings", at.clone()));
    }
    if let Some(s) = &structure {
        finding = finding.with_source(Source::new(
            "mt5",
            format!("{symbol} {} bars", s.timeframe),
            at,
        ));
    } else {
        finding = finding.with_note("No bars for chart structure.");
    }
    finding
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zig-zag between 90 and 110 with a final close at 100.
    fn zigzag() -> Vec<Bar> {
        let mut bars = Vec::new();
        for i in 0..60 {
            let phase = i % 10;
            let mid = if phase < 5 {
                90.0 + 4.0 * phase as f64
            } else {
                110.0 - 4.0 * (phase - 5) as f64
            };
            bars.push(Bar {
                time: i,
                open: mid,
                high: mid + 1.0,
                low: mid - 1.0,
                close: mid,
                tick_volume: 0,
                spread: 0,
            });
        }
        bars
    }

    #[test]
    fn finds_supply_above_and_demand_below() {
        let s = structure("H1", &zigzag(), Some(100.0), 2);
        assert!(s.atr.is_some());
        assert!(!s.supply.is_empty() && !s.demand.is_empty());
        assert!(s.supply.iter().all(|z| z.level > 100.0));
        assert!(s.demand.iter().all(|z| z.level < 100.0));
        assert!(s.supply[0].low < s.supply[0].level && s.supply[0].high > s.supply[0].level);
        // Repeated highs at the same price are a liquidity pool.
        assert!(s.liquidity.iter().any(|l| l.kind == "equal highs"));
        assert!(s.liquidity.iter().any(|l| l.kind == "previous high"));
    }

    #[test]
    fn no_bars_means_empty_structure_and_a_note() {
        let s = structure("H1", &[], None, 2);
        assert!(s.supply.is_empty() && s.price.is_none() && s.atr.is_none());
        let f = analyse("NVDA", None, vec![], None, None);
        assert!(f.summary.starts_with("No fundamentals"));
        assert_eq!(f.notes.len(), 2);
        assert!(f.data["fundamentals"].is_null());
    }

    #[test]
    fn fundamentals_are_summarised_as_reported() {
        let f = analyse(
            "NVDA",
            Some(Fundamentals {
                pe_ttm: Some(52.34),
                week52_low: Some(86.62),
                week52_high: Some(212.19),
                ..Default::default()
            }),
            vec![Filing {
                form: "10-Q".into(),
                filed: "2026-11-19".into(),
                url: None,
            }],
            None,
            None,
        );
        assert!(
            f.summary
                .contains("P/E 52.3, 52-week range 86.62 to 212.19")
        );
        assert_eq!(f.data["filings"][0]["form"], "10-Q");
    }
}

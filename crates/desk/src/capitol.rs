//! Capitol (Smart money): who inside the company, and in Congress, is buying or selling.
//!
//! Insider rows come from Finnhub's insider transactions (free tier); congressional trades from
//! its congressional-trading endpoint (premium), passed as `None` when not available. Only open
//! market purchases (`P`) and sales (`S`) count towards the net; grants and option exercises are
//! listed but not counted.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AgentId, Finding, Source, round_to};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsiderTrade {
    pub name: String,
    /// Shares bought (positive) or sold (negative).
    pub change: f64,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub price: Option<f64>,
    /// SEC transaction code (`P` purchase, `S` sale, `A` grant, `M` exercise, ...).
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CongressTrade {
    pub name: String,
    /// `"purchase"` or `"sale"` as reported.
    pub kind: String,
    pub date: String,
    /// The reported amount band, e.g. `"$1,001 - $15,000"`.
    pub amount: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsiderSummary {
    pub since: String,
    pub buys: usize,
    pub sells: usize,
    pub net_shares: f64,
    /// Net value of priced open-market trades; `None` when no trade had a price.
    pub net_value: Option<f64>,
    /// Up to five open-market trades, largest value first.
    pub notable: Vec<InsiderTrade>,
}

/// Open-market trades on or after `since` (`YYYY-MM-DD`).
pub fn summarise(trades: &[InsiderTrade], since: &str) -> InsiderSummary {
    let open: Vec<&InsiderTrade> = trades
        .iter()
        .filter(|t| t.date.as_str() >= since && matches!(t.code.as_str(), "P" | "S"))
        .collect();
    let buys = open.iter().filter(|t| t.change > 0.0).count();
    let sells = open.iter().filter(|t| t.change < 0.0).count();
    let net_shares = open.iter().map(|t| t.change).sum();
    let priced: Vec<f64> = open
        .iter()
        .filter_map(|t| t.price.map(|p| p * t.change))
        .collect();
    let net_value = (!priced.is_empty())
        .then(|| round_to(priced.iter().sum(), 0))
        .flatten();
    let mut notable: Vec<InsiderTrade> = open.iter().map(|t| (*t).clone()).collect();
    notable.sort_by(|a, b| {
        let v = |t: &InsiderTrade| (t.change * t.price.unwrap_or(0.0)).abs();
        v(b).total_cmp(&v(a))
    });
    notable.truncate(5);
    InsiderSummary {
        since: since.to_owned(),
        buys,
        sells,
        net_shares,
        net_value,
        notable,
    }
}

/// Capitol's finding. `insiders` is `None` when the feed is not available (no key).
pub fn smart_money(
    symbol: &str,
    insiders: Option<&[InsiderTrade]>,
    congress: Option<&[CongressTrade]>,
    since: &str,
    at: Option<String>,
) -> Finding {
    let summary_ins = insiders.map(|t| summarise(t, since));
    let mut lines = Vec::new();
    match &summary_ins {
        Some(s) if s.buys + s.sells > 0 => lines.push(format!(
            "{symbol} insiders since {since}: {} buys, {} sells, net {:+.0} shares",
            s.buys, s.sells, s.net_shares
        )),
        Some(_) => lines.push(format!(
            "No open-market insider buying or selling in {symbol} since {since}"
        )),
        None => {}
    }
    if let Some(c) = congress {
        let recent = c.iter().filter(|t| t.date.as_str() >= since).count();
        lines.push(format!("{recent} reported congressional trades"));
    }
    let summary = if lines.is_empty() {
        format!("No smart-money data for {symbol}: the insider feed is not available.")
    } else {
        format!("{}.", lines.join("; "))
    };
    let mut f = Finding::new(
        AgentId::Capitol,
        summary,
        json!({ "symbol": symbol, "insiders": summary_ins,
                "congress": congress.map(|c| c.iter().filter(|t| t.date.as_str() >= since).collect::<Vec<_>>()) }),
    );
    if insiders.is_some() {
        f = f.with_source(Source::new("finnhub", "insider transactions", at.clone()));
    } else {
        f = f.with_note("Insider transactions need a Finnhub key.");
    }
    if congress.is_some() {
        f = f.with_source(Source::new("finnhub", "congressional trading", at));
    } else {
        f = f.with_note("Congressional trades need Finnhub's premium plan; shown as \"—\".");
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(change: f64, price: Option<f64>, code: &str, date: &str) -> InsiderTrade {
        InsiderTrade {
            name: "A. Person".into(),
            change,
            date: date.into(),
            price,
            code: code.into(),
        }
    }

    #[test]
    fn only_open_market_trades_in_the_window_count() {
        let trades = vec![
            t(1000.0, Some(10.0), "P", "2026-11-01"),
            t(-500.0, Some(12.0), "S", "2026-10-01"),
            t(-9000.0, Some(11.0), "S", "2026-01-01"), // too old
            t(50000.0, None, "A", "2026-11-02"),       // a grant: not counted
        ];
        let s = summarise(&trades, "2026-08-26");
        assert_eq!((s.buys, s.sells), (1, 1));
        assert_eq!(s.net_shares, 500.0);
        assert_eq!(s.net_value, Some(4000.0));
        assert_eq!(s.notable[0].change, 1000.0);
        assert_eq!(s.notable.len(), 2);
    }

    #[test]
    fn missing_feeds_are_null_with_notes() {
        let f = smart_money("NVDA", None, None, "2026-08-26", None);
        assert!(f.summary.starts_with("No smart-money data"));
        assert!(f.data["insiders"].is_null());
        assert!(f.data["congress"].is_null());
        assert_eq!(f.notes.len(), 2);
    }

    #[test]
    fn congress_is_counted_when_present() {
        let c = vec![CongressTrade {
            name: "Rep. X".into(),
            kind: "purchase".into(),
            date: "2026-11-10".into(),
            amount: Some("$1,001 - $15,000".into()),
        }];
        let f = smart_money("NVDA", Some(&[]), Some(&c), "2026-08-26", None);
        assert!(f.summary.contains("1 reported congressional trades"));
        assert!(f.summary.contains("No open-market insider"));
    }
}

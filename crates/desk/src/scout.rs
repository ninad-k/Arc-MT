//! Scout (Recon): what is moving today across the scan universe.
//!
//! From each symbol's daily bars (oldest first, the last one still forming): the change from the
//! previous close, today's range as a share of the previous close, and relative volume (today's
//! tick volume over the average of up to 20 earlier days). Ranked by the size of the move.

use jarvis_market::Bar;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AgentId, Finding, Source, round_to};

/// One symbol's input.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanInput {
    /// The name the trader uses (canonical).
    pub symbol: String,
    /// Daily bars, oldest first; the last one is today.
    pub daily: Vec<Bar>,
    /// The live price, when there is one (else today's close so far).
    pub last: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mover {
    pub symbol: String,
    pub last: Option<f64>,
    pub change_pct: Option<f64>,
    pub range_pct: Option<f64>,
    pub rel_volume: Option<f64>,
    /// `"up"`, `"down"` or `"flat"`.
    pub direction: String,
}

fn mover(input: &ScanInput) -> Mover {
    let n = input.daily.len();
    let today = input.daily.last();
    let prev = n.checked_sub(2).and_then(|i| input.daily.get(i));
    let last = input.last.or(today.map(|b| b.close));
    let change_pct = match (last, prev) {
        (Some(l), Some(p)) if p.close > 0.0 => round_to((l / p.close - 1.0) * 100.0, 2),
        _ => None,
    };
    let range_pct = match (today, prev) {
        (Some(t), Some(p)) if p.close > 0.0 => round_to((t.high - t.low) / p.close * 100.0, 2),
        _ => None,
    };
    let rel_volume = today.and_then(|t| {
        let earlier = &input.daily[n.saturating_sub(21)..n.saturating_sub(1)];
        if earlier.is_empty() || t.tick_volume == 0 {
            return None;
        }
        let avg = earlier.iter().map(|b| b.tick_volume as f64).sum::<f64>() / earlier.len() as f64;
        (avg > 0.0)
            .then(|| round_to(t.tick_volume as f64 / avg, 2))
            .flatten()
    });
    let direction = match change_pct {
        Some(c) if c > 0.05 => "up",
        Some(c) if c < -0.05 => "down",
        _ => "flat",
    };
    Mover {
        symbol: input.symbol.clone(),
        last,
        change_pct,
        range_pct,
        rel_volume,
        direction: direction.into(),
    }
}

/// Every symbol's move, biggest first; symbols without a change go last.
pub fn rank(inputs: &[ScanInput]) -> Vec<Mover> {
    let mut all: Vec<Mover> = inputs.iter().map(mover).collect();
    all.sort_by(|a, b| {
        let key = |m: &Mover| m.change_pct.map(f64::abs).unwrap_or(-1.0);
        key(b).total_cmp(&key(a))
    });
    all
}

/// Scout's finding: the top `top` movers.
pub fn movers(inputs: &[ScanInput], top: usize, at: Option<String>) -> Finding {
    let ranked = rank(inputs);
    let with_data: Vec<&Mover> = ranked.iter().filter(|m| m.change_pct.is_some()).collect();
    let summary = if with_data.is_empty() {
        "No movers to report: no daily bars came back for the scan list.".to_owned()
    } else {
        let parts: Vec<String> = with_data
            .iter()
            .take(3)
            .map(|m| format!("{} {:+.2}%", m.symbol, m.change_pct.unwrap_or(0.0)))
            .collect();
        format!(
            "Biggest moves today across {} symbols: {}.",
            inputs.len(),
            parts.join(", ")
        )
    };
    let missing: Vec<&str> = ranked
        .iter()
        .filter(|m| m.change_pct.is_none())
        .map(|m| m.symbol.as_str())
        .collect();
    let mut f = Finding::new(
        AgentId::Scout,
        summary,
        json!({ "movers": ranked.iter().take(top).collect::<Vec<_>>(), "scanned": inputs.len() }),
    )
    .with_source(Source::new("mt5", "daily bars of the scan list", at));
    if !missing.is_empty() {
        f = f.with_note(format!("No daily bars for: {}", missing.join(", ")));
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn bar(close: f64, high: f64, low: f64, vol: u64) -> Bar {
        Bar {
            time: 0,
            open: close,
            high,
            low,
            close,
            tick_volume: vol,
            spread: 0,
        }
    }

    #[test]
    fn ranks_by_size_of_move_and_computes_figures() {
        let up = ScanInput {
            symbol: "NVDA".into(),
            daily: vec![bar(100.0, 101.0, 99.0, 100), bar(104.0, 105.0, 99.0, 300)],
            last: None,
        };
        let down = ScanInput {
            symbol: "AAPL".into(),
            daily: vec![bar(200.0, 201.0, 199.0, 100), bar(198.0, 200.0, 197.0, 100)],
            last: Some(197.0),
        };
        let empty = ScanInput {
            symbol: "MSFT".into(),
            daily: vec![],
            last: None,
        };
        let ranked = rank(&[down.clone(), empty.clone(), up.clone()]);
        assert_eq!(ranked[0].symbol, "NVDA");
        assert_eq!(ranked[0].change_pct, Some(4.0));
        assert_eq!(ranked[0].range_pct, Some(6.0));
        assert_eq!(ranked[0].rel_volume, Some(3.0));
        assert_eq!(ranked[0].direction, "up");
        assert_eq!(ranked[1].symbol, "AAPL");
        assert_eq!(ranked[1].change_pct, Some(-1.5));
        assert_eq!(ranked[1].direction, "down");
        assert_eq!(ranked[2].symbol, "MSFT");
        assert_eq!(ranked[2].change_pct, None);

        let f = movers(&[up, down, empty], 5, None);
        assert!(f.summary.contains("NVDA +4.00%"));
        assert_eq!(f.notes, vec!["No daily bars for: MSFT".to_owned()]);
        assert!(f.data["movers"][2]["change_pct"].is_null());
    }

    #[test]
    fn no_data_says_so() {
        let f = movers(&[], 5, None);
        assert!(f.summary.starts_with("No movers"));
    }
}

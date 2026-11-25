//! Opinions (spec-architecture-page): each general member's finding turned into one structured
//! vote on one symbol, so the council can weigh them and the learning loop can score them.
//!
//! The mapping is code, not a model: free, deterministic and versioned ([`MAPPER_VERSION`]). A
//! member with no usable data votes neutral with confidence 0 and says why in its evidence, so a
//! missing feed never looks like a view.
//!
//! The rules, per member (v1):
//! - **Oracle**: P(up) ≥ 0.55 is bull, ≤ 0.45 bear; confidence = |P(up) − 0.5| × 200.
//! - **Scout**: today's direction; confidence from how much of the day's range the move covers,
//!   plus up to 20 for relative volume.
//! - **Chartist**: room to the nearest level above versus below the price (supply, demand,
//!   liquidity, targets, stops). Ratio ≥ 1.5 is bull, ≤ 1/1.5 bear.
//! - **Athena**: the price at or inside a supply zone (within a quarter ATR) is bear, at a demand
//!   zone bull; both or neither is neutral. Fundamentals are shown as evidence only.
//! - **Atlas**: risk tone of the headlines and the indices' moves, for stocks and indices only.
//! - **Capitol**: insider open-market buys versus sales, for stocks only. Sales are often routine,
//!   so a bearish vote counts half.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AgentId, Finding, Source, round_to};

/// Bump when a rule below changes, so old opinions can be told apart.
pub const MAPPER_VERSION: &str = "v1";

/// The six members that vote.
pub const MEMBERS: [AgentId; 6] = [
    AgentId::Atlas,
    AgentId::Capitol,
    AgentId::Scout,
    AgentId::Athena,
    AgentId::Chartist,
    AgentId::Oracle,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    Bull,
    Bear,
    Neutral,
}

impl Stance {
    /// +1, −1 or 0.
    pub const fn sign(self) -> f64 {
        match self {
            Stance::Bull => 1.0,
            Stance::Bear => -1.0,
            Stance::Neutral => 0.0,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Stance::Bull => "bull",
            Stance::Bear => "bear",
            Stance::Neutral => "neutral",
        }
    }
}

/// What kind of instrument the symbol is; some members only have a view on some kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// A single company's stock (or a stock CFD).
    Stock,
    /// A stock index or index CFD.
    Index,
    /// FX, metals, energy, crypto and the rest.
    Other,
}

/// What every mapper may use besides the finding.
#[derive(Debug, Clone, PartialEq)]
pub struct OpinionContext {
    pub symbol: String,
    pub kind: AssetKind,
    /// The live price, when known.
    pub price: Option<f64>,
    /// The ATR on the decision timeframe, when known.
    pub atr: Option<f64>,
    /// How far ahead the vote looks, e.g. `"4h"`.
    pub horizon: String,
}

/// One member's vote on one symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Opinion {
    pub agent: AgentId,
    pub symbol: String,
    pub stance: Stance,
    /// 0 to 100.
    pub confidence: u8,
    pub horizon: String,
    /// Plain lines, each from the finding's figures.
    pub evidence: Vec<String>,
    pub sources: Vec<Source>,
    pub mapper: String,
}

impl Opinion {
    /// The vote as a number from −1 to +1.
    pub fn signed(&self) -> f64 {
        self.stance.sign() * f64::from(self.confidence) / 100.0
    }
}

fn conf(x: f64) -> u8 {
    if x.is_finite() {
        x.round().clamp(0.0, 100.0) as u8
    } else {
        0
    }
}

fn f(v: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_f64().filter(|x| x.is_finite())
}

struct Vote {
    stance: Stance,
    confidence: u8,
    evidence: Vec<String>,
}

fn neutral(why: impl Into<String>) -> Vote {
    Vote {
        stance: Stance::Neutral,
        confidence: 0,
        evidence: vec![why.into()],
    }
}

/// The member's opinion on `ctx.symbol` from its finding. `None` for a seat that does not vote
/// (Friday, Sentinel, Pilot, Ledger).
pub fn opinion(finding: &Finding, ctx: &OpinionContext) -> Option<Opinion> {
    let vote = match finding.agent {
        AgentId::Oracle => oracle(&finding.data),
        AgentId::Scout => scout(&finding.data, &ctx.symbol),
        AgentId::Chartist => chartist(&finding.data, ctx),
        AgentId::Athena => athena(&finding.data, ctx),
        AgentId::Atlas => atlas(&finding.data, ctx.kind),
        AgentId::Capitol => capitol(&finding.data, ctx.kind),
        _ => return None,
    };
    Some(Opinion {
        agent: finding.agent,
        symbol: ctx.symbol.clone(),
        stance: if vote.confidence == 0 {
            Stance::Neutral
        } else {
            vote.stance
        },
        confidence: vote.confidence,
        horizon: ctx.horizon.clone(),
        evidence: vote.evidence,
        sources: finding.sources.clone(),
        mapper: MAPPER_VERSION.to_owned(),
    })
}

/// A neutral, zero-confidence opinion for a member whose finding did not come back.
pub fn missing(agent: AgentId, ctx: &OpinionContext, why: &str) -> Opinion {
    Opinion {
        agent,
        symbol: ctx.symbol.clone(),
        stance: Stance::Neutral,
        confidence: 0,
        horizon: ctx.horizon.clone(),
        evidence: vec![why.to_owned()],
        sources: Vec::new(),
        mapper: MAPPER_VERSION.to_owned(),
    }
}

fn oracle(data: &Value) -> Vote {
    let Some(p) = f(data, &["forecast", "p_up"]) else {
        return neutral("Not enough history for a probability.");
    };
    let samples = f(data, &["forecast", "samples"]).unwrap_or(0.0);
    let horizon = f(data, &["forecast", "horizon_bars"]).unwrap_or(0.0);
    let stance = if p >= 0.55 {
        Stance::Bull
    } else if p <= 0.45 {
        Stance::Bear
    } else {
        Stance::Neutral
    };
    let mut evidence = vec![format!(
        "P(up over {horizon} bars) {p:.2} from {samples} past windows."
    )];
    if let (Some(lo), Some(hi)) = (
        f(data, &["forecast", "range_low"]),
        f(data, &["forecast", "range_high"]),
    ) {
        evidence.push(format!("10th to 90th percentile range {lo} to {hi}."));
    }
    Vote {
        stance,
        confidence: if stance == Stance::Neutral {
            0
        } else {
            conf((p - 0.5).abs() * 200.0)
        },
        evidence,
    }
}

fn scout(data: &Value, symbol: &str) -> Vote {
    let Some(m) = data.get("movers").and_then(Value::as_array).and_then(|a| {
        a.iter()
            .find(|m| m.get("symbol").and_then(Value::as_str) == Some(symbol))
    }) else {
        return neutral(format!("{symbol} is not among today's movers."));
    };
    let Some(change) = f(m, &["change_pct"]) else {
        return neutral("No change from the previous close.");
    };
    let stance = match m.get("direction").and_then(Value::as_str) {
        Some("up") => Stance::Bull,
        Some("down") => Stance::Bear,
        _ => return neutral(format!("Flat today ({change:+}%).")),
    };
    let mut evidence = vec![format!("{change:+}% from the previous close.")];
    let base = match f(m, &["range_pct"]) {
        Some(r) if r > 0.0 => {
            let share = (change.abs() / r).min(1.0);
            evidence.push(format!(
                "The move is {:.0}% of today's {r}% range.",
                share * 100.0
            ));
            share * 70.0
        }
        _ => (change.abs() * 20.0).min(50.0),
    };
    let vol = match f(m, &["rel_volume"]) {
        Some(rv) => {
            evidence.push(format!("Volume {rv}× the 20-day average."));
            if rv >= 1.5 {
                20.0
            } else if rv >= 1.0 {
                10.0
            } else {
                0.0
            }
        }
        None => 0.0,
    };
    Vote {
        stance,
        confidence: conf(base + vol),
        evidence,
    }
}

fn chartist(data: &Value, ctx: &OpinionContext) -> Vote {
    let Some(price) = ctx.price else {
        return neutral("No live price to place the levels against.");
    };
    let levels: Vec<f64> = data
        .get("levels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|l| {
                    // Entry markers sit at the price and say nothing about room.
                    !matches!(
                        l.get("kind").and_then(Value::as_str),
                        Some("entry" | "marker")
                    )
                })
                .filter_map(|l| f(l, &["price"]))
                .collect()
        })
        .unwrap_or_default();
    let above = levels
        .iter()
        .filter(|l| **l > price)
        .map(|l| l - price)
        .fold(f64::INFINITY, f64::min);
    let below = levels
        .iter()
        .filter(|l| **l < price)
        .map(|l| price - l)
        .fold(f64::INFINITY, f64::min);
    if !above.is_finite() || !below.is_finite() || above <= 0.0 || below <= 0.0 {
        return neutral("Levels on only one side of the price, so no room ratio.");
    }
    let ratio = above / below;
    let digits = |x: f64| round_to(x, 5).unwrap_or(x);
    let evidence = vec![format!(
        "Nearest level {} above and {} below: room ratio {ratio:.2}.",
        digits(above),
        digits(below)
    )];
    let stance = if ratio >= 1.5 {
        Stance::Bull
    } else if ratio <= 1.0 / 1.5 {
        Stance::Bear
    } else {
        Stance::Neutral
    };
    Vote {
        stance,
        confidence: if stance == Stance::Neutral {
            0
        } else {
            conf(ratio.ln().abs() * 60.0)
        },
        evidence,
    }
}

fn athena(data: &Value, ctx: &OpinionContext) -> Vote {
    let s = data.get("structure").unwrap_or(&Value::Null);
    let price = ctx.price.or_else(|| f(s, &["price"]));
    let atr = ctx.atr.or_else(|| f(s, &["atr"]));
    let (Some(price), Some(atr)) = (price, atr) else {
        return neutral("No price or ATR for the chart structure.");
    };
    if atr <= 0.0 {
        return neutral("No price or ATR for the chart structure.");
    }
    // Distance from the price to the zone, in ATR; 0 when inside.
    let near = |side: &str| -> Option<(f64, f64, f64)> {
        s.get(side)?.as_array()?.iter().find_map(|z| {
            let (lo, hi) = (f(z, &["low"])?, f(z, &["high"])?);
            let d = if price < lo {
                lo - price
            } else if price > hi {
                price - hi
            } else {
                0.0
            } / atr;
            (d <= 0.25).then_some((lo, hi, d))
        })
    };
    let mut evidence = Vec::new();
    if let Some(pe) = f(data, &["fundamentals", "pe_ttm"]) {
        evidence.push(format!("P/E {pe:.1} (shown, not voted on)."));
    }
    let vote = match (near("supply"), near("demand")) {
        (Some((lo, hi, d)), None) => {
            evidence.insert(0, format!("At supply {lo} to {hi} ({d:.2} ATR away)."));
            (Stance::Bear, 60.0 - d * 120.0)
        }
        (None, Some((lo, hi, d))) => {
            evidence.insert(0, format!("At demand {lo} to {hi} ({d:.2} ATR away)."));
            (Stance::Bull, 60.0 - d * 120.0)
        }
        (Some(_), Some(_)) => {
            evidence.insert(0, "Between a supply and a demand zone.".to_owned());
            (Stance::Neutral, 0.0)
        }
        (None, None) => {
            evidence.insert(0, "Not at a supply or demand zone.".to_owned());
            (Stance::Neutral, 0.0)
        }
    };
    Vote {
        stance: vote.0,
        confidence: conf(vote.1),
        evidence,
    }
}

fn atlas(data: &Value, kind: AssetKind) -> Vote {
    if kind == AssetKind::Other {
        return neutral("Macro risk tone is not mapped to this market.");
    }
    let tone = f(data, &["tone", "score"]);
    let idx: Vec<f64> = data
        .get("indices")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|i| f(i, &["change_pct"])).collect())
        .unwrap_or_default();
    let idx_avg = (!idx.is_empty()).then(|| idx.iter().sum::<f64>() / idx.len() as f64);
    let score = match (tone, idx_avg) {
        (Some(t), Some(i)) => t * 0.7 + (i / 1.0).clamp(-1.0, 1.0) * 0.3,
        (Some(t), None) => t,
        (None, Some(i)) => (i / 1.0).clamp(-1.0, 1.0),
        (None, None) => return neutral("No headline tone or index moves."),
    };
    let mut evidence = Vec::new();
    if let Some(t) = tone {
        evidence.push(format!("Headline tone {t:+.2}."));
    }
    if let Some(i) = idx_avg {
        evidence.push(format!("Indices {i:+.2}% on average today."));
    }
    let near_event = data
        .get("high_impact_events")
        .and_then(Value::as_array)
        .is_some_and(|a| {
            a.iter()
                .any(|e| f(e, &["hours_away"]).is_some_and(|h| (0.0..1.0).contains(&h)))
        });
    if near_event {
        evidence.push("A high-impact US event is less than an hour away.".to_owned());
        return Vote {
            stance: Stance::Neutral,
            confidence: 0,
            evidence,
        };
    }
    let stance = if score >= 0.15 {
        Stance::Bull
    } else if score <= -0.15 {
        Stance::Bear
    } else {
        Stance::Neutral
    };
    Vote {
        stance,
        confidence: if stance == Stance::Neutral {
            0
        } else {
            conf((score.abs() * 100.0).min(60.0))
        },
        evidence,
    }
}

fn capitol(data: &Value, kind: AssetKind) -> Vote {
    if kind != AssetKind::Stock {
        return neutral("Insider and congressional trades apply to single stocks only.");
    }
    let ins = data.get("insiders").unwrap_or(&Value::Null);
    let buys = f(ins, &["buys"]).unwrap_or(0.0);
    let sells = f(ins, &["sells"]).unwrap_or(0.0);
    let n = buys + sells;
    if n <= 0.0 {
        return neutral("No insider open-market trades in the window.");
    }
    let ratio = (buys - sells) / n;
    let mut evidence = vec![format!(
        "{buys} insider buys and {sells} sales on the open market."
    )];
    if let Some(net) = f(ins, &["net_shares"]) {
        evidence.push(format!("Net {net:+} shares."));
    }
    let sample = (n / 5.0).min(1.0);
    let (stance, c) = if ratio >= 0.34 {
        (Stance::Bull, ratio * 60.0 * sample)
    } else if ratio <= -0.34 {
        (Stance::Bear, ratio.abs() * 30.0 * sample)
    } else {
        (Stance::Neutral, 0.0)
    };
    Vote {
        stance,
        confidence: conf(c),
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ctx(kind: AssetKind, price: Option<f64>, atr: Option<f64>) -> OpinionContext {
        OpinionContext {
            symbol: "NVDA".into(),
            kind,
            price,
            atr,
            horizon: "4h".into(),
        }
    }

    fn op(agent: AgentId, data: Value, c: &OpinionContext) -> Opinion {
        opinion(&Finding::new(agent, "s", data), c).expect("a member")
    }

    #[test]
    fn oracle_maps_probability_to_stance_and_confidence() {
        let c = ctx(AssetKind::Stock, None, None);
        let o = op(
            AgentId::Oracle,
            json!({"forecast": {"p_up": 0.66, "samples": 480, "horizon_bars": 4}}),
            &c,
        );
        assert_eq!((o.stance, o.confidence), (Stance::Bull, 32));
        let o = op(AgentId::Oracle, json!({"forecast": {"p_up": 0.40}}), &c);
        assert_eq!((o.stance, o.confidence), (Stance::Bear, 20));
        let o = op(AgentId::Oracle, json!({"forecast": {"p_up": 0.52}}), &c);
        assert_eq!((o.stance, o.confidence), (Stance::Neutral, 0));
        let o = op(AgentId::Oracle, json!({"forecast": {"p_up": null}}), &c);
        assert_eq!((o.stance, o.confidence), (Stance::Neutral, 0));
        assert!(o.evidence[0].contains("Not enough history"));
    }

    #[test]
    fn scout_uses_direction_range_share_and_volume() {
        let c = ctx(AssetKind::Stock, None, None);
        let data = json!({"movers": [
            {"symbol": "AAPL", "change_pct": -3.0, "direction": "down"},
            {"symbol": "NVDA", "change_pct": 2.1, "range_pct": 3.0, "rel_volume": 1.8, "direction": "up"}
        ]});
        let o = op(AgentId::Scout, data, &c);
        assert_eq!(o.stance, Stance::Bull);
        assert_eq!(o.confidence, 69); // 0.7 * 70 + 20
        let o = op(AgentId::Scout, json!({"movers": []}), &c);
        assert_eq!((o.stance, o.confidence), (Stance::Neutral, 0));
        let o = op(
            AgentId::Scout,
            json!({"movers": [{"symbol": "NVDA", "change_pct": 0.01, "direction": "flat"}]}),
            &c,
        );
        assert_eq!(o.stance, Stance::Neutral);
    }

    #[test]
    fn chartist_compares_room_above_and_below() {
        let c = ctx(AssetKind::Stock, Some(100.0), None);
        let data = json!({"levels": [
            {"kind": "supply", "price": 106.0}, {"kind": "entry", "price": 100.0},
            {"kind": "demand", "price": 98.0}
        ]});
        let o = op(AgentId::Chartist, data, &c);
        assert_eq!(o.stance, Stance::Bull); // ratio 3
        assert_eq!(o.confidence, 66);
        let data = json!({"levels": [{"kind": "supply", "price": 101.0}, {"kind": "demand", "price": 96.0}]});
        assert_eq!(op(AgentId::Chartist, data, &c).stance, Stance::Bear);
        let one_side = json!({"levels": [{"kind": "supply", "price": 101.0}]});
        assert_eq!(op(AgentId::Chartist, one_side, &c).confidence, 0);
        let no_price = ctx(AssetKind::Stock, None, None);
        assert_eq!(
            op(AgentId::Chartist, json!({"levels": []}), &no_price).stance,
            Stance::Neutral
        );
    }

    #[test]
    fn athena_votes_on_the_zone_the_price_is_at() {
        let c = ctx(AssetKind::Stock, Some(100.0), Some(2.0));
        let at_supply = json!({"structure": {
            "supply": [{"low": 100.2, "high": 101.0, "level": 100.6}],
            "demand": [{"low": 90.0, "high": 91.0, "level": 90.5}]}});
        let o = op(AgentId::Athena, at_supply, &c);
        assert_eq!(o.stance, Stance::Bear);
        assert_eq!(o.confidence, 48); // 60 - 0.1 * 120
        let inside_demand =
            json!({"structure": {"supply": [], "demand": [{"low": 99.0, "high": 100.5}]}});
        let o = op(AgentId::Athena, inside_demand, &c);
        assert_eq!((o.stance, o.confidence), (Stance::Bull, 60));
        let nowhere =
            json!({"structure": {"supply": [{"low": 110.0, "high": 111.0}], "demand": []}});
        assert_eq!(op(AgentId::Athena, nowhere, &c).stance, Stance::Neutral);
    }

    #[test]
    fn atlas_only_votes_on_stocks_and_indices_and_stands_aside_before_events() {
        let data = json!({"tone": {"score": 0.4}, "indices": [{"change_pct": 1.0}], "high_impact_events": []});
        let o = op(
            AgentId::Atlas,
            data.clone(),
            &ctx(AssetKind::Index, None, None),
        );
        assert_eq!(o.stance, Stance::Bull);
        assert_eq!(o.confidence, 58); // 0.4*0.7 + 1*0.3 = 0.58
        let fx = op(AgentId::Atlas, data, &ctx(AssetKind::Other, None, None));
        assert_eq!(fx.stance, Stance::Neutral);
        let event = json!({"tone": {"score": 0.9}, "high_impact_events": [{"hours_away": 0.5}]});
        let o = op(AgentId::Atlas, event, &ctx(AssetKind::Stock, None, None));
        assert_eq!((o.stance, o.confidence), (Stance::Neutral, 0));
    }

    #[test]
    fn capitol_counts_insider_buys_and_halves_sales() {
        let c = ctx(AssetKind::Stock, None, None);
        let o = op(
            AgentId::Capitol,
            json!({"insiders": {"buys": 3, "sells": 0, "net_shares": 41000}}),
            &c,
        );
        assert_eq!((o.stance, o.confidence), (Stance::Bull, 36)); // 1 * 60 * 0.6
        let o = op(
            AgentId::Capitol,
            json!({"insiders": {"buys": 0, "sells": 5}}),
            &c,
        );
        assert_eq!((o.stance, o.confidence), (Stance::Bear, 30));
        let idx = op(
            AgentId::Capitol,
            json!({"insiders": {"buys": 5, "sells": 0}}),
            &ctx(AssetKind::Index, None, None),
        );
        assert_eq!(idx.stance, Stance::Neutral);
    }

    #[test]
    fn non_members_have_no_opinion_and_signed_is_bounded() {
        let c = ctx(AssetKind::Stock, None, None);
        assert!(opinion(&Finding::new(AgentId::Sentinel, "s", json!({})), &c).is_none());
        let o = op(AgentId::Oracle, json!({"forecast": {"p_up": 1.0}}), &c);
        assert_eq!(o.confidence, 100);
        assert!((o.signed() - 1.0).abs() < 1e-9);
        assert_eq!(missing(AgentId::Atlas, &c, "feed down").confidence, 0);
    }
}

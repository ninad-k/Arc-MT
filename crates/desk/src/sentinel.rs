//! Sentinel (Risk officer): approves or vetoes a trade plan. Nothing reaches Pilot without its
//! approval, and a veto is final for that plan.
//!
//! Sentinel checks the Desk rules (spec-nimbus-desk, defaults approved 2026-11-24):
//! 1. a stop is required, on the losing side of the entry;
//! 2. a target, when given, is on the winning side;
//! 3. the risk is at or under the Desk cap (default 0.5% of equity), and the sized volume is
//!    above zero;
//! 4. fewer open Desk positions than the cap (default 2);
//! 5. no new trade in the last minutes before the US close (default 10), nor while the US
//!    market is closed for a symbol that trades in the US session;
//! 6. not while the kill switch is engaged.
//!
//! These come on top of the Arc MT gate (lot size, positions, daily loss, symbols, hours), which
//! still checks the preview Pilot asks for. Approvals are one-time, bound to the exact plan, and
//! expire ([`ApprovalStore`]).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::session::Phase;
use crate::{AgentId, Finding, Source};

/// How long an approval can be handed to Pilot.
pub const APPROVAL_TTL_MS: i64 = 120_000;
const MAX_PLANS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub const fn as_str(self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

/// The Desk's own risk rules. `Default` is the approved defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    pub risk_percent: f64,
    pub max_open_positions: u32,
    pub no_new_trades_before_close_minutes: i64,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            risk_percent: 0.5,
            max_open_positions: 2,
            no_new_trades_before_close_minutes: 10,
        }
    }
}

/// A trade plan, already sized by the API from the account and the symbol's specs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Canonical name (what the trader says).
    pub symbol: String,
    /// The broker's own name (what MT5 trades).
    pub broker_symbol: String,
    pub side: Side,
    /// The live price the plan was made at.
    pub entry: f64,
    pub stop: Option<f64>,
    pub target: Option<f64>,
    /// Lots, from sizing at `risk_percent`. Zero when sizing failed.
    pub volume: f64,
    pub risk_percent: f64,
    pub risk_amount: Option<f64>,
    pub currency: Option<String>,
    /// Why sizing failed, when it did.
    pub sizing_error: Option<String>,
}

/// The state of the account and the market when the plan is reviewed.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    pub open_desk_positions: u32,
    pub phase: Phase,
    pub seconds_to_close: Option<i64>,
    /// The symbol trades in the US equity session (a US stock or index CFD).
    pub us_session: bool,
    pub kill_switch: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Verdict {
    Approved {
        reasons: Vec<String>,
        warnings: Vec<String>,
    },
    Vetoed {
        reasons: Vec<String>,
    },
}

impl Verdict {
    pub fn approved(&self) -> bool {
        matches!(self, Verdict::Approved { .. })
    }
}

/// Review a plan against the rules.
pub fn review(plan: &Plan, ctx: &Context, rules: &Rules) -> Verdict {
    let mut veto = Vec::new();
    let mut ok = Vec::new();
    let mut warnings = Vec::new();

    if ctx.kill_switch {
        veto.push("The kill switch is engaged.".to_owned());
    }
    match plan.stop {
        None => veto.push("A stop is required on every Desk trade.".to_owned()),
        Some(stop) => {
            let right_side = match plan.side {
                Side::Buy => stop < plan.entry,
                Side::Sell => stop > plan.entry,
            };
            if right_side {
                ok.push(format!(
                    "Stop {stop} is on the right side of {}.",
                    plan.entry
                ));
            } else {
                veto.push(format!(
                    "The stop {stop} is on the wrong side of the entry {} for a {}.",
                    plan.entry,
                    plan.side.as_str()
                ));
            }
        }
    }
    if let Some(target) = plan.target {
        let right_side = match plan.side {
            Side::Buy => target > plan.entry,
            Side::Sell => target < plan.entry,
        };
        if !right_side {
            veto.push(format!(
                "The target {target} is on the wrong side of the entry {}.",
                plan.entry
            ));
        } else if let Some(stop) = plan.stop {
            let reward = (target - plan.entry).abs();
            let risk = (plan.entry - stop).abs();
            if risk > 0.0 && reward / risk < 1.0 {
                warnings.push(format!("Reward to risk is {:.2}, under 1.", reward / risk));
            }
        }
    } else {
        warnings.push("No target set.".to_owned());
    }
    if plan.risk_percent > rules.risk_percent + 1e-9 {
        veto.push(format!(
            "Risk {}% is over the Desk cap of {}%.",
            plan.risk_percent, rules.risk_percent
        ));
    } else {
        ok.push(format!(
            "Risk {}% is within the Desk cap of {}%.",
            plan.risk_percent, rules.risk_percent
        ));
    }
    if let Some(e) = &plan.sizing_error {
        veto.push(format!("The position could not be sized: {e}"));
    } else if plan.volume <= 0.0 {
        veto.push("The sized volume is zero.".to_owned());
    }
    if ctx.open_desk_positions >= rules.max_open_positions {
        veto.push(format!(
            "{} Desk positions are open; the cap is {}.",
            ctx.open_desk_positions, rules.max_open_positions
        ));
    } else {
        ok.push(format!(
            "{} of {} Desk positions in use.",
            ctx.open_desk_positions, rules.max_open_positions
        ));
    }
    let buffer = rules.no_new_trades_before_close_minutes * 60;
    if ctx.phase == Phase::Open && ctx.seconds_to_close.is_some_and(|s| s <= buffer) {
        veto.push(format!(
            "No new trades in the last {} minutes before the US close.",
            rules.no_new_trades_before_close_minutes
        ));
    }
    if ctx.us_session && ctx.phase != Phase::Open {
        veto.push(format!(
            "{} trades in the US session, and the market is {}.",
            plan.symbol,
            ctx.phase.label().to_lowercase()
        ));
    }

    if veto.is_empty() {
        Verdict::Approved {
            reasons: ok,
            warnings,
        }
    } else {
        Verdict::Vetoed { reasons: veto }
    }
}

/// A reviewed plan, kept so Pilot can act on it once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanRecord {
    pub id: String,
    pub plan: Plan,
    pub verdict: Verdict,
    pub issued_ms: i64,
    pub expires_ms: i64,
    pub used: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApprovalError {
    #[error("there is no Sentinel plan with id {0}")]
    Unknown(String),
    #[error("Sentinel vetoed this plan, so Pilot cannot act on it")]
    Vetoed,
    #[error("this Sentinel approval has already been used")]
    Used,
    #[error("this Sentinel approval has expired; ask Sentinel again")]
    Expired,
}

/// Reviewed plans by id. Ids come from the caller (random, unguessable).
#[derive(Debug, Default)]
pub struct ApprovalStore {
    plans: Mutex<HashMap<String, PlanRecord>>,
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep a reviewed plan (approved or vetoed).
    pub fn insert(&self, id: String, plan: Plan, verdict: Verdict, now_ms: i64) -> PlanRecord {
        let record = PlanRecord {
            id: id.clone(),
            plan,
            verdict,
            issued_ms: now_ms,
            expires_ms: now_ms + APPROVAL_TTL_MS,
            used: false,
        };
        let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        if plans.len() >= MAX_PLANS {
            let oldest = plans
                .values()
                .min_by_key(|r| r.issued_ms)
                .map(|r| r.id.clone());
            if let Some(o) = oldest {
                plans.remove(&o);
            }
        }
        plans.insert(id, record.clone());
        record
    }

    /// Hand an approved plan to Pilot: once, before it expires.
    pub fn take(&self, id: &str, now_ms: i64) -> Result<PlanRecord, ApprovalError> {
        let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        let record = plans
            .get_mut(id)
            .ok_or_else(|| ApprovalError::Unknown(id.to_owned()))?;
        if !record.verdict.approved() {
            return Err(ApprovalError::Vetoed);
        }
        if record.used {
            return Err(ApprovalError::Used);
        }
        if now_ms >= record.expires_ms {
            return Err(ApprovalError::Expired);
        }
        record.used = true;
        Ok(record.clone())
    }

    /// Put back an approval Pilot took but could not use (the gate refused the preview), so the
    /// trader sees the plan as still open until it expires. Never extends the expiry.
    pub fn restore(&self, id: &str) {
        let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(r) = plans.get_mut(id) {
            r.used = false;
        }
    }

    pub fn get(&self, id: &str) -> Option<PlanRecord> {
        let plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        plans.get(id).cloned()
    }

    /// Plans newest first.
    pub fn recent(&self, limit: usize) -> Vec<PlanRecord> {
        let plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
        let mut all: Vec<PlanRecord> = plans.values().cloned().collect();
        all.sort_by_key(|r| std::cmp::Reverse(r.issued_ms));
        all.truncate(limit);
        all
    }
}

/// Sentinel's finding for a reviewed plan.
pub fn finding(record: &PlanRecord, at: Option<String>) -> Finding {
    let p = &record.plan;
    let summary = match &record.verdict {
        Verdict::Approved { warnings, .. } => format!(
            "Approved: {} {} lots {} at about {}, stop {}, risking {}% ({}). Plan {} is valid for \
             two minutes.{}",
            p.side.as_str(),
            p.volume,
            p.symbol,
            p.entry,
            p.stop.map_or("—".to_owned(), |s| s.to_string()),
            p.risk_percent,
            match (p.risk_amount, &p.currency) {
                (Some(a), Some(c)) => format!("{a} {c}"),
                _ => "—".to_owned(),
            },
            record.id,
            if warnings.is_empty() {
                String::new()
            } else {
                format!(" Warnings: {}", warnings.join(" "))
            }
        ),
        Verdict::Vetoed { reasons } => format!(
            "Vetoed {} {}: {}",
            p.side.as_str(),
            p.symbol,
            reasons.join(" ")
        ),
    };
    Finding::new(
        AgentId::Sentinel,
        summary,
        json!({ "plan_id": record.id, "plan": p, "verdict": record.verdict,
                "expires_ms": record.expires_ms }),
    )
    .with_source(Source::new(
        "mt5",
        "account, positions and symbol specs",
        at,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            symbol: "NVDA".into(),
            broker_symbol: "NVDA.US".into(),
            side: Side::Buy,
            entry: 120.0,
            stop: Some(118.0),
            target: Some(126.0),
            volume: 25.0,
            risk_percent: 0.5,
            risk_amount: Some(50.0),
            currency: Some("USD".into()),
            sizing_error: None,
        }
    }

    fn ctx() -> Context {
        Context {
            open_desk_positions: 0,
            phase: Phase::Open,
            seconds_to_close: Some(3600),
            us_session: true,
            kill_switch: false,
        }
    }

    fn vetoed_for(plan: &Plan, ctx: &Context, needle: &str) {
        match review(plan, ctx, &Rules::default()) {
            Verdict::Vetoed { reasons } => assert!(
                reasons.iter().any(|r| r.contains(needle)),
                "{reasons:?} has no '{needle}'"
            ),
            v => panic!("expected a veto for '{needle}', got {v:?}"),
        }
    }

    #[test]
    fn a_clean_plan_is_approved() {
        let v = review(&plan(), &ctx(), &Rules::default());
        assert!(v.approved(), "{v:?}");
    }

    #[test]
    fn each_rule_vetoes() {
        vetoed_for(
            &Plan {
                stop: None,
                ..plan()
            },
            &ctx(),
            "stop is required",
        );
        vetoed_for(
            &Plan {
                stop: Some(121.0),
                ..plan()
            },
            &ctx(),
            "wrong side of the entry",
        );
        vetoed_for(
            &Plan {
                side: Side::Sell,
                stop: Some(118.0),
                target: Some(110.0),
                ..plan()
            },
            &ctx(),
            "wrong side",
        );
        vetoed_for(
            &Plan {
                target: Some(119.0),
                ..plan()
            },
            &ctx(),
            "target",
        );
        vetoed_for(
            &Plan {
                risk_percent: 1.0,
                ..plan()
            },
            &ctx(),
            "over the Desk cap",
        );
        vetoed_for(
            &Plan {
                volume: 0.0,
                ..plan()
            },
            &ctx(),
            "volume is zero",
        );
        vetoed_for(
            &Plan {
                sizing_error: Some("no tick value".into()),
                ..plan()
            },
            &ctx(),
            "could not be sized",
        );
        vetoed_for(
            &plan(),
            &Context {
                open_desk_positions: 2,
                ..ctx()
            },
            "cap is 2",
        );
        vetoed_for(
            &plan(),
            &Context {
                seconds_to_close: Some(600),
                ..ctx()
            },
            "last 10 minutes",
        );
        vetoed_for(
            &plan(),
            &Context {
                phase: Phase::AfterHours,
                seconds_to_close: None,
                ..ctx()
            },
            "US session",
        );
        vetoed_for(
            &plan(),
            &Context {
                kill_switch: true,
                ..ctx()
            },
            "kill switch",
        );
    }

    #[test]
    fn non_us_symbols_may_trade_outside_the_session() {
        let c = Context {
            phase: Phase::Closed,
            seconds_to_close: None,
            us_session: false,
            ..ctx()
        };
        assert!(review(&plan(), &c, &Rules::default()).approved());
    }

    #[test]
    fn poor_reward_is_a_warning_not_a_veto() {
        match review(
            &Plan {
                target: Some(121.0),
                ..plan()
            },
            &ctx(),
            &Rules::default(),
        ) {
            Verdict::Approved { warnings, .. } => {
                assert!(warnings.iter().any(|w| w.contains("under 1")))
            }
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn approvals_are_one_time_and_expire() {
        let store = ApprovalStore::new();
        let ok = review(&plan(), &ctx(), &Rules::default());
        store.insert("a".into(), plan(), ok.clone(), 1_000);
        assert!(store.take("a", 2_000).is_ok());
        assert_eq!(store.take("a", 2_000), Err(ApprovalError::Used));
        store.restore("a");
        assert!(store.take("a", 2_000).is_ok());

        store.insert("b".into(), plan(), ok, 1_000);
        assert_eq!(
            store.take("b", 1_000 + APPROVAL_TTL_MS),
            Err(ApprovalError::Expired)
        );

        let veto = review(
            &Plan {
                stop: None,
                ..plan()
            },
            &ctx(),
            &Rules::default(),
        );
        store.insert("c".into(), plan(), veto, 1_000);
        assert_eq!(store.take("c", 1_500), Err(ApprovalError::Vetoed));
        assert_eq!(
            store.take("zzz", 1_500),
            Err(ApprovalError::Unknown("zzz".into()))
        );
        assert_eq!(store.recent(10).len(), 3);
    }

    #[test]
    fn finding_states_the_plan() {
        let store = ApprovalStore::new();
        let r = store.insert(
            "p1".into(),
            plan(),
            review(&plan(), &ctx(), &Rules::default()),
            0,
        );
        let f = finding(&r, None);
        assert!(f.summary.starts_with("Approved: buy 25 lots NVDA"));
        assert_eq!(f.data["plan_id"], "p1");
        assert_eq!(f.data["verdict"]["decision"], "approved");
    }
}

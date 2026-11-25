//! The council (spec-architecture-page): weighs the members' opinions into one score, builds the
//! bull and bear cases, and runs the risk seat's checks that need no plan. Everything here is
//! code, so a council verdict is free and repeatable.
//!
//! - Score = Σ wᵢ · sign(stanceᵢ) · confidenceᵢ / 100, from −1 to +1. A member that did not vote
//!   counts as neutral; its weight is not given to the others, so missing data lowers the score.
//! - The council passes when at least `min_non_neutral` members took a side, |score| is at least
//!   `min_score`, and the risk seat raised nothing.
//!
//! [`StopCode`] names the first stage that stopped a decision, for the whole pipeline.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::AgentId;
use crate::opinion::{MEMBERS, Opinion, Stance};

/// Member weights. They sum to 1 after [`Weights::normalised`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Weights(pub BTreeMap<AgentId, f64>);

impl Default for Weights {
    /// Equal weights over the six members.
    fn default() -> Self {
        Self(
            MEMBERS
                .iter()
                .map(|a| (*a, 1.0 / MEMBERS.len() as f64))
                .collect(),
        )
    }
}

impl Weights {
    pub fn get(&self, agent: AgentId) -> f64 {
        self.0.get(&agent).copied().unwrap_or(0.0)
    }

    /// Only the members, negatives and non-finite values dropped, scaled to sum to 1. Equal
    /// weights when nothing usable is left.
    pub fn normalised(&self) -> Weights {
        let kept: BTreeMap<AgentId, f64> = MEMBERS
            .iter()
            .map(|a| (*a, self.get(*a)))
            .map(|(a, w)| (a, if w.is_finite() && w > 0.0 { w } else { 0.0 }))
            .collect();
        let sum: f64 = kept.values().sum();
        if sum <= 0.0 {
            return Weights::default();
        }
        Weights(kept.into_iter().map(|(a, w)| (a, w / sum)).collect())
    }
}

/// The council's thresholds. `Default` is the approved defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouncilRules {
    pub min_score: f64,
    pub min_non_neutral: usize,
}

impl Default for CouncilRules {
    fn default() -> Self {
        Self {
            min_score: 0.30,
            min_non_neutral: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contribution {
    pub agent: AgentId,
    pub weight: f64,
    pub stance: Stance,
    pub confidence: u8,
    /// weight × signed vote.
    pub value: f64,
}

/// One side's case: its members' contributions, strongest first, with their best evidence line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub total: f64,
    pub points: Vec<CasePoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CasePoint {
    pub agent: AgentId,
    pub value: f64,
    pub evidence: String,
}

/// What the risk seat raised before any plan exists: `(rule id, plain reason)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskFlag {
    pub rule: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouncilVerdict {
    pub score: f64,
    pub threshold: f64,
    pub non_neutral: usize,
    pub contributions: Vec<Contribution>,
    pub bull: Case,
    pub bear: Case,
    pub risk: Vec<RiskFlag>,
    pub passed: bool,
    /// The side the council favours when it passed.
    pub side: Option<Stance>,
    /// Why it did not pass.
    pub stop: Option<(StopCode, String)>,
}

/// Weigh the opinions. `risk` is the risk seat's pre-checks ([`crate::sentinel::precheck`]).
pub fn deliberate(
    opinions: &[Opinion],
    weights: &Weights,
    rules: &CouncilRules,
    risk: Vec<RiskFlag>,
) -> CouncilVerdict {
    let w = weights.normalised();
    let mut contributions: Vec<Contribution> = MEMBERS
        .iter()
        .map(|agent| {
            let op = opinions.iter().find(|o| o.agent == *agent);
            let weight = w.get(*agent);
            let (stance, confidence, signed) = op
                .map(|o| (o.stance, o.confidence, o.signed()))
                .unwrap_or((Stance::Neutral, 0, 0.0));
            Contribution {
                agent: *agent,
                weight,
                stance,
                confidence,
                value: weight * signed,
            }
        })
        .collect();
    contributions.sort_by(|a, b| b.value.abs().total_cmp(&a.value.abs()));
    let score = contributions.iter().map(|c| c.value).sum::<f64>();
    let score = (score * 1000.0).round() / 1000.0;
    let non_neutral = contributions
        .iter()
        .filter(|c| c.stance != Stance::Neutral && c.confidence > 0)
        .count();
    let case = |want: Stance| -> Case {
        let points: Vec<CasePoint> = contributions
            .iter()
            .filter(|c| c.stance == want && c.value != 0.0)
            .map(|c| CasePoint {
                agent: c.agent,
                value: c.value,
                evidence: opinions
                    .iter()
                    .find(|o| o.agent == c.agent)
                    .and_then(|o| o.evidence.first().cloned())
                    .unwrap_or_default(),
            })
            .collect();
        Case {
            total: points.iter().map(|p| p.value).sum(),
            points,
        }
    };
    let (bull, bear) = (case(Stance::Bull), case(Stance::Bear));

    let stop = if let Some(flag) = risk.first() {
        // The kill switch is its own code, as at Pilot; the other flags are Sentinel's rules.
        let code = if flag.rule == "kill_switch" {
            StopCode::KillSwitch
        } else {
            StopCode::SentinelVeto(flag.rule.clone())
        };
        Some((code, flag.reason.clone()))
    } else if non_neutral < rules.min_non_neutral {
        Some((
            StopCode::MembersSplit,
            format!(
                "{non_neutral} of {} members took a side; at least {} must.",
                MEMBERS.len(),
                rules.min_non_neutral
            ),
        ))
    } else if score.abs() + 1e-9 < rules.min_score {
        Some((
            StopCode::CouncilBelowThreshold,
            format!(
                "Score {score:+.2} is inside the ±{:.2} threshold.",
                rules.min_score
            ),
        ))
    } else {
        None
    };
    let passed = stop.is_none();
    CouncilVerdict {
        score,
        threshold: rules.min_score,
        non_neutral,
        contributions,
        bull,
        bear,
        risk,
        passed,
        side: passed.then_some(if score > 0.0 {
            Stance::Bull
        } else {
            Stance::Bear
        }),
        stop,
    }
}

/// The stage of the pipeline a decision reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Signal,
    Members,
    Council,
    Master,
    Execution,
}

impl Stage {
    pub const ALL: [Stage; 5] = [
        Stage::Signal,
        Stage::Members,
        Stage::Council,
        Stage::Master,
        Stage::Execution,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Stage::Signal => "signal",
            Stage::Members => "members",
            Stage::Council => "council",
            Stage::Master => "master",
            Stage::Execution => "execution",
        }
    }
}

/// Why a decision did not end in a filled trade: the first stage that stopped it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum StopCode {
    MarketClosed,
    PreCloseCutoff,
    EventBlackout,
    BudgetExhausted,
    NoData,
    MembersSplit,
    StaleData,
    CouncilBelowThreshold,
    SentinelVeto(String),
    NotAsked,
    FridayStoodAside,
    PlanInvalid,
    GateLimit(String),
    KillSwitch,
    AccountMismatch,
    IntentMismatch,
    PreviewExpired,
    UserDeclined,
    TradeFailed(String),
    OutcomeUnknown,
    Error(String),
}

impl StopCode {
    /// The stage the code belongs to.
    pub fn stage(&self) -> Stage {
        use StopCode::*;
        match self {
            MarketClosed | PreCloseCutoff | EventBlackout | BudgetExhausted | NoData => {
                Stage::Signal
            }
            MembersSplit | StaleData => Stage::Members,
            CouncilBelowThreshold | SentinelVeto(_) => Stage::Council,
            NotAsked | FridayStoodAside | PlanInvalid => Stage::Master,
            GateLimit(_) | KillSwitch | AccountMismatch | IntentMismatch | PreviewExpired
            | UserDeclined | TradeFailed(_) | OutcomeUnknown | Error(_) => Stage::Execution,
        }
    }

    /// A short plain-English label for the funnel and the banner.
    pub fn label(&self) -> String {
        use StopCode::*;
        match self {
            MarketClosed => "Market closed".into(),
            PreCloseCutoff => "Too close to the US close".into(),
            EventBlackout => "High-impact event nearby".into(),
            BudgetExhausted => "Monthly budget used up".into(),
            NoData => "No market data".into(),
            MembersSplit => "Too few members took a side".into(),
            StaleData => "Data too old".into(),
            CouncilBelowThreshold => "Council score below threshold".into(),
            SentinelVeto(r) => format!("Sentinel veto: {}", r.replace('_', " ")),
            NotAsked => "Waiting for Ask Friday".into(),
            FridayStoodAside => "Friday stood aside".into(),
            PlanInvalid => "Friday's plan was invalid".into(),
            GateLimit(r) => format!("Gate limit: {}", r.replace('_', " ")),
            KillSwitch => "Kill switch engaged".into(),
            AccountMismatch => "MT5 is not on the master account".into(),
            IntentMismatch => "Preview did not match the request".into(),
            PreviewExpired => "Preview expired before confirm".into(),
            UserDeclined => "You declined".into(),
            TradeFailed(r) => format!("Broker rejected ({r})"),
            OutcomeUnknown => "Sent, outcome unknown".into(),
            Error(e) => format!("Error: {e}"),
        }
    }

    /// Whether a decision with this code may still go on (it is waiting, not stopped).
    pub fn is_waiting(&self) -> bool {
        matches!(self, StopCode::NotAsked)
    }
}

impl fmt::Display for StopCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use StopCode::*;
        let s = match self {
            MarketClosed => "market_closed",
            PreCloseCutoff => "pre_close_cutoff",
            EventBlackout => "event_blackout",
            BudgetExhausted => "budget_exhausted",
            NoData => "no_data",
            MembersSplit => "members_split",
            StaleData => "stale_data",
            CouncilBelowThreshold => "council_below_threshold",
            SentinelVeto(r) => return write!(f, "sentinel_veto:{r}"),
            NotAsked => "not_asked",
            FridayStoodAside => "friday_stood_aside",
            PlanInvalid => "plan_invalid",
            GateLimit(r) => return write!(f, "gate_limit:{r}"),
            KillSwitch => "kill_switch",
            AccountMismatch => "account_mismatch",
            IntentMismatch => "intent_mismatch",
            PreviewExpired => "preview_expired",
            UserDeclined => "user_declined",
            TradeFailed(r) => return write!(f, "trade_failed:{r}"),
            OutcomeUnknown => "outcome_unknown",
            Error(e) => return write!(f, "error:{e}"),
        };
        f.write_str(s)
    }
}

impl From<StopCode> for String {
    fn from(c: StopCode) -> String {
        c.to_string()
    }
}

impl TryFrom<String> for StopCode {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        use StopCode::*;
        if let Some((head, rest)) = s.split_once(':') {
            let rest = rest.to_owned();
            return match head {
                "sentinel_veto" => Ok(SentinelVeto(rest)),
                "gate_limit" => Ok(GateLimit(rest)),
                "trade_failed" => Ok(TradeFailed(rest)),
                "error" => Ok(Error(rest)),
                _ => Err(format!("unknown stop code {s}")),
            };
        }
        Ok(match s.as_str() {
            "market_closed" => MarketClosed,
            "pre_close_cutoff" => PreCloseCutoff,
            "event_blackout" => EventBlackout,
            "budget_exhausted" => BudgetExhausted,
            "no_data" => NoData,
            "members_split" => MembersSplit,
            "stale_data" => StaleData,
            "council_below_threshold" => CouncilBelowThreshold,
            "not_asked" => NotAsked,
            "friday_stood_aside" => FridayStoodAside,
            "plan_invalid" => PlanInvalid,
            "kill_switch" => KillSwitch,
            "account_mismatch" => AccountMismatch,
            "intent_mismatch" => IntentMismatch,
            "preview_expired" => PreviewExpired,
            "user_declined" => UserDeclined,
            "outcome_unknown" => OutcomeUnknown,
            _ => return Err(format!("unknown stop code {s}")),
        })
    }
}

/// The council's risk-seat flags from Sentinel's pre-checks.
pub fn risk_flags(checks: Vec<(&'static str, String)>) -> Vec<RiskFlag> {
    checks
        .into_iter()
        .map(|(rule, reason)| RiskFlag {
            rule: rule.to_owned(),
            reason,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(agent: AgentId, stance: Stance, confidence: u8) -> Opinion {
        Opinion {
            agent,
            symbol: "NVDA".into(),
            stance,
            confidence,
            horizon: "4h".into(),
            evidence: vec![format!("{agent} says {}", stance.as_str())],
            sources: vec![],
            mapper: "v1".into(),
        }
    }

    fn weights(pairs: &[(AgentId, f64)]) -> Weights {
        Weights(pairs.iter().copied().collect())
    }

    #[test]
    fn score_is_the_weighted_sum_and_passes_over_threshold() {
        let w = weights(&[
            (AgentId::Atlas, 0.16),
            (AgentId::Capitol, 0.15),
            (AgentId::Scout, 0.20),
            (AgentId::Athena, 0.15),
            (AgentId::Chartist, 0.17),
            (AgentId::Oracle, 0.17),
        ]);
        let ops = [
            op(AgentId::Scout, Stance::Bull, 50),
            op(AgentId::Oracle, Stance::Bull, 40),
            op(AgentId::Chartist, Stance::Bull, 36),
            op(AgentId::Athena, Stance::Bear, 34),
            op(AgentId::Atlas, Stance::Neutral, 0),
        ];
        let v = deliberate(&ops, &w, &CouncilRules::default(), vec![]);
        assert!((v.score - 0.178).abs() < 1e-9, "{}", v.score);
        assert_eq!(v.non_neutral, 4);
        assert!(!v.passed);
        assert_eq!(v.stop.as_ref().unwrap().0, StopCode::CouncilBelowThreshold);
        assert_eq!(v.bull.points.len(), 3);
        assert_eq!(v.bull.points[0].agent, AgentId::Scout);
        assert!((v.bear.total + 0.051).abs() < 1e-9);
        let lower = CouncilRules {
            min_score: 0.15,
            ..CouncilRules::default()
        };
        let v = deliberate(&ops, &w, &lower, vec![]);
        assert!(v.passed);
        assert_eq!(v.side, Some(Stance::Bull));
    }

    #[test]
    fn too_few_sides_stop_before_the_score() {
        let ops = [
            op(AgentId::Scout, Stance::Bull, 100),
            op(AgentId::Oracle, Stance::Bull, 100),
        ];
        let v = deliberate(&ops, &Weights::default(), &CouncilRules::default(), vec![]);
        assert_eq!(v.stop.unwrap().0, StopCode::MembersSplit);
    }

    #[test]
    fn the_risk_seat_vetoes_first() {
        let ops: Vec<Opinion> = MEMBERS.iter().map(|a| op(*a, Stance::Bear, 90)).collect();
        let risk = risk_flags(vec![("max_open_positions", "2 open".into())]);
        let v = deliberate(&ops, &Weights::default(), &CouncilRules::default(), risk);
        assert!(!v.passed);
        assert_eq!(
            v.stop.unwrap().0,
            StopCode::SentinelVeto("max_open_positions".into())
        );
        let v = deliberate(&ops, &Weights::default(), &CouncilRules::default(), vec![]);
        assert_eq!(v.side, Some(Stance::Bear));
        assert!((v.score + 0.9).abs() < 1e-9);
    }

    #[test]
    fn weights_normalise_and_fall_back_to_equal() {
        let w = weights(&[
            (AgentId::Oracle, 2.0),
            (AgentId::Scout, 2.0),
            (AgentId::Friday, 5.0),
        ]);
        let n = w.normalised();
        assert!((n.get(AgentId::Oracle) - 0.5).abs() < 1e-9);
        assert_eq!(n.get(AgentId::Friday), 0.0);
        let bad = weights(&[(AgentId::Oracle, f64::NAN), (AgentId::Scout, -1.0)]);
        assert!((bad.normalised().get(AgentId::Atlas) - 1.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn stop_codes_round_trip_and_know_their_stage() {
        let all = [
            StopCode::MarketClosed,
            StopCode::PreCloseCutoff,
            StopCode::EventBlackout,
            StopCode::BudgetExhausted,
            StopCode::NoData,
            StopCode::MembersSplit,
            StopCode::StaleData,
            StopCode::CouncilBelowThreshold,
            StopCode::SentinelVeto("pre_close".into()),
            StopCode::NotAsked,
            StopCode::FridayStoodAside,
            StopCode::PlanInvalid,
            StopCode::GateLimit("max_lot".into()),
            StopCode::KillSwitch,
            StopCode::AccountMismatch,
            StopCode::IntentMismatch,
            StopCode::PreviewExpired,
            StopCode::UserDeclined,
            StopCode::TradeFailed("10019".into()),
            StopCode::OutcomeUnknown,
            StopCode::Error("bridge".into()),
        ];
        for code in all {
            let s = code.to_string();
            assert_eq!(StopCode::try_from(s.clone()).unwrap(), code, "{s}");
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(serde_json::from_str::<StopCode>(&json).unwrap(), code);
            assert!(!code.label().is_empty());
        }
        assert_eq!(StopCode::NotAsked.stage(), Stage::Master);
        assert_eq!(StopCode::GateLimit("x".into()).stage(), Stage::Execution);
        assert!(StopCode::try_from("nope".to_owned()).is_err());
    }
}

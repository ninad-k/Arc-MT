//! Decision records for the architecture page (spec-architecture-page).
//!
//! One row per decision: the signal that started it, every member's opinion, the council's
//! verdict, Friday's proposal with the exact prompt, Sentinel's plan, Pilot's preview, and what
//! happened at execution. A decision that did not trade carries exactly one stop code, the first
//! stage that stopped it. The audit table stays the record of every action; these rows are a
//! readable index over it, updated as a decision moves on.
//!
//! Also here: the versioned council weights and Desk guards (`config_versions`). The newest
//! version is live; nothing changes them but an explicit apply or rollback.

use std::collections::BTreeMap;
use std::sync::RwLock;

use chrono::{SecondsFormat, Utc};
use jarvis_desk::council::{CouncilRules, CouncilVerdict, Stage, StopCode, Weights};
use jarvis_desk::opinion::Opinion;
use jarvis_desk::sentinel::{PlanRecord, Rules, Verdict};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::trading::{TradeResultDto, TradingError};

/// A previewed decision with no execution after this long is resolved from the audit log.
pub const PREVIEW_SETTLE_MS: i64 = 30_000;
/// A Sentinel approval Pilot never used is stale after this long.
pub const PLAN_SETTLE_MS: i64 = jarvis_desk::sentinel::APPROVAL_TTL_MS + 10_000;
/// An "Ask Friday" turn with no plan after this long was cut off (a restart).
pub const ASK_SETTLE_MS: i64 = 10 * 60 * 1000;
/// A decision waiting for Ask Friday ends after this long (the council verdict is stale).
pub const WAITING_TTL_MS: i64 = 2 * 3600 * 1000;
const MAX_MASTER_CHARS: usize = 60_000;

/// The tunable guards: Sentinel's rules and the council's thresholds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guards {
    pub risk_percent: f64,
    pub max_open_positions: u32,
    pub no_new_trades_before_close_minutes: i64,
    pub min_reward_risk: f64,
    pub min_council_score: f64,
    pub min_non_neutral: usize,
}

impl Guards {
    pub fn new(rules: &Rules, council: &CouncilRules) -> Self {
        Self {
            risk_percent: rules.risk_percent,
            max_open_positions: rules.max_open_positions,
            no_new_trades_before_close_minutes: rules.no_new_trades_before_close_minutes,
            min_reward_risk: rules.min_reward_risk,
            min_council_score: council.min_score,
            min_non_neutral: council.min_non_neutral,
        }
    }

    pub fn rules(&self) -> Rules {
        Rules {
            risk_percent: self.risk_percent,
            max_open_positions: self.max_open_positions,
            no_new_trades_before_close_minutes: self.no_new_trades_before_close_minutes,
            min_reward_risk: self.min_reward_risk,
        }
    }

    pub fn council(&self) -> CouncilRules {
        CouncilRules {
            min_score: self.min_council_score,
            min_non_neutral: self.min_non_neutral,
        }
    }

    /// The bounds any version must stay within, whoever proposes it.
    pub fn validate(&self) -> Result<(), String> {
        let ok = self.risk_percent.is_finite()
            && (0.05..=2.0).contains(&self.risk_percent)
            && (1..=10).contains(&self.max_open_positions)
            && (0..=390).contains(&self.no_new_trades_before_close_minutes)
            && self.min_reward_risk.is_finite()
            && (0.5..=5.0).contains(&self.min_reward_risk)
            && self.min_council_score.is_finite()
            && (0.05..=0.9).contains(&self.min_council_score)
            && (1..=6).contains(&self.min_non_neutral);
        if ok {
            Ok(())
        } else {
            Err("a guard is outside its allowed range".into())
        }
    }
}

/// The live weights and guards, and the version they come from (`None`: the config defaults).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveConfig {
    pub version: Option<i64>,
    pub weights: Weights,
    pub guards: Guards,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VersionRow {
    pub version: i64,
    pub created_at: String,
    pub weights: Weights,
    pub guards: Guards,
    pub reason: String,
    pub proposal_id: Option<String>,
    pub actor: Option<String>,
}

/// A decision whose opinions wait for the price to judge them.
#[derive(Debug, Clone, PartialEq)]
pub struct Unresolved {
    pub id: String,
    pub symbol: String,
    pub broker_symbol: Option<String>,
    pub created_ms: i64,
    pub price: Option<f64>,
    pub atr: Option<f64>,
}

/// What started a decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    /// `scan`, `question` or `ask`.
    pub kind: String,
    pub detail: Option<String>,
}

/// A decision as the scan or a Desk turn creates it.
#[derive(Debug, Clone)]
pub struct NewDecision {
    pub session_id: Option<Uuid>,
    pub symbol: String,
    pub broker_symbol: Option<String>,
    pub venue: String,
    pub trigger: Trigger,
    pub price: Option<f64>,
    pub atr: Option<f64>,
    /// Each member's opinion with the finding data it came from.
    pub opinions: Vec<(Opinion, Option<Value>)>,
    pub council: Option<CouncilVerdict>,
    /// A stop before the members ran (market closed, no data): no council then.
    pub early_stop: Option<(Stage, StopCode, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionSummary {
    pub id: String,
    pub created_at: String,
    pub created_ms: i64,
    pub symbol: String,
    pub venue: String,
    pub trigger: Trigger,
    pub stage: String,
    pub status: String,
    pub stop_code: Option<String>,
    pub stop_label: Option<String>,
    pub stop_detail: Option<String>,
    pub score: Option<f64>,
    pub threshold: Option<f64>,
    pub council_passed: bool,
    pub side: Option<String>,
    pub has_plan: bool,
    pub has_preview: bool,
    pub result_r: Option<f64>,
    /// 0 signal, 1 members ran, 2 council passed, 3 plan, 4 preview, 5 traded.
    pub step: u8,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpinionRow {
    pub agent: String,
    pub stance: String,
    pub confidence: i64,
    pub weight: f64,
    pub contribution: f64,
    pub horizon: String,
    pub evidence: Vec<String>,
    pub sources: Value,
    pub data: Option<Value>,
    pub mapper: String,
    pub outcome: Option<String>,
    pub move_atr: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionDetail {
    #[serde(flatten)]
    pub summary: DecisionSummary,
    pub session_id: Option<String>,
    pub broker_symbol: Option<String>,
    pub price: Option<f64>,
    pub atr: Option<f64>,
    pub council: Option<Value>,
    pub weights_version: Option<i64>,
    pub master: Option<Value>,
    pub plan_id: Option<String>,
    pub plan: Option<Value>,
    pub verdict: Option<Value>,
    pub preview_id: Option<String>,
    pub tickets: Vec<u64>,
    pub pnl: Option<f64>,
    pub closed_at: Option<String>,
    pub opinions: Vec<OpinionRow>,
    pub review: Option<Review>,
}

/// The trader's manual analysis of a decision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub setup: Option<String>,
    #[serde(default)]
    pub mistakes: Vec<String>,
    pub regime: Option<String>,
    pub rating: Option<u8>,
    pub take_again: Option<bool>,
    pub notes: Option<String>,
    #[serde(default, skip_deserializing)]
    pub updated_at: Option<String>,
}

impl Review {
    pub fn validate(&self) -> Result<(), String> {
        let tag_ok = |t: &str| !t.trim().is_empty() && t.chars().count() <= 64;
        if self.setup.as_deref().is_some_and(|s| !tag_ok(s))
            || self.regime.as_deref().is_some_and(|s| !tag_ok(s))
        {
            return Err("setup and regime are 1 to 64 characters".into());
        }
        if self.mistakes.len() > 10 || self.mistakes.iter().any(|m| !tag_ok(m)) {
            return Err("up to 10 mistake tags of 1 to 64 characters".into());
        }
        if self.rating.is_some_and(|r| !(1..=5).contains(&r)) {
            return Err("rating is 1 to 5".into());
        }
        if self
            .notes
            .as_deref()
            .is_some_and(|n| n.chars().count() > 4000)
        {
            return Err("notes are at most 4000 characters".into());
        }
        Ok(())
    }
}

/// Filters for the decisions list.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DecisionFilter {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub symbol: Option<String>,
    pub status: Option<String>,
    pub stage: Option<String>,
    /// A stop code or its prefix (`sentinel_veto` matches `sentinel_veto:pre_close`).
    pub stop: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FunnelStep {
    pub step: u8,
    pub label: &'static str,
    /// Decisions that reached this step.
    pub count: u64,
    /// Decisions that stopped right after this step, by stop code.
    pub dropped: BTreeMap<String, u64>,
}

pub const STEP_LABELS: [&str; 6] = [
    "Signals",
    "Members ran",
    "Council passed",
    "Master proposed",
    "Previewed",
    "Executed",
];

pub struct Decisions {
    pool: SqlitePool,
    defaults: LiveConfig,
    live: RwLock<LiveConfig>,
    /// Held from reading the live config to saving the next version (apply, rollback), so two
    /// changes never build on the same version and undo each other.
    config_lock: tokio::sync::Mutex<()>,
}

/// Every member's weight stays within [0.5/N, 2/N] (a little slack for rounding).
pub fn validate_weights(w: &Weights) -> Result<(), String> {
    let n = jarvis_desk::opinion::MEMBERS.len() as f64;
    let (lo, hi) = (0.5 / n - 0.002, 2.0 / n + 0.002);
    let w = w.normalised();
    if jarvis_desk::opinion::MEMBERS
        .iter()
        .all(|a| (lo..=hi).contains(&w.get(*a)))
    {
        Ok(())
    } else {
        Err(format!(
            "every weight must be between {:.3} and {:.3}",
            0.5 / n,
            2.0 / n
        ))
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn iso_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn truncate(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        s
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

fn step_of(status: &str, preview: bool, plan: bool, passed: bool, stage: &str) -> u8 {
    if status == "traded" {
        5
    } else if preview {
        4
    } else if plan {
        3
    } else if passed {
        2
    } else if stage != Stage::Signal.as_str() {
        1
    } else {
        0
    }
}

/// The stop code for a refusal from the trading service.
pub fn stop_for_trading_error(e: &TradingError) -> StopCode {
    use jarvis_gate::GateError;
    match e {
        TradingError::Gate(GateError::LimitExceeded { rule, .. }) => {
            StopCode::GateLimit((*rule).to_owned())
        }
        TradingError::Gate(GateError::KillSwitchEngaged) | TradingError::KillSwitch => {
            StopCode::KillSwitch
        }
        TradingError::Gate(GateError::Ambiguous { .. }) => StopCode::GateLimit("ambiguous".into()),
        TradingError::Gate(GateError::Invalid(_)) => StopCode::PlanInvalid,
        TradingError::Gate(GateError::NothingToDo(_)) => {
            StopCode::GateLimit("nothing_to_do".into())
        }
        TradingError::IntentMismatch(_) => StopCode::IntentMismatch,
        TradingError::Token(_) => StopCode::PreviewExpired,
        TradingError::ModeMismatch => StopCode::GateLimit("mode_mismatch".into()),
        TradingError::BrokerChanged => StopCode::AccountMismatch,
        TradingError::Bridge(_) => StopCode::Error("bridge".into()),
        TradingError::Snapshot(_) => StopCode::NoData,
        TradingError::UserIsAnswering => StopCode::Error("user_is_answering".into()),
        TradingError::RecheckRejected(_) => StopCode::GateLimit("recheck".into()),
        TradingError::RateLimited { .. } => StopCode::Error("rate_limited".into()),
        TradingError::TradeFailed(m) => {
            StopCode::TradeFailed(retcode_in(m).unwrap_or_else(|| "rejected".into()))
        }
        TradingError::OutcomeUnknown(_) => StopCode::OutcomeUnknown,
        TradingError::Internal(_) => StopCode::Error("internal".into()),
    }
}

/// The first number of four or more digits in a message (MT5 retcodes are 10004 and up).
fn retcode_in(m: &str) -> Option<String> {
    m.split(|c: char| !c.is_ascii_digit())
        .find(|t| t.len() >= 4)
        .map(str::to_owned)
}

/// The stop code for a Sentinel verdict that vetoed.
pub fn stop_for_verdict(v: &Verdict) -> Option<(StopCode, String)> {
    match v {
        Verdict::Approved { .. } => None,
        Verdict::Vetoed { reasons, rules } => Some((
            StopCode::SentinelVeto(rules.first().cloned().unwrap_or_else(|| "veto".into())),
            reasons.join(" "),
        )),
    }
}

impl Decisions {
    pub fn new(pool: SqlitePool, defaults: LiveConfig) -> Self {
        Self {
            pool,
            live: RwLock::new(defaults.clone()),
            defaults,
            config_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Load the newest saved version, if any, as the live config.
    pub async fn load(&self) -> anyhow::Result<()> {
        if let Some(v) = self.versions(1).await?.into_iter().next() {
            *self.live.write().unwrap_or_else(|p| p.into_inner()) = LiveConfig {
                version: Some(v.version),
                weights: v.weights,
                guards: v.guards,
            };
        }
        Ok(())
    }

    /// Hold this from reading [`Decisions::live`] to [`Decisions::save_version`] when the new
    /// version is built from the live one.
    pub async fn lock_config(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.config_lock.lock().await
    }

    pub fn live(&self) -> LiveConfig {
        self.live.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn defaults(&self) -> &LiveConfig {
        &self.defaults
    }

    /// Save weights and guards as the new live version.
    pub async fn save_version(
        &self,
        weights: &Weights,
        guards: &Guards,
        reason: &str,
        proposal_id: Option<&str>,
        actor: Option<&str>,
    ) -> anyhow::Result<i64> {
        guards.validate().map_err(anyhow::Error::msg)?;
        validate_weights(weights).map_err(anyhow::Error::msg)?;
        let weights = weights.normalised();
        let row = sqlx::query(
            "INSERT INTO config_versions (created_at, weights_json, guards_json, reason, proposal_id, actor)
             VALUES (?, ?, ?, ?, ?, ?) RETURNING version",
        )
        .bind(iso_now())
        .bind(serde_json::to_string(&weights)?)
        .bind(serde_json::to_string(guards)?)
        .bind(reason)
        .bind(proposal_id)
        .bind(actor)
        .fetch_one(&self.pool)
        .await?;
        let version: i64 = row.try_get("version")?;
        *self.live.write().unwrap_or_else(|p| p.into_inner()) = LiveConfig {
            version: Some(version),
            weights,
            guards: guards.clone(),
        };
        Ok(version)
    }

    pub async fn versions(&self, limit: u32) -> anyhow::Result<Vec<VersionRow>> {
        let rows = sqlx::query(
            "SELECT version, created_at, weights_json, guards_json, reason, proposal_id, actor
             FROM config_versions ORDER BY version DESC LIMIT ?",
        )
        .bind(i64::from(limit.clamp(1, 200)))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(VersionRow {
                    version: r.try_get("version")?,
                    created_at: r.try_get("created_at")?,
                    weights: serde_json::from_str(&r.try_get::<String, _>("weights_json")?)?,
                    guards: serde_json::from_str(&r.try_get::<String, _>("guards_json")?)?,
                    reason: r.try_get("reason")?,
                    proposal_id: r.try_get("proposal_id")?,
                    actor: r.try_get("actor")?,
                })
            })
            .collect()
    }

    /// Make an older version live again, as a new version.
    pub async fn rollback(&self, version: i64, actor: Option<&str>) -> anyhow::Result<i64> {
        let row =
            sqlx::query("SELECT weights_json, guards_json FROM config_versions WHERE version = ?")
                .bind(version)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| anyhow::anyhow!("there is no version {version}"))?;
        let weights: Weights = serde_json::from_str(&row.try_get::<String, _>("weights_json")?)?;
        let guards: Guards = serde_json::from_str(&row.try_get::<String, _>("guards_json")?)?;
        self.save_version(
            &weights,
            &guards,
            &format!("Rolled back to v{version}"),
            None,
            actor,
        )
        .await
    }

    /// Record a new decision with its opinions. Returns its id.
    pub async fn create(&self, d: NewDecision) -> anyhow::Result<String> {
        let id = Uuid::new_v4().simple().to_string();
        let ms = now_ms();
        let live = self.live();
        let (stage, status, stop): (Stage, &str, Option<(StopCode, String)>) =
            match (&d.early_stop, &d.council) {
                (Some((stage, code, detail)), _) => {
                    (*stage, "stopped", Some((code.clone(), detail.clone())))
                }
                (None, Some(c)) if c.passed => (
                    Stage::Master,
                    "waiting",
                    Some((
                        StopCode::NotAsked,
                        "The council passed. Ask Friday for a plan.".to_owned(),
                    )),
                ),
                (None, Some(c)) => {
                    let (code, detail) = c.stop.clone().unwrap_or((
                        StopCode::CouncilBelowThreshold,
                        "The council did not pass.".to_owned(),
                    ));
                    let stage = if code == StopCode::MembersSplit {
                        Stage::Members
                    } else {
                        Stage::Council
                    };
                    (stage, "stopped", Some((code, detail)))
                }
                (None, None) => (Stage::Members, "open", None),
            };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO decisions (id, created_at, created_ms, updated_ms, session_id, symbol,
                broker_symbol, venue, trigger_kind, trigger_detail, stage, status, stop_code,
                stop_detail, price, atr, score, threshold, council_passed, side, council_json,
                weights_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(iso_now())
        .bind(ms)
        .bind(ms)
        .bind(d.session_id.map(|s| s.to_string()))
        .bind(&d.symbol)
        .bind(&d.broker_symbol)
        .bind(&d.venue)
        .bind(&d.trigger.kind)
        .bind(&d.trigger.detail)
        .bind(stage.as_str())
        .bind(status)
        .bind(stop.as_ref().map(|s| s.0.to_string()))
        .bind(stop.as_ref().map(|s| s.1.clone()))
        .bind(d.price)
        .bind(d.atr)
        .bind(d.council.as_ref().map(|c| c.score))
        .bind(d.council.as_ref().map(|c| c.threshold))
        .bind(d.council.as_ref().is_some_and(|c| c.passed))
        .bind(
            d.council
                .as_ref()
                .and_then(|c| c.side)
                .map(|s| s.as_str().to_owned()),
        )
        .bind(d.council.as_ref().map(serde_json::to_string).transpose()?)
        .bind(live.version)
        .execute(&mut *tx)
        .await?;
        for (op, data) in &d.opinions {
            let (weight, contribution) = d
                .council
                .as_ref()
                .and_then(|c| c.contributions.iter().find(|x| x.agent == op.agent))
                .map_or((live.weights.normalised().get(op.agent), 0.0), |x| {
                    (x.weight, x.value)
                });
            sqlx::query(
                "INSERT OR REPLACE INTO decision_opinions (decision_id, agent, stance, confidence,
                    weight, contribution, horizon, evidence_json, sources_json, data_json, mapper)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(op.agent.as_str())
            .bind(op.stance.as_str())
            .bind(i64::from(op.confidence))
            .bind(weight)
            .bind(contribution)
            .bind(&op.horizon)
            .bind(serde_json::to_string(&op.evidence)?)
            .bind(serde_json::to_string(&op.sources)?)
            .bind(data.as_ref().map(serde_json::to_string).transpose()?)
            .bind(&op.mapper)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(id)
    }

    /// Friday's proposal for a decision: the master record and, when there is one, Sentinel's
    /// plan. No plan means Friday stood aside; a vetoed plan stops the decision.
    pub async fn record_master(
        &self,
        id: &str,
        master: &Value,
        plan: Option<&PlanRecord>,
        reply: &str,
    ) -> anyhow::Result<()> {
        let master_text = fit_master(master).to_string();
        let (status, stage, stop) = match plan {
            None => (
                "stopped",
                Stage::Master,
                Some((
                    StopCode::FridayStoodAside,
                    truncate(reply.trim().to_owned(), 400),
                )),
            ),
            Some(p) => match stop_for_verdict(&p.verdict) {
                Some(s) => ("stopped", Stage::Master, Some(s)),
                None => ("open", Stage::Master, None),
            },
        };
        sqlx::query(
            "UPDATE decisions SET master_json = ?, plan_id = ?, plan_json = ?, verdict_json = ?,
                status = ?, stage = ?, stop_code = ?, stop_detail = ?, updated_ms = ?
             WHERE id = ? AND status != 'traded'",
        )
        .bind(master_text)
        .bind(plan.map(|p| p.id.clone()))
        .bind(plan.map(|p| serde_json::to_string(&p.plan)).transpose()?)
        .bind(
            plan.map(|p| serde_json::to_string(&p.verdict))
                .transpose()?,
        )
        .bind(status)
        .bind(stage.as_str())
        .bind(stop.as_ref().map(|s| s.0.to_string()))
        .bind(stop.as_ref().map(|s| s.1.clone()))
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stop one decision at a stage (keeps a traded decision as it is).
    pub async fn stop(
        &self,
        id: &str,
        stage: Stage,
        code: &StopCode,
        detail: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE decisions SET status = 'stopped', stage = ?, stop_code = ?, stop_detail = ?,
                updated_ms = ? WHERE id = ? AND status != 'traded'",
        )
        .bind(stage.as_str())
        .bind(code.to_string())
        .bind(truncate(detail.to_owned(), 400))
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// An "Ask Friday" turn that failed or was cut off: the decision waits again.
    pub async fn reset_waiting(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE decisions SET status = 'waiting', stop_code = 'not_asked',
                stop_detail = 'Friday did not answer; ask again.', updated_ms = ?
             WHERE id = ? AND status = 'open' AND plan_id IS NULL AND preview_id IS NULL",
        )
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stop a decision found by its Sentinel plan.
    pub async fn stop_by_plan(
        &self,
        plan_id: &str,
        code: &StopCode,
        detail: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE decisions SET status = 'stopped', stop_code = ?, stop_detail = ?, updated_ms = ?
             WHERE plan_id = ? AND status = 'open' AND preview_id IS NULL",
        )
        .bind(code.to_string())
        .bind(truncate(detail.to_owned(), 400))
        .bind(now_ms())
        .bind(plan_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stop a decision found by its preview (a refusal at confirm).
    pub async fn stop_by_preview(
        &self,
        preview_id: Uuid,
        code: &StopCode,
        detail: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE decisions SET status = 'stopped', stop_code = ?, stop_detail = ?, updated_ms = ?
             WHERE preview_id = ? AND status != 'traded'",
        )
        .bind(code.to_string())
        .bind(truncate(detail.to_owned(), 400))
        .bind(now_ms())
        .bind(preview_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Pilot previewed the plan: the decision is at execution, waiting for the confirm.
    pub async fn link_preview(&self, plan_id: &str, preview_id: Uuid) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE decisions SET preview_id = ?, stage = 'execution', status = 'open',
                stop_code = NULL, stop_detail = NULL, updated_ms = ?
             WHERE plan_id = ? AND status != 'traded'",
        )
        .bind(preview_id.to_string())
        .bind(now_ms())
        .bind(plan_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The confirmed preview was sent: traded when every write succeeded.
    pub async fn executed(
        &self,
        preview_id: Uuid,
        results: &[TradeResultDto],
    ) -> anyhow::Result<()> {
        let tickets: Vec<u64> = results
            .iter()
            .filter(|r| r.ok)
            .filter_map(|r| r.ticket.or(r.order))
            .collect();
        let failed = results.iter().find(|r| !r.ok);
        let (status, stop) = match failed {
            None if !results.is_empty() => ("traded", None),
            None => (
                "stopped",
                Some((StopCode::Error("empty_result".into()), String::new())),
            ),
            Some(r) => (
                "stopped",
                Some((
                    StopCode::TradeFailed(
                        r.retcode
                            .map_or_else(|| "rejected".to_owned(), |c| c.to_string()),
                    ),
                    r.retcode_description
                        .clone()
                        .or_else(|| r.error.clone())
                        .unwrap_or_default(),
                )),
            ),
        };
        sqlx::query(
            "UPDATE decisions SET status = ?, stage = 'execution', stop_code = ?, stop_detail = ?,
                tickets_json = ?, updated_ms = ?
             WHERE preview_id = ?",
        )
        .bind(status)
        .bind(stop.as_ref().map(|s| s.0.to_string()))
        .bind(stop.as_ref().map(|s| s.1.clone()))
        .bind(serde_json::to_string(&tickets)?)
        .bind(now_ms())
        .bind(preview_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Settle decisions left open: a preview nobody confirmed (or that the re-check refused), and
    /// a Sentinel approval Pilot never used. Reads the audit log for what happened.
    pub async fn settle(&self, now_ms: i64) -> anyhow::Result<u64> {
        // An Ask Friday turn cut off by a restart: no plan after ten minutes, so ask again.
        let reset = sqlx::query(
            "UPDATE decisions SET status = 'waiting', stop_code = 'not_asked',
                stop_detail = 'Friday did not answer; ask again.', updated_ms = ?
             WHERE status = 'open' AND trigger_kind = 'ask' AND plan_id IS NULL
               AND preview_id IS NULL AND updated_ms < ?",
        )
        .bind(now_ms)
        .bind(now_ms - ASK_SETTLE_MS)
        .execute(&self.pool)
        .await?
        .rows_affected();
        // A council verdict goes stale: a decision nobody asked about within two hours ends.
        let stale = sqlx::query(
            "UPDATE decisions SET status = 'stopped',
                stop_detail = 'Not asked within two hours; the council verdict is stale.',
                updated_ms = ?
             WHERE status = 'waiting' AND created_ms < ?",
        )
        .bind(now_ms)
        .bind(now_ms - WAITING_TTL_MS)
        .execute(&self.pool)
        .await?
        .rows_affected();
        let rows = sqlx::query(
            "SELECT id, preview_id, plan_id, updated_ms FROM decisions
             WHERE status = 'open' AND (preview_id IS NOT NULL OR plan_id IS NOT NULL)",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut settled = reset + stale;
        for r in rows {
            let id: String = r.try_get("id")?;
            let updated: i64 = r.try_get("updated_ms")?;
            let preview: Option<String> = r.try_get("preview_id")?;
            let fate = match &preview {
                Some(p) => {
                    if now_ms - updated < PREVIEW_SETTLE_MS {
                        continue;
                    }
                    self.preview_fate(p).await?
                }
                None => {
                    if now_ms - updated < PLAN_SETTLE_MS {
                        continue;
                    }
                    Some((
                        StopCode::PreviewExpired,
                        "Sentinel's approval expired before Pilot previewed it.".to_owned(),
                    ))
                }
            };
            if let Some((code, detail)) = fate {
                sqlx::query(
                    "UPDATE decisions SET status = 'stopped', stop_code = ?, stop_detail = ?,
                        updated_ms = ? WHERE id = ? AND status = 'open'",
                )
                .bind(code.to_string())
                .bind(detail)
                .bind(now_ms)
                .bind(&id)
                .execute(&self.pool)
                .await?;
            } else {
                sqlx::query("UPDATE decisions SET status = 'traded', updated_ms = ? WHERE id = ?")
                    .bind(now_ms)
                    .bind(&id)
                    .execute(&self.pool)
                    .await?;
            }
            settled += 1;
        }
        Ok(settled)
    }

    /// What became of a preview, from the audit rows that name it: `None` when it traded.
    async fn preview_fate(&self, preview_id: &str) -> anyhow::Result<Option<(StopCode, String)>> {
        let rows = sqlx::query(
            "SELECT kind, payload FROM audit
             WHERE kind IN ('trade_result', 'gate_rejected')
               AND json_extract(payload, '$.preview_id') = ?
             ORDER BY id",
        )
        .bind(preview_id)
        .fetch_all(&self.pool)
        .await?;
        for r in &rows {
            let kind: String = r.try_get("kind")?;
            let payload: Value =
                serde_json::from_str(&r.try_get::<String, _>("payload")?).unwrap_or(Value::Null);
            match kind.as_str() {
                "gate_rejected" => {
                    let rule = payload["rule"]
                        .as_str()
                        .or_else(|| payload["reason"].as_str())
                        .unwrap_or("recheck");
                    let rule = rule.strip_prefix("limit:").unwrap_or(rule);
                    // The same codes as a refusal at Pilot (`stop_for_trading_error`).
                    let code = match rule {
                        "kill_switch" => StopCode::KillSwitch,
                        "broker_changed" => StopCode::AccountMismatch,
                        "intent_mismatch" => StopCode::IntentMismatch,
                        other => StopCode::GateLimit(other.to_owned()),
                    };
                    return Ok(Some((
                        code,
                        payload["message"].as_str().unwrap_or("").to_owned(),
                    )));
                }
                "trade_result" if payload["outcome"] == "unknown" => {
                    return Ok(Some((
                        StopCode::OutcomeUnknown,
                        "Sent with no reply; never re-sent.".into(),
                    )));
                }
                "trade_result" if payload["ok"] == false => {
                    let code = payload["retcode"]
                        .as_i64()
                        .map_or_else(|| "rejected".to_owned(), |c| c.to_string());
                    return Ok(Some((
                        StopCode::TradeFailed(code),
                        payload["retcode_description"]
                            .as_str()
                            .unwrap_or("")
                            .to_owned(),
                    )));
                }
                "trade_result" => return Ok(None),
                _ => {}
            }
        }
        Ok(Some((
            StopCode::PreviewExpired,
            "Not confirmed within 15 seconds.".into(),
        )))
    }

    pub async fn list(&self, f: &DecisionFilter) -> anyhow::Result<Vec<DecisionSummary>> {
        let mut sql = String::from(
            "SELECT d.id, d.created_at, d.created_ms, d.symbol, d.venue, d.trigger_kind,
                d.trigger_detail, d.stage, d.status, d.stop_code, d.stop_detail, d.score,
                d.threshold, d.council_passed, d.side, d.plan_id, d.preview_id, d.result_r,
                r.setup, r.mistakes_json, r.regime
             FROM decisions d LEFT JOIN decision_reviews r ON r.decision_id = d.id WHERE 1 = 1",
        );
        if f.from_ms.is_some() {
            sql.push_str(" AND d.created_ms >= ?");
        }
        if f.to_ms.is_some() {
            sql.push_str(" AND d.created_ms < ?");
        }
        if f.symbol.is_some() {
            sql.push_str(" AND d.symbol = ?");
        }
        if f.status.is_some() {
            sql.push_str(" AND d.status = ?");
        }
        if f.stage.is_some() {
            sql.push_str(" AND d.stage = ?");
        }
        if f.stop.is_some() {
            sql.push_str(" AND (d.stop_code = ? OR d.stop_code LIKE ? || ':%')");
        }
        sql.push_str(" ORDER BY d.created_ms DESC LIMIT ?");
        let mut q = sqlx::query(&sql);
        if let Some(v) = f.from_ms {
            q = q.bind(v);
        }
        if let Some(v) = f.to_ms {
            q = q.bind(v);
        }
        if let Some(v) = &f.symbol {
            q = q.bind(v.to_ascii_uppercase());
        }
        if let Some(v) = &f.status {
            q = q.bind(v.clone());
        }
        if let Some(v) = &f.stage {
            q = q.bind(v.clone());
        }
        if let Some(v) = &f.stop {
            q = q.bind(v.clone()).bind(v.clone());
        }
        q = q.bind(i64::from(f.limit.unwrap_or(200).clamp(1, 2000)));
        let rows = q.fetch_all(&self.pool).await?;
        rows.iter().map(summary_from).collect()
    }

    pub async fn get(&self, id: &str) -> anyhow::Result<Option<DecisionDetail>> {
        let Some(r) = sqlx::query(
            "SELECT d.*, r.setup, r.mistakes_json, r.regime FROM decisions d
             LEFT JOIN decision_reviews r ON r.decision_id = d.id WHERE d.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let summary = summary_from(&r)?;
        let json_col = |name: &str| -> anyhow::Result<Option<Value>> {
            Ok(r.try_get::<Option<String>, _>(name)?
                .and_then(|s| serde_json::from_str(&s).ok()))
        };
        let opinions = sqlx::query(
            "SELECT agent, stance, confidence, weight, contribution, horizon, evidence_json,
                sources_json, data_json, mapper, outcome, move_atr
             FROM decision_opinions WHERE decision_id = ? ORDER BY abs(contribution) DESC, agent",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|o| {
            Ok(OpinionRow {
                agent: o.try_get("agent")?,
                stance: o.try_get("stance")?,
                confidence: o.try_get("confidence")?,
                weight: o.try_get("weight")?,
                contribution: o.try_get("contribution")?,
                horizon: o.try_get("horizon")?,
                evidence: serde_json::from_str(&o.try_get::<String, _>("evidence_json")?)
                    .unwrap_or_default(),
                sources: serde_json::from_str(&o.try_get::<String, _>("sources_json")?)
                    .unwrap_or(Value::Null),
                data: o
                    .try_get::<Option<String>, _>("data_json")?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                mapper: o.try_get("mapper")?,
                outcome: o.try_get("outcome")?,
                move_atr: o.try_get("move_atr")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(DecisionDetail {
            session_id: r.try_get("session_id")?,
            broker_symbol: r.try_get("broker_symbol")?,
            price: r.try_get("price")?,
            atr: r.try_get("atr")?,
            council: json_col("council_json")?,
            weights_version: r.try_get("weights_version")?,
            master: json_col("master_json")?,
            plan_id: r.try_get("plan_id")?,
            plan: json_col("plan_json")?,
            verdict: json_col("verdict_json")?,
            preview_id: r.try_get("preview_id")?,
            tickets: json_col("tickets_json")?
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default(),
            pnl: r.try_get("pnl")?,
            closed_at: r.try_get("closed_at")?,
            opinions,
            review: self.review(id).await?,
            summary,
        }))
    }

    /// Counts per step and what stopped decisions after each step, for decisions created in
    /// `[from_ms, to_ms)`.
    pub async fn funnel(&self, from_ms: i64, to_ms: i64) -> anyhow::Result<Vec<FunnelStep>> {
        let rows = sqlx::query(
            "SELECT status, stage, council_passed, plan_id IS NOT NULL AS has_plan,
                preview_id IS NOT NULL AS has_preview, stop_code
             FROM decisions WHERE created_ms >= ? AND created_ms < ?",
        )
        .bind(from_ms)
        .bind(to_ms)
        .fetch_all(&self.pool)
        .await?;
        let mut steps: Vec<FunnelStep> = STEP_LABELS
            .iter()
            .enumerate()
            .map(|(i, l)| FunnelStep {
                step: i as u8,
                label: l,
                ..FunnelStep::default()
            })
            .collect();
        for r in rows {
            let status: String = r.try_get("status")?;
            let stage: String = r.try_get("stage")?;
            let step = step_of(
                &status,
                r.try_get("has_preview")?,
                r.try_get("has_plan")?,
                r.try_get("council_passed")?,
                &stage,
            );
            for s in steps.iter_mut().take(step as usize + 1) {
                s.count += 1;
            }
            if status != "traded" {
                let code: Option<String> = r.try_get("stop_code")?;
                let key = code.unwrap_or_else(|| "in_progress".into());
                *steps[step as usize].dropped.entry(key).or_default() += 1;
            }
        }
        Ok(steps)
    }

    pub async fn review(&self, id: &str) -> anyhow::Result<Option<Review>> {
        let Some(r) = sqlx::query(
            "SELECT setup, mistakes_json, regime, rating, take_again, notes, updated_at
             FROM decision_reviews WHERE decision_id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        Ok(Some(Review {
            setup: r.try_get("setup")?,
            mistakes: serde_json::from_str(&r.try_get::<String, _>("mistakes_json")?)
                .unwrap_or_default(),
            regime: r.try_get("regime")?,
            rating: r
                .try_get::<Option<i64>, _>("rating")?
                .and_then(|v| u8::try_from(v).ok()),
            take_again: r.try_get::<Option<bool>, _>("take_again")?,
            notes: r.try_get("notes")?,
            updated_at: r.try_get("updated_at")?,
        }))
    }

    /// Save the trader's review. `Ok(false)` when there is no such decision.
    pub async fn save_review(&self, id: &str, review: &Review) -> anyhow::Result<bool> {
        let exists = sqlx::query("SELECT 1 FROM decisions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .is_some();
        if !exists {
            return Ok(false);
        }
        let clean = |s: &Option<String>| s.as_ref().map(|v| v.trim().to_owned());
        sqlx::query(
            "INSERT INTO decision_reviews (decision_id, setup, mistakes_json, regime, rating,
                take_again, notes, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (decision_id) DO UPDATE SET setup = excluded.setup,
                mistakes_json = excluded.mistakes_json, regime = excluded.regime,
                rating = excluded.rating, take_again = excluded.take_again,
                notes = excluded.notes, updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(clean(&review.setup))
        .bind(serde_json::to_string(
            &review
                .mistakes
                .iter()
                .map(|m| m.trim().to_owned())
                .collect::<Vec<_>>(),
        )?)
        .bind(clean(&review.regime))
        .bind(review.rating.map(i64::from))
        .bind(review.take_again)
        .bind(&review.notes)
        .bind(iso_now())
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    /// The newest decision for a symbol that is waiting for Ask Friday.
    pub async fn waiting(&self, id: &str) -> anyhow::Result<Option<(String, String)>> {
        Ok(
            sqlx::query("SELECT symbol, venue FROM decisions WHERE id = ? AND status = 'waiting'")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .map(|r| (r.get("symbol"), r.get("venue"))),
        )
    }

    /// Mark a waiting decision as being asked, so a second click does not ask twice.
    pub async fn mark_asked(&self, id: &str, session_id: Uuid) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "UPDATE decisions SET status = 'open', stop_code = NULL, stop_detail = NULL,
                trigger_kind = 'ask', session_id = ?, updated_ms = ?
             WHERE id = ? AND status = 'waiting'",
        )
        .bind(session_id.to_string())
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Each member's newest opinion and how many it has given, for the map.
    pub async fn member_latest(&self) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query(
            "SELECT agent, stance, confidence, weight, contribution, evidence_json, symbol,
                created_at, decision_id, n
             FROM (SELECT o.agent, o.stance, o.confidence, o.weight, o.contribution,
                       o.evidence_json, d.symbol, d.created_at, d.id AS decision_id,
                       count(*) OVER (PARTITION BY o.agent) AS n,
                       row_number() OVER (PARTITION BY o.agent
                                          ORDER BY d.created_ms DESC) AS rn
                   FROM decision_opinions o JOIN decisions d ON d.id = o.decision_id)
             WHERE rn = 1",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(json!({
                    "agent": r.try_get::<String, _>("agent")?,
                    "stance": r.try_get::<String, _>("stance")?,
                    "confidence": r.try_get::<i64, _>("confidence")?,
                    "weight": r.try_get::<f64, _>("weight")?,
                    "contribution": r.try_get::<f64, _>("contribution")?,
                    "evidence": serde_json::from_str::<Value>(&r.try_get::<String, _>("evidence_json")?)
                        .unwrap_or(Value::Null),
                    "symbol": r.try_get::<String, _>("symbol")?,
                    "at": r.try_get::<String, _>("created_at")?,
                    "decision_id": r.try_get::<String, _>("decision_id")?,
                    "opinions": r.try_get::<i64, _>("n")?,
                }))
            })
            .collect()
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Decisions older than the horizon whose opinions are not resolved yet, oldest first.
    pub async fn unresolved(&self, before_ms: i64, limit: u32) -> anyhow::Result<Vec<Unresolved>> {
        let rows = sqlx::query(
            "SELECT d.id, d.symbol, d.broker_symbol, d.created_ms, d.price, d.atr FROM decisions d
             WHERE d.created_ms < ? AND EXISTS (SELECT 1 FROM decision_opinions o
                   WHERE o.decision_id = d.id AND o.resolved_ms IS NULL)
             ORDER BY d.created_ms LIMIT ?",
        )
        .bind(before_ms)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(Unresolved {
                    id: r.try_get("id")?,
                    symbol: r.try_get("symbol")?,
                    broker_symbol: r.try_get("broker_symbol")?,
                    created_ms: r.try_get("created_ms")?,
                    price: r.try_get("price")?,
                    atr: r.try_get("atr")?,
                })
            })
            .collect()
    }

    /// Resolve every opinion of a decision with the move that followed (`None`: it cannot be
    /// judged any more, e.g. no bars that far back).
    pub async fn resolve(
        &self,
        id: &str,
        move_atr: Option<f64>,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let rows = sqlx::query("SELECT agent, stance FROM decision_opinions WHERE decision_id = ?")
            .bind(id)
            .fetch_all(&self.pool)
            .await?;
        for r in rows {
            let agent: String = r.try_get("agent")?;
            let stance: String = r.try_get("stance")?;
            let outcome = move_atr.map(|m| {
                let st = match stance.as_str() {
                    "bull" => jarvis_desk::opinion::Stance::Bull,
                    "bear" => jarvis_desk::opinion::Stance::Bear,
                    _ => jarvis_desk::opinion::Stance::Neutral,
                };
                jarvis_desk::learning::judge(st, m).as_str()
            });
            sqlx::query(
                "UPDATE decision_opinions SET outcome = ?, move_atr = ?, resolved_ms = ?
                 WHERE decision_id = ? AND agent = ?",
            )
            .bind(outcome.unwrap_or("unresolvable"))
            .bind(move_atr)
            .bind(now_ms)
            .bind(id)
            .bind(agent)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Every resolved opinion with a move, for the learning statistics.
    pub async fn resolved_opinions(&self) -> anyhow::Result<Vec<jarvis_desk::learning::Resolved>> {
        let rows = sqlx::query(
            "SELECT o.decision_id, o.agent, o.stance, o.confidence, o.move_atr, d.status, d.created_ms
             FROM decision_opinions o JOIN decisions d ON d.id = o.decision_id
             WHERE o.move_atr IS NOT NULL ORDER BY d.created_ms",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for r in rows {
            let Some(agent) = jarvis_desk::AgentId::parse(&r.try_get::<String, _>("agent")?) else {
                continue;
            };
            let stance = match r.try_get::<String, _>("stance")?.as_str() {
                "bull" => jarvis_desk::opinion::Stance::Bull,
                "bear" => jarvis_desk::opinion::Stance::Bear,
                _ => jarvis_desk::opinion::Stance::Neutral,
            };
            out.push(jarvis_desk::learning::Resolved {
                decision_id: r.try_get("decision_id")?,
                agent,
                stance,
                confidence: u8::try_from(r.try_get::<i64, _>("confidence")?).unwrap_or(0),
                move_atr: r.try_get("move_atr")?,
                counterfactual: r.try_get::<String, _>("status")? != "traded",
                at_ms: r.try_get("created_ms")?,
            });
        }
        Ok(out)
    }

    /// How each decision ended, with its move when resolved, for the guard statistics.
    pub async fn decision_ends(
        &self,
    ) -> anyhow::Result<(Vec<jarvis_desk::learning::DecisionEnd>, usize)> {
        let rows = sqlx::query(
            "SELECT d.stop_code, d.status, d.score,
                (SELECT o.move_atr FROM decision_opinions o WHERE o.decision_id = d.id
                 AND o.move_atr IS NOT NULL LIMIT 1) AS move_atr
             FROM decisions d WHERE d.status IN ('stopped', 'traded')",
        )
        .fetch_all(&self.pool)
        .await?;
        let n = rows.len();
        let ends = rows
            .into_iter()
            .map(|r| {
                Ok(jarvis_desk::learning::DecisionEnd {
                    stop_code: r.try_get("stop_code")?,
                    traded: r.try_get::<String, _>("status")? == "traded",
                    score: r.try_get("score")?,
                    move_atr: r.try_get("move_atr")?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok((ends, n))
    }

    /// When the live version was saved, in ms (None for the config defaults).
    pub async fn live_since_ms(&self) -> anyhow::Result<Option<i64>> {
        let Some(v) = self.live().version else {
            return Ok(None);
        };
        let row = sqlx::query("SELECT created_at FROM config_versions WHERE version = ?")
            .bind(v)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .and_then(|r| r.try_get::<String, _>("created_at").ok())
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
            .map(|t| t.timestamp_millis()))
    }

    /// Serialise a list as CSV (one row per decision).
    pub fn to_csv(rows: &[DecisionSummary]) -> String {
        fn cell(s: &str) -> String {
            // Quote every cell; neutralise spreadsheet formulas.
            let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) && s.parse::<f64>().is_err()
            {
                format!("'{s}")
            } else {
                s.to_owned()
            };
            format!("\"{}\"", s.replace('"', "\"\""))
        }
        let mut out = String::from(
            "id,created_at,symbol,venue,trigger,stage,status,stop_code,stop_detail,score,threshold,side,result_r,tags\n",
        );
        for r in rows {
            let fields = [
                r.id.clone(),
                r.created_at.clone(),
                r.symbol.clone(),
                r.venue.clone(),
                r.trigger.kind.clone(),
                r.stage.clone(),
                r.status.clone(),
                r.stop_code.clone().unwrap_or_default(),
                r.stop_detail.clone().unwrap_or_default(),
                r.score.map(|v| format!("{v:.3}")).unwrap_or_default(),
                r.threshold.map(|v| format!("{v:.2}")).unwrap_or_default(),
                r.side.clone().unwrap_or_default(),
                r.result_r.map(|v| format!("{v:.2}")).unwrap_or_default(),
                r.tags.join("; "),
            ];
            out.push_str(&fields.iter().map(|f| cell(f)).collect::<Vec<_>>().join(","));
            out.push('\n');
        }
        out
    }
}

fn summary_from(r: &sqlx::sqlite::SqliteRow) -> anyhow::Result<DecisionSummary> {
    let status: String = r.try_get("status")?;
    let stage: String = r.try_get("stage")?;
    let plan: Option<String> = r.try_get("plan_id")?;
    let preview: Option<String> = r.try_get("preview_id")?;
    let passed: bool = r.try_get("council_passed")?;
    let stop_code: Option<String> = r.try_get("stop_code")?;
    let mut tags = Vec::new();
    if let Some(s) = r.try_get::<Option<String>, _>("setup")? {
        tags.push(s);
    }
    if let Some(m) = r.try_get::<Option<String>, _>("mistakes_json")? {
        tags.extend(serde_json::from_str::<Vec<String>>(&m).unwrap_or_default());
    }
    if let Some(s) = r.try_get::<Option<String>, _>("regime")? {
        tags.push(s);
    }
    Ok(DecisionSummary {
        id: r.try_get("id")?,
        created_at: r.try_get("created_at")?,
        created_ms: r.try_get("created_ms")?,
        symbol: r.try_get("symbol")?,
        venue: r.try_get("venue")?,
        trigger: Trigger {
            kind: r.try_get("trigger_kind")?,
            detail: r.try_get("trigger_detail")?,
        },
        step: step_of(&status, preview.is_some(), plan.is_some(), passed, &stage),
        stop_label: stop_code
            .clone()
            .and_then(|c| StopCode::try_from(c).ok())
            .map(|c| c.label()),
        stop_code,
        stop_detail: r.try_get("stop_detail")?,
        score: r.try_get("score")?,
        threshold: r.try_get("threshold")?,
        council_passed: passed,
        side: r.try_get("side")?,
        has_plan: plan.is_some(),
        has_preview: preview.is_some(),
        result_r: r.try_get("result_r")?,
        stage,
        status,
        tags,
    })
}

/// Keep a master record under [`MAX_MASTER_CHARS`] and valid JSON: long message contents are
/// cut first, then the oldest messages are dropped (the count is kept in `messages_dropped`).
fn fit_master(master: &Value) -> Value {
    let mut m = master.clone();
    if m.to_string().len() <= MAX_MASTER_CHARS {
        return m;
    }
    if let Some(list) = m.get_mut("messages").and_then(Value::as_array_mut) {
        for msg in list.iter_mut() {
            for key in ["text", "content", "data_base64"] {
                if let Some(Value::String(t)) = msg.get(key)
                    && t.chars().count() > 2_000
                {
                    let cut = truncate(t.clone(), 2_000);
                    msg[key] = Value::String(cut);
                }
            }
        }
    }
    let mut dropped = 0;
    while m.to_string().len() > MAX_MASTER_CHARS {
        let Some(list) = m.get_mut("messages").and_then(Value::as_array_mut) else {
            break;
        };
        if list.len() <= 2 {
            break;
        }
        list.remove(0);
        dropped += 1;
    }
    m["messages_dropped"] = json!(dropped);
    if m.to_string().len() > MAX_MASTER_CHARS {
        m["system"] = Value::String("(left out: too long; see system_hash)".into());
    }
    m
}

/// The master record of one Friday turn: what was sent and what came back.
pub fn master_record(
    provider: &str,
    model: &str,
    system: &str,
    messages: &Value,
    reply: &str,
    usage: Value,
) -> Value {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(system.as_bytes());
    let hash: String = hash.iter().take(6).map(|b| format!("{b:02x}")).collect();
    json!({
        "provider": provider,
        "model": model,
        "system_hash": hash,
        "system": system,
        "messages": messages,
        "reply": reply,
        "usage": usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_master_records_stay_valid_json() {
        let msgs: Vec<Value> = (0..200)
            .map(|i| json!({ "role": "user", "text": format!("{i} {}", "x".repeat(3_000)) }))
            .collect();
        let m = master_record("fake", "m", "sys", &json!(msgs), "ok", Value::Null);
        let fitted = fit_master(&m);
        let text = fitted.to_string();
        assert!(text.len() <= MAX_MASTER_CHARS, "{}", text.len());
        assert!(fitted["messages_dropped"].as_u64().unwrap() > 0);
        assert_eq!(fitted["reply"], "ok");
        let small = master_record("fake", "m", "sys", &json!([]), "ok", Value::Null);
        assert_eq!(fit_master(&small), small);
    }

    #[test]
    fn steps_follow_the_furthest_point() {
        assert_eq!(step_of("traded", true, true, true, "execution"), 5);
        assert_eq!(step_of("stopped", true, true, true, "execution"), 4);
        assert_eq!(step_of("stopped", false, true, true, "master"), 3);
        assert_eq!(step_of("waiting", false, false, true, "master"), 2);
        assert_eq!(step_of("stopped", false, false, false, "council"), 1);
        assert_eq!(step_of("stopped", false, false, false, "signal"), 0);
    }

    #[test]
    fn retcodes_are_found_in_messages() {
        assert_eq!(
            retcode_in("MT5 rejected the order: 10019 no money").as_deref(),
            Some("10019")
        );
        assert_eq!(retcode_in("no code"), None);
    }

    #[test]
    fn reviews_are_bounded() {
        assert!(Review::default().validate().is_ok());
        let bad = Review {
            rating: Some(6),
            ..Review::default()
        };
        assert!(bad.validate().is_err());
        let long = Review {
            mistakes: vec!["x".repeat(65)],
            ..Review::default()
        };
        assert!(long.validate().is_err());
    }

    #[test]
    fn guards_have_hard_bounds() {
        let g = Guards::new(&Rules::default(), &CouncilRules::default());
        assert!(g.validate().is_ok());
        assert!(
            Guards {
                risk_percent: 5.0,
                ..g.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            Guards {
                min_council_score: f64::NAN,
                ..g
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let row = DecisionSummary {
            id: "a".into(),
            created_at: "t".into(),
            created_ms: 0,
            symbol: "=HYPERLINK(\"x\")".into(),
            venue: "mt5".into(),
            trigger: Trigger {
                kind: "scan".into(),
                detail: None,
            },
            stage: "council".into(),
            status: "stopped".into(),
            stop_code: Some("council_below_threshold".into()),
            stop_label: None,
            stop_detail: Some("Score +0.18".into()),
            score: Some(0.178),
            threshold: Some(0.3),
            council_passed: false,
            side: None,
            has_plan: false,
            has_preview: false,
            result_r: None,
            step: 1,
            tags: vec![],
        };
        let csv = Decisions::to_csv(&[row]);
        assert!(csv.contains("\"'=HYPERLINK(\"\"x\"\")\""));
        assert!(csv.contains("\"0.178\""));
    }
}

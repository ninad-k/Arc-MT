//! The learning review (spec-architecture-page): statistics computed in code, explained and
//! ranked by Claude, checked again in code, applied only by the trader.
//!
//! - Claude receives statistics only: no symbols' prices, accounts, keys or logins.
//! - Claude has no tools and writes nothing. Its answer is parsed as JSON. A guard value must be
//!   one of the candidates the code offered, and every figure in a rationale must appear in the
//!   statistics. Anything else is rejected and listed as rejected.
//! - The weights item is always the code's own proposal; Claude only explains it.
//! - Tightening a guard and re-weighting apply with one click when the evidence gates pass.
//!   Loosening needs 100 closed decisions, a counterfactual record that favours it, a second
//!   confirmation and, when login is on, an authenticator code. Every apply is a new config
//!   version (rollback is one click) and an audit row.

use std::collections::BTreeMap;

use chrono::{SecondsFormat, Utc};
use jarvis_desk::council::Weights;
use jarvis_desk::learning::{self, GuardStat, LearnConfig, MemberStats, WeightProposal};
use jarvis_llm::{ChatRequest, LlmError, LlmProvider, Message};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::audit::AuditKind;
use crate::decisions::{Guards, LiveConfig};
use crate::desk::Desk;
use crate::spend::SpendStatus;

pub const MIN_DECISIONS_TIGHTEN: usize = 30;
pub const MIN_DECISIONS_LOOSEN: usize = 100;
pub const MIN_GUARD_RESOLVED_LOOSEN: usize = 30;
pub const REVIEW_EVERY_RESOLVED: usize = 20;
pub const REVIEW_EVERY_MS: i64 = 7 * 24 * 3600 * 1000;
const MAX_GUARD_ITEMS: usize = 3;

/// A tunable guard: its name, the stop code it produces, the steps a review may propose, and
/// which way is tighter.
struct GuardSpec {
    name: &'static str,
    stop_code: &'static str,
    steps: &'static [f64],
    /// `true`: a larger value is stricter.
    larger_is_tighter: bool,
    decimals: i32,
}

const GUARDS: [GuardSpec; 6] = [
    GuardSpec {
        name: "risk_percent",
        stop_code: "sentinel_veto:risk_cap",
        steps: &[0.1, 0.25],
        larger_is_tighter: false,
        decimals: 2,
    },
    GuardSpec {
        name: "max_open_positions",
        stop_code: "sentinel_veto:max_open_positions",
        steps: &[1.0],
        larger_is_tighter: false,
        decimals: 0,
    },
    GuardSpec {
        name: "no_new_trades_before_close_minutes",
        stop_code: "sentinel_veto:pre_close",
        steps: &[5.0, 15.0],
        larger_is_tighter: true,
        decimals: 0,
    },
    GuardSpec {
        name: "min_reward_risk",
        stop_code: "sentinel_veto:min_reward_risk",
        steps: &[0.25, 0.5],
        larger_is_tighter: true,
        decimals: 2,
    },
    GuardSpec {
        name: "min_council_score",
        stop_code: "council_below_threshold",
        steps: &[0.05, 0.1],
        larger_is_tighter: true,
        decimals: 2,
    },
    GuardSpec {
        name: "min_non_neutral",
        stop_code: "members_split",
        steps: &[1.0],
        larger_is_tighter: true,
        decimals: 0,
    },
];

fn spec(name: &str) -> Option<&'static GuardSpec> {
    GUARDS.iter().find(|g| g.name == name)
}

pub fn guard_get(g: &Guards, name: &str) -> Option<f64> {
    Some(match name {
        "risk_percent" => g.risk_percent,
        "max_open_positions" => f64::from(g.max_open_positions),
        "no_new_trades_before_close_minutes" => g.no_new_trades_before_close_minutes as f64,
        "min_reward_risk" => g.min_reward_risk,
        "min_council_score" => g.min_council_score,
        "min_non_neutral" => g.min_non_neutral as f64,
        _ => return None,
    })
}

pub fn guard_set(g: &Guards, name: &str, v: f64) -> Option<Guards> {
    let mut out = g.clone();
    match name {
        "risk_percent" => out.risk_percent = v,
        "max_open_positions" => out.max_open_positions = u32::try_from(v as i64).ok()?,
        "no_new_trades_before_close_minutes" => out.no_new_trades_before_close_minutes = v as i64,
        "min_reward_risk" => out.min_reward_risk = v,
        "min_council_score" => out.min_council_score = v,
        "min_non_neutral" => out.min_non_neutral = usize::try_from(v as i64).ok()?,
        _ => return None,
    }
    out.validate().ok().map(|()| out)
}

fn round_to(v: f64, decimals: i32) -> f64 {
    let f = 10f64.powi(decimals);
    (v * f).round() / f
}

/// The values a review may propose for each guard: the live value plus or minus each step,
/// within the hard bounds.
pub fn candidates(live: &Guards) -> BTreeMap<String, Vec<f64>> {
    GUARDS
        .iter()
        .map(|g| {
            let cur = guard_get(live, g.name).unwrap_or_default();
            let mut vals: Vec<f64> = g
                .steps
                .iter()
                .flat_map(|s| [cur - s, cur + s])
                .map(|v| round_to(v, g.decimals))
                .filter(|v| guard_set(live, g.name, *v).is_some())
                .collect();
            vals.sort_by(f64::total_cmp);
            vals.dedup();
            (g.name.to_owned(), vals)
        })
        .collect()
}

/// `"tighten"` or `"loosen"` for a guard change.
pub fn direction(name: &str, old: f64, new: f64) -> &'static str {
    let larger = spec(name).is_some_and(|g| g.larger_is_tighter);
    if (new > old) == larger {
        "tighten"
    } else {
        "loosen"
    }
}

/// The guards that `new` loosens compared with `old`.
pub fn loosened(old: &Guards, new: &Guards) -> Vec<String> {
    GUARDS
        .iter()
        .filter_map(|g| {
            let (a, b) = (guard_get(old, g.name)?, guard_get(new, g.name)?);
            ((a - b).abs() > 1e-9 && direction(g.name, a, b) == "loosen").then(|| g.name.to_owned())
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct Gates {
    pub decisions_closed: usize,
    pub resolved_opinions: usize,
    pub resolved_since_last_review: usize,
    pub last_review_ms: Option<i64>,
    pub can_review: bool,
    pub why_not: Option<String>,
    pub min_decisions_tighten: usize,
    pub min_decisions_loosen: usize,
    pub min_guard_resolved_loosen: usize,
    pub review_every_resolved: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub members: Vec<MemberStats>,
    pub guards: Vec<GuardStat>,
    pub proposal: WeightProposal,
    pub live: LiveConfig,
    pub candidates: BTreeMap<String, Vec<f64>>,
    pub gates: Gates,
}

/// Everything the Learning tab shows, computed in code. Free.
pub async fn snapshot(desk: &Desk) -> anyhow::Result<Snapshot> {
    let decisions = desk.decisions();
    let resolved = decisions.resolved_opinions().await?;
    let (ends, closed) = decisions.decision_ends().await?;
    let live = decisions.live();
    let proposal = learning::propose_weights(&live.weights, &resolved, &LearnConfig::default());
    let last = sqlx::query("SELECT max(created_ms) AS t FROM learning_reviews")
        .fetch_one(decisions.pool())
        .await?
        .try_get::<Option<i64>, _>("t")?;
    let since: i64 = sqlx::query(
        "SELECT count(DISTINCT decision_id) AS n FROM decision_opinions
         WHERE move_atr IS NOT NULL AND resolved_ms > ?",
    )
    .bind(last.unwrap_or(0))
    .fetch_one(decisions.pool())
    .await?
    .try_get("n")?;
    let since = usize::try_from(since).unwrap_or(0);
    let now = Utc::now().timestamp_millis();
    let why_not = match last {
        Some(t) if now - t < REVIEW_EVERY_MS && since < REVIEW_EVERY_RESOLVED => Some(format!(
            "The last review was less than a week ago and {since} of {REVIEW_EVERY_RESOLVED} new decisions have resolved since."
        )),
        _ if resolved.is_empty() => Some("No decision has resolved yet.".into()),
        _ => None,
    };
    Ok(Snapshot {
        members: learning::member_stats(&resolved),
        guards: learning::guard_stats(&ends),
        candidates: candidates(&live.guards),
        proposal,
        gates: Gates {
            decisions_closed: closed,
            resolved_opinions: resolved.len(),
            resolved_since_last_review: since,
            last_review_ms: last,
            can_review: why_not.is_none(),
            why_not,
            min_decisions_tighten: MIN_DECISIONS_TIGHTEN,
            min_decisions_loosen: MIN_DECISIONS_LOOSEN,
            min_guard_resolved_loosen: MIN_GUARD_RESOLVED_LOOSEN,
            review_every_resolved: REVIEW_EVERY_RESOLVED,
        },
        live,
    })
}

/// One checked proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// `weights` or `guard`.
    pub kind: String,
    /// The guard's name, or `weights`.
    pub param: String,
    pub old: Value,
    pub new: Value,
    /// `reweight`, `tighten` or `loosen`.
    pub direction: String,
    pub rationale: String,
    pub evidence_refs: Vec<String>,
    pub confidence: f64,
    /// Why it cannot be applied with one click (`None`: it can).
    pub locked: Option<String>,
    pub applied_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewRecord {
    pub id: String,
    pub created_at: String,
    pub model: String,
    pub summary: Option<String>,
    pub items: Vec<Item>,
    pub rejected: Vec<Value>,
    pub cost_usd: f64,
    pub live_version: Option<i64>,
}

#[derive(Debug)]
pub enum LearnError {
    NotFound,
    TooSoon(String),
    Budget(SpendStatus),
    Llm(LlmError),
    BadOutput(String),
    Stale,
    Locked(String),
    NeedsConfirm,
    Applied,
    Store(anyhow::Error),
}

impl std::fmt::Display for LearnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LearnError::NotFound => f.write_str("no such review or proposal"),
            LearnError::TooSoon(w) => write!(f, "not yet: {w}"),
            LearnError::Budget(s) => write!(
                f,
                "the learning review's monthly cap is reached (${:.2} of ${:.2})",
                s.spent_usd, s.budget_usd
            ),
            LearnError::Llm(e) => write!(f, "{e}"),
            LearnError::BadOutput(e) => write!(f, "the review's answer could not be used: {e}"),
            LearnError::Stale => f.write_str(
                "the live weights or guards changed since this review; run a new review",
            ),
            LearnError::Locked(w) => write!(f, "this change cannot be applied yet: {w}"),
            LearnError::NeedsConfirm => {
                f.write_str("loosening a guard needs a second confirmation")
            }
            LearnError::Applied => f.write_str("this proposal was applied already"),
            LearnError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl From<anyhow::Error> for LearnError {
    fn from(e: anyhow::Error) -> Self {
        LearnError::Store(e)
    }
}

const SYSTEM: &str = "You review the statistics of a trading desk's decision pipeline. Six members \
vote bull, bear or neutral with a confidence; a council weighs the votes; guards stop decisions. \
Every number in the input was computed by code. Do not compute or invent numbers: quote figures \
exactly as they appear in the input. Reply with one JSON object and nothing else: \
{\"summary\": string (at most 3 sentences), \"items\": [{\"type\": \"weights\" | \"guard\", \
\"param\": string, \"new\": number, \"rationale\": string (at most 3 sentences), \
\"evidence_refs\": [string], \"confidence\": number from 0 to 1}]}. \
For type \"weights\" set param to \"weights\" and omit new: include it only when \
weights_proposal.eligible is true, and explain it. For type \"guard\", param is one of the keys of \
guard_candidates and new must be one of its listed values. Propose at most 3 guard changes, and \
none when the evidence is thin (fewer than 30 resolved decisions behind it). Prefer no change to a \
weak one. This is analysis, not financial advice.";

fn input_json(s: &Snapshot) -> Value {
    let weights = s.live.weights.normalised();
    json!({
        "members": s.members.iter().map(|m| json!({
            "member": m.agent, "weight": (weights.get(m.agent) * 1000.0).round() / 1000.0,
            "scored": m.n, "taken": m.n_taken, "counterfactual": m.n_counterfactual,
            "neutral": m.neutral, "flat": m.flat, "hit_rate": m.hit_rate,
            "hit_rate_taken": m.hit_rate_taken, "hit_rate_counterfactual": m.hit_rate_counterfactual,
            "brier": m.brier, "skill": m.skill, "skill_80_low": m.skill_low,
            "skill_80_high": m.skill_high,
        })).collect::<Vec<_>>(),
        "guards": s.guards.iter().map(|g| json!({
            "stop_code": g.code, "param": GUARDS.iter().find(|x| x.stop_code == g.code).map(|x| x.name),
            "stops": g.stops, "resolved": g.resolved,
            "mean_move_atr_after_stop": g.mean_move_atr,
        })).collect::<Vec<_>>(),
        "weights_proposal": {
            "eligible": s.proposal.eligible,
            "changes": s.proposal.changes,
            "holdout": s.proposal.holdout,
            "notes": s.proposal.notes,
        },
        "live_guards": s.live.guards,
        "guard_candidates": s.candidates,
        "decisions_closed": s.gates.decisions_closed,
        "resolved_opinions": s.gates.resolved_opinions,
        "notes": [
            "Skill is 1 - Brier/0.25: 0 is a coin, positive is better.",
            "mean_move_atr_after_stop is the move in the council's direction after a stop: positive means the stop missed a move, negative that it avoided one. No fills or costs.",
            "Counterfactual opinions count half in the scores."
        ],
    })
}

/// Every number written in a text.
fn numbers_in(text: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<f64>| {
        let t = cur.trim_matches(|c: char| c == '.' || c == '+');
        if let Ok(v) = t.parse::<f64>()
            && t.chars().any(|c| c.is_ascii_digit())
        {
            out.push(v);
        }
        cur.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' || ((ch == '-' || ch == '+') && cur.is_empty()) {
            cur.push(ch);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Whether every figure in `text` comes from `allowed` (also as a percentage, or as a small
/// count of ten or less).
fn grounded(text: &str, allowed: &[f64]) -> Result<(), String> {
    for y in numbers_in(text) {
        let ok = (y.fract() == 0.0 && y.abs() <= 10.0)
            || allowed
                .iter()
                .any(|z| (z - y).abs() < 0.0051 || (z * 100.0 - y).abs() < 0.51);
        if !ok {
            return Err(format!("cites {y}, which is not in the statistics"));
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct RawAnswer {
    summary: Option<String>,
    #[serde(default)]
    items: Vec<Value>,
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// A checked answer: the summary, the usable items and the rejected ones with the reason.
pub type Checked = (Option<String>, Vec<Item>, Vec<Value>);

/// Check Claude's answer against the snapshot. Returns the usable items and the rejected ones
/// with the reason.
pub fn check_answer(text: &str, s: &Snapshot, input: &Value) -> Result<Checked, String> {
    let start = text.find('{').ok_or("no JSON object")?;
    let end = text.rfind('}').ok_or("no JSON object")?;
    let raw: RawAnswer = serde_json::from_str(&text[start..=end]).map_err(|e| e.to_string())?;
    let allowed = numbers_in(&input.to_string());
    let mut items: Vec<Item> = Vec::new();
    let mut rejected = Vec::new();
    let live = &s.live;
    let tighten_lock = (s.gates.decisions_closed < MIN_DECISIONS_TIGHTEN).then(|| {
        format!(
            "{} of {MIN_DECISIONS_TIGHTEN} closed decisions",
            s.gates.decisions_closed
        )
    });
    for v in raw.items {
        let kind = v["type"].as_str().unwrap_or_default();
        let rationale = clip(v["rationale"].as_str().unwrap_or_default().trim(), 600);
        let refs: Vec<String> = v["evidence_refs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .take(10)
                    .map(|r| clip(r, 80))
                    .filter(|r| grounded(r, &allowed).is_ok())
                    .collect()
            })
            .unwrap_or_default();
        let confidence = v["confidence"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
        let reject = |why: String, rejected: &mut Vec<Value>| {
            rejected.push(json!({ "item": v, "reason": why }));
        };
        if let Err(why) = grounded(&rationale, &allowed) {
            reject(why, &mut rejected);
            continue;
        }
        match kind {
            "weights" => {
                if !s.proposal.eligible {
                    reject(
                        "the code's weight proposal is not eligible".into(),
                        &mut rejected,
                    );
                } else if items.iter().any(|i| i.kind == "weights") {
                    reject("a second weights item".into(), &mut rejected);
                } else {
                    items.push(weights_item(s, rationale, refs, confidence));
                }
            }
            "guard" => {
                let param = v["param"].as_str().unwrap_or_default();
                let Some(g) = spec(param) else {
                    reject(format!("{param} is not a tunable guard"), &mut rejected);
                    continue;
                };
                let Some(new) = v["new"].as_f64() else {
                    reject("no new value".into(), &mut rejected);
                    continue;
                };
                let ok = s
                    .candidates
                    .get(param)
                    .is_some_and(|c| c.iter().any(|x| (x - new).abs() < 1e-9));
                if !ok {
                    reject(
                        format!("{new} is not one of the offered values for {param}"),
                        &mut rejected,
                    );
                    continue;
                }
                if items.iter().any(|i| i.param == param) {
                    reject(format!("a second change to {param}"), &mut rejected);
                    continue;
                }
                if items.iter().filter(|i| i.kind == "guard").count() >= MAX_GUARD_ITEMS {
                    reject("more than three guard changes".into(), &mut rejected);
                    continue;
                }
                let old = guard_get(&live.guards, param).unwrap_or_default();
                let dir = direction(param, old, new);
                let locked = if dir == "tighten" {
                    tighten_lock.clone()
                } else {
                    let stat = s.guards.iter().find(|x| x.code == g.stop_code);
                    if s.gates.decisions_closed < MIN_DECISIONS_LOOSEN {
                        Some(format!(
                            "loosening needs {MIN_DECISIONS_LOOSEN} closed decisions, there are {}",
                            s.gates.decisions_closed
                        ))
                    } else if !stat.is_some_and(|x| {
                        x.resolved >= MIN_GUARD_RESOLVED_LOOSEN
                            && x.mean_move_atr.is_some_and(|m| m > 0.0)
                    }) {
                        Some(format!(
                            "loosening needs {MIN_GUARD_RESOLVED_LOOSEN} resolved stops by this guard that missed moves on average"
                        ))
                    } else {
                        None
                    }
                };
                items.push(Item {
                    id: format!("g{}", items.len() + 1),
                    kind: "guard".into(),
                    param: param.into(),
                    old: json!(old),
                    new: json!(new),
                    direction: dir.into(),
                    rationale,
                    evidence_refs: refs,
                    confidence,
                    locked,
                    applied_version: None,
                });
            }
            other => reject(format!("unknown type {other:?}"), &mut rejected),
        }
    }
    // The weights are the code's proposal: offered even when the answer left them out.
    if s.proposal.eligible && !items.iter().any(|i| i.kind == "weights") {
        items.insert(
            0,
            weights_item(
                s,
                "The code's weight proposal (the review did not comment on it).".into(),
                vec![],
                0.5,
            ),
        );
    }
    // The summary is checked like a rationale; an ungrounded one is replaced, not shown.
    let summary = raw.summary.map(|t| clip(t.trim(), 800)).map(|t| {
        if grounded(&t, &allowed).is_ok() {
            t
        } else {
            "(The review's summary was left out: it cited a figure not in the statistics.)"
                .to_owned()
        }
    });
    Ok((summary, items, rejected))
}

fn weights_item(s: &Snapshot, rationale: String, refs: Vec<String>, confidence: f64) -> Item {
    Item {
        id: "w1".into(),
        kind: "weights".into(),
        param: "weights".into(),
        old: json!(s.proposal.current),
        new: json!(s.proposal.proposed),
        direction: "reweight".into(),
        rationale,
        evidence_refs: refs,
        confidence,
        locked: None,
        applied_version: None,
    }
}

/// Run a review: statistics to Claude, the answer checked and stored. Paid (metered).
pub async fn review(
    desk: &Desk,
    provider: &dyn LlmProvider,
    metered: bool,
    actor: Option<&str>,
) -> Result<ReviewRecord, LearnError> {
    let s = snapshot(desk).await?;
    if let Some(w) = &s.gates.why_not {
        return Err(LearnError::TooSoon(w.clone()));
    }
    let input = input_json(&s);
    let max_tokens = desk.learning_config().max_tokens;
    let request = ChatRequest {
        system: Some(SYSTEM.to_owned()),
        messages: vec![Message::User {
            text: input.to_string(),
        }],
        tools: vec![],
        max_tokens: Some(max_tokens),
        temperature: None,
        effort: None,
        cache_system: false,
    };
    let spend = desk.learning_spend();
    let reservation = if metered {
        let worst = spend.worst_case(
            crate::assistant::input_tokens_estimate(&request),
            max_tokens,
        );
        Some(
            spend
                .reserve(Utc::now(), worst)
                .map_err(LearnError::Budget)?,
        )
    } else {
        None
    };
    let response = match provider.chat(&request).await {
        Ok(r) => r,
        Err(e) => {
            if let Some(r) = reservation {
                let billed = !matches!(
                    &e,
                    LlmError::Config(_)
                        | LlmError::Api {
                            status: 400..=499,
                            ..
                        }
                );
                if billed {
                    let _ = spend.settle(Utc::now(), r, None);
                } else {
                    spend.release(Utc::now(), r);
                }
            }
            return Err(LearnError::Llm(e));
        }
    };
    let cost = match reservation {
        Some(r) => spend.settle(Utc::now(), r, response.usage.as_ref()).1,
        None => 0.0,
    };
    let text = response.text.clone().unwrap_or_default();
    let audit = desk.audit();
    let u = response.usage.unwrap_or_default();
    audit
        .record(
            None,
            AuditKind::ClaudeUsage,
            json!({ "purpose": "learning", "provider": provider.name(), "model": provider.model(),
                    "input_tokens": u.input_tokens, "output_tokens": u.output_tokens,
                    "cost_usd": cost, "month_usd": spend.status(Utc::now()).spent_usd }),
        )
        .await;
    let (summary, items, rejected) =
        check_answer(&text, &s, &input).map_err(LearnError::BadOutput)?;
    let id = Uuid::new_v4().simple().to_string();
    let now = Utc::now();
    let created_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    sqlx::query(
        "INSERT INTO learning_reviews (id, created_at, created_ms, provider, model, live_version,
            input_json, output_text, summary, items_json, rejected_json, cost_usd)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&created_at)
    .bind(now.timestamp_millis())
    .bind(provider.name())
    .bind(provider.model())
    .bind(s.live.version)
    .bind(input.to_string())
    .bind(&text)
    .bind(&summary)
    .bind(serde_json::to_string(&items).map_err(anyhow::Error::from)?)
    .bind(serde_json::to_string(&rejected).map_err(anyhow::Error::from)?)
    .bind(cost)
    .execute(desk.decisions().pool())
    .await
    .map_err(anyhow::Error::from)?;
    audit
        .record(
            None,
            AuditKind::LearningReview,
            json!({ "review_id": id, "model": provider.model(), "items": items.len(),
                    "rejected": rejected.len(), "cost_usd": cost, "user": actor }),
        )
        .await;
    Ok(ReviewRecord {
        id,
        created_at,
        model: provider.model().to_owned(),
        summary,
        items,
        rejected,
        cost_usd: cost,
        live_version: s.live.version,
    })
}

/// The newest reviews, newest first.
pub async fn recent(desk: &Desk, limit: u32) -> anyhow::Result<Vec<ReviewRecord>> {
    let rows = sqlx::query(
        "SELECT id, created_at, model, summary, items_json, rejected_json, cost_usd, live_version
         FROM learning_reviews ORDER BY created_ms DESC LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(desk.decisions().pool())
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(ReviewRecord {
                id: r.try_get("id")?,
                created_at: r.try_get("created_at")?,
                model: r.try_get("model")?,
                summary: r.try_get("summary")?,
                items: serde_json::from_str(&r.try_get::<String, _>("items_json")?)?,
                rejected: serde_json::from_str(&r.try_get::<String, _>("rejected_json")?)?,
                cost_usd: r.try_get("cost_usd")?,
                live_version: r.try_get("live_version")?,
            })
        })
        .collect()
}

async fn load_item(
    desk: &Desk,
    review_id: &str,
    item_id: &str,
) -> Result<(Vec<Item>, usize), LearnError> {
    let row = sqlx::query("SELECT items_json FROM learning_reviews WHERE id = ?")
        .bind(review_id)
        .fetch_optional(desk.decisions().pool())
        .await
        .map_err(anyhow::Error::from)?
        .ok_or(LearnError::NotFound)?;
    let items: Vec<Item> = serde_json::from_str(
        &row.try_get::<String, _>("items_json")
            .map_err(anyhow::Error::from)?,
    )
    .map_err(anyhow::Error::from)?;
    let i = items
        .iter()
        .position(|i| i.id == item_id)
        .ok_or(LearnError::NotFound)?;
    Ok((items, i))
}

/// Whether applying this item loosens a guard (the route then asks for the second factor).
pub async fn item_loosens(desk: &Desk, review_id: &str, item_id: &str) -> Result<bool, LearnError> {
    let (items, i) = load_item(desk, review_id, item_id).await?;
    Ok(items[i].direction == "loosen")
}

/// Apply one checked item as a new config version.
pub async fn apply(
    desk: &Desk,
    review_id: &str,
    item_id: &str,
    confirm_loosen: bool,
    actor: Option<&str>,
) -> Result<i64, LearnError> {
    // From reading the live config and the review to saving both: one change at a time.
    let _one = desk.decisions().lock_config().await;
    let (mut items, i) = load_item(desk, review_id, item_id).await?;
    let item = items[i].clone();
    if item.applied_version.is_some() {
        return Err(LearnError::Applied);
    }
    if let Some(w) = &item.locked {
        return Err(LearnError::Locked(w.clone()));
    }
    let decisions = desk.decisions();
    let live = decisions.live();
    let (weights, guards) = match item.kind.as_str() {
        "weights" => {
            let old: Weights =
                serde_json::from_value(item.old.clone()).map_err(anyhow::Error::from)?;
            let cur = live.weights.normalised();
            let same = jarvis_desk::opinion::MEMBERS
                .iter()
                .all(|a| (old.get(*a) - cur.get(*a)).abs() < 1e-3);
            if !same {
                return Err(LearnError::Stale);
            }
            let new: Weights =
                serde_json::from_value(item.new.clone()).map_err(anyhow::Error::from)?;
            (new, live.guards.clone())
        }
        _ => {
            let old = item.old.as_f64().unwrap_or(f64::NAN);
            if guard_get(&live.guards, &item.param).is_none_or(|v| (v - old).abs() > 1e-9) {
                return Err(LearnError::Stale);
            }
            if item.direction == "loosen" && !confirm_loosen {
                return Err(LearnError::NeedsConfirm);
            }
            let new = item.new.as_f64().unwrap_or(f64::NAN);
            let g = guard_set(&live.guards, &item.param, new)
                .ok_or_else(|| LearnError::Locked("outside the allowed range".into()))?;
            (live.weights.clone(), g)
        }
    };
    let reason = match item.kind.as_str() {
        "weights" => "Learning review: council weights".to_owned(),
        _ => format!(
            "Learning review: {} {} -> {} ({})",
            item.param, item.old, item.new, item.direction
        ),
    };
    let version = decisions
        .save_version(
            &weights,
            &guards,
            &reason,
            Some(&format!("{review_id}:{item_id}")),
            actor,
        )
        .await?;
    items[i].applied_version = Some(version);
    sqlx::query("UPDATE learning_reviews SET items_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&items).map_err(anyhow::Error::from)?)
        .bind(review_id)
        .execute(decisions.pool())
        .await
        .map_err(anyhow::Error::from)?;
    desk.audit()
        .record(
            None,
            AuditKind::CouncilConfigChanged,
            json!({ "version": version, "review_id": review_id, "item": item_id,
                    "param": item.param, "old": item.old, "new": item.new,
                    "direction": item.direction, "user": actor }),
        )
        .await;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use jarvis_desk::council::CouncilRules;
    use jarvis_desk::sentinel::Rules;

    use super::*;

    fn guards() -> Guards {
        Guards::new(&Rules::default(), &CouncilRules::default())
    }

    #[test]
    fn candidates_stay_in_bounds_and_directions_are_right() {
        let c = candidates(&guards());
        assert_eq!(c["max_open_positions"], vec![1.0, 3.0]);
        assert_eq!(c["min_council_score"], vec![0.2, 0.25, 0.35, 0.4]);
        assert!(c["risk_percent"].contains(&0.25));
        assert_eq!(direction("max_open_positions", 2.0, 3.0), "loosen");
        assert_eq!(direction("min_council_score", 0.3, 0.35), "tighten");
        assert_eq!(direction("risk_percent", 0.5, 0.4), "tighten");
        let looser = guard_set(&guards(), "max_open_positions", 3.0).unwrap();
        assert_eq!(
            loosened(&guards(), &looser),
            vec!["max_open_positions".to_owned()]
        );
        assert!(loosened(&looser, &guards()).is_empty());
        assert!(guard_set(&guards(), "risk_percent", 9.0).is_none());
    }

    #[test]
    fn figures_must_come_from_the_statistics() {
        let allowed = [0.112, 84.0, 0.58];
        assert!(
            grounded(
                "Oracle's skill is +0.11 over 84 opinions, hit rate 58%.",
                &allowed
            )
            .is_ok()
        );
        assert!(grounded("It would add 2 positions.", &allowed).is_ok());
        assert!(grounded("Expect 37% more profit.", &allowed).is_err());
        assert_eq!(
            numbers_in("a -0.5 b 1,200 c 3."),
            vec![-0.5, 1.0, 200.0, 3.0]
        );
    }

    fn snap(closed: usize) -> Snapshot {
        let live = LiveConfig {
            version: None,
            weights: Weights::default(),
            guards: guards(),
        };
        Snapshot {
            members: vec![],
            guards: vec![GuardStat {
                code: "sentinel_veto:max_open_positions".into(),
                stops: 40,
                resolved: 35,
                mean_move_atr: Some(0.34),
            }],
            proposal: learning::propose_weights(&Weights::default(), &[], &LearnConfig::default()),
            candidates: candidates(&live.guards),
            gates: Gates {
                decisions_closed: closed,
                resolved_opinions: 0,
                resolved_since_last_review: 0,
                last_review_ms: None,
                can_review: true,
                why_not: None,
                min_decisions_tighten: MIN_DECISIONS_TIGHTEN,
                min_decisions_loosen: MIN_DECISIONS_LOOSEN,
                min_guard_resolved_loosen: MIN_GUARD_RESOLVED_LOOSEN,
                review_every_resolved: REVIEW_EVERY_RESOLVED,
            },
            live,
        }
    }

    #[test]
    fn answers_are_checked_item_by_item() {
        let s = snap(40);
        let input = input_json(&s);
        let answer = r#"Here you go: {"summary": "Thin data.", "items": [
            {"type": "guard", "param": "min_council_score", "new": 0.35, "rationale": "40 decisions closed.", "confidence": 0.6},
            {"type": "guard", "param": "max_open_positions", "new": 3, "rationale": "Stops missed 0.34 ATR.", "confidence": 0.4},
            {"type": "guard", "param": "risk_percent", "new": 0.9, "rationale": "More.", "confidence": 0.9},
            {"type": "guard", "param": "min_reward_risk", "new": 1.75, "rationale": "Profit up 37%.", "confidence": 0.9},
            {"type": "weights", "rationale": "x", "confidence": 1},
            {"type": "delete_everything"}
        ]}"#;
        let (summary, items, rejected) = check_answer(answer, &s, &input).unwrap();
        assert_eq!(summary.as_deref(), Some("Thin data."));
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(items[0].direction, "tighten");
        assert!(items[0].locked.is_none());
        assert_eq!(items[1].direction, "loosen");
        assert!(
            items[1]
                .locked
                .as_deref()
                .unwrap()
                .contains("100 closed decisions")
        );
        assert_eq!(rejected.len(), 4, "{rejected:?}");
        assert!(check_answer("no json", &s, &input).is_err());
        // With few decisions even tightening is locked.
        let (_, items, _) = check_answer(answer, &snap(10), &input).unwrap();
        assert!(items[0].locked.is_some());
    }
}

//! The architecture page over HTTP (spec-architecture-page): the pipeline map, the free council
//! scan, the decisions (list, detail, export, funnel, review), "Ask Friday", and the versioned
//! council weights and Desk guards.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use jarvis_desk::AgentId;
use jarvis_desk::opinion::MEMBERS;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::audit::AuditKind;
use crate::auth::http::MaybeSession;
use crate::decisions::{DecisionFilter, Review};
use crate::desk::{Desk, DeskUser, pipeline::AskError};
use crate::routes::ApiError;
use crate::state::AppState;

fn desk(state: &AppState) -> Result<Arc<Desk>, ApiError> {
    state.desk().cloned().ok_or_else(|| {
        ApiError::NotFound("Desk mode is turned off ([desk] enabled = false)".into())
    })
}

async fn desk_user(
    state: &AppState,
    session: Option<&crate::auth::Session>,
    intent: jarvis_gate::intent::Intent,
) -> DeskUser {
    let settings = state.user_settings(session).await;
    DeskUser {
        resolver: state.resolver_for(&settings),
        watchlist: settings.watchlist.clone(),
        intent,
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    tracing::error!("architecture: {e}");
    ApiError::Internal
}

/// `GET /api/architecture/map`: the tiers, each seat with its role, weight and newest opinion,
/// the live guards and the version they come from.
pub async fn map(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let live = desk.decisions().live();
    let weights = live.weights.normalised();
    let latest = desk.decisions().member_latest().await.map_err(internal)?;
    let seats = desk.seats();
    let seat = |a: AgentId| seats.iter().find(|s| s.id == a).cloned();
    let members: Vec<Value> = MEMBERS
        .iter()
        .map(|a| {
            json!({
                "agent": a, "title": a.title(), "group": a.group(),
                "weight": weights.get(*a),
                "latest": latest.iter().find(|l| l["agent"] == a.as_str()),
                "seat": seat(*a),
            })
        })
        .collect();
    let execution: Vec<Value> = ["sentinel", "pilot", "gate", "confirm", "broker"]
        .iter()
        .map(|s| json!({ "id": s, "seat": AgentId::parse(s).and_then(seat) }))
        .collect();
    let config = &state.config().desk;
    Ok(Json(json!({
        "members": members,
        "council": ["bull_case", "bear_case", "risk", "chair"],
        "master": { "agent": "friday", "provider": config.provider,
                    "model": state.provider(&config.provider).map(|e| e.config.model.clone()),
                    "seat": seat(AgentId::Friday),
                    "spend": desk.spend().status(Utc::now()) },
        "execution": execution,
        "live": live,
        "defaults": desk.decisions().defaults(),
        "session": desk.session_status(),
        "scan_every_minutes": config.scan_every_minutes,
        "kill_switch": state.trading().kill_switch_engaged(),
    })))
}

/// `POST /api/architecture/scan`: run the free council scan now.
pub async fn scan(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let user = desk_user(&state, session.as_ref(), jarvis_gate::intent::Intent::Other).await;
    let ids = desk.scan(&user).await;
    let filter = DecisionFilter {
        limit: Some(ids.len().max(1) as u32),
        ..DecisionFilter::default()
    };
    let rows = desk.decisions().list(&filter).await.map_err(internal)?;
    Ok(Json(json!({ "ids": ids, "decisions": rows })))
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    #[serde(flatten)]
    filter: DecisionFilter,
    /// `json` (default) or `csv`; `full` = JSON with every record in full.
    format: Option<String>,
}

/// `GET /api/architecture/decisions`: newest first, filtered; `?format=csv` or `full` exports.
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let desk = desk(&state)?;
    let decisions = desk.decisions();
    let _ = decisions.settle(Utc::now().timestamp_millis()).await;
    let rows = decisions.list(&q.filter).await.map_err(internal)?;
    match q.format.as_deref() {
        Some("csv") => Ok((
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"decisions.csv\"",
                ),
            ],
            crate::decisions::Decisions::to_csv(&rows),
        )
            .into_response()),
        Some("full") => {
            let mut full = Vec::new();
            for r in rows.iter().take(500) {
                if let Some(d) = decisions.get(&r.id).await.map_err(internal)? {
                    full.push(d);
                }
            }
            Ok((
                [(
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"decisions.json\"",
                )],
                Json(json!({ "decisions": full })),
            )
                .into_response())
        }
        None | Some("json") => Ok(Json(json!({ "decisions": rows })).into_response()),
        Some(other) => Err(ApiError::BadRequest(format!(
            "format must be json, csv or full, not {other}"
        ))),
    }
}

/// `GET /api/architecture/decisions/{id}`: one decision in full.
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let d = desk
        .decisions()
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ApiError::NotFound(format!("no decision {id}")))?;
    Ok(Json(json!(d)))
}

#[derive(Debug, Default, Deserialize)]
pub struct FunnelQuery {
    from_ms: Option<i64>,
    to_ms: Option<i64>,
}

/// `GET /api/architecture/funnel`: how many decisions reached each step and why the rest
/// stopped. Default window: the last 24 hours.
pub async fn funnel(
    State(state): State<AppState>,
    Query(q): Query<FunnelQuery>,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let now = Utc::now().timestamp_millis();
    let to = q.to_ms.unwrap_or(now + 1);
    let from = q.from_ms.unwrap_or(to - 24 * 3600 * 1000);
    if from >= to {
        return Err(ApiError::BadRequest("from_ms must be before to_ms".into()));
    }
    let _ = desk.decisions().settle(now).await;
    let steps = desk.decisions().funnel(from, to).await.map_err(internal)?;
    Ok(Json(
        json!({ "from_ms": from, "to_ms": to, "steps": steps }),
    ))
}

/// `PUT /api/architecture/decisions/{id}/review`: save the trader's analysis (audited).
pub async fn review_put(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
    Json(review): Json<Review>,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    review.validate().map_err(ApiError::BadRequest)?;
    if !desk
        .decisions()
        .save_review(&id, &review)
        .await
        .map_err(internal)?
    {
        return Err(ApiError::NotFound(format!("no decision {id}")));
    }
    state
        .audit()
        .record(
            None,
            AuditKind::DecisionReviewed,
            json!({ "decision_id": id, "setup": review.setup, "mistakes": review.mistakes,
                    "regime": review.regime, "rating": review.rating,
                    "take_again": review.take_again,
                    "user": session.as_ref().map(|s| s.username.clone()) }),
        )
        .await;
    let saved = desk.decisions().review(&id).await.map_err(internal)?;
    Ok(Json(json!({ "review": saved })))
}

/// `POST /api/architecture/decisions/{id}/ask`: Ask Friday for a plan on a decision the
/// council passed. A Desk turn: paid on Claude, inside the Desk budget.
pub async fn ask(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    if state.mode_mismatch() {
        return Err(ApiError::ModeMismatch);
    }
    let (provider, metered) = state.desk_provider().map_err(ApiError::BadRequest)?;
    let user = desk_user(&state, session.as_ref(), jarvis_gate::intent::Intent::Other).await;
    // Its own task, so the turn and its spending record finish even if the request is dropped.
    let turn = {
        let desk = desk.clone();
        tokio::spawn(async move {
            desk.ask_friday(provider.as_ref(), metered, &user, &id)
                .await
                .map(|o| (o, id))
        })
    };
    match turn.await.map_err(|_| ApiError::Internal)? {
        Ok((outcome, id)) => {
            let decision = desk.decisions().get(&id).await.map_err(internal)?;
            Ok(Json(json!({
                "session_id": outcome.session_id,
                "reply": outcome.reply,
                "preview": outcome.preview,
                "plans": outcome.plans,
                "spend": outcome.spend,
                "decision": decision,
            })))
        }
        Err(AskError::NotWaiting) => Err(ApiError::BadRequest(
            "this decision is not waiting for Friday (it was asked already or did not pass)".into(),
        )),
        Err(AskError::Llm(e)) => Err(e.into()),
        Err(AskError::Store(e)) => Err(internal(e)),
    }
}

/// `GET /api/architecture/versions`: the council weights and Desk guards, newest first.
pub async fn versions(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let list = desk.decisions().versions(50).await.map_err(internal)?;
    Ok(Json(json!({
        "live": desk.decisions().live(),
        "defaults": desk.decisions().defaults(),
        "versions": list,
    })))
}

fn learn_error(e: crate::learning::LearnError) -> ApiError {
    use crate::learning::LearnError as L;
    match e {
        L::NotFound => ApiError::NotFound(e.to_string()),
        L::Llm(e) => e.into(),
        L::Store(e) => internal(e),
        other => ApiError::BadRequest(other.to_string()),
    }
}

/// `GET /api/architecture/learning`: member scorecards, guard statistics, the code's weight
/// proposal, the gates, the newest reviews and the review's spend. Free.
pub async fn learning(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let snapshot = crate::learning::snapshot(&desk).await.map_err(internal)?;
    let reviews = crate::learning::recent(&desk, 5).await.map_err(internal)?;
    let cfg = &state.config().learning;
    let available = state.learning_provider().map(
        |(p, metered)| json!({ "provider": cfg.provider, "model": p.model(), "metered": metered }),
    );
    Ok(Json(json!({
        "snapshot": snapshot,
        "reviews": reviews,
        "spend": desk.learning_spend().status(Utc::now()),
        "brain": available.as_ref().ok(),
        "brain_error": available.err(),
        "estimate_usd": 0.03,
    })))
}

/// `POST /api/architecture/learning/review`: send the statistics to Claude for a review. Paid
/// (about $0.03 on Sonnet 5.5), capped by `[learning] monthly_budget_usd`.
pub async fn learning_review(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let (provider, metered) = state.learning_provider().map_err(ApiError::BadRequest)?;
    let actor = session.as_ref().map(|s| s.username.clone());
    let record = crate::learning::review(&desk, provider.as_ref(), metered, actor.as_deref())
        .await
        .map_err(learn_error)?;
    Ok(Json(json!({ "review": record,
                    "spend": desk.learning_spend().status(Utc::now()) })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyBody {
    review_id: String,
    item_id: String,
    #[serde(default)]
    confirm_loosen: bool,
    #[serde(default)]
    totp: Option<String>,
}

/// `POST /api/architecture/learning/apply`: apply one checked proposal as a new version.
/// Loosening a guard also needs `confirm_loosen` and, when login is on, a fresh code.
pub async fn learning_apply(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Json(body): Json<ApplyBody>,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    if crate::learning::item_loosens(&desk, &body.review_id, &body.item_id)
        .await
        .map_err(learn_error)?
    {
        if !body.confirm_loosen {
            return Err(learn_error(crate::learning::LearnError::NeedsConfirm));
        }
        state
            .release_guard()
            .authorize_release(session.as_ref(), body.totp.as_deref())
            .await
            .map_err(ApiError::Unauthorized)?;
    }
    let actor = session.as_ref().map(|s| s.username.clone());
    let version = crate::learning::apply(
        &desk,
        &body.review_id,
        &body.item_id,
        body.confirm_loosen,
        actor.as_deref(),
    )
    .await
    .map_err(learn_error)?;
    Ok(Json(
        json!({ "version": version, "live": desk.decisions().live() }),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackBody {
    #[serde(default)]
    confirm_loosen: bool,
    #[serde(default)]
    totp: Option<String>,
}

/// `POST /api/architecture/versions/{v}/rollback`: make an older version live again (as a new
/// version). When that loosens a guard it needs the same confirmation as applying a loosening.
pub async fn rollback(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(version): Path<i64>,
    body: Option<Json<RollbackBody>>,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let body = body.map(|b| b.0).unwrap_or_default();
    let decisions = desk.decisions();
    // From reading the live guards to saving the rollback: one change at a time.
    let _one = decisions.lock_config().await;
    let target = decisions
        .versions(200)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|v| v.version == version)
        .ok_or_else(|| ApiError::NotFound(format!("no version {version}")))?;
    let loosened = crate::learning::loosened(&decisions.live().guards, &target.guards);
    if !loosened.is_empty() {
        if !body.confirm_loosen {
            return Err(ApiError::BadRequest(format!(
                "rolling back to v{version} loosens {}; confirm it to go on",
                loosened.join(", ")
            )));
        }
        state
            .release_guard()
            .authorize_release(session.as_ref(), body.totp.as_deref())
            .await
            .map_err(ApiError::Unauthorized)?;
    }
    let actor = session.as_ref().map(|s| s.username.clone());
    let new = decisions
        .rollback(version, actor.as_deref())
        .await
        .map_err(internal)?;
    state
        .audit()
        .record(
            None,
            AuditKind::CouncilConfigChanged,
            json!({ "version": new, "rolled_back_to": version, "loosened": loosened,
                    "user": actor }),
        )
        .await;
    Ok(Json(json!({ "version": new, "live": decisions.live() })))
}

/// `GET /api/architecture/account-lock`: the master account lock as the browser may see it:
/// the expected account the trader set and whether MT5 matches. Never the reported number.
pub async fn account_lock_get(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    Ok(Json(json!(desk.master_account_view().await)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockBody {
    /// The account number (0 turns the lock off).
    expected: u64,
    /// The same number typed again.
    confirm: u64,
    #[serde(default)]
    broker: Option<String>,
    #[serde(default)]
    totp: Option<String>,
}

/// `PUT /api/architecture/account-lock`: set the master account. No MT5 password: MT5 is
/// already logged in. Needs the number typed twice and, when login is on, a fresh code.
pub async fn account_lock_put(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Json(body): Json<LockBody>,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    if body.expected != body.confirm {
        return Err(ApiError::BadRequest(
            "the two account numbers do not match".into(),
        ));
    }
    if body.expected > 999_999_999_999 {
        return Err(ApiError::BadRequest(
            "an MT5 account number has at most 12 digits".into(),
        ));
    }
    let broker = body
        .broker
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(|b| b.chars().take(60).collect::<String>())
        .unwrap_or_else(|| desk.account_lock().expected().1);
    state
        .release_guard()
        .authorize_release(session.as_ref(), body.totp.as_deref())
        .await
        .map_err(ApiError::Unauthorized)?;
    let (old, _) = desk.account_lock().expected();
    let actor = session.as_ref().map(|s| s.username.clone());
    desk.account_lock()
        .set(body.expected, &broker, actor.as_deref())
        .await
        .map_err(internal)?;
    state
        .audit()
        .record(
            None,
            AuditKind::AccountLockChanged,
            json!({ "old": crate::account_lock::mask(old),
                    "new": crate::account_lock::mask(body.expected),
                    "broker": broker, "user": actor }),
        )
        .await;
    Ok(Json(json!(desk.master_account_view().await)))
}

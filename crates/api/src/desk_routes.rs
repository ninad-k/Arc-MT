//! Desk mode over HTTP (spec-nimbus-desk): the HUD's state, the market pulse and the "Send to
//! Pilot" button. The chat itself is `POST /api/chat` with `mode: "desk"` (see `routes.rs`).

use axum::Json;
use axum::extract::{Path, State};
use chrono::{Datelike, Months, Utc};
use jarvis_desk::calendar;
use jarvis_desk::sentinel::ApprovalError;
use jarvis_desk::session::ny_local;
use serde_json::{Value, json};

use crate::auth::http::MaybeSession;
use crate::desk::{Desk, DeskUser, PilotError};
use crate::routes::ApiError;
use crate::state::AppState;
use crate::trading::{PreviewDto, ProposeSource};

fn desk(state: &AppState) -> Result<std::sync::Arc<Desk>, ApiError> {
    state.desk().cloned().ok_or_else(|| {
        ApiError::NotFound("Desk mode is turned off ([desk] enabled = false)".into())
    })
}

/// `GET /api/desk/state`: everything the Desk HUD shows except the pulse. Cheap: no bridge call.
pub async fn state(State(state): State<AppState>) -> Json<Value> {
    let Some(desk) = state.desk().cloned() else {
        return Json(json!({ "enabled": false,
            "reason": "Desk mode is turned off ([desk] enabled = false)." }));
    };
    let config = &state.config().desk;
    let model = state
        .provider(&config.provider)
        .map(|e| e.config.model.clone());
    let (available, reason, metered) = match state.desk_provider() {
        Ok((_, metered)) => (true, None, metered),
        Err(reason) => (false, Some(reason), false),
    };
    let now = Utc::now();
    let today = ny_local(now).date();
    let fomc = calendar::fomc_days(&[], &config.fomc_days());
    let exchange = desk.exchange();
    let next_month = today.checked_add_months(Months::new(1)).unwrap_or(today);
    let months: Vec<Value> = [today, next_month]
        .iter()
        .map(|d| {
            json!({ "year": d.year(), "month": d.month(),
                    "marks": calendar::month_marks(exchange, d.year(), d.month(), &fomc) })
        })
        .collect();
    let next_fomc = calendar::next_fomc(&fomc, today).map(|(d, src)| {
        json!({ "date": d.format("%Y-%m-%d").to_string(), "source": src,
                                "days": (d - today).num_days() })
    });
    Json(json!({
        "enabled": true,
        "brain": { "provider": config.provider, "model": model, "available": available,
                   "reason": reason, "metered": metered },
        "session": desk.session_status(),
        "calendar": { "today": today.format("%Y-%m-%d").to_string(), "months": months,
                      "next_fomc": next_fomc },
        "seats": desk.seats(),
        "signals": desk.signals(40),
        "plans": desk.plans(10),
        "journal": desk.journal(25).await,
        "spend": desk.spend().status(now),
        "finnhub": desk.finnhub_configured(),
        "rules": config.rules(),
        "kill_switch": state.trading().kill_switch_engaged(),
    }))
}

/// `GET /api/desk/pulse`: price and today's change of the watchlist and the indices. A symbol
/// without data has `null` figures.
pub async fn pulse(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> Result<Json<Value>, ApiError> {
    let desk = desk(&state)?;
    let settings = state.user_settings(session.as_ref()).await;
    let user = DeskUser {
        resolver: state.resolver_for(&settings),
        watchlist: settings.watchlist.clone(),
        intent: jarvis_gate::intent::Intent::Other,
    };
    let quotes = desk.pulse(&user).await;
    Ok(Json(
        json!({ "quotes": quotes, "at": Utc::now().to_rfc3339() }),
    ))
}

/// `POST /api/desk/plans/{id}/pilot`: the "Send to Pilot" button on an approved plan card. The
/// preview it returns is confirmed like any other (`POST /api/confirm/{token}`).
pub async fn pilot(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PreviewDto>, ApiError> {
    let desk = desk(&state)?;
    if state.mode_mismatch() {
        return Err(ApiError::ModeMismatch);
    }
    match desk.pilot(&id, ProposeSource::Ui, None, false).await {
        Ok(preview) => Ok(Json(preview)),
        Err(PilotError::Approval(ApprovalError::Unknown(_))) => {
            Err(ApiError::NotFound(format!("no Sentinel plan with id {id}")))
        }
        Err(PilotError::Refused(e)) => Err(e.into()),
        Err(e) => Err(ApiError::GateRejected(e.to_string())),
    }
}

//! `/api/market/*` (docs/api.md, "Market view").
//!
//! Reading is open to every logged-in user. Views are information only; `prepare` builds the
//! usual trade preview from a stored view and never confirms it.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::audit::AuditKind;
use crate::auth::http::MaybeSession;
use crate::market::{Actor, MarketError, MarketService, PrepareKind};
use crate::routes::{ApiError, propose_from_ui};
use crate::state::AppState;
use crate::trading::PreviewDto;

/// Journal rows returned when `limit` is not given, and the most a caller may ask for.
const JOURNAL_DEFAULT_LIMIT: u32 = 20;
const JOURNAL_MAX_LIMIT: u32 = 200;
const CALENDAR_DEFAULT_HOURS: u32 = 24;
const CALENDAR_MAX_HOURS: u32 = 168;

impl From<MarketError> for ApiError {
    fn from(e: MarketError) -> Self {
        match e {
            MarketError::UnusableSymbol(_) | MarketError::Invalid(_) => {
                ApiError::BadRequest(e.to_string())
            }
            MarketError::EaOutdated => ApiError::EaOutdated(e.to_string()),
            MarketError::Bridge(b) => b.into(),
            MarketError::ViewNotFound => ApiError::NotFound(e.to_string()),
            MarketError::ViewExpired => ApiError::ViewExpired(e.to_string()),
            MarketError::NoSetup => ApiError::NoSetup(e.to_string()),
            MarketError::Storage(err) => {
                tracing::error!("market journal: {err:#}");
                ApiError::Internal
            }
        }
    }
}

fn market(state: &AppState) -> Result<&Arc<MarketService>, ApiError> {
    state.market().ok_or_else(|| {
        ApiError::NotFound(
            "market views are turned off ([market] enabled = false in the config)".to_owned(),
        )
    })
}

/// A symbol typed by the user, trimmed; empty is a client error.
fn required_symbol(symbol: Option<String>) -> Result<String, ApiError> {
    symbol
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::BadRequest("symbol is required".to_owned()))
}

// ---------------------------------------------------------------------------
// GET /api/market/view
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SymbolQuery {
    #[serde(default)]
    symbol: Option<String>,
}

pub async fn view_get(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(query): Query<SymbolQuery>,
) -> Result<Json<Value>, ApiError> {
    let market = market(&state)?;
    let symbol = required_symbol(query.symbol)?;
    let settings = state.user_settings(session.as_ref()).await;
    let resolver = state.resolver_for(&settings);
    let actor = Actor {
        session: None,
        persona: Some(state.persona_for(&settings).id.clone()),
        source: "ui",
    };
    let view = market.view(&resolver, &symbol, &actor).await?;
    Ok(Json(serde_json::to_value(view).map_err(|e| {
        tracing::error!("encoding a market view failed: {e}");
        ApiError::Internal
    })?))
}

// ---------------------------------------------------------------------------
// GET /api/market/calendar
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    #[serde(default)]
    hours: Option<u32>,
    /// Comma-separated currency codes.
    #[serde(default)]
    currencies: Option<String>,
}

/// `USD,eur` -> `["USD", "EUR"]`; anything that is not a three-letter code is an error.
pub fn parse_currencies(text: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if part.len() != 3 || !part.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(format!(
                "'{part}' is not a currency code; use three letters such as USD"
            ));
        }
        let code = part.to_ascii_uppercase();
        if !out.contains(&code) {
            out.push(code);
        }
    }
    Ok(out)
}

pub async fn calendar_get(
    State(state): State<AppState>,
    Query(query): Query<CalendarQuery>,
) -> Result<Json<Value>, ApiError> {
    let market = market(&state)?;
    let hours = query.hours.unwrap_or(CALENDAR_DEFAULT_HOURS);
    if !(1..=CALENDAR_MAX_HOURS).contains(&hours) {
        return Err(ApiError::BadRequest(format!(
            "hours must be between 1 and {CALENDAR_MAX_HOURS}"
        )));
    }
    let currencies = parse_currencies(query.currencies.as_deref().unwrap_or(""))
        .map_err(ApiError::BadRequest)?;
    let events = market.calendar(hours, &currencies).await?;
    Ok(Json(json!({ "events": events })))
}

// ---------------------------------------------------------------------------
// GET /api/market/news
// ---------------------------------------------------------------------------

pub async fn news_get(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(query): Query<SymbolQuery>,
) -> Result<Json<Value>, ApiError> {
    let market = market(&state)?;
    let symbol = required_symbol(query.symbol)?;
    let resolver = state.resolver_for(&state.user_settings(session.as_ref()).await);
    let report = market.news(&resolver, &symbol).await?;
    Ok(Json(serde_json::to_value(report).map_err(|e| {
        tracing::error!("encoding a news report failed: {e}");
        ApiError::Internal
    })?))
}

// ---------------------------------------------------------------------------
// GET /api/market/journal
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct JournalQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    symbol: Option<String>,
}

pub async fn journal_get(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(query): Query<JournalQuery>,
) -> Result<Json<Value>, ApiError> {
    let market = market(&state)?;
    let limit = query
        .limit
        .unwrap_or(JOURNAL_DEFAULT_LIMIT)
        .clamp(1, JOURNAL_MAX_LIMIT);
    let symbol = match query
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => {
            let resolver = state.resolver_for(&state.user_settings(session.as_ref()).await);
            Some(resolver.canonical(raw).ok_or_else(|| {
                ApiError::BadRequest(format!("'{raw}' is not a usable symbol name"))
            })?)
        }
    };
    let journal = market.journal();
    let (stats, views) = tokio::try_join!(
        journal.stats(symbol.as_deref()),
        journal.list(symbol.as_deref(), limit)
    )
    .map_err(|e| {
        tracing::error!("reading the market journal failed: {e:#}");
        ApiError::Internal
    })?;
    Ok(Json(json!({ "stats": stats, "views": views })))
}

// ---------------------------------------------------------------------------
// POST /api/market/prepare
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareBody {
    view_id: String,
    /// Required: there is no default size (SPEC rule 9).
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    order_type: Option<String>,
}

pub async fn prepare_post(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    body: Bytes,
) -> Result<Json<PreviewDto>, ApiError> {
    let market = market(&state)?;
    let req: PrepareBody = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?;
    let view_id = req.view_id.trim();
    if view_id.is_empty() {
        return Err(ApiError::BadRequest("view_id is required".to_owned()));
    }
    let volume = req
        .volume
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or_else(|| {
            ApiError::BadRequest(
                "volume is required and must be greater than 0: say how many lots to trade"
                    .to_owned(),
            )
        })?;
    let kind = req
        .order_type
        .as_deref()
        .and_then(PrepareKind::parse)
        .ok_or_else(|| {
            ApiError::BadRequest("order_type is required: \"market\" or \"limit\"".to_owned())
        })?;

    let settings = state.user_settings(session.as_ref()).await;
    let resolver = state.resolver_for(&settings);
    let action = market.prepare(&resolver, view_id, volume, kind).await?;

    // Noted before the gate decides, so a refused preview still shows which view it came from.
    state
        .audit()
        .record_as(
            None,
            Some(&state.persona_for(&settings).id),
            AuditKind::MarketPrepare,
            json!({
                "view_id": view_id,
                "volume": volume,
                "order_type": kind.as_str(),
                "action": serde_json::to_value(&action).unwrap_or(Value::Null),
            }),
        )
        .await;
    propose_from_ui(&state, session.as_ref(), action).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currencies_are_trimmed_uppercased_and_deduplicated() {
        assert_eq!(parse_currencies(" usd, EUR ,usd,").unwrap(), ["USD", "EUR"]);
        assert!(parse_currencies("").unwrap().is_empty());
        assert!(parse_currencies("USD,EU").is_err());
        assert!(parse_currencies("US1").is_err());
        assert!(parse_currencies("dollar").is_err());
    }
}

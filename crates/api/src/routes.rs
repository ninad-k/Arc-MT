//! HTTP routes. The contract is `docs/api.md`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::handler::Handler;
use axum::http::request::Parts;
use axum::http::{Extensions, HeaderValue, Method, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use jarvis_bridge_client::BridgeError;
use jarvis_bridge_client::protocol::{Account, Orders, Positions, Price, Side};
use jarvis_gate::{CancelSelector, CloseSelector, KillSwitchState, ProposedAction};
use jarvis_llm::LlmError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::task::JoinSet;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::agent::{ToolCallSummary, TurnUser};
use crate::auth::http::{self as auth_http, MaybeSession, authenticate_headers, peer_ip_of};
use crate::security::{self, Security};
use crate::state::{AppState, Budget, ProviderSlot};
use crate::static_site::StaticSite;
use crate::trading::{ConfirmResponse, PreviewDto, ProposeRequest, ProposeSource, TradingError};
use crate::{market_routes, settings_routes, symbols_routes, voice, ws};

/// Longest accepted chat message, in characters.
pub const MAX_MESSAGE_CHARS: usize = 2000;
/// Largest request body of the JSON routes.
pub const JSON_BODY_LIMIT: usize = 64 * 1024;
/// Largest request body of `POST /api/voice/transcribe` (WAV audio).
pub const VOICE_BODY_LIMIT: usize = 5 * 1024 * 1024;

/// Routes that need no session: the health probe and login.
pub const PUBLIC_ROUTES: [(&str, &str); 2] = [("GET", "/api/health"), ("POST", "/api/auth/login")];

/// The whole API (and the static SPA) as a router. Tests call this with a hand-built
/// [`AppState`].
///
/// Every `/api` route sits behind the session middleware except [`PUBLIC_ROUTES`]; an
/// unknown `/api` path gets the same check (401 before 404). Everything outside `/api` is
/// the static site, when one is configured.
pub fn app(state: AppState) -> Router {
    // CORS is off unless `[server] dev_origins` names origins. The SPA is served from the API's
    // own origin (also in the desktop shell), and the Vite dev proxy is same-origin too.
    let dev_origins: Vec<HeaderValue> = state
        .config()
        .server
        .dev_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(dev_origins)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE])
        .allow_credentials(true);

    // Time limits (NET-1). Most routes answer within `request_timeout_secs`; the ones that wait
    // for a language model, speech or a batch of trades get `long_request_timeout_secs`; the
    // WebSocket has none (the upgrade is answered at once, and what follows is not a request).
    // A request that runs out of time gets a 408 in the standard error shape (`security::apply`).
    let server = &state.config().server;
    let timeout = |secs: u64| {
        TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(secs))
    };
    let (short, long) = (
        timeout(server.request_timeout_secs),
        timeout(server.long_request_timeout_secs),
    );

    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(auth_http::login))
        .layer(short);

    let quick = Router::new()
        .route("/api/status", get(status))
        .route("/api/account", get(account))
        .route("/api/positions", get(positions))
        .route("/api/orders", get(orders))
        .route("/api/watchlist", get(watchlist))
        .route("/api/personas", get(settings_routes::personas))
        .route(
            "/api/settings",
            get(settings_routes::settings_get).put(settings_routes::settings_put),
        )
        .route(
            "/api/settings/limits",
            get(settings_routes::limits_get)
                .put(settings_routes::limits_put)
                .delete(settings_routes::limits_delete),
        )
        .route("/api/symbols", get(symbols_routes::symbols_get))
        .route("/api/symbols/map", put(symbols_routes::map_put))
        .route("/api/symbols/detect", post(symbols_routes::detect_post))
        .route("/api/symbols/catalog", get(symbols_routes::catalog_get))
        .route("/api/symbols/export", get(symbols_routes::export_get))
        .route("/api/market/calendar", get(market_routes::calendar_get))
        .route("/api/market/journal", get(market_routes::journal_get))
        .route("/api/market/prepare", post(market_routes::prepare_post))
        .route("/api/cancel/{token}", post(cancel))
        .route("/api/readback/{token}", post(readback))
        .route("/api/positions/{ticket}/close", post(close_position))
        .route("/api/positions/close", post(close_positions))
        .route("/api/orders/{ticket}/cancel", post(cancel_order))
        .route("/api/killswitch", get(killswitch_get).post(killswitch_set))
        .route("/api/audit", get(audit_log))
        .route("/api/egress", get(egress_log))
        .route("/api/notes/graph", get(crate::notes_routes::graph))
        .route("/api/notes/rescan", post(crate::notes_routes::rescan))
        .route("/api/notes/{id}", get(crate::notes_routes::note))
        .route("/api/notes/capture", post(crate::assistant_routes::capture))
        .route("/api/models", get(crate::assistant_routes::models_list))
        .route("/api/models/{id}", get(crate::assistant_routes::model_get))
        .route(
            "/api/models/{id}/file",
            get(crate::assistant_routes::model_file),
        )
        .route(
            "/api/models/{id}/info",
            put(crate::assistant_routes::model_info),
        )
        // Uploads (spec-model-uploads): their own body limits, checked before the body is read.
        .route(
            "/api/models/upload",
            post(crate::assistant_routes::upload).layer(DefaultBodyLimit::max(
                crate::models3d::MAX_FILE_BYTES + 1024,
            )),
        )
        .route(
            "/api/models/relief",
            post(crate::assistant_routes::relief).layer(DefaultBodyLimit::max(
                crate::models3d::MAX_IMAGE_BYTES + 1024,
            )),
        )
        .route("/api/desk/state", get(crate::desk_routes::state))
        .route(
            "/api/assistant/status",
            get(crate::assistant_routes::status),
        )
        .route("/api/auth/logout", post(auth_http::logout))
        .route("/api/auth/me", get(auth_http::me))
        .layer(short);

    // Chat and transcription also have a per-session request budget (NET-2), checked before the
    // body is read.
    let budget = |which: Budget| from_fn_with_state((state.clone(), which), spend_budget);
    let slow = Router::new()
        .route("/api/chat", post(chat).layer(budget(Budget::Chat)))
        .route("/api/confirm/{token}", post(confirm))
        // Desk mode (spec-nimbus-desk): Pilot asks for a preview like the Prepare-trade button;
        // the pulse reads two bridge calls per symbol.
        .route(
            "/api/desk/plans/{id}/pilot",
            post(crate::desk_routes::pilot),
        )
        .route(
            "/api/desk/pulse",
            get(crate::desk_routes::pulse).layer(budget(Budget::Market)),
        )
        // Claude builds from a picture: as slow as a chat turn, with the chat budget.
        .route(
            "/api/models/from-image",
            post(crate::assistant_routes::from_image)
                .layer(DefaultBodyLimit::max(
                    crate::models3d::MAX_IMAGE_BYTES + 1024,
                ))
                .layer(budget(Budget::Chat)),
        )
        // A view reads a handful of bars, the calendar and news before it answers, so it has
        // the long timeout and its own budget.
        .route(
            "/api/market/view",
            get(market_routes::view_get).layer(budget(Budget::Market)),
        )
        .route(
            "/api/market/news",
            get(market_routes::news_get).layer(budget(Budget::Market)),
        )
        .route("/api/voice/speak", post(voice::speak))
        .route(
            "/api/voice/transcribe",
            post(voice::transcribe)
                .layer(DefaultBodyLimit::max(VOICE_BODY_LIMIT))
                .layer(budget(Budget::Transcribe)),
        )
        .layer(long);

    let sockets = Router::new().route("/api/ws", get(ws::handler));

    let protected = quick
        .merge(slow)
        .merge(sockets)
        .route_layer(from_fn_with_state(
            state.clone(),
            auth_http::require_session,
        ));

    let config = state.config();
    let site = config.server.static_dir.as_deref().map(StaticSite::new);
    let script_hashes = config
        .server
        .static_dir
        .as_deref()
        .map(security::collect_inline_script_hashes)
        .unwrap_or_default();
    let hardening = Arc::new(Security::new(config.tls_enabled(), &script_hashes));
    // Host / Origin checks on /api (see `security::request_guard`).
    let guard = Arc::new(security::RequestGuard::new(
        &config.server.dev_origins,
        !config.auth.enabled,
    ));

    let fallback_state = state.clone();
    let fallback_handler = move |req: Request| {
        let state = fallback_state.clone();
        let site = site.clone();
        async move { fallback(state, site, req).await }
    };
    Router::new()
        .merge(public)
        .merge(protected)
        .fallback(fallback_handler.layer(short))
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT))
        .layer(
            TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
                // Confirmation tokens are bearer secrets: keep them out of the request log.
                tracing::debug_span!(
                    "http_request",
                    method = %req.method(),
                    path = %redact_path(req.uri().path()),
                )
            }),
        )
        .layer(cors)
        .layer(from_fn_with_state(guard, security::request_guard))
        .layer(from_fn_with_state(hardening, security::apply))
        .with_state(state)
}

/// Middleware of the expensive routes (NET-2): one request of the caller's budget, or a 429 with
/// `Retry-After`. The caller is the login session, or the peer address when login is off.
async fn spend_budget(
    State((state, budget)): State<(AppState, Budget)>,
    req: Request,
    next: Next,
) -> Response {
    let key = client_key_of(req.extensions());
    match state.spend_budget(budget, &key) {
        Ok(()) => next.run(req).await,
        Err(retry_after_secs) => {
            tracing::warn!(?budget, "request budget used up");
            ApiError::RateLimited { retry_after_secs }.into_response()
        }
    }
}

/// Unmatched requests: `/api/*` is a JSON 404 (after the session check), anything else is the
/// static site or a plain 404.
async fn fallback(state: AppState, site: Option<StaticSite>, req: Request) -> Response {
    let path = req.uri().path();
    if path == "/api" || path.starts_with("/api/") {
        return match authenticate_headers(&state, req.headers()).await {
            Err(e) => e.into_response(),
            Ok(_) => ApiError::NotFound("no such endpoint".to_owned()).into_response(),
        };
    }
    match site {
        Some(site) => site.serve(req).await,
        None => ApiError::NotFound("no web UI is configured (server.static_dir)".to_owned())
            .into_response(),
    }
}

/// `GET /api/health`: liveness for service monitors and the desktop launcher. No auth, no
/// details.
async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// `/api/confirm/<token>` -> `/api/confirm/<redacted>` (same for cancel and readback).
fn redact_path(path: &str) -> String {
    for prefix in ["/api/confirm/", "/api/cancel/", "/api/readback/"] {
        if path.starts_with(prefix) {
            return format!("{prefix}<redacted>");
        }
    }
    path.to_owned()
}

// ---------------------------------------------------------------------------
// Errors: `{"error": {"code", "message"}}`
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    BridgeUnavailable(String),
    BridgeError(String),
    LlmError(String),
    /// Additive: DEMO configured but the account is real (rule 7).
    ModeMismatch,
    /// 401: no valid session, a failed login, or a kill-switch release the guard refused.
    Unauthorized(String),
    /// 403: the request is understood and refused (foreign WebSocket origin).
    Forbidden(String),
    /// 404 for an unknown `/api` path or a missing web UI.
    NotFound(String),
    /// 405 (normalised from the framework's empty response).
    MethodNotAllowed,
    /// 408: the request took longer than the server allows (NET-1).
    RequestTimeout,
    /// 413: body over the route's limit.
    PayloadTooLarge,
    /// 422: the gate refused to build a preview.
    GateRejected(String),
    /// 502: the EA is older than 0.5.0 and has no price history or calendar commands.
    EaOutdated(String),
    /// 410: a market view is past its `valid_until`.
    ViewExpired(String),
    /// 422: a market view has no trade setup to prepare.
    NoSetup(String),
    /// 422 `limit_not_stricter`: a limit override that is looser than the config.
    LimitNotStricter(String),
    TokenUnknown,
    TokenExpired,
    TokenUsed,
    /// 423: the kill switch is engaged.
    KillSwitch,
    /// 409: MT5 is on another broker account than the one a preview was made for.
    BrokerChanged,
    /// 422: a symbol override names a symbol the broker's catalog does not have.
    UnknownBrokerSymbol(String),
    /// 503: the instrument catalog could not be loaded, so symbols cannot be mapped.
    SymbolsUnavailable(String),
    RateLimited {
        retry_after_secs: u64,
    },
    /// 503: voice disabled or models missing (voice task).
    VoiceUnavailable(String),
    /// 502: MT5 rejected a write.
    TradeFailed(String),
    /// 504: a write was sent and no reply came back.
    OutcomeUnknown(String),
    Internal,
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            ApiError::BridgeUnavailable(m) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "bridge_unavailable",
                m.clone(),
            ),
            ApiError::BridgeError(m) => (StatusCode::BAD_GATEWAY, "bridge_error", m.clone()),
            ApiError::LlmError(m) => (StatusCode::BAD_GATEWAY, "llm_error", m.clone()),
            ApiError::ModeMismatch => (
                StatusCode::CONFLICT,
                "mode_mismatch",
                "Jarvis is configured for a demo account but MetaTrader 5 reports a real \
                 account. Chat is disabled. Switch the terminal to a demo account, or set \
                 mode = \"live\" on purpose."
                    .to_owned(),
            ),
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, "unauthorized", m.clone()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone()),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.clone()),
            ApiError::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "this method is not allowed on that endpoint".to_owned(),
            ),
            ApiError::RequestTimeout => (
                StatusCode::REQUEST_TIMEOUT,
                "request_timeout",
                "the request took too long and was stopped".to_owned(),
            ),
            ApiError::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "the request body is too large".to_owned(),
            ),
            ApiError::GateRejected(m) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "gate_rejected", m.clone())
            }
            ApiError::EaOutdated(m) => (StatusCode::BAD_GATEWAY, "ea_outdated", m.clone()),
            ApiError::ViewExpired(m) => (StatusCode::GONE, "view_expired", m.clone()),
            ApiError::NoSetup(m) => (StatusCode::UNPROCESSABLE_ENTITY, "no_setup", m.clone()),
            ApiError::LimitNotStricter(m) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "limit_not_stricter",
                m.clone(),
            ),
            ApiError::TokenUnknown => (
                StatusCode::NOT_FOUND,
                "token_unknown",
                "confirmation token not found".to_owned(),
            ),
            ApiError::TokenExpired => (
                StatusCode::GONE,
                "token_expired",
                "the confirmation window has closed; ask for the action again".to_owned(),
            ),
            ApiError::TokenUsed => (
                StatusCode::CONFLICT,
                "token_used",
                "this confirmation was already used or cancelled".to_owned(),
            ),
            ApiError::KillSwitch => (
                StatusCode::LOCKED,
                "kill_switch",
                "the kill switch is engaged; trading is disabled".to_owned(),
            ),
            ApiError::BrokerChanged => (
                StatusCode::CONFLICT,
                "broker_changed",
                "the broker account in MetaTrader 5 changed since this was proposed; nothing \
                 was sent. Ask for the action again."
                    .to_owned(),
            ),
            ApiError::SymbolsUnavailable(m) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "symbols_unavailable",
                m.clone(),
            ),
            ApiError::UnknownBrokerSymbol(m) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unknown_broker_symbol",
                m.clone(),
            ),
            ApiError::RateLimited { retry_after_secs } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("too many requests; retry in {retry_after_secs} s"),
            ),
            ApiError::VoiceUnavailable(m) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "voice_unavailable",
                m.clone(),
            ),
            ApiError::TradeFailed(m) => (StatusCode::BAD_GATEWAY, "trade_failed", m.clone()),
            ApiError::OutcomeUnknown(m) => {
                (StatusCode::GATEWAY_TIMEOUT, "outcome_unknown", m.clone())
            }
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error; see the server log".to_owned(),
            ),
        }
    }
}

impl From<BridgeError> for ApiError {
    fn from(e: BridgeError) -> Self {
        match e {
            BridgeError::Timeout(_) | BridgeError::Transport(_) => {
                ApiError::BridgeUnavailable(e.to_string())
            }
            BridgeError::Remote { .. } | BridgeError::Protocol(_) => {
                ApiError::BridgeError(e.to_string())
            }
        }
    }
}

impl From<LlmError> for ApiError {
    fn from(e: LlmError) -> Self {
        ApiError::LlmError(e.to_string())
    }
}

impl From<TradingError> for ApiError {
    fn from(e: TradingError) -> Self {
        use jarvis_gate::{GateError, TokenError};
        match e {
            TradingError::Gate(GateError::KillSwitchEngaged) | TradingError::KillSwitch => {
                ApiError::KillSwitch
            }
            TradingError::Gate(g) => ApiError::GateRejected(g.to_string()),
            TradingError::IntentMismatch(m) => ApiError::GateRejected(m.to_string()),
            TradingError::UserIsAnswering => ApiError::GateRejected(e.to_string()),
            TradingError::RecheckRejected(m) => ApiError::GateRejected(m),
            TradingError::Token(TokenError::Unknown) => ApiError::TokenUnknown,
            TradingError::Token(TokenError::Expired) => ApiError::TokenExpired,
            TradingError::Token(TokenError::AlreadyUsed) => ApiError::TokenUsed,
            TradingError::Token(TokenError::KillSwitchEngaged) => ApiError::KillSwitch,
            TradingError::ModeMismatch => ApiError::ModeMismatch,
            TradingError::BrokerChanged => ApiError::BrokerChanged,
            TradingError::Bridge(b) => b.into(),
            TradingError::Snapshot(m) => ApiError::BridgeError(m),
            TradingError::RateLimited { retry_after_secs } => {
                ApiError::RateLimited { retry_after_secs }
            }
            TradingError::TradeFailed(m) => ApiError::TradeFailed(m),
            TradingError::OutcomeUnknown(m) => ApiError::OutcomeUnknown(m),
            TradingError::Internal(m) => {
                tracing::error!("trading internal error: {m}");
                ApiError::Internal
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        let retry_after = match &self {
            ApiError::RateLimited { retry_after_secs } => Some(*retry_after_secs),
            _ => None,
        };
        let mut response = (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response();
        if let Some(secs) = retry_after
            && let Ok(value) = HeaderValue::from_str(&secs.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

// ---------------------------------------------------------------------------
// GET /api/status
// ---------------------------------------------------------------------------

async fn status(State(state): State<AppState>, MaybeSession(session): MaybeSession) -> Json<Value> {
    let settings = state.user_settings(session.as_ref()).await;
    let persona = state.persona_for(&settings);
    let started = Instant::now();
    let ping = state.ping_only().await;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if ping.is_ok() {
        // The same ping tells whether the account switched to another broker.
        state.broker().observe().await;
    }
    let bridge = match ping {
        Ok(info) => json!({
            "connected": true,
            "latency_ms": latency_ms,
            "ea_version": info.ea_version,
            "protocol_version": info.protocol_version,
            "trade_mode": info.trade_mode,
            "terminal_connected": info.terminal_connected,
            "error": null,
        }),
        Err(e) => json!({
            "connected": false,
            "latency_ms": null,
            "ea_version": null,
            "protocol_version": null,
            "trade_mode": null,
            "terminal_connected": null,
            "error": e.to_string(),
        }),
    };
    let providers: Vec<Value> = state
        .providers()
        .iter()
        .map(|p| {
            json!({
                "name": p.config.name,
                "kind": p.kind_str(),
                "model": p.config.model,
                "available": matches!(p.slot, ProviderSlot::Available(_)),
            })
        })
        .collect();
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "mode": state.config().mode.as_str(),
        "mode_mismatch": state.mode_mismatch(),
        "bridge": bridge,
        "llm": { "default": state.config().llm.default, "providers": providers },
        "kill_switch": state.trading().kill_switch_state(),
        "auth": state.auth_status(session.as_ref()),
        "voice": state.voice_status(),
        "limits": state.trading().limits(),
        "persona": { "id": persona.id, "name": persona.name },
        "broker": state.broker().book().status_view(),
        "market": state.market_status(),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/account, /api/positions
// ---------------------------------------------------------------------------

async fn account(State(state): State<AppState>) -> Result<Json<Account>, ApiError> {
    let account = state.bridge().get_account().await?;
    state.observe_trade_mode(account.trade_mode);
    Ok(Json(account))
}

/// Serialise rows that carry a broker `symbol` and add `canonical` next to it.
fn with_canonical<T: Serialize>(state: &AppState, rows: &[T]) -> Vec<Value> {
    let book = state.broker().book();
    rows.iter()
        .filter_map(|row| serde_json::to_value(row).ok())
        .map(|mut value| {
            if let Some(object) = value.as_object_mut()
                && let Some(symbol) = object.get("symbol").and_then(Value::as_str)
            {
                let canonical = book.canonical_of_broker_symbol(symbol);
                object.insert("canonical".to_owned(), Value::String(canonical));
            }
            value
        })
        .collect()
}

/// `GET /api/positions`: the bridge's `Positions` shape, each row with its `canonical` name.
async fn positions(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let positions: Positions = state.bridge().get_positions(None).await?;
    Ok(Json(
        json!({ "positions": with_canonical(&state, &positions.positions) }),
    ))
}

/// `GET /api/orders`: pending orders, in the bridge's `Order` shape, with `canonical`.
async fn orders(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let orders: Orders = state.bridge().get_orders(None).await?;
    Ok(Json(
        json!({ "orders": with_canonical(&state, &orders.orders) }),
    ))
}

// ---------------------------------------------------------------------------
// GET /api/watchlist
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct WatchlistError {
    /// The name as it is on the watchlist (canonical).
    symbol: String,
    canonical: String,
    message: String,
}

#[derive(Serialize)]
struct Watchlist {
    /// Prices as the bridge returned them (`symbol` is the broker's name), with `canonical`.
    prices: Vec<Value>,
    errors: Vec<WatchlistError>,
}

async fn watchlist(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> Result<Json<Watchlist>, ApiError> {
    // The caller's own watchlist (the configured one until they change it), resolved with
    // their aliases.
    let settings = state.user_settings(session.as_ref()).await;
    let resolver = state.resolver_for(&settings);
    let names = settings.watchlist.clone();
    let mut tasks = JoinSet::new();
    for (index, name) in names.iter().enumerate() {
        let bridge = state.bridge().clone();
        let resolved = resolver.resolve(name);
        let name = resolver.canonical(name).unwrap_or_else(|| name.clone());
        tasks.spawn(async move {
            let result = match resolved {
                Some(symbol) => bridge.get_price(&symbol).await,
                None => Err(BridgeError::Remote {
                    message: format!("'{name}' is not a usable symbol name"),
                    retcode: 0,
                }),
            };
            (index, name, result)
        });
    }
    let mut results = Vec::with_capacity(names.len());
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(r) => results.push(r),
            Err(e) => {
                tracing::error!("watchlist task failed: {e}");
                return Err(ApiError::Internal);
            }
        }
    }
    results.sort_by_key(|(index, _, _)| *index);

    let mut prices: Vec<Price> = Vec::new();
    let mut errors = Vec::new();
    let mut unreachable: Option<BridgeError> = None;
    for (_, name, result) in results {
        match result {
            Ok(price) => prices.push(price),
            Err(e) => {
                if matches!(e, BridgeError::Timeout(_) | BridgeError::Transport(_)) {
                    unreachable.get_or_insert_with(|| e.clone());
                }
                errors.push(WatchlistError {
                    canonical: name.clone(),
                    symbol: name,
                    message: e.to_string(),
                });
            }
        }
    }
    // Every symbol failed because the bridge itself is down: that is a 503, not a list of errors.
    if prices.is_empty()
        && let Some(e) = unreachable
    {
        return Err(e.into());
    }
    Ok(Json(Watchlist {
        prices: with_canonical(&state, &prices),
        errors,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/chat
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChatRequestBody {
    #[serde(default)]
    session_id: Option<String>,
    message: String,
    #[serde(default)]
    provider: Option<String>,
    /// The message was spoken: previews get the after-read-back confirm window.
    #[serde(default)]
    voice: bool,
    /// `"trading"` (default) or `"assistant"` (spec-assistant-mode).
    #[serde(default)]
    mode: Option<String>,
    /// Assistant mode: the model on the user's screen, `{id}` (spec-model-uploads).
    #[serde(default)]
    stage_model: Option<StageModelBody>,
}

#[derive(Deserialize)]
struct StageModelBody {
    id: String,
}

#[derive(Serialize)]
struct ChatResponseBody {
    session_id: Uuid,
    reply: String,
    provider: String,
    persona: String,
    tool_calls: Vec<ToolCallSummary>,
    preview: Option<PreviewDto>,
    /// The notes the reply drew on, `{id, title}` (ids as in `/api/notes/graph`).
    notes: Vec<crate::agent::CitedNote>,
    /// `"trading"` or `"assistant"`.
    mode: &'static str,
    /// The model that answered: `{provider, model}`.
    brain: Value,
    /// 3D models built in this turn (Assistant mode).
    models: Vec<crate::assistant::ModelRef>,
    /// Notes written in this turn by "remember that ..." (Assistant mode).
    captured: Vec<jarvis_notes::Captured>,
    /// This month's Claude spending (Assistant mode, and Desk mode on Claude).
    spend: Option<crate::spend::SpendStatus>,
    /// Desk mode: `{findings, chart, plans}` (spec-nimbus-desk). Absent in the other modes.
    #[serde(skip_serializing_if = "Option::is_none")]
    desk: Option<Value>,
}

async fn chat(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    body: Bytes,
) -> Result<Json<ChatResponseBody>, ApiError> {
    // Parse by hand so malformed bodies get the standard error shape.
    let req: ChatRequestBody = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?;

    let message = req.message.trim();
    if message.is_empty() {
        return Err(ApiError::BadRequest("message must not be empty".to_owned()));
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(ApiError::BadRequest(format!(
            "message is too long (maximum {MAX_MESSAGE_CHARS} characters)"
        )));
    }
    let session_id = match req
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(s) => Some(
            Uuid::parse_str(s)
                .map_err(|_| ApiError::BadRequest("session_id is not a valid UUID".to_owned()))?,
        ),
    };
    match req.mode.as_deref().map(str::trim).unwrap_or("trading") {
        "" | "trading" => {}
        "assistant" => {
            let stage = req.stage_model.map(|s| s.id);
            return assistant_chat(&state, session_id, message, req.voice, stage).await;
        }
        "desk" => {
            return desk_chat(&state, session.as_ref(), session_id, message, req.voice).await;
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown mode '{other}' (use \"trading\", \"assistant\" or \"desk\")"
            )));
        }
    }
    let settings = state.user_settings(session.as_ref()).await;
    let explicit = req
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // Without a provider in the request, the user's preferred one (if it is usable right now),
    // else the server default.
    let preferred = settings.llm_provider.as_deref().filter(|name| {
        state
            .provider(name)
            .is_some_and(|e| matches!(e.slot, ProviderSlot::Available(_)))
    });
    let provider_name = explicit
        .or(preferred)
        .unwrap_or(&state.config().llm.default);
    let entry = state.provider(provider_name).ok_or_else(|| {
        let known: Vec<&str> = state
            .providers()
            .iter()
            .map(|p| p.config.name.as_str())
            .collect();
        ApiError::BadRequest(format!(
            "unknown provider '{provider_name}' (available names: {})",
            known.join(", ")
        ))
    })?;
    let provider = match &entry.slot {
        ProviderSlot::Available(p) => p.clone(),
        ProviderSlot::Unavailable(reason) => {
            return Err(ApiError::BadRequest(format!(
                "provider '{provider_name}' is not available: {reason}"
            )));
        }
    };

    if state.mode_mismatch() {
        return Err(ApiError::ModeMismatch);
    }

    let user = TurnUser {
        persona: state.persona_for(&settings),
        resolver: state.resolver_for(&settings),
    };
    let outcome = match state
        .agent()
        .run_turn(provider.as_ref(), &user, session_id, message, req.voice)
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(provider = provider_name, "LLM call failed: {e}");
            return Err(e.into());
        }
    };
    Ok(Json(ChatResponseBody {
        session_id: outcome.session_id,
        reply: outcome.reply,
        provider: provider_name.to_owned(),
        persona: outcome.persona,
        tool_calls: outcome.tool_calls,
        preview: outcome.preview,
        notes: outcome.notes,
        mode: "trading",
        brain: json!({ "provider": provider_name, "model": provider.model() }),
        models: vec![],
        captured: vec![],
        spend: None,
        desk: None,
    }))
}

/// `POST /api/chat` with `mode: "assistant"`: the Assistant mode loop, which has no MT5 tools.
async fn assistant_chat(
    state: &AppState,
    session_id: Option<Uuid>,
    message: &str,
    voice: bool,
    stage_id: Option<String>,
) -> Result<Json<ChatResponseBody>, ApiError> {
    let assistant = state.assistant().ok_or_else(|| {
        ApiError::BadRequest("Assistant mode is turned off ([assistant] enabled = false)".into())
    })?;
    let config = &state.config().assistant;
    let model = state
        .provider(&config.provider)
        .map(|e| e.config.model.clone());
    let brain = json!({ "provider": config.provider, "model": model });
    let reply_only = |reply: String| ChatResponseBody {
        session_id: session_id.unwrap_or_else(Uuid::new_v4),
        reply,
        provider: config.provider.clone(),
        persona: crate::assistant::ASSISTANT_PERSONA.to_owned(),
        tool_calls: vec![],
        preview: None,
        notes: vec![],
        mode: "assistant",
        brain: brain.clone(),
        models: vec![],
        captured: vec![],
        spend: Some(assistant.spend().status(chrono::Utc::now())),
        desk: None,
    };
    // Without a usable provider nothing is sent anywhere; the reply says how to set it up.
    let provider = match state.assistant_provider() {
        Ok(p) => p,
        Err(reason) => {
            return Ok(Json(reply_only(format!(
                "Assistant mode needs Claude, and it is not set up yet ({reason}). Set \
                 ANTHROPIC_API_KEY in the server's environment or .env file, then restart Arc MT."
            ))));
        }
    };
    // A task of its own, so the turn (and its spending record) finishes even if this request
    // is dropped by the time limit or a closed connection.
    // The model on screen, looked up here: its title and kind come from the store, not the client.
    let on_screen = match stage_id.filter(|id| crate::models3d::valid_id(id)) {
        None => None,
        Some(id) => {
            let store = assistant.models().clone();
            tokio::task::spawn_blocking(move || store.get(&id))
                .await
                .ok()
                .flatten()
                .map(|m| crate::assistant::OnScreen {
                    id: m.id,
                    title: m.title,
                    kind: m.kind,
                })
        }
    };
    let turn = {
        let assistant = assistant.clone();
        let message = message.to_owned();
        tokio::spawn(async move {
            assistant
                .run_turn_on_screen(provider.as_ref(), session_id, &message, voice, on_screen)
                .await
        })
    };
    let outcome = match turn.await.map_err(|_| ApiError::Internal)? {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(provider = %config.provider, "assistant LLM call failed: {e}");
            return Err(e.into());
        }
    };
    Ok(Json(ChatResponseBody {
        session_id: outcome.session_id,
        reply: outcome.reply,
        provider: config.provider.clone(),
        persona: crate::assistant::ASSISTANT_PERSONA.to_owned(),
        tool_calls: outcome.tool_calls,
        preview: None,
        notes: outcome.notes,
        mode: "assistant",
        brain,
        models: outcome.models,
        captured: outcome.captured,
        spend: Some(outcome.spend),
        desk: None,
    }))
}

/// `POST /api/chat` with `mode: "desk"`: Friday and the specialists (spec-nimbus-desk).
async fn desk_chat(
    state: &AppState,
    session: Option<&crate::auth::Session>,
    session_id: Option<Uuid>,
    message: &str,
    voice: bool,
) -> Result<Json<ChatResponseBody>, ApiError> {
    let desk = state
        .desk()
        .ok_or_else(|| {
            ApiError::BadRequest("Desk mode is turned off ([desk] enabled = false)".into())
        })?
        .clone();
    let config = &state.config().desk;
    let model = state
        .provider(&config.provider)
        .map(|e| e.config.model.clone());
    let brain = json!({ "provider": config.provider, "model": model });
    let (provider, metered) = match state.desk_provider() {
        Ok(p) => p,
        Err(reason) => {
            return Ok(Json(ChatResponseBody {
                session_id: session_id.unwrap_or_else(Uuid::new_v4),
                reply: format!(
                    "Desk mode is not set up yet ({reason}). Set ANTHROPIC_API_KEY in the server's \
                     environment or .env file and restart, or point [desk] provider at a local model."
                ),
                provider: config.provider.clone(),
                persona: crate::desk::DESK_PERSONA.to_owned(),
                tool_calls: vec![],
                preview: None,
                notes: vec![],
                mode: "desk",
                brain,
                models: vec![],
                captured: vec![],
                spend: Some(desk.spend().status(chrono::Utc::now())),
                desk: Some(json!({ "findings": [], "chart": null, "plans": [] })),
            }));
        }
    };
    if state.mode_mismatch() {
        return Err(ApiError::ModeMismatch);
    }
    let settings = state.user_settings(session).await;
    let resolver = state.resolver_for(&settings);
    let user = crate::desk::DeskUser {
        intent: state.agent().tools().parse_intent(message),
        resolver,
        watchlist: settings.watchlist.clone(),
    };
    // A task of its own, so the turn (and its spending record) finishes even if this request
    // is dropped by the time limit or a closed connection.
    let turn = {
        let desk = desk.clone();
        let message = message.to_owned();
        tokio::spawn(async move {
            desk.run_turn(
                provider.as_ref(),
                metered,
                &user,
                session_id,
                &message,
                voice,
            )
            .await
        })
    };
    let outcome = match turn.await.map_err(|_| ApiError::Internal)? {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(provider = %config.provider, "desk LLM call failed: {e}");
            return Err(e.into());
        }
    };
    Ok(Json(ChatResponseBody {
        session_id: outcome.session_id,
        reply: outcome.reply,
        provider: config.provider.clone(),
        persona: crate::desk::DESK_PERSONA.to_owned(),
        tool_calls: outcome.tool_calls,
        preview: outcome.preview,
        notes: vec![],
        mode: "desk",
        brain,
        models: vec![],
        captured: vec![],
        spend: outcome.spend,
        desk: Some(
            json!({ "findings": outcome.findings, "chart": outcome.chart,
                           "plans": outcome.plans }),
        ),
    }))
}

// ---------------------------------------------------------------------------
// Trading: confirm / cancel / readback
// ---------------------------------------------------------------------------

/// Who is calling, for rate limits: the login session when there is one (so `/api/confirm`
/// is limited per session), otherwise the peer IP. Requests with no peer address
/// (in-process tests) share one bucket.
fn client_key(parts: &Parts) -> String {
    client_key_of(&parts.extensions)
}

fn client_key_of(extensions: &Extensions) -> String {
    match extensions.get::<crate::auth::Session>() {
        Some(session) => format!("session:{}", session.id),
        None => peer_ip_of(extensions),
    }
}

async fn confirm(
    State(state): State<AppState>,
    Path(token): Path<String>,
    parts: Parts,
) -> Result<Json<ConfirmResponse>, ApiError> {
    let key = client_key(&parts);
    let response = state
        .router()
        .for_token(&token)
        .confirm(&token, &key)
        .await?;
    Ok(Json(response))
}

async fn cancel(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.router().for_token(&token).cancel(&token).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn readback(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let expires_at_ms = state.router().for_token(&token).readback_finished(&token)?;
    Ok(Json(json!({ "expires_at_ms": expires_at_ms })))
}

// ---------------------------------------------------------------------------
// Trading: UI buttons (same gate path as the propose tools)
// ---------------------------------------------------------------------------

/// Parse an optional JSON body; an empty body means "all defaults".
fn optional_body<T: serde::de::DeserializeOwned + Default>(body: &Bytes) -> Result<T, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))
}

pub(crate) async fn propose_from_ui(
    state: &AppState,
    session: Option<&crate::auth::Session>,
    action: ProposedAction,
) -> Result<Json<PreviewDto>, ApiError> {
    propose_sized_from_ui(state, session, action, None).await
}

/// [`propose_from_ui`] for a new order whose volume is worked out from `risk_percent` of
/// equity and its stop loss (`None`: the action's own volume is used).
pub(crate) async fn propose_sized_from_ui(
    state: &AppState,
    session: Option<&crate::auth::Session>,
    action: ProposedAction,
    risk_percent: Option<f64>,
) -> Result<Json<PreviewDto>, ApiError> {
    // The audit rows of a button press carry the persona the user has active.
    let settings = state.user_settings(session).await;
    // MT5 or Alpaca, by the symbol or ticket the action names (spec-alpaca).
    let trading = state.router().for_action(&action).await.clone();
    let preview = trading
        .propose(ProposeRequest {
            action,
            session: None,
            voice: false,
            source: ProposeSource::Ui,
            persona: Some(state.persona_for(&settings).id.clone()),
            risk_percent,
        })
        .await?;
    Ok(Json(preview))
}

#[derive(Default, Deserialize)]
struct CloseOneBody {
    /// Partial close volume; absent or null closes the whole position.
    #[serde(default)]
    volume: Option<f64>,
}

/// A ticket from the URL; a bad one gets the standard error shape.
fn parse_ticket(raw: &str) -> Result<u64, ApiError> {
    raw.parse::<u64>()
        .map_err(|_| ApiError::BadRequest(format!("'{raw}' is not a valid ticket number")))
}

async fn close_position(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(ticket): Path<String>,
    body: Bytes,
) -> Result<Json<PreviewDto>, ApiError> {
    let ticket = parse_ticket(&ticket)?;
    let body: CloseOneBody = optional_body(&body)?;
    propose_from_ui(
        &state,
        session.as_ref(),
        ProposedAction::Close(CloseSelector {
            tickets: vec![ticket],
            volume: body.volume,
            ..CloseSelector::default()
        }),
    )
    .await
}

#[derive(Default, Deserialize)]
struct CloseManyBody {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    side: Option<Side>,
    #[serde(default)]
    all: bool,
}

async fn close_positions(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    body: Bytes,
) -> Result<Json<PreviewDto>, ApiError> {
    let body: CloseManyBody = optional_body(&body)?;
    let resolver = state.resolver_for(&state.user_settings(session.as_ref()).await);
    let symbol =
        match body
            .symbol
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            None => None,
            Some(s) => Some(resolver.resolve(s).ok_or_else(|| {
                ApiError::BadRequest(format!("'{s}' is not a usable symbol name"))
            })?),
        };
    propose_from_ui(
        &state,
        session.as_ref(),
        ProposedAction::Close(CloseSelector {
            tickets: Vec::new(),
            symbol,
            side: body.side,
            all: body.all,
            volume: None,
        }),
    )
    .await
}

async fn cancel_order(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Path(ticket): Path<String>,
) -> Result<Json<PreviewDto>, ApiError> {
    let ticket = parse_ticket(&ticket)?;
    propose_from_ui(
        &state,
        session.as_ref(),
        ProposedAction::Cancel(CancelSelector {
            tickets: vec![ticket],
            ..CancelSelector::default()
        }),
    )
    .await
}

// ---------------------------------------------------------------------------
// Kill switch
// ---------------------------------------------------------------------------

async fn killswitch_get(State(state): State<AppState>) -> Json<KillSwitchState> {
    Json(state.trading().kill_switch_state())
}

#[derive(Deserialize)]
struct KillSwitchBody {
    engaged: bool,
    #[serde(default)]
    reason: Option<String>,
    /// Required to release when auth is enabled (checked by the state's `ReleaseGuard`).
    #[serde(default)]
    totp: Option<String>,
}

async fn killswitch_set(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    body: Bytes,
) -> Result<Json<KillSwitchState>, ApiError> {
    let req: KillSwitchBody = serde_json::from_slice(&body)
        .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?;
    let new_state = if req.engaged {
        let reason = req
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .unwrap_or("manual");
        let reason: String = reason.chars().take(200).collect();
        let engaged = state.trading().engage_kill_switch(&reason).await;
        // The switch is shared; the other venues' live previews are revoked too.
        for t in state.router().all().into_iter().skip(1) {
            t.revoke_tokens();
        }
        engaged
    } else {
        state
            .release_guard()
            .authorize_release(session.as_ref(), req.totp.as_deref())
            .await
            .map_err(ApiError::Unauthorized)?;
        state.trading().release_kill_switch().await
    };
    Ok(Json(new_state))
}

// ---------------------------------------------------------------------------
// GET /api/egress
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EgressQuery {
    #[serde(default)]
    since: Option<u64>,
}

/// Outbound requests to other services, oldest first: the last 200, or those after `since`.
/// `non_loopback` lists the hosts that are not this machine.
async fn egress_log(State(state): State<AppState>, Query(q): Query<EgressQuery>) -> Json<Value> {
    let log = state.egress();
    let entries = log.since(q.since.unwrap_or(0));
    Json(json!({
        "entries": entries,
        "non_loopback": crate::egress::non_loopback_hosts(&entries),
        "last_seq": log.last_seq(),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/audit
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    before_id: Option<i64>,
}

const AUDIT_DEFAULT_LIMIT: u32 = 100;
const AUDIT_MAX_LIMIT: u32 = 500;

async fn audit_log(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = q
        .limit
        .unwrap_or(AUDIT_DEFAULT_LIMIT)
        .clamp(1, AUDIT_MAX_LIMIT);
    let rows = state
        .audit()
        .recent(limit, q.before_id)
        .await
        .map_err(|e| {
            tracing::error!("reading the audit log failed: {e:#}");
            ApiError::Internal
        })?;
    let entries: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "ts": r.ts,
                "session_id": r.session_id,
                "kind": r.kind,
                "payload": r.payload,
                "persona": r.persona,
            })
        })
        .collect();
    Ok(Json(json!({ "entries": entries })))
}

#[cfg(test)]
mod tests {
    use super::redact_path;

    #[test]
    fn confirmation_tokens_are_redacted_from_logged_paths() {
        assert_eq!(
            redact_path("/api/confirm/abcdef"),
            "/api/confirm/<redacted>"
        );
        assert_eq!(redact_path("/api/cancel/abcdef"), "/api/cancel/<redacted>");
        assert_eq!(
            redact_path("/api/readback/abcdef"),
            "/api/readback/<redacted>"
        );
        assert_eq!(redact_path("/api/status"), "/api/status");
        assert_eq!(
            redact_path("/api/positions/7/close"),
            "/api/positions/7/close"
        );
    }
}

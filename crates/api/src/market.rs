//! Market views: price bars, the economic calendar and news in, one scored and journaled
//! view out (`tasks/spec-market-view.md`, section 4).
//!
//! A view is information only. Nothing here can place or propose a trade by itself: turning a
//! view into a preview is [`MarketService::prepare`], which builds an ordinary order intent
//! that still goes through `Trading::propose` and every limit (SPEC rules 1, 3 and 9).
//!
//! Time. Bars, events and `now` handed to the engine are all broker server time (unix
//! seconds). The API shows UTC, converted with the offset the [`ServerClock`] has learned.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use jarvis_bridge_client::protocol::{
    self as wire, CalendarEvent, CalendarQuery, ExecType, Importance as WireImportance,
    RETCODE_UNKNOWN_COMMAND, RatesQuery, Side as WireSide,
};
use jarvis_bridge_client::{BridgeClient, BridgeError};
use jarvis_gate::{OrderIntent, ProposedAction};
use jarvis_market::{
    Bar, Bias, EventInput, Factor, Importance, Levels, NewsInput, Price, Setup, Side, Timeframe,
    TimeframeSummary, ViewConfig, ViewInput, analyze,
};
use jarvis_news::{Headline, NewsError, NewsQuery, NewsService, NewsSummary, currency_terms};
use jarvis_symbols::InstrumentCatalog;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::audit::{Audit, AuditKind};
use crate::clock::ServerClock;
use crate::config::MarketConfig;
use crate::market_journal::{Journal, JournalEntry, M5_SECS, score};
use crate::symbols::SymbolResolver;

/// The longest a news provider may hold up a view. News is optional; the view goes on without.
pub const NEWS_TIMEOUT: Duration = Duration::from_secs(4);
/// The calendar window of a view: this far back and ahead of now.
const CALENDAR_LOOKBACK_SECS: i64 = 3600;
const CALENDAR_LOOKAHEAD_SECS: i64 = 24 * 3600;
/// Bars asked for each frame.
const BARS_INTRADAY: u32 = 300;
const BARS_DAILY: u32 = 30;
/// Most M5 bars one evaluation fetch asks for (the bridge allows 1000).
const EVALUATION_BARS_MAX: i64 = 1000;
/// Headlines in a news answer.
const HEADLINES: usize = 5;
/// The longest headline the model is given, in characters.
pub const HEADLINE_CHARS: usize = 200;
/// Events returned to the model by the calendar tool.
pub const TOOL_EVENTS_MAX: usize = 40;

/// Who asked for a view, for the audit log.
#[derive(Debug, Clone, Default)]
pub struct Actor {
    pub session: Option<Uuid>,
    pub persona: Option<String>,
    /// `tool` (the model asked) or `ui` (a button or the HTTP API).
    pub source: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum MarketError {
    #[error("'{0}' is not a usable symbol name")]
    UnusableSymbol(String),
    /// The EA answered "unknown command" to `get_rates` or `get_calendar`.
    #[error(
        "The MetaTrader 5 expert advisor is too old for market views: it cannot send price \
         history or the economic calendar. Update JarvisBridge to version 0.5.0 or newer \
         (install it again with scripts/install-ea.ps1, then restart the EA)."
    )]
    EaOutdated,
    #[error(transparent)]
    Bridge(BridgeError),
    #[error("no such market view")]
    ViewNotFound,
    #[error("this market view has expired; ask for a new one")]
    ViewExpired,
    #[error("this market view has no trade setup (it stood aside or the bias was neutral)")]
    NoSetup,
    #[error("{0}")]
    Invalid(String),
    #[error("the market journal could not be read or written")]
    Storage(#[source] anyhow::Error),
}

impl From<BridgeError> for MarketError {
    fn from(e: BridgeError) -> Self {
        match e {
            BridgeError::Remote { retcode, .. } if retcode == RETCODE_UNKNOWN_COMMAND => {
                MarketError::EaOutdated
            }
            other => MarketError::Bridge(other),
        }
    }
}

// ---------------------------------------------------------------------------
// The HTTP shapes
// ---------------------------------------------------------------------------

/// A calendar event as the API shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventDto {
    /// ISO 8601, UTC.
    pub time: String,
    /// Whole minutes from the moment the view or the answer was made; negative once passed.
    pub minutes_away: i64,
    pub currency: String,
    /// ISO country code; only the calendar endpoint and tool have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    pub title: String,
    pub importance: Importance,
    pub actual: Option<f64>,
    pub forecast: Option<f64>,
    pub previous: Option<f64>,
    pub unit: Option<String>,
}

/// The news section of a view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsDto {
    pub configured: bool,
    pub score: f64,
    pub label: String,
    pub count: u32,
    pub headlines: Vec<Headline>,
}

/// `GET /api/market/view`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketViewDto {
    pub id: String,
    /// The canonical name (`XAUUSD`).
    pub symbol: String,
    /// The broker's own name for it (`XAUUSD_`).
    pub broker_symbol: String,
    pub digits: u32,
    pub generated_at: String,
    pub valid_until: String,
    pub price: Price,
    pub bias: Bias,
    pub confidence: u32,
    pub score: f64,
    pub stand_aside: bool,
    pub stand_aside_reason: Option<String>,
    pub setup: Option<Setup>,
    pub timeframes: Vec<TimeframeSummary>,
    pub levels: Levels,
    pub factors: Vec<Factor>,
    pub events: Vec<EventDto>,
    pub news: NewsDto,
    pub summary: String,
    pub disclaimer: String,
    /// What was missing or skipped, in plain words.
    pub notes: Vec<String>,
}

impl MarketViewDto {
    /// The view as the model gets it: every number a summary may repeat, without the bulky
    /// parts (links, EMA values, ids).
    pub fn for_tool(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            for key in ["id", "broker_symbol", "digits"] {
                o.remove(key);
            }
        }
        v["timeframes"] = self
            .timeframes
            .iter()
            .map(|t| {
                json!({
                    "timeframe": t.timeframe,
                    "trend": t.trend,
                    "rsi": t.rsi,
                    "atr": t.atr,
                })
            })
            .collect();
        v["news"]["headlines"] = self
            .news
            .headlines
            .iter()
            .map(|h| {
                json!({
                    "title": model_safe_title(&h.title),
                    "source": h.source,
                    "published_at": h.published_at,
                    "tone": h.tone,
                })
            })
            .collect();
        v
    }
}

/// A headline is text from a third party. It goes to the model on one line and at most
/// [`HEADLINE_CHARS`] characters long, so it cannot carry a block of instructions.
pub fn model_safe_title(title: &str) -> String {
    let one_line = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = one_line.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= HEADLINE_CHARS {
        clean
    } else {
        let cut: String = clean.chars().take(HEADLINE_CHARS).collect();
        format!("{}...", cut.trim_end())
    }
}

/// `GET /api/market/news` and the `get_news` tool.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewsReport {
    pub configured: bool,
    pub provider: Option<&'static str>,
    pub summary: Option<NewsSummary>,
    /// Set when a configured provider could not be reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl NewsReport {
    /// The report as the model gets it (no links).
    pub fn for_tool(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(summary) = &self.summary {
            v["summary"]["headlines"] = summary
                .headlines
                .iter()
                .map(|h| {
                    json!({
                        "title": model_safe_title(&h.title),
                        "source": h.source,
                        "published_at": h.published_at,
                        "tone": h.tone,
                    })
                })
                .collect();
        }
        if !self.configured {
            v["note"] = json!("No news provider is configured, so there is no news to report.");
        }
        v
    }
}

// ---------------------------------------------------------------------------
// Instruments
// ---------------------------------------------------------------------------

const FIAT: &[&str] = &[
    "USD", "EUR", "GBP", "JPY", "CHF", "AUD", "CAD", "NZD", "CNH", "CNY", "SEK", "NOK", "DKK",
    "PLN", "CZK", "HUF", "TRY", "ZAR", "MXN", "SGD", "HKD",
];
const METALS: &[&str] = &["XAU", "XAG", "XPT", "XPD"];

fn is_fiat(code: &str) -> bool {
    FIAT.contains(&code)
}

/// MT5's calendar knows the yuan as CNY.
fn calendar_code(code: &str) -> &str {
    if code == "CNH" { "CNY" } else { code }
}

/// The currencies whose news and releases move an instrument.
///
/// FX pairs: base and quote. Metals, energy, crypto and unknown names: USD. Indices: the
/// currency of their home market (US indices USD, GER40/FRA40/EU50 EUR, UK100 GBP, JP225 JPY,
/// HK50 HKD, AUS200 AUD). Names are canonical (`XAUUSD`, not the broker's `XAUUSD_`).
pub fn instrument_currencies(canonical: &str) -> Vec<String> {
    let name = canonical.trim().to_ascii_uppercase();
    let one = |c: &str| vec![c.to_owned()];
    match name.as_str() {
        "US30" | "US100" | "US500" | "US2000" => return one("USD"),
        "GER40" | "FRA40" | "EU50" => return one("EUR"),
        "UK100" => return one("GBP"),
        "JP225" => return one("JPY"),
        "HK50" => return one("HKD"),
        "AUS200" => return one("AUD"),
        _ => {}
    }
    if name.len() == 6 && name.is_ascii() {
        let (base, quote) = (&name[..3], &name[3..]);
        if METALS.contains(&base) {
            let quote = calendar_code(quote);
            return if is_fiat(quote) {
                one(quote)
            } else {
                one("USD")
            };
        }
        if is_fiat(base) && is_fiat(quote) {
            return vec![
                calendar_code(base).to_owned(),
                calendar_code(quote).to_owned(),
            ];
        }
    }
    one("USD")
}

/// The base and quote to point headlines by: FX pairs and metals against a currency.
fn news_pair(canonical: &str) -> Option<(String, String)> {
    let name = canonical.trim().to_ascii_uppercase();
    if name.len() != 6 || !name.is_ascii() {
        return None;
    }
    let (base, quote) = (&name[..3], &name[3..]);
    let base_ok = is_fiat(base) || METALS.contains(&base);
    (base_ok && is_fiat(quote)).then(|| (base.to_owned(), quote.to_owned()))
}

/// What to search the news for: the canonical name, its aliases and keywords from the
/// instrument catalog, and the names of the two currencies of a pair.
pub fn news_query(canonical: &str, catalog: Option<&InstrumentCatalog>) -> NewsQuery {
    let mut terms: Vec<String> = vec![canonical.to_owned()];
    if let Some(instrument) = catalog.and_then(|c| {
        c.instruments()
            .iter()
            .find(|i| i.canonical.eq_ignore_ascii_case(canonical))
    }) {
        terms.extend(instrument.aliases.iter().cloned());
        terms.extend(instrument.keywords.iter().cloned());
    }
    let pair = news_pair(canonical);
    if let Some((base, quote)) = &pair {
        for code in [base, quote] {
            terms.extend(currency_terms(code).iter().map(|t| (*t).to_owned()));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    terms.retain(|t| !t.trim().is_empty() && seen.insert(t.to_lowercase()));
    let query = NewsQuery::new(canonical).with_terms(terms);
    match pair {
        Some((base, quote)) => query.with_pair(base, quote),
        None => query,
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn importance(i: WireImportance) -> Importance {
    match i {
        WireImportance::Low => Importance::Low,
        WireImportance::Moderate => Importance::Moderate,
        WireImportance::High => Importance::High,
    }
}

fn bar(b: &wire::Bar) -> Bar {
    Bar {
        time: b.time,
        open: b.open,
        high: b.high,
        low: b.low,
        close: b.close,
        tick_volume: b.tick_volume,
        spread: b.spread,
    }
}

/// Server-time seconds as ISO 8601 UTC.
pub fn iso(server_secs: i64, offset_secs: i64) -> String {
    DateTime::from_timestamp(server_secs - offset_secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn unit_text(unit: &str) -> Option<String> {
    let unit = unit.trim();
    (!unit.is_empty()).then(|| unit.to_owned())
}

fn event_dto(e: &CalendarEvent, now: i64, offset: i64, with_country: bool) -> EventDto {
    EventDto {
        time: iso(e.time, offset),
        minutes_away: e.time.saturating_sub(now) / 60,
        currency: e.currency.clone(),
        country: with_country.then(|| e.country.clone()),
        title: e.event.clone(),
        importance: importance(e.importance),
        actual: e.actual,
        forecast: e.forecast,
        previous: e.previous,
        unit: unit_text(&e.unit),
    }
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

pub struct MarketParts {
    pub bridge: BridgeClient,
    pub clock: ServerClock,
    pub news: NewsService,
    pub audit: Audit,
    pub config: MarketConfig,
}

enum NewsOutcome {
    /// No provider (or its key was refused).
    Off,
    /// A provider is set up and did not answer in time.
    Failed(String),
    Ok(NewsSummary),
}

/// What one evaluation run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EvaluationReport {
    /// Views looked at.
    pub checked: usize,
    /// Views whose outcome or bias score changed.
    pub updated: usize,
    /// Symbols whose bars could not be read; their views stay as they are.
    pub failed_symbols: usize,
}

/// Builds, stores and scores market views.
pub struct MarketService {
    bridge: BridgeClient,
    clock: ServerClock,
    news: NewsService,
    audit: Audit,
    journal: Journal,
    config: MarketConfig,
    view_config: ViewConfig,
}

impl MarketService {
    pub fn new(parts: MarketParts) -> Arc<Self> {
        let journal = Journal::new(parts.audit.pool().clone());
        let view_config = parts.config.view_config();
        Arc::new(Self {
            bridge: parts.bridge,
            clock: parts.clock,
            news: parts.news,
            audit: parts.audit,
            journal,
            config: parts.config,
            view_config,
        })
    }

    pub fn config(&self) -> &MarketConfig {
        &self.config
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn news_configured(&self) -> bool {
        self.news.is_configured()
    }

    pub fn news_provider(&self) -> Option<&'static str> {
        self.news.provider_name()
    }

    /// Learn the server clock offset from a ping (a failure is not fatal: the last offset
    /// stays) and return the current server time.
    async fn sync_clock(&self) -> i64 {
        if let Ok(info) = self.bridge.ping().await {
            self.clock.observe(info.server_time, Utc::now().timestamp());
        }
        self.clock.now_server().timestamp()
    }

    /// One view: fetch, analyse, journal, audit.
    pub async fn view(
        &self,
        resolver: &SymbolResolver,
        raw: &str,
        actor: &Actor,
    ) -> Result<MarketViewDto, MarketError> {
        let resolved = resolver
            .resolve_full(raw)
            .ok_or_else(|| MarketError::UnusableSymbol(raw.trim().to_owned()))?;
        let (canonical, broker) = (resolved.canonical, resolved.broker);
        let catalog = resolver.book().catalog();

        let now = self.sync_clock().await;
        let offset = self.clock.offset_secs();
        let currencies = instrument_currencies(&canonical);
        let calendar_query = CalendarQuery {
            from: now - CALENDAR_LOOKBACK_SECS,
            to: now + CALENDAR_LOOKAHEAD_SECS,
            currencies: currencies.clone(),
            min_importance: Some(self.config.calendar_min_importance),
        };
        let rates = |timeframe: wire::Timeframe, count: u32| {
            self.bridge.get_rates(RatesQuery {
                symbol: broker.clone(),
                timeframe,
                count: Some(count),
            })
        };

        let (price, info, m5, m15, h1, h4, d1, calendar, news) = tokio::join!(
            self.bridge.get_price(&broker),
            self.bridge.get_symbol(&broker),
            rates(wire::Timeframe::M5, BARS_INTRADAY),
            rates(wire::Timeframe::M15, BARS_INTRADAY),
            rates(wire::Timeframe::H1, BARS_INTRADAY),
            rates(wire::Timeframe::H4, BARS_INTRADAY),
            rates(wire::Timeframe::D1, BARS_DAILY),
            self.bridge.get_calendar(calendar_query),
            self.fetch_news(&canonical, catalog),
        );
        let price = price?;
        let (m5, m15, h1, h4, d1) = (m5?, m15?, h1?, h4?, d1?);
        let mut notes = Vec::new();
        let events = match calendar {
            Ok(c) => c.events,
            Err(e) => match MarketError::from(e) {
                MarketError::EaOutdated => return Err(MarketError::EaOutdated),
                other => {
                    tracing::warn!("the economic calendar could not be read: {other}");
                    notes.push(
                        "The economic calendar could not be read, so upcoming events were \
                         not checked."
                            .to_owned(),
                    );
                    Vec::new()
                }
            },
        };
        let digits = price.digits;
        let point = info
            .map(|i| i.point)
            .ok()
            .filter(|p| p.is_finite() && *p > 0.0)
            .unwrap_or_else(|| 10f64.powi(-i32::try_from(digits).unwrap_or(0)));

        let frames: BTreeMap<Timeframe, Vec<Bar>> = [
            (Timeframe::M5, &m5),
            (Timeframe::M15, &m15),
            (Timeframe::H1, &h1),
            (Timeframe::H4, &h4),
            (Timeframe::D1, &d1),
        ]
        .into_iter()
        .map(|(tf, r)| (tf, r.bars.iter().map(bar).collect()))
        .collect();
        let input = ViewInput {
            symbol: canonical.clone(),
            digits,
            point,
            bid: price.bid,
            ask: price.ask,
            now,
            frames,
            currencies,
            events: events
                .iter()
                .map(|e| EventInput {
                    time: e.time,
                    currency: e.currency.clone(),
                    title: e.event.clone(),
                    importance: importance(e.importance),
                })
                .collect(),
            news: match &news {
                NewsOutcome::Ok(s) => Some(NewsInput {
                    score: s.score,
                    count: s.count,
                }),
                _ => None,
            },
        };
        let view = analyze(&input, &self.view_config);

        let news_dto = match &news {
            NewsOutcome::Off => NewsDto {
                configured: false,
                score: 0.0,
                label: "not configured".to_owned(),
                count: 0,
                headlines: Vec::new(),
            },
            NewsOutcome::Failed(reason) => {
                notes.push(format!("News was unavailable: {reason}."));
                NewsDto {
                    configured: true,
                    score: 0.0,
                    label: "unavailable".to_owned(),
                    count: 0,
                    headlines: Vec::new(),
                }
            }
            NewsOutcome::Ok(s) => NewsDto {
                configured: true,
                score: view.news.score,
                label: view.news.label.clone(),
                count: s.count,
                headlines: s.headlines.iter().take(HEADLINES).cloned().collect(),
            },
        };
        notes.extend(view.notes.iter().cloned());

        let events_dto = view
            .events
            .iter()
            .map(|e| {
                let source = events
                    .iter()
                    .find(|c| c.time == e.time && c.currency == e.currency && c.event == e.title);
                EventDto {
                    time: iso(e.time, offset),
                    minutes_away: e.minutes_away,
                    currency: e.currency.clone(),
                    country: None,
                    title: e.title.clone(),
                    importance: e.importance,
                    actual: source.and_then(|c| c.actual),
                    forecast: source.and_then(|c| c.forecast),
                    previous: source.and_then(|c| c.previous),
                    unit: source.and_then(|c| unit_text(&c.unit)),
                }
            })
            .collect();

        let dto = MarketViewDto {
            id: Uuid::new_v4().to_string(),
            symbol: canonical,
            broker_symbol: broker,
            digits,
            generated_at: iso(now, offset),
            valid_until: iso(view.valid_until, offset),
            price: view.price,
            bias: view.bias,
            confidence: view.confidence,
            score: view.score,
            stand_aside: view.stand_aside,
            stand_aside_reason: view.stand_aside_reason,
            setup: view.setup,
            timeframes: view.timeframes,
            levels: view.levels,
            factors: view.factors,
            events: events_dto,
            news: news_dto,
            summary: view.summary,
            disclaimer: view.disclaimer,
            notes,
        };

        if let Err(e) = self
            .journal
            .insert(&JournalEntry {
                view: &dto,
                created_server: now,
                valid_server: view.valid_until,
            })
            .await
        {
            // The view is still good to read; only "prepare trade" and the track record
            // need the row.
            tracing::error!("the market view was not stored: {e:#}");
        }
        self.audit
            .record_as(
                actor.session,
                actor.persona.as_deref(),
                AuditKind::MarketView,
                json!({
                    "view_id": dto.id,
                    "symbol": dto.symbol,
                    "broker_symbol": dto.broker_symbol,
                    "bias": dto.bias,
                    "confidence": dto.confidence,
                    "stand_aside": dto.stand_aside,
                    "has_setup": dto.setup.is_some(),
                    "source": actor.source,
                }),
            )
            .await;
        Ok(dto)
    }

    /// News for the view. Never fails: a slow or broken provider is reported, not raised.
    async fn fetch_news(
        &self,
        canonical: &str,
        catalog: Option<&InstrumentCatalog>,
    ) -> NewsOutcome {
        if !self.news.is_configured() {
            return NewsOutcome::Off;
        }
        let query = news_query(canonical, catalog);
        match tokio::time::timeout(NEWS_TIMEOUT, self.news.summary(&query)).await {
            Ok(Ok(summary)) => NewsOutcome::Ok(summary),
            Ok(Err(NewsError::NotConfigured)) => {
                tracing::warn!("the news provider refused its API key; news is off");
                NewsOutcome::Off
            }
            Ok(Err(e)) => {
                tracing::warn!("news unavailable: {e}");
                NewsOutcome::Failed(e.to_string())
            }
            Err(_) => {
                tracing::warn!("news did not answer within {NEWS_TIMEOUT:?}");
                NewsOutcome::Failed("the provider did not answer in time".to_owned())
            }
        }
    }

    /// Economic calendar events from an hour ago to `hours_ahead` hours from now, for
    /// `currencies` (empty: all), at or above the configured importance.
    pub async fn calendar(
        &self,
        hours_ahead: u32,
        currencies: &[String],
    ) -> Result<Vec<EventDto>, MarketError> {
        let now = self.sync_clock().await;
        let offset = self.clock.offset_secs();
        let query = CalendarQuery {
            from: now - CALENDAR_LOOKBACK_SECS,
            to: now + i64::from(hours_ahead) * 3600,
            currencies: currencies
                .iter()
                .map(|c| calendar_code(&c.to_ascii_uppercase()).to_owned())
                .collect(),
            min_importance: Some(self.config.calendar_min_importance),
        };
        let calendar = self.bridge.get_calendar(query).await?;
        Ok(calendar
            .events
            .iter()
            .map(|e| event_dto(e, now, offset, true))
            .collect())
    }

    /// Headline news for an instrument.
    pub async fn news(
        &self,
        resolver: &SymbolResolver,
        raw: &str,
    ) -> Result<NewsReport, MarketError> {
        let canonical = resolver
            .canonical(raw)
            .ok_or_else(|| MarketError::UnusableSymbol(raw.trim().to_owned()))?;
        if !self.news.is_configured() {
            return Ok(NewsReport {
                configured: false,
                provider: None,
                summary: None,
                error: None,
            });
        }
        let provider = self.news.provider_name();
        let query = news_query(&canonical, resolver.book().catalog());
        let (summary, error) =
            match tokio::time::timeout(NEWS_TIMEOUT, self.news.summary(&query)).await {
                Ok(Ok(s)) => (Some(s), None),
                Ok(Err(NewsError::NotConfigured)) => {
                    return Ok(NewsReport {
                        configured: false,
                        provider: None,
                        summary: None,
                        error: None,
                    });
                }
                Ok(Err(e)) => (None, Some(e.to_string())),
                Err(_) => (None, Some("the provider did not answer in time".to_owned())),
            };
        Ok(NewsReport {
            configured: true,
            provider,
            summary,
            error,
        })
    }

    /// The order a stored view stands for: a market order with the setup's stop and first
    /// target, or a limit order at the near edge of the entry zone (buy: its low, sell: its
    /// high). The volume is the caller's; there is no default (SPEC rule 9). `None` leaves the
    /// volume at 0 for `Trading::propose` to work out from a `risk_percent` (an order still at
    /// 0 lots is refused by the gate). The result still has to go through `Trading::propose`.
    pub async fn prepare(
        &self,
        resolver: &SymbolResolver,
        view_id: &str,
        volume: Option<f64>,
        kind: PrepareKind,
    ) -> Result<ProposedAction, MarketError> {
        if volume.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            return Err(MarketError::Invalid(
                "volume is required and must be greater than 0".to_owned(),
            ));
        }
        let volume = volume.unwrap_or(0.0);
        let stored = self
            .journal
            .get(view_id)
            .await
            .map_err(MarketError::Storage)?
            .ok_or(MarketError::ViewNotFound)?;
        if self.clock.now_server().timestamp() >= stored.valid_server {
            return Err(MarketError::ViewExpired);
        }
        let setup = stored.view.setup.as_ref().ok_or(MarketError::NoSetup)?;
        // The broker symbol is looked up again: the account may have changed since the view.
        let symbol = resolver
            .resolve(&stored.view.symbol)
            .ok_or_else(|| MarketError::UnusableSymbol(stored.view.symbol.clone()))?;
        let (side, price) = match (setup.side, kind) {
            (Side::Buy, PrepareKind::Limit) => (WireSide::Buy, Some(setup.entry_low)),
            (Side::Sell, PrepareKind::Limit) => (WireSide::Sell, Some(setup.entry_high)),
            (Side::Buy, PrepareKind::Market) => (WireSide::Buy, None),
            (Side::Sell, PrepareKind::Market) => (WireSide::Sell, None),
        };
        Ok(ProposedAction::Order(OrderIntent {
            symbol,
            side,
            volume,
            exec: match kind {
                PrepareKind::Market => ExecType::Market,
                PrepareKind::Limit => ExecType::Limit,
            },
            price,
            sl: Some(setup.stop_loss),
            tp: Some(setup.take_profit),
        }))
    }

    /// Score the views that are still open (and the finished ones whose bias is not judged
    /// yet) against M5 bars. A symbol whose bars cannot be read is skipped until next time.
    pub async fn evaluate_open(&self) -> EvaluationReport {
        let now = self.sync_clock().await;
        self.evaluate_at(now).await
    }

    /// [`MarketService::evaluate_open`] at a given server time.
    pub async fn evaluate_at(&self, now: i64) -> EvaluationReport {
        let mut report = EvaluationReport::default();
        let pending = match self.journal.pending().await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("the market journal could not be read: {e:#}");
                return report;
            }
        };
        let mut by_symbol: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for p in pending {
            by_symbol
                .entry(p.broker_symbol.clone())
                .or_default()
                .push(p);
        }
        for (symbol, group) in by_symbol {
            report.checked += group.len();
            let oldest = group
                .iter()
                .map(|p| p.tracked.created_server)
                .min()
                .unwrap_or(now);
            let count = ((now - oldest) / M5_SECS + 3).clamp(2, EVALUATION_BARS_MAX);
            let rates = match self
                .bridge
                .get_rates(RatesQuery {
                    symbol: symbol.clone(),
                    timeframe: wire::Timeframe::M5,
                    count: u32::try_from(count).ok(),
                })
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!("market views of {symbol} not scored this time: {e}");
                    report.failed_symbols += 1;
                    continue;
                }
            };
            let bars: Vec<Bar> = rates.bars.iter().map(bar).collect();
            for p in group {
                let verdict = score(&p.tracked, &bars, now);
                match self.journal.apply(&p.tracked, &verdict).await {
                    Ok(()) => {
                        if verdict.outcome != p.tracked.outcome
                            || verdict.bias_done != p.tracked.bias_done
                        {
                            report.updated += 1;
                        }
                    }
                    Err(e) => tracing::error!("a market view was not updated: {e:#}"),
                }
            }
        }
        report
    }
}

/// How `POST /api/market/prepare` places the trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareKind {
    Market,
    Limit,
}

impl PrepareKind {
    pub fn parse(text: &str) -> Option<PrepareKind> {
        match text.trim().to_ascii_lowercase().as_str() {
            "market" => Some(PrepareKind::Market),
            "limit" => Some(PrepareKind::Limit),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PrepareKind::Market => "market",
            PrepareKind::Limit => "limit",
        }
    }
}

/// A message the model can relay when a market tool fails.
pub fn tool_error_message(e: &MarketError) -> String {
    match e {
        MarketError::UnusableSymbol(raw) => {
            format!("'{raw}' is not a usable symbol name. Ask the user which instrument they mean.")
        }
        MarketError::EaOutdated => format!("{e} Tell the user; do not guess any values."),
        MarketError::Bridge(b) => match b {
            BridgeError::Timeout(_) | BridgeError::Transport(_) => format!(
                "MetaTrader 5 is not reachable right now ({b}). Tell the user the MT5 bridge \
                 looks offline; do not guess any values."
            ),
            _ => format!(
                "MT5 bridge error: {b}. Tell the user the request failed; do not guess values."
            ),
        },
        other => format!("{other}. Tell the user the request failed."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cur(name: &str) -> Vec<String> {
        instrument_currencies(name)
    }

    #[test]
    fn fx_pairs_use_both_currencies() {
        assert_eq!(cur("EURUSD"), ["EUR", "USD"]);
        assert_eq!(cur("usdjpy"), ["USD", "JPY"]);
        assert_eq!(cur("GBPJPY"), ["GBP", "JPY"]);
        assert_eq!(cur("AUDUSD"), ["AUD", "USD"]);
        assert_eq!(cur("USDCHF"), ["USD", "CHF"]);
        // MT5's calendar calls the yuan CNY.
        assert_eq!(cur("USDCNH"), ["USD", "CNY"]);
    }

    #[test]
    fn metals_energy_and_crypto_follow_the_dollar() {
        for name in [
            "XAUUSD", "XAGUSD", "XPTUSD", "XPDUSD", "WTI", "BRENT", "NATGAS", "BTCUSD", "ETHUSD",
            "LTCUSD",
        ] {
            assert_eq!(cur(name), ["USD"], "{name}");
        }
        assert_eq!(cur("XAUEUR"), ["EUR"]);
    }

    #[test]
    fn indices_follow_their_home_currency() {
        for (name, currency) in [
            ("US30", "USD"),
            ("US100", "USD"),
            ("US500", "USD"),
            ("US2000", "USD"),
            ("GER40", "EUR"),
            ("FRA40", "EUR"),
            ("EU50", "EUR"),
            ("UK100", "GBP"),
            ("JP225", "JPY"),
            ("HK50", "HKD"),
            ("AUS200", "AUD"),
        ] {
            assert_eq!(cur(name), [currency], "{name}");
        }
    }

    #[test]
    fn anything_else_is_the_dollar() {
        for name in ["FOO", "SPX", "ABCDEF", "", "  ", "XYZUSD"] {
            assert_eq!(cur(name), ["USD"], "{name:?}");
        }
    }

    #[test]
    fn the_news_pair_is_set_for_fx_and_metals_only() {
        assert_eq!(
            news_pair("XAUUSD"),
            Some(("XAU".to_owned(), "USD".to_owned()))
        );
        assert_eq!(
            news_pair("XAGUSD"),
            Some(("XAG".to_owned(), "USD".to_owned()))
        );
        assert_eq!(
            news_pair("EURUSD"),
            Some(("EUR".to_owned(), "USD".to_owned()))
        );
        assert_eq!(news_pair("BTCUSD"), None);
        assert_eq!(news_pair("US100"), None);
        assert_eq!(news_pair("WTI"), None);
    }

    #[test]
    fn news_terms_include_names_and_currency_words_without_repeats() {
        let q = news_query("EURUSD", None);
        assert_eq!(q.base.as_deref(), Some("EUR"));
        assert_eq!(q.quote.as_deref(), Some("USD"));
        assert_eq!(q.terms[0], "EURUSD");
        for t in ["ECB", "euro", "Fed", "dollar"] {
            assert!(q.terms.iter().any(|x| x == t), "{t} in {:?}", q.terms);
        }
        let lower: Vec<String> = q.terms.iter().map(|t| t.to_lowercase()).collect();
        let unique: std::collections::BTreeSet<_> = lower.iter().collect();
        assert_eq!(lower.len(), unique.len());

        let gold = news_query("XAUUSD", None);
        assert_eq!(gold.base.as_deref(), Some("XAU"));
        assert!(gold.terms.iter().any(|t| t == "gold"));
    }

    #[test]
    fn news_terms_use_the_catalog_aliases_and_keywords() {
        let catalog = InstrumentCatalog::load(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/instruments.toml"
        )))
        .expect("the shipped catalog loads");
        let q = news_query("US100", Some(&catalog));
        assert!(q.terms.iter().any(|t| t.eq_ignore_ascii_case("nasdaq")));
        assert_eq!(q.base, None);
    }

    #[test]
    fn iso_times_apply_the_server_offset() {
        // 2026-10-30 14:00:00 on a UTC+3 server is 11:00 UTC.
        let server = 1_793_368_800;
        assert_eq!(iso(server, 3 * 3600), "2026-10-30T11:00:00Z");
        assert_eq!(iso(server, 0), "2026-10-30T14:00:00Z");
    }

    #[test]
    fn minutes_away_truncate_toward_zero_like_the_engine() {
        let e = CalendarEvent {
            time: 1_000 + 90,
            currency: "USD".into(),
            country: "US".into(),
            event: "x".into(),
            importance: WireImportance::High,
            actual: None,
            forecast: Some(1.0),
            previous: None,
            unit: " % ".into(),
        };
        let d = event_dto(&e, 1_000, 0, true);
        assert_eq!(d.minutes_away, 1);
        assert_eq!(d.unit.as_deref(), Some("%"));
        assert_eq!(d.country.as_deref(), Some("US"));
        let d = event_dto(&e, 1_000 + 3_000, 0, false);
        assert_eq!(d.minutes_away, -48);
        assert_eq!(d.country, None);
    }

    #[test]
    fn an_old_ea_is_told_apart_from_other_bridge_errors() {
        let old = MarketError::from(BridgeError::Remote {
            message: "unknown command: get_rates".into(),
            retcode: RETCODE_UNKNOWN_COMMAND,
        });
        assert!(matches!(old, MarketError::EaOutdated), "{old}");
        let text = old.to_string();
        assert!(text.contains("0.5.0"), "{text}");
        let other = MarketError::from(BridgeError::Remote {
            message: "unknown symbol".into(),
            retcode: 4301,
        });
        assert!(matches!(other, MarketError::Bridge(_)));
    }

    #[test]
    fn headlines_reach_the_model_as_short_single_lines() {
        assert_eq!(
            model_safe_title("  Gold   climbs \n on weak payrolls "),
            "Gold climbs on weak payrolls"
        );
        assert_eq!(model_safe_title("a\u{0}b\tc\r\nd"), "ab c d");
        let long = "x".repeat(500);
        let cut = model_safe_title(&long);
        assert_eq!(cut.chars().count(), HEADLINE_CHARS + 3);
        assert!(cut.ends_with("..."));
        assert_eq!(model_safe_title("short"), "short");
    }

    #[test]
    fn prepare_kinds_parse() {
        assert_eq!(PrepareKind::parse("Market"), Some(PrepareKind::Market));
        assert_eq!(PrepareKind::parse(" limit "), Some(PrepareKind::Limit));
        assert_eq!(PrepareKind::parse("stop"), None);
    }
}

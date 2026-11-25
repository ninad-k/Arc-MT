//! Alpaca Trading API and Market Data API client for US stocks, **paper only**
//! (spec-alpaca, ADR-0024).
//!
//! Endpoints as used by the ReyCompass adapter (checked against docs.alpaca.markets on 2026-11-09):
//! account (`GET /v2/account`), clock (`GET /v2/clock`), positions (`GET /v2/positions`,
//! `DELETE /v2/positions/{symbol}`), orders (`GET/POST /v2/orders`, `DELETE /v2/orders/{id}`,
//! `GET /v2/orders:by_client_order_id`), assets (`GET /v2/assets/{symbol}`), and on the data host the
//! latest quote (`GET /v2/stocks/{symbol}/quotes/latest`) and bars (`GET /v2/stocks/{symbol}/bars`).
//!
//! Safety:
//! - Alpaca's responses carry no paper/live flag; the base URL *is* the environment. Only the paper host
//!   `paper-api.alpaca.markets` counts as paper, and every write is refused on any other host.
//! - The keys travel in the `APCA-API-KEY-ID` / `APCA-API-SECRET-KEY` headers, so only `https` is
//!   accepted (plain `http` only for a loopback test double). Keys are never logged or put in errors.
//! - A request that may have reached Alpaca before it failed says so (`maybe_sent`), so the caller can
//!   look the order up by its `client_order_id` instead of sending it again.

use std::fmt;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The only host Arc MT treats as paper trading.
pub const PAPER_HOST: &str = "paper-api.alpaca.markets";
pub const DEFAULT_TRADING_URL: &str = "https://paper-api.alpaca.markets";
pub const DEFAULT_DATA_URL: &str = "https://data.alpaca.markets";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AlpacaError {
    /// Missing or invalid settings (URL, keys).
    #[error("Alpaca is not configured: {0}")]
    Config(String),
    /// A write to an endpoint that is not Alpaca paper.
    #[error("the Alpaca endpoint is not paper trading; orders are refused")]
    NotPaper,
    /// HTTP 401/403: the keys were rejected.
    #[error("Alpaca rejected the API keys")]
    Auth,
    #[error("Alpaca rate limit reached")]
    RateLimited,
    #[error("Alpaca: not found")]
    NotFound,
    /// A 4xx with Alpaca's message (an order rejected, a symbol unknown).
    #[error("Alpaca refused the request (HTTP {status}): {message}")]
    Api { status: u16, message: String },
    /// The request failed before or while Alpaca answered. `maybe_sent`: Alpaca may have acted on it.
    #[error("Alpaca could not be reached: {message}")]
    Transport { message: String, maybe_sent: bool },
    #[error("Alpaca's answer could not be read: {0}")]
    Decode(String),
}

impl AlpacaError {
    /// Whether Alpaca may have processed the request although no answer came back.
    pub fn maybe_sent(&self) -> bool {
        matches!(
            self,
            AlpacaError::Transport {
                maybe_sent: true,
                ..
            }
        )
    }
}

/// Settings. `Debug` never shows the keys.
#[derive(Clone)]
pub struct AlpacaConfig {
    pub trading_url: String,
    pub data_url: String,
    pub key_id: String,
    pub secret_key: String,
    /// `iex` (free) or `sip` (subscription).
    pub feed: String,
    pub timeout: Duration,
}

impl fmt::Debug for AlpacaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlpacaConfig")
            .field("trading_url", &self.trading_url)
            .field("data_url", &self.data_url)
            .field("key_id", &"[redacted]")
            .field("secret_key", &"[redacted]")
            .field("feed", &self.feed)
            .finish()
    }
}

/// Accept the URL as Alpaca's dashboard shows it (`.../v2`) and keep the origin part.
pub fn normalize_base_url(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    u.strip_suffix("/v2").unwrap_or(u).to_owned()
}

fn parse_base(raw: &str, what: &str) -> Result<(String, Url), AlpacaError> {
    let base = normalize_base_url(raw);
    let url = Url::parse(&base).map_err(|_| AlpacaError::Config(format!("{what} is not a URL")))?;
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(AlpacaError::Config(format!(
            "{what} must use https (the API keys travel in headers)"
        )));
    }
    Ok((base, url))
}

/// Whether a trading URL is Alpaca's paper host.
pub fn is_paper_url(url: &str) -> bool {
    Url::parse(&normalize_base_url(url))
        .is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some(PAPER_HOST))
}

/// Alpaca sends most numbers as strings ("12.5"); accept both, and null.
fn num<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(n.as_f64()),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .map(Some)
            .map_err(|_| de::Error::custom(format!("not a number: {s}"))),
        Some(other) => Err(de::Error::custom(format!("not a number: {other}"))),
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Account {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub currency: String,
    #[serde(default, deserialize_with = "num")]
    pub cash: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub equity: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub last_equity: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub buying_power: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub initial_margin: Option<f64>,
    #[serde(default)]
    pub trading_blocked: bool,
    #[serde(default)]
    pub account_blocked: bool,
    #[serde(default)]
    pub trade_suspended_by_user: bool,
}

impl Account {
    /// Why the account cannot trade, or `None`.
    pub fn blocked_reason(&self) -> Option<String> {
        let mut r = Vec::new();
        if self.account_blocked {
            r.push("account blocked".to_owned());
        }
        if self.trading_blocked {
            r.push("trading blocked".to_owned());
        }
        if self.trade_suspended_by_user {
            r.push("trading suspended by the user".to_owned());
        }
        if !self.status.is_empty() && !self.status.eq_ignore_ascii_case("ACTIVE") {
            r.push(format!("account status {}", self.status));
        }
        (!r.is_empty()).then(|| r.join(", "))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Clock {
    pub timestamp: String,
    pub is_open: bool,
    pub next_open: String,
    pub next_close: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Position {
    pub asset_id: String,
    pub symbol: String,
    /// Shares, positive; see `side`.
    #[serde(deserialize_with = "num")]
    pub qty: Option<f64>,
    /// `long` or `short`.
    #[serde(default)]
    pub side: String,
    #[serde(default, deserialize_with = "num")]
    pub avg_entry_price: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub current_price: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub unrealized_pl: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Order {
    pub id: String,
    #[serde(default)]
    pub client_order_id: String,
    #[serde(default)]
    pub symbol: String,
    #[serde(default, deserialize_with = "num")]
    pub qty: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub filled_qty: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub filled_avg_price: Option<f64>,
    #[serde(default)]
    pub side: String,
    #[serde(default, rename = "type")]
    pub order_type: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, deserialize_with = "num")]
    pub limit_price: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    pub stop_price: Option<f64>,
    #[serde(default)]
    pub order_class: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub legs: Option<Vec<Order>>,
}

impl Order {
    /// Rejected or otherwise dead on arrival.
    pub fn is_rejected(&self) -> bool {
        matches!(self.status.as_str(), "rejected" | "suspended")
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Asset {
    #[serde(default)]
    pub id: String,
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tradable: bool,
    #[serde(default)]
    pub fractionable: bool,
    #[serde(default)]
    pub shortable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quote {
    pub bid: f64,
    pub ask: f64,
    /// Unix seconds.
    pub time: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// Bar open, Unix seconds.
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// `POST /v2/orders`. Whole shares, `time_in_force` day.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewOrder {
    pub symbol: String,
    /// Whole shares, as a string.
    pub qty: String,
    /// `buy` or `sell`.
    pub side: String,
    /// `market` or `limit`.
    #[serde(rename = "type")]
    pub order_type: String,
    pub time_in_force: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<String>,
    pub client_order_id: String,
    /// `bracket` (both legs) or `oto` (one leg); absent for a simple order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<TakeProfit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<StopLoss>,
    pub extended_hours: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TakeProfit {
    pub limit_price: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StopLoss {
    pub stop_price: String,
}

/// A price as Alpaca wants it: cents for $1 and up, four decimals below.
pub fn price_str(p: f64) -> String {
    if p >= 1.0 {
        format!("{p:.2}")
    } else {
        format!("{p:.4}")
    }
}

impl NewOrder {
    /// A market or limit order for whole shares, with an optional stop loss and take profit as a
    /// bracket (both) or one-triggers-other (one) order.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        symbol: &str,
        shares: u64,
        buy: bool,
        limit: Option<f64>,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
        client_order_id: &str,
    ) -> Self {
        let order_class = match (stop_loss, take_profit) {
            (Some(_), Some(_)) => Some("bracket".to_owned()),
            (Some(_), None) | (None, Some(_)) => Some("oto".to_owned()),
            (None, None) => None,
        };
        Self {
            symbol: symbol.to_owned(),
            qty: shares.to_string(),
            side: if buy { "buy" } else { "sell" }.to_owned(),
            order_type: if limit.is_some() { "limit" } else { "market" }.to_owned(),
            time_in_force: "day".to_owned(),
            limit_price: limit.map(price_str),
            client_order_id: client_order_id.to_owned(),
            order_class,
            take_profit: take_profit.map(|p| TakeProfit {
                limit_price: price_str(p),
            }),
            stop_loss: stop_loss.map(|p| StopLoss {
                stop_price: price_str(p),
            }),
            extended_hours: false,
        }
    }
}

/// Alpaca client. Cheap to clone.
#[derive(Clone)]
pub struct AlpacaClient {
    http: reqwest::Client,
    trading: String,
    data: String,
    paper: bool,
    feed: String,
    secrets: [String; 2],
}

impl fmt::Debug for AlpacaClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlpacaClient")
            .field("trading", &self.trading)
            .field("data", &self.data)
            .field("paper", &self.paper)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct QuoteBody {
    quote: Option<RawQuote>,
}

#[derive(Deserialize)]
struct RawQuote {
    #[serde(default, deserialize_with = "num")]
    bp: Option<f64>,
    #[serde(default, deserialize_with = "num")]
    ap: Option<f64>,
    #[serde(default)]
    t: Option<String>,
}

#[derive(Deserialize)]
struct BarsBody {
    #[serde(default)]
    bars: Option<Vec<RawBar>>,
}

#[derive(Deserialize)]
struct RawBar {
    t: String,
    o: f64,
    h: f64,
    l: f64,
    c: f64,
    #[serde(default)]
    v: f64,
}

fn unix(iso: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|t| t.timestamp())
}

impl AlpacaClient {
    pub fn new(config: &AlpacaConfig) -> Result<Self, AlpacaError> {
        if config.key_id.trim().is_empty() || config.secret_key.trim().is_empty() {
            return Err(AlpacaError::Config(
                "the key id and secret key must be set".into(),
            ));
        }
        let (trading, _) = parse_base(&config.trading_url, "the trading URL")?;
        let (data, _) = parse_base(&config.data_url, "the data URL")?;
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("apca-api-key-id", config.key_id.trim()),
            ("apca-api-secret-key", config.secret_key.trim()),
        ] {
            let mut v = HeaderValue::from_str(value).map_err(|_| {
                AlpacaError::Config("the API keys contain invalid characters".into())
            })?;
            v.set_sensitive(true);
            headers.insert(name, v);
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(config.timeout)
            .timeout(config.timeout)
            // The outbound log records the configured host; a redirect would reach another one.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("jarvis-alpaca/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| {
                AlpacaError::Config(format!("cannot build HTTP client: {}", e.without_url()))
            })?;
        Ok(Self {
            http,
            paper: is_paper_url(&trading),
            trading,
            data,
            feed: config.feed.trim().to_owned(),
            secrets: [
                config.key_id.trim().to_owned(),
                config.secret_key.trim().to_owned(),
            ],
        })
    }

    /// Whether the trading URL is Alpaca's paper host. Writes are refused otherwise.
    pub fn is_paper(&self) -> bool {
        self.paper
    }

    pub fn trading_url(&self) -> &str {
        &self.trading
    }

    pub fn data_url(&self) -> &str {
        &self.data
    }

    /// Lets tests aim writes at a loopback double. It only takes effect when the trading URL is a
    /// loopback address, so it can never mark a real Alpaca host (or any remote host) as paper.
    #[doc(hidden)]
    pub fn assume_paper_for_tests(mut self) -> Self {
        let local = Url::parse(&self.trading).is_ok_and(|u| {
            u.host_str()
                .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "::1")
        });
        if local {
            self.paper = true;
        }
        self
    }

    fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for s in &self.secrets {
            if !s.is_empty() {
                out = out.replace(s.as_str(), "[redacted]");
            }
        }
        out.chars().take(300).collect()
    }

    async fn send(
        &self,
        method: Method,
        url: String,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<(StatusCode, String), AlpacaError> {
        let write = method != Method::GET;
        let mut url = Url::parse(&url)
            .map_err(|_| AlpacaError::Config("invalid Alpaca request URL".into()))?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        let mut req = self.http.request(method, url);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| {
            // A connect failure never reached Alpaca; anything later might have.
            let maybe_sent = write && !e.is_connect();
            AlpacaError::Transport {
                message: self.redact(&e.without_url().to_string()),
                maybe_sent,
            }
        })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| AlpacaError::Transport {
            message: self.redact(&e.without_url().to_string()),
            maybe_sent: write,
        })?;
        match status.as_u16() {
            401 | 403 => Err(AlpacaError::Auth),
            404 => Err(AlpacaError::NotFound),
            429 => Err(AlpacaError::RateLimited),
            500..=599 => Err(AlpacaError::Transport {
                message: format!("Alpaca server error HTTP {}", status.as_u16()),
                maybe_sent: write,
            }),
            400..=499 => Err(AlpacaError::Api {
                status: status.as_u16(),
                message: self.redact(&api_message(&text)),
            }),
            _ => Ok((status, text)),
        }
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        base: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, AlpacaError> {
        let (_, text) = self
            .send(Method::GET, format!("{base}{path}"), query, None)
            .await?;
        serde_json::from_str(&text).map_err(|e| AlpacaError::Decode(self.redact(&e.to_string())))
    }

    pub async fn account(&self) -> Result<Account, AlpacaError> {
        self.get(&self.trading, "/v2/account", &[]).await
    }

    pub async fn clock(&self) -> Result<Clock, AlpacaError> {
        self.get(&self.trading, "/v2/clock", &[]).await
    }

    pub async fn positions(&self) -> Result<Vec<Position>, AlpacaError> {
        self.get(&self.trading, "/v2/positions", &[]).await
    }

    pub async fn open_orders(&self) -> Result<Vec<Order>, AlpacaError> {
        self.get(
            &self.trading,
            "/v2/orders",
            &[
                ("status", "open".into()),
                ("limit", "500".into()),
                ("nested", "true".into()),
            ],
        )
        .await
    }

    /// The order with this `client_order_id`, or `None`.
    pub async fn order_by_client_id(
        &self,
        client_order_id: &str,
    ) -> Result<Option<Order>, AlpacaError> {
        match self
            .get(
                &self.trading,
                "/v2/orders:by_client_order_id",
                &[("client_order_id", client_order_id.to_owned())],
            )
            .await
        {
            Ok(o) => Ok(Some(o)),
            Err(AlpacaError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn asset(&self, symbol: &str) -> Result<Asset, AlpacaError> {
        self.get(
            &self.trading,
            &format!("/v2/assets/{}", path_part(symbol)),
            &[],
        )
        .await
    }

    fn require_paper(&self) -> Result<(), AlpacaError> {
        if self.paper {
            Ok(())
        } else {
            Err(AlpacaError::NotPaper)
        }
    }

    /// Send an order. Refused on any host but paper.
    pub async fn submit_order(&self, order: &NewOrder) -> Result<Order, AlpacaError> {
        self.require_paper()?;
        let body = serde_json::to_value(order).map_err(|e| AlpacaError::Decode(e.to_string()))?;
        let (_, text) = self
            .send(
                Method::POST,
                format!("{}/v2/orders", self.trading),
                &[],
                Some(&body),
            )
            .await?;
        serde_json::from_str(&text).map_err(|e| AlpacaError::Decode(self.redact(&e.to_string())))
    }

    /// Cancel one order. Refused on any host but paper.
    pub async fn cancel_order(&self, order_id: &str) -> Result<(), AlpacaError> {
        self.require_paper()?;
        self.send(
            Method::DELETE,
            format!("{}/v2/orders/{}", self.trading, path_part(order_id)),
            &[],
            None,
        )
        .await
        .map(|_| ())
    }

    /// Close a position at market, fully or `shares` of it. Refused on any host but paper.
    pub async fn close_position(
        &self,
        symbol: &str,
        shares: Option<u64>,
    ) -> Result<Order, AlpacaError> {
        self.require_paper()?;
        let query: Vec<(&str, String)> =
            shares.map(|q| ("qty", q.to_string())).into_iter().collect();
        let (_, text) = self
            .send(
                Method::DELETE,
                format!("{}/v2/positions/{}", self.trading, path_part(symbol)),
                &query,
                None,
            )
            .await?;
        serde_json::from_str(&text).map_err(|e| AlpacaError::Decode(self.redact(&e.to_string())))
    }

    /// The latest two-sided quote. A zero bid or ask (no active quote) is an error, never a price.
    pub async fn latest_quote(&self, symbol: &str) -> Result<Quote, AlpacaError> {
        let body: QuoteBody = self
            .get(
                &self.data,
                &format!("/v2/stocks/{}/quotes/latest", path_part(symbol)),
                &[("feed", self.feed.clone())],
            )
            .await?;
        let q = body
            .quote
            .ok_or_else(|| AlpacaError::Decode(format!("no quote for {symbol}")))?;
        match (q.bp, q.ap) {
            (Some(bid), Some(ask)) if bid > 0.0 && ask > 0.0 => Ok(Quote {
                bid,
                ask,
                time: q.t.as_deref().and_then(unix).unwrap_or(0),
            }),
            _ => Err(AlpacaError::Decode(format!(
                "no two-sided quote for {symbol}"
            ))),
        }
    }

    /// Recent bars, oldest first. `timeframe` is Alpaca's (`1Min`, `15Min`, `1Hour`, `1Day`, ...);
    /// `start` is RFC 3339. Malformed bars are dropped, never patched.
    pub async fn bars(
        &self,
        symbol: &str,
        timeframe: &str,
        start: &str,
        limit: u32,
    ) -> Result<Vec<Bar>, AlpacaError> {
        let body: BarsBody = self
            .get(
                &self.data,
                &format!("/v2/stocks/{}/bars", path_part(symbol)),
                &[
                    ("timeframe", timeframe.to_owned()),
                    ("start", start.to_owned()),
                    ("limit", limit.to_string()),
                    ("sort", "desc".to_owned()),
                    ("feed", self.feed.clone()),
                    ("adjustment", "split".to_owned()),
                ],
            )
            .await?;
        let mut out: Vec<Bar> = body
            .bars
            .unwrap_or_default()
            .into_iter()
            .filter_map(|b| {
                Some(Bar {
                    time: unix(&b.t)?,
                    open: b.o,
                    high: b.h,
                    low: b.l,
                    close: b.c,
                    volume: b.v,
                })
            })
            .filter(|b| {
                [b.open, b.high, b.low, b.close]
                    .iter()
                    .all(|v| v.is_finite() && *v > 0.0)
            })
            .collect();
        out.sort_by_key(|b| b.time);
        Ok(out)
    }
}

/// A symbol or id as one URL path segment (letters, digits, `.`, `-`, `/` for class shares encoded).
fn path_part(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect()
}

fn api_message(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| text.chars().take(200).collect())
}

/// A stable 64-bit ticket for an Alpaca id (FNV-1a), so Alpaca positions and orders fit Arc MT's ticket
/// numbers. Never 0.
pub fn ticket_for(id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    // Keep it within JSON's exact-integer range (2^53) for the browser.
    (h & ((1u64 << 53) - 1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_paper_host_is_paper() {
        assert!(is_paper_url("https://paper-api.alpaca.markets"));
        assert!(is_paper_url("https://paper-api.alpaca.markets/v2/"));
        assert!(!is_paper_url("https://api.alpaca.markets"));
        assert!(!is_paper_url("http://paper-api.alpaca.markets"));
        assert!(!is_paper_url("https://paper-api.alpaca.markets.evil.com"));
    }

    #[test]
    fn orders_pick_their_class_from_the_legs() {
        let b = NewOrder::new("NVDA", 5, true, None, Some(118.0), Some(130.5), "c1");
        assert_eq!(b.order_class.as_deref(), Some("bracket"));
        assert_eq!(b.stop_loss.as_ref().unwrap().stop_price, "118.00");
        assert_eq!(b.take_profit.as_ref().unwrap().limit_price, "130.50");
        assert_eq!(b.order_type, "market");
        let o = NewOrder::new("NVDA", 5, false, Some(0.5), Some(0.6), None, "c2");
        assert_eq!(o.order_class.as_deref(), Some("oto"));
        assert_eq!(o.limit_price.as_deref(), Some("0.5000"));
        assert_eq!(o.side, "sell");
        assert!(
            NewOrder::new("X", 1, true, None, None, None, "c3")
                .order_class
                .is_none()
        );
    }

    #[test]
    fn tickets_are_stable_and_safe_for_json() {
        let a = ticket_for("904837e3-3b76-47ec-b432-046db621571b");
        assert_eq!(a, ticket_for("904837e3-3b76-47ec-b432-046db621571b"));
        assert_ne!(a, ticket_for("904837e3-3b76-47ec-b432-046db621571c"));
        assert!(a < (1u64 << 53) && a > 0);
    }

    #[test]
    fn blocked_reasons_are_named() {
        let a: Account = serde_json::from_str(
            r#"{"status":"ACTIVE","currency":"USD","cash":"100","equity":"100.5","last_equity":"99","trading_blocked":true}"#,
        )
        .unwrap();
        assert_eq!(a.equity, Some(100.5));
        assert_eq!(a.blocked_reason().as_deref(), Some("trading blocked"));
    }
}

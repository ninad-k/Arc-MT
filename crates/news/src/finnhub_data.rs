//! Finnhub company data for Nimbus Desk (spec-nimbus-desk): insider transactions, basic
//! financials, SEC filings and (premium) congressional trades.
//!
//! Endpoints and fields as documented at <https://finnhub.io/docs/api> (checked 2026-11-24):
//! - `GET /api/v1/stock/insider-transactions?symbol&from&to` -> `{data: [{name, share, change,
//!   filingDate, transactionDate, transactionCode, transactionPrice}], symbol}` (free tier);
//! - `GET /api/v1/stock/metric?symbol&metric=all` -> `{metric: {...}, series, metricType, symbol}`;
//! - `GET /api/v1/stock/filings?symbol&from&to` -> `[{accessNumber, symbol, cik, form, filedDate,
//!   acceptedDate, reportUrl, filingUrl}]`;
//! - `GET /api/v1/stock/congressional-trading?symbol&from&to` (premium) -> `{data: [{amountFrom,
//!   amountTo, assetName, filingDate, name, ownerType, position, symbol, transactionDate,
//!   transactionType}], symbol}`.
//!
//! The key goes in the `X-Finnhub-Token` header, never in the URL. HTTP 401 is
//! [`NewsError::NotConfigured`]; HTTP 403 (a premium endpoint on a free key) is reported the same
//! way, so the caller shows "—".

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

use reqwest::Url;
use reqwest::header::HeaderValue;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::NewsError;
use crate::http::{self, DEFAULT_TIMEOUT};

const DEFAULT_BASE_URL: &str = "https://finnhub.io";
const KEY_HEADER: &str = "x-finnhub-token";

/// One insider transaction, as reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsiderTransaction {
    #[serde(default)]
    pub name: String,
    /// Shares held after the transaction.
    #[serde(default)]
    pub share: Option<f64>,
    /// Shares bought (positive) or sold (negative).
    #[serde(default)]
    pub change: Option<f64>,
    #[serde(default)]
    pub filing_date: Option<String>,
    #[serde(default)]
    pub transaction_date: Option<String>,
    #[serde(default)]
    pub transaction_code: Option<String>,
    #[serde(default)]
    pub transaction_price: Option<f64>,
}

/// One SEC filing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecFiling {
    #[serde(default)]
    pub form: String,
    /// `%Y-%m-%d %H:%M:%S`.
    #[serde(default)]
    pub filed_date: Option<String>,
    #[serde(default)]
    pub report_url: Option<String>,
    #[serde(default)]
    pub filing_url: Option<String>,
}

/// One trade disclosed by a member of Congress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CongressionalTrade {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub position: Option<String>,
    /// `"Sale"` or `"Purchase"`.
    #[serde(default)]
    pub transaction_type: Option<String>,
    #[serde(default)]
    pub transaction_date: Option<String>,
    #[serde(default)]
    pub amount_from: Option<f64>,
    #[serde(default)]
    pub amount_to: Option<f64>,
}

/// The `metric` map of basic financials, with the fields the Desk reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BasicFinancials {
    pub metric: BTreeMap<String, Value>,
}

impl BasicFinancials {
    /// The first of `keys` that is a number.
    pub fn number(&self, keys: &[&str]) -> Option<f64> {
        keys.iter()
            .find_map(|k| self.metric.get(*k).and_then(Value::as_f64))
            .filter(|v| v.is_finite())
    }
    pub fn pe_ttm(&self) -> Option<f64> {
        self.number(&["peTTM", "peBasicExclExtraTTM", "peExclExtraTTM"])
    }
    pub fn eps_ttm(&self) -> Option<f64> {
        self.number(&[
            "epsTTM",
            "epsBasicExclExtraItemsTTM",
            "epsExclExtraItemsTTM",
        ])
    }
    pub fn net_margin_ttm(&self) -> Option<f64> {
        self.number(&["netProfitMarginTTM", "netMarginTTM"])
    }
    pub fn revenue_growth_ttm_yoy(&self) -> Option<f64> {
        self.number(&["revenueGrowthTTMYoy"])
    }
    pub fn market_cap_millions(&self) -> Option<f64> {
        self.number(&["marketCapitalization"])
    }
    pub fn beta(&self) -> Option<f64> {
        self.number(&["beta"])
    }
    pub fn week52_high(&self) -> Option<f64> {
        self.number(&["52WeekHigh"])
    }
    pub fn week52_low(&self) -> Option<f64> {
        self.number(&["52WeekLow"])
    }
}

#[derive(Deserialize)]
struct DataList<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Deserialize)]
struct MetricBody {
    #[serde(default)]
    metric: Option<BTreeMap<String, Value>>,
}

/// Finnhub company-data client.
pub struct FinnhubData {
    client: reqwest::Client,
    base: String,
    key: String,
    key_header: HeaderValue,
    timeout: Duration,
}

impl fmt::Debug for FinnhubData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FinnhubData")
            .field("base", &self.base)
            .field("key", &"[redacted]")
            .finish()
    }
}

impl FinnhubData {
    /// An empty key is [`NewsError::NotConfigured`].
    pub fn new(api_key: impl AsRef<str>) -> Result<Self, NewsError> {
        let key = api_key.as_ref().trim().to_string();
        if key.is_empty() {
            return Err(NewsError::NotConfigured);
        }
        let mut key_header = HeaderValue::from_str(&key).map_err(|_| NewsError::NotConfigured)?;
        key_header.set_sensitive(true);
        Ok(Self {
            client: http::build_client()?,
            base: DEFAULT_BASE_URL.to_string(),
            key,
            key_header,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Point at another server (tests, a proxy). `http` is accepted for localhost only.
    pub fn with_base_url(mut self, url: &str) -> Result<Self, NewsError> {
        self.base = http::parse_base(url)?;
        Ok(self)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The base URL requests go to (for the outbound-connection log).
    pub fn base_url(&self) -> &str {
        &self.base
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, NewsError> {
        let mut url = Url::parse(&format!("{}/api/v1/{path}", self.base))
            .map_err(|_| NewsError::Transport("invalid Finnhub base URL".into()))?;
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
        }
        let start = Instant::now();
        let req = self
            .client
            .get(url)
            .header(KEY_HEADER, self.key_header.clone());
        let (status, body) = http::get_text(req, self.timeout, &self.key).await?;
        tracing::debug!(
            provider = "finnhub",
            path,
            status,
            elapsed_ms = start.elapsed().as_millis() as u64,
            "finnhub data response"
        );
        if status == 403 {
            return Err(NewsError::NotConfigured);
        }
        if !(200..300).contains(&status) {
            return Err(http::status_error(status, &body, &self.key));
        }
        serde_json::from_str(&body).map_err(|e| http::decode_error(&e, &self.key))
    }

    /// Insider transactions between two `YYYY-MM-DD` dates (at most 100, newest first).
    pub async fn insider_transactions(
        &self,
        symbol: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<InsiderTransaction>, NewsError> {
        let list: DataList<InsiderTransaction> = self
            .get(
                "stock/insider-transactions",
                &[("symbol", symbol), ("from", from), ("to", to)],
            )
            .await?;
        Ok(list.data)
    }

    pub async fn basic_financials(&self, symbol: &str) -> Result<BasicFinancials, NewsError> {
        let body: MetricBody = self
            .get("stock/metric", &[("symbol", symbol), ("metric", "all")])
            .await?;
        Ok(BasicFinancials {
            metric: body.metric.unwrap_or_default(),
        })
    }

    /// Filings between two `YYYY-MM-DD` dates.
    pub async fn filings(
        &self,
        symbol: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<SecFiling>, NewsError> {
        self.get(
            "stock/filings",
            &[("symbol", symbol), ("from", from), ("to", to)],
        )
        .await
    }

    /// Congressional trades (premium). A free key gets [`NewsError::NotConfigured`].
    pub async fn congressional_trades(
        &self,
        symbol: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<CongressionalTrade>, NewsError> {
        let list: DataList<CongressionalTrade> = self
            .get(
                "stock/congressional-trading",
                &[("symbol", symbol), ("from", from), ("to", to)],
            )
            .await?;
        Ok(list.data)
    }
}

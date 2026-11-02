//! Alpha Vantage `NEWS_SENTIMENT` (`GET {base}/query?function=NEWS_SENTIMENT&...`).
//!
//! Things to know about this API:
//!
//! - The key can only go in the query string, so the request URL is never logged here and
//!   every error is built from the status, a redacted excerpt or a fixed message.
//! - It answers HTTP 200 for a used-up quota or a bad key, with an `Information` or `Note`
//!   field instead of a `feed`. Those map to [`NewsError::RateLimited`] and
//!   [`NewsError::NotConfigured`]; the text itself is not passed on.
//! - Several `tickers` (or several `topics`) are ANDed: the articles must mention all of them.
//!   A pair therefore takes one request per currency (`FOREX:EUR`, then `FOREX:USD`) and the
//!   results are merged. Metals, indices and energy use a single topic and the service's
//!   term filter does the rest. The free tier is small (about 25 calls a day), so the service
//!   cache matters.
//! - Scores run roughly -1..1 with the provider's own bands (above 0.35 is "bullish"), which
//!   line up with the labels used by [`crate::NewsService`], so they are clamped, not scaled.
//! - `time_published` is `YYYYMMDDTHHMMSS` in UTC, and `ticker_sentiment_score` is a string.

use std::fmt;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::Url;
use serde::Deserialize;

use crate::finnhub::dedupe;
use crate::http::{self, DEFAULT_TIMEOUT};
use crate::instrument::{Kind, classify, crypto_base, parts};
use crate::{Article, Headline, NewsError, NewsProvider, NewsQuery, lexicon};

const DEFAULT_BASE_URL: &str = "https://www.alphavantage.co";
const FEED_LIMIT: &str = "50";

/// Alpha Vantage provider.
pub struct AlphaVantage {
    client: reqwest::Client,
    base: String,
    key: String,
    timeout: Duration,
}

impl fmt::Debug for AlphaVantage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlphaVantage")
            .field("base", &self.base)
            .field("key", &"[redacted]")
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// A number the API sometimes sends as a string.
#[derive(Deserialize)]
#[serde(untagged)]
enum Flex {
    Num(f64),
    Str(String),
}

impl Flex {
    fn value(&self) -> Option<f64> {
        let v = match self {
            Flex::Num(n) => *n,
            Flex::Str(s) => s.trim().parse().ok()?,
        };
        v.is_finite().then_some(v.clamp(-1.0, 1.0))
    }
}

#[derive(Deserialize)]
struct TickerSentiment {
    #[serde(default)]
    ticker: String,
    #[serde(default)]
    ticker_sentiment_score: Option<Flex>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    time_published: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    overall_sentiment_score: Option<Flex>,
    #[serde(default)]
    ticker_sentiment: Vec<TickerSentiment>,
}

#[derive(Deserialize)]
struct Response {
    feed: Option<Vec<Item>>,
    #[serde(rename = "Information")]
    information: Option<String>,
    #[serde(rename = "Note")]
    note: Option<String>,
    #[serde(rename = "Error Message")]
    error_message: Option<String>,
}

/// What one request asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Filter {
    Ticker(String),
    Topic(&'static str),
}

/// The requests for an instrument and the tickers whose scores we read.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    filters: Vec<Filter>,
    /// Ticker for the base side (or the only ticker).
    primary: Option<String>,
    /// Ticker for the quote side of a pair.
    secondary: Option<String>,
}

fn plan(q: &NewsQuery) -> Plan {
    let (base, quote) = parts(q).unzip();
    match classify(q) {
        Kind::Crypto => {
            let coin = crypto_base(q).or(base).unwrap_or_default();
            let t = format!("CRYPTO:{coin}");
            Plan {
                filters: vec![Filter::Ticker(t.clone())],
                primary: Some(t),
                secondary: None,
            }
        }
        Kind::FxPair => {
            let b = format!("FOREX:{}", base.unwrap_or_default());
            let s = format!("FOREX:{}", quote.unwrap_or_default());
            Plan {
                filters: vec![Filter::Ticker(b.clone()), Filter::Ticker(s.clone())],
                primary: Some(b),
                secondary: Some(s),
            }
        }
        Kind::Metal => topic("economy_monetary"),
        Kind::Energy => topic("energy_transportation"),
        Kind::Other => topic("financial_markets"),
    }
}

fn topic(t: &'static str) -> Plan {
    Plan {
        filters: vec![Filter::Topic(t)],
        primary: None,
        secondary: None,
    }
}

/// `YYYYMMDDTHHMMSS` (or without seconds), UTC.
fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    ["%Y%m%dT%H%M%S", "%Y%m%dT%H%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .map(|n| n.and_utc())
}

/// Turn an `Information`/`Note`/`Error Message` body into an error, or `None` when it only
/// says there is nothing to show.
fn body_notice(r: &Response) -> Option<Result<(), NewsError>> {
    let text = r
        .information
        .as_deref()
        .or(r.note.as_deref())
        .map(str::to_lowercase);
    if let Some(t) = text {
        if t.contains("no articles") {
            return Some(Ok(()));
        }
        let quota = [
            "rate limit",
            "requests per",
            "per day",
            "per minute",
            "call frequency",
        ]
        .iter()
        .any(|k| t.contains(k));
        let key = t.contains("apikey") || t.contains("api key");
        return Some(Err(if quota {
            NewsError::RateLimited
        } else if key {
            NewsError::NotConfigured
        } else {
            // Unknown notices are nearly always throttling.
            NewsError::RateLimited
        }));
    }
    r.error_message.as_ref().map(|_| {
        // Fixed text: the provider's wording can echo the request.
        Err(NewsError::Decode(
            "the provider rejected the request".to_string(),
        ))
    })
}

impl AlphaVantage {
    /// An empty key is [`NewsError::NotConfigured`].
    pub fn new(api_key: impl AsRef<str>) -> Result<Self, NewsError> {
        let key = api_key.as_ref().trim().to_string();
        if key.is_empty() {
            return Err(NewsError::NotConfigured);
        }
        Ok(Self {
            client: http::build_client()?,
            base: DEFAULT_BASE_URL.to_string(),
            key,
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

    async fn request(
        &self,
        filter: &Filter,
        q: &NewsQuery,
        plan: &Plan,
    ) -> Result<Vec<Article>, NewsError> {
        let mut url = Url::parse(&format!("{}/query", self.base))
            .map_err(|_| NewsError::Transport("invalid news provider base URL".into()))?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("function", "NEWS_SENTIMENT");
            match filter {
                Filter::Ticker(t) => pairs.append_pair("tickers", t),
                Filter::Topic(t) => pairs.append_pair("topics", t),
            };
            pairs.append_pair("time_from", &q.since.format("%Y%m%dT%H%M").to_string());
            pairs.append_pair("limit", FEED_LIMIT);
            pairs.append_pair("apikey", &self.key);
        }
        // The URL holds the key: it is handed to reqwest and never logged or kept.
        let start = Instant::now();
        let (status, body) = http::get_text(self.client.get(url), self.timeout, &self.key).await?;
        tracing::debug!(
            provider = "alphavantage",
            status,
            elapsed_ms = start.elapsed().as_millis() as u64,
            "news http response"
        );
        if !(200..300).contains(&status) {
            return Err(http::status_error(status, &body, &self.key));
        }
        let resp: Response =
            serde_json::from_str(&body).map_err(|e| http::decode_error(&e, &self.key))?;
        if resp.feed.is_none() || resp.information.is_some() || resp.note.is_some() {
            match body_notice(&resp) {
                Some(Ok(())) => return Ok(Vec::new()),
                Some(Err(e)) => return Err(e),
                None => {}
            }
        }
        let Some(feed) = resp.feed else {
            return Err(NewsError::Decode("response has no feed".into()));
        };
        Ok(feed.into_iter().filter_map(|i| article(i, plan)).collect())
    }
}

fn score_for(item: &Item, ticker: Option<&str>) -> Option<f64> {
    let ticker = ticker?;
    item.ticker_sentiment
        .iter()
        .find(|t| t.ticker.eq_ignore_ascii_case(ticker))
        .and_then(|t| t.ticker_sentiment_score.as_ref())
        .and_then(Flex::value)
}

fn article(item: Item, plan: &Plan) -> Option<Article> {
    let title = item.title.trim().to_string();
    let published_at = parse_time(&item.time_published)?;
    if title.is_empty() {
        return None;
    }
    let first = score_for(&item, plan.primary.as_deref());
    let second = score_for(&item, plan.secondary.as_deref());
    // Ticker scores first. For a pair, base minus quote, halved to stay in -1..1.
    let ticker_tone = match (first, second, plan.secondary.is_some()) {
        (Some(b), Some(q), true) => Some((b - q) / 2.0),
        (Some(b), None, _) => Some(b),
        (None, Some(q), true) => Some(-q),
        _ => None,
    };
    let overall = item.overall_sentiment_score.as_ref().and_then(Flex::value);
    // No provider score at all: fall back to the word list.
    let (tone, scoped) = match (ticker_tone, overall) {
        (Some(t), _) => (t, true),
        (None, Some(o)) => (o, false),
        (None, None) => (lexicon::headline_tone(&title, &item.summary), false),
    };
    Some(Article {
        headline: Headline {
            title,
            source: item.source.trim().to_string(),
            url: item.url.trim().to_string(),
            published_at,
            tone,
        },
        summary: item.summary,
        scoped,
        directed: scoped,
    })
}

#[async_trait]
impl NewsProvider for AlphaVantage {
    fn name(&self) -> &'static str {
        "alphavantage"
    }

    async fn fetch(&self, q: &NewsQuery) -> Result<Vec<Headline>, NewsError> {
        Ok(self
            .fetch_articles(q)
            .await?
            .into_iter()
            .map(|a| a.headline)
            .collect())
    }

    async fn fetch_articles(&self, q: &NewsQuery) -> Result<Vec<Article>, NewsError> {
        let plan = plan(q);
        let mut all = Vec::new();
        for (i, filter) in plan.filters.iter().enumerate() {
            match self.request(filter, q, &plan).await {
                Ok(items) => all.extend(items),
                // The first request failing is the error; a later one only loses its half.
                Err(e) if i == 0 => return Err(e),
                Err(e) => {
                    tracing::warn!(provider = "alphavantage", error = %e, "news fetch failed");
                    break;
                }
            }
        }
        Ok(dedupe(all))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans() {
        let p = plan(&NewsQuery::new("EURUSD"));
        assert_eq!(
            p.filters,
            [
                Filter::Ticker("FOREX:EUR".into()),
                Filter::Ticker("FOREX:USD".into())
            ]
        );
        let p = plan(&NewsQuery::new("BTCUSD"));
        assert_eq!(p.filters, [Filter::Ticker("CRYPTO:BTC".into())]);
        assert_eq!(
            plan(&NewsQuery::new("XAUUSD")).filters,
            [Filter::Topic("economy_monetary")]
        );
        assert_eq!(
            plan(&NewsQuery::new("US500")).filters,
            [Filter::Topic("financial_markets")]
        );
        assert_eq!(
            plan(&NewsQuery::new("USOIL")).filters,
            [Filter::Topic("energy_transportation")]
        );
    }

    #[test]
    fn time_format() {
        let t = parse_time("20261030T134500").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-10-30T13:45:00+00:00");
        assert!(parse_time("20261030T1345").is_some());
        assert!(parse_time("2026-10-30").is_none());
    }

    #[test]
    fn flex_scores() {
        assert_eq!(Flex::Str("0.25".into()).value(), Some(0.25));
        assert_eq!(Flex::Num(3.0).value(), Some(1.0));
        assert_eq!(Flex::Str("n/a".into()).value(), None);
    }
}

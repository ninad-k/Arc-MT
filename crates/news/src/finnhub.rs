//! Finnhub market news (`GET {base}/api/v1/news?category=...`).
//!
//! The key goes in the `X-Finnhub-Token` header, never in the URL. The endpoint returns the
//! latest items of a whole category (forex, general, crypto, merger) and cannot filter by
//! instrument, so relevance is decided afterwards by the service. Free keys allow 60 calls a
//! minute; an exhausted quota answers HTTP 429 and a bad key HTTP 401.

use std::fmt;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::DateTime;
use reqwest::Url;
use reqwest::header::HeaderValue;
use serde::Deserialize;

use crate::http::{self, DEFAULT_TIMEOUT};
use crate::instrument::{Kind, classify};
use crate::{Article, Headline, NewsError, NewsProvider, NewsQuery, lexicon};

const DEFAULT_BASE_URL: &str = "https://finnhub.io";
const KEY_HEADER: &str = "x-finnhub-token";

/// Finnhub provider.
pub struct Finnhub {
    client: reqwest::Client,
    base: String,
    key: String,
    key_header: HeaderValue,
    timeout: Duration,
}

impl fmt::Debug for Finnhub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Finnhub")
            .field("base", &self.base)
            .field("key", &"[redacted]")
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    headline: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    datetime: Option<i64>,
}

impl Finnhub {
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

    /// Finnhub categories for an instrument: crypto, forex, general. Metals read both forex
    /// and general news (gold stories often sit in the general feed).
    fn categories(q: &NewsQuery) -> &'static [&'static str] {
        match classify(q) {
            Kind::Crypto => &["crypto"],
            Kind::FxPair => &["forex"],
            Kind::Metal => &["forex", "general"],
            Kind::Energy | Kind::Other => &["general"],
        }
    }

    async fn fetch_category(&self, category: &str) -> Result<Vec<Article>, NewsError> {
        let mut url = Url::parse(&format!("{}/api/v1/news", self.base))
            .map_err(|_| NewsError::Transport("invalid news provider base URL".into()))?;
        url.query_pairs_mut().append_pair("category", category);
        let start = Instant::now();
        let req = self
            .client
            .get(url)
            .header(KEY_HEADER, self.key_header.clone());
        let (status, body) = http::get_text(req, self.timeout, &self.key).await?;
        tracing::debug!(
            provider = "finnhub",
            category,
            status,
            elapsed_ms = start.elapsed().as_millis() as u64,
            "news http response"
        );
        if !(200..300).contains(&status) {
            return Err(http::status_error(status, &body, &self.key));
        }
        let items: Vec<Item> =
            serde_json::from_str(&body).map_err(|e| http::decode_error(&e, &self.key))?;
        Ok(items.into_iter().filter_map(article).collect())
    }
}

fn article(item: Item) -> Option<Article> {
    let title = item.headline.trim().to_string();
    let published_at = DateTime::from_timestamp(item.datetime.filter(|t| *t > 0)?, 0)?;
    if title.is_empty() {
        return None;
    }
    let tone = lexicon::headline_tone(&title, &item.summary);
    Some(Article {
        headline: Headline {
            title,
            source: item.source.trim().to_string(),
            url: item.url.trim().to_string(),
            published_at,
            tone,
        },
        summary: item.summary,
        scoped: false,
        directed: false,
    })
}

/// Keep the first of each URL (or title when there is no URL).
pub(crate) fn dedupe(articles: Vec<Article>) -> Vec<Article> {
    let mut seen = std::collections::HashSet::new();
    articles
        .into_iter()
        .filter(|a| {
            let key = if a.headline.url.is_empty() {
                a.headline.title.to_lowercase()
            } else {
                a.headline.url.clone()
            };
            seen.insert(key)
        })
        .collect()
}

#[async_trait]
impl NewsProvider for Finnhub {
    fn name(&self) -> &'static str {
        "finnhub"
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
        let mut all = Vec::new();
        let mut first_error = None;
        for category in Self::categories(q) {
            match self.fetch_category(category).await {
                Ok(items) => all.extend(items),
                Err(e) => {
                    tracing::warn!(provider = "finnhub", category, error = %e, "news fetch failed");
                    first_error.get_or_insert(e);
                }
            }
        }
        // One category failing is fine as long as another gave something.
        match (all.is_empty(), first_error) {
            (true, Some(e)) => Err(e),
            _ => Ok(dedupe(all)),
        }
    }
}

//! `NewsService`: cache, relevance, direction and the recency-weighted score.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::finnhub::dedupe;
use crate::relevance::{directed_tone, is_relevant, mentions};
use crate::{Article, Headline, NewsError, NewsProvider, NewsQuery, NewsSummary};

/// How long a fetched summary is served without asking the provider again.
pub const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);
/// How old a cached summary may be when the provider fails and we serve it anyway.
pub const DEFAULT_STALE_MAX: Duration = Duration::from_secs(60 * 60);
/// Only headlines from this window count.
const WINDOW_HOURS: i64 = 24;
/// A headline's weight halves every this many hours.
const HALF_LIFE_HOURS: f64 = 6.0;

type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

struct Entry {
    fetched_at: DateTime<Utc>,
    score: f64,
    /// Every relevant headline in the window, newest first, tone already directed.
    headlines: Vec<Headline>,
}

impl Entry {
    fn summary(&self, limit: usize) -> NewsSummary {
        NewsSummary {
            score: self.score,
            label: label(self.score).to_string(),
            count: u32::try_from(self.headlines.len()).unwrap_or(u32::MAX),
            headlines: self.headlines.iter().take(limit).cloned().collect(),
        }
    }
}

/// Provider plus a cache per instrument. One request per instrument is in flight at a time, so
/// several callers asking for the same symbol cost a single provider call.
pub struct NewsService {
    provider: Option<Box<dyn NewsProvider>>,
    ttl: Duration,
    stale_max: Duration,
    clock: Clock,
    cache: StdMutex<HashMap<String, Arc<Mutex<Option<Entry>>>>>,
}

impl NewsService {
    pub fn new(provider: Box<dyn NewsProvider>) -> Self {
        Self {
            provider: Some(provider),
            ttl: DEFAULT_TTL,
            stale_max: DEFAULT_STALE_MAX,
            clock: Arc::new(Utc::now),
            cache: StdMutex::new(HashMap::new()),
        }
    }

    /// A service with no provider: `summary` always answers [`NewsError::NotConfigured`].
    pub fn disabled() -> Self {
        Self {
            provider: None,
            ..Self::new(Box::new(Unused))
        }
    }

    /// Cache lifetime (default 10 minutes).
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Oldest cached summary served when the provider fails (default 1 hour).
    pub fn with_stale_max(mut self, stale_max: Duration) -> Self {
        self.stale_max = stale_max;
        self
    }

    /// Replace the clock, so tests control time.
    pub fn with_clock(mut self, clock: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    pub fn is_configured(&self) -> bool {
        self.provider.is_some()
    }

    /// Provider name for display, `None` when disabled.
    pub fn provider_name(&self) -> Option<&'static str> {
        self.provider.as_ref().map(|p| p.name())
    }

    pub async fn summary(&self, q: &NewsQuery) -> Result<NewsSummary, NewsError> {
        let provider = self.provider.as_deref().ok_or(NewsError::NotConfigured)?;
        let limit = q.effective_limit();
        let slot = {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache
                .entry(q.canonical.trim().to_ascii_uppercase())
                .or_default()
                .clone()
        };
        // Held across the fetch on purpose: a second caller waits and then hits the cache.
        let mut slot = slot.lock().await;
        let now = (self.clock)();
        if let Some(e) = slot.as_ref()
            && age(now, e.fetched_at) < self.ttl
        {
            return Ok(e.summary(limit));
        }
        match provider.fetch_articles(q).await {
            Ok(articles) => {
                let entry = analyse(q, articles, now);
                let out = entry.summary(limit);
                *slot = Some(entry);
                Ok(out)
            }
            Err(e) => match slot.as_ref() {
                Some(old) if age(now, old.fetched_at) < self.stale_max => {
                    tracing::warn!(error = %e, "news fetch failed, serving cached summary");
                    Ok(old.summary(limit))
                }
                _ => Err(e),
            },
        }
    }
}

/// Used only to fill the provider slot of a disabled service.
struct Unused;

#[async_trait::async_trait]
impl NewsProvider for Unused {
    fn name(&self) -> &'static str {
        "none"
    }
    async fn fetch(&self, _q: &NewsQuery) -> Result<Vec<Headline>, NewsError> {
        Err(NewsError::NotConfigured)
    }
}

/// Elapsed time; a clock that moved backwards counts as just fetched.
fn age(now: DateTime<Utc>, then: DateTime<Utc>) -> Duration {
    (now - then).to_std().unwrap_or(Duration::ZERO)
}

/// Label for a score: `negative` ≤ -0.35 < `slightly negative` ≤ -0.1 < `neutral` < 0.1 ≤
/// `slightly positive` < 0.35 ≤ `positive`.
pub fn label(score: f64) -> &'static str {
    if score <= -0.35 {
        "negative"
    } else if score <= -0.1 {
        "slightly negative"
    } else if score < 0.1 {
        "neutral"
    } else if score < 0.35 {
        "slightly positive"
    } else {
        "positive"
    }
}

fn analyse(q: &NewsQuery, articles: Vec<Article>, now: DateTime<Utc>) -> Entry {
    let since = q.since.max(now - chrono::Duration::hours(WINDOW_HOURS));
    let mut terms = q.terms.clone();
    terms.push(q.canonical.clone());
    let pair = match (q.base.as_deref(), q.quote.as_deref()) {
        (Some(b), Some(qt)) if !b.trim().is_empty() && !qt.trim().is_empty() => {
            Some((b.trim(), qt.trim()))
        }
        _ => None,
    };

    let mut kept: Vec<Headline> = dedupe(articles)
        .into_iter()
        .filter(|a| a.headline.published_at >= since)
        .filter(|a| a.scoped || is_relevant(&terms, &a.headline.title, &a.summary))
        .map(|a| {
            let mut h = a.headline;
            h.tone = h.tone.clamp(-1.0, 1.0);
            if let (Some((base, quote)), false) = (pair, a.directed) {
                let m = mentions(&h.title, &a.summary, base, quote);
                h.tone = directed_tone(h.tone, m);
            }
            h
        })
        .collect();
    kept.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| a.title.cmp(&b.title))
    });

    let (mut weighted, mut total) = (0.0, 0.0);
    for h in &kept {
        let hours = (now - h.published_at).num_seconds().max(0) as f64 / 3600.0;
        let w = 0.5_f64.powf(hours / HALF_LIFE_HOURS);
        weighted += w * h.tone;
        total += w;
    }
    let score = if total > 0.0 {
        (weighted / total).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    Entry {
        fetched_at: now,
        score,
        headlines: kept,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_thresholds() {
        assert_eq!(label(-1.0), "negative");
        assert_eq!(label(-0.35), "negative");
        assert_eq!(label(-0.3499), "slightly negative");
        assert_eq!(label(-0.1), "slightly negative");
        assert_eq!(label(-0.0999), "neutral");
        assert_eq!(label(0.0), "neutral");
        assert_eq!(label(0.0999), "neutral");
        assert_eq!(label(0.1), "slightly positive");
        assert_eq!(label(0.3499), "slightly positive");
        assert_eq!(label(0.35), "positive");
        assert_eq!(label(1.0), "positive");
    }
}

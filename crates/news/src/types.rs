use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::NewsError;

/// One news item. `tone` is -1..1; from a service summary it already points in the
/// instrument's direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Headline {
    pub title: String,
    pub source: String,
    pub url: String,
    pub published_at: DateTime<Utc>,
    pub tone: f64,
}

/// What to look for. `terms` are the words that make a headline relevant (canonical name,
/// aliases, keywords, currency names); `base`/`quote` turn on direction mapping for a pair.
#[derive(Debug, Clone)]
pub struct NewsQuery {
    pub canonical: String,
    pub terms: Vec<String>,
    pub base: Option<String>,
    pub quote: Option<String>,
    /// Oldest headline to keep. The service never looks back further than 24 hours.
    pub since: DateTime<Utc>,
    /// Headlines to return in the summary; 0 means [`NewsQuery::DEFAULT_LIMIT`].
    pub limit: usize,
}

impl NewsQuery {
    pub const DEFAULT_LIMIT: usize = 5;

    /// A query for the last 24 hours with no terms (the canonical name still counts).
    pub fn new(canonical: impl Into<String>) -> Self {
        Self {
            canonical: canonical.into(),
            terms: Vec::new(),
            base: None,
            quote: None,
            since: Utc::now() - Duration::hours(24),
            limit: Self::DEFAULT_LIMIT,
        }
    }

    pub fn with_terms<I, S>(mut self, terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.terms = terms.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_pair(mut self, base: impl Into<String>, quote: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self.quote = Some(quote.into());
        self
    }

    pub(crate) fn effective_limit(&self) -> usize {
        if self.limit == 0 {
            Self::DEFAULT_LIMIT
        } else {
            self.limit
        }
    }
}

/// A fetched item plus what the service needs to judge it. Providers fill this in; a provider
/// that only implements [`NewsProvider::fetch`] gets the defaults.
#[derive(Debug, Clone)]
pub struct Article {
    pub headline: Headline,
    /// Body text or blurb, used for relevance and mentions. May be empty.
    pub summary: String,
    /// The provider already selected this article for the instrument (for example by ticker),
    /// so the term filter is skipped.
    pub scoped: bool,
    /// `headline.tone` already points in the pair's direction, so mentions are not applied.
    pub directed: bool,
}

impl Article {
    pub fn from_headline(headline: Headline) -> Self {
        Self {
            headline,
            summary: String::new(),
            scoped: false,
            directed: false,
        }
    }
}

#[async_trait]
pub trait NewsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Recent headlines for the instrument, tone from the lexicon or the provider's own score.
    /// Not filtered for relevance.
    async fn fetch(&self, q: &NewsQuery) -> Result<Vec<Headline>, NewsError>;

    /// Like [`fetch`](Self::fetch) with the extra text and hints the service uses. Providers
    /// that have them override this; the default wraps `fetch`.
    async fn fetch_articles(&self, q: &NewsQuery) -> Result<Vec<Article>, NewsError> {
        Ok(self
            .fetch(q)
            .await?
            .into_iter()
            .map(Article::from_headline)
            .collect())
    }
}

/// The news section of a market view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsSummary {
    /// -1..1, in the instrument's direction.
    pub score: f64,
    pub label: String,
    /// Relevant headlines in the last 24 hours (the list below is capped by `limit`).
    pub count: u32,
    pub headlines: Vec<Headline>,
}

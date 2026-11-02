//! News headlines and sentiment for market views.
//!
//! Flow: a [`NewsProvider`] ([`Finnhub`] or [`AlphaVantage`]) fetches recent headlines.
//! [`NewsService::summary`] keeps the ones that name the instrument (whole-word match of the
//! query terms), gives each a tone (the provider's own score, or the word list in
//! [`lexicon`]), points that tone in the instrument's direction for a currency pair, and
//! averages the last 24 hours with a 6-hour half-life so recent items count more. Results are
//! cached per instrument, and a recent cached summary is served if the provider fails.
//!
//! Limits: a word list cannot read sarcasm, context or "bad news is good news" for safe
//! havens such as gold, the yen or the franc, and it sees a rate cut only as bad for the
//! currency. That is why the market view gives news a small weight. Keys are never logged or
//! put in errors; see the provider modules for how each one sends its key.
//!
//! See `docs/market-view.md` for how the market view uses the summary.

mod error;
pub mod lexicon;
mod types;

mod alphavantage;
mod finnhub;
mod http;
mod instrument;
mod relevance;
mod service;
mod text;

pub use alphavantage::AlphaVantage;
pub use error::NewsError;
pub use finnhub::Finnhub;
pub use http::DEFAULT_TIMEOUT;
pub use relevance::{Mentions, currency_terms, directed_tone, is_relevant, mentions};
pub use service::{DEFAULT_STALE_MAX, DEFAULT_TTL, NewsService, label};
pub use types::{Article, Headline, NewsProvider, NewsQuery, NewsSummary};

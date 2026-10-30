use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use jarvis_news::{Headline, NewsError, NewsProvider, NewsQuery, NewsService, lexicon};

type Responder = Box<dyn FnMut(usize) -> Result<Vec<Headline>, NewsError> + Send>;

/// A provider whose answers come from a closure called with the call number (0-based).
struct Fake {
    calls: Arc<AtomicUsize>,
    respond: Mutex<Responder>,
    delay: Duration,
}

#[async_trait]
impl NewsProvider for Fake {
    fn name(&self) -> &'static str {
        "fake"
    }
    async fn fetch(&self, _q: &NewsQuery) -> Result<Vec<Headline>, NewsError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        (self.respond.lock().unwrap())(n)
    }
}

struct Rig {
    service: NewsService,
    calls: Arc<AtomicUsize>,
    clock: Arc<AtomicI64>,
}

impl Rig {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn advance(&self, d: Duration) {
        self.clock.fetch_add(d.as_secs() as i64, Ordering::SeqCst);
    }
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.clock.load(Ordering::SeqCst), 0)
            .unwrap()
    }
    fn query(&self, canonical: &str) -> NewsQuery {
        let mut q = NewsQuery::new(canonical);
        q.since = self.now() - chrono::Duration::hours(24);
        q
    }
}

/// 2026-10-30 14:00 UTC.
const T0: i64 = 1_793_368_800;

fn rig_with(
    delay: Duration,
    respond: impl FnMut(usize) -> Result<Vec<Headline>, NewsError> + Send + 'static,
) -> Rig {
    let calls = Arc::new(AtomicUsize::new(0));
    let clock = Arc::new(AtomicI64::new(T0));
    let fake = Fake {
        calls: calls.clone(),
        respond: Mutex::new(Box::new(respond)),
        delay,
    };
    let c = clock.clone();
    let service = NewsService::new(Box::new(fake))
        .with_clock(move || Utc.timestamp_opt(c.load(Ordering::SeqCst), 0).unwrap());
    Rig {
        service,
        calls,
        clock,
    }
}

fn rig(respond: impl FnMut(usize) -> Result<Vec<Headline>, NewsError> + Send + 'static) -> Rig {
    rig_with(Duration::ZERO, respond)
}

/// A headline `age_minutes` old whose tone comes from the word list, like a real provider.
fn headline(title: &str, age_minutes: i64) -> Headline {
    Headline {
        title: title.to_string(),
        source: "Wire".to_string(),
        url: format!("https://example.com/{}", title.replace(' ', "-")),
        published_at: Utc.timestamp_opt(T0 - age_minutes * 60, 0).unwrap(),
        tone: lexicon::tone(title),
    }
}

fn with_tone(mut h: Headline, tone: f64) -> Headline {
    h.tone = tone;
    h
}

fn eurusd(rig: &Rig) -> NewsQuery {
    rig.query("EURUSD")
        .with_terms(["EURUSD", "EUR/USD", "euro", "dollar", "ECB", "Fed"])
        .with_pair("EUR", "USD")
}

#[tokio::test]
async fn disabled_service_says_not_configured() {
    let s = NewsService::disabled();
    assert!(!s.is_configured());
    assert_eq!(s.provider_name(), None);
    let err = s.summary(&NewsQuery::new("EURUSD")).await.unwrap_err();
    assert!(matches!(err, NewsError::NotConfigured));
}

#[tokio::test]
async fn configured_service_reports_its_provider() {
    let r = rig(|_| Ok(vec![]));
    assert!(r.service.is_configured());
    assert_eq!(r.service.provider_name(), Some("fake"));
}

#[tokio::test]
async fn cache_serves_within_ttl_and_refreshes_after() {
    let r = rig(|_| Ok(vec![headline("Euro rallies", 5)]));
    let q = eurusd(&r);
    r.service.summary(&q).await.unwrap();
    r.advance(Duration::from_secs(9 * 60));
    r.service.summary(&q).await.unwrap();
    assert_eq!(r.calls(), 1, "inside the 10 minute default");
    r.advance(Duration::from_secs(2 * 60));
    r.service.summary(&q).await.unwrap();
    assert_eq!(r.calls(), 2, "past the TTL");
    // The new fetch restarts the TTL.
    r.advance(Duration::from_secs(5 * 60));
    r.service.summary(&q).await.unwrap();
    assert_eq!(r.calls(), 2);
}

#[tokio::test]
async fn ttl_is_configurable() {
    let r = rig(|_| Ok(vec![]));
    let service = r.service.with_ttl(Duration::from_secs(60));
    let q = NewsQuery::new("US500");
    service.summary(&q).await.unwrap();
    service.summary(&q).await.unwrap();
    assert_eq!(r.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cache_is_per_instrument_and_ignores_case() {
    let r = rig(|_| Ok(vec![]));
    r.service.summary(&r.query("EURUSD")).await.unwrap();
    r.service.summary(&r.query("eurusd")).await.unwrap();
    assert_eq!(r.calls(), 1);
    r.service.summary(&r.query("GBPUSD")).await.unwrap();
    assert_eq!(r.calls(), 2);
}

#[tokio::test]
async fn the_limit_applies_to_the_reply_not_the_cache() {
    let r = rig(|_| {
        Ok((0..7)
            .map(|i| headline(&format!("Euro rallies {i}"), 10 + i))
            .collect())
    });
    let mut q = eurusd(&r);
    q.limit = 3;
    let s = r.service.summary(&q).await.unwrap();
    assert_eq!(s.count, 7);
    assert_eq!(s.headlines.len(), 3);
    // Newest first.
    assert!(s.headlines[0].published_at > s.headlines[1].published_at);
    assert_eq!(s.headlines[0].title, "Euro rallies 0");

    // A different limit is answered from the cache.
    q.limit = 0;
    let s = r.service.summary(&q).await.unwrap();
    assert_eq!(s.headlines.len(), 5, "default limit is 5");
    q.limit = 100;
    assert_eq!(r.service.summary(&q).await.unwrap().headlines.len(), 7);
    assert_eq!(r.calls(), 1);
}

#[tokio::test]
async fn stale_summary_is_served_when_the_provider_fails() {
    let r = rig(|n| {
        if n == 0 {
            Ok(vec![headline("Euro rallies", 5)])
        } else {
            Err(NewsError::RateLimited)
        }
    });
    let q = eurusd(&r);
    let fresh = r.service.summary(&q).await.unwrap();

    r.advance(Duration::from_secs(20 * 60));
    let stale = r.service.summary(&q).await.unwrap();
    assert_eq!(r.calls(), 2, "it did try again");
    assert_eq!(stale, fresh);

    // An hour or more old: the error comes through.
    r.advance(Duration::from_secs(45 * 60));
    let err = r.service.summary(&q).await.unwrap_err();
    assert!(matches!(err, NewsError::RateLimited), "{err:?}");
}

#[tokio::test]
async fn error_without_a_cached_value_is_returned() {
    let r = rig(|_| Err(NewsError::Timeout));
    let err = r.service.summary(&eurusd(&r)).await.unwrap_err();
    assert!(matches!(err, NewsError::Timeout));
    // And a failure is not cached: the next call asks again.
    let _ = r.service.summary(&eurusd(&r)).await;
    assert_eq!(r.calls(), 2);
}

#[tokio::test]
async fn concurrent_callers_share_one_fetch() {
    let r = rig_with(Duration::from_millis(100), |_| {
        Ok(vec![headline("Euro rallies", 5)])
    });
    let q = eurusd(&r);
    let (a, b, c) = tokio::join!(
        r.service.summary(&q),
        r.service.summary(&q),
        r.service.summary(&q)
    );
    assert!(a.is_ok() && b.is_ok() && c.is_ok());
    assert_eq!(r.calls(), 1);
}

#[tokio::test]
async fn irrelevant_headlines_are_dropped_and_not_counted() {
    let r = rig(|_| {
        Ok(vec![
            headline("Euro rallies", 10),
            headline("Pizza chain posts record sales", 10),
            headline("Goldman upgrades banks", 10),
        ])
    });
    let s = r.service.summary(&eurusd(&r)).await.unwrap();
    assert_eq!(s.count, 1);
    assert_eq!(s.headlines.len(), 1);
}

#[tokio::test]
async fn the_canonical_name_is_always_a_term() {
    let r = rig(|_| Ok(vec![headline("XAUUSD breaks resistance", 10)]));
    let s = r.service.summary(&r.query("XAUUSD")).await.unwrap();
    assert_eq!(s.count, 1);
}

#[tokio::test]
async fn only_the_last_24_hours_count() {
    let r = rig(|_| {
        Ok(vec![
            headline("Euro rallies", 23 * 60),
            headline("Euro plunges", 25 * 60),
        ])
    });
    // Even with a wider `since`, the window stops at 24 hours.
    let mut q = eurusd(&r);
    q.since = r.now() - chrono::Duration::hours(72);
    let s = r.service.summary(&q).await.unwrap();
    assert_eq!(s.count, 1);
    assert_eq!(s.headlines[0].title, "Euro rallies");
}

#[tokio::test]
async fn a_later_since_narrows_the_window() {
    let r = rig(|_| {
        Ok(vec![
            headline("Euro rallies", 30),
            headline("Euro plunges", 120),
        ])
    });
    let mut q = eurusd(&r);
    q.since = r.now() - chrono::Duration::hours(1);
    assert_eq!(r.service.summary(&q).await.unwrap().count, 1);
}

#[tokio::test]
async fn no_headlines_is_neutral() {
    let r = rig(|_| Ok(vec![]));
    let s = r.service.summary(&eurusd(&r)).await.unwrap();
    assert_eq!(s.count, 0);
    assert_eq!(s.score, 0.0);
    assert_eq!(s.label, "neutral");
    assert!(s.headlines.is_empty());
}

#[tokio::test]
async fn recency_weighting_uses_a_six_hour_half_life() {
    // Fresh +1 (weight 1) and 6 hours old -1 (weight 0.5): (1 - 0.5) / 1.5.
    let r = rig(|_| {
        Ok(vec![
            headline("Euro rallies", 0),
            headline("Euro plunges", 6 * 60),
        ])
    });
    let s = r.service.summary(&eurusd(&r)).await.unwrap();
    assert!((s.score - 1.0 / 3.0).abs() < 1e-9, "{}", s.score);
    assert_eq!(s.label, "slightly positive");

    // 12 hours old counts a quarter: (1 - 0.25) / 1.25 = 0.6.
    let r = rig(|_| {
        Ok(vec![
            headline("Euro rallies", 0),
            headline("Euro plunges", 12 * 60),
        ])
    });
    let s = r.service.summary(&eurusd(&r)).await.unwrap();
    assert!((s.score - 0.6).abs() < 1e-9, "{}", s.score);
    assert_eq!(s.label, "positive");
}

#[tokio::test]
async fn equal_ages_are_a_plain_mean() {
    let r = rig(|_| {
        Ok(vec![
            with_tone(headline("Euro item one", 60), 0.8),
            with_tone(headline("Euro item two", 60), 0.2),
            with_tone(headline("Euro item three", 60), -0.4),
        ])
    });
    let s = r.service.summary(&eurusd(&r)).await.unwrap();
    assert!((s.score - 0.2).abs() < 1e-9);
}

async fn score_of(title: &str) -> (f64, Vec<Headline>) {
    let title = title.to_string();
    let r = rig(move |_| Ok(vec![headline(&title, 0)]));
    let s = r.service.summary(&eurusd(&r)).await.unwrap();
    (s.score, s.headlines)
}

#[tokio::test]
async fn euro_positive_headline_is_positive_for_eurusd() {
    let (score, headlines) = score_of("Euro rallies as ECB turns hawkish").await;
    assert_eq!(score, 1.0);
    assert_eq!(headlines[0].tone, 1.0);
}

#[tokio::test]
async fn dollar_positive_headline_is_negative_for_eurusd() {
    let (score, headlines) = score_of("Dollar surges on strong payrolls").await;
    assert_eq!(score, -1.0);
    assert_eq!(
        headlines[0].tone, -1.0,
        "the returned tone is the directed one"
    );
}

#[tokio::test]
async fn a_headline_about_both_currencies_counts_zero() {
    let (score, headlines) = score_of("Euro slips as dollar rallies").await;
    assert_eq!(score, 0.0);
    assert_eq!(headlines[0].tone, 0.0);
    assert_eq!(headlines.len(), 1, "it still counts as a relevant headline");
}

#[tokio::test]
async fn naming_the_pair_keeps_the_tone() {
    let (score, _) = score_of("EUR/USD falls to a two week low").await;
    assert_eq!(score, -1.0);
}

#[tokio::test]
async fn other_instruments_keep_the_tone() {
    let r = rig(|_| Ok(vec![headline("Dollar surges, gold rallies", 0)]));
    let q = r.query("XAUUSD").with_terms(["gold"]);
    let s = r.service.summary(&q).await.unwrap();
    assert_eq!(s.score, 1.0);
}

#[tokio::test]
async fn gold_as_a_pair_flips_on_dollar_headlines() {
    let r = rig(|_| Ok(vec![headline("Dollar surges", 0)]));
    let q = r
        .query("XAUUSD")
        .with_terms(["gold", "dollar"])
        .with_pair("XAU", "USD");
    assert_eq!(r.service.summary(&q).await.unwrap().score, -1.0);
}

#[tokio::test]
async fn duplicate_urls_count_once() {
    let r = rig(|_| {
        Ok(vec![
            headline("Euro rallies", 10),
            headline("Euro rallies", 10),
        ])
    });
    assert_eq!(r.service.summary(&eurusd(&r)).await.unwrap().count, 1);
}

#[tokio::test]
async fn future_dated_items_do_not_get_extra_weight() {
    // A skewed source clock: the item is 10 minutes ahead. It weighs 1, like a fresh one.
    let r = rig(|_| {
        Ok(vec![
            headline("Euro rallies", -10),
            headline("Euro plunges", 0),
        ])
    });
    let s = r.service.summary(&eurusd(&r)).await.unwrap();
    assert!(s.score.abs() < 1e-9, "{}", s.score);
}

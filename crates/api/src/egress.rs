//! Outbound connections (FL-3): every HTTP request the backend makes to another service is
//! recorded here with its host and why it was made, so the user can check that nothing left
//! the PC. In memory only, bounded, never written to disk.
//!
//! Recording happens in thin wrappers around the callers ([`EgressProvider`] for language
//! models, [`EgressNews`] for news providers); the HTTP clients have redirects turned off, so
//! the recorded host is the host that was contacted.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use jarvis_llm::{ChatRequest, ChatResponse, LlmError, LlmProvider};
use jarvis_news::{Article, Headline, NewsError, NewsProvider, NewsQuery};
use serde::Serialize;

/// How many entries are kept.
pub const EGRESS_CAPACITY: usize = 200;

/// One outbound request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EgressEntry {
    /// Increases by one per entry; ask for "everything after seq N" with [`EgressLog::since`].
    pub seq: u64,
    /// Unix milliseconds when the request finished.
    pub ts_ms: i64,
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    /// `llm:<provider>` or `news:<provider>` (the preflight adds `preflight:ollama`).
    pub purpose: String,
    pub ok: bool,
    pub ms: u64,
    /// Whether the host is this machine ([`is_loopback`]).
    pub local: bool,
}

/// The parts of a URL the log keeps: scheme, host and explicit port. `None` when the URL has no
/// host. Parsed with the same URL parser the HTTP client uses, so the host recorded is the host
/// the client connects to (a hand-written split can be fooled by `\` or `@` tricks).
pub fn url_target(url: &str) -> Option<(String, String, Option<u16>)> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    let host = parsed
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some((parsed.scheme().to_owned(), host, parsed.port()))
}

/// `localhost`, `127.0.0.0/8` and `::1` (also an IPv4-mapped `::ffff:127.x.y.z`). Nothing else:
/// not a LAN address (another machine), and not `*.localhost` names, which a resolver may send
/// anywhere.
pub fn is_loopback(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).trim_end_matches('.');
    if h.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        Err(_) => false,
    }
}

/// The distinct hosts in `entries` that are not this machine, sorted.
pub fn non_loopback_hosts(entries: &[EgressEntry]) -> Vec<String> {
    let mut hosts: Vec<String> = entries
        .iter()
        .filter(|e| !e.local)
        .map(|e| e.host.clone())
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

#[derive(Default)]
struct Ring {
    next_seq: u64,
    entries: VecDeque<EgressEntry>,
}

/// The log. Cheap to share (`Arc`).
#[derive(Default)]
pub struct EgressLog {
    ring: Mutex<Ring>,
}

impl EgressLog {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record one finished request to `url`. A URL without a host is recorded with the host
    /// `unknown` (and so counts as not local). Returns the entry's `seq`.
    pub fn record(&self, url: &str, purpose: &str, ok: bool, ms: u64) -> u64 {
        let (scheme, host, port) =
            url_target(url).unwrap_or_else(|| (String::new(), "unknown".to_owned(), None));
        let local = is_loopback(&host);
        let mut ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        ring.next_seq += 1;
        let seq = ring.next_seq;
        if ring.entries.len() == EGRESS_CAPACITY {
            ring.entries.pop_front();
        }
        ring.entries.push_back(EgressEntry {
            seq,
            ts_ms: chrono::Utc::now().timestamp_millis(),
            scheme,
            host,
            port,
            purpose: purpose.to_owned(),
            ok,
            ms,
            local,
        });
        seq
    }

    /// The `seq` of the newest entry (0 when there is none yet).
    pub fn last_seq(&self) -> u64 {
        self.ring.lock().unwrap_or_else(|p| p.into_inner()).next_seq
    }

    /// Entries with a `seq` greater than `seq`, oldest first (0 = everything still kept).
    pub fn since(&self, seq: u64) -> Vec<EgressEntry> {
        self.ring
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entries
            .iter()
            .filter(|e| e.seq > seq)
            .cloned()
            .collect()
    }
}

/// A language model whose every request is recorded in the egress log.
pub struct EgressProvider {
    inner: Arc<dyn LlmProvider>,
    url: String,
    purpose: String,
    log: Arc<EgressLog>,
}

impl EgressProvider {
    /// `url`: the provider's base URL (where its requests go).
    pub fn wrap(
        inner: Arc<dyn LlmProvider>,
        url: &str,
        name: &str,
        log: Arc<EgressLog>,
    ) -> Arc<dyn LlmProvider> {
        Arc::new(Self {
            inner,
            url: url.to_owned(),
            purpose: format!("llm:{name}"),
            log,
        })
    }
}

#[async_trait]
impl LlmProvider for EgressProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn model(&self) -> &str {
        self.inner.model()
    }
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let started = Instant::now();
        let result = self.inner.chat(request).await;
        self.log.record(
            &self.url,
            &self.purpose,
            result.is_ok(),
            elapsed_ms(started),
        );
        result
    }
}

/// A news provider whose every request is recorded in the egress log.
pub struct EgressNews {
    inner: Box<dyn NewsProvider>,
    url: String,
    purpose: String,
    log: Arc<EgressLog>,
}

impl EgressNews {
    pub fn wrap(inner: Box<dyn NewsProvider>, log: Arc<EgressLog>) -> Box<dyn NewsProvider> {
        let url = inner.base_url().unwrap_or_default().to_owned();
        let purpose = format!("news:{}", inner.name());
        Box::new(Self {
            inner,
            url,
            purpose,
            log,
        })
    }

    fn done(&self, started: Instant, ok: bool) {
        self.log
            .record(&self.url, &self.purpose, ok, elapsed_ms(started));
    }
}

#[async_trait]
impl NewsProvider for EgressNews {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn base_url(&self) -> Option<&str> {
        self.inner.base_url()
    }
    async fn fetch(&self, q: &NewsQuery) -> Result<Vec<Headline>, NewsError> {
        let started = Instant::now();
        let result = self.inner.fetch(q).await;
        self.done(started, result.is_ok());
        result
    }
    async fn fetch_articles(&self, q: &NewsQuery) -> Result<Vec<Article>, NewsError> {
        let started = Instant::now();
        let result = self.inner.fetch_articles(q).await;
        self.done(started, result.is_ok());
        result
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_only_this_machine() {
        for local in [
            "localhost",
            "LOCALHOST",
            "localhost.",
            "127.0.0.1",
            "127.8.9.10",
            "::1",
            "[::1]",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_loopback(local), "{local}");
        }
        for remote in [
            "api.anthropic.com",
            "192.168.1.10",
            "10.0.0.1",
            "0.0.0.0",
            "localhost.example.com",
            "127.0.0.1.nip.io",
            "app.localhost",
            "ollama.localhost",
            "",
            "unknown",
        ] {
            assert!(!is_loopback(remote), "{remote}");
        }
    }

    #[test]
    fn url_target_reads_scheme_host_and_port() {
        assert_eq!(
            url_target("http://127.0.0.1:11434"),
            Some(("http".into(), "127.0.0.1".into(), Some(11434)))
        );
        assert_eq!(
            url_target("https://API.Anthropic.com/v1/messages?x=1"),
            Some(("https".into(), "api.anthropic.com".into(), None))
        );
        assert_eq!(
            url_target("http://[::1]:3900/health"),
            Some(("http".into(), "::1".into(), Some(3900)))
        );
        assert_eq!(
            url_target("https://user:pw@finnhub.io/api"),
            Some(("https".into(), "finnhub.io".into(), None))
        );
        assert_eq!(url_target("no scheme"), None);
        assert_eq!(url_target("mailto:someone"), None);
        // The parser the HTTP client uses: a backslash ends the host, so this goes to evil.com.
        assert_eq!(
            url_target(r"http://evil.com\@127.0.0.1:11434").map(|t| t.1),
            Some("evil.com".into())
        );
        assert_eq!(
            url_target("http://127.0.0.1:11434@evil.com/").map(|t| t.1),
            Some("evil.com".into())
        );
    }

    #[test]
    fn the_log_is_bounded_and_answers_since() {
        let log = EgressLog::new();
        assert_eq!(log.last_seq(), 0);
        for i in 0..(EGRESS_CAPACITY + 5) {
            log.record("http://127.0.0.1:11434", &format!("llm:{i}"), true, 1);
        }
        let all = log.since(0);
        assert_eq!(all.len(), EGRESS_CAPACITY);
        assert_eq!(all[0].seq, 6);
        assert_eq!(log.last_seq(), (EGRESS_CAPACITY + 5) as u64);
        let mark = log.last_seq();
        log.record("https://api.anthropic.com", "llm:claude", false, 9);
        let new = log.since(mark);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].host, "api.anthropic.com");
        assert!(!new[0].local && !new[0].ok);
        assert_eq!(non_loopback_hosts(&log.since(0)), ["api.anthropic.com"]);
    }

    #[test]
    fn a_url_without_a_host_counts_as_remote() {
        let log = EgressLog::new();
        log.record("", "news:finnhub", true, 0);
        assert_eq!(non_loopback_hosts(&log.since(0)), ["unknown"]);
    }
}

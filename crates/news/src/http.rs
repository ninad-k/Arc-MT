//! Plumbing shared by the providers: client, URL checks, error mapping and key redaction.

use std::time::Duration;

use reqwest::Url;

use crate::NewsError;

/// Per-request limit for every provider unless a test or the API overrides it.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

const EXCERPT_CHARS: usize = 200;

pub(crate) fn build_client() -> Result<reqwest::Client, NewsError> {
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_TIMEOUT)
        .user_agent(concat!("jarvis-news/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| NewsError::Transport(format!("cannot build HTTP client: {}", e.without_url())))
}

/// Validate a base URL and drop trailing slashes. Plain `http` is accepted for a loopback host
/// only (a test double), because the key is sent with every request.
pub(crate) fn parse_base(raw: &str) -> Result<String, NewsError> {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    let url = Url::parse(&trimmed)
        .map_err(|_| NewsError::Transport("invalid news provider base URL".into()))?;
    let host = url
        .host_str()
        .unwrap_or("")
        .trim_start_matches('[')
        .trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if url.scheme() == "https" || (url.scheme() == "http" && loopback) {
        Ok(trimmed)
    } else {
        Err(NewsError::Transport(
            "news provider base URL must use https (http is accepted for localhost only)".into(),
        ))
    }
}

/// Remove the key (raw and percent-encoded) from text that may echo it.
pub(crate) fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    let mut out = text.replace(secret, "[redacted]");
    let encoded = percent_encode(secret);
    if encoded != secret {
        out = out.replace(&encoded, "[redacted]");
    }
    out
}

fn percent_encode(s: &str) -> String {
    let mut u = Url::parse("http://x/").expect("static URL");
    u.query_pairs_mut().append_pair("k", s);
    u.query()
        .and_then(|q| q.strip_prefix("k="))
        .unwrap_or(s)
        .to_string()
}

/// One line, redacted, at most [`EXCERPT_CHARS`] characters.
pub(crate) fn excerpt(body: &str, secret: &str) -> String {
    let clean = redact(body, secret);
    let one_line = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return "empty response body".to_string();
    }
    if one_line.chars().count() <= EXCERPT_CHARS {
        one_line
    } else {
        let mut cut: String = one_line.chars().take(EXCERPT_CHARS).collect();
        cut.push_str("...");
        cut
    }
}

pub(crate) fn status_error(status: u16, body: &str, secret: &str) -> NewsError {
    match status {
        401 => NewsError::NotConfigured,
        429 => NewsError::RateLimited,
        _ => NewsError::Http {
            status,
            body: excerpt(body, secret),
        },
    }
}

pub(crate) fn map_reqwest(e: reqwest::Error, secret: &str) -> NewsError {
    if e.is_timeout() {
        NewsError::Timeout
    } else {
        // `without_url` matters: for Alpha Vantage the URL carries the key.
        NewsError::Transport(excerpt(&e.without_url().to_string(), secret))
    }
}

pub(crate) fn decode_error(e: &serde_json::Error, secret: &str) -> NewsError {
    NewsError::Decode(excerpt(&e.to_string(), secret))
}

/// Send a prepared GET and return the status and body text.
pub(crate) async fn get_text(
    req: reqwest::RequestBuilder,
    timeout: Duration,
    secret: &str,
) -> Result<(u16, String), NewsError> {
    let resp = req
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| map_reqwest(e, secret))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| map_reqwest(e, secret))?;
    Ok((status, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_raw_and_encoded_key() {
        let out = redact("bad key a b&c and a+b%26c", "a b&c");
        assert!(!out.contains("a b&c") && !out.contains("a+b%26c"), "{out}");
    }

    #[test]
    fn excerpt_is_short_and_single_line() {
        let body = format!("line one\n{}", "x".repeat(500));
        let e = excerpt(&body, "k");
        assert!(!e.contains('\n'));
        assert!(e.chars().count() <= EXCERPT_CHARS + 3);
    }

    #[test]
    fn base_url_rules() {
        assert!(parse_base("https://finnhub.io/").is_ok());
        assert!(parse_base("http://127.0.0.1:8080").is_ok());
        assert!(parse_base("http://example.com").is_err());
        assert!(parse_base("not a url").is_err());
    }
}

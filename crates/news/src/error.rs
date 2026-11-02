/// Errors from a news provider or the service.
///
/// No variant ever carries an API key: provider code redacts response excerpts and strips the
/// request URL from transport errors before building one of these.
#[derive(Debug, thiserror::Error)]
pub enum NewsError {
    /// No provider is set up, or the provider rejected the key (HTTP 401 or an invalid-key
    /// message).
    #[error("news provider not configured or its API key was rejected")]
    NotConfigured,
    /// Any other non-2xx answer. `body` is a short, single-line excerpt with the key removed.
    #[error("news provider returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("news provider request timed out")]
    Timeout,
    /// HTTP 429, or a 200 body saying the quota is used up.
    #[error("news provider rate limit reached")]
    RateLimited,
    /// The answer was not the JSON we expected.
    #[error("news provider response could not be read: {0}")]
    Decode(String),
    /// The request failed before an answer arrived (DNS, connection, TLS).
    #[error("news provider request failed: {0}")]
    Transport(String),
}

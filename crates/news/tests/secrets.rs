//! The API key must not show up in any error text or debug output, whatever the failure.

use jarvis_news::{AlphaVantage, Finnhub, NewsError, NewsProvider, NewsQuery};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "test-key-123";

fn shown(err: &NewsError) -> String {
    format!("{err} | {err:?}")
}

/// A loopback address nothing listens on.
fn closed_port_uri() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn transport_errors_carry_no_url_or_key() {
    let uri = closed_port_uri();
    let av = AlphaVantage::new(KEY).unwrap().with_base_url(&uri).unwrap();
    let err = av.fetch(&NewsQuery::new("BTCUSD")).await.unwrap_err();
    assert!(matches!(err, NewsError::Transport(_)), "{err:?}");
    let text = shown(&err);
    assert!(!text.contains(KEY), "{text}");
    assert!(!text.contains("apikey"), "{text}");
    assert!(!text.contains("/query"), "{text}");

    let fh = Finnhub::new(KEY).unwrap().with_base_url(&uri).unwrap();
    let err = fh.fetch(&NewsQuery::new("EURUSD")).await.unwrap_err();
    assert!(!shown(&err).contains(KEY));
}

#[tokio::test]
async fn http_error_bodies_that_echo_the_key_are_redacted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string(format!(
            "upstream error for GET /query?apikey={KEY}&function=NEWS_SENTIMENT token {KEY}"
        )))
        .mount(&server)
        .await;
    let av = AlphaVantage::new(KEY)
        .unwrap()
        .with_base_url(&server.uri())
        .unwrap();
    let err = av.fetch(&NewsQuery::new("US500")).await.unwrap_err();
    let text = shown(&err);
    assert!(text.contains("[redacted]"), "{text}");
    assert!(!text.contains(KEY), "{text}");

    let fh = Finnhub::new(KEY)
        .unwrap()
        .with_base_url(&server.uri())
        .unwrap();
    let err = fh.fetch(&NewsQuery::new("EURUSD")).await.unwrap_err();
    assert!(!shown(&err).contains(KEY));
}

#[tokio::test]
async fn decode_errors_do_not_echo_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("{KEY} is not json")))
        .mount(&server)
        .await;
    let av = AlphaVantage::new(KEY)
        .unwrap()
        .with_base_url(&server.uri())
        .unwrap();
    let err = av.fetch(&NewsQuery::new("US500")).await.unwrap_err();
    assert!(matches!(err, NewsError::Decode(_)));
    assert!(!shown(&err).contains(KEY));
}

#[test]
fn provider_debug_output_hides_the_key() {
    let av = format!("{:?}", AlphaVantage::new(KEY).unwrap());
    let fh = format!("{:?}", Finnhub::new(KEY).unwrap());
    assert!(!av.contains(KEY) && !fh.contains(KEY), "{av} {fh}");
    assert!(av.contains("redacted") && fh.contains("redacted"));
}

#[test]
fn fixed_error_texts_are_key_free() {
    for e in [
        NewsError::NotConfigured,
        NewsError::RateLimited,
        NewsError::Timeout,
    ] {
        assert!(!shown(&e).contains(KEY));
    }
}

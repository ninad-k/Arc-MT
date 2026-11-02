use std::time::Duration;

use jarvis_news::{Finnhub, NewsError, NewsProvider, NewsQuery};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "test-key-123";

fn provider(server: &MockServer) -> Finnhub {
    Finnhub::new(KEY)
        .unwrap()
        .with_base_url(&server.uri())
        .unwrap()
}

fn news_body() -> serde_json::Value {
    json!([
        {
            "category": "forex", "datetime": 1793367900, "headline": "Euro rallies as ECB turns hawkish",
            "id": 1, "image": "", "related": "", "source": "Wire", "summary": "Policy makers signalled more hikes.",
            "url": "https://example.com/a"
        },
        {
            "category": "forex", "datetime": 1793364300, "headline": "Dollar slips on weak payrolls",
            "id": 2, "image": "", "related": "", "source": "Desk", "summary": "",
            "url": "https://example.com/b"
        },
        { "category": "forex", "datetime": 0, "headline": "Broken item without a time", "url": "https://example.com/c" },
        { "category": "forex", "datetime": 1793364300, "headline": "  ", "url": "https://example.com/d" }
    ])
}

async fn mock_category(server: &MockServer, category: &str, resp: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/api/v1/news"))
        .and(query_param("category", category))
        .respond_with(resp)
        .mount(server)
        .await;
}

#[tokio::test]
async fn sends_key_in_header_only_and_parses_items() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/news"))
        .and(query_param("category", "forex"))
        .and(header("X-Finnhub-Token", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(news_body()))
        .expect(1)
        .mount(&server)
        .await;

    let got = provider(&server)
        .fetch(&NewsQuery::new("EURUSD"))
        .await
        .unwrap();

    // The item without a time and the empty headline are skipped.
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].title, "Euro rallies as ECB turns hawkish");
    assert_eq!(got[0].source, "Wire");
    assert_eq!(got[0].url, "https://example.com/a");
    assert_eq!(got[0].published_at.timestamp(), 1_793_367_900);
    assert_eq!(got[0].tone, 1.0);
    assert!(got[1].tone < 0.0);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let query = requests[0].url.query().unwrap_or("");
    assert!(!query.contains(KEY), "key leaked into the URL: {query}");
    assert_eq!(query, "category=forex");
}

#[tokio::test]
async fn category_follows_the_instrument() {
    for (canonical, category) in [
        ("EURUSD", "forex"),
        ("BTCUSD", "crypto"),
        ("US500", "general"),
        ("USOIL", "general"),
    ] {
        let server = MockServer::start().await;
        mock_category(
            &server,
            category,
            ResponseTemplate::new(200).set_body_json(json!([])),
        )
        .await;
        provider(&server)
            .fetch(&NewsQuery::new(canonical))
            .await
            .unwrap_or_else(|e| panic!("{canonical}: {e}"));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "{canonical}");
    }
}

#[tokio::test]
async fn metals_merge_forex_and_general_without_duplicates() {
    let server = MockServer::start().await;
    mock_category(
        &server,
        "forex",
        ResponseTemplate::new(200).set_body_json(json!([
            {"datetime": 1793367900, "headline": "Gold climbs", "url": "https://example.com/g1", "source": "A"},
            {"datetime": 1793367000, "headline": "Dollar firms", "url": "https://example.com/d1", "source": "A"}
        ])),
    )
    .await;
    mock_category(
        &server,
        "general",
        ResponseTemplate::new(200).set_body_json(json!([
            {"datetime": 1793367900, "headline": "Gold climbs", "url": "https://example.com/g1", "source": "B"},
            {"datetime": 1793366000, "headline": "Bullion demand", "url": "https://example.com/g2", "source": "B"}
        ])),
    )
    .await;

    let got = provider(&server)
        .fetch(&NewsQuery::new("XAUUSD"))
        .await
        .unwrap();
    let urls: Vec<_> = got.iter().map(|h| h.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://example.com/g1",
            "https://example.com/d1",
            "https://example.com/g2"
        ]
    );
}

#[tokio::test]
async fn one_failing_category_still_returns_the_other() {
    let server = MockServer::start().await;
    mock_category(
        &server,
        "forex",
        ResponseTemplate::new(200).set_body_json(news_body()),
    )
    .await;
    mock_category(&server, "general", ResponseTemplate::new(500)).await;
    let got = provider(&server)
        .fetch(&NewsQuery::new("XAUUSD"))
        .await
        .unwrap();
    assert_eq!(got.len(), 2);
}

#[tokio::test]
async fn all_categories_failing_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("XAUUSD"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, NewsError::Http { status: 500, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn status_codes_map_to_errors() {
    for (status, check) in [
        (401u16, "not configured"),
        (429, "rate limit"),
        (500, "HTTP 500"),
        (403, "HTTP 403"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_string("nope"))
            .mount(&server)
            .await;
        let err = provider(&server)
            .fetch(&NewsQuery::new("EURUSD"))
            .await
            .unwrap_err();
        let shown = err.to_string();
        assert!(shown.contains(check), "{status}: {shown}");
        match status {
            401 => assert!(matches!(err, NewsError::NotConfigured)),
            429 => assert!(matches!(err, NewsError::RateLimited)),
            s => assert!(matches!(err, NewsError::Http { status, .. } if status == s)),
        }
    }
}

#[tokio::test]
async fn http_error_keeps_a_short_excerpt() {
    let server = MockServer::start().await;
    let long = format!("upstream exploded\n{}", "x".repeat(1000));
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(502).set_body_string(long))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("EURUSD"))
        .await
        .unwrap_err();
    let NewsError::Http { status, body } = err else {
        panic!("wrong error");
    };
    assert_eq!(status, 502);
    assert!(body.starts_with("upstream exploded x"));
    assert!(body.len() < 260 && !body.contains('\n'));
}

#[tokio::test]
async fn slow_server_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([]))
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&server)
        .await;
    let err = provider(&server)
        .with_timeout(Duration::from_millis(200))
        .fetch(&NewsQuery::new("EURUSD"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::Timeout), "{err:?}");
}

#[tokio::test]
async fn bad_json_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"error\": \"oops\"}"))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("EURUSD"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::Decode(_)), "{err:?}");
}

#[test]
fn construction_rules() {
    assert!(matches!(Finnhub::new("  "), Err(NewsError::NotConfigured)));
    assert!(
        Finnhub::new(KEY)
            .unwrap()
            .with_base_url("http://example.com")
            .is_err()
    );
    assert!(
        Finnhub::new(KEY)
            .unwrap()
            .with_base_url("http://localhost:9999/")
            .is_ok()
    );
    assert_eq!(Finnhub::new(KEY).unwrap().name(), "finnhub",);
}

use std::time::Duration;

use jarvis_news::{AlphaVantage, NewsError, NewsProvider, NewsQuery};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "test-key-123";

fn provider(server: &MockServer) -> AlphaVantage {
    AlphaVantage::new(KEY)
        .unwrap()
        .with_base_url(&server.uri())
        .unwrap()
}

fn item(title: &str, url: &str, time: &str, overall: f64, tickers: Value) -> Value {
    json!({
        "title": title, "url": url, "time_published": time,
        "summary": "A short blurb.", "source": "Wire",
        "overall_sentiment_score": overall, "overall_sentiment_label": "Neutral",
        "ticker_sentiment": tickers
    })
}

fn feed(items: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!({"items": items.len().to_string(), "feed": items}))
}

async fn mock_ticker(server: &MockServer, ticker: &str, resp: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/query"))
        .and(query_param("function", "NEWS_SENTIMENT"))
        .and(query_param("tickers", ticker))
        .and(query_param("limit", "50"))
        .and(query_param("apikey", KEY))
        .respond_with(resp)
        .mount(server)
        .await;
}

#[tokio::test]
async fn pair_makes_one_request_per_currency_and_uses_ticker_scores() {
    let server = MockServer::start().await;
    mock_ticker(
        &server,
        "FOREX:EUR",
        feed(vec![
            item(
                "ECB talks",
                "https://example.com/1",
                "20261030T134500",
                -0.9,
                json!([{"ticker": "FOREX:EUR", "ticker_sentiment_score": "0.42", "relevance_score": "0.9", "ticker_sentiment_label": "Bullish"}]),
            ),
            item(
                "Both sides",
                "https://example.com/2",
                "20261030T120000",
                0.0,
                json!([
                    {"ticker": "FOREX:EUR", "ticker_sentiment_score": "0.5"},
                    {"ticker": "FOREX:USD", "ticker_sentiment_score": "-0.3"}
                ]),
            ),
        ]),
    )
    .await;
    mock_ticker(
        &server,
        "FOREX:USD",
        feed(vec![
            item(
                "Fed talks",
                "https://example.com/3",
                "20261030T110000",
                0.7,
                json!([{"ticker": "FOREX:USD", "ticker_sentiment_score": "0.2"}]),
            ),
            // Seen in both requests: kept once.
            item(
                "Both sides",
                "https://example.com/2",
                "20261030T120000",
                0.0,
                json!([
                    {"ticker": "FOREX:EUR", "ticker_sentiment_score": "0.5"},
                    {"ticker": "FOREX:USD", "ticker_sentiment_score": "-0.3"}
                ]),
            ),
            // No score for the ticker, and no overall score: the word list decides.
            json!({"title": "Dollar slumps", "url": "https://example.com/4",
                   "time_published": "20261030T100000", "summary": "", "source": "Wire"}),
        ]),
    )
    .await;

    let articles = provider(&server)
        .fetch_articles(&NewsQuery::new("EURUSD"))
        .await
        .unwrap();
    assert_eq!(articles.len(), 4);

    // Base ticker score, not the contradicting overall score.
    assert!((articles[0].headline.tone - 0.42).abs() < 1e-12);
    assert_eq!(
        articles[0].headline.published_at.to_rfc3339(),
        "2026-10-30T13:45:00+00:00"
    );
    assert!(articles[0].scoped && articles[0].directed);
    // Both sides present: (base - quote) / 2.
    assert!((articles[1].headline.tone - 0.4).abs() < 1e-12);
    // Quote only: flipped into the pair's direction.
    assert!((articles[2].headline.tone - -0.2).abs() < 1e-12);
    // Lexicon fallback leaves direction to the service.
    assert_eq!(articles[3].headline.tone, -1.0);
    assert!(!articles[3].scoped && !articles[3].directed);

    // Key in the query string only, never as a header; requests go out in order.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for r in &requests {
        assert!(
            r.headers
                .iter()
                .all(|(_, v)| !v.to_str().unwrap_or("").contains(KEY))
        );
        assert!(r.url.query().unwrap().contains("time_from="));
    }
}

#[tokio::test]
async fn crypto_uses_the_coin_ticker_and_overall_score_as_fallback() {
    let server = MockServer::start().await;
    mock_ticker(
        &server,
        "CRYPTO:BTC",
        feed(vec![
            item(
                "Bitcoin news",
                "https://example.com/1",
                "20261030T130000",
                0.1,
                json!([{"ticker": "CRYPTO:BTC", "ticker_sentiment_score": 0.3}]),
            ),
            item(
                "Crypto roundup",
                "https://example.com/2",
                "20261030T130000",
                -0.25,
                json!([{"ticker": "CRYPTO:ETH", "ticker_sentiment_score": "0.9"}]),
            ),
        ]),
    )
    .await;
    let got = provider(&server)
        .fetch(&NewsQuery::new("BTCUSD"))
        .await
        .unwrap();
    assert_eq!(got.len(), 2);
    assert!((got[0].tone - 0.3).abs() < 1e-12);
    assert!((got[1].tone - -0.25).abs() < 1e-12);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn metals_and_indices_use_topics() {
    for (canonical, topic) in [
        ("XAUUSD", "economy_monetary"),
        ("US500", "financial_markets"),
        ("USOIL", "energy_transportation"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/query"))
            .and(query_param("topics", topic))
            .and(query_param("function", "NEWS_SENTIMENT"))
            .and(query_param("limit", "50"))
            .respond_with(feed(vec![item(
                "Markets",
                "https://example.com/1",
                "20261030T130000",
                0.2,
                json!([]),
            )]))
            .expect(1)
            .mount(&server)
            .await;
        let got = provider(&server)
            .fetch_articles(&NewsQuery::new(canonical))
            .await
            .unwrap();
        assert_eq!(got.len(), 1, "{canonical}");
        assert!(!got[0].scoped);
        let q = server.received_requests().await.unwrap()[0]
            .url
            .query()
            .unwrap()
            .to_string();
        assert!(!q.contains("tickers="), "{q}");
    }
}

#[tokio::test]
async fn quota_message_in_a_200_is_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Information": "Thank you for using Alpha Vantage! Our standard API rate limit is 25 requests per day. Please subscribe to any of the premium plans."
        })))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("BTCUSD"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::RateLimited), "{err:?}");
}

#[tokio::test]
async fn note_is_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Note": "Thank you for using Alpha Vantage! Our standard API call frequency is 5 calls per minute."
        })))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("US500"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::RateLimited), "{err:?}");
}

#[tokio::test]
async fn bad_key_message_is_not_configured_and_is_not_echoed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Information": format!("the parameter apikey is invalid or missing: {KEY}. Please claim your free API key.")
        })))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("US500"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::NotConfigured), "{err:?}");
    assert!(!format!("{err} {err:?}").contains(KEY));
}

#[tokio::test]
async fn error_message_does_not_surface_the_provider_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Error Message": format!("Invalid API call for apikey={KEY}")
        })))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("US500"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::Decode(_)), "{err:?}");
    assert!(!format!("{err} {err:?}").contains(KEY));
}

#[tokio::test]
async fn no_articles_notice_is_an_empty_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"Information": "No articles found for the given parameters."}),
            ),
        )
        .mount(&server)
        .await;
    let got = provider(&server)
        .fetch(&NewsQuery::new("US500"))
        .await
        .unwrap();
    assert!(got.is_empty());
}

#[tokio::test]
async fn http_statuses_map_to_errors() {
    for status in [401u16, 429, 500] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string(format!("failed for apikey={KEY}")),
            )
            .mount(&server)
            .await;
        let err = provider(&server)
            .fetch(&NewsQuery::new("US500"))
            .await
            .unwrap_err();
        match status {
            401 => assert!(matches!(err, NewsError::NotConfigured)),
            429 => assert!(matches!(err, NewsError::RateLimited)),
            _ => assert!(
                matches!(err, NewsError::Http { status: 500, .. }),
                "{err:?}"
            ),
        }
        assert!(!format!("{err} {err:?}").contains(KEY), "{status}");
    }
}

#[tokio::test]
async fn second_request_failing_keeps_the_first_half() {
    let server = MockServer::start().await;
    mock_ticker(
        &server,
        "FOREX:EUR",
        feed(vec![item(
            "ECB talks",
            "https://example.com/1",
            "20261030T134500",
            0.2,
            json!([{"ticker": "FOREX:EUR", "ticker_sentiment_score": "0.2"}]),
        )]),
    )
    .await;
    mock_ticker(&server, "FOREX:USD", ResponseTemplate::new(429)).await;
    let got = provider(&server)
        .fetch(&NewsQuery::new("EURUSD"))
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
}

#[tokio::test]
async fn first_request_failing_is_the_error() {
    let server = MockServer::start().await;
    mock_ticker(&server, "FOREX:EUR", ResponseTemplate::new(429)).await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("EURUSD"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::RateLimited));
}

#[tokio::test]
async fn slow_server_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(feed(vec![]).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;
    let err = provider(&server)
        .with_timeout(Duration::from_millis(200))
        .fetch(&NewsQuery::new("US500"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::Timeout), "{err:?}");
}

#[tokio::test]
async fn missing_feed_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"something": "else"})))
        .mount(&server)
        .await;
    let err = provider(&server)
        .fetch(&NewsQuery::new("US500"))
        .await
        .unwrap_err();
    assert!(matches!(err, NewsError::Decode(_)), "{err:?}");
}

#[test]
fn construction_rules() {
    assert!(matches!(
        AlphaVantage::new(""),
        Err(NewsError::NotConfigured)
    ));
    assert!(
        AlphaVantage::new(KEY)
            .unwrap()
            .with_base_url("http://example.com")
            .is_err()
    );
    assert_eq!(AlphaVantage::new(KEY).unwrap().name(), "alphavantage");
}

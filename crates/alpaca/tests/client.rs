use std::time::Duration;

use jarvis_alpaca::{AlpacaClient, AlpacaConfig, AlpacaError, NewOrder};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "PKTESTKEY123";
const SECRET: &str = "s3cr3t-value-xyz";

fn config(trading: &str, data: &str) -> AlpacaConfig {
    AlpacaConfig {
        trading_url: trading.to_owned(),
        data_url: data.to_owned(),
        key_id: KEY.to_owned(),
        secret_key: SECRET.to_owned(),
        feed: "iex".to_owned(),
        timeout: Duration::from_secs(2),
    }
}

async fn paper(server: &MockServer) -> AlpacaClient {
    AlpacaClient::new(&config(&server.uri(), &server.uri()))
        .unwrap()
        .assume_paper_for_tests()
}

#[tokio::test]
async fn reads_send_the_keys_and_parse_string_numbers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/account"))
        .and(header("apca-api-key-id", KEY))
        .and(header("apca-api-secret-key", SECRET))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "a1", "status": "ACTIVE", "currency": "USD", "cash": "98000.5",
            "equity": "100250.75", "last_equity": "100000", "buying_power": "200000"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/positions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "asset_id": "u-1", "symbol": "NVDA", "qty": "5", "side": "long",
              "avg_entry_price": "120.10", "current_price": "121.00", "unrealized_pl": "4.5" }
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/clock"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "timestamp": "2026-11-25T10:00:00-05:00", "is_open": true,
            "next_open": "2026-11-27T09:30:00-05:00", "next_close": "2026-11-25T16:00:00-05:00"
        })))
        .mount(&server)
        .await;
    let c = paper(&server).await;
    let a = c.account().await.unwrap();
    assert_eq!(a.equity, Some(100250.75));
    assert_eq!(a.blocked_reason(), None);
    let p = c.positions().await.unwrap();
    assert_eq!(p[0].qty, Some(5.0));
    assert_eq!(p[0].avg_entry_price, Some(120.1));
    assert!(c.clock().await.unwrap().is_open);
}

#[tokio::test]
async fn quotes_need_both_sides_and_bars_come_oldest_first() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/stocks/NVDA/quotes/latest"))
        .and(query_param("feed", "iex"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbol": "NVDA", "quote": { "bp": 120.5, "ap": 120.6, "t": "2026-11-25T15:00:00Z" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/stocks/AAPL/quotes/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbol": "AAPL", "quote": { "bp": 0, "ap": 230.1, "t": "2026-11-25T15:00:00Z" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/stocks/NVDA/bars"))
        .and(query_param("timeframe", "1Hour"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "bars": [
            { "t": "2026-11-25T15:00:00Z", "o": 121, "h": 122, "l": 120, "c": 121.5, "v": 1000 },
            { "t": "2026-11-25T14:00:00Z", "o": 120, "h": 121, "l": 119, "c": 121, "v": 900 },
            { "t": "bad", "o": 1, "h": 1, "l": 1, "c": 1, "v": 1 }
        ], "symbol": "NVDA", "next_page_token": null })))
        .mount(&server)
        .await;
    let c = paper(&server).await;
    let q = c.latest_quote("NVDA").await.unwrap();
    assert_eq!((q.bid, q.ask), (120.5, 120.6));
    assert!(matches!(
        c.latest_quote("AAPL").await,
        Err(AlpacaError::Decode(_))
    ));
    let bars = c
        .bars("NVDA", "1Hour", "2026-11-20T00:00:00Z", 10)
        .await
        .unwrap();
    assert_eq!(bars.len(), 2);
    assert!(bars[0].time < bars[1].time);
    assert_eq!(bars[1].close, 121.5);
}

#[tokio::test]
async fn a_bracket_order_is_sent_as_documented() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/orders"))
        .and(body_partial_json(json!({
            "symbol": "NVDA", "qty": "5", "side": "buy", "type": "market", "time_in_force": "day",
            "client_order_id": "arc-1-0", "order_class": "bracket",
            "stop_loss": { "stop_price": "118.00" }, "take_profit": { "limit_price": "130.00" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "o-1", "client_order_id": "arc-1-0", "symbol": "NVDA", "qty": "5",
            "filled_qty": "0", "side": "buy", "type": "market", "status": "accepted",
            "order_class": "bracket"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let c = paper(&server).await;
    let o = c
        .submit_order(&NewOrder::new(
            "NVDA",
            5,
            true,
            None,
            Some(118.0),
            Some(130.0),
            "arc-1-0",
        ))
        .await
        .unwrap();
    assert_eq!(o.status, "accepted");
}

#[tokio::test]
async fn writes_are_refused_off_the_paper_host() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    // A loopback double without the test switch counts as live.
    let live = AlpacaClient::new(&config(&server.uri(), &server.uri())).unwrap();
    assert!(!live.is_paper());
    let order = NewOrder::new("NVDA", 1, true, None, None, None, "x");
    assert_eq!(live.submit_order(&order).await, Err(AlpacaError::NotPaper));
    assert_eq!(live.cancel_order("o-1").await, Err(AlpacaError::NotPaper));
    assert_eq!(
        live.close_position("NVDA", None).await,
        Err(AlpacaError::NotPaper)
    );
    // The real live host is never paper, whatever the test switch says.
    let real_live = AlpacaClient::new(&config(
        "https://api.alpaca.markets",
        "https://data.alpaca.markets",
    ))
    .unwrap()
    .assume_paper_for_tests();
    assert!(!real_live.is_paper());
    let real_paper = AlpacaClient::new(&config(
        "https://paper-api.alpaca.markets/v2",
        "https://data.alpaca.markets",
    ))
    .unwrap();
    assert!(real_paper.is_paper());
}

#[tokio::test]
async fn errors_are_typed_and_never_carry_the_keys() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/orders"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "code": 42210000, "message": format!("insufficient qty for {SECRET}")
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/account"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/orders:by_client_order_id"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "not found" })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v2/orders/o-9"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let c = paper(&server).await;
    let err = c
        .submit_order(&NewOrder::new("NVDA", 1, true, None, None, None, "x"))
        .await
        .unwrap_err();
    assert!(matches!(err, AlpacaError::Api { status: 422, .. }));
    assert!(!err.to_string().contains(SECRET), "{err}");
    assert_eq!(c.account().await, Err(AlpacaError::Auth));
    assert_eq!(c.order_by_client_id("nope").await, Ok(None));
    let e = c.cancel_order("o-9").await.unwrap_err();
    assert!(e.maybe_sent(), "a 5xx on a write may have been processed");
    assert!(!format!("{c:?}").contains(SECRET));
}

#[tokio::test]
async fn a_connection_refused_write_was_not_sent() {
    // A port that was just freed: the connection is refused at once.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    // Windows reports a refused loopback connection only after about two seconds of retries; the
    // timeout must be longer, or the refusal arrives as a timeout (which counts as maybe sent).
    let mut cfg = config(&url, &url);
    cfg.timeout = Duration::from_secs(8);
    let c = AlpacaClient::new(&cfg).unwrap().assume_paper_for_tests();
    let e = c
        .submit_order(&NewOrder::new("NVDA", 1, true, None, None, None, "x"))
        .await
        .unwrap_err();
    assert!(matches!(e, AlpacaError::Transport { .. }));
    assert!(!e.maybe_sent(), "{e:?}");
}

#[tokio::test]
async fn settings_are_checked() {
    assert!(matches!(
        AlpacaClient::new(&config("http://example.com", "https://data.alpaca.markets")),
        Err(AlpacaError::Config(_))
    ));
    let mut c = config(
        "https://paper-api.alpaca.markets",
        "https://data.alpaca.markets",
    );
    c.secret_key = " ".into();
    assert!(matches!(AlpacaClient::new(&c), Err(AlpacaError::Config(_))));
}

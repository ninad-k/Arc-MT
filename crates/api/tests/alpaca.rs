//! Alpaca as a second venue (spec-alpaca, ADR-0024): the same gate, preview, token and confirm as MT5,
//! against a wiremock Alpaca. Nothing here reaches the real Alpaca.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use jarvis_alpaca::{AlpacaClient, AlpacaConfig, ticket_for};
use jarvis_bridge_client::mock::MockBridge;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SYMBOLS: &str = "symbols = ['NVDA']";
const NVDA_ASSET: &str = "asset-nvda";

fn client(server: &MockServer, paper: bool) -> AlpacaClient {
    let c = AlpacaClient::new(&AlpacaConfig {
        trading_url: server.uri(),
        data_url: server.uri(),
        key_id: "PKTEST".into(),
        secret_key: "secret-test".into(),
        feed: "iex".into(),
        timeout: Duration::from_secs(3),
    })
    .unwrap();
    if paper { c.assume_paper_for_tests() } else { c }
}

/// The read side of a paper account: market open, NVDA at 120.00/120.10, $100,000 equity.
async fn mount_reads(server: &MockServer, market_open: bool, positions: Value) {
    Mock::given(method("GET"))
        .and(path("/v2/clock"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "timestamp": "2026-11-25T10:00:00-05:00", "is_open": market_open,
            "next_open": "2026-11-27T09:30:00-05:00", "next_close": "2026-11-25T16:00:00-05:00"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/account"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "acct", "status": "ACTIVE", "currency": "USD", "cash": "100000",
            "equity": "100000", "last_equity": "100000", "buying_power": "200000"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/positions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(positions))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/orders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/assets/NVDA"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": NVDA_ASSET, "symbol": "NVDA", "name": "NVIDIA Corp", "exchange": "NASDAQ",
            "class": "us_equity", "status": "active", "tradable": true
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/stocks/NVDA/quotes/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "symbol": "NVDA", "quote": { "bp": 120.0, "ap": 120.1, "t": "2026-11-25T15:00:00Z" }
        })))
        .mount(server)
        .await;
}

fn nvda_position() -> Value {
    json!([{ "asset_id": NVDA_ASSET, "symbol": "NVDA", "qty": "5", "side": "long",
             "avg_entry_price": "119.50", "current_price": "120.00", "unrealized_pl": "2.5" }])
}

async fn app_with(
    mock: &MockBridge,
    alpaca: &MockServer,
    llm: std::sync::Arc<ScriptedLlm>,
) -> TestApp {
    build_app(
        mock,
        llm,
        Options {
            alpaca: SYMBOLS,
            alpaca_client: Some(client(alpaca, true)),
            ..Options::default()
        },
    )
    .await
}

fn order_llm(volume: f64) -> std::sync::Arc<ScriptedLlm> {
    ScriptedLlm::new(
        "fake",
        vec![
            Ok(tool_use_response(
                "p1",
                "propose_order",
                json!({ "symbol": "NVDA", "side": "buy", "volume": volume, "sl": 118.0 }),
            )),
            Ok(text_response("The preview is ready.")),
        ],
    )
}

async fn post(app: &TestApp, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    request(&app.router, "POST", uri, body).await
}

#[tokio::test]
async fn an_alpaca_order_is_previewed_and_only_confirm_sends_it() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, json!([])).await;
    Mock::given(method("POST"))
        .and(path("/v2/orders"))
        .and(body_partial_json(json!({
            "symbol": "NVDA", "qty": "5", "side": "buy", "type": "market", "time_in_force": "day",
            "order_class": "oto", "stop_loss": { "stop_price": "118.00" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "o-1", "client_order_id": "arc-x", "symbol": "NVDA", "qty": "5", "filled_qty": "5",
            "filled_avg_price": "120.10", "side": "buy", "type": "market", "status": "filled",
            "order_class": "oto"
        })))
        .expect(1)
        .mount(&alpaca)
        .await;
    let app = app_with(&mock, &alpaca, order_llm(5.0)).await;
    assert!(app.state.alpaca_off().is_none());

    let (status, body) = chat(&app, "buy 5 NVDA with a stop at 118").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let preview = &body["preview"];
    let token = preview["token"].as_str().expect("a preview").to_owned();
    assert_eq!(preview["symbols"][0], "NVDA");
    assert_eq!(preview["total_volume"], 5.0);
    // Estimated loss at the stop in dollars: 5 shares x (120.10 - 118.00).
    let at_sl = preview["est_pnl"]["at_sl"].as_f64().unwrap();
    assert!((at_sl + 10.5).abs() < 0.01, "{at_sl}");
    assert_eq!(mock.executed_writes(), 0, "nothing reaches MT5");

    let (status, done) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["results"][0]["ok"], true, "{done}");
    // `expect(1)` on the POST is checked when the mock server is dropped.
}

#[tokio::test]
async fn alpaca_limits_market_hours_and_the_kill_switch_hold() {
    // Over 100 shares, then over $10,000.
    for (volume, why) in [(101.0, "max_lot"), (90.0, "max_notional_usd")] {
        let mock = MockBridge::start().await;
        let alpaca = MockServer::start().await;
        mount_reads(&alpaca, true, json!([])).await;
        let app = app_with(&mock, &alpaca, order_llm(volume)).await;
        let (_, body) = chat(&app, &format!("buy {volume} NVDA with a stop at 118")).await;
        assert!(body["preview"].is_null(), "{why}: {body}");
        assert_eq!(body["tool_calls"][0]["ok"], false, "{why}");
    }

    // Market closed.
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, false, json!([])).await;
    let app = app_with(&mock, &alpaca, order_llm(5.0)).await;
    let (_, body) = chat(&app, "buy 5 NVDA with a stop at 118").await;
    assert!(body["preview"].is_null(), "market closed: {body}");

    // Kill switch between preview and confirm: the Alpaca token is revoked, nothing is sent.
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, json!([])).await;
    Mock::given(method("POST"))
        .and(path("/v2/orders"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&alpaca)
        .await;
    let app = app_with(&mock, &alpaca, order_llm(5.0)).await;
    let (_, body) = chat(&app, "buy 5 NVDA with a stop at 118").await;
    let token = body["preview"]["token"]
        .as_str()
        .expect("a preview")
        .to_owned();
    let (status, _) = post(
        &app,
        "/api/killswitch",
        Some(json!({ "engaged": true, "reason": "test" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, refused) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert!(status.is_client_error(), "{status} {refused}");
}

#[tokio::test]
async fn a_live_alpaca_url_turns_alpaca_off_and_nothing_is_sent() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&alpaca)
        .await;
    let app = build_app(
        &mock,
        order_llm(5.0),
        Options {
            alpaca: SYMBOLS,
            // Not marked paper: a loopback double counts as live, like any non-paper host.
            alpaca_client: Some(client(&alpaca, false)),
            ..Options::default()
        },
    )
    .await;
    let why = app.state.alpaca_off().expect("Alpaca must be off");
    assert!(why.contains("paper"), "{why}");
    assert!(app.state.router().alpaca().is_none());
    let (_, body) = chat(&app, "buy 5 NVDA with a stop at 118").await;
    assert!(body["preview"].is_null(), "{body}");
}

#[tokio::test]
async fn an_alpaca_position_is_closed_through_a_preview() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, nvda_position()).await;
    Mock::given(method("DELETE"))
        .and(path("/v2/positions/NVDA"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "o-close", "client_order_id": "c", "symbol": "NVDA", "qty": "5", "side": "sell",
            "type": "market", "status": "accepted"
        })))
        .expect(1)
        .mount(&alpaca)
        .await;
    let app = app_with(&mock, &alpaca, ScriptedLlm::new("fake", vec![])).await;
    let ticket = ticket_for(NVDA_ASSET);
    let (status, preview) = post(
        &app,
        &format!("/api/positions/{ticket}/close"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["tickets"][0], ticket);
    let token = preview["token"].as_str().unwrap().to_owned();
    let (status, done) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["results"][0]["ok"], true, "{done}");
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn an_unclear_alpaca_submit_is_never_sent_twice() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, json!([])).await;
    // Alpaca fails after the request may have arrived, and the order cannot be found by its id.
    Mock::given(method("POST"))
        .and(path("/v2/orders"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&alpaca)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/orders:by_client_order_id"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "not found" })))
        .mount(&alpaca)
        .await;
    let app = app_with(&mock, &alpaca, order_llm(5.0)).await;
    let (_, body) = chat(&app, "buy 5 NVDA with a stop at 118").await;
    let token = body["preview"]["token"]
        .as_str()
        .expect("a preview")
        .to_owned();
    let (status, done) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{done}");
    assert_eq!(done["error"]["code"], "outcome_unknown");
    let message = done["error"]["message"].as_str().unwrap();
    assert!(message.contains("Alpaca paper account"), "{message}");
    // A second confirm of the same token is refused: the outcome stays unknown, never resent.
    let (status, _) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert!(status.is_client_error());
}

#[tokio::test]
async fn moving_a_stop_on_alpaca_is_refused_at_preview() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, nvda_position()).await;
    let ticket = ticket_for(NVDA_ASSET);
    let llm = ScriptedLlm::new(
        "fake",
        vec![
            Ok(tool_use_response(
                "m1",
                "propose_modify",
                json!({ "ticket": ticket, "sl": 117.0 }),
            )),
            Ok(text_response("That is not possible on Alpaca.")),
        ],
    );
    let app = app_with(&mock, &alpaca, llm).await;
    let (_, body) = chat(&app, &format!("move the stop on {ticket} to 117")).await;
    assert!(body["preview"].is_null(), "{body}");
    assert_eq!(body["tool_calls"][0]["ok"], false);
}

async fn get(app: &TestApp, uri: &str) -> (StatusCode, Value) {
    request(&app.router, "GET", uri, None).await
}

#[tokio::test]
async fn reads_merge_both_venues_with_a_venue_tag() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, nvda_position()).await;
    let app = app_with(&mock, &alpaca, ScriptedLlm::new("fake", vec![])).await;

    let (status, body) = get(&app, "/api/positions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["positions"].as_array().unwrap();
    let alpaca_rows: Vec<&Value> = rows.iter().filter(|r| r["venue"] == "alpaca").collect();
    assert_eq!(alpaca_rows.len(), 1);
    assert_eq!(alpaca_rows[0]["symbol"], "NVDA");
    assert_eq!(alpaca_rows[0]["ticket"], ticket_for(NVDA_ASSET));
    assert!(
        rows.iter().filter(|r| r["venue"] == "mt5").count() >= 1,
        "MT5 rows are kept"
    );

    let (_, account) = get(&app, "/api/alpaca/account").await;
    assert_eq!(account["enabled"], true);
    assert_eq!(account["equity"], 100000.0);
    assert_eq!(account["market_open"], true);

    let (_, status_body) = get(&app, "/api/status").await;
    assert_eq!(status_body["alpaca"]["connected"], true);
    assert_eq!(status_body["alpaca"]["paper"], true);
    assert_eq!(status_body["alpaca"]["symbols"][0], "NVDA");
}

#[tokio::test]
async fn without_alpaca_the_reads_are_unchanged() {
    let mock = MockBridge::start().await;
    let app = build_app(&mock, ScriptedLlm::new("fake", vec![]), Options::default()).await;
    let (_, body) = get(&app, "/api/positions").await;
    assert!(body["positions"][0].get("venue").is_none(), "{body}");
    let (_, account) = get(&app, "/api/alpaca/account").await;
    assert_eq!(account["enabled"], false);
    assert!(
        account["reason"]
            .as_str()
            .unwrap()
            .contains("keys are not set")
    );
    let (_, s) = get(&app, "/api/status").await;
    assert_eq!(s["alpaca"]["enabled"], false);
}

#[tokio::test]
async fn the_price_tool_asks_alpaca_for_an_alpaca_symbol() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, json!([])).await;
    let llm = ScriptedLlm::new(
        "fake",
        vec![
            Ok(tool_use_response(
                "q1",
                "get_price",
                json!({ "symbol": "NVDA" }),
            )),
            Ok(text_response("NVDA is 120.00 bid, 120.10 ask on Alpaca.")),
        ],
    );
    let app = app_with(&mock, &alpaca, llm).await;
    let (status, body) = chat(&app, "what's NVDA at?").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tool_calls"][0]["ok"], true, "{body}");
    // The figures came from Alpaca's quote, so the grounding check let them through.
    assert!(body["reply"].as_str().unwrap().contains("120.10"), "{body}");
}

#[tokio::test]
async fn desk_sizes_an_alpaca_plan_from_alpaca() {
    let mock = MockBridge::start().await;
    let alpaca = MockServer::start().await;
    mount_reads(&alpaca, true, json!([])).await;
    let app = app_with(&mock, &alpaca, ScriptedLlm::new("fake", vec![])).await;
    let desk = app.state.desk().unwrap();
    let r = desk
        .review_plan(
            "NVDA",
            "NVDA",
            jarvis_desk::sentinel::Side::Buy,
            Some(118.0),
            None,
            None,
        )
        .await
        .unwrap();
    // Entry at Alpaca's ask; 0.5% of $100,000 over a $2.10 stop is 238 whole shares.
    assert_eq!(r.plan.entry, 120.1);
    assert_eq!(r.plan.volume, 238.0);
    assert_eq!(r.plan.broker_symbol, "NVDA");
}

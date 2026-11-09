//! Market views end to end against `MockBridge`: the view, calendar, news, journal and
//! prepare routes, the evaluator, the three model tools and the grounding of what the model
//! repeats from them.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use common::*;
use jarvis_api::market::{self, MarketViewDto};
use jarvis_api::market_journal::JournalEntry;
use jarvis_api::routes::ApiError;
use jarvis_bridge_client::mock::{MockBridge, MockState};
use jarvis_bridge_client::{BridgeError, protocol};
use jarvis_gate::intent::Intent;
use jarvis_llm::{ChatRequest, ChatResponse, LlmError, LlmProvider, Message};
use jarvis_news::{Finnhub, NewsService};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ------------------------------------------------------------------ helpers

async fn get(app: &TestApp, uri: &str) -> (StatusCode, Value) {
    request(&app.router, "GET", uri, None).await
}

async fn post(app: &TestApp, uri: &str, body: Value) -> (StatusCode, Value) {
    request(&app.router, "POST", uri, Some(body)).await
}

fn now_utc() -> i64 {
    Utc::now().timestamp()
}

fn parse_iso(text: &str) -> i64 {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text} is not ISO 8601: {e}"))
        .timestamp()
}

/// A mock whose server clock is UTC+3, with the calendar and bars on that clock.
fn utc_plus_3() -> MockState {
    let mut state = MockState::default();
    let server = now_utc() + 3 * 3600;
    state.ping.server_time = server;
    state.clock = Some(server);
    state
}

/// Names of the CFI-like catalog the mock serves (`XAUUSD_`, `US100_Spot`, ...).
fn cfi_like() -> MockState {
    let mut state = MockState::default();
    let map: HashMap<&str, &str> = [
        ("XAUUSD", "XAUUSD_"),
        ("US100", "US100_Spot"),
        ("US30", "US30_SPOT"),
        ("EURUSD", "EURUSD_"),
        ("GBPUSD", "GBPUSD_"),
    ]
    .into_iter()
    .collect();
    let swap = |s: &mut String| {
        if let Some(to) = map.get(s.as_str()) {
            *s = (*to).to_owned();
        }
    };
    state.prices.iter_mut().for_each(|p| swap(&mut p.symbol));
    state.symbols.iter_mut().for_each(|p| swap(&mut p.symbol));
    state.positions.iter_mut().for_each(|p| swap(&mut p.symbol));
    state.orders.iter_mut().for_each(|p| swap(&mut p.symbol));
    state.deals.iter_mut().for_each(|p| swap(&mut p.symbol));
    state
}

async fn app_on(mock: &MockBridge, opts: Options<'_>) -> TestApp {
    build_app(mock, ScriptedLlm::new("fake", vec![]), opts).await
}

/// A Finnhub-backed news service pointed at a wiremock server.
fn finnhub_at(server: &MockServer) -> NewsService {
    NewsService::new(Box::new(
        Finnhub::new("test-key-123")
            .unwrap()
            .with_base_url(&server.uri())
            .unwrap(),
    ))
    .with_ttl(Duration::from_secs(60))
}

async fn finnhub_serving_gold_news() -> MockServer {
    let server = MockServer::start().await;
    let at = now_utc() - 600;
    Mock::given(method("GET"))
        .and(path("/api/v1/news"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "category": "forex", "datetime": at, "headline": "Gold climbs as the dollar slips on weak payrolls",
                "id": 1, "source": "Wire", "summary": "Bullion rallied.", "url": "https://example.com/gold-a"
            },
            {
                "category": "general", "datetime": at - 60, "headline": "Gold steadies ahead of the Fed decision",
                "id": 2, "source": "Desk", "summary": "", "url": "https://example.com/gold-b"
            },
            {
                "category": "general", "datetime": at - 120, "headline": "Oil output rises",
                "id": 3, "source": "Desk", "summary": "", "url": "https://example.com/oil"
            }
        ])))
        .mount(&server)
        .await;
    server
}

/// A view as the API stores it; `setup` is `(side, entry_low, entry_high, stop, tp)`.
fn synthetic(
    id: &str,
    bias: &str,
    setup: Option<(&str, f64, f64, f64, f64)>,
    created_server: i64,
    valid_server: i64,
) -> MarketViewDto {
    let setup = setup.map(|(side, low, high, stop, tp)| {
        json!({
            "side": side, "entry_low": low, "entry_high": high, "stop_loss": stop,
            "take_profit": tp, "take_profit_2": tp, "risk_reward": 1.5,
            "atr_h1": 4.0, "atr_m15": 1.5
        })
    });
    serde_json::from_value(json!({
        "id": id, "symbol": "XAUUSD", "broker_symbol": "XAUUSD", "digits": 2,
        "generated_at": market::iso(created_server, 0),
        "valid_until": market::iso(valid_server, 0),
        "price": { "bid": 2391.12, "ask": 2391.40 },
        "bias": bias, "confidence": 60, "score": 60.0,
        "stand_aside": false, "stand_aside_reason": null,
        "setup": setup,
        "timeframes": [{ "timeframe": "H1", "trend": "up", "ema20": 1.0, "ema50": 1.0, "rsi": 50.0, "macd_hist": 0.1, "atr": 4.0 }],
        "levels": { "pivot": null, "r1": null, "r2": null, "s1": null, "s2": null,
                    "prev_day_high": null, "prev_day_low": null, "vwap": null,
                    "support": [], "resistance": [] },
        "factors": [], "events": [],
        "news": { "configured": false, "score": 0.0, "label": "not configured", "count": 0, "headlines": [] },
        "summary": "A stored view.", "disclaimer": "Not advice.", "notes": []
    }))
    .expect("a valid view")
}

async fn store(app: &TestApp, view: &MarketViewDto, created_server: i64, valid_server: i64) {
    app.state
        .market()
        .expect("market is on")
        .journal()
        .insert(&JournalEntry {
            view,
            created_server,
            valid_server,
        })
        .await
        .expect("stored");
}

fn keys(v: &Value) -> BTreeSet<&str> {
    v.as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect()
}

// ------------------------------------------------------------------ the view

#[tokio::test]
async fn the_view_has_the_documented_shape_and_resolves_the_broker_symbol() {
    let mock = MockBridge::start_with(cfi_like()).await;
    let app = app_on(
        &mock,
        Options {
            maps: true,
            ..Options::default()
        },
    )
    .await;
    app.state.startup_check().await.unwrap();

    let (status, view) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        keys(&view),
        BTreeSet::from([
            "id",
            "symbol",
            "broker_symbol",
            "digits",
            "generated_at",
            "valid_until",
            "price",
            "bias",
            "confidence",
            "score",
            "stand_aside",
            "stand_aside_reason",
            "setup",
            "timeframes",
            "levels",
            "factors",
            "events",
            "news",
            "summary",
            "disclaimer",
            "notes",
        ])
    );
    assert_eq!(view["symbol"], "XAUUSD");
    assert_eq!(view["broker_symbol"], "XAUUSD_");
    assert_eq!(view["digits"], 2);
    assert!(view["id"].as_str().is_some_and(|id| id.len() == 36));
    assert!(["bullish", "bearish", "neutral"].contains(&view["bias"].as_str().unwrap()));
    assert!(view["confidence"].as_u64().unwrap() <= 100);
    assert!(view["price"]["bid"].as_f64().unwrap() > 0.0);
    assert!(
        view["disclaimer"]
            .as_str()
            .unwrap()
            .contains("not financial advice")
    );

    // Times are ISO 8601 UTC; the view is valid for the configured four hours.
    let generated = parse_iso(view["generated_at"].as_str().unwrap());
    let valid = parse_iso(view["valid_until"].as_str().unwrap());
    assert!((generated - now_utc()).abs() < 30);
    assert_eq!(valid - generated, 240 * 60);
    assert!(view["generated_at"].as_str().unwrap().ends_with('Z'));

    // No news provider: the section says so.
    assert_eq!(view["news"]["configured"], false);
    assert_eq!(view["news"]["label"], "not configured");
    assert_eq!(view["news"]["count"], 0);
    assert_eq!(view["news"]["headlines"], json!([]));

    // Frames, levels and factors are there; the factors add up to the score.
    let frames: Vec<&str> = view["timeframes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["timeframe"].as_str().unwrap())
        .collect();
    assert!(
        frames.contains(&"H4") && frames.contains(&"H1"),
        "{frames:?}"
    );
    let sum: f64 = view["factors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["points"].as_f64().unwrap())
        .sum();
    let score = view["score"].as_f64().unwrap();
    if score.abs() < 100.0 {
        assert!((sum - score).abs() < 0.1, "factors {sum} vs score {score}");
    }
    assert!(!view["factors"].as_array().unwrap().is_empty());

    // A setup never comes with stand-aside, and has stop and targets on the right side.
    if view["setup"].is_object() {
        assert_eq!(view["stand_aside"], false);
        let s = &view["setup"];
        let (stop, tp) = (
            s["stop_loss"].as_f64().unwrap(),
            s["take_profit"].as_f64().unwrap(),
        );
        if s["side"] == "buy" {
            assert!(
                stop < s["entry_low"].as_f64().unwrap() && tp > s["entry_high"].as_f64().unwrap()
            );
        } else {
            assert!(
                stop > s["entry_high"].as_f64().unwrap() && tp < s["entry_low"].as_f64().unwrap()
            );
        }
    } else {
        assert!(view["setup"].is_null());
    }
    if view["stand_aside"] == true {
        assert!(view["stand_aside_reason"].is_string());
        assert!(view["setup"].is_null());
    }
}

#[tokio::test]
async fn the_symbol_may_be_a_word_a_canonical_name_or_a_broker_name() {
    let mock = MockBridge::start_with(cfi_like()).await;
    let app = app_on(
        &mock,
        Options {
            maps: true,
            ..Options::default()
        },
    )
    .await;
    app.state.startup_check().await.unwrap();
    for input in ["gold", "Gold", "XAUUSD", "XAUUSD_", "  gold  "] {
        let (status, view) = get(
            &app,
            &format!("/api/market/view?symbol={}", input.replace(' ', "%20")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{input}: {view}");
        assert_eq!(view["symbol"], "XAUUSD", "{input}");
        assert_eq!(view["broker_symbol"], "XAUUSD_", "{input}");
    }
    let (status, body) = get(&app, "/api/market/view").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "bad_request");
    let (status, _) = get(&app, "/api/market/view?symbol=%20").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = get(&app, "/api/market/view?symbol=notasymbol").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
}

#[tokio::test]
async fn times_follow_the_server_clock_offset() {
    let mock = MockBridge::start_with(utc_plus_3()).await;
    let app = app_on(&mock, Options::default()).await;
    let (status, view) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(app.state.clock().offset_secs(), 3 * 3600);
    let generated = parse_iso(view["generated_at"].as_str().unwrap());
    assert!(
        (generated - now_utc()).abs() < 30,
        "generated_at must be UTC, not server time: {generated} vs {}",
        now_utc()
    );
    let valid = parse_iso(view["valid_until"].as_str().unwrap());
    assert_eq!(valid - generated, 240 * 60);
    for e in view["events"].as_array().unwrap() {
        let at = parse_iso(e["time"].as_str().unwrap());
        let minutes = e["minutes_away"].as_i64().unwrap();
        assert!(
            ((at - now_utc()) / 60 - minutes).abs() <= 1,
            "{e} is not {minutes} minutes from now"
        );
    }
}

#[tokio::test]
async fn a_view_is_journaled_and_audited() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let (_, first) = get(&app, "/api/market/view?symbol=gold").await;
    let (_, second) = get(&app, "/api/market/view?symbol=eurusd").await;
    assert_ne!(first["id"], second["id"]);

    let (status, journal) = get(&app, "/api/market/journal").await;
    assert_eq!(status, StatusCode::OK, "{journal}");
    assert_eq!(journal["stats"]["total"], 2);
    assert_eq!(journal["stats"]["open"], 2);
    let views = journal["views"].as_array().unwrap();
    assert_eq!(views.len(), 2);
    // Newest first.
    assert_eq!(views[0]["id"], second["id"]);
    assert_eq!(views[1]["id"], first["id"]);
    let row = &views[1];
    assert_eq!(
        keys(row),
        BTreeSet::from([
            "id",
            "created_at",
            "symbol",
            "broker_symbol",
            "bias",
            "confidence",
            "score",
            "stand_aside",
            "side",
            "entry",
            "stop",
            "tp",
            "price",
            "valid_until",
            "outcome",
            "evaluated_at",
            "bias_correct",
            "move_atr",
        ])
    );
    assert_eq!(row["symbol"], "XAUUSD");
    assert_eq!(row["outcome"], "open");
    assert_eq!(row["bias"], first["bias"]);
    assert_eq!(row["confidence"], first["confidence"]);
    assert_eq!(row["valid_until"], first["valid_until"]);
    assert_eq!(row["created_at"], first["generated_at"]);
    assert!(row["evaluated_at"].is_null() && row["bias_correct"].is_null());
    let mid =
        (first["price"]["bid"].as_f64().unwrap() + first["price"]["ask"].as_f64().unwrap()) / 2.0;
    assert!((row["price"].as_f64().unwrap() - mid).abs() < 1e-9);
    match first["setup"].as_object() {
        Some(setup) => {
            assert_eq!(row["side"], setup["side"]);
            let middle = (setup["entry_low"].as_f64().unwrap()
                + setup["entry_high"].as_f64().unwrap())
                / 2.0;
            assert!((row["entry"].as_f64().unwrap() - middle).abs() < 1e-9);
            assert_eq!(row["stop"], setup["stop_loss"]);
            assert_eq!(row["tp"], setup["take_profit"]);
        }
        None => assert!(row["side"].is_null() && row["entry"].is_null() && row["tp"].is_null()),
    }

    // The symbol filter takes the user's word, and stats follow it.
    let (_, filtered) = get(&app, "/api/market/journal?symbol=gold&limit=5").await;
    assert_eq!(filtered["views"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["stats"]["total"], 1);
    let (status, _) = get(&app, "/api/market/journal?symbol=%20%20").await;
    assert_eq!(status, StatusCode::OK);
    let (_, none) = get(&app, "/api/market/journal?symbol=us100").await;
    assert_eq!(none["views"], json!([]));
    assert_eq!(none["stats"]["total"], 0);
    assert!(none["stats"]["tp_rate"].is_null() && none["stats"]["bias_accuracy"].is_null());
    let (_, limited) = get(&app, "/api/market/journal?limit=1").await;
    assert_eq!(limited["views"].as_array().unwrap().len(), 1);

    // One audit row per request, with what the view said.
    let rows: Vec<_> = app
        .audit
        .rows(None)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == "market_view")
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].payload["symbol"], "XAUUSD");
    assert_eq!(rows[0].payload["bias"], first["bias"]);
    assert_eq!(rows[0].payload["confidence"], first["confidence"]);
    assert_eq!(rows[0].payload["stand_aside"], first["stand_aside"]);
    assert_eq!(rows[0].payload["view_id"], first["id"]);
    assert_eq!(rows[0].payload["source"], "ui");
    assert_eq!(rows[0].persona.as_deref(), Some("jarvis"));
}

#[tokio::test]
async fn history_that_is_not_loaded_yet_is_retried_once() {
    let mock = MockBridge::start().await;
    mock.history_not_ready(1);
    let app = app_on(&mock, Options::default()).await;
    let (status, view) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(status, StatusCode::OK, "{view}");
}

#[tokio::test]
async fn a_bridge_that_is_down_is_503() {
    let app = build_app(
        DEAD_ENDPOINT,
        ScriptedLlm::new("fake", vec![]),
        Options::default(),
    )
    .await;
    let (status, body) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"]["code"], "bridge_unavailable");
}

#[tokio::test]
async fn an_ea_that_does_not_know_the_command_gets_the_update_message() {
    let old = BridgeError::Remote {
        message: "unknown command: get_rates".to_owned(),
        retcode: protocol::RETCODE_UNKNOWN_COMMAND,
    };
    let error = ApiError::from(market::MarketError::from(old));
    let response = axum::response::IntoResponse::into_response(error);
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "ea_outdated");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("0.5.0") && message.contains("Update"),
        "{message}"
    );
}

// ------------------------------------------------------------------ news

#[tokio::test]
async fn headlines_appear_when_a_provider_is_configured() {
    let server = finnhub_serving_gold_news().await;
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            news: Some(finnhub_at(&server)),
            ..Options::default()
        },
    )
    .await;

    let (_, status) = get(&app, "/api/status").await;
    assert_eq!(
        status["market"],
        json!({ "enabled": true, "news_provider": "finnhub", "news_configured": true })
    );

    let (code, view) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(code, StatusCode::OK, "{view}");
    let news = &view["news"];
    assert_eq!(news["configured"], true);
    assert_eq!(news["count"], 2, "{news}");
    let headlines = news["headlines"].as_array().unwrap();
    assert_eq!(headlines.len(), 2);
    assert!(
        headlines
            .iter()
            .any(|h| h["title"].as_str().unwrap().starts_with("Gold climbs"))
    );
    for h in headlines {
        assert!(
            h["url"]
                .as_str()
                .unwrap()
                .starts_with("https://example.com/")
        );
        assert!(h["source"].is_string() && h["tone"].is_number());
        assert!(parse_iso(h["published_at"].as_str().unwrap()) > 0);
    }
    assert!(news["score"].as_f64().unwrap().abs() <= 1.0, "{news}");
    // The news factor is part of the score.
    assert!(
        view["factors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "News")
    );

    let (code, report) = get(&app, "/api/market/news?symbol=gold").await;
    assert_eq!(code, StatusCode::OK, "{report}");
    assert_eq!(report["configured"], true);
    assert_eq!(report["provider"], "finnhub");
    assert_eq!(report["summary"]["count"], 2);
    assert_eq!(report["summary"]["headlines"].as_array().unwrap().len(), 2);
    // The key never reaches the client.
    assert!(!view.to_string().contains("test-key-123"));
    assert!(!report.to_string().contains("test-key-123"));
}

#[tokio::test]
async fn without_a_provider_the_news_route_says_not_configured() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let (code, report) = get(&app, "/api/market/news?symbol=gold").await;
    assert_eq!(code, StatusCode::OK, "{report}");
    assert_eq!(
        report,
        json!({ "configured": false, "provider": null, "summary": null })
    );
    let (_, status) = get(&app, "/api/status").await;
    assert_eq!(
        status["market"],
        json!({ "enabled": true, "news_provider": "none", "news_configured": false })
    );
    let (code, _) = get(&app, "/api/market/news").await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_failing_news_provider_does_not_fail_the_view() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/news"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            news: Some(finnhub_at(&server)),
            ..Options::default()
        },
    )
    .await;
    let (code, view) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(code, StatusCode::OK, "{view}");
    assert_eq!(view["news"]["configured"], true);
    assert_eq!(view["news"]["label"], "unavailable");
    assert_eq!(view["news"]["count"], 0);
    assert!(
        view["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("News was unavailable")),
        "{}",
        view["notes"]
    );
    // The technical factors are all still there.
    assert!(view["factors"].as_array().unwrap().len() >= 3);

    let (code, report) = get(&app, "/api/market/news?symbol=gold").await;
    assert_eq!(code, StatusCode::OK, "{report}");
    assert_eq!(report["configured"], true);
    assert!(report["summary"].is_null());
    assert!(
        report["error"].as_str().unwrap().contains("500"),
        "{report}"
    );
}

#[tokio::test]
async fn a_slow_news_provider_cannot_stall_the_view() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/news"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([]))
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            news: Some(finnhub_at(&server)),
            ..Options::default()
        },
    )
    .await;
    let started = std::time::Instant::now();
    let (code, view) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(code, StatusCode::OK, "{view}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(view["news"]["label"], "unavailable");
}

// ------------------------------------------------------------------ calendar

#[tokio::test]
async fn the_calendar_filters_and_converts_times() {
    let mock = MockBridge::start_with(utc_plus_3()).await;
    let app = app_on(&mock, Options::default()).await;

    let (status, body) = get(&app, "/api/market/calendar?hours=48&currencies=usd").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let events = body["events"].as_array().unwrap();
    assert!(!events.is_empty(), "{body}");
    assert_eq!(app.state.clock().offset_secs(), 3 * 3600);
    for e in events {
        assert_eq!(e["currency"], "USD", "{e}");
        assert!(
            ["moderate", "high"].contains(&e["importance"].as_str().unwrap()),
            "{e}"
        );
        assert!(e["title"].is_string() && e["country"].is_string());
        for field in ["actual", "forecast", "previous", "unit"] {
            assert!(e.get(field).is_some(), "{field} missing in {e}");
        }
        let at = parse_iso(e["time"].as_str().unwrap());
        assert!(e["time"].as_str().unwrap().ends_with('Z'));
        let minutes = e["minutes_away"].as_i64().unwrap();
        assert!(
            ((at - now_utc()) / 60 - minutes).abs() <= 1,
            "{e}: UTC time and minutes_away disagree"
        );
        assert!((-61..=48 * 60).contains(&minutes), "{e}");
    }
    // Sorted by time.
    let times: Vec<i64> = events
        .iter()
        .map(|e| parse_iso(e["time"].as_str().unwrap()))
        .collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]));

    // A shorter window is a subset; other currencies come in when asked for.
    let (_, short) = get(&app, "/api/market/calendar?hours=1&currencies=USD").await;
    let short = short["events"].as_array().unwrap();
    assert!(short.len() <= events.len());
    assert!(
        short
            .iter()
            .all(|e| e["minutes_away"].as_i64().unwrap() <= 61)
    );
    let (_, both) = get(&app, "/api/market/calendar?hours=48&currencies=USD,EUR").await;
    let currencies: BTreeSet<&str> = both["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["currency"].as_str().unwrap())
        .collect();
    assert!(
        currencies.contains("EUR") && currencies.contains("USD"),
        "{currencies:?}"
    );
    // No currency filter: everything.
    let (_, all) = get(&app, "/api/market/calendar").await;
    assert!(all["events"].as_array().unwrap().len() >= short.len());

    for bad in [
        "hours=0",
        "hours=169",
        "hours=abc",
        "currencies=US",
        "currencies=USD,dollar",
    ] {
        let (status, body) = get(&app, &format!("/api/market/calendar?{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
}

#[tokio::test]
async fn the_configured_importance_limits_the_calendar() {
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            market: "calendar_min_importance = \"high\"",
            ..Options::default()
        },
    )
    .await;
    let (_, body) = get(&app, "/api/market/calendar?hours=168").await;
    let events = body["events"].as_array().unwrap();
    assert!(!events.is_empty());
    assert!(events.iter().all(|e| e["importance"] == "high"), "{body}");
}

// ------------------------------------------------------------------ evaluator

#[tokio::test]
async fn open_views_are_scored_against_the_m5_bars() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let now = now_utc();
    let h = 3600;

    // Bars around 2391 (the mock's gold price); the forming bar closes at the bid.
    let target = synthetic(
        "tp",
        "bullish",
        Some(("buy", 2390.0, 2391.4, 1.0, 2390.0)),
        now - 2 * h,
        now + 2 * h,
    );
    let stopped = synthetic(
        "sl",
        "bullish",
        Some(("buy", 2390.0, 2391.4, 2400.0, 99_999.0)),
        now - 2 * h,
        now + 2 * h,
    );
    let waiting = synthetic(
        "open",
        "bullish",
        Some(("buy", 2390.0, 2391.4, 1.0, 99_999.0)),
        now - h,
        now + 3 * h,
    );
    let lapsed = synthetic(
        "expired",
        "bullish",
        Some(("buy", 2390.0, 2391.4, 1.0, 99_999.0)),
        now - 3 * h,
        now - h,
    );
    let idle = synthetic("idle", "neutral", None, now - 3 * h, now - h);
    let future = synthetic("later", "neutral", None, now - 600, now + 4 * h);
    for (view, created, valid) in [
        (&target, now - 2 * h, now + 2 * h),
        (&stopped, now - 2 * h, now + 2 * h),
        (&waiting, now - h, now + 3 * h),
        (&lapsed, now - 3 * h, now - h),
        (&idle, now - 3 * h, now - h),
        (&future, now - 600, now + 4 * h),
    ] {
        store(&app, view, created, valid).await;
    }

    let report = app.state.market().unwrap().evaluate_at(now).await;
    assert_eq!(report.checked, 6, "{report:?}");
    assert_eq!(report.failed_symbols, 0);

    let (_, journal) = get(&app, "/api/market/journal?limit=50").await;
    let outcome = |id: &str| -> &Value {
        &journal["views"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id)
            .unwrap_or_else(|| panic!("{id} missing"))["outcome"]
    };
    assert_eq!(outcome("tp"), "tp_hit");
    assert_eq!(outcome("sl"), "sl_hit");
    assert_eq!(outcome("open"), "open");
    assert_eq!(outcome("expired"), "expired");
    assert_eq!(outcome("idle"), "no_setup");
    assert_eq!(outcome("later"), "open");

    let row = |id: &str| -> &Value {
        journal["views"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id)
            .unwrap()
    };
    // Past their expiry the bias is judged, with the move in ATRs; before it, not.
    for id in ["expired", "idle"] {
        assert!(row(id)["bias_correct"].is_boolean(), "{id}: {}", row(id));
        assert!(row(id)["move_atr"].is_number(), "{id}");
        assert!(row(id)["evaluated_at"].is_string(), "{id}");
    }
    for id in ["tp", "sl", "open", "later"] {
        assert!(row(id)["bias_correct"].is_null(), "{id}");
    }
    assert!(row("tp")["evaluated_at"].is_string());
    assert!(row("open")["evaluated_at"].is_null());

    let stats = &journal["stats"];
    assert_eq!(stats["total"], 6);
    assert_eq!(stats["setups"], 4);
    assert_eq!(stats["open"], 2);
    assert_eq!(stats["tp_hit"], 1);
    assert_eq!(stats["sl_hit"], 1);
    assert_eq!(stats["expired"], 1);
    assert_eq!(stats["no_setup"], 1);
    // One target hit among three closed setups.
    assert_eq!(stats["tp_rate"], 0.333);
    let scored = stats["bias_correct"].as_u64().unwrap() + stats["bias_wrong"].as_u64().unwrap();
    assert_eq!(scored, 2);
    assert!(stats["bias_accuracy"].is_number());

    // Scored views are final: a second run changes nothing, and a view whose target was hit
    // is not looked at for its outcome again.
    let again = app.state.market().unwrap().evaluate_at(now).await;
    assert_eq!(again.updated, 0, "{again:?}");
    // The two resolved-but-not-yet-expired views are still waiting for their bias.
    assert_eq!(again.checked, 4, "{again:?}");
    let later = app.state.market().unwrap().evaluate_at(now + 5 * h).await;
    assert_eq!(later.checked, 4);
}

#[tokio::test]
async fn a_bridge_error_leaves_views_open_for_the_next_run() {
    let app = build_app(
        DEAD_ENDPOINT,
        ScriptedLlm::new("fake", vec![]),
        Options::default(),
    )
    .await;
    let now = now_utc();
    let view = synthetic(
        "v",
        "bullish",
        Some(("buy", 2390.0, 2391.4, 1.0, 99_999.0)),
        now - 7200,
        now - 3600,
    );
    store(&app, &view, now - 7200, now - 3600).await;
    let report = app.state.market().unwrap().evaluate_at(now).await;
    assert_eq!(report.failed_symbols, 1, "{report:?}");
    assert_eq!(report.updated, 0);
    let (_, journal) = get(&app, "/api/market/journal").await;
    assert_eq!(journal["views"][0]["outcome"], "open");
    assert_eq!(journal["stats"]["open"], 1);
}

#[tokio::test]
async fn the_evaluator_task_stops_on_shutdown() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let task = app.state.spawn_market_evaluator().expect("market is on");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!task.is_finished());
    app.state.shutdown().trigger();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the evaluator stops")
        .unwrap();
}

// ------------------------------------------------------------------ prepare

const GOLD_BUY: (&str, f64, f64, f64, f64) = ("buy", 2390.0, 2391.4, 2380.0, 2410.0);
const GOLD_SELL: (&str, f64, f64, f64, f64) = ("sell", 2391.12, 2395.0, 2405.0, 2375.0);

async fn app_with_view(
    id: &str,
    bias: &str,
    setup: Option<(&str, f64, f64, f64, f64)>,
    valid_in_secs: i64,
) -> (MockBridge, TestApp) {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let now = now_utc();
    store(
        &app,
        &synthetic(id, bias, setup, now - 60, now + valid_in_secs),
        now - 60,
        now + valid_in_secs,
    )
    .await;
    (mock, app)
}

#[tokio::test]
async fn prepare_needs_an_explicit_volume_and_order_type() {
    let (mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    for body in [
        json!({ "view_id": "v1", "order_type": "market" }),
        json!({ "view_id": "v1", "volume": null, "order_type": "market" }),
        json!({ "view_id": "v1", "volume": 0, "order_type": "market" }),
        json!({ "view_id": "v1", "volume": -1, "order_type": "market" }),
        json!({ "view_id": "v1", "volume": "0.1", "order_type": "market" }),
        json!({ "view_id": "v1", "volume": 0.1 }),
        json!({ "view_id": "v1", "volume": 0.1, "order_type": "stop" }),
        json!({ "volume": 0.1, "order_type": "market" }),
        json!({ "view_id": " ", "volume": 0.1, "order_type": "market" }),
        json!({ "view_id": "v1", "volume": 0.1, "order_type": "market", "sl": 1 }),
    ] {
        let (status, response) = post(&app, "/api/market/prepare", body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        assert_eq!(response["error"]["code"], "bad_request", "{body}");
    }
    assert_eq!(mock.executed_writes(), 0);
    assert!(
        !audit_kinds(&app).await.contains(&"preview".to_owned()),
        "a refused request must not create a preview"
    );
}

#[tokio::test]
async fn prepare_refuses_a_missing_an_expired_and_a_setupless_view() {
    let (_mock, app) = app_with_view("fresh", "bullish", Some(GOLD_BUY), 3600).await;
    let now = now_utc();
    store(
        &app,
        &synthetic("old", "bullish", Some(GOLD_BUY), now - 7200, now - 60),
        now - 7200,
        now - 60,
    )
    .await;
    store(
        &app,
        &synthetic("flat", "neutral", None, now - 60, now + 3600),
        now - 60,
        now + 3600,
    )
    .await;
    let body = |id: &str| json!({ "view_id": id, "volume": 0.1, "order_type": "market" });

    let (status, response) = post(&app, "/api/market/prepare", body("nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{response}");
    assert_eq!(response["error"]["code"], "not_found");

    let (status, response) = post(&app, "/api/market/prepare", body("old")).await;
    assert_eq!(status, StatusCode::GONE, "{response}");
    assert_eq!(response["error"]["code"], "view_expired");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("expired")
    );

    let (status, response) = post(&app, "/api/market/prepare", body("flat")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
    assert_eq!(response["error"]["code"], "no_setup");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no trade setup")
    );

    let (status, _) = post(&app, "/api/market/prepare", body("fresh")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_market_preview_needs_confirmation_and_sends_nothing() {
    let (mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    let before = mock.state();
    let (status, preview) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "v1", "volume": 0.1, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let description = preview["description"].as_str().unwrap();
    assert!(
        description.contains("Buy 0.10 XAUUSD at market")
            && description.contains("SL 2380")
            && description.contains("TP 2410"),
        "{description}"
    );
    assert_eq!(preview["symbols"], json!(["XAUUSD"]));
    assert!(preview["token"].as_str().is_some_and(|t| !t.is_empty()));
    assert!(preview["expires_at_ms"].as_i64().unwrap() > 0);
    assert_eq!(preview["is_batch"], false);

    // Nothing was sent: the preview waits for the confirm call.
    assert_eq!(mock.executed_writes(), 0);
    let after = mock.state();
    assert_eq!(after.positions.len(), before.positions.len());
    assert_eq!(after.orders.len(), before.orders.len());

    // Audited like another UI preview, with the view it came from.
    let rows = app.audit.rows(None).await.unwrap();
    let prepared = rows
        .iter()
        .find(|r| r.kind == "market_prepare")
        .expect("market_prepare row");
    assert_eq!(prepared.payload["view_id"], "v1");
    assert_eq!(prepared.payload["volume"], 0.1);
    assert_eq!(prepared.payload["order_type"], "market");
    assert_eq!(prepared.persona.as_deref(), Some("jarvis"));
    let logged = rows
        .iter()
        .find(|r| r.kind == "preview")
        .expect("preview row");
    assert_eq!(logged.payload["source"], "ui");

    // Confirming is a separate step and is what sends the order.
    let token = preview["token"].as_str().unwrap();
    let (status, confirmed) = post(&app, &format!("/api/confirm/{token}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    assert_eq!(mock.executed_writes(), 1);
}

#[tokio::test]
async fn a_limit_preview_uses_the_near_edge_of_the_entry_zone() {
    let (mock, app) = app_with_view("buy", "bullish", Some(GOLD_BUY), 3600).await;
    let now = now_utc();
    store(
        &app,
        &synthetic("sell", "bearish", Some(GOLD_SELL), now - 60, now + 3600),
        now - 60,
        now + 3600,
    )
    .await;

    let (status, buy) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "buy", "volume": 0.2, "order_type": "limit" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{buy}");
    let d = buy["description"].as_str().unwrap();
    assert!(d.starts_with("Buy limit 0.20 XAUUSD at 2390"), "{d}");
    assert!(d.contains("SL 2380") && d.contains("TP 2410"), "{d}");

    let (status, sell) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "sell", "volume": 0.2, "order_type": "limit" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sell}");
    let d = sell["description"].as_str().unwrap();
    assert!(d.starts_with("Sell limit 0.20 XAUUSD at 2395"), "{d}");
    assert!(d.contains("SL 2405") && d.contains("TP 2375"), "{d}");

    let (status, market_sell) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "sell", "volume": 0.2, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{market_sell}");
    assert!(
        market_sell["description"]
            .as_str()
            .unwrap()
            .starts_with("Sell 0.20 XAUUSD at market")
    );
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn a_risk_sized_preview_works_the_lots_out_from_the_stop() {
    let (mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    // Mock: equity 10,070.55 USD; gold ask 2391.40, tick 0.01 = 1.00 USD per lot.
    // 1% = 100.71 USD. Stop 2380: 11.40 away = 1,140 USD per lot. 100.71 / 1140 = 0.088 -> 0.08.
    let (status, preview) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "v1", "risk_percent": 1.0, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["total_volume"], 0.08, "{preview}");
    assert_eq!(
        preview["sizing"],
        json!({
            "risk_percent": 1.0,
            "risk_amount": 100.71,
            "currency": "USD",
            "stop_distance": 11.4,
            "volume": 0.08,
        })
    );
    let description = preview["description"].as_str().unwrap();
    assert!(
        description.contains("Buy 0.08 XAUUSD at market")
            && description.contains("1% risk (100.71 USD)"),
        "{description}"
    );
    assert_eq!(mock.executed_writes(), 0);

    let rows = app.audit.rows(None).await.unwrap();
    let prepared = rows
        .iter()
        .find(|r| r.kind == "market_prepare")
        .expect("market_prepare row");
    assert_eq!(prepared.payload["risk_percent"], 1.0);

    // The confirmed order carries the worked-out volume.
    let token = preview["token"].as_str().unwrap();
    let (status, confirmed) = post(&app, &format!("/api/confirm/{token}"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    assert_eq!(confirmed["results"][0]["volume"], 0.08, "{confirmed}");
    assert_eq!(mock.executed_writes(), 1);
}

#[tokio::test]
async fn a_plain_volume_preview_has_no_sizing() {
    let (_mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    let (status, preview) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "v1", "volume": 0.1, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert!(preview["sizing"].is_null(), "{preview}");
}

#[tokio::test]
async fn prepare_takes_a_volume_or_a_risk_never_both() {
    let (mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    for body in [
        json!({ "view_id": "v1", "volume": 0.1, "risk_percent": 1.0, "order_type": "market" }),
        json!({ "view_id": "v1", "risk_percent": 0, "order_type": "market" }),
        json!({ "view_id": "v1", "risk_percent": -1, "order_type": "market" }),
        json!({ "view_id": "v1", "risk_percent": "1", "order_type": "market" }),
    ] {
        let (status, response) = post(&app, "/api/market/prepare", body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
    }
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn a_risk_above_the_limit_or_too_small_for_the_smallest_lot_is_refused() {
    let (mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    // The default max_risk_percent is 2.
    let (status, response) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "v1", "risk_percent": 3.0, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
    assert_eq!(response["error"]["code"], "gate_rejected");
    let message = response["error"]["message"].as_str().unwrap();
    assert!(message.contains("max_risk_percent"), "{message}");

    // 0.1% = 10.07 USD; the smallest lot (0.01) risks 11.40 USD.
    let (status, response) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "v1", "risk_percent": 0.1, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
    let message = response["error"]["message"].as_str().unwrap();
    assert!(message.contains("risks 11.40 USD"), "{message}");
    assert_eq!(mock.executed_writes(), 0);
    assert!(!audit_kinds(&app).await.contains(&"preview".to_owned()));
}

#[tokio::test]
async fn the_gate_limits_still_apply_to_a_prepared_trade() {
    let (mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    // The test config allows 1 lot.
    let (status, response) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "v1", "volume": 5.0, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
    assert_eq!(response["error"]["code"], "gate_rejected");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("max_lot"),
        "{response}"
    );
    assert!(
        audit_kinds(&app)
            .await
            .contains(&"gate_rejected".to_owned())
    );
    assert_eq!(mock.executed_writes(), 0);

    // A symbol outside allowed_symbols is refused as well.
    let tight = Options {
        limits: "max_lot = 1.0\nmax_positions = 5\nmax_daily_loss = 500.0\nallowed_symbols = [\"EURUSD\"]",
        ..Options::default()
    };
    let mock2 = MockBridge::start().await;
    let app2 = app_on(&mock2, tight).await;
    let now = now_utc();
    store(
        &app2,
        &synthetic("v2", "bullish", Some(GOLD_BUY), now - 60, now + 3600),
        now - 60,
        now + 3600,
    )
    .await;
    let (status, response) = post(
        &app2,
        "/api/market/prepare",
        json!({ "view_id": "v2", "volume": 0.1, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
    assert_eq!(response["error"]["code"], "gate_rejected");
}

#[tokio::test]
async fn the_kill_switch_blocks_prepare() {
    let (mock, app) = app_with_view("v1", "bullish", Some(GOLD_BUY), 3600).await;
    let (status, _) = post(
        &app,
        "/api/killswitch",
        json!({ "engaged": true, "reason": "test" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, response) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "v1", "volume": 0.1, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::LOCKED, "{response}");
    assert_eq!(response["error"]["code"], "kill_switch");
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn a_view_never_creates_a_preview_by_itself() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    for _ in 0..3 {
        let (status, _) = get(&app, "/api/market/view?symbol=gold").await;
        assert_eq!(status, StatusCode::OK);
    }
    let kinds = audit_kinds(&app).await;
    for forbidden in ["preview", "gate_rejected", "confirm", "trade_result"] {
        assert!(
            !kinds.contains(&forbidden.to_owned()),
            "{forbidden} in {kinds:?}"
        );
    }
    assert_eq!(mock.executed_writes(), 0);
}

// ------------------------------------------------------------------ access

#[tokio::test]
async fn market_routes_need_a_session_when_login_is_on() {
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            auth: true,
            ..Options::default()
        },
    )
    .await;
    for (method, uri) in [
        ("GET", "/api/market/view?symbol=gold"),
        ("GET", "/api/market/calendar"),
        ("GET", "/api/market/news?symbol=gold"),
        ("GET", "/api/market/journal"),
        ("POST", "/api/market/prepare"),
    ] {
        let (status, _) = request(&app.router, method, uri, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
    let user = create_user(&app, "ann", PASSWORD).await;
    let cookie = login(&app, &user).await;
    let (status, body) =
        request_as(&app, &cookie, "GET", "/api/market/view?symbol=gold", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn views_and_news_have_a_request_budget() {
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            market: "rate_limit_per_minute = 2",
            ..Options::default()
        },
    )
    .await;
    for _ in 0..2 {
        let (status, _) = get(&app, "/api/market/news?symbol=gold").await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, body) = get(&app, "/api/market/view?symbol=gold").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"]["code"], "rate_limited");
    // Reading the journal is not budgeted.
    let (status, _) = get(&app, "/api/market/journal").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn market_views_can_be_turned_off() {
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            market: "enabled = false",
            ..Options::default()
        },
    )
    .await;
    for (method, uri) in [
        ("GET", "/api/market/view?symbol=gold"),
        ("GET", "/api/market/calendar"),
        ("GET", "/api/market/news?symbol=gold"),
        ("GET", "/api/market/journal"),
    ] {
        let (status, body) = request(&app.router, method, uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
    let (status, body) = post(
        &app,
        "/api/market/prepare",
        json!({ "view_id": "x", "volume": 1, "order_type": "market" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (_, status) = get(&app, "/api/status").await;
    assert_eq!(
        status["market"],
        json!({ "enabled": false, "news_provider": "none", "news_configured": false })
    );
    let names: Vec<String> = app
        .state
        .agent()
        .tools()
        .specs()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(
        !names
            .iter()
            .any(|n| n == "get_market_view" || n == "get_news"),
        "{names:?}"
    );
    assert!(app.state.spawn_market_evaluator().is_none());
}

// ------------------------------------------------------------------ tools

fn ctx() -> jarvis_api::tools::ToolContext {
    jarvis_api::tools::ToolContext {
        session_id: uuid::Uuid::new_v4(),
        intent: Intent::Other,
        voice: false,
        persona: Some("jarvis".to_owned()),
    }
}

#[tokio::test]
async fn the_market_tools_are_listed_and_read_only() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let specs = app.state.agent().tools().specs();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    for tool in ["get_market_view", "get_economic_calendar", "get_news"] {
        assert!(names.contains(&tool), "{tool} in {names:?}");
        let parsed = jarvis_api::tools::ReadTool::from_name(tool).expect("a read tool");
        assert_eq!(parsed.name(), tool);
        assert!(jarvis_api::tools::ProposeTool::from_name(tool).is_none());
        let spec = specs.iter().find(|s| s.name == tool).unwrap();
        assert!(spec.description.len() > 100, "{tool}");
        assert_eq!(spec.parameters["type"], "object");
    }
    let view = specs.iter().find(|s| s.name == "get_market_view").unwrap();
    assert_eq!(view.parameters["required"], json!(["symbol"]));
    let calendar = specs
        .iter()
        .find(|s| s.name == "get_economic_calendar")
        .unwrap();
    assert_eq!(
        calendar.parameters["properties"]["hours_ahead"]["minimum"],
        1
    );
    assert_eq!(
        calendar.parameters["properties"]["hours_ahead"]["maximum"],
        168
    );

    // Calling them writes nothing and proposes nothing.
    let tools = app.state.agent().tools();
    let out = tools
        .execute_in(&ctx(), "get_market_view", &json!({ "symbol": "gold" }))
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.preview.is_none());
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn the_view_tool_returns_the_numbers_without_the_bulk() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let tools = app.state.agent().tools();
    let context = ctx();
    let out = tools
        .execute_in(&context, "get_market_view", &json!({ "symbol": "gold" }))
        .await;
    assert!(!out.is_error, "{}", out.content);
    let v: Value = serde_json::from_str(&out.content).unwrap();
    for key in [
        "symbol",
        "bias",
        "confidence",
        "score",
        "price",
        "factors",
        "levels",
        "events",
        "news",
        "summary",
        "disclaimer",
        "valid_until",
        "stand_aside",
    ] {
        assert!(v.get(key).is_some(), "{key} missing: {}", out.content);
    }
    for key in ["id", "broker_symbol", "digits"] {
        assert!(v.get(key).is_none(), "{key} should be left out");
    }
    for t in v["timeframes"].as_array().unwrap() {
        assert_eq!(
            keys(t),
            BTreeSet::from(["timeframe", "trend", "rsi", "atr"])
        );
    }
    assert!(!out.content.contains("ema20"));
    assert!(out.content.len() < 12_000, "{} bytes", out.content.len());

    // The call is audited with the conversation it belongs to.
    let rows = app.audit.rows(Some(context.session_id)).await.unwrap();
    let row = rows
        .iter()
        .find(|r| r.kind == "market_view")
        .expect("audited");
    assert_eq!(row.payload["source"], "tool");
    assert_eq!(row.persona.as_deref(), Some("jarvis"));

    // Errors read well for the model.
    let bad = tools
        .execute_in(&context, "get_market_view", &json!({}))
        .await;
    assert!(
        bad.is_error && bad.content.contains("invalid arguments"),
        "{}",
        bad.content
    );
    let unknown = tools
        .execute_in(
            &context,
            "get_market_view",
            &json!({ "symbol": "notasymbol" }),
        )
        .await;
    assert!(unknown.is_error, "{}", unknown.content);
    assert!(
        unknown.content.contains("MT5") || unknown.content.contains("symbol"),
        "{}",
        unknown.content
    );
}

#[tokio::test]
async fn the_calendar_and_news_tools_validate_and_answer() {
    let mock = MockBridge::start().await;
    let app = app_on(&mock, Options::default()).await;
    let tools = app.state.agent().tools();
    let context = ctx();

    let out = tools
        .execute_in(
            &context,
            "get_economic_calendar",
            &json!({ "hours_ahead": 168, "currencies": ["usd", "EUR"] }),
        )
        .await;
    assert!(!out.is_error, "{}", out.content);
    let v: Value = serde_json::from_str(&out.content).unwrap();
    assert_eq!(v["hours_ahead"], 168);
    assert_eq!(v["currencies"], json!(["USD", "EUR"]));
    let events = v["events"].as_array().unwrap();
    assert!(!events.is_empty());
    assert_eq!(v["listed"], events.len());
    assert!(
        events
            .iter()
            .all(|e| ["USD", "EUR"].contains(&e["currency"].as_str().unwrap()))
    );

    // Default window, a string of codes, a number sent as text.
    for args in [
        json!({}),
        json!({ "currencies": "usd,eur" }),
        json!({ "hours_ahead": "6" }),
    ] {
        let out = tools
            .execute_in(&context, "get_economic_calendar", &args)
            .await;
        assert!(!out.is_error, "{args}: {}", out.content);
    }
    for args in [
        json!({ "hours_ahead": 0 }),
        json!({ "hours_ahead": 169 }),
        json!({ "hours_ahead": 1.5 }),
        json!({ "currencies": ["dollar"] }),
        json!({ "hours_ahead": "x" }),
    ] {
        let out = tools
            .execute_in(&context, "get_economic_calendar", &args)
            .await;
        assert!(out.is_error, "{args}: {}", out.content);
    }

    let out = tools
        .execute_in(&context, "get_news", &json!({ "symbol": "gold" }))
        .await;
    assert!(!out.is_error, "{}", out.content);
    let v: Value = serde_json::from_str(&out.content).unwrap();
    assert_eq!(v["configured"], false);
    assert!(v["note"].as_str().unwrap().contains("No news provider"));
}

#[tokio::test]
async fn the_news_tool_returns_headlines_without_links() {
    let server = finnhub_serving_gold_news().await;
    let mock = MockBridge::start().await;
    let app = app_on(
        &mock,
        Options {
            news: Some(finnhub_at(&server)),
            ..Options::default()
        },
    )
    .await;
    let tools = app.state.agent().tools();
    let out = tools
        .execute_in(&ctx(), "get_news", &json!({ "symbol": "gold" }))
        .await;
    assert!(!out.is_error, "{}", out.content);
    let v: Value = serde_json::from_str(&out.content).unwrap();
    assert_eq!(v["configured"], true);
    assert_eq!(v["provider"], "finnhub");
    assert_eq!(v["summary"]["count"], 2);
    let headlines = v["summary"]["headlines"].as_array().unwrap();
    assert_eq!(headlines.len(), 2);
    assert!(headlines.iter().all(|h| h.get("url").is_none()));
    assert!(!out.content.contains("test-key-123"));
}

/// A model that asks for a view of gold and then repeats the numbers of the result.
struct EchoModel {
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmProvider for EchoModel {
    fn name(&self) -> &str {
        "fake"
    }
    fn model(&self) -> &str {
        "echo-1"
    }
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmError> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        if call == 1 {
            return Ok(tool_use_response(
                "t1",
                "get_market_view",
                json!({ "symbol": "gold" }),
            ));
        }
        let result = request
            .messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .expect("a tool result");
        let v: Value = serde_json::from_str(&result).expect("JSON result");
        let mut reply = format!(
            "Gold has a {} bias with confidence {}. Bid is {} and ask is {}.",
            v["bias"].as_str().unwrap(),
            v["confidence"],
            v["price"]["bid"],
            v["price"]["ask"],
        );
        if let Some(setup) = v["setup"].as_object() {
            reply.push_str(&format!(
                " The {} setup has a stop at {} and a first target at {}.",
                setup["side"].as_str().unwrap(),
                setup["stop_loss"],
                setup["take_profit"],
            ));
        }
        reply.push_str(" This is a computed analysis, not a guarantee.");
        Ok(text_response(&reply))
    }
}

#[tokio::test]
async fn the_figures_the_model_repeats_from_a_view_pass_the_grounding_check() {
    let mock = MockBridge::start().await;
    let model = std::sync::Arc::new(EchoModel {
        calls: Mutex::new(0),
    });
    let mut slots = HashMap::new();
    slots.insert(
        "fake".to_owned(),
        jarvis_api::state::ProviderSlot::Available(model.clone() as std::sync::Arc<dyn LlmProvider>),
    );
    let toml = config_toml(
        "demo",
        &mock.endpoint,
        "",
        &["XAUUSD"],
        DEFAULT_LIMITS,
        "",
        "",
        false,
    );
    let config = jarvis_api::config::Config::from_toml_str(&toml).unwrap();
    let audit = jarvis_api::audit::Audit::connect(&temp_db_url())
        .await
        .unwrap();
    let state = jarvis_api::AppState::new_with(
        config,
        bridge_client(&mock, 300),
        audit.clone(),
        slots,
        jarvis_api::state::Overrides {
            personas: Some(real_personas()),
            news: Some(NewsService::disabled()),
            ..Default::default()
        },
    );
    let router = jarvis_api::app(state);
    let (status, body) = request(
        &router,
        "POST",
        "/api/chat",
        Some(json!({ "message": "what is your view on gold today?" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tool_calls"][0]["name"], "get_market_view");
    assert_eq!(body["tool_calls"][0]["ok"], true);
    let reply = body["reply"].as_str().unwrap();
    assert!(reply.starts_with("Gold has a "), "{reply}");
    assert_ne!(reply, jarvis_api::grounding::UNVERIFIED_REPLY);
    let kinds: Vec<String> = audit
        .rows(None)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.kind)
        .collect();
    assert!(!kinds.contains(&"ungrounded_reply".to_owned()), "{kinds:?}");
    assert!(kinds.contains(&"market_view".to_owned()));
    assert_eq!(*model.calls.lock().unwrap(), 2, "no retry was needed");
    // The view did not start a trade.
    assert!(!kinds.contains(&"preview".to_owned()));
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn a_figure_the_view_did_not_give_is_caught() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new(
        "fake",
        vec![
            tool_use("t1", "get_market_view", json!({ "symbol": "gold" })),
            text("Gold looks bullish with a stop at 1234.56."),
            text("Gold looks bullish."),
        ],
    );
    let app = build_app(&mock, llm.clone(), Options::default()).await;
    let (status, body) = chat(&app, "your view on gold?").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reply"], "Gold looks bullish.");
    assert!(
        audit_kinds(&app)
            .await
            .contains(&"ungrounded_reply".to_owned())
    );
}

#[tokio::test]
async fn the_system_prompt_carries_the_view_rules() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![text("hi")]);
    let app = build_app(&mock, llm.clone(), Options::default()).await;
    chat(&app, "hello").await;
    let requests = llm.requests.lock().unwrap();
    let system = requests[0].system.as_deref().unwrap();
    for needle in [
        "get_market_view",
        "get_economic_calendar",
        "get_news",
        "computed analysis and not a guarantee",
        "never present it as certain",
        "only when the user asks for that trade with a volume",
    ] {
        assert!(system.contains(needle), "{needle} missing");
    }
    assert!(!system.contains("do not give investment recommendations"));
}

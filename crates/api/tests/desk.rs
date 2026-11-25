//! Desk mode (spec-nimbus-desk, ADR-0023): Friday's tools, and the one gated path for money.
//! Sentinel must approve the exact plan, Pilot only asks for a preview, and the trader confirms.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use jarvis_api::clock::ManualClock;
use jarvis_bridge_client::mock::MockBridge;
use jarvis_desk::sentinel::Side;
use serde_json::{Value, json};

const FAKE_DESK: &str = "provider = 'fake'";

async fn post(app: &TestApp, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    request(&app.router, "POST", uri, body).await
}

async fn get(app: &TestApp, uri: &str) -> (StatusCode, Value) {
    request(&app.router, "GET", uri, None).await
}

async fn desk_chat(app: &TestApp, message: &str) -> (StatusCode, Value) {
    post(
        app,
        "/api/chat",
        Some(json!({ "message": message, "mode": "desk" })),
    )
    .await
}

async fn desk_app(mock: &MockBridge, llm: Arc<ScriptedLlm>, desk: &str) -> TestApp {
    build_app(
        mock,
        llm,
        Options {
            desk,
            ..Options::default()
        },
    )
    .await
}

/// Sentinel's review of a gold buy with a stop under the price, straight from the Desk.
async fn approved_gold_plan(app: &TestApp) -> String {
    let desk = app.state.desk().expect("desk on");
    let record = desk
        .review_plan(
            "XAUUSD",
            "XAUUSD",
            Side::Buy,
            Some(2380.0),
            Some(2410.0),
            None,
        )
        .await
        .expect("plan");
    assert!(record.verdict.approved(), "{:?}", record.verdict);
    assert!(record.plan.volume > 0.0);
    record.id
}

#[tokio::test]
async fn friday_gets_the_nine_specialists_and_nothing_else() {
    let mock = MockBridge::start().await;
    let app = desk_app(&mock, ScriptedLlm::new("fake", vec![]), FAKE_DESK).await;
    let desk = app.state.desk().unwrap();
    let mut names: Vec<String> = desk.tool_specs().into_iter().map(|s| s.name).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "ask_athena",
            "ask_atlas",
            "ask_capitol",
            "ask_chartist",
            "ask_ledger",
            "ask_oracle",
            "ask_pilot",
            "ask_scout",
            "ask_sentinel"
        ]
    );
    // No tool writes: none is a trading tool, and Pilot's says it sends nothing.
    for s in desk.tool_specs() {
        assert!(!s.name.starts_with("propose_"), "{}", s.name);
    }
    let pilot = desk
        .tool_specs()
        .into_iter()
        .find(|s| s.name == "ask_pilot")
        .unwrap();
    assert!(pilot.description.contains("Sends nothing"));
}

#[tokio::test]
async fn sentinel_then_pilot_then_confirm_is_the_only_way_to_trade() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new(
        "fake",
        vec![
            Ok(tool_use_response(
                "c1",
                "ask_sentinel",
                json!({ "symbol": "XAUUSD", "side": "buy", "stop": 2380.0, "target": 2410.0 }),
            )),
            Ok(text_response("Sentinel approved the plan.")),
        ],
    );
    let app = desk_app(&mock, llm, FAKE_DESK).await;
    let (status, body) = desk_chat(&app, "size a long on gold with a stop at 2380").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "desk");
    assert!(body["preview"].is_null(), "Sentinel alone makes no preview");
    let plan = &body["desk"]["plans"][0];
    assert_eq!(plan["verdict"]["decision"], "approved", "{plan}");
    let plan_id = plan["id"].as_str().unwrap().to_owned();
    assert_eq!(mock.executed_writes(), 0);

    // The plan card's button: Pilot asks for a preview, which still needs the confirm.
    let (status, preview) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let token = preview["token"].as_str().unwrap().to_owned();
    assert!(preview["total_volume"].as_f64().unwrap() > 0.0);
    assert_eq!(mock.executed_writes(), 0, "a preview writes nothing");

    // The same approval cannot be used twice.
    let (status, again) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{again}");
    assert!(
        again["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already been used")
    );

    let (status, done) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(mock.executed_writes(), 1);

    let kinds = audit_kinds(&app).await;
    for k in [
        "sentinel_verdict",
        "pilot_preview",
        "desk_finding",
        "confirm",
    ] {
        assert!(kinds.iter().any(|x| x == k), "{k} missing from {kinds:?}");
    }
}

#[tokio::test]
async fn pilot_refuses_unknown_vetoed_and_expired_plans() {
    let mock = MockBridge::start().await;
    let clock = ManualClock::starting_now();
    let app = build_app(
        &mock,
        ScriptedLlm::new("fake", vec![]),
        Options {
            desk: FAKE_DESK,
            clock: Some(clock.clone()),
            ..Options::default()
        },
    )
    .await;
    let (status, _) = post(&app, "/api/desk/plans/nope/pilot", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // No stop: vetoed, and a veto is final.
    let desk = app.state.desk().unwrap();
    let vetoed = desk
        .review_plan("XAUUSD", "XAUUSD", Side::Buy, None, None, None)
        .await
        .unwrap();
    assert!(!vetoed.verdict.approved());
    let (status, body) = post(&app, &format!("/api/desk/plans/{}/pilot", vetoed.id), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("vetoed")
    );

    // An approval is valid for two minutes.
    let id = approved_gold_plan(&app).await;
    clock.advance_ms(jarvis_desk::sentinel::APPROVAL_TTL_MS + 1);
    let (status, body) = post(&app, &format!("/api/desk/plans/{id}/pilot"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("expired")
    );
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn the_kill_switch_and_gate_limits_still_apply_to_pilot() {
    let mock = MockBridge::start().await;
    let app = desk_app(&mock, ScriptedLlm::new("fake", vec![]), FAKE_DESK).await;
    let id = approved_gold_plan(&app).await;
    let (status, _) = post(
        &app,
        "/api/killswitch",
        Some(json!({ "engaged": true, "reason": "test" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = post(&app, &format!("/api/desk/plans/{id}/pilot"), None).await;
    assert_eq!(status, StatusCode::LOCKED, "{body}");
    // With the switch engaged Sentinel vetoes new plans too.
    let desk = app.state.desk().unwrap();
    let r = desk
        .review_plan("XAUUSD", "XAUUSD", Side::Buy, Some(2380.0), None, None)
        .await
        .unwrap();
    assert!(!r.verdict.approved());

    // A gate limit Sentinel does not know about: the lot cap.
    let mock = MockBridge::start().await;
    let app = build_app(
        &mock,
        ScriptedLlm::new("fake", vec![]),
        Options {
            desk: "provider = 'fake'\nrisk_percent = 2.0",
            limits: "max_lot = 0.01\nmax_positions = 5\nmax_daily_loss = 500.0\nmax_risk_percent = 2.0\nallowed_symbols = [\"XAUUSD\"]",
            ..Options::default()
        },
    )
    .await;
    let desk = app.state.desk().unwrap();
    let r = desk
        .review_plan("XAUUSD", "XAUUSD", Side::Buy, Some(2380.0), None, None)
        .await
        .unwrap();
    assert!(r.verdict.approved(), "{:?}", r.verdict);
    let (status, body) = post(&app, &format!("/api/desk/plans/{}/pilot", r.id), None).await;
    assert!(status.is_client_error(), "{status} {body}");
    assert_eq!(mock.executed_writes(), 0);
}

#[tokio::test]
async fn friday_cannot_send_pilot_unless_the_trader_asked_to_trade() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone(), FAKE_DESK).await;
    let id = approved_gold_plan(&app).await;
    llm.push(tool_use_response(
        "p1",
        "ask_pilot",
        json!({ "plan_id": id }),
    ));
    llm.push(text_response("Pilot could not act on that."));
    let (status, body) = desk_chat(&app, "what do you make of gold today?").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["preview"].is_null(), "{body}");
    assert_eq!(body["tool_calls"][0]["ok"], false);

    // When the trader asks in their own words, the same plan becomes a preview.
    let id = approved_gold_plan(&app).await;
    llm.push(tool_use_response(
        "p2",
        "ask_pilot",
        json!({ "plan_id": id }),
    ));
    llm.push(text_response(
        "The preview is ready; confirm it when you are happy.",
    ));
    let (status, body) = desk_chat(&app, "buy gold with a stop at 2380").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["preview"]["token"].is_string(), "{body}");
    assert_eq!(mock.executed_writes(), 0, "only the confirm writes");
}

#[tokio::test]
async fn the_desk_cap_stops_claude_before_any_request() {
    let mock = MockBridge::start().await;
    let claude = ScriptedLlm::always("claude", text_response("hello"));
    let app = build_app(
        &mock,
        ScriptedLlm::new("fake", vec![]),
        Options {
            claude_llm: Some(claude.clone()),
            desk: "monthly_budget_usd = 0.0",
            ..Options::default()
        },
    )
    .await;
    let (status, body) = desk_chat(&app, "what's moving?").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["reply"]
            .as_str()
            .unwrap()
            .contains("monthly Claude budget"),
        "{body}"
    );
    assert_eq!(claude.request_count(), 0);
}

#[tokio::test]
async fn figures_no_seat_reported_are_not_spoken() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::always("fake", text_response("NVDA will hit 987.65 by Friday."));
    let app = desk_app(&mock, llm, FAKE_DESK).await;
    let (status, body) = desk_chat(&app, "where is NVDA going?").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reply = body["reply"].as_str().unwrap();
    assert!(!reply.contains("987.65"), "{reply}");
    let kinds = audit_kinds(&app).await;
    assert!(
        kinds.iter().any(|k| k == "ungrounded_reply_blocked"),
        "{kinds:?}"
    );
}

#[tokio::test]
async fn specialists_report_from_mt5_and_show_missing_data_as_null() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new(
        "fake",
        vec![
            Ok(tool_use_response(
                "o1",
                "ask_oracle",
                json!({ "symbol": "XAUUSD", "timeframe": "H1" }),
            )),
            Ok(tool_use_response(
                "c1",
                "ask_capitol",
                json!({ "symbol": "NVDA" }),
            )),
            Ok(tool_use_response(
                "h1",
                "ask_chartist",
                json!({ "symbol": "XAUUSD", "forecast": true }),
            )),
            Ok(text_response("Here is the desk view.")),
        ],
    );
    let app = desk_app(&mock, llm, FAKE_DESK).await;
    let (status, body) = desk_chat(&app, "desk view on gold").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let findings = body["desk"]["findings"].as_array().unwrap();
    let agents: Vec<&str> = findings
        .iter()
        .map(|f| f["agent"].as_str().unwrap())
        .collect();
    assert_eq!(agents, ["oracle", "capitol", "chartist"]);
    // No Finnhub key: Capitol says so, with nulls rather than numbers.
    assert!(findings[1]["data"]["insiders"].is_null());
    assert!(!findings[1]["notes"].as_array().unwrap().is_empty());
    // The chart the web draws came with the reply.
    assert_eq!(body["desk"]["chart"]["symbol"], "XAUUSD");
    assert!(!body["desk"]["chart"]["bars"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_state_endpoint_has_every_panel() {
    let mock = MockBridge::start().await;
    let app = desk_app(&mock, ScriptedLlm::new("fake", vec![]), FAKE_DESK).await;
    let (status, s) = get(&app, "/api/desk/state").await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["enabled"], true);
    assert_eq!(s["brain"]["available"], true);
    assert_eq!(s["brain"]["metered"], false);
    assert_eq!(s["seats"].as_array().unwrap().len(), 10);
    assert_eq!(s["seats"][2]["id"], "capitol");
    assert_eq!(s["seats"][2]["state"], "offline", "no Finnhub key");
    assert!(s["session"]["phase"].is_string());
    assert_eq!(s["calendar"]["months"].as_array().unwrap().len(), 2);
    assert_eq!(s["rules"]["risk_percent"], 0.5);

    let (status, p) = get(&app, "/api/desk/pulse").await;
    assert_eq!(status, StatusCode::OK, "{p}");
    let quotes = p["quotes"].as_array().unwrap();
    assert!(
        quotes
            .iter()
            .any(|q| q["symbol"] == "XAUUSD" && q["price"].is_number())
    );
    // An index the mock does not have shows nulls, not numbers.
    assert!(
        quotes
            .iter()
            .any(|q| q["symbol"] == "NAS100" && q["price"].is_null())
    );
}

#[tokio::test]
async fn friday_can_pilot_when_the_trader_states_the_risk() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone(), FAKE_DESK).await;
    let id = approved_gold_plan(&app).await;
    llm.push(tool_use_response(
        "p1",
        "ask_pilot",
        json!({ "plan_id": id }),
    ));
    llm.push(text_response("The preview is ready."));
    let (status, body) = desk_chat(&app, "buy gold with a stop at 2380 risking 0.5%").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["preview"]["token"].is_string(), "{body}");

    // A different risk than Sentinel sized is refused by the cross-check.
    let id = approved_gold_plan(&app).await;
    llm.push(tool_use_response(
        "p2",
        "ask_pilot",
        json!({ "plan_id": id }),
    ));
    llm.push(text_response("Pilot could not act on that."));
    let (_, body) = desk_chat(&app, "buy gold with a stop at 2380 risking 1%").await;
    assert!(body["preview"].is_null(), "{body}");
    assert_eq!(body["tool_calls"][0]["ok"], false);
}

#[tokio::test]
async fn pilot_rechecks_the_position_cap_and_it_survives_a_restart() {
    let mock = MockBridge::start().await;
    let desk = "provider = 'fake'\nmax_open_positions = 1";
    let app = desk_app(&mock, ScriptedLlm::new("fake", vec![]), desk).await;
    // Two approvals while no Desk position is open.
    let first = approved_gold_plan(&app).await;
    let second = approved_gold_plan(&app).await;

    let (status, preview) = post(&app, &format!("/api/desk/plans/{first}/pilot"), None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let token = preview["token"].as_str().unwrap().to_owned();
    let (status, done) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(mock.executed_writes(), 1);
    // The Desk follows the execution from the event (give the listener a moment).
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // The second approval was valid when given, but the cap is full now.
    let (status, body) = post(&app, &format!("/api/desk/plans/{second}/pilot"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cap is 1"),
        "{body}"
    );
    assert_eq!(mock.executed_writes(), 1);

    // After a restart over the same audit log, the open Desk position still counts.
    let again = build_app(
        &mock,
        ScriptedLlm::new("fake", vec![]),
        Options {
            desk,
            audit: Some(app.audit.clone()),
            ..Options::default()
        },
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let r = again
        .state
        .desk()
        .unwrap()
        .review_plan("XAUUSD", "XAUUSD", Side::Buy, Some(2380.0), None, None)
        .await
        .unwrap();
    assert!(!r.verdict.approved(), "{:?}", r.verdict);
}

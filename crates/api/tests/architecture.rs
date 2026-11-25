//! The architecture page (spec-architecture-page): decision records from Friday's turns and the
//! free council scan, stop codes for every way a decision ends without a trade, the funnel, the
//! trader's review and the exports.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use jarvis_bridge_client::mock::MockBridge;
use serde_json::{Value, json};

const FAKE_DESK: &str = "provider = 'fake'\nscan_every_minutes = 0";
/// The Desk table, then the learning review on the scripted model.
const LEARNING_DESK: &str =
    "provider = 'fake'\nscan_every_minutes = 0\n\n[learning]\nprovider = 'fake'\nmodel = ''";

async fn post(app: &TestApp, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    request(&app.router, "POST", uri, body).await
}

async fn get(app: &TestApp, uri: &str) -> (StatusCode, Value) {
    request(&app.router, "GET", uri, None).await
}

async fn put(app: &TestApp, uri: &str, body: Value) -> (StatusCode, Value) {
    request(&app.router, "PUT", uri, Some(body)).await
}

async fn desk_app(mock: &MockBridge, llm: Arc<ScriptedLlm>) -> TestApp {
    build_app(
        mock,
        llm,
        Options {
            desk: FAKE_DESK,
            watchlist: &["EURUSD", "XAUUSD"],
            ..Options::default()
        },
    )
    .await
}

/// A Desk turn in which Friday has Oracle look at gold and Sentinel approve a long.
async fn gold_turn(app: &TestApp, llm: &ScriptedLlm) -> Value {
    llm.push(tool_use_response(
        "o1",
        "ask_oracle",
        json!({ "symbol": "XAUUSD" }),
    ));
    llm.push(tool_use_response(
        "s1",
        "ask_sentinel",
        json!({ "symbol": "XAUUSD", "side": "buy", "stop": 2380.0, "target": 2430.0 }),
    ));
    llm.push(text_response("Sentinel approved the gold long."));
    let (status, body) = post(
        app,
        "/api/chat",
        Some(json!({ "message": "size a long on gold", "mode": "desk" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn only_decision(app: &TestApp) -> Value {
    let (status, list) = get(app, "/api/architecture/decisions").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let rows = list["decisions"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{list}");
    let id = rows[0]["id"].as_str().unwrap();
    let (status, d) = get(app, &format!("/api/architecture/decisions/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{d}");
    d
}

#[tokio::test]
async fn a_friday_turn_records_the_decision_from_opinions_to_execution() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    let body = gold_turn(&app, &llm).await;
    let plan_id = body["desk"]["plans"][0]["id"].as_str().unwrap().to_owned();

    let d = only_decision(&app).await;
    assert_eq!(d["symbol"], "XAUUSD");
    assert_eq!(d["trigger"]["kind"], "question");
    assert_eq!(d["plan_id"], plan_id.as_str());
    assert_eq!(d["status"], "open", "{d}");
    // Every member has an opinion; the ones Friday did not ask say so.
    let ops = d["opinions"].as_array().unwrap();
    assert_eq!(ops.len(), 6, "{d}");
    let atlas = ops.iter().find(|o| o["agent"] == "atlas").unwrap();
    assert_eq!(atlas["confidence"], 0);
    assert!(
        atlas["evidence"][0]
            .as_str()
            .unwrap()
            .contains("did not ask")
    );
    // The master record: model, system prompt hash, the messages and the reply.
    let master = &d["master"];
    assert_eq!(master["provider"], "fake");
    assert_eq!(master["system_hash"].as_str().unwrap().len(), 12);
    assert!(
        master["messages"].as_array().unwrap().len() >= 3,
        "{master}"
    );
    assert_eq!(master["reply"], "Sentinel approved the gold long.");
    assert!(d["council"]["contributions"].is_array());

    // Pilot previews: the decision is at execution.
    let (status, preview) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let d = only_decision(&app).await;
    assert_eq!(d["stage"], "execution");
    assert_eq!(d["preview_id"], preview["id"]);

    // The trader confirms: traded, with the ticket.
    let token = preview["token"].as_str().unwrap();
    let (status, done) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let mut traded = Value::Null;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        traded = only_decision(&app).await;
        if traded["status"] == "traded" {
            break;
        }
    }
    assert_eq!(traded["status"], "traded", "{traded}");
    assert_eq!(traded["step"], 5);
    assert!(traded["stop_code"].is_null());
    assert_eq!(traded["tickets"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_pilot_refusal_stops_the_decision_with_its_code() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    let body = gold_turn(&app, &llm).await;
    let plan_id = body["desk"]["plans"][0]["id"].as_str().unwrap().to_owned();
    app.state.trading().engage_kill_switch("test").await;
    let (status, _) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::LOCKED);
    let d = only_decision(&app).await;
    assert_eq!(d["status"], "stopped");
    assert_eq!(d["stop_code"], "kill_switch");
    assert_eq!(d["stop_label"], "Kill switch engaged");
}

#[tokio::test]
async fn a_vetoed_plan_is_stopped_at_the_master_with_sentinels_rule() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    // Target too close: reward to risk under the 1.5 minimum.
    llm.push(tool_use_response(
        "s1",
        "ask_sentinel",
        json!({ "symbol": "XAUUSD", "side": "buy", "stop": 2380.0, "target": 2400.5 }),
    ));
    llm.push(text_response("Sentinel vetoed it."));
    let (status, body) = post(
        &app,
        "/api/chat",
        Some(json!({ "message": "size a long on gold", "mode": "desk" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let d = only_decision(&app).await;
    assert_eq!(d["status"], "stopped", "{d}");
    assert_eq!(d["stage"], "master");
    assert_eq!(d["stop_code"], "sentinel_veto:min_reward_risk", "{d}");
    assert_eq!(d["step"], 3);
}

#[tokio::test]
async fn the_scan_records_every_symbol_without_any_model_call() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    let (status, body) = post(&app, "/api/architecture/scan", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(llm.request_count(), 0, "the scan never calls a model");
    let rows = body["decisions"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{body}");
    for r in rows {
        assert_eq!(r["trigger"]["kind"], "scan");
        // Exactly one of: traded (never from a scan), waiting for Friday, or a stop code.
        let waiting = r["status"] == "waiting";
        assert_eq!(waiting, r["stop_code"] == "not_asked", "{r}");
        assert!(r["stop_code"].is_string(), "{r}");
    }
    // The funnel's counts add up: every decision is counted at step 0 and dropped once.
    let (status, f) = get(&app, "/api/architecture/funnel").await;
    assert_eq!(status, StatusCode::OK, "{f}");
    let steps = f["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 6);
    assert_eq!(steps[0]["count"], 2);
    let dropped: u64 = steps
        .iter()
        .flat_map(|s| s["dropped"].as_object().unwrap().values())
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert_eq!(dropped, 2, "{f}");
    let (status, map) = get(&app, "/api/architecture/map").await;
    assert_eq!(status, StatusCode::OK, "{map}");
    assert_eq!(map["members"].as_array().unwrap().len(), 6);
    assert!(map["live"]["version"].is_null());
}

#[tokio::test]
async fn ask_friday_only_works_on_a_waiting_decision() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    gold_turn(&app, &llm).await;
    let d = only_decision(&app).await;
    let id = d["id"].as_str().unwrap();
    let before = llm.request_count();
    let (status, body) = post(&app, &format!("/api/architecture/decisions/{id}/ask"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(llm.request_count(), before, "nothing was sent to the model");
    let (status, _) = post(&app, "/api/architecture/decisions/nope/ask", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reviews_are_validated_saved_audited_and_exported() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    gold_turn(&app, &llm).await;
    let d = only_decision(&app).await;
    let id = d["id"].as_str().unwrap();
    let uri = format!("/api/architecture/decisions/{id}/review");

    let (status, _) = put(&app, &uri, json!({ "rating": 9 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let review = json!({ "setup": "Momentum", "mistakes": ["Target too tight"],
                         "regime": "Trend", "rating": 4, "take_again": true,
                         "notes": "Ran past the target." });
    let (status, saved) = put(&app, &uri, review).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["review"]["rating"], 4);
    assert!(
        audit_kinds(&app)
            .await
            .contains(&"decision_reviewed".to_owned())
    );
    let (status, _) = put(&app, "/api/architecture/decisions/nope/review", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, list) = get(&app, "/api/architecture/decisions?symbol=XAUUSD").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list["decisions"][0]["tags"],
        json!(["Momentum", "Target too tight", "Trend"])
    );
    let (status, _, bytes) = raw_request(
        &app.router,
        "GET",
        "/api/architecture/decisions?format=csv",
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let csv = String::from_utf8(bytes).unwrap();
    assert!(csv.starts_with("id,created_at,symbol"), "{csv}");
    assert!(csv.contains("\"XAUUSD\""));
    let (status, full) = get(&app, "/api/architecture/decisions?format=full").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(full["decisions"][0]["review"]["setup"], "Momentum");
    let (status, _) = get(&app, "/api/architecture/decisions?format=xml").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

async fn learning_app(mock: &MockBridge, llm: Arc<ScriptedLlm>) -> TestApp {
    build_app(
        mock,
        llm,
        Options {
            desk: LEARNING_DESK,
            watchlist: &["EURUSD", "XAUUSD"],
            ..Options::default()
        },
    )
    .await
}

#[tokio::test]
async fn the_learning_review_sends_statistics_only_and_checks_the_answer() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = learning_app(&mock, llm.clone()).await;
    let (status, body) = get(&app, "/api/architecture/learning").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["snapshot"]["gates"]["can_review"], false);
    let (status, _) = post(&app, "/api/architecture/learning/review", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "nothing resolved yet");
    assert_eq!(llm.request_count(), 0);

    // Two scanned decisions, resolved by hand.
    let (_, scan) = post(&app, "/api/architecture/scan", None).await;
    let desk = app.state.desk().unwrap();
    for id in scan["ids"].as_array().unwrap() {
        desk.decisions()
            .resolve(id.as_str().unwrap(), Some(0.6), 1)
            .await
            .unwrap();
    }
    llm.push(text_response(
        r#"{"summary": "Too little data.", "items": [
            {"type": "guard", "param": "min_council_score", "new": 0.35, "rationale": "2 decisions only.", "confidence": 0.2},
            {"type": "guard", "param": "risk_percent", "new": 5.0, "rationale": "Bigger.", "confidence": 0.9}]}"#,
    ));
    let (status, body) = post(&app, "/api/architecture/learning/review", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let review = &body["review"];
    assert_eq!(review["items"].as_array().unwrap().len(), 1, "{review}");
    assert_eq!(review["rejected"].as_array().unwrap().len(), 1);
    let item = &review["items"][0];
    assert_eq!(item["direction"], "tighten");
    assert!(item["locked"].as_str().unwrap().contains("of 30"), "{item}");

    // Only statistics went out: no symbol, no account.
    let sent = format!(
        "{:?}",
        llm.requests.lock().unwrap().last().unwrap().messages
    );
    for secret in ["EURUSD", "XAUUSD", "login", "balance"] {
        assert!(!sent.contains(secret), "{secret} was sent: {sent}");
    }
    assert!(
        audit_kinds(&app)
            .await
            .contains(&"learning_review".to_owned())
    );

    // A locked item cannot be applied.
    let (status, body) = post(
        &app,
        "/api/architecture/learning/apply",
        Some(json!({ "review_id": review["id"], "item_id": item["id"] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(desk.decisions().live().version, None, "nothing changed");
}

#[tokio::test]
async fn rolling_back_to_looser_guards_needs_a_confirmation() {
    let mock = MockBridge::start().await;
    let app = learning_app(&mock, ScriptedLlm::new("fake", vec![])).await;
    let d = app.state.desk().unwrap().decisions();
    let live = d.live();
    let v1 = d
        .save_version(&live.weights, &live.guards, "start", None, None)
        .await
        .unwrap();
    let mut tighter = live.guards.clone();
    tighter.max_open_positions = 1;
    let v2 = d
        .save_version(&live.weights, &tighter, "tighten", None, None)
        .await
        .unwrap();
    assert_eq!(d.live().guards.max_open_positions, 1);
    assert_eq!(
        app.state.desk().unwrap().rules().max_open_positions,
        1,
        "Sentinel uses the live version"
    );

    let uri = format!("/api/architecture/versions/{v1}/rollback");
    let (status, body) = post(&app, &uri, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("max_open_positions")
    );
    let (status, body) = post(&app, &uri, Some(json!({ "confirm_loosen": true }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["version"].as_i64().unwrap() > v2);
    assert_eq!(body["live"]["guards"]["max_open_positions"], 2);
    let (status, list) = get(&app, "/api/architecture/versions").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["versions"].as_array().unwrap().len(), 3);
    assert!(
        audit_kinds(&app)
            .await
            .contains(&"council_config_changed".to_owned())
    );
}

/// The Desk on a bridge whose MT5 reports account `login`.
async fn on_account(login: Option<u64>) -> (MockBridge, TestApp, Arc<ScriptedLlm>) {
    let mock = MockBridge::start().await;
    mock.update_state(|s| s.ping.login = login);
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    (mock, app, llm)
}

#[tokio::test]
async fn the_master_account_lock_blocks_pilot_on_another_account() {
    let (mock, app, llm) = on_account(Some(11_112_222)).await;
    let (status, lock) = get(&app, "/api/architecture/account-lock").await;
    assert_eq!(status, StatusCode::OK, "{lock}");
    assert_eq!(lock["expected"], 53_107_904);
    assert_eq!(lock["masked"], "••••7904");
    assert_eq!(lock["state"], "mismatch");
    // Rule 8 (ADR-0025): the reported number never reaches the browser.
    assert!(!lock.to_string().contains("11112222"), "{lock}");

    let body = gold_turn(&app, &llm).await;
    let plan_id = body["desk"]["plans"][0]["id"].as_str().unwrap().to_owned();
    let (status, refused) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different account")
    );
    assert_eq!(mock.executed_writes(), 0);
    let d = only_decision(&app).await;
    assert_eq!(d["stop_code"], "account_mismatch");

    // Nothing the browser or the model sees carries the reported number.
    for uri in [
        "/api/status",
        "/api/account",
        "/api/desk/state",
        "/api/architecture/map",
    ] {
        let (_, v) = get(&app, uri).await;
        assert!(!v.to_string().contains("11112222"), "{uri}: {v}");
    }
    for r in llm.requests.lock().unwrap().iter() {
        let sent = format!("{r:?}");
        assert!(
            !sent.contains("11112222") && !sent.contains("53107904"),
            "{sent}"
        );
    }
}

#[tokio::test]
async fn an_old_ea_leaves_the_lock_unverified_and_the_desk_does_not_trade_mt5() {
    let (_mock, app, _) = on_account(None).await;
    let (_, lock) = get(&app, "/api/architecture/account-lock").await;
    assert_eq!(lock["state"], "unverified");
    assert!(lock["message"].as_str().unwrap().contains("0.7.0"));
}

#[tokio::test]
async fn the_lock_is_changed_from_the_ui_and_checked_again_at_confirm() {
    let (mock, app, llm) = on_account(Some(11_112_222)).await;
    let uri = "/api/architecture/account-lock";
    let (status, _) = put(
        &app,
        uri,
        json!({ "expected": 11_112_222, "confirm": 11_112_223 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, lock) = put(
        &app,
        uri,
        json!({ "expected": 11_112_222, "confirm": 11_112_222 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lock}");
    assert_eq!(lock["state"], "locked");
    assert_eq!(lock["broker"], "IC Markets");
    // The audit row is masked.
    let rows = app.audit.rows(None).await.unwrap();
    let row = rows
        .iter()
        .find(|r| r.kind == "account_lock_changed")
        .unwrap();
    assert_eq!(row.payload["new"], "••••2222");
    assert!(!row.payload.to_string().contains("11112222"));

    let body = gold_turn(&app, &llm).await;
    let plan_id = body["desk"]["plans"][0]["id"].as_str().unwrap().to_owned();
    let (status, preview) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    // MT5 switches account between preview and confirm: the confirm is refused.
    mock.update_state(|s| s.ping.login = Some(99_999_999));
    let token = preview["token"].as_str().unwrap();
    let (status, body) = post(&app, &format!("/api/confirm/{token}"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(mock.executed_writes(), 0);
    let d = only_decision(&app).await;
    assert_eq!(d["stop_code"], "account_mismatch");

    // 0 turns the lock off.
    let (_, lock) = put(&app, uri, json!({ "expected": 0, "confirm": 0 })).await;
    assert_eq!(lock["state"], "off");
}

#[tokio::test]
async fn friday_never_sees_the_master_account_number_when_pilot_is_refused() {
    let (mock, app, llm) = on_account(Some(11_112_222)).await;
    let desk = app.state.desk().unwrap();
    let plan = desk
        .review_plan(
            "XAUUSD",
            "XAUUSD",
            jarvis_desk::sentinel::Side::Buy,
            Some(2380.0),
            Some(2430.0),
            None,
        )
        .await
        .unwrap();
    llm.push(tool_use_response(
        "p1",
        "ask_pilot",
        json!({ "plan_id": plan.id }),
    ));
    llm.push(text_response("Pilot could not act on that."));
    let (status, body) = post(
        &app,
        "/api/chat",
        Some(json!({ "message": "buy gold with a stop at 2380 risking 0.5%", "mode": "desk" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tool_calls"][0]["ok"], false);
    assert_eq!(mock.executed_writes(), 0);
    let requests = llm.requests.lock().unwrap();
    let sent = format!("{:?}", requests.last().unwrap().messages);
    assert!(sent.contains("different account"), "{sent}");
    for digits in ["7904", "53107904", "2222", "11112222"] {
        assert!(!sent.contains(digits), "{digits} reached the model: {sent}");
    }
}

/// A decision the council passed, waiting for Ask Friday.
async fn waiting_decision(app: &TestApp) -> String {
    use jarvis_desk::council::{CouncilRules, Weights, deliberate};
    use jarvis_desk::opinion::{AssetKind, MEMBERS, OpinionContext, Stance, missing};
    let ctx = OpinionContext {
        symbol: "XAUUSD".into(),
        kind: AssetKind::Other,
        price: Some(2400.0),
        atr: Some(10.0),
        horizon: "4h".into(),
    };
    let ops: Vec<_> = MEMBERS
        .iter()
        .map(|a| {
            let mut o = missing(*a, &ctx, "test");
            o.stance = Stance::Bull;
            o.confidence = 80;
            o
        })
        .collect();
    let verdict = deliberate(&ops, &Weights::default(), &CouncilRules::default(), vec![]);
    assert!(verdict.passed);
    app.state
        .desk()
        .unwrap()
        .decisions()
        .create(jarvis_api::decisions::NewDecision {
            session_id: None,
            symbol: "XAUUSD".into(),
            broker_symbol: Some("XAUUSD".into()),
            venue: "mt5".into(),
            trigger: jarvis_api::decisions::Trigger {
                kind: "scan".into(),
                detail: None,
            },
            price: Some(2400.0),
            atr: Some(10.0),
            opinions: ops.into_iter().map(|o| (o, None)).collect(),
            council: Some(verdict),
            early_stop: None,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_failed_ask_friday_leaves_the_decision_waiting_to_be_asked_again() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    let id = waiting_decision(&app).await;
    // The scripted model has nothing to say: the call fails.
    let (status, body) = post(&app, &format!("/api/architecture/decisions/{id}/ask"), None).await;
    assert_ne!(status, StatusCode::OK, "{body}");
    let (_, d) = get(&app, &format!("/api/architecture/decisions/{id}")).await;
    assert_eq!(d["status"], "waiting", "{d}");
    assert_eq!(d["stop_code"], "not_asked");
    // Asked again, Friday stands aside: that is recorded with her reason.
    llm.push(text_response("No edge here; I stand aside."));
    let (status, body) = post(&app, &format!("/api/architecture/decisions/{id}/ask"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["decision"]["stop_code"], "friday_stood_aside");
    assert!(
        body["decision"]["stop_detail"]
            .as_str()
            .unwrap()
            .contains("No edge")
    );
}

#[tokio::test]
async fn two_applies_at_once_both_stick_and_weights_stay_in_bounds() {
    let mock = MockBridge::start().await;
    let app = learning_app(&mock, ScriptedLlm::new("fake", vec![])).await;
    let desk = app.state.desk().unwrap().clone();
    let item = |id: &str, param: &str, old: f64, new: f64| {
        json!({ "id": id, "kind": "guard", "param": param, "old": old, "new": new,
                "direction": "tighten", "rationale": "", "evidence_refs": [],
                "confidence": 0.5, "locked": null, "applied_version": null })
    };
    let items = json!([
        item("g1", "max_open_positions", 2.0, 1.0),
        item("g2", "min_reward_risk", 1.5, 1.75)
    ]);
    sqlx::query(
        "INSERT INTO learning_reviews (id, created_at, created_ms, provider, model, input_json,
            output_text, items_json, rejected_json) VALUES ('r1', 't', 1, 'fake', 'm', '{}', '', ?, '[]')",
    )
    .bind(items.to_string())
    .execute(desk.decisions().pool())
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        jarvis_api::learning::apply(&desk, "r1", "g1", false, None),
        jarvis_api::learning::apply(&desk, "r1", "g2", false, None)
    );
    a.unwrap();
    b.unwrap();
    let live = desk.decisions().live();
    assert_eq!(live.guards.max_open_positions, 1, "{live:?}");
    assert!(
        (live.guards.min_reward_risk - 1.75).abs() < 1e-9,
        "{live:?}"
    );
    let reviews = jarvis_api::learning::recent(&desk, 1).await.unwrap();
    assert!(reviews[0].items.iter().all(|i| i.applied_version.is_some()));

    // Weights out of bounds are refused on every save, whoever builds them.
    let mut lopsided = jarvis_desk::council::Weights::default();
    lopsided.0.insert(jarvis_desk::AgentId::Oracle, 5.0);
    assert!(
        desk.decisions()
            .save_version(&lopsided, &live.guards, "test", None, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn sending_the_same_plan_to_pilot_twice_does_not_stop_the_previewed_decision() {
    let mock = MockBridge::start().await;
    let llm = ScriptedLlm::new("fake", vec![]);
    let app = desk_app(&mock, llm.clone()).await;
    let body = gold_turn(&app, &llm).await;
    let plan_id = body["desk"]["plans"][0]["id"].as_str().unwrap().to_owned();
    let (status, _) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post(&app, &format!("/api/desk/plans/{plan_id}/pilot"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let d = only_decision(&app).await;
    assert_eq!(d["status"], "open", "{d}");
    assert!(d["stop_code"].is_null(), "{d}");
}

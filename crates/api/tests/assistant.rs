//! Assistant mode (tasks/spec-assistant-mode.md): its own tools, no MT5 access, 3D models,
//! "remember that ...", the monthly Claude cap, refusals and the status route.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::*;
use jarvis_api::models3d::BUILDER_PROMPT;
use jarvis_llm::{ChatResponse, LlmError, Message, StopReason, Usage};
use serde_json::{Value, json};

const CUBE_BUILD: &str =
    "TITLE: Red cube\n```javascript\nscene.add(new THREE.Mesh(new THREE.BoxGeometry(2,2,2)));\n```";

fn with_usage(mut r: ChatResponse, input: u32, output: u32) -> ChatResponse {
    r.usage = Some(Usage {
        input_tokens: input,
        output_tokens: output,
        ..Usage::default()
    });
    r
}

fn reply(text: &str) -> Result<ChatResponse, LlmError> {
    Ok(with_usage(text_response(text), 1000, 100))
}

fn call(id: &str, name: &str, args: Value) -> Result<ChatResponse, LlmError> {
    Ok(with_usage(tool_use_response(id, name, args), 1000, 50))
}

async fn assistant_app(
    claude: Arc<ScriptedLlm>,
    notes: Option<std::path::PathBuf>,
    extra: &'static str,
) -> TestApp {
    // `fake` (the trading brain) must never be asked anything in these tests.
    let trading = ScriptedLlm::new("fake", vec![]);
    build_app(
        DEAD_ENDPOINT,
        trading,
        Options {
            claude_llm: Some(claude),
            notes,
            assistant: extra,
            ..Options::default()
        },
    )
    .await
}

async fn ask(app: &TestApp, message: &str, session: Option<&str>) -> (StatusCode, Value) {
    let mut body = json!({ "message": message, "mode": "assistant" });
    if let Some(s) = session {
        body["session_id"] = json!(s);
    }
    request(&app.router, "POST", "/api/chat", Some(body)).await
}

fn notes_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jarvis-anotes-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("plans")).unwrap();
    std::fs::write(
        dir.join("plans/gold.md"),
        "# Gold news-day rule\nNo new gold trades 30 minutes either side of CPI.",
    )
    .unwrap();
    dir
}

fn tool_names(req: &jarvis_llm::ChatRequest) -> Vec<String> {
    req.tools.iter().map(|t| t.name.clone()).collect()
}

#[tokio::test]
async fn an_assistant_turn_uses_the_butler_and_only_the_assistant_tools() {
    let claude = ScriptedLlm::new("claude", vec![reply("Good evening, sir.")]);
    let app = assistant_app(claude.clone(), Some(notes_dir()), "").await;
    let (status, body) = ask(&app, "hello", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reply"], "Good evening, sir.");
    assert_eq!(body["mode"], "assistant");
    assert_eq!(body["persona"], "jarvis");
    assert_eq!(
        body["brain"],
        json!({"provider": "claude", "model": "claude-test"})
    );
    assert!(body["spend"]["spent_usd"].as_f64().unwrap() > 0.0);

    let req = claude.requests.lock().unwrap()[0].clone();
    assert_eq!(
        tool_names(&req),
        [
            "search_notes",
            "remember_note",
            "build_3d_model",
            "list_3d_models"
        ]
    );
    assert!(req.system.as_deref().unwrap().contains("British butler"));
    assert_eq!(req.effort.as_deref(), Some("low"));
    assert_eq!(app.llm.request_count(), 0, "the trading brain is not asked");
}

#[tokio::test]
async fn without_notes_only_the_model_tools_are_offered() {
    let claude = ScriptedLlm::new("claude", vec![reply("Very good.")]);
    let app = assistant_app(claude.clone(), None, "").await;
    ask(&app, "hello", None).await;
    let req = claude.requests.lock().unwrap()[0].clone();
    assert_eq!(tool_names(&req), ["build_3d_model", "list_3d_models"]);
}

#[tokio::test]
async fn a_trading_tool_call_is_refused_in_assistant_mode() {
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call(
                "t1",
                "propose_order",
                json!({"symbol": "XAUUSD", "side": "buy", "volume": 1.0}),
            ),
            reply("I cannot trade in this mode, sir."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "").await;
    let (status, body) = ask(&app, "buy 1 lot of gold", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["preview"].is_null());
    assert_eq!(body["tool_calls"][0]["ok"], false);
    let second = claude.requests.lock().unwrap()[1].clone();
    let Some(Message::ToolResult {
        content, is_error, ..
    }) = second.messages.last()
    else {
        panic!("no tool result")
    };
    assert!(*is_error);
    assert!(content.contains("Trading"), "{content}");
    let kinds = audit_kinds(&app).await;
    assert!(
        !kinds
            .iter()
            .any(|k| k == "preview" || k == "bridge_request")
    );
}

#[tokio::test]
async fn without_a_key_the_reply_says_so_and_nothing_is_sent() {
    let trading = ScriptedLlm::new("fake", vec![]);
    let app = build_app(DEAD_ENDPOINT, trading, Options::default()).await;
    let (status, body) = ask(&app, "hello", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["reply"]
            .as_str()
            .unwrap()
            .contains("ANTHROPIC_API_KEY"),
        "{body}"
    );
    assert_eq!(app.llm.request_count(), 0);

    let (_, s) = request(&app.router, "GET", "/api/assistant/status", None).await;
    assert_eq!(s["enabled"], true);
    assert_eq!(s["available"], false);
    assert!(s["reason"].as_str().unwrap().contains("key"));
}

#[tokio::test]
async fn building_a_model_saves_it_and_returns_it() {
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call("b1", "build_3d_model", json!({"description": "a red cube"})),
            Ok(with_usage(text_response(CUBE_BUILD), 2000, 3000)),
            reply("A red cube, sir, perfectly square."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "").await;
    let (status, body) = ask(&app, "show me a 3D red cube", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["models"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(body["models"][0]["title"], "Red cube");

    let reqs = claude.requests.lock().unwrap().clone();
    let build = &reqs[1];
    assert_eq!(build.system.as_deref(), Some(BUILDER_PROMPT));
    assert_eq!(build.effort.as_deref(), Some("medium"));
    assert!(build.tools.is_empty());

    let (status, list) = request(&app.router, "GET", "/api/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["models"][0]["id"], id.as_str());
    let (status, model) = request(&app.router, "GET", &format!("/api/models/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(model["code"].as_str().unwrap().contains("BoxGeometry"));
    let (status, _) = request(&app.router, "GET", "/api/models/../../etc", None).await;
    assert_ne!(status, StatusCode::OK);
    let (status, _) = request(&app.router, "GET", "/api/models/aaaaaaaaaaaa", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let kinds = audit_kinds(&app).await;
    assert!(kinds.iter().any(|k| k == "model_built"));
    assert_eq!(kinds.iter().filter(|k| *k == "claude_usage").count(), 3);
}

#[tokio::test]
async fn changing_a_model_sends_its_code_back() {
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call("b1", "build_3d_model", json!({"description": "a cube"})),
            Ok(with_usage(text_response(CUBE_BUILD), 10, 10)),
            reply("Done, sir."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "").await;
    let (_, first) = ask(&app, "show me a cube", None).await;
    let id = first["models"][0]["id"].as_str().unwrap().to_owned();
    let session = first["session_id"].as_str().unwrap().to_owned();

    // A fresh script for the second turn.
    let claude2 = ScriptedLlm::new(
        "claude",
        vec![
            call(
                "b2",
                "build_3d_model",
                json!({"description": "make it blue", "previous_id": id}),
            ),
            Ok(with_usage(
                text_response("TITLE: Blue cube\n```js\nscene.add(new THREE.Group());\n```"),
                10,
                10,
            )),
            reply("Blue it is."),
        ],
    );
    // The first script is used up: run the second turn on the same assistant with a new one.
    let assistant = app.state.assistant().unwrap().clone();
    let out = assistant
        .run_turn(
            claude2.as_ref(),
            Some(uuid::Uuid::parse_str(&session).unwrap()),
            "make it blue",
            false,
        )
        .await
        .unwrap();
    assert_eq!(out.models[0].title, "Blue cube");
    let build = claude2.requests.lock().unwrap()[1].clone();
    let Message::User { text } = &build.messages[0] else {
        panic!()
    };
    assert!(
        text.contains("BoxGeometry") && text.contains("make it blue"),
        "{text}"
    );

    // An unknown previous id is an error the model can recover from.
    let claude3 = ScriptedLlm::new(
        "claude",
        vec![
            call(
                "b3",
                "build_3d_model",
                json!({"description": "x", "previous_id": "ffffffffffff"}),
            ),
            reply("I could not find it."),
        ],
    );
    let out = assistant
        .run_turn(claude3.as_ref(), None, "change it", false)
        .await
        .unwrap();
    assert!(out.models.is_empty());
    assert_eq!(
        claude3.request_count(),
        2,
        "no builder call for an unknown model"
    );
}

#[tokio::test]
async fn bad_builder_output_is_reported_not_saved() {
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call("b1", "build_3d_model", json!({"description": "a cube"})),
            Ok(with_usage(
                text_response("I would rather not write code."),
                10,
                10,
            )),
            reply("I am afraid the model did not come out, sir."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "").await;
    let (_, body) = ask(&app, "show me a cube", None).await;
    assert_eq!(body["models"], json!([]));
    assert_eq!(body["tool_calls"][0]["ok"], false);
    let (_, list) = request(&app.router, "GET", "/api/models", None).await;
    assert_eq!(list["models"], json!([]));
}

#[tokio::test]
async fn remember_that_writes_a_note_that_the_next_search_finds() {
    let dir = notes_dir();
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call(
                "r1",
                "remember_note",
                json!({"text": "my broker closes gold at 23:55"}),
            ),
            reply("Noted, sir: My broker closes gold at."),
            call("s1", "search_notes", json!({"query": "broker closes gold"})),
            reply("Your note says 23:55."),
        ],
    );
    let app = assistant_app(claude.clone(), Some(dir.clone()), "").await;
    let (status, body) = ask(&app, "remember that my broker closes gold at 23:55", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let path = body["captured"][0]["path"].as_str().unwrap().to_owned();
    assert!(path.starts_with("captures/"), "{path}");
    assert!(dir.join(&path).exists());
    assert_eq!(body["notes"][0]["id"], body["captured"][0]["id"]);

    let (_, body) = ask(&app, "when does my broker close gold?", None).await;
    let result = claude.requests.lock().unwrap()[3].clone();
    let Some(Message::ToolResult { content, .. }) = result.messages.last() else {
        panic!()
    };
    assert!(content.contains("23:55"), "{content}");
    assert_eq!(body["reply"], "Your note says 23:55.");
    assert!(audit_kinds(&app).await.iter().any(|k| k == "note_captured"));
}

#[tokio::test]
async fn the_capture_route_writes_a_note() {
    let dir = notes_dir();
    let app = assistant_app(ScriptedLlm::new("claude", vec![]), Some(dir.clone()), "").await;
    let (status, body) = request(
        &app.router,
        "POST",
        "/api/notes/capture",
        Some(json!({"text": "the finish window should be 900 ms"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(dir.join(body["path"].as_str().unwrap()).exists());
    let (status, _) = request(
        &app.router,
        "POST",
        "/api/notes/capture",
        Some(json!({"text": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_monthly_cap_stops_requests_before_they_are_sent() {
    // 8,000 output tokens at $20 per million (+ input) = $0.164 of a $0.20 cap. The next chat
    // request could cost up to about $0.08 more, so it is not sent.
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            Ok(with_usage(text_response("First."), 1000, 8000)),
            reply("Never sent."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "monthly_budget_usd = 0.2").await;
    let (_, first) = ask(&app, "hello", None).await;
    assert_eq!(first["reply"], "First.");
    assert_eq!(first["spend"]["warn"], true);
    assert!((first["spend"]["spent_usd"].as_f64().unwrap() - 0.164).abs() < 1e-9);

    let (status, second) = ask(&app, "hello again", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        second["reply"].as_str().unwrap().contains("budget"),
        "{second}"
    );
    assert_eq!(
        claude.request_count(),
        1,
        "nothing is sent once the cap could be passed"
    );

    let (_, s) = request(&app.router, "GET", "/api/assistant/status", None).await;
    assert_eq!(s["available"], true);
    assert_eq!(s["spend"]["warn"], true);
}

#[tokio::test]
async fn a_build_is_not_started_when_it_could_pass_the_cap() {
    // The chat request fits under $0.30; the build (16,000 output tokens = $0.32 + input) does not.
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call("b1", "build_3d_model", json!({"description": "a cube"})),
            reply("I am afraid the budget does not stretch to it, sir."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "monthly_budget_usd = 0.3").await;
    let (_, body) = ask(&app, "show me a cube", None).await;
    assert_eq!(body["models"], json!([]));
    assert_eq!(claude.request_count(), 2, "no builder request");
    let result = claude.requests.lock().unwrap()[1].clone();
    let Some(Message::ToolResult {
        content, is_error, ..
    }) = result.messages.last()
    else {
        panic!()
    };
    assert!(*is_error && content.contains("budget"), "{content}");
}

#[tokio::test]
async fn one_build_per_message() {
    let two = ChatResponse {
        text: None,
        tool_calls: vec![
            jarvis_llm::ToolCall {
                id: "b1".into(),
                name: "build_3d_model".into(),
                arguments: json!({"description": "a cube"}),
            },
            jarvis_llm::ToolCall {
                id: "b2".into(),
                name: "build_3d_model".into(),
                arguments: json!({"description": "a sphere"}),
            },
        ],
        stop_reason: StopReason::ToolUse,
        usage: Some(Usage::default()),
    };
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            Ok(two),
            Ok(with_usage(text_response(CUBE_BUILD), 10, 10)),
            reply("One at a time, sir."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "").await;
    let (_, body) = ask(&app, "show me a cube and a sphere", None).await;
    assert_eq!(body["models"].as_array().unwrap().len(), 1);
    assert_eq!(body["tool_calls"][1]["ok"], false);
    assert_eq!(claude.request_count(), 3, "one builder request only");
}

#[tokio::test]
async fn a_failure_after_a_tool_ran_keeps_what_was_done() {
    let dir = notes_dir();
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call("r1", "remember_note", json!({"text": "buy milk"})),
            Err(LlmError::Timeout),
        ],
    );
    let app = assistant_app(claude, Some(dir.clone()), "").await;
    let (status, body) = ask(&app, "remember that buy milk", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["reply"]
            .as_str()
            .unwrap()
            .contains("stopped answering")
    );
    assert_eq!(body["captured"].as_array().unwrap().len(), 1);
    // The timed-out request stays counted at its worst case.
    assert!(body["spend"]["spent_usd"].as_f64().unwrap() > 0.08);
}

#[tokio::test]
async fn a_first_request_failure_is_an_error() {
    let claude = ScriptedLlm::new(
        "claude",
        vec![Err(LlmError::Api {
            status: 401,
            message: "invalid x-api-key".into(),
        })],
    );
    let app = assistant_app(claude, None, "").await;
    let (status, _) = ask(&app, "hello", None).await;
    assert_ne!(status, StatusCode::OK);
    // A refused request was not billed, so nothing is counted.
    let (_, s) = request(&app.router, "GET", "/api/assistant/status", None).await;
    assert_eq!(s["spend"]["spent_usd"], 0.0);
}

#[tokio::test]
async fn only_a_claude_provider_can_run_assistant_mode() {
    let app = assistant_app(
        ScriptedLlm::new("claude", vec![]),
        None,
        "provider = \"fake\"",
    )
    .await;
    let (_, s) = request(&app.router, "GET", "/api/assistant/status", None).await;
    assert_eq!(s["available"], false);
    assert!(s["reason"].as_str().unwrap().contains("anthropic"), "{s}");
    let (_, body) = ask(&app, "hello", None).await;
    assert!(body["reply"].as_str().unwrap().contains("not set up"));
    assert_eq!(app.llm.request_count(), 0, "the local model is never asked");
}

#[tokio::test]
async fn assistant_prompts_are_cached_and_trading_prompts_are_not() {
    let claude = ScriptedLlm::new(
        "claude",
        vec![
            call("b1", "build_3d_model", json!({"description": "a cube"})),
            Ok(with_usage(text_response(CUBE_BUILD), 10, 10)),
            reply("Done."),
        ],
    );
    let app = assistant_app(claude.clone(), None, "").await;
    ask(&app, "show me a cube", None).await;
    assert!(
        claude
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.cache_system)
    );

    let trading = ScriptedLlm::new("fake", vec![text("Hi.")]);
    let app = build_app(DEAD_ENDPOINT, trading.clone(), Options::default()).await;
    chat(&app, "hello").await;
    assert!(
        !trading.requests.lock().unwrap()[0].cache_system,
        "the trading prompt has a clock"
    );
}

#[tokio::test]
async fn a_refusal_gets_a_plain_reply() {
    let refusal = ChatResponse {
        text: None,
        tool_calls: vec![],
        stop_reason: StopReason::Refusal,
        usage: None,
    };
    let claude = ScriptedLlm::new("claude", vec![Ok(refusal)]);
    let app = assistant_app(claude, None, "").await;
    let (_, body) = ask(&app, "something", None).await;
    assert_eq!(body["reply"], jarvis_api::assistant::REFUSAL_REPLY);
}

#[tokio::test]
async fn an_unknown_mode_is_a_bad_request_and_trading_is_unchanged() {
    let claude = ScriptedLlm::new("claude", vec![]);
    let app = assistant_app(claude, None, "").await;
    let (status, _) = request(
        &app.router,
        "POST",
        "/api/chat",
        Some(json!({"message": "hi", "mode": "party"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let trading = ScriptedLlm::new("fake", vec![text("Trading here.")]);
    let app = build_app(DEAD_ENDPOINT, trading, Options::default()).await;
    let (status, body) = chat(&app, "hello").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "trading");
    assert_eq!(body["models"], json!([]));
    assert_eq!(body["brain"]["provider"], "fake");
}

#[tokio::test]
async fn assistant_mode_can_be_turned_off() {
    let app = assistant_app(ScriptedLlm::new("claude", vec![]), None, "enabled = false").await;
    let (status, _) = ask(&app, "hello", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, s) = request(&app.router, "GET", "/api/assistant/status", None).await;
    assert_eq!(s["enabled"], false);
}

#[tokio::test]
async fn a_refusal_in_trading_mode_gets_a_plain_line_too() {
    let refusal = ChatResponse {
        text: None,
        tool_calls: vec![],
        stop_reason: StopReason::Refusal,
        usage: None,
    };
    let trading = ScriptedLlm::new("fake", vec![Ok(refusal)]);
    let app = build_app(DEAD_ENDPOINT, trading, Options::default()).await;
    let (_, body) = chat(&app, "something").await;
    assert!(
        body["reply"].as_str().unwrap().contains("declined"),
        "{body}"
    );
}

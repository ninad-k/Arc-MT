//! Uploads (tasks/spec-model-uploads.md): 3D files, reliefs and images Claude builds from.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use http_body_util::BodyExt;
use jarvis_llm::{LlmError, Message, Usage};
use serde_json::{Value, json};
use tower::ServiceExt;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR fake image body";

fn glb() -> Vec<u8> {
    let mut b = b"glTF".to_vec();
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&20u32.to_le_bytes());
    b.extend_from_slice(&[0; 8]);
    b
}

async fn send(app: &TestApp, method: &str, uri: &str, body: Vec<u8>) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/octet-stream")
        .body(Body::from(body))
        .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    (
        status,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

async fn app_with(claude: Option<Arc<ScriptedLlm>>) -> TestApp {
    build_app(
        DEAD_ENDPOINT,
        ScriptedLlm::new("fake", vec![]),
        Options {
            claude_llm: claude,
            ..Options::default()
        },
    )
    .await
}

#[tokio::test]
async fn a_3d_file_is_stored_as_it_is_and_listed() {
    let app = app_with(None).await;
    let (status, body) = send(&app, "POST", "/api/models/upload?name=red_car.glb", glb()).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let v = json_of(&body);
    assert_eq!(v["kind"], "file");
    assert_eq!(v["format"], "glb");
    assert_eq!(v["title"], "red car");
    let id = v["id"].as_str().unwrap().to_owned();

    let (status, bytes) = send(&app, "GET", &format!("/api/models/{id}/file"), vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, glb(), "the exact bytes come back");
    let (_, list) = request(&app.router, "GET", "/api/models", None).await;
    assert_eq!(list["models"][0]["kind"], "file");
    let (_, model) = request(&app.router, "GET", &format!("/api/models/{id}"), None).await;
    assert_eq!(model["code"], "");
    assert!(audit_kinds(&app).await.iter().any(|k| k == "model_built"));
}

#[tokio::test]
async fn bad_uploads_are_refused() {
    let app = app_with(None).await;
    for (uri, body) in [
        ("/api/models/upload?name=setup.exe", b"MZ\x90\x00".to_vec()),
        (
            "/api/models/upload?name=fake.glb",
            b"MZ not a model".to_vec(),
        ),
        ("/api/models/upload?name=empty.obj", vec![]),
        (
            "/api/models/upload?name=scene.gltf",
            br#"{"asset":{},"buffers":[{"uri":"https://example.com/x.bin"}]}"#.to_vec(),
        ),
        ("/api/models/relief?name=x.gif", b"GIF89a....".to_vec()),
        ("/api/models/from-image", b"<svg onload=alert(1)/>".to_vec()),
    ] {
        let (status, body) = send(&app, "POST", uri, body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{uri}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    let too_big = vec![b'v'; jarvis_api::models3d::MAX_FILE_BYTES + 4096];
    let (status, _) = send(&app, "POST", "/api/models/upload?name=big.obj", too_big).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let (_, list) = request(&app.router, "GET", "/api/models", None).await;
    assert_eq!(list["models"], json!([]));
}

#[tokio::test]
async fn a_picture_becomes_a_relief_for_free() {
    let app = app_with(None).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/models/relief?name=holiday.png",
        PNG.to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_of(&body);
    assert_eq!(
        (v["kind"].as_str(), v["format"].as_str()),
        (Some("relief"), Some("png"))
    );
    let (_, s) = request(&app.router, "GET", "/api/assistant/status", None).await;
    assert_eq!(s["spend"]["spent_usd"], 0.0);
}

#[tokio::test]
async fn claude_builds_from_an_image_and_the_image_is_not_kept() {
    let mut reply =
        text_response("TITLE: 1 BHK flat\n```javascript\nscene.add(new THREE.Group());\n```");
    reply.usage = Some(Usage {
        input_tokens: 2000,
        output_tokens: 6000,
        ..Usage::default()
    });
    let claude = ScriptedLlm::new("claude", vec![Ok(reply)]);
    let app = app_with(Some(claude.clone())).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/models/from-image?note=1%20BHK%2C%20walls%203%20m",
        PNG.to_vec(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v = json_of(&body);
    assert_eq!(v["model"]["title"], "1 BHK flat", "{v}");
    assert!(v["spend"]["spent_usd"].as_f64().unwrap() > 0.1);

    let req = claude.requests.lock().unwrap()[0].clone();
    let Message::UserImage {
        text,
        media_type,
        data_base64,
    } = &req.messages[0]
    else {
        panic!("no image sent")
    };
    assert!(text.contains("1 BHK, walls 3 m"));
    assert_eq!(media_type, "image/png");
    use base64::Engine;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(data_base64)
            .unwrap(),
        PNG
    );
    assert_eq!(req.effort.as_deref(), Some("medium"));

    let id = v["model"]["id"].as_str().unwrap();
    let (_, model) = request(&app.router, "GET", &format!("/api/models/{id}"), None).await;
    assert_eq!(model["kind"], "code");
    let (status, _) = send(&app, "GET", &format!("/api/models/{id}/file"), vec![]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the picture is not kept");
}

#[tokio::test]
async fn an_image_build_reports_failures_in_plain_words() {
    // No key: nothing is sent.
    let app = app_with(None).await;
    let (_, body) = send(&app, "POST", "/api/models/from-image", PNG.to_vec()).await;
    let v = json_of(&body);
    assert!(v["model"].is_null());
    assert!(v["message"].as_str().unwrap().contains("ANTHROPIC_API_KEY"));

    // Claude answers without code, or fails.
    let claude = ScriptedLlm::new(
        "claude",
        vec![text("I cannot make out the plan."), Err(LlmError::Timeout)],
    );
    let app = app_with(Some(claude)).await;
    let (_, body) = send(&app, "POST", "/api/models/from-image", PNG.to_vec()).await;
    assert!(
        json_of(&body)["message"]
            .as_str()
            .unwrap()
            .contains("could not be built")
    );
    let (_, body) = send(&app, "POST", "/api/models/from-image", PNG.to_vec()).await;
    let v = json_of(&body);
    assert!(v["message"].as_str().unwrap().contains("timed out"), "{v}");
    let (_, list) = request(&app.router, "GET", "/api/models", None).await;
    assert_eq!(list["models"], json!([]));
}

#[tokio::test]
async fn an_image_build_respects_the_cap() {
    let claude = ScriptedLlm::new("claude", vec![]);
    let app = build_app(
        DEAD_ENDPOINT,
        ScriptedLlm::new("fake", vec![]),
        Options {
            claude_llm: Some(claude.clone()),
            assistant: "monthly_budget_usd = 0.1",
            ..Options::default()
        },
    )
    .await;
    let (_, body) = send(&app, "POST", "/api/models/from-image", PNG.to_vec()).await;
    assert!(
        json_of(&body)["message"]
            .as_str()
            .unwrap()
            .contains("budget")
    );
    assert_eq!(claude.request_count(), 0);
}

#[tokio::test]
async fn changing_an_uploaded_file_adds_an_overlay_on_a_copy() {
    let app = app_with(None).await;
    let (_, body) = send(&app, "POST", "/api/models/upload?name=car.glb", glb()).await;
    let id = json_of(&body)["id"].as_str().unwrap().to_owned();
    let (status, _) = request(
        &app.router,
        "PUT",
        &format!("/api/models/{id}/info"),
        Some(json!({"parts": ["Body", "Wheel_FL"], "size": [4.2, 1.4, 1.8]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request(
        &app.router,
        "PUT",
        &format!("/api/models/{id}/info"),
        Some(json!({"parts": [], "size": [1.0, -2.0, 1.0]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let claude = ScriptedLlm::new(
        "claude",
        vec![
            tool_use(
                "b1",
                "build_3d_model",
                json!({"description": "make the body red", "previous_id": id}),
            ),
            text("TITLE: Red car\n```javascript\nhelpers.model.traverse(o => {});\n```"),
            text("Red it is, sir."),
        ],
    );
    let assistant = app.state.assistant().unwrap().clone();
    let out = assistant
        .run_turn(claude.as_ref(), None, "make the body red", false)
        .await
        .unwrap();
    let new_id = out.models[0].id.clone();
    let build = claude.requests.lock().unwrap()[1].clone();
    assert_eq!(
        build.system.as_deref(),
        Some(jarvis_api::models3d::OVERLAY_PROMPT)
    );
    let Message::User { text } = &build.messages[0] else {
        panic!()
    };
    assert!(
        text.contains("Wheel_FL") && text.contains("make the body red"),
        "{text}"
    );

    let (_, model) = request(&app.router, "GET", &format!("/api/models/{new_id}"), None).await;
    assert_eq!(model["kind"], "file");
    assert_eq!(model["previous_id"], id.as_str());
    assert!(model["overlay"].as_str().unwrap().contains("helpers.model"));
    let (_, bytes) = send(&app, "GET", &format!("/api/models/{new_id}/file"), vec![]).await;
    assert_eq!(bytes, glb(), "the change keeps the uploaded file");
}

#[tokio::test]
async fn uploads_need_assistant_mode_on() {
    let app = build_app(
        DEAD_ENDPOINT,
        ScriptedLlm::new("fake", vec![]),
        Options {
            assistant: "enabled = false",
            ..Options::default()
        },
    )
    .await;
    let (status, _) = send(&app, "POST", "/api/models/upload?name=a.glb", glb()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn claude_is_told_which_model_is_on_screen() {
    let claude = ScriptedLlm::new("claude", vec![text("Red it shall be, sir.")]);
    let app = app_with(Some(claude.clone())).await;
    let (_, body) = send(&app, "POST", "/api/models/upload?name=my_car.glb", glb()).await;
    let id = json_of(&body)["id"].as_str().unwrap().to_owned();
    let (status, _) = request(
        &app.router,
        "POST",
        "/api/chat",
        Some(json!({"message": "make it red", "mode": "assistant", "stage_model": {"id": id}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let req = claude.requests.lock().unwrap()[0].clone();
    let Some(Message::User { text: sent }) = req.messages.last() else {
        panic!()
    };
    assert!(sent.starts_with("make it red"), "{sent}");
    assert!(
        sent.contains(&id) && sent.contains("my car") && sent.contains("uploaded 3D file"),
        "{sent}"
    );

    // An unknown or malformed id adds nothing.
    let claude = ScriptedLlm::new("claude", vec![text("Hm."), text("Hm.")]);
    let app = app_with(Some(claude.clone())).await;
    for stage in [json!({"id": "ffffffffffff"}), json!({"id": "../../etc"})] {
        request(
            &app.router,
            "POST",
            "/api/chat",
            Some(json!({"message": "make it red", "mode": "assistant", "stage_model": stage})),
        )
        .await;
    }
    for req in claude.requests.lock().unwrap().iter() {
        let Some(Message::User { text: sent }) = req.messages.last() else {
            panic!()
        };
        assert_eq!(sent, "make it red");
    }
}

#[tokio::test]
async fn an_upload_larger_than_a_json_body_is_accepted() {
    let app = app_with(None).await;
    let mut obj = b"o Big\n".to_vec();
    while obj.len() < 4 * jarvis_api::routes::JSON_BODY_LIMIT {
        obj.extend_from_slice(b"v 0.123456 1.234567 2.345678\n");
    }
    obj.extend_from_slice(b"f 1 2 3\n");
    let (status, body) = send(&app, "POST", "/api/models/upload?name=big.obj", obj).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn a_picture_too_large_for_claude_is_refused_before_sending() {
    let claude = ScriptedLlm::new("claude", vec![]);
    let app = app_with(Some(claude.clone())).await;
    let mut png = PNG.to_vec();
    png.resize(jarvis_api::models3d::MAX_CLAUDE_IMAGE_BYTES + 10, 0);
    let (status, body) = send(&app, "POST", "/api/models/from-image", png.clone()).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(claude.request_count(), 0);
    // The same picture is fine for a free relief (5 MB).
    let (status, _) = send(&app, "POST", "/api/models/relief?name=big.png", png).await;
    assert_eq!(status, StatusCode::OK);
}

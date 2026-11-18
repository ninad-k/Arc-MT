//! Voice routes with fake engines (the real ones need models; see the real-run notes).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use common::*;
use jarvis_api::voice::Voice;
use jarvis_bridge_client::mock::MockBridge;
use jarvis_voice::{SpeechToText, TextToSpeech, Transcript, VoiceError};
use serde_json::{Value, json};

/// Says whatever it was told to, and records how many calls ran at the same time.
struct FakeStt {
    text: String,
    delay: Duration,
    active: AtomicUsize,
    max_active: AtomicUsize,
    calls: AtomicUsize,
    last_len: AtomicUsize,
}

impl FakeStt {
    fn new(text: &str, delay_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            text: text.to_owned(),
            delay: Duration::from_millis(delay_ms),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            last_len: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl SpeechToText for FakeStt {
    async fn transcribe(&self, samples: Vec<f32>) -> Result<Transcript, VoiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.last_len.store(samples.len(), Ordering::SeqCst);
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(Transcript {
            text: self.text.clone(),
            raw: format!("raw {}", self.text),
            audio_ms: (samples.len() / 16) as u64,
            elapsed_ms: 7,
        })
    }
}

struct FakeTts;

#[async_trait]
impl TextToSpeech for FakeTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        let mut wav = b"RIFFxxxxWAVE".to_vec();
        wav.extend_from_slice(text.as_bytes());
        Ok(wav)
    }
}

/// A 16-bit mono PCM WAV of `seconds` of silence at 16 kHz.
fn wav(seconds: f32) -> Vec<u8> {
    let samples = (16_000.0 * seconds) as u32;
    let data_len = samples * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(out.len() + data_len as usize, 0);
    out
}

async fn voice_app(voice: Option<Voice>, auth: bool) -> (TestApp, MockBridge) {
    let mock = MockBridge::start().await;
    let app = build_app(
        &mock,
        ScriptedLlm::new("fake", vec![]),
        Options {
            voice,
            auth,
            ..Options::default()
        },
    )
    .await;
    (app, mock)
}

async fn transcribe(
    app: &TestApp,
    content_type: &str,
    body: Vec<u8>,
    cookie: Option<&str>,
) -> (StatusCode, Value) {
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let mut req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/voice/transcribe")
        .header("content-type", content_type);
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    let response = app
        .router
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn both_routes_say_503_when_voice_is_off() {
    let (app, _mock) = voice_app(None, false).await;
    let (status, body) = transcribe(&app, "audio/wav", wav(0.5), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"]["code"], "voice_unavailable");
    let (status, body) = request(
        &app.router,
        "POST",
        "/api/voice/speak",
        Some(json!({ "text": "hi" })),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"]["code"], "voice_unavailable");
    let (_, status_body) = request(&app.router, "GET", "/api/status", None).await;
    assert_eq!(status_body["voice"]["stt"], false);
    assert_eq!(status_body["voice"]["tts"], false);
}

#[tokio::test]
async fn a_half_configured_voice_reports_each_engine() {
    let stt = FakeStt::new("hello", 0);
    let (app, _mock) = voice_app(Some(Voice::new(Some(stt), None, 30.0)), false).await;
    let (_, status_body) = request(&app.router, "GET", "/api/status", None).await;
    assert_eq!(status_body["voice"]["stt"], true);
    assert_eq!(status_body["voice"]["tts"], false);
    assert_eq!(status_body["voice"]["tts_engine"], "piper");
    let (status, _) = request(
        &app.router,
        "POST",
        "/api/voice/speak",
        Some(json!({ "text": "hi" })),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn transcribe_returns_text_and_intent_and_audits_text_only() {
    let stt = FakeStt::new("yes confirm", 0);
    let voice = Voice::new(Some(stt.clone()), Some(Arc::new(FakeTts)), 30.0);
    let (app, _mock) = voice_app(Some(voice), false).await;
    let audio = wav(1.0);
    let (status, body) = transcribe(&app, "audio/wav", audio.clone(), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["text"], "yes confirm");
    assert_eq!(body["raw"], "raw yes confirm");
    assert_eq!(body["intent"], "confirm");
    assert_eq!(body["audio_ms"], 1000);
    assert!(body["elapsed_ms"].is_u64());
    assert_eq!(
        stt.last_len.load(Ordering::SeqCst),
        16_000,
        "decoded to 16 kHz mono samples"
    );

    let rows = app.audit.rows(None).await.unwrap();
    let row = rows
        .iter()
        .find(|r| r.kind == "voice_stt")
        .expect("voice_stt row");
    assert_eq!(row.payload["text"], "yes confirm");
    assert_eq!(row.payload["intent"], "confirm");
    let dump = row.payload.to_string();
    assert!(
        dump.len() < 400,
        "the audio is not stored: {} bytes of audit",
        dump.len()
    );

    for (said, intent) in [
        ("cancel", "cancel"),
        ("what is gold doing", "other"),
        ("nahi", "cancel"),
    ] {
        let stt = FakeStt::new(said, 0);
        let voice = Voice::new(Some(stt), None, 30.0);
        let (app, _mock) = voice_app(Some(voice), false).await;
        let (_, body) = transcribe(&app, "audio/wav; charset=binary", wav(0.2), None).await;
        assert_eq!(body["intent"], intent, "{said}");
    }
}

#[tokio::test]
async fn bad_audio_is_a_400_and_never_reaches_the_engine() {
    let stt = FakeStt::new("x", 0);
    let (app, _mock) = voice_app(Some(Voice::new(Some(stt.clone()), None, 2.0)), false).await;
    for (label, content_type, body) in [
        ("wrong content type", "application/json", wav(0.2)),
        ("empty", "audio/wav", vec![]),
        (
            "not a wav",
            "audio/wav",
            b"this is not audio at all".to_vec(),
        ),
        ("too long", "audio/wav", wav(3.0)),
    ] {
        let (status, body) = transcribe(&app, content_type, body, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {body}");
        assert_eq!(body["error"]["code"], "bad_request", "{label}");
    }
    assert_eq!(stt.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn uploads_over_five_megabytes_are_413() {
    let stt = FakeStt::new("x", 0);
    let (app, _mock) = voice_app(Some(Voice::new(Some(stt.clone()), None, 30.0)), false).await;
    let (status, body) =
        transcribe(&app, "audio/wav", vec![0u8; 5 * 1024 * 1024 + 1024], None).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"]["code"], "payload_too_large");
    assert_eq!(stt.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn only_one_transcription_runs_at_a_time() {
    let stt = FakeStt::new("hello", 150);
    let (app, _mock) = voice_app(Some(Voice::new(Some(stt.clone()), None, 30.0)), false).await;
    let app = Arc::new(app);
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            transcribe(&app, "audio/wav", wav(0.1), None).await.0
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), StatusCode::OK);
    }
    assert_eq!(stt.calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        stt.max_active.load(Ordering::SeqCst),
        1,
        "the gate serialises the engine"
    );
}

#[tokio::test]
async fn speak_returns_wav_and_checks_its_input() {
    let voice = Voice::new(None, Some(Arc::new(FakeTts)), 30.0);
    let (app, _mock) = voice_app(Some(voice), false).await;
    let (status, headers, bytes) = raw_request(
        &app.router,
        "POST",
        "/api/voice/speak",
        &[],
        Some(json!({ "text": "Buy gold, 0.1 lots" }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("content-type").unwrap(), "audio/wav");
    assert!(bytes.starts_with(b"RIFF"));
    assert!(bytes.ends_with(b"Buy gold, 0.1 lots"));
    let rows = app.audit.rows(None).await.unwrap();
    assert!(
        rows.iter()
            .any(|r| r.kind == "voice_tts" && r.payload["chars"] == 18)
    );

    for (label, body) in [
        ("empty", json!({ "text": "   " })),
        ("too long", json!({ "text": "x".repeat(601) })),
        ("missing field", json!({ "words": "hi" })),
    ] {
        let (status, body) = request(&app.router, "POST", "/api/voice/speak", Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {body}");
    }
    let (status, _) = request(
        &app.router,
        "POST",
        "/api/voice/speak",
        Some(json!({ "text": "x".repeat(600) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "exactly 600 characters is fine");
}

#[tokio::test]
async fn voice_routes_need_a_session_when_login_is_on() {
    let stt = FakeStt::new("hello", 0);
    let voice = Voice::new(Some(stt.clone()), Some(Arc::new(FakeTts)), 30.0);
    let (app, _mock) = voice_app(Some(voice), true).await;
    let (status, _) = transcribe(&app, "audio/wav", wav(0.1), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(stt.calls.load(Ordering::SeqCst), 0);
    let user = create_user(&app, "ninad", PASSWORD).await;
    let cookie = login(&app, &user).await;
    let (status, body) = transcribe(&app, "audio/wav", wav(0.1), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Counts how often it is asked to speak.
struct CountingTts(AtomicUsize);

#[async_trait]
impl TextToSpeech for CountingTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        FakeTts.synthesize(text).await
    }
    fn voice_id(&self) -> String {
        "fake:counting".to_owned()
    }
}

#[tokio::test]
async fn repeated_lines_are_served_from_the_memory_cache() {
    let tts = Arc::new(CountingTts(Default::default()));
    let voice = Voice::new(None, Some(tts.clone()), 30.0);
    let (app, _mock) = voice_app(Some(voice), false).await;
    let speak = |text: &'static str| {
        let router = app.router.clone();
        async move {
            raw_request(
                &router,
                "POST",
                "/api/voice/speak",
                &[],
                Some(json!({ "text": text }).to_string()),
            )
            .await
        }
    };
    let (status, first_headers, first) = speak("Good afternoon, sir.").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first_headers.get("x-jarvis-cache").unwrap(), "miss");
    assert_eq!(
        first_headers.get("x-jarvis-voice").unwrap(),
        "fake:counting"
    );

    let (_, again_headers, again) = speak("Good afternoon, sir.").await;
    assert_eq!(again_headers.get("x-jarvis-cache").unwrap(), "hit");
    assert_eq!(again, first);
    assert_eq!(tts.0.load(Ordering::SeqCst), 1, "the engine ran once");

    let (_, other_headers, _) = speak("Something else.").await;
    assert_eq!(other_headers.get("x-jarvis-cache").unwrap(), "miss");
    assert_eq!(tts.0.load(Ordering::SeqCst), 2);
}

/// VoiceStudio that connects (health, engine, profiles) but whose speech endpoint fails.
async fn failing_studio() -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    for (verb, at) in [("GET", "/health"), ("POST", "/engines/select")] {
        Mock::given(method(verb))
            .and(path(at))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/profiles"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!([{"id": "p1", "name": "Jarvis"}, {"id": "p2", "name": "Friday"}]),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn when_voicestudio_fails_piper_speaks_and_the_status_says_so() {
    let studio = failing_studio().await;
    let cfg = jarvis_voice::VoiceStudioConfig {
        url: studio.uri(),
        ..Default::default()
    };
    let tts = Arc::new(CountingTts(Default::default()));
    let voice =
        Voice::new(None, Some(tts.clone()), 30.0).with_voicestudio(&cfg, real_personas().list());
    assert_eq!(voice.status()["tts_engine"], "voicestudio");
    let (app, _mock) = voice_app(Some(voice), false).await;

    for _ in 0..2 {
        let (status, headers, bytes) = raw_request(
            &app.router,
            "POST",
            "/api/voice/speak",
            &[],
            Some(json!({ "text": "Good afternoon." }).to_string()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(bytes.starts_with(b"RIFF"));
        assert_eq!(headers.get("x-jarvis-voice").unwrap(), "fake:counting");
        // The fallback's audio is not cached as if VoiceStudio had spoken it.
        assert_eq!(headers.get("x-jarvis-cache").unwrap(), "miss");
    }
    assert_eq!(tts.0.load(Ordering::SeqCst), 2);
    let (_, status) = request(&app.router, "GET", "/api/status", None).await;
    assert_eq!(status["voice"]["last_voice"], "fake:counting", "{status}");
}

#[tokio::test]
async fn when_voicestudio_is_down_at_start_piper_speaks_and_start_up_does_not_wait() {
    let cfg = jarvis_voice::VoiceStudioConfig {
        url: "http://127.0.0.1:1".into(),
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let tts = Arc::new(CountingTts(Default::default()));
    let voice =
        Voice::new(None, Some(tts.clone()), 30.0).with_voicestudio(&cfg, real_personas().list());
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "no wait at start-up"
    );
    assert_eq!(voice.status()["tts_engine"], "voicestudio");
    let (app, _mock) = voice_app(Some(voice), false).await;
    let (status, headers, _) = raw_request(
        &app.router,
        "POST",
        "/api/voice/speak",
        &[],
        Some(json!({ "text": "Hello." }).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("x-jarvis-voice").unwrap(), "fake:counting");
    let (_, body) = request(&app.router, "GET", "/api/status", None).await;
    assert_eq!(body["voice"]["voicestudio_connected"], false, "{body}");
}

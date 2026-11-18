//! VoiceStudio contract (FL-6), against a wiremock stand-in.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jarvis_voice::{
    FallbackTts, StudioLink, TextToSpeech, VoiceError, VoiceStudioClient, VoiceStudioConfig,
    VoiceStudioTts,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn wav_bytes() -> Vec<u8> {
    let mut wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    wav.extend_from_slice(&[0; 24]);
    wav
}

/// A VoiceStudio that is up, pins engines, has a "Jarvis" profile and speaks.
async fn studio() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/engines/select"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/profiles"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([{"id": "prof-1", "name": "Jarvis"}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wav_bytes()))
        .mount(&server)
        .await;
    server
}

fn config(server: &MockServer) -> VoiceStudioConfig {
    VoiceStudioConfig {
        url: server.uri(),
        ..VoiceStudioConfig::default()
    }
}

async fn bodies(server: &MockServer, at: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == at)
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[tokio::test]
async fn connect_pins_the_engine_and_speech_uses_the_profile_found_by_name() {
    let server = studio().await;
    let link = StudioLink::new(&config(&server)).unwrap();
    link.client().await.unwrap();
    assert!(link.is_connected());
    assert_eq!(
        bodies(&server, "/engines/select").await,
        [
            json!({"family": "tts", "backend_id": "mlx-audio", "model_id": "mlx-community/csm-1b-8bit"})
        ]
    );
    let tts = VoiceStudioTts::new(link, "jarvis");
    let wav = tts.synthesize("Good afternoon.").await.unwrap();
    assert!(wav.starts_with(b"RIFF"));
    assert_eq!(tts.voice_id(), "voicestudio:jarvis");
    assert_eq!(
        bodies(&server, "/v1/audio/speech").await,
        [json!({
            "model": "mlx-community/csm-1b-8bit",
            "input": "Good afternoon.",
            "voice": "prof-1",
            "response_format": "wav",
            "language": "en"
        })]
    );
    // The profile id is remembered: one lookup for two lines.
    tts.synthesize("Again.").await.unwrap();
    let lookups = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/profiles")
        .count();
    assert_eq!(lookups, 1);
}

#[tokio::test]
async fn connect_fails_when_voicestudio_is_down_or_refuses_the_engine() {
    let server = MockServer::start().await; // answers 404 to everything
    assert!(VoiceStudioClient::connect(&config(&server)).await.is_err());

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/engines/select"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;
    let err = VoiceStudioClient::connect(&config(&server))
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("refused engine"), "{err}");
}

#[tokio::test]
async fn a_slow_health_check_counts_as_down() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;
    assert!(VoiceStudioClient::connect(&config(&server)).await.is_err());
}

#[tokio::test]
async fn a_non_loopback_url_is_refused_before_any_request() {
    let cfg = VoiceStudioConfig {
        url: "http://192.168.1.20:3900".into(),
        ..VoiceStudioConfig::default()
    };
    let err = VoiceStudioClient::connect(&cfg).await.err().unwrap();
    assert!(matches!(err, VoiceError::Config(_)), "{err}");
    assert!(StudioLink::new(&cfg).is_err());
}

/// Mount the endpoints of a working VoiceStudio on `server`.
async fn come_up(server: &MockServer) {
    for (verb, at) in [("GET", "/health"), ("POST", "/engines/select")] {
        Mock::given(method(verb))
            .and(path(at))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn a_voicestudio_opened_after_start_up_is_picked_up_later() {
    let server = MockServer::start().await; // closed: 404 to everything
    let link = StudioLink::with_retry(&config(&server), Duration::ZERO).unwrap();
    assert!(link.client().await.is_err());
    assert!(!link.is_connected());
    come_up(&server).await;
    assert!(link.client().await.is_ok());
    assert!(link.is_connected());
}

#[tokio::test]
async fn reconnects_wait_for_the_interval() {
    let server = MockServer::start().await;
    let link = StudioLink::with_retry(&config(&server), Duration::from_secs(3600)).unwrap();
    assert!(link.client().await.is_err());
    come_up(&server).await;
    let before = server.received_requests().await.unwrap().len();
    assert!(
        link.client().await.is_err(),
        "inside the interval: no new attempt"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn an_unknown_profile_or_a_non_wav_answer_is_an_error() {
    let server = studio().await;
    let link = StudioLink::new(&config(&server)).unwrap();
    let nobody = VoiceStudioTts::new(link.clone(), "Nobody");
    assert!(nobody.synthesize("hi").await.is_err());
    assert!(
        link.is_connected(),
        "an unknown profile does not drop the connection"
    );

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
            ResponseTemplate::new(200).set_body_json(json!([{"id": "x", "name": "Jarvis"}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not audio"))
        .mount(&server)
        .await;
    let link = StudioLink::new(&config(&server)).unwrap();
    let err = VoiceStudioTts::new(link, "Jarvis")
        .synthesize("hi")
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("did not return a WAV"), "{err}");
}

struct Named(&'static str, bool);

#[async_trait]
impl TextToSpeech for Named {
    async fn synthesize(&self, _text: &str) -> Result<Vec<u8>, VoiceError> {
        if self.1 {
            Ok(self.0.as_bytes().to_vec())
        } else {
            Err(VoiceError::Tts("down".into()))
        }
    }
    fn voice_id(&self) -> String {
        self.0.to_owned()
    }
}

#[tokio::test]
async fn the_fallback_speaks_when_the_primary_fails_and_says_so() {
    let up = FallbackTts::new(
        Arc::new(Named("voicestudio:jarvis", true)),
        Some(Arc::new(Named("piper:alan", true))),
    );
    let (wav, by) = up.synthesize_labeled("x").await.unwrap();
    assert_eq!(
        (wav.as_slice(), by.as_str()),
        (&b"voicestudio:jarvis"[..], "voicestudio:jarvis")
    );

    let down = FallbackTts::new(
        Arc::new(Named("voicestudio:jarvis", false)),
        Some(Arc::new(Named("piper:alan", true))),
    );
    let (wav, by) = down.synthesize_labeled("x").await.unwrap();
    assert_eq!(
        (wav.as_slice(), by.as_str()),
        (&b"piper:alan"[..], "piper:alan")
    );
    assert_eq!(down.voice_id(), "voicestudio:jarvis");
    assert_eq!(down.synthesize("x").await.unwrap(), b"piper:alan");

    let alone = FallbackTts::new(Arc::new(Named("voicestudio:jarvis", false)), None);
    assert!(alone.synthesize("x").await.is_err());
}

#[tokio::test]
async fn keep_warm_renders_only_after_the_idle_time() {
    let server = studio().await;
    let tts = VoiceStudioTts::new(StudioLink::new(&config(&server)).unwrap(), "Jarvis");
    assert!(!tts.keep_warm(Duration::from_secs(600)).await, "just used");
    assert!(tts.keep_warm(Duration::ZERO).await);
    let spoken = bodies(&server, "/v1/audio/speech").await;
    assert_eq!(spoken.len(), 1);
    assert_eq!(spoken[0]["input"], "Standing by.");
}

//! Text-to-speech through VoiceStudio (FL-6): a local app that speaks with a voice cloned from a
//! short clip, on `http://127.0.0.1:3900`. Optional; Piper stays the fallback ([`FallbackTts`]).
//!
//! The engine is pinned at start-up (`POST /engines/select`) so the app never swaps in another
//! one (whose model may carry a different licence). Profiles are found by name. Only a loopback
//! URL is accepted: spoken text never leaves the machine.
//!
//! The HTTP contract here follows VoiceStudio's documented endpoints (`/health`, `/profiles`,
//! `/engines/select`, `/v1/audio/speech`); it is covered by contract tests, not by a live install.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tts::MAX_SPEECH_CHARS;
use crate::{TextToSpeech, VoiceError};

/// How long `/health` may take before VoiceStudio counts as down.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(1);
/// How long one synthesis may take.
const SPEECH_TIMEOUT: Duration = Duration::from_secs(30);
/// The line rendered and thrown away to keep the model loaded.
pub const KEEP_WARM_LINE: &str = "Standing by.";

/// `[voice.voicestudio]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceStudioConfig {
    #[serde(default = "default_url")]
    pub url: String,
    /// Engine family VoiceStudio should use, e.g. `mlx-audio` (Apple Silicon).
    #[serde(default = "default_engine")]
    pub engine: String,
    #[serde(default = "default_model")]
    pub model_id: String,
    #[serde(default = "default_language")]
    pub language: String,
    /// Render [`KEEP_WARM_LINE`] after this many idle minutes; 0 = never.
    #[serde(default = "default_keep_warm")]
    pub keep_warm_minutes: u64,
    /// Persona id -> VoiceStudio profile name.
    #[serde(default)]
    pub profiles: BTreeMap<String, String>,
}

fn default_url() -> String {
    "http://127.0.0.1:3900".to_owned()
}
fn default_engine() -> String {
    "mlx-audio".to_owned()
}
fn default_model() -> String {
    "mlx-community/csm-1b-8bit".to_owned()
}
fn default_language() -> String {
    "en".to_owned()
}
fn default_keep_warm() -> u64 {
    10
}

impl Default for VoiceStudioConfig {
    fn default() -> Self {
        Self {
            url: default_url(),
            engine: default_engine(),
            model_id: default_model(),
            language: default_language(),
            keep_warm_minutes: default_keep_warm(),
            profiles: BTreeMap::new(),
        }
    }
}

/// `http://` to `localhost`, `127.x.y.z` or `[::1]`, with a port; anything else is refused.
pub fn loopback_base(url: &str) -> Result<String, VoiceError> {
    let refuse = || {
        VoiceError::Config(format!(
            "voicestudio.url must be http:// on this machine (localhost, 127.0.0.1 or [::1]), got '{url}'"
        ))
    };
    let base = url.trim().trim_end_matches('/');
    let rest = base.strip_prefix("http://").ok_or_else(refuse)?;
    if rest.contains(['/', '?', '#', '@']) {
        return Err(refuse());
    }
    let host = match rest.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map(|(h, _)| h).ok_or_else(refuse)?,
        None => rest.rsplit_once(':').map_or(rest, |(h, _)| h),
    };
    let local = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if local {
        Ok(base.to_owned())
    } else {
        Err(refuse())
    }
}

/// The connection to VoiceStudio, shared by every persona's voice.
pub struct VoiceStudioClient {
    base: String,
    http: reqwest::Client,
    config: VoiceStudioConfig,
}

impl VoiceStudioClient {
    /// Check the URL, that VoiceStudio answers `/health` within a second, and pin the engine.
    pub async fn connect(config: &VoiceStudioConfig) -> Result<Arc<Self>, VoiceError> {
        let base = loopback_base(&config.url)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| VoiceError::Config(format!("voicestudio client: {e}")))?;
        let client = Arc::new(Self {
            base,
            http,
            config: config.clone(),
        });
        if !client.healthy().await {
            return Err(VoiceError::Tts(format!(
                "VoiceStudio does not answer at {} (is the app open?)",
                client.base
            )));
        }
        client.pin_engine().await?;
        Ok(client)
    }

    pub fn config(&self) -> &VoiceStudioConfig {
        &self.config
    }

    /// `GET /health` answers 2xx within [`HEALTH_TIMEOUT`].
    pub async fn healthy(&self) -> bool {
        self.http
            .get(format!("{}/health", self.base))
            .timeout(HEALTH_TIMEOUT)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    /// `POST /engines/select`, so VoiceStudio uses exactly the configured engine and model.
    pub async fn pin_engine(&self) -> Result<(), VoiceError> {
        let body = json!({
            "family": "tts",
            "backend_id": self.config.engine,
            "model_id": self.config.model_id,
        });
        let r = self
            .http
            .post(format!("{}/engines/select", self.base))
            .timeout(Duration::from_secs(10))
            .json(&body)
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("VoiceStudio engine select: {e}")))?;
        if !r.status().is_success() {
            return Err(VoiceError::Tts(format!(
                "VoiceStudio refused engine '{}' with model '{}' ({})",
                self.config.engine,
                self.config.model_id,
                r.status()
            )));
        }
        Ok(())
    }

    /// The id of the profile called `name` (case-insensitive) from `GET /profiles`.
    pub async fn profile_id(&self, name: &str) -> Result<String, VoiceError> {
        let v: Value = self
            .http
            .get(format!("{}/profiles", self.base))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("VoiceStudio profiles: {e}")))?
            .json()
            .await
            .map_err(|e| VoiceError::Tts(format!("VoiceStudio profiles: {e}")))?;
        find_profile(&v, name).ok_or_else(|| {
            VoiceError::Config(format!(
                "VoiceStudio has no profile called '{name}' (create it under Profiles)"
            ))
        })
    }

    /// `POST /v1/audio/speech` with the pinned model; returns the WAV.
    pub async fn speak(&self, profile_id: &str, text: &str) -> Result<Vec<u8>, VoiceError> {
        let body = json!({
            "model": self.config.model_id,
            "input": text,
            "voice": profile_id,
            "response_format": "wav",
            "language": self.config.language,
        });
        let r = self
            .http
            .post(format!("{}/v1/audio/speech", self.base))
            .timeout(SPEECH_TIMEOUT)
            .json(&body)
            .send()
            .await
            .map_err(|e| VoiceError::Tts(format!("VoiceStudio: {e}")))?;
        let status = r.status();
        let bytes = r
            .bytes()
            .await
            .map_err(|e| VoiceError::Tts(format!("VoiceStudio: {e}")))?;
        if !status.is_success() {
            return Err(VoiceError::Tts(format!("VoiceStudio answered {status}")));
        }
        if !(bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE") {
            return Err(VoiceError::Tts(
                "VoiceStudio did not return a WAV".to_owned(),
            ));
        }
        Ok(bytes.to_vec())
    }
}

/// `[{"id","name"}]` or `{"profiles": [...]}`.
fn find_profile(v: &Value, name: &str) -> Option<String> {
    let list = v
        .as_array()
        .or_else(|| v.get("profiles").and_then(Value::as_array))?;
    list.iter().find_map(|p| {
        let n = p.get("name").and_then(Value::as_str)?;
        if !n.trim().eq_ignore_ascii_case(name.trim()) {
            return None;
        }
        match p.get("id")? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    })
}

/// How long after a failed connection the next request may try again.
pub const RECONNECT_AFTER: Duration = Duration::from_secs(30);

/// The connection to VoiceStudio as the voices see it: made on first use (or by
/// [`StudioLink::connect_in_background`] at start-up), dropped after a failed request, and made
/// again (engine pinned again) at most every [`RECONNECT_AFTER`]. Nothing ever waits for it at
/// start-up, and a VoiceStudio that was closed at boot is picked up once it is opened.
pub struct StudioLink {
    config: VoiceStudioConfig,
    retry_after: Duration,
    state: tokio::sync::Mutex<LinkState>,
    connected: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
struct LinkState {
    client: Option<Arc<VoiceStudioClient>>,
    last_failure: Option<Instant>,
}

impl StudioLink {
    /// Checks the URL only (it must be loopback); no request is made.
    pub fn new(config: &VoiceStudioConfig) -> Result<Arc<Self>, VoiceError> {
        Self::with_retry(config, RECONNECT_AFTER)
    }

    /// Like [`StudioLink::new`] with another reconnect interval (tests).
    pub fn with_retry(
        config: &VoiceStudioConfig,
        retry_after: Duration,
    ) -> Result<Arc<Self>, VoiceError> {
        loopback_base(&config.url)?;
        Ok(Arc::new(Self {
            config: config.clone(),
            retry_after,
            state: tokio::sync::Mutex::new(LinkState::default()),
            connected: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    pub fn config(&self) -> &VoiceStudioConfig {
        &self.config
    }

    /// Whether the last connection attempt worked and no request has failed since.
    pub fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Try to connect now without waiting for the result.
    pub fn connect_in_background(self: &Arc<Self>) {
        let link = self.clone();
        tokio::spawn(async move {
            if let Err(e) = link.client().await {
                tracing::warn!("VoiceStudio is not reachable yet, Piper speaks meanwhile: {e}");
            } else {
                tracing::info!("VoiceStudio connected, engine pinned");
            }
        });
    }

    /// The connected client: the current one, or a new connection unless the last attempt
    /// failed less than the reconnect interval ago.
    pub async fn client(&self) -> Result<Arc<VoiceStudioClient>, VoiceError> {
        let mut state = self.state.lock().await;
        if let Some(client) = &state.client {
            return Ok(client.clone());
        }
        if state
            .last_failure
            .is_some_and(|at| at.elapsed() < self.retry_after)
        {
            return Err(VoiceError::Tts(
                "VoiceStudio was not reachable a moment ago; trying again shortly".to_owned(),
            ));
        }
        match VoiceStudioClient::connect(&self.config).await {
            Ok(client) => {
                state.client = Some(client.clone());
                state.last_failure = None;
                self.connected
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(client)
            }
            Err(e) => {
                state.last_failure = Some(Instant::now());
                self.connected
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                Err(e)
            }
        }
    }

    /// A request failed: connect again (and pin the engine again) next time, after the interval.
    pub async fn forget(&self) {
        let mut state = self.state.lock().await;
        state.client = None;
        state.last_failure = Some(Instant::now());
        self.connected
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One persona's VoiceStudio voice.
pub struct VoiceStudioTts {
    link: Arc<StudioLink>,
    profile: String,
    /// Looked up on first use and again after a failure (the profile may have been recreated).
    profile_id: Mutex<Option<String>>,
    last_used: Mutex<Instant>,
}

impl VoiceStudioTts {
    pub fn new(link: Arc<StudioLink>, profile: &str) -> Self {
        Self {
            link,
            profile: profile.to_owned(),
            profile_id: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        }
    }

    async fn id(&self, client: &VoiceStudioClient) -> Result<String, VoiceError> {
        if let Some(id) = self
            .profile_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            return Ok(id);
        }
        let id = client.profile_id(&self.profile).await?;
        *self.profile_id.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.clone());
        Ok(id)
    }

    /// Render [`KEEP_WARM_LINE`] and throw it away when nothing was spoken for `idle`. Returns
    /// whether it did.
    pub async fn keep_warm(&self, idle: Duration) -> bool {
        let quiet = self
            .last_used
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .elapsed();
        if quiet < idle {
            return false;
        }
        let ok = self.synthesize(KEEP_WARM_LINE).await.is_ok();
        if !ok {
            tracing::debug!(profile = %self.profile, "VoiceStudio keep-warm failed");
        }
        ok
    }
}

#[async_trait]
impl TextToSpeech for VoiceStudioTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(MAX_SPEECH_CHARS)
            .collect();
        if text.trim().is_empty() {
            return Err(VoiceError::Tts("nothing to say".to_owned()));
        }
        *self.last_used.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
        let client = self.link.client().await?;
        let result = match self.id(&client).await {
            Ok(id) => client.speak(&id, &text).await,
            Err(e @ VoiceError::Config(_)) => return Err(e), // no such profile: VoiceStudio is fine
            Err(e) => Err(e),
        };
        if result.is_err() {
            // Look the profile up again next time, and reconnect (VoiceStudio may have restarted
            // and forgotten the pinned engine).
            *self.profile_id.lock().unwrap_or_else(|p| p.into_inner()) = None;
            self.link.forget().await;
        }
        result
    }

    fn voice_id(&self) -> String {
        format!("voicestudio:{}", self.profile)
    }
}

/// A voice with a fallback: when the primary fails, the fallback speaks (VoiceStudio first,
/// Piper when it is down). [`TextToSpeech::synthesize_labeled`] says which one spoke.
pub struct FallbackTts {
    primary: Arc<dyn TextToSpeech>,
    fallback: Option<Arc<dyn TextToSpeech>>,
}

impl FallbackTts {
    pub fn new(primary: Arc<dyn TextToSpeech>, fallback: Option<Arc<dyn TextToSpeech>>) -> Self {
        Self { primary, fallback }
    }
}

#[async_trait]
impl TextToSpeech for FallbackTts {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, VoiceError> {
        Ok(self.synthesize_labeled(text).await?.0)
    }

    async fn synthesize_labeled(&self, text: &str) -> Result<(Vec<u8>, String), VoiceError> {
        match self.primary.synthesize_labeled(text).await {
            Ok(spoken) => Ok(spoken),
            Err(e) => {
                let Some(fallback) = &self.fallback else {
                    return Err(e);
                };
                tracing::warn!(
                    voice = %self.primary.voice_id(),
                    "speaking with {} instead: {e}",
                    fallback.voice_id()
                );
                fallback.synthesize_labeled(text).await
            }
        }
    }

    fn voice_id(&self) -> String {
        self.primary.voice_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_http_is_accepted() {
        for ok in [
            "http://127.0.0.1:3900",
            "http://127.0.0.1:3900/",
            "http://localhost:3900",
            "http://[::1]:3900",
        ] {
            assert!(loopback_base(ok).is_ok(), "{ok}");
        }
        for bad in [
            "https://127.0.0.1:3900",
            "http://192.168.1.4:3900",
            "http://voicestudio.example.com",
            "http://127.0.0.1:3900/v1",
            "http://user@127.0.0.1:3900",
            "http://127.0.0.1.nip.io:3900",
            "127.0.0.1:3900",
        ] {
            assert!(loopback_base(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn profiles_are_found_by_name_in_either_shape() {
        let list = json!([{"id": "p1", "name": "Calm"}, {"id": 7, "name": "Jarvis"}]);
        assert_eq!(find_profile(&list, "jarvis").as_deref(), Some("7"));
        let wrapped = json!({"profiles": [{"id": "abc", "name": " Friday "}]});
        assert_eq!(find_profile(&wrapped, "Friday").as_deref(), Some("abc"));
        assert_eq!(find_profile(&list, "nobody"), None);
        assert_eq!(find_profile(&json!({}), "Jarvis"), None);
    }
}

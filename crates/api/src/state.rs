//! Shared application state.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::bail;
use async_trait::async_trait;
use jarvis_bridge_client::protocol::{PingInfo, TradeMode};
use jarvis_bridge_client::{BridgeClient, BridgeConfig, BridgeError};
use jarvis_gate::KillSwitch;
use jarvis_llm::{LlmProvider, ProviderConfig, ProviderKind, build_provider, effective_base_url};

use crate::egress::{EgressLog, EgressNews, EgressProvider};
use jarvis_news::{AlphaVantage, Finnhub, NewsService};
use jarvis_persona::{PersonaPack, PersonaRegistry, Theme};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, watch};

use crate::agent::Agent;
use crate::audit::Audit;
use crate::auth::crypto::PasswordHasherCfg;
use crate::auth::{AuthParts, AuthReleaseGuard, AuthService, Session};
use crate::broker::{self, BrokerParts, BrokerService, DataPaths};
use crate::clock::{ServerClock, TimeSource, WallClock};
use crate::config::{Config, MarketNewsConfig, Mode, NewsProviderKind};
use crate::events::EventBus;
use crate::limit_overrides::PartialLimits;
use crate::market::{MarketParts, MarketService};
use crate::ratelimit::RateLimiter;
use crate::settings::{
    LOCAL_USER, Rules, SettingsError, SettingsPatch, SettingsStore, UserSettings, apply_patch,
};
use crate::symbols::SymbolResolver;
use crate::tools::ToolRegistry;
use crate::trading::{Trading, TradingParts, WriteSink};
use crate::voice::Voice;

/// Environment variable holding the session signing key (at least 32 bytes).
pub const JWT_SECRET_ENV: &str = "JARVIS_JWT_SECRET";

/// How often expired previews and idle rate-limit keys are cleaned up.
pub const PURGE_INTERVAL: Duration = Duration::from_secs(5);

/// How often the market views are scored against the price.
pub const MARKET_EVALUATE_EVERY: Duration = Duration::from_secs(5 * 60);

/// The broker account is re-read every this many purge ticks (30 s).
const BROKER_CHECK_EVERY_TICKS: u32 = 6;

/// Replacements for production defaults, for tests and embedding.
#[derive(Default)]
pub struct Overrides {
    /// Time source for token windows and rate limits (default: the real clock).
    pub time: Option<Arc<dyn TimeSource>>,
    /// Destination of confirmed writes (default: the bridge client).
    pub write_sink: Option<Arc<dyn WriteSink>>,
    /// Login service. Required when `[auth] enabled = true`: without it every protected
    /// route answers 401 (fail closed). [`AppState::build`] creates it from the environment.
    pub auth: Option<Arc<AuthService>>,
    /// Speech engines (default: voice off).
    pub voice: Option<Voice>,
    /// The persona packs (default: loaded from `[personas] dir`, or one built-in Jarvis pack
    /// when that folder cannot be read; [`AppState::build`] refuses to start instead).
    pub personas: Option<PersonaRegistry>,
    /// The news service (default: built from `[market.news]`).
    pub news: Option<NewsService>,
    /// Desk mode's Finnhub client (default: from the key in `[desk] finnhub_key_env`).
    pub finnhub: Option<jarvis_news::FinnhubData>,
    /// The Alpaca client (default: from the URL and keys named in `[alpaca]`).
    pub alpaca: Option<jarvis_alpaca::AlpacaClient>,
}

/// A configured LLM provider that is either ready or missing something (usually an API key).
#[derive(Clone)]
pub enum ProviderSlot {
    Available(Arc<dyn LlmProvider>),
    Unavailable(String),
}

pub struct ProviderEntry {
    pub config: ProviderConfig,
    pub slot: ProviderSlot,
}

impl ProviderEntry {
    pub fn kind_str(&self) -> &'static str {
        match self.config.kind {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
            ProviderKind::Ollama => "ollama",
        }
    }
}

/// SPEC rule 7: tracks whether DEMO mode is configured while the account is real.
/// Shared by [`AppState`] and [`Trading`].
#[derive(Debug)]
pub struct ModeMonitor {
    configured: Mode,
    mismatch: AtomicBool,
}

impl ModeMonitor {
    pub fn new(configured: Mode) -> Self {
        Self {
            configured,
            mismatch: AtomicBool::new(false),
        }
    }

    pub fn mismatch(&self) -> bool {
        self.mismatch.load(Ordering::SeqCst)
    }

    /// Record the account trade mode seen on a successful bridge call.
    pub fn observe(&self, seen: TradeMode) {
        let mismatch = self.configured == Mode::Demo && seen == TradeMode::Real;
        let was = self.mismatch.swap(mismatch, Ordering::SeqCst);
        if mismatch && !was {
            tracing::error!(
                "MODE MISMATCH: config mode is demo but the MT5 account trade_mode is real. \
                 Chat and trading are disabled until the account or the config is fixed."
            );
        } else if !mismatch && was {
            tracing::warn!("mode mismatch cleared: account trade_mode is now {seen:?}");
        }
    }
}

/// Decides whether the kill switch may be released. With login enabled the guard is
/// [`AuthReleaseGuard`] (a session and a fresh TOTP code); with login off, releases are open.
/// Replace it with [`AppState::set_release_guard`].
#[async_trait]
pub trait ReleaseGuard: Send + Sync {
    /// `session` is the caller's login session (`None` when login is disabled); `totp` is
    /// the code from the request body, if any. `Err(message)` refuses.
    async fn authorize_release(
        &self,
        session: Option<&Session>,
        totp: Option<&str>,
    ) -> Result<(), String>;
}

/// Releases are always allowed (auth disabled).
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenRelease;

#[async_trait]
impl ReleaseGuard for OpenRelease {
    async fn authorize_release(
        &self,
        _session: Option<&Session>,
        _totp: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Tells long-lived tasks (WebSocket connections) to finish because the server is stopping.
#[derive(Debug, Clone)]
pub struct ShutdownSignal {
    tx: Arc<watch::Sender<bool>>,
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self {
            tx: Arc::new(watch::channel(false).0),
        }
    }

    pub fn trigger(&self) {
        self.tx.send_replace(true);
    }

    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }

    /// A receiver whose `changed()` completes when the server starts shutting down (at once
    /// if it already has).
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        let mut rx = self.tx.subscribe();
        if *rx.borrow() {
            rx.mark_changed();
        }
        rx
    }
}

/// Why `PUT /api/settings` failed.
#[derive(Debug)]
pub enum SettingsUpdateError {
    /// 400.
    Invalid(SettingsError),
    /// 500.
    Storage(anyhow::Error),
}

/// The personas of the config folder, or, when that cannot be read (tests and embedding that
/// do not ship a `personas/` folder), one built-in pack so the state is still complete.
fn load_personas_or_builtin(config: &Config) -> PersonaRegistry {
    match PersonaRegistry::load_dir(&config.personas.dir, &config.personas.default) {
        Ok(registry) => registry,
        Err(e) => {
            tracing::warn!("{e}; using the built-in Jarvis persona");
            builtin_personas()
        }
    }
}

pub(crate) fn builtin_personas() -> PersonaRegistry {
    let jarvis = PersonaPack {
        id: "jarvis".to_owned(),
        name: "Jarvis".to_owned(),
        description: "Calm, formal and concise.".to_owned(),
        system_prompt: "Speak like a composed, formal British butler who happens to be an \
                        expert trader's aide. Be brief and precise: lead with the answer, then \
                        the essential numbers."
            .to_owned(),
        voice_model: "models/piper/en_GB-alan-medium.onnx".into(),
        wake_word: "hey jarvis".to_owned(),
        theme: Theme {
            accent: "#38bdf8".to_owned(),
            accent_soft: "#0c4a6e".to_owned(),
            orb_core: "#7dd3fc".to_owned(),
            orb_glow: "#0ea5e9".to_owned(),
        },
    };
    PersonaRegistry::from_packs(vec![jarvis], "jarvis")
        .expect("the built-in persona is valid by construction")
}

/// Open WebSocket connections allowed at once.
const MAX_SOCKETS: usize = crate::ws::MAX_SOCKETS;

struct Inner {
    config: Config,
    bridge: BridgeClient,
    providers: Vec<ProviderEntry>,
    audit: Audit,
    agent: Agent,
    resolver: Arc<SymbolResolver>,
    /// The broker account and its symbol maps.
    broker: Arc<BrokerService>,
    clock: ServerClock,
    /// Rule 7: set when the config says DEMO but the account reports a real trade mode.
    mode: Arc<ModeMonitor>,
    trading: Arc<Trading>,
    events: EventBus,
    release_guard: RwLock<Arc<dyn ReleaseGuard>>,
    auth: Option<Arc<AuthService>>,
    voice: Voice,
    personas: PersonaRegistry,
    settings: SettingsStore,
    shutdown: ShutdownSignal,
    ws_slots: Arc<Semaphore>,
    /// Per-session budgets of the expensive routes (NET-2).
    chat_limiter: RateLimiter,
    transcribe_limiter: RateLimiter,
    market_limiter: RateLimiter,
    /// Market views, the calendar and news; `None` when `[market] enabled = false`.
    market: Option<Arc<MarketService>>,
    /// Every outbound request to another service (FL-3).
    egress: Arc<EgressLog>,
    /// The user's notes; `None` when `[notes] enabled = false`.
    notes: Option<Arc<jarvis_notes::NotesService>>,
    /// Assistant mode; `None` when `[assistant] enabled = false`.
    assistant: Option<Arc<crate::assistant::Assistant>>,
    desk: Option<Arc<crate::desk::Desk>>,
    router: Arc<crate::venue::VenueRouter>,
    /// Why Alpaca is off (`None` when it is on).
    alpaca_off: Option<String>,
}

/// The window of the per-session request budgets.
const REQUEST_RATE_WINDOW_MS: i64 = 60_000;

/// Which expensive route a request budget is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    Chat,
    Transcribe,
    Market,
}

/// Cheap to clone; shared by every request handler.
#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    /// Assemble state from ready parts. `slots` maps provider name to its slot; configured
    /// providers without a slot are reported unavailable.
    pub fn new(
        config: Config,
        bridge: BridgeClient,
        audit: Audit,
        slots: HashMap<String, ProviderSlot>,
    ) -> Self {
        Self::new_with(config, bridge, audit, slots, Overrides::default())
    }

    /// Like [`AppState::new`] with [`Overrides`] (tests move time by hand, fake a write
    /// transport).
    pub fn new_with(
        mut config: Config,
        bridge: BridgeClient,
        audit: Audit,
        mut slots: HashMap<String, ProviderSlot>,
        overrides: Overrides,
    ) -> Self {
        let time: Arc<dyn TimeSource> = overrides.time.unwrap_or_else(|| Arc::new(WallClock));
        let auth = overrides.auth;
        let release_guard: Arc<dyn ReleaseGuard> = match &auth {
            Some(service) => Arc::new(AuthReleaseGuard(service.clone())),
            None => Arc::new(OpenRelease),
        };
        // Symbol maps: names in the config become canonical, the resolver reads the book.
        let book = broker::book_from_config(&config);
        config.canonicalize_symbols(|name| book.canonical_loose(name));
        let resolver = Arc::new(SymbolResolver::with_book(
            &config.symbols.aliases,
            book.clone(),
        ));
        let events = EventBus::new();
        let broker = BrokerService::new(BrokerParts {
            book,
            bridge: bridge.clone(),
            audit: audit.clone(),
            events: events.clone(),
            paths: DataPaths::from_dir(&config.symbols.data_dir),
            auto_detect: config.symbols.auto_detect,
        });
        let personas = overrides
            .personas
            .unwrap_or_else(|| load_personas_or_builtin(&config));
        let settings = SettingsStore::new(audit.pool().clone());
        let egress = EgressLog::new();
        let providers = config
            .llm
            .providers
            .iter()
            .map(|p| ProviderEntry {
                config: p.clone(),
                slot: match slots.remove(&p.name) {
                    // Every model request is recorded with the host it goes to.
                    Some(ProviderSlot::Available(provider)) => {
                        ProviderSlot::Available(EgressProvider::wrap(
                            provider,
                            &effective_base_url(p),
                            &p.name,
                            egress.clone(),
                        ))
                    }
                    Some(other) => other,
                    None => ProviderSlot::Unavailable("not configured".to_owned()),
                },
            })
            .collect();
        let clock = ServerClock::default();
        let (chat_per_minute, transcribe_per_minute) = (
            config.server.chat_rate_limit_per_minute,
            config.server.transcribe_rate_limit_per_minute,
        );
        let mode = Arc::new(ModeMonitor::new(config.mode));
        // One kill switch for every venue (clones share their state).
        let kill_switch = KillSwitch::new(config.trading.kill_switch_engaged);
        let trading = Trading::new(TradingParts {
            bridge: bridge.clone(),
            reads: None,
            venue: crate::venue::VenueKind::Mt5,
            max_notional: None,
            write_sink: overrides.write_sink,
            clock: clock.clone(),
            mode: mode.clone(),
            audit: audit.clone(),
            limits: config.effective_limits(),
            config: config.trading.clone(),
            kill_switch: kill_switch.clone(),
            time: time.clone(),
            events: events.clone(),
            broker: broker.clone(),
        });
        let market = config.market.enabled.then(|| {
            let news = overrides.news.unwrap_or_else(|| {
                news_from_config(
                    &config.market.news,
                    |name| std::env::var(name).ok(),
                    &egress,
                )
            });
            MarketService::new(MarketParts {
                bridge: bridge.clone(),
                clock: clock.clone(),
                news,
                audit: audit.clone(),
                config: config.market.clone(),
            })
        });
        // Alpaca (spec-alpaca): a second Trading instance on the same gate, kill switch, audit and
        // events, with its own limits and tokens. Off, with the reason, unless the keys are set and
        // the URL is Alpaca paper.
        let (alpaca_route, alpaca_off) = build_alpaca(
            &config,
            overrides.alpaca,
            AlpacaWiring {
                bridge: &bridge,
                audit: &audit,
                kill_switch: &kill_switch,
                time: &time,
                events: &events,
                broker: &broker,
                egress: &egress,
            },
        );
        let router = Arc::new(crate::venue::VenueRouter::new(
            trading.clone(),
            alpaca_route,
        ));
        let mut tools = ToolRegistry::new(bridge.clone(), resolver.clone(), clock.clone())
            .with_trading(trading.clone())
            .with_router(router.clone());
        if let Some(market) = &market {
            tools = tools.with_market(market.clone());
        }
        let notes = config.notes.enabled.then(|| {
            Arc::new(jarvis_notes::NotesService::new(
                config.notes.dir.clone(),
                config.notes.limits(),
            ))
        });
        if let Some(notes) = &notes {
            tools = tools.with_notes(notes.clone());
        }
        let market_per_minute = config.market.rate_limit_per_minute;
        let agent = Agent::new(tools, audit.clone(), clock.clone());
        // Assistant mode: its own loop and tools, with no bridge, trading or market service.
        let assistant = config.assistant.enabled.then(|| {
            let a = &config.assistant;
            Arc::new(crate::assistant::Assistant::new(
                crate::assistant::AssistantParts {
                    audit: audit.clone(),
                    notes: notes.clone(),
                    captures_dir: a.captures_dir.clone(),
                    models: Arc::new(crate::models3d::ModelStore::new(a.models_dir.clone())),
                    spend: Arc::new(crate::spend::SpendMeter::new(
                        Some(a.spend_file.clone()),
                        a.monthly_budget_usd,
                        a.prices(),
                    )),
                    chat_effort: a.chat_effort.clone(),
                    build_effort: a.build_effort.clone(),
                    max_model_bytes: a.max_model_kb * 1024,
                },
            ))
        });
        // Desk mode: Friday and the eight specialists (spec-nimbus-desk). Its own spend meter.
        let desk = config.desk.enabled.then(|| {
            let d = &config.desk;
            let finnhub = overrides.finnhub.or_else(|| {
                std::env::var(&d.finnhub_key_env)
                    .ok()
                    .and_then(|key| jarvis_news::FinnhubData::new(key).ok())
            });
            let desk = Arc::new(crate::desk::Desk::new(crate::desk::DeskParts {
                config: d.clone(),
                audit: audit.clone(),
                bridge: bridge.clone(),
                market: market.clone(),
                trading: trading.clone(),
                router: router.clone(),
                finnhub,
                egress: egress.clone(),
                spend: Arc::new(crate::spend::SpendMeter::new(
                    Some(d.spend_file.clone()),
                    d.monthly_budget_usd,
                    d.prices(),
                )),
                events: events.clone(),
                time: time.clone(),
                learning: config.learning.clone(),
            }));
            desk.follow_trades();
            desk
        });
        Self(Arc::new(Inner {
            config,
            bridge,
            providers,
            audit,
            agent,
            resolver,
            broker,
            clock,
            mode,
            trading,
            events,
            release_guard: RwLock::new(release_guard),
            auth,
            voice: overrides.voice.unwrap_or_else(Voice::disabled),
            personas,
            settings,
            shutdown: ShutdownSignal::new(),
            ws_slots: Arc::new(Semaphore::new(MAX_SOCKETS)),
            chat_limiter: RateLimiter::new(chat_per_minute, REQUEST_RATE_WINDOW_MS),
            transcribe_limiter: RateLimiter::new(transcribe_per_minute, REQUEST_RATE_WINDOW_MS),
            market_limiter: RateLimiter::new(market_per_minute, REQUEST_RATE_WINDOW_MS),
            market,
            egress,
            notes,
            assistant,
            desk,
            router,
            alpaca_off,
        }))
    }

    /// Production wiring: bridge client, providers via `build_provider`, audit database.
    pub async fn build(config: Config) -> anyhow::Result<Self> {
        // Refuse to start without the shared bridge key: the EA would refuse every request
        // anyway, and a server that cannot reach it must say why, not fail on the first call.
        let key = config
            .bridge_key()
            .map_err(|e| anyhow::anyhow!("refusing to start: {e:#}"))?;
        let bridge = BridgeClient::new(BridgeConfig::new(
            config.bridge.endpoint.clone(),
            Duration::from_millis(config.bridge.timeout_ms),
            key,
        ))?;
        let personas = PersonaRegistry::load_dir(&config.personas.dir, &config.personas.default)
            .map_err(|e| {
                anyhow::anyhow!(
                    "refusing to start: the personas are unusable: {e}. Fix or restore the pack \
                     files in {}, or point [personas] dir and default at working ones.",
                    config.personas.dir.display()
                )
            })?;
        tracing::info!(
            default = personas.default_id(),
            "personas loaded: {}",
            personas
                .list()
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let audit = Audit::connect(&config.database.url).await?;
        let auth = if config.auth.enabled {
            let secret = std::env::var(JWT_SECRET_ENV).unwrap_or_default();
            let service = AuthService::new(AuthParts {
                secret,
                session_hours: config.auth.session_hours,
                cookie_secure: config.cookie_secure(),
                time: Arc::new(WallClock),
                audit: audit.clone(),
                hasher: PasswordHasherCfg::owasp(),
            })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "refusing to start: login is enabled but the session key is unusable: {e:#}. \
                     Set {JWT_SECRET_ENV} in .env (or the environment), or set [auth] enabled = \
                     false on a loopback-only server."
                )
            })?;
            if service.accounts().user_count().await? == 0 {
                tracing::warn!(
                    "login is enabled but there are no users yet; create one with \
                     `jarvis-api user add <name>`"
                );
            }
            Some(service)
        } else {
            tracing::warn!(
                "login is DISABLED ([auth] enabled = false): anyone who can reach the server can trade"
            );
            None
        };
        let voice = Voice::from_section(config.voice.as_ref(), personas.list()).await;
        let mut slots = HashMap::new();
        for p in &config.llm.providers {
            let slot = match build_provider(p) {
                Ok(provider) => ProviderSlot::Available(Arc::from(provider)),
                Err(e) => {
                    tracing::warn!(provider = %p.name, "LLM provider unavailable: {e}");
                    ProviderSlot::Unavailable(e.to_string())
                }
            };
            slots.insert(p.name.clone(), slot);
        }
        if config.limits.is_none() {
            tracing::warn!(
                "no [limits] section in the config: allowed_symbols is empty, so every new order \
                 will be refused. See config/jarvis.example.toml."
            );
        }
        let state = Self::new_with(
            config,
            bridge,
            audit,
            slots,
            Overrides {
                auth,
                voice: Some(voice),
                personas: Some(personas),
                ..Overrides::default()
            },
        );
        // A kill switch engaged before a restart stays engaged (SPEC rule 6).
        state.trading().restore_kill_switch().await;
        // So do limits the user tightened.
        state.restore_limit_overrides().await;
        Ok(state)
    }

    pub fn config(&self) -> &Config {
        &self.0.config
    }
    pub fn bridge(&self) -> &BridgeClient {
        &self.0.bridge
    }
    pub fn audit(&self) -> &Audit {
        &self.0.audit
    }
    pub fn agent(&self) -> &Agent {
        &self.0.agent
    }
    pub fn clock(&self) -> &ServerClock {
        &self.0.clock
    }
    pub fn resolver(&self) -> &SymbolResolver {
        &self.0.resolver
    }
    /// The broker account and its symbol maps.
    pub fn broker(&self) -> &Arc<BrokerService> {
        &self.0.broker
    }
    pub fn trading(&self) -> &Arc<Trading> {
        &self.0.trading
    }
    /// Picks the venue's trading instance for a proposal, token or ticket (spec-alpaca).
    pub fn router(&self) -> &Arc<crate::venue::VenueRouter> {
        &self.0.router
    }
    /// Why Alpaca is off; `None` when it is on.
    pub fn alpaca_off(&self) -> Option<&str> {
        self.0.alpaca_off.as_deref()
    }
    /// Market views, the calendar and news; `None` when they are turned off.
    pub fn market(&self) -> Option<&Arc<MarketService>> {
        self.0.market.as_ref()
    }
    /// `market` object of `GET /api/status`.
    pub fn market_status(&self) -> Value {
        match &self.0.market {
            Some(m) => json!({
                "enabled": true,
                "news_provider": m.news_provider().unwrap_or("none"),
                "news_configured": m.news_configured(),
            }),
            None => json!({ "enabled": false, "news_provider": "none", "news_configured": false }),
        }
    }
    /// Server events (kill switch, trade results). The WebSocket route subscribes here.
    pub fn events(&self) -> &EventBus {
        &self.0.events
    }
    pub fn release_guard(&self) -> Arc<dyn ReleaseGuard> {
        self.0
            .release_guard
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    /// Install the guard that authorises kill-switch releases (the auth task).
    pub fn set_release_guard(&self, guard: Arc<dyn ReleaseGuard>) {
        *self
            .0
            .release_guard
            .write()
            .unwrap_or_else(|p| p.into_inner()) = guard;
    }
    /// The login service; `None` when `[auth] enabled = false`.
    pub fn auth(&self) -> Option<&Arc<AuthService>> {
        self.0.auth.as_ref()
    }
    /// `auth` object of `GET /api/status`: whether login is on and who the caller is.
    pub fn auth_status(&self, session: Option<&Session>) -> Value {
        json!({
            "enabled": self.0.config.auth.enabled,
            "username": session.map(|s| s.username.as_str()),
        })
    }
    /// Desk mode, when `[desk]` is on.
    pub fn desk(&self) -> Option<&Arc<crate::desk::Desk>> {
        self.0.desk.as_ref()
    }
    /// The provider Friday answers on, and whether it is billed (Claude, so the Desk cap applies).
    pub fn desk_provider(&self) -> Result<(Arc<dyn jarvis_llm::LlmProvider>, bool), String> {
        let name = &self.0.config.desk.provider;
        let entry = self
            .provider(name)
            .ok_or_else(|| format!("no [[llm.providers]] entry named '{name}'"))?;
        match &entry.slot {
            ProviderSlot::Available(p) => Ok((
                p.clone(),
                entry.config.kind == jarvis_llm::ProviderKind::Anthropic,
            )),
            ProviderSlot::Unavailable(reason) => Err(format!("provider '{name}': {reason}")),
        }
    }
    /// The provider the learning review asks, with `[learning] model` in place of a Claude
    /// provider's own model, and whether it is metered (Claude).
    pub fn learning_provider(&self) -> Result<(Arc<dyn jarvis_llm::LlmProvider>, bool), String> {
        let cfg = &self.0.config.learning;
        if !cfg.enabled {
            return Err("the learning review is off ([learning] enabled = false)".into());
        }
        let entry = self
            .provider(&cfg.provider)
            .ok_or_else(|| format!("no [[llm.providers]] entry named '{}'", cfg.provider))?;
        let ProviderSlot::Available(p) = &entry.slot else {
            return Err(format!("provider '{}' is not available", cfg.provider));
        };
        let claude = entry.config.kind == jarvis_llm::ProviderKind::Anthropic;
        if !claude || cfg.model.is_empty() || cfg.model == entry.config.model {
            return Ok((p.clone(), claude));
        }
        let mut c = entry.config.clone();
        c.model = cfg.model.clone();
        jarvis_llm::build_provider(&c)
            .map(|b| (Arc::from(b), true))
            .map_err(|e| format!("provider '{}' with model {}: {e}", cfg.provider, cfg.model))
    }

    /// The user's notes, when `[notes]` is on.
    pub fn notes(&self) -> Option<&Arc<jarvis_notes::NotesService>> {
        self.0.notes.as_ref()
    }
    /// Assistant mode, when `[assistant]` is on.
    pub fn assistant(&self) -> Option<&Arc<crate::assistant::Assistant>> {
        self.0.assistant.as_ref()
    }

    /// The provider Assistant mode answers with, or why it cannot be used.
    pub fn assistant_provider(&self) -> Result<Arc<dyn jarvis_llm::LlmProvider>, String> {
        let name = &self.0.config.assistant.provider;
        match self.provider(name) {
            None => Err(format!(
                "no [[llm.providers]] entry is named '{name}' ([assistant] provider)"
            )),
            // Prices, effort and prompt caching are Claude's: another kind would be mislabelled
            // and mispriced.
            Some(entry) if entry.config.kind != ProviderKind::Anthropic => Err(format!(
                "Assistant mode needs a Claude (anthropic) provider; '{name}' is {}",
                entry.kind_str()
            )),
            Some(entry) => match &entry.slot {
                ProviderSlot::Available(p) => Ok(p.clone()),
                ProviderSlot::Unavailable(reason) => Err(reason.clone()),
            },
        }
    }

    /// The outbound-connection log.
    pub fn egress(&self) -> &Arc<EgressLog> {
        &self.0.egress
    }
    pub fn voice(&self) -> &Voice {
        &self.0.voice
    }
    /// The persona packs.
    pub fn personas(&self) -> &PersonaRegistry {
        &self.0.personas
    }
    pub fn settings_store(&self) -> &SettingsStore {
        &self.0.settings
    }

    /// The persona a user has chosen (the default if theirs no longer exists).
    pub fn persona_for(&self, settings: &UserSettings) -> &PersonaPack {
        self.0
            .personas
            .get(&settings.persona)
            .unwrap_or_else(|_| self.0.personas.default_persona())
    }

    /// The symbol resolver for a user's turns: the configured aliases with the user's own
    /// merged over them.
    pub fn resolver_for(&self, settings: &UserSettings) -> Arc<SymbolResolver> {
        if settings.aliases.is_empty() {
            self.0.resolver.clone()
        } else {
            Arc::new(self.0.resolver.with_user_aliases(&settings.aliases))
        }
    }

    /// Where a caller's settings are stored: their login name, or one shared local profile
    /// when login is off.
    pub fn settings_key(session: Option<&Session>) -> &str {
        session.map_or(LOCAL_USER, |s| s.username.as_str())
    }

    fn default_settings(&self) -> UserSettings {
        UserSettings::defaults(
            self.0.personas.default_id(),
            &self.0.config.symbols.watchlist,
        )
    }

    /// Replace values that no longer exist (a persona pack or provider that was removed).
    fn sanitize(&self, mut settings: UserSettings) -> UserSettings {
        if self.0.personas.get(&settings.persona).is_err() {
            settings.persona = self.0.personas.default_id().to_owned();
        }
        if settings
            .llm_provider
            .as_deref()
            .is_some_and(|name| self.provider(name).is_none())
        {
            settings.llm_provider = None;
        }
        // Older settings stored broker names (`XAUUSD.m`); the watchlist is canonical now.
        let book = self.0.broker.book();
        let mut watchlist: Vec<String> = Vec::with_capacity(settings.watchlist.len());
        for name in &settings.watchlist {
            if let Some(c) = book.canonical_loose(name)
                && !watchlist.contains(&c)
            {
                watchlist.push(c);
            }
        }
        settings.watchlist = watchlist;
        settings
    }

    /// The caller's settings. A storage failure is logged and answered with the defaults, so
    /// chat keeps working.
    pub async fn user_settings(&self, session: Option<&Session>) -> UserSettings {
        match self.stored_settings(session).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("reading user settings failed, using the defaults: {e:#}");
                self.default_settings()
            }
        }
    }

    async fn stored_settings(&self, session: Option<&Session>) -> anyhow::Result<UserSettings> {
        let stored = self.0.settings.get(Self::settings_key(session)).await?;
        Ok(self.sanitize(stored.unwrap_or_else(|| self.default_settings())))
    }

    /// `PUT /api/settings`: validate and store a partial update; returns the full settings.
    pub async fn update_settings(
        &self,
        session: Option<&Session>,
        patch: SettingsPatch,
    ) -> Result<UserSettings, SettingsUpdateError> {
        let key = Self::settings_key(session);
        let _write = self.0.settings.lock().await;
        let current = self
            .stored_settings(session)
            .await
            .map_err(SettingsUpdateError::Storage)?;
        let rules = Rules {
            personas: &self.0.personas,
            providers: self
                .0
                .providers
                .iter()
                .map(|p| p.config.name.as_str())
                .collect(),
            resolver: &self.0.resolver,
        };
        let next = apply_patch(current, patch, &rules).map_err(SettingsUpdateError::Invalid)?;
        self.0
            .settings
            .put(key, &next)
            .await
            .map_err(SettingsUpdateError::Storage)?;
        Ok(next)
    }

    /// Make `persona` the caller's active persona (a spoken wake phrase). `persona` must be a
    /// loaded pack.
    pub async fn set_user_persona(
        &self,
        session: Option<&Session>,
        persona: &str,
    ) -> anyhow::Result<UserSettings> {
        let key = Self::settings_key(session);
        let _write = self.0.settings.lock().await;
        let mut settings = self.stored_settings(session).await?;
        persona.clone_into(&mut settings.persona);
        let settings = self.sanitize(settings);
        self.0.settings.put(key, &settings).await?;
        Ok(settings)
    }

    /// Startup: load the stored tighten-only limit overrides into the gate. If they cannot be
    /// read, trading is switched off by an empty symbol list until they are reset (fail
    /// closed, like the kill switch).
    pub async fn restore_limit_overrides(&self) {
        match self.0.settings.load_limit_overrides().await {
            Ok(Some(mut overrides)) => {
                // Stored by an older version as broker names; limits are canonical now.
                if let Some(list) = &overrides.allowed_symbols {
                    let book = self.0.broker.book();
                    let mut canonical: Vec<String> = Vec::with_capacity(list.len());
                    for name in list {
                        if let Some(c) = book.canonical_loose(name)
                            && !canonical.contains(&c)
                        {
                            canonical.push(c);
                        }
                    }
                    overrides.allowed_symbols = Some(canonical);
                }
                let effective = self.0.trading.set_limit_overrides(Some(overrides));
                tracing::info!(?effective, "tightened limits restored");
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!(
                    "cannot read the saved limit overrides; new orders are refused until they \
                     are reset in Settings: {e:#}"
                );
                self.0.trading.set_limit_overrides(Some(PartialLimits {
                    allowed_symbols: Some(Vec::new()),
                    ..PartialLimits::default()
                }));
            }
        }
    }
    /// `voice` object of `GET /api/status`.
    pub fn voice_status(&self) -> Value {
        self.0.voice.status()
    }
    /// Fires when the server begins to stop; WebSocket tasks close on it.
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.0.shutdown
    }
    /// Permits for open WebSocket connections.
    pub fn ws_slots(&self) -> Arc<Semaphore> {
        self.0.ws_slots.clone()
    }
    /// Spend one request of `budget` for `key` (the session, or the peer address when there is
    /// none). `Err(retry_after_secs)` when the key is over its limit (NET-2).
    pub fn spend_budget(&self, budget: Budget, key: &str) -> Result<(), u64> {
        let now = self.0.trading.now_ms();
        match budget {
            Budget::Chat => self.0.chat_limiter.check(key, now),
            Budget::Transcribe => self.0.transcribe_limiter.check(key, now),
            Budget::Market => self.0.market_limiter.check(key, now),
        }
    }

    pub fn providers(&self) -> &[ProviderEntry] {
        &self.0.providers
    }
    pub fn provider(&self, name: &str) -> Option<&ProviderEntry> {
        self.0.providers.iter().find(|p| p.config.name == name)
    }

    /// True when DEMO mode is configured but the account reports a real trade mode.
    pub fn mode_mismatch(&self) -> bool {
        self.0.mode.mismatch()
    }

    /// Record the account trade mode seen on a successful bridge call (rule 7).
    pub fn observe_trade_mode(&self, seen: TradeMode) {
        self.0.mode.observe(seen);
    }

    /// Ping the bridge and run the rule-7 check on success. Does not look at the broker
    /// account (see [`AppState::ping_bridge`]).
    pub async fn ping_only(&self) -> Result<PingInfo, BridgeError> {
        let info = self.0.bridge.ping().await?;
        self.observe_trade_mode(info.trade_mode);
        self.0
            .clock
            .observe(info.server_time, chrono::Utc::now().timestamp());
        Ok(info)
    }

    /// [`AppState::ping_only`], then, on success, check which broker account MT5 is on: a
    /// different broker starts a symbol detection.
    pub async fn ping_bridge(&self) -> Result<PingInfo, BridgeError> {
        let info = self.ping_only().await?;
        self.0.broker.observe().await;
        Ok(info)
    }

    /// Start the periodic cleanup of expired previews and idle rate-limit keys. Call once
    /// from `main`; tests call [`Trading::purge`] directly.
    pub fn spawn_background_tasks(&self) -> tokio::task::JoinHandle<()> {
        let trading = self.0.trading.clone();
        let auth = self.0.auth.clone();
        let state = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PURGE_INTERVAL);
            let mut ticks: u32 = 0;
            loop {
                tick.tick().await;
                ticks = ticks.wrapping_add(1);
                trading.purge();
                // Notice an account switch even when nobody has the UI open.
                if ticks.is_multiple_of(BROKER_CHECK_EVERY_TICKS) {
                    state.0.broker.observe().await;
                }
                let now = trading.now_ms();
                state.0.chat_limiter.purge(now);
                state.0.transcribe_limiter.purge(now);
                state.0.market_limiter.purge(now);
                if let Some(auth) = &auth {
                    auth.purge().await;
                }
            }
        })
    }

    /// Load the local default model with one tiny request, so the first real question does
    /// not wait for it (FL-2). Returns the provider name when the model answered. Never
    /// fails: a model that is not running yet is only a warning.
    pub async fn warm_up(&self) -> Option<String> {
        let name = self.config().llm.warm_up_provider()?.name.clone();
        let ProviderSlot::Available(provider) = &self.provider(&name)?.slot else {
            return None;
        };
        let started = std::time::Instant::now();
        let request = jarvis_llm::ChatRequest {
            system: None,
            messages: vec![jarvis_llm::Message::User {
                text: "Reply with the single word OK.".to_owned(),
            }],
            tools: vec![],
            max_tokens: Some(8),
            temperature: Some(0.0),
            effort: None,
            cache_system: false,
        };
        match provider.chat(&request).await {
            Ok(_) => {
                tracing::info!(
                    provider = %name,
                    seconds = started.elapsed().as_secs_f32(),
                    "local model loaded"
                );
                Some(name)
            }
            Err(e) => {
                tracing::warn!(
                    provider = %name,
                    "could not load the local model at start-up ({e}); the first question will load it \
                     instead. Is Ollama running?"
                );
                None
            }
        }
    }

    /// Keep a VoiceStudio voice loaded while the server runs (FL-6); `None` without one.
    pub fn spawn_voice_keep_warm(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.0.voice.spawn_keep_warm(self.0.shutdown.subscribe())
    }

    /// [`AppState::warm_up`] in the background; start-up never waits for it.
    pub fn spawn_warm_up(&self) -> tokio::task::JoinHandle<Option<String>> {
        let state = self.clone();
        tokio::spawn(async move { state.warm_up().await })
    }

    /// Start the task that scores the market views every [`MARKET_EVALUATE_EVERY`]: open views
    /// against the M5 bars since they were made. It runs once at start-up (to catch up after
    /// a restart) and stops when the server shuts down. `None` when market views are off.
    pub fn spawn_market_evaluator(&self) -> Option<tokio::task::JoinHandle<()>> {
        let market = self.0.market.clone()?;
        let mut stop = self.0.shutdown.subscribe();
        Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(MARKET_EVALUATE_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let report = market.evaluate_open().await;
                        if report.updated > 0 {
                            tracing::info!(
                                checked = report.checked,
                                updated = report.updated,
                                "market views scored"
                            );
                        }
                    }
                    _ = stop.changed() => break,
                }
            }
        }))
    }

    /// Start the free council scan (spec-architecture-page): every `[desk] scan_every_minutes`
    /// while the US market is open, over the default watchlist and `[desk] universe`. No
    /// language model runs. `None` when Desk mode or the scan is off.
    pub fn spawn_council_scanner(&self) -> Option<tokio::task::JoinHandle<()>> {
        let desk = self.desk()?.clone();
        let minutes = self.0.config.desk.scan_every_minutes;
        if minutes == 0 {
            return None;
        }
        let state = self.clone();
        let mut stop = self.0.shutdown.subscribe();
        Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(minutes * 60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let _ = desk.decisions().settle(chrono::Utc::now().timestamp_millis()).await;
                        let resolved = desk.resolve_outcomes().await;
                        if resolved > 0 {
                            tracing::info!(resolved, "decision outcomes resolved");
                        }
                        if desk.session_status().phase != jarvis_desk::session::Phase::Open {
                            continue;
                        }
                        let settings = state.user_settings(None).await;
                        let user = crate::desk::DeskUser {
                            resolver: state.resolver_for(&settings),
                            watchlist: settings.watchlist.clone(),
                            intent: jarvis_gate::intent::Intent::Other,
                        };
                        let ids = desk.scan(&user).await;
                        tracing::info!(decisions = ids.len(), "council scan");
                    }
                    _ = stop.changed() => break,
                }
            }
        }))
    }

    /// Startup check (rule 7 / A10). Refuses to start when DEMO is configured against a real
    /// account. A bridge that is down is not fatal: the check repeats on later pings.
    pub async fn startup_check(&self) -> anyhow::Result<()> {
        match self.ping_bridge().await {
            Ok(info) => {
                if self.mode_mismatch() {
                    bail!(
                        "refusing to start: config mode is 'demo' but the MT5 account reports \
                         trade_mode '{:?}'. Use a demo account, or set mode = \"live\" \
                         deliberately.",
                        info.trade_mode
                    );
                }
                tracing::info!(
                    ea_version = %info.ea_version,
                    trade_mode = ?info.trade_mode,
                    terminal_connected = info.terminal_connected,
                    "bridge reachable"
                );
            }
            Err(e) => tracing::warn!(
                "bridge not reachable at startup ({e}); starting anyway, the account mode will \
                 be checked when the bridge comes up"
            ),
        }
        Ok(())
    }
}

/// The news service for `[market.news]`. `env` reads an environment variable. A provider whose
/// key is missing runs as "not configured" and says so in the log; the key itself is never
/// logged.
pub fn news_from_config(
    cfg: &MarketNewsConfig,
    env: impl Fn(&str) -> Option<String>,
    egress: &Arc<EgressLog>,
) -> NewsService {
    if cfg.provider == NewsProviderKind::None {
        return NewsService::disabled();
    }
    let Some(var) = cfg.key_env() else {
        return NewsService::disabled();
    };
    let key = env(var).map(|k| k.trim().to_owned()).unwrap_or_default();
    if key.is_empty() {
        tracing::warn!(
            provider = cfg.provider.as_str(),
            "the news provider is configured but {var} is empty or not set; news is off"
        );
        return NewsService::disabled();
    }
    let ttl = Duration::from_secs(cfg.cache_minutes * 60);
    let provider: Result<Box<dyn jarvis_news::NewsProvider>, _> = match cfg.provider {
        NewsProviderKind::None => return NewsService::disabled(),
        NewsProviderKind::Finnhub => Finnhub::new(&key).map(|p| Box::new(p) as _),
        NewsProviderKind::Alphavantage => AlphaVantage::new(&key).map(|p| Box::new(p) as _),
    };
    match provider {
        Ok(p) => {
            tracing::info!(provider = cfg.provider.as_str(), "news provider ready");
            NewsService::new(EgressNews::wrap(p, egress.clone())).with_ttl(ttl)
        }
        Err(e) => {
            tracing::warn!(
                provider = cfg.provider.as_str(),
                "the news provider is unusable ({e}); news is off"
            );
            NewsService::disabled()
        }
    }
}

/// What the Alpaca trading instance shares with the MT5 one.
struct AlpacaWiring<'a> {
    bridge: &'a BridgeClient,
    audit: &'a Audit,
    kill_switch: &'a KillSwitch,
    time: &'a Arc<dyn TimeSource>,
    events: &'a EventBus,
    broker: &'a Arc<BrokerService>,
    egress: &'a Arc<EgressLog>,
}

/// The Alpaca venue and its trading instance, or why there is none.
fn build_alpaca(
    config: &Config,
    client: Option<jarvis_alpaca::AlpacaClient>,
    w: AlpacaWiring<'_>,
) -> (Option<crate::venue::AlpacaRoute>, Option<String>) {
    let a = &config.alpaca;
    if !a.enabled {
        return (
            None,
            Some("Alpaca is turned off ([alpaca] enabled = false)".to_owned()),
        );
    }
    let client = match client {
        Some(c) => c,
        None => {
            let env = |name: &str| {
                std::env::var(name)
                    .ok()
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
            };
            let (Some(key_id), Some(secret_key)) = (env(&a.key_id_env), env(&a.secret_key_env))
            else {
                return (
                    None,
                    Some(format!(
                        "Alpaca keys are not set ({} and {})",
                        a.key_id_env, a.secret_key_env
                    )),
                );
            };
            let cfg = jarvis_alpaca::AlpacaConfig {
                trading_url: env(&a.trading_url_env).unwrap_or_else(|| a.trading_url.clone()),
                data_url: a.data_url.clone(),
                key_id,
                secret_key,
                feed: a.feed.clone(),
                timeout: Duration::from_secs(a.timeout_secs),
            };
            match jarvis_alpaca::AlpacaClient::new(&cfg) {
                Ok(c) => c,
                Err(e) => return (None, Some(e.to_string())),
            }
        }
    };
    if !client.is_paper() {
        tracing::warn!("Alpaca is off: the trading URL is not the paper host");
        return (
            None,
            Some("the Alpaca trading URL is not paper-api.alpaca.markets; only paper trading is allowed".to_owned()),
        );
    }
    let mut symbols: std::collections::BTreeSet<String> = a
        .symbols
        .iter()
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    if a.include_desk_universe && config.desk.enabled {
        symbols.extend(
            config
                .desk
                .universe
                .iter()
                .map(|s| s.trim().to_ascii_uppercase())
                .filter(|s| !s.is_empty()),
        );
    }
    let venue = Arc::new(crate::alpaca_venue::AlpacaVenue::new(
        client,
        w.egress.clone(),
        a.extended_hours,
        a.timeout_secs * 1000,
    ));
    let list: Vec<String> = symbols.iter().cloned().collect();
    let trading = Trading::new(TradingParts {
        bridge: w.bridge.clone(),
        reads: Some(venue.clone()),
        venue: crate::venue::VenueKind::Alpaca,
        max_notional: Some(a.max_notional_usd),
        write_sink: Some(venue.clone()),
        clock: ServerClock::default(),
        // Paper only: the venue reports a demo account, so this monitor never sees a live one.
        mode: Arc::new(ModeMonitor::new(crate::config::Mode::Demo)),
        audit: w.audit.clone(),
        limits: a.limits(&list),
        config: config.trading.clone(),
        kill_switch: w.kill_switch.clone(),
        time: w.time.clone(),
        events: w.events.clone(),
        broker: w.broker.clone(),
    });
    (
        Some(crate::venue::AlpacaRoute {
            trading,
            venue,
            symbols,
        }),
        None,
    )
}

#[cfg(test)]
mod news_config_tests {
    use super::*;

    fn cfg(provider: NewsProviderKind, env: Option<&str>) -> MarketNewsConfig {
        MarketNewsConfig {
            provider,
            api_key_env: env.map(str::to_owned),
            cache_minutes: 10,
        }
    }

    fn env_with(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[tokio::test]
    async fn no_provider_means_no_news_whatever_the_key_variable_says() {
        let news = news_from_config(
            &cfg(NewsProviderKind::None, Some("FINNHUB_API_KEY")),
            env_with(&[("FINNHUB_API_KEY", "abc")]),
            &EgressLog::new(),
        );
        assert!(!news.is_configured());
    }

    #[tokio::test]
    async fn a_provider_without_a_key_runs_as_not_configured() {
        let none = news_from_config(
            &cfg(NewsProviderKind::Finnhub, None),
            env_with(&[]),
            &EgressLog::new(),
        );
        assert!(!none.is_configured());
        let empty = news_from_config(
            &cfg(NewsProviderKind::Finnhub, None),
            env_with(&[("FINNHUB_API_KEY", "")]),
            &EgressLog::new(),
        );
        assert!(!empty.is_configured());
        let blank = news_from_config(
            &cfg(NewsProviderKind::Finnhub, None),
            env_with(&[("FINNHUB_API_KEY", "   ")]),
            &EgressLog::new(),
        );
        assert!(!blank.is_configured());
        assert_eq!(blank.provider_name(), None);
    }

    #[tokio::test]
    async fn a_provider_with_a_key_is_configured_and_each_has_its_own_variable() {
        let env = env_with(&[
            ("FINNHUB_API_KEY", "fh-key"),
            ("ALPHAVANTAGE_API_KEY", "av-key"),
            ("MY_NEWS_KEY", "mine"),
        ]);
        let finnhub = news_from_config(
            &cfg(NewsProviderKind::Finnhub, None),
            &env,
            &EgressLog::new(),
        );
        assert_eq!(finnhub.provider_name(), Some("finnhub"));
        let alpha = news_from_config(
            &cfg(NewsProviderKind::Alphavantage, None),
            &env,
            &EgressLog::new(),
        );
        assert_eq!(alpha.provider_name(), Some("alphavantage"));
        let custom = news_from_config(
            &cfg(NewsProviderKind::Finnhub, Some("MY_NEWS_KEY")),
            &env,
            &EgressLog::new(),
        );
        assert!(custom.is_configured());
        // The named variable is the one read: the usual one is ignored.
        let wrong = news_from_config(
            &cfg(NewsProviderKind::Finnhub, Some("OTHER")),
            env_with(&[("FINNHUB_API_KEY", "fh-key")]),
            &EgressLog::new(),
        );
        assert!(!wrong.is_configured());
    }
}

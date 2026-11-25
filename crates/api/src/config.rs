//! Configuration: non-secret settings from `config/jarvis.toml`, secrets from the
//! environment (`.env` is loaded first).

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use jarvis_bridge_client::BridgeKey;
use jarvis_bridge_client::auth::KEY_ENV;
use jarvis_gate::Limits;
use jarvis_llm::{ProviderConfig, ProviderKind};
use serde::Deserialize;

use crate::symbols::{alias_key, clean};

/// Env var that overrides the config file location.
pub const CONFIG_ENV: &str = "JARVIS_CONFIG";
/// Default config file, relative to the working directory.
pub const DEFAULT_CONFIG_PATH: &str = "config/jarvis.toml";

/// Operating mode. LIVE needs an explicit setting; the default is DEMO (rule 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Demo,
    Live,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Demo => "demo",
            Mode::Live => "live",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub bridge: BridgeSection,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub symbols: SymbolsConfig,
    pub llm: LlmConfig,
    /// `[limits]`. When the section is absent the limits fail closed (see
    /// [`Config::effective_limits`]).
    #[serde(default)]
    pub limits: Option<Limits>,
    #[serde(default)]
    pub trading: TradingConfig,
    /// `[auth]`. Login is on unless the section says otherwise.
    #[serde(default)]
    pub auth: AuthConfig,
    /// `[voice]`. Absent means voice is off.
    #[serde(default)]
    pub voice: Option<VoiceSection>,
    /// `[personas]`: where the persona packs live and which one is the default.
    #[serde(default)]
    pub personas: PersonasConfig,
    /// `[market]`: intraday market views, the economic calendar and optional news.
    #[serde(default)]
    pub market: MarketConfig,
    /// `[notes]`: the user's markdown notes (galaxy page and the `search_notes` tool).
    #[serde(default)]
    pub notes: NotesConfig,
    /// `[assistant]`: Assistant mode (Claude, notes, captures, 3D models).
    #[serde(default)]
    pub assistant: AssistantConfig,
    /// `[desk]`: Desk mode, the nine-agent Nimbus Desk (spec-nimbus-desk).
    #[serde(default)]
    pub desk: DeskConfig,
    /// The architecture page's learning review on Claude (spec-architecture-page).
    #[serde(default)]
    pub learning: LearningConfig,
    /// `[alpaca]`: US stocks on an Alpaca paper account, a second venue (spec-alpaca).
    #[serde(default)]
    pub alpaca: AlpacaSection,
}

/// `[assistant]` (spec-assistant-mode). On by default; it needs the provider named here to be
/// usable (for Claude, `ANTHROPIC_API_KEY` set), and says so when it is not.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// An `[[llm.providers]]` name. Its model answers every assistant turn.
    #[serde(default = "default_assistant_provider")]
    pub provider: String,
    /// `output_config.effort` for chat turns and for 3D builds (`low`..`max`).
    #[serde(default = "default_chat_effort")]
    pub chat_effort: String,
    #[serde(default = "default_build_effort")]
    pub build_effort: String,
    /// Assistant mode stops calling the provider once this month's cost reaches it.
    #[serde(default = "default_monthly_budget")]
    pub monthly_budget_usd: f64,
    /// US dollars per million tokens, used to count the cost. Claude Opus 5.5 by default.
    #[serde(default = "default_price_input")]
    pub price_input_per_mtok: f64,
    #[serde(default = "default_price_output")]
    pub price_output_per_mtok: f64,
    #[serde(default = "default_price_cache_read")]
    pub price_cache_read_per_mtok: f64,
    #[serde(default = "default_price_cache_write")]
    pub price_cache_write_per_mtok: f64,
    /// Where built 3D models are kept (the newest 50).
    #[serde(default = "default_models_dir")]
    pub models_dir: PathBuf,
    /// The file that keeps this month's spending.
    #[serde(default = "default_spend_file")]
    pub spend_file: PathBuf,
    /// The largest 3D model code accepted, in KB.
    #[serde(default = "default_max_model_kb")]
    pub max_model_kb: usize,
    /// The sub-folder of `[notes] dir` that "remember that ..." writes to.
    #[serde(default = "default_captures_dir")]
    pub captures_dir: String,
}

fn default_assistant_provider() -> String {
    "claude".to_owned()
}
fn default_chat_effort() -> String {
    "low".to_owned()
}
fn default_build_effort() -> String {
    // Measured on Claude Opus 5.5: at `high` a model took 5,000-12,000 output tokens (mostly
    // thinking) and 45-155 s, and complex ones passed the provider timeout. `medium` thinks less.
    "medium".to_owned()
}
fn default_monthly_budget() -> f64 {
    20.0
}
fn default_price_input() -> f64 {
    4.0
}
fn default_price_output() -> f64 {
    20.0
}
fn default_price_cache_read() -> f64 {
    0.2
}
fn default_price_cache_write() -> f64 {
    5.0
}
fn default_models_dir() -> PathBuf {
    PathBuf::from("data/models3d")
}
fn default_spend_file() -> PathBuf {
    PathBuf::from("data/assistant-spend.json")
}
fn default_max_model_kb() -> usize {
    64
}
fn default_captures_dir() -> String {
    "captures".to_owned()
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: default_assistant_provider(),
            chat_effort: default_chat_effort(),
            build_effort: default_build_effort(),
            monthly_budget_usd: default_monthly_budget(),
            price_input_per_mtok: default_price_input(),
            price_output_per_mtok: default_price_output(),
            price_cache_read_per_mtok: default_price_cache_read(),
            price_cache_write_per_mtok: default_price_cache_write(),
            models_dir: default_models_dir(),
            spend_file: default_spend_file(),
            max_model_kb: default_max_model_kb(),
            captures_dir: default_captures_dir(),
        }
    }
}

const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

impl AssistantConfig {
    pub fn prices(&self) -> crate::spend::Prices {
        crate::spend::Prices {
            input: self.price_input_per_mtok,
            output: self.price_output_per_mtok,
            cache_read: self.price_cache_read_per_mtok,
            cache_write: self.price_cache_write_per_mtok,
        }
    }

    /// A provider name that matches no `[[llm.providers]]` entry is not an error here: Assistant
    /// mode then reports itself unavailable and says why, and Trading mode is unaffected.
    fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        for (name, value) in [
            ("chat_effort", &self.chat_effort),
            ("build_effort", &self.build_effort),
        ] {
            if !EFFORTS.contains(&value.as_str()) {
                bail!("assistant.{name} must be one of {}", EFFORTS.join(", "));
            }
        }
        let money = [
            ("monthly_budget_usd", self.monthly_budget_usd),
            ("price_input_per_mtok", self.price_input_per_mtok),
            ("price_output_per_mtok", self.price_output_per_mtok),
            ("price_cache_read_per_mtok", self.price_cache_read_per_mtok),
            (
                "price_cache_write_per_mtok",
                self.price_cache_write_per_mtok,
            ),
        ];
        for (name, value) in money {
            if !(0.0..=100_000.0).contains(&value) {
                bail!("assistant.{name} must be between 0 and 100000");
            }
        }
        if !(1..=1024).contains(&self.max_model_kb) {
            bail!("assistant.max_model_kb must be between 1 and 1024");
        }
        let plain = !self.captures_dir.is_empty()
            && !self.captures_dir.starts_with('.')
            && self
                .captures_dir
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !plain {
            bail!("assistant.captures_dir must be one plain folder name, like \"captures\"");
        }
        Ok(())
    }
}

/// `[learning]`: the learning review. Claude reads statistics the code computed and explains
/// and ranks proposals; nothing is applied without the trader. Runs only when asked.
#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LearningConfig {
    pub enabled: bool,
    /// An `[[llm.providers]]` name. Its model is replaced by `model` for Claude providers.
    pub provider: String,
    /// Empty = the provider's own model.
    pub model: String,
    pub monthly_budget_usd: f64,
    pub price_input_per_mtok: f64,
    pub price_output_per_mtok: f64,
    pub price_cache_read_per_mtok: f64,
    pub price_cache_write_per_mtok: f64,
    pub spend_file: String,
    pub max_tokens: u32,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "claude".into(),
            model: "claude-sonnet-5-5".into(),
            monthly_budget_usd: 2.0,
            price_input_per_mtok: 2.0,
            price_output_per_mtok: 10.0,
            price_cache_read_per_mtok: 0.2,
            price_cache_write_per_mtok: 2.5,
            spend_file: "data/learning-spend.json".into(),
            max_tokens: 3000,
        }
    }
}

impl LearningConfig {
    pub fn prices(&self) -> crate::spend::Prices {
        crate::spend::Prices {
            input: self.price_input_per_mtok,
            output: self.price_output_per_mtok,
            cache_read: self.price_cache_read_per_mtok,
            cache_write: self.price_cache_write_per_mtok,
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !(self.monthly_budget_usd.is_finite() && (0.0..=50.0).contains(&self.monthly_budget_usd))
        {
            bail!("learning.monthly_budget_usd must be between 0 and 50");
        }
        if !(500..=8000).contains(&self.max_tokens) {
            bail!("learning.max_tokens must be between 500 and 8000");
        }
        Ok(())
    }
}

/// `[desk]` (spec-nimbus-desk, ADR-0023). On by default; Friday answers on the provider named
/// here, and Desk mode says so when it is not usable.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeskConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// An `[[llm.providers]]` name. Claude by default; any kind works.
    #[serde(default = "default_assistant_provider")]
    pub provider: String,
    /// `output_config.effort` for Friday's turns (`low`..`max`; Claude only).
    #[serde(default = "default_desk_effort")]
    pub effort: String,
    /// Desk mode stops calling a Claude provider once this month's cost reaches it.
    #[serde(default = "default_monthly_budget")]
    pub monthly_budget_usd: f64,
    #[serde(default = "default_price_input")]
    pub price_input_per_mtok: f64,
    #[serde(default = "default_price_output")]
    pub price_output_per_mtok: f64,
    #[serde(default = "default_price_cache_read")]
    pub price_cache_read_per_mtok: f64,
    #[serde(default = "default_price_cache_write")]
    pub price_cache_write_per_mtok: f64,
    /// The file that keeps this month's Desk spending.
    #[serde(default = "default_desk_spend_file")]
    pub spend_file: PathBuf,
    /// US stocks Scout scans (with the watchlist). Sentinel treats them as US-session symbols.
    #[serde(default)]
    pub universe: Vec<String>,
    /// Index symbols Atlas reports (canonical names).
    #[serde(default = "default_desk_indices")]
    pub indices: Vec<String>,
    /// Sentinel: percent of equity at risk per trade (the gate's `max_risk_percent` still applies).
    #[serde(default = "default_desk_risk")]
    pub risk_percent: f64,
    /// Sentinel: open Desk positions at most.
    #[serde(default = "default_desk_positions")]
    pub max_open_positions: u32,
    /// Sentinel: no new trades this many minutes before the US close.
    #[serde(default = "default_desk_close_buffer")]
    pub no_new_trades_before_close_minutes: i64,
    /// Sentinel: smallest reward to risk for a plan with a target (spec-architecture-page).
    #[serde(default = "default_desk_min_rr")]
    pub min_reward_risk: f64,
    /// Council: smallest |score| that passes (spec-architecture-page).
    #[serde(default = "default_desk_min_council")]
    pub min_council_score: f64,
    /// Council: members that must take a side.
    #[serde(default = "default_desk_min_sides")]
    pub min_non_neutral: usize,
    /// Minutes between free council scans while the US market is open; 0 turns it off.
    #[serde(default = "default_desk_scan_minutes")]
    pub scan_every_minutes: u64,
    /// The MT5 account the master bot may trade on (ADR-0025); 0 turns the lock off. The UI can
    /// change it; this is the default.
    #[serde(default = "default_master_account")]
    pub master_account: u64,
    #[serde(default = "default_master_broker")]
    pub master_broker: String,
    /// Extra FOMC decision days, `YYYY-MM-DD` (the 2026 schedule is built in).
    #[serde(default)]
    pub fomc_dates: Vec<String>,
    /// Extra NYSE holidays, `YYYY-MM-DD` (2026 and 2027 are built in).
    #[serde(default)]
    pub holidays: Vec<String>,
    /// The environment variable that holds the Finnhub key (insiders, fundamentals, filings).
    #[serde(default = "default_finnhub_env")]
    pub finnhub_key_env: String,
    /// Movers Scout reports.
    #[serde(default = "default_scout_top")]
    pub scout_top: usize,
}

fn default_desk_effort() -> String {
    "medium".to_owned()
}
fn default_desk_spend_file() -> PathBuf {
    PathBuf::from("data/desk-spend.json")
}
fn default_desk_indices() -> Vec<String> {
    vec!["US500".to_owned(), "NAS100".to_owned()]
}
fn default_desk_risk() -> f64 {
    0.5
}
fn default_desk_positions() -> u32 {
    2
}
fn default_desk_min_rr() -> f64 {
    1.5
}

fn default_desk_min_council() -> f64 {
    0.30
}

fn default_desk_min_sides() -> usize {
    3
}

fn default_desk_scan_minutes() -> u64 {
    15
}

fn default_master_account() -> u64 {
    53_107_904
}

fn default_master_broker() -> String {
    "IC Markets".into()
}

fn default_desk_close_buffer() -> i64 {
    10
}
fn default_finnhub_env() -> String {
    "FINNHUB_API_KEY".to_owned()
}
fn default_scout_top() -> usize {
    8
}

impl Default for DeskConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: default_assistant_provider(),
            effort: default_desk_effort(),
            monthly_budget_usd: default_monthly_budget(),
            price_input_per_mtok: default_price_input(),
            price_output_per_mtok: default_price_output(),
            price_cache_read_per_mtok: default_price_cache_read(),
            price_cache_write_per_mtok: default_price_cache_write(),
            spend_file: default_desk_spend_file(),
            universe: Vec::new(),
            indices: default_desk_indices(),
            risk_percent: default_desk_risk(),
            max_open_positions: default_desk_positions(),
            no_new_trades_before_close_minutes: default_desk_close_buffer(),
            min_reward_risk: default_desk_min_rr(),
            min_council_score: default_desk_min_council(),
            min_non_neutral: default_desk_min_sides(),
            scan_every_minutes: default_desk_scan_minutes(),
            master_account: default_master_account(),
            master_broker: default_master_broker(),
            fomc_dates: Vec::new(),
            holidays: Vec::new(),
            finnhub_key_env: default_finnhub_env(),
            scout_top: default_scout_top(),
        }
    }
}

impl DeskConfig {
    pub fn prices(&self) -> crate::spend::Prices {
        crate::spend::Prices {
            input: self.price_input_per_mtok,
            output: self.price_output_per_mtok,
            cache_read: self.price_cache_read_per_mtok,
            cache_write: self.price_cache_write_per_mtok,
        }
    }

    pub fn rules(&self) -> jarvis_desk::sentinel::Rules {
        jarvis_desk::sentinel::Rules {
            risk_percent: self.risk_percent,
            max_open_positions: self.max_open_positions,
            no_new_trades_before_close_minutes: self.no_new_trades_before_close_minutes,
            min_reward_risk: self.min_reward_risk,
        }
    }

    pub fn council_rules(&self) -> jarvis_desk::council::CouncilRules {
        jarvis_desk::council::CouncilRules {
            min_score: self.min_council_score,
            min_non_neutral: self.min_non_neutral,
        }
    }

    fn dates(list: &[String], name: &str) -> anyhow::Result<Vec<chrono::NaiveDate>> {
        list.iter()
            .map(|d| {
                chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")
                    .map_err(|_| anyhow!("desk.{name}: '{d}' is not a YYYY-MM-DD date"))
            })
            .collect()
    }

    pub fn fomc_days(&self) -> Vec<chrono::NaiveDate> {
        Self::dates(&self.fomc_dates, "fomc_dates").unwrap_or_default()
    }

    pub fn exchange(&self) -> jarvis_desk::session::Exchange {
        let mut x = jarvis_desk::session::Exchange::default();
        x.holidays
            .extend(Self::dates(&self.holidays, "holidays").unwrap_or_default());
        x
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if !EFFORTS.contains(&self.effort.as_str()) {
            bail!("desk.effort must be one of {}", EFFORTS.join(", "));
        }
        let money = [
            ("monthly_budget_usd", self.monthly_budget_usd),
            ("price_input_per_mtok", self.price_input_per_mtok),
            ("price_output_per_mtok", self.price_output_per_mtok),
            ("price_cache_read_per_mtok", self.price_cache_read_per_mtok),
            (
                "price_cache_write_per_mtok",
                self.price_cache_write_per_mtok,
            ),
        ];
        for (name, value) in money {
            if !(0.0..=100_000.0).contains(&value) {
                bail!("desk.{name} must be between 0 and 100000");
            }
        }
        if !(0.01..=5.0).contains(&self.risk_percent) {
            bail!("desk.risk_percent must be between 0.01 and 5");
        }
        if !(1..=50).contains(&self.max_open_positions) {
            bail!("desk.max_open_positions must be between 1 and 50");
        }
        if !(self.min_reward_risk.is_finite() && (0.5..=5.0).contains(&self.min_reward_risk)) {
            bail!("desk.min_reward_risk must be between 0.5 and 5");
        }
        if !(self.min_council_score.is_finite() && (0.05..=0.9).contains(&self.min_council_score)) {
            bail!("desk.min_council_score must be between 0.05 and 0.9");
        }
        if self.scan_every_minutes != 0 && !(5..=240).contains(&self.scan_every_minutes) {
            bail!("desk.scan_every_minutes must be 0 (off) or between 5 and 240");
        }
        if !(1..=6).contains(&self.min_non_neutral) {
            bail!("desk.min_non_neutral must be between 1 and 6");
        }
        if !(0..=390).contains(&self.no_new_trades_before_close_minutes) {
            bail!("desk.no_new_trades_before_close_minutes must be between 0 and 390");
        }
        if !(1..=50).contains(&self.scout_top) {
            bail!("desk.scout_top must be between 1 and 50");
        }
        if self.universe.len() > 50 {
            bail!("desk.universe holds at most 50 symbols");
        }
        Self::dates(&self.fomc_dates, "fomc_dates")?;
        Self::dates(&self.holidays, "holidays")?;
        Ok(())
    }
}

/// `[alpaca]` (spec-alpaca, ADR-0024): US stocks on an Alpaca **paper** account, as a second venue
/// next to MT5. On by default; without the keys it reports itself off and sends nothing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlpacaSection {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Only `"paper"` is accepted; live Alpaca needs its own ADR.
    #[serde(default = "default_alpaca_mode")]
    pub mode: String,
    /// Environment variables that hold the trading URL and the keys.
    #[serde(default = "default_alpaca_url_env")]
    pub trading_url_env: String,
    #[serde(default = "default_alpaca_key_env")]
    pub key_id_env: String,
    #[serde(default = "default_alpaca_secret_env")]
    pub secret_key_env: String,
    /// Used when `trading_url_env` is not set.
    #[serde(default = "default_alpaca_trading_url")]
    pub trading_url: String,
    #[serde(default = "default_alpaca_data_url")]
    pub data_url: String,
    /// `iex` (free) or `sip` (subscription).
    #[serde(default = "default_alpaca_feed")]
    pub feed: String,
    /// US stocks that trade on Alpaca (canonical names, e.g. `NVDA`).
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Also send `[desk] universe` to Alpaca.
    #[serde(default = "default_true")]
    pub include_desk_universe: bool,
    /// Allow orders outside the regular session. Off: no Alpaca order while the market is closed.
    #[serde(default)]
    pub extended_hours: bool,
    /// Limits for Alpaca orders (the `[limits]` section stays MT5's).
    #[serde(default = "default_alpaca_max_shares")]
    pub max_shares: f64,
    #[serde(default = "default_alpaca_max_notional")]
    pub max_notional_usd: f64,
    #[serde(default = "default_alpaca_max_positions")]
    pub max_positions: u32,
    #[serde(default = "default_alpaca_max_daily_loss")]
    pub max_daily_loss_usd: f64,
    #[serde(default = "default_alpaca_timeout")]
    pub timeout_secs: u64,
}

fn default_alpaca_mode() -> String {
    "paper".to_owned()
}
fn default_alpaca_url_env() -> String {
    "ALPACA_TRADING_URL".to_owned()
}
fn default_alpaca_key_env() -> String {
    "ALPACA_KEY_ID".to_owned()
}
fn default_alpaca_secret_env() -> String {
    "ALPACA_SECRET_KEY".to_owned()
}
fn default_alpaca_trading_url() -> String {
    jarvis_alpaca::DEFAULT_TRADING_URL.to_owned()
}
fn default_alpaca_data_url() -> String {
    jarvis_alpaca::DEFAULT_DATA_URL.to_owned()
}
fn default_alpaca_feed() -> String {
    "iex".to_owned()
}
fn default_alpaca_max_shares() -> f64 {
    100.0
}
fn default_alpaca_max_notional() -> f64 {
    10_000.0
}
fn default_alpaca_max_positions() -> u32 {
    5
}
fn default_alpaca_max_daily_loss() -> f64 {
    500.0
}
fn default_alpaca_timeout() -> u64 {
    10
}

impl Default for AlpacaSection {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: default_alpaca_mode(),
            trading_url_env: default_alpaca_url_env(),
            key_id_env: default_alpaca_key_env(),
            secret_key_env: default_alpaca_secret_env(),
            trading_url: default_alpaca_trading_url(),
            data_url: default_alpaca_data_url(),
            feed: default_alpaca_feed(),
            symbols: Vec::new(),
            include_desk_universe: true,
            extended_hours: false,
            max_shares: default_alpaca_max_shares(),
            max_notional_usd: default_alpaca_max_notional(),
            max_positions: default_alpaca_max_positions(),
            max_daily_loss_usd: default_alpaca_max_daily_loss(),
            timeout_secs: default_alpaca_timeout(),
        }
    }
}

impl AlpacaSection {
    /// The gate limits of the Alpaca venue. `allowed_symbols` are the Alpaca symbols.
    pub fn limits(&self, symbols: &[String]) -> Limits {
        Limits {
            max_lot: self.max_shares,
            max_positions: self.max_positions,
            max_daily_loss: self.max_daily_loss_usd,
            allowed_symbols: symbols.to_vec(),
            trading_hours: None,
            max_risk_percent: jarvis_gate::DEFAULT_MAX_RISK_PERCENT,
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.mode != "paper" {
            bail!("alpaca.mode must be \"paper\": live Alpaca trading is not supported");
        }
        if !(self.feed == "iex" || self.feed == "sip") {
            bail!("alpaca.feed must be \"iex\" or \"sip\"");
        }
        if !(1.0..=100_000.0).contains(&self.max_shares) || self.max_shares.fract() != 0.0 {
            bail!("alpaca.max_shares must be a whole number from 1 to 100000");
        }
        if !(1.0..=10_000_000.0).contains(&self.max_notional_usd) {
            bail!("alpaca.max_notional_usd must be between 1 and 10000000");
        }
        if !(1..=100).contains(&self.max_positions) {
            bail!("alpaca.max_positions must be between 1 and 100");
        }
        if !(0.0..=10_000_000.0).contains(&self.max_daily_loss_usd) {
            bail!("alpaca.max_daily_loss_usd must be between 0 and 10000000");
        }
        if !(1..=60).contains(&self.timeout_secs) {
            bail!("alpaca.timeout_secs must be between 1 and 60");
        }
        if self.symbols.len() > 200 {
            bail!("alpaca.symbols holds at most 200 symbols");
        }
        Ok(())
    }
}

/// `[notes]`. On by default, reading `notes/`; a missing folder only means "no notes yet".
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Folder of `.md` files; sub-folders become groups. Read only, never written.
    #[serde(default = "default_notes_dir")]
    pub dir: PathBuf,
    /// Files beyond this many are skipped (at most 3000).
    #[serde(default = "default_max_notes")]
    pub max_notes: usize,
    /// Larger files are skipped.
    #[serde(default = "default_max_note_kb")]
    pub max_file_kb: u64,
}

fn default_notes_dir() -> PathBuf {
    PathBuf::from("notes")
}
fn default_max_notes() -> usize {
    2000
}
fn default_max_note_kb() -> u64 {
    256
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: default_notes_dir(),
            max_notes: default_max_notes(),
            max_file_kb: default_max_note_kb(),
        }
    }
}

impl NotesConfig {
    pub fn limits(&self) -> jarvis_notes::Limits {
        jarvis_notes::Limits {
            // The galaxy lays notes out in the browser (O(n^2) per step): keep it responsive.
            max_notes: self.max_notes.clamp(1, 3000),
            max_file_bytes: self.max_file_kb.clamp(1, 4096) * 1024,
        }
    }
}

/// `[market]`. Views are on by default; news is off until a provider is chosen.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketConfig {
    /// Turn market views off: the routes answer 404 and the model gets no market tools.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// How long a view stays valid, in minutes (15 to 1440).
    #[serde(default = "default_horizon_minutes")]
    pub horizon_minutes: i64,
    /// A high-impact event this many minutes before or after now makes a view stand aside
    /// (0 to 240).
    #[serde(default = "default_blackout_minutes")]
    pub event_blackout_minutes: i64,
    /// The least important calendar events that are fetched: low, moderate or high.
    #[serde(default = "default_calendar_importance")]
    pub calendar_min_importance: jarvis_bridge_client::protocol::Importance,
    /// `GET /api/market/view` and `/api/market/news` requests per minute and session (1 to 10000).
    #[serde(default = "default_market_rate_limit")]
    pub rate_limit_per_minute: usize,
    #[serde(default)]
    pub news: MarketNewsConfig,
    /// Advanced: overrides for the scoring model (weights, thresholds, ATR multiples). See
    /// `jarvis_market::ViewConfig` for the keys. `horizon_minutes` and `event_blackout_minutes`
    /// above win over the same keys here.
    #[serde(default)]
    pub view: jarvis_market::ViewConfig,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            horizon_minutes: default_horizon_minutes(),
            event_blackout_minutes: default_blackout_minutes(),
            calendar_min_importance: default_calendar_importance(),
            rate_limit_per_minute: default_market_rate_limit(),
            news: MarketNewsConfig::default(),
            view: jarvis_market::ViewConfig::default(),
        }
    }
}

impl MarketConfig {
    /// The scoring model with the `[market]` horizon and blackout applied.
    pub fn view_config(&self) -> jarvis_market::ViewConfig {
        let mut cfg = self.view.clone();
        cfg.horizon_minutes = self.horizon_minutes;
        cfg.event_blackout_minutes = self.event_blackout_minutes;
        cfg
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !(15..=1440).contains(&self.horizon_minutes) {
            bail!("market.horizon_minutes must be between 15 and 1440");
        }
        if !(0..=240).contains(&self.event_blackout_minutes) {
            bail!("market.event_blackout_minutes must be between 0 and 240");
        }
        if !(1..=10_000).contains(&self.rate_limit_per_minute) {
            bail!("market.rate_limit_per_minute must be between 1 and 10000");
        }
        if !(1..=240).contains(&self.news.cache_minutes) {
            bail!("market.news.cache_minutes must be between 1 and 240");
        }
        if let Some(name) = &self.news.api_key_env
            && (name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        {
            bail!(
                "market.news.api_key_env must be the NAME of an environment variable                  (letters, digits and underscores), not the key itself"
            );
        }
        // TOML can spell inf and nan; they serialise to null.
        let model = serde_json::to_value(&self.view).context("market.view")?;
        if model
            .as_object()
            .is_some_and(|o| o.values().any(serde_json::Value::is_null))
        {
            bail!("market.view has a value that is not a finite number");
        }
        Ok(())
    }
}

/// The news provider of the market view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NewsProviderKind {
    #[default]
    None,
    Finnhub,
    Alphavantage,
}

impl NewsProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NewsProviderKind::None => "none",
            NewsProviderKind::Finnhub => "finnhub",
            NewsProviderKind::Alphavantage => "alphavantage",
        }
    }

    /// The variable that holds the key unless `api_key_env` names another one.
    pub fn default_key_env(self) -> Option<&'static str> {
        match self {
            NewsProviderKind::None => None,
            NewsProviderKind::Finnhub => Some("FINNHUB_API_KEY"),
            NewsProviderKind::Alphavantage => Some("ALPHAVANTAGE_API_KEY"),
        }
    }
}

/// `[market.news]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketNewsConfig {
    /// `none`, `finnhub` or `alphavantage`.
    #[serde(default)]
    pub provider: NewsProviderKind,
    /// Name of the environment variable that holds the provider's key. Default: the
    /// provider's usual one (`FINNHUB_API_KEY`, `ALPHAVANTAGE_API_KEY`).
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// How long a news summary is reused, in minutes (1 to 240).
    #[serde(default = "default_news_cache_minutes")]
    pub cache_minutes: u64,
}

impl Default for MarketNewsConfig {
    fn default() -> Self {
        Self {
            provider: NewsProviderKind::None,
            api_key_env: None,
            cache_minutes: default_news_cache_minutes(),
        }
    }
}

impl MarketNewsConfig {
    /// The environment variable to read the key from, when a provider is chosen.
    pub fn key_env(&self) -> Option<&str> {
        self.api_key_env
            .as_deref()
            .or_else(|| self.provider.default_key_env())
    }
}

fn default_horizon_minutes() -> i64 {
    240
}
fn default_blackout_minutes() -> i64 {
    30
}
fn default_calendar_importance() -> jarvis_bridge_client::protocol::Importance {
    jarvis_bridge_client::protocol::Importance::Moderate
}
fn default_market_rate_limit() -> usize {
    30
}
fn default_news_cache_minutes() -> u64 {
    10
}

/// `[personas]`: a folder of `<id>.toml` packs (see `personas/jarvis.toml`) and the persona
/// a user gets until they pick another. Both are optional.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonasConfig {
    /// Relative paths are resolved from the working directory.
    #[serde(default = "default_personas_dir")]
    pub dir: PathBuf,
    /// The id of a pack in `dir`.
    #[serde(default = "default_persona")]
    pub default: String,
}

impl Default for PersonasConfig {
    fn default() -> Self {
        Self {
            dir: default_personas_dir(),
            default: default_persona(),
        }
    }
}

fn default_personas_dir() -> PathBuf {
    PathBuf::from("personas")
}
fn default_persona() -> String {
    "jarvis".to_owned()
}

/// `[auth]`: password + TOTP login with a JWT session cookie (ADR-0011).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuthConfig {
    /// Require a session on every `/api/*` route except login and health. The secret comes
    /// from the environment variable `JARVIS_JWT_SECRET` (at least 32 bytes).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Session lifetime in hours (1 to 168).
    #[serde(default = "default_session_hours")]
    pub session_hours: u64,
    /// Force the `Secure` cookie attribute on or off. Unset: `Secure` whenever the server
    /// itself serves TLS. Set it to `true` behind a TLS-terminating reverse proxy that
    /// forwards to a plain-HTTP loopback port.
    #[serde(default)]
    pub cookie_secure: Option<bool>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            session_hours: default_session_hours(),
            cookie_secure: None,
        }
    }
}

fn yes() -> bool {
    true
}
fn default_session_hours() -> u64 {
    12
}

/// `[voice]`. Every path is optional in the file so that `enabled = false` alone is valid;
/// [`VoiceSection::to_voice_config`] checks them when voice is on.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VoiceSection {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub whisper_model: Option<PathBuf>,
    #[serde(default = "default_voice_language")]
    pub language: String,
    #[serde(default = "default_voice_threads")]
    pub threads: u32,
    #[serde(default)]
    pub piper_exe: Option<PathBuf>,
    #[serde(default)]
    pub piper_voice: Option<PathBuf>,
    #[serde(default = "default_voice_max_seconds")]
    pub max_seconds: f32,
    /// Who speaks: Piper (default), or VoiceStudio with Piper as the fallback.
    #[serde(default)]
    pub tts_engine: TtsEngine,
    /// `[voice.voicestudio]`, used when `tts_engine = "voicestudio"`.
    #[serde(default)]
    pub voicestudio: Option<jarvis_voice::VoiceStudioConfig>,
}

/// `[voice] tts_engine`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsEngine {
    #[default]
    Piper,
    Voicestudio,
}

fn default_voice_language() -> String {
    "en".to_owned()
}
fn default_voice_threads() -> u32 {
    4
}
fn default_voice_max_seconds() -> f32 {
    30.0
}

impl VoiceSection {
    /// The voice crate's config, or `None` when voice is disabled.
    pub fn to_voice_config(&self) -> anyhow::Result<Option<jarvis_voice::VoiceConfig>> {
        if !self.enabled {
            return Ok(None);
        }
        let need = |v: &Option<PathBuf>, key: &str| {
            v.clone().ok_or_else(|| {
                anyhow!("[voice] enabled = true needs '{key}' (see config/jarvis.example.toml)")
            })
        };
        if !(self.max_seconds.is_finite() && self.max_seconds > 0.0 && self.max_seconds <= 120.0) {
            bail!("voice.max_seconds must be between 0 and 120");
        }
        if self.tts_engine == TtsEngine::Voicestudio {
            let url = self.voicestudio.as_ref().map_or_else(
                || jarvis_voice::VoiceStudioConfig::default().url,
                |v| v.url.clone(),
            );
            jarvis_voice::voicestudio::loopback_base(&url).map_err(|e| anyhow!("{e}"))?;
        }
        Ok(Some(jarvis_voice::VoiceConfig {
            enabled: true,
            whisper_model: need(&self.whisper_model, "whisper_model")?,
            language: self.language.clone(),
            threads: self.threads,
            piper_exe: need(&self.piper_exe, "piper_exe")?,
            piper_voice: need(&self.piper_voice, "piper_voice")?,
            max_seconds: self.max_seconds,
        }))
    }
}

/// `[trading]`: confirmation window and execution settings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TradingConfig {
    /// How long a confirmation token is valid.
    #[serde(default = "default_token_ttl_secs")]
    pub token_ttl_secs: u64,
    /// Voice flow: the window starts after the read-back, never later than this after issue.
    #[serde(default = "default_voice_cap_secs")]
    pub voice_token_cap_secs: u64,
    /// Max slippage for market orders and closes, in points.
    #[serde(default = "default_deviation_points")]
    pub deviation_points: u32,
    /// Start with trading disabled.
    #[serde(default)]
    pub kill_switch_engaged: bool,
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            token_ttl_secs: default_token_ttl_secs(),
            voice_token_cap_secs: default_voice_cap_secs(),
            deviation_points: default_deviation_points(),
            kill_switch_engaged: false,
        }
    }
}

fn default_token_ttl_secs() -> u64 {
    15
}
fn default_voice_cap_secs() -> u64 {
    30
}
fn default_deviation_points() -> u32 {
    20
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Built web UI served at `/` (with `index.html` fallback for client-side routes).
    #[serde(default)]
    pub static_dir: Option<PathBuf>,
    /// PEM certificate chain. Set together with `tls_key` to serve HTTPS/WSS.
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,
    /// PEM private key.
    #[serde(default)]
    pub tls_key: Option<PathBuf>,
    /// Extra browser origins allowed to call the API with the session cookie (CORS and the
    /// WebSocket `Origin` check), for example `["http://localhost:5173"]` for a Vite dev
    /// server that talks to the API directly. Empty by default: the SPA is served from the
    /// API's own origin, and the Vite proxy (`pnpm dev`) is same-origin too. Anything listed
    /// here can drive the whole API, so list only origins you control.
    #[serde(default)]
    pub dev_origins: Vec<String>,
    /// Seconds a client has to send the headers of a request (NET-1). It also ends a keep-alive
    /// connection that has been idle for this long, and bounds the TLS handshake. 1 to 120.
    #[serde(default = "default_header_read_timeout_secs")]
    pub header_read_timeout_secs: u64,
    /// Seconds a request may take to be answered, body upload included (NET-1), for every
    /// route except the ones below and the WebSocket (which has no limit). 1 to 600.
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// The same for the routes that wait for a language model, speech or a batch of trades:
    /// `/api/chat`, `/api/voice/*` and `/api/confirm/*`. At least `request_timeout_secs`, at
    /// most 3600. Keep it above the `timeout_secs` of your slowest `[[llm.providers]]` entry.
    #[serde(default = "default_long_request_timeout_secs")]
    pub long_request_timeout_secs: u64,
    /// Open connections the server accepts at once (NET-1); further ones are closed at once.
    /// 1 to 100000.
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    /// `POST /api/chat` requests per minute and session (NET-2); each can cost several model
    /// calls. 1 to 10000.
    #[serde(default = "default_chat_rate_limit")]
    pub chat_rate_limit_per_minute: usize,
    /// `POST /api/voice/transcribe` requests per minute and session (NET-2). 1 to 10000.
    #[serde(default = "default_transcribe_rate_limit")]
    pub transcribe_rate_limit_per_minute: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            static_dir: None,
            tls_cert: None,
            tls_key: None,
            dev_origins: Vec::new(),
            header_read_timeout_secs: default_header_read_timeout_secs(),
            request_timeout_secs: default_request_timeout_secs(),
            long_request_timeout_secs: default_long_request_timeout_secs(),
            max_connections: default_max_connections(),
            chat_rate_limit_per_minute: default_chat_rate_limit(),
            transcribe_rate_limit_per_minute: default_transcribe_rate_limit(),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:8787".to_owned()
}
fn default_header_read_timeout_secs() -> u64 {
    10
}
fn default_request_timeout_secs() -> u64 {
    30
}
fn default_long_request_timeout_secs() -> u64 {
    200
}
fn default_max_connections() -> usize {
    256
}
fn default_chat_rate_limit() -> usize {
    20
}
fn default_transcribe_rate_limit() -> usize {
    20
}

#[derive(Debug, Clone, Deserialize)]
pub struct BridgeSection {
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// File that holds the shared bridge key (64 hex characters): the `bridge.key` that
    /// `scripts/install-ea.ps1` creates in the MT5 data folder (`MQL5\Files\JarvisMT`). The
    /// environment variable `JARVIS_BRIDGE_KEY` (the same 64 characters) overrides it. The
    /// server does not start without one of them (BR-1, ADR-0016).
    #[serde(default)]
    pub key_file: Option<PathBuf>,
}

impl Default for BridgeSection {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            timeout_ms: default_timeout_ms(),
            key_file: None,
        }
    }
}

/// Where the bridge key comes from, for error messages.
const KEY_HELP: &str = r"The EA only answers a holder of the shared bridge key. Run `pwsh scripts\install-ea.ps1` (it creates MQL5\Files\JarvisMT\bridge.key in your MT5 data folder and prints the path), then set `key_file` under [bridge] in config/jarvis.toml to that path, or put the key's 64 hex characters in JARVIS_BRIDGE_KEY.";

fn default_endpoint() -> String {
    "tcp://127.0.0.1:5555".to_owned()
}
fn default_timeout_ms() -> u64 {
    3000
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_url")]
    pub url: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: default_db_url(),
        }
    }
}

fn default_db_url() -> String {
    "sqlite://data/jarvis.db".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolsConfig {
    /// Last-resort suffix (`.m`, `#`, ...), appended to resolved names only while the current
    /// broker has no symbol map in any layer (see `docs/api.md`, "Broker symbol mapping").
    #[serde(default)]
    pub suffix: String,
    /// Symbols shown in the UI watchlist (canonical names).
    #[serde(default)]
    pub watchlist: Vec<String>,
    /// Spoken/typed name -> canonical instrument. Keys are lowercased on load.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    /// The canonical instruments and their generic aliases.
    #[serde(default = "default_instruments")]
    pub instruments: PathBuf,
    /// Curated per-broker maps (checked into the repository).
    #[serde(default = "default_broker_map")]
    pub broker_map: PathBuf,
    /// Where the generated and user maps and the broker catalog snapshots are written.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    /// Detect a broker's symbols automatically (at start-up and on an account switch). Off:
    /// only the curated and user maps apply, and `POST /api/symbols/detect` still works.
    #[serde(default = "default_true")]
    pub auto_detect: bool,
}

impl Default for SymbolsConfig {
    fn default() -> Self {
        Self {
            suffix: String::new(),
            watchlist: Vec::new(),
            aliases: BTreeMap::new(),
            instruments: default_instruments(),
            broker_map: default_broker_map(),
            data_dir: default_data_dir(),
            auto_detect: true,
        }
    }
}

fn default_instruments() -> PathBuf {
    PathBuf::from("config/instruments.toml")
}

fn default_broker_map() -> PathBuf {
    PathBuf::from("config/broker_symbols.toml")
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub default: String,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Load a local (Ollama) default model in the background at start-up, so the first
    /// question does not wait for it. Default on.
    #[serde(default = "default_true")]
    pub warm_up: bool,
}

impl LlmConfig {
    /// The provider the start-up warm-up talks to: the default one, when it is Ollama and
    /// `warm_up` is on. Cloud providers are never warmed up (it would cost a request).
    pub fn warm_up_provider(&self) -> Option<&ProviderConfig> {
        if !self.warm_up {
            return None;
        }
        self.providers
            .iter()
            .find(|p| p.name == self.default)
            .filter(|p| p.kind == ProviderKind::Ollama)
    }
}

impl Config {
    /// Path of the config file: `JARVIS_CONFIG` or `config/jarvis.toml`.
    pub fn path() -> PathBuf {
        std::env::var_os(CONFIG_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
    }

    /// Load `.env` (if present), then the config file.
    pub fn load() -> anyhow::Result<Self> {
        Self::load_with(None)
    }

    /// Like [`Config::load`] with an explicit config path (the `--config` flag).
    pub fn load_with(path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        // A missing .env is normal; a broken one shows up later as missing API keys.
        let _ = dotenvy::dotenv();
        match path {
            Some(p) => Self::load_from(p),
            None => Self::load_from(&Self::path()),
        }
    }

    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
                "config file not found at {}. Copy config/jarvis.example.toml to \
                 config/jarvis.toml (or point {CONFIG_ENV} at your file) and edit it.",
                path.display()
            ),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Self::from_toml_str(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    /// Parse, normalise and validate.
    pub fn from_toml_str(text: &str) -> anyhow::Result<Self> {
        let mut config: Config = toml::from_str(text).context("parsing TOML")?;
        config.normalise();
        config.validate()?;
        Ok(config)
    }

    fn normalise(&mut self) {
        let aliases = std::mem::take(&mut self.symbols.aliases);
        self.symbols.aliases = aliases
            .into_iter()
            .map(|(k, v)| (alias_key(&k), v))
            .collect();
        self.symbols.suffix = self.symbols.suffix.trim().to_owned();
        self.symbols.watchlist.retain(|s| !s.trim().is_empty());
        // Symbols in the config are canonical names (upper case, no broker suffix). The
        // instrument catalog maps generic aliases (USTEC -> US100) once it is loaded
        // (`AppState::new_with`).
        // A name written with the old broker suffix (`EURUSD.m`) loses it.
        let suffix = self.symbols.suffix.to_ascii_uppercase();
        let strip = |s: &str| {
            let name = clean(s);
            match name.strip_suffix(&suffix) {
                Some(stem) if !suffix.is_empty() && !stem.is_empty() => stem.to_owned(),
                _ => name,
            }
        };
        if let Some(limits) = &mut self.limits {
            limits.allowed_symbols = limits
                .allowed_symbols
                .iter()
                .map(|s| strip(s))
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    /// Rewrite every symbol name in the config (`allowed_symbols`, `watchlist`) through
    /// `canonical`; names it rejects are dropped, duplicates removed.
    pub fn canonicalize_symbols(&mut self, canonical: impl Fn(&str) -> Option<String>) {
        fn dedupe(list: &[String], canonical: &impl Fn(&str) -> Option<String>) -> Vec<String> {
            let mut out: Vec<String> = Vec::with_capacity(list.len());
            for name in list {
                if let Some(c) = canonical(name)
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
            out
        }
        self.symbols.watchlist = dedupe(&self.symbols.watchlist, &canonical);
        if let Some(limits) = &mut self.limits {
            limits.allowed_symbols = dedupe(&limits.allowed_symbols, &canonical);
        }
    }

    /// The shared key for the bridge: `JARVIS_BRIDGE_KEY` (hex) if set and non-empty, otherwise
    /// the file named by `[bridge] key_file`. Fails, with a message that says what to do, when
    /// there is no usable key. Never includes the key in an error.
    pub fn bridge_key(&self) -> anyhow::Result<BridgeKey> {
        let from_env = std::env::var(KEY_ENV).ok();
        self.bridge_key_from(from_env.as_deref())
    }

    /// [`Config::bridge_key`] with the environment value passed in (tests).
    fn bridge_key_from(&self, from_env: Option<&str>) -> anyhow::Result<BridgeKey> {
        match (
            from_env.filter(|v| !v.trim().is_empty()),
            &self.bridge.key_file,
        ) {
            (Some(hex), _) => BridgeKey::from_hex(hex)
                .map_err(|e| anyhow!("{KEY_ENV} is not a usable bridge key: {e}. {KEY_HELP}")),
            (None, Some(path)) => BridgeKey::from_file(path)
                .map_err(|e| anyhow!("[bridge] key_file is not usable: {e}. {KEY_HELP}")),
            (None, None) => bail!("no bridge key is configured. {KEY_HELP}"),
        }
    }

    /// The limits the gate enforces. Without a `[limits]` section nothing may be traded:
    /// `allowed_symbols` is empty and the gate fails closed on that.
    pub fn effective_limits(&self) -> Limits {
        self.limits.clone().unwrap_or_else(|| Limits {
            max_lot: 1.0,
            max_positions: 5,
            max_daily_loss: 500.0,
            allowed_symbols: Vec::new(),
            trading_hours: None,
            max_risk_percent: jarvis_gate::DEFAULT_MAX_RISK_PERCENT,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        // `mode` is checked by serde (demo | live).
        validate_loopback_endpoint(&self.bridge.endpoint)?;
        if self.bridge.timeout_ms == 0 {
            bail!("bridge.timeout_ms must be greater than 0");
        }
        self.bind_addr()?;
        for origin in &self.server.dev_origins {
            validate_origin(origin)?;
        }
        self.validate_server_limits()?;
        if !(1..=168).contains(&self.auth.session_hours) {
            bail!("auth.session_hours must be between 1 and 168");
        }
        if self.llm.providers.is_empty() {
            bail!("at least one [[llm.providers]] entry is required");
        }
        let mut seen = std::collections::BTreeSet::new();
        for p in &self.llm.providers {
            if p.name.trim().is_empty() {
                bail!("an [[llm.providers]] entry has an empty name");
            }
            if !seen.insert(p.name.as_str()) {
                bail!("duplicate llm provider name '{}'", p.name);
            }
        }
        if !self
            .llm
            .providers
            .iter()
            .any(|p| p.name == self.llm.default)
        {
            bail!(
                "llm.default = '{}' does not match any [[llm.providers]] name (have: {})",
                self.llm.default,
                self.llm
                    .providers
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        self.validate_trading()?;
        self.market.validate()?;
        self.assistant.validate()?;
        self.desk.validate()?;
        self.learning.validate()?;
        self.alpaca.validate()?;
        Ok(())
    }

    fn validate_server_limits(&self) -> anyhow::Result<()> {
        let s = &self.server;
        if !(1..=120).contains(&s.header_read_timeout_secs) {
            bail!("server.header_read_timeout_secs must be between 1 and 120");
        }
        if !(1..=600).contains(&s.request_timeout_secs) {
            bail!("server.request_timeout_secs must be between 1 and 600");
        }
        if !(s.request_timeout_secs..=3600).contains(&s.long_request_timeout_secs) {
            bail!(
                "server.long_request_timeout_secs must be between server.request_timeout_secs \
                 and 3600"
            );
        }
        if !(1..=100_000).contains(&s.max_connections) {
            bail!("server.max_connections must be between 1 and 100000");
        }
        for (name, value) in [
            ("chat_rate_limit_per_minute", s.chat_rate_limit_per_minute),
            (
                "transcribe_rate_limit_per_minute",
                s.transcribe_rate_limit_per_minute,
            ),
        ] {
            if !(1..=10_000).contains(&value) {
                bail!("server.{name} must be between 1 and 10000");
            }
        }
        Ok(())
    }

    /// Providers whose own timeout is not shorter than the time a chat request may take: a turn
    /// that waits for them can be cut short by the server. For a start-up warning.
    pub fn providers_slower_than_chat_timeout(&self) -> Vec<&str> {
        self.llm
            .providers
            .iter()
            .filter(|p| p.timeout_secs >= self.server.long_request_timeout_secs)
            .map(|p| p.name.as_str())
            .collect()
    }

    fn validate_trading(&self) -> anyhow::Result<()> {
        let t = &self.trading;
        if t.token_ttl_secs == 0 || t.token_ttl_secs > 120 {
            bail!("trading.token_ttl_secs must be between 1 and 120");
        }
        if t.voice_token_cap_secs < t.token_ttl_secs {
            bail!("trading.voice_token_cap_secs must be at least trading.token_ttl_secs");
        }
        if t.deviation_points > 1000 {
            bail!("trading.deviation_points must be at most 1000");
        }
        if let Some(limits) = &self.limits {
            limits
                .validate()
                .map_err(|e| anyhow!("invalid [limits]: {e}"))?;
        }
        Ok(())
    }

    /// True when the server terminates TLS itself (`tls_cert` and `tls_key` are set).
    pub fn tls_enabled(&self) -> bool {
        self.server.tls_cert.is_some() && self.server.tls_key.is_some()
    }

    /// Whether the session cookie gets the `Secure` attribute: the explicit
    /// `auth.cookie_secure`, otherwise whenever the server serves TLS. Plain HTTP on
    /// loopback (development) is the only case without it.
    pub fn cookie_secure(&self) -> bool {
        self.auth
            .cookie_secure
            .unwrap_or_else(|| self.tls_enabled())
    }

    /// Parsed `server.bind`. Anything other than loopback is refused unless the server
    /// serves TLS **and** login is enabled (ADR-0011): without both, whoever can reach the
    /// port could confirm trades.
    pub fn bind_addr(&self) -> anyhow::Result<SocketAddr> {
        let addr: SocketAddr = self.server.bind.parse().map_err(|e| {
            anyhow!(
                "server.bind '{}' is not a valid host:port ({e})",
                self.server.bind
            )
        })?;
        if self.server.tls_cert.is_some() != self.server.tls_key.is_some() {
            bail!("server.tls_cert and server.tls_key must be set together");
        }
        let protected = self.tls_enabled() && self.auth.enabled;
        if !addr.ip().is_loopback() && !protected {
            bail!(
                "refusing to bind {addr}: a non-loopback address needs HTTPS and login. Set \
                 server.tls_cert and server.tls_key, and keep [auth] enabled = true, or use a \
                 loopback address such as 127.0.0.1:8787"
            );
        }
        Ok(addr)
    }
}

/// `server.dev_origins` entries are exactly `scheme://host[:port]` (what a browser sends as
/// `Origin`): no path, no wildcard, no credentials.
fn validate_origin(origin: &str) -> anyhow::Result<()> {
    let bad = || {
        anyhow!(
            "server.dev_origins entry '{origin}' must look like http://localhost:5173 (scheme, host and optional port only)"
        )
    };
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .ok_or_else(bad)?;
    let ok = !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if ok { Ok(()) } else { Err(bad()) }
}

/// `tcp://127.0.0.1:5555` style endpoint on a loopback host.
pub fn validate_loopback_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let rest = endpoint.strip_prefix("tcp://").ok_or_else(|| {
        anyhow!("bridge.endpoint '{endpoint}' must look like tcp://127.0.0.1:5555")
    })?;
    let (host, port) = rest
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("bridge.endpoint '{endpoint}' has no port"))?;
    port.parse::<u16>()
        .map_err(|_| anyhow!("bridge.endpoint '{endpoint}' has an invalid port '{port}'"))?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    if !loopback {
        bail!(
            "bridge.endpoint '{endpoint}' must be a loopback address (127.0.0.1, ::1 or localhost)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../config/jarvis.example.toml");

    #[test]
    fn personas_section_defaults_and_overrides() {
        let c = Config::from_toml_str(&minimal("")).unwrap();
        assert_eq!(c.personas.dir, PathBuf::from("personas"));
        assert_eq!(c.personas.default, "jarvis");
        let c = Config::from_toml_str(&minimal(
            "[personas]
dir = \"packs\"
default = \"friday\"",
        ))
        .unwrap();
        assert_eq!(c.personas.dir, PathBuf::from("packs"));
        assert_eq!(c.personas.default, "friday");
        let c = Config::from_toml_str(&minimal(
            "[personas]
default = \"friday\"",
        ))
        .unwrap();
        assert_eq!(c.personas.dir, PathBuf::from("personas"));
        assert!(
            Config::from_toml_str(&minimal(
                "[personas]
folder = \"x\""
            ))
            .is_err()
        );
    }

    #[test]
    fn market_section_defaults_match_the_spec() {
        let c = Config::from_toml_str(&minimal("")).unwrap();
        let m = &c.market;
        assert!(m.enabled);
        assert_eq!(m.horizon_minutes, 240);
        assert_eq!(m.event_blackout_minutes, 30);
        assert_eq!(
            m.calendar_min_importance,
            jarvis_bridge_client::protocol::Importance::Moderate
        );
        assert_eq!(m.news.provider, NewsProviderKind::None);
        assert_eq!(m.news.cache_minutes, 10);
        assert_eq!(m.news.key_env(), None);
        assert_eq!(m.rate_limit_per_minute, 30);
        let view = m.view_config();
        assert_eq!(view.horizon_minutes, 240);
        assert_eq!(view.event_blackout_minutes, 30);
        assert_eq!(view, jarvis_market::ViewConfig::default());
    }

    #[test]
    fn market_section_overrides_and_the_example_file() {
        let c = Config::from_toml_str(&minimal(
            r#"[market]
horizon_minutes = 120
event_blackout_minutes = 45
calendar_min_importance = "high"
[market.news]
provider = "alphavantage"
cache_minutes = 30
[market.view]
weight_h4_trend = 30.0
horizon_minutes = 999"#,
        ))
        .unwrap();
        assert_eq!(c.market.news.provider, NewsProviderKind::Alphavantage);
        assert_eq!(c.market.news.key_env(), Some("ALPHAVANTAGE_API_KEY"));
        assert_eq!(c.market.news.cache_minutes, 30);
        let view = c.market.view_config();
        assert_eq!(view.weight_h4_trend, 30.0);
        // [market] wins over [market.view] for the two keys they share.
        assert_eq!(view.horizon_minutes, 120);
        assert_eq!(view.event_blackout_minutes, 45);

        let finnhub =
            Config::from_toml_str(&minimal("[market.news]\nprovider = \"finnhub\"")).unwrap();
        assert_eq!(finnhub.market.news.key_env(), Some("FINNHUB_API_KEY"));
        let named = Config::from_toml_str(&minimal(
            "[market.news]\nprovider = \"finnhub\"\napi_key_env = \"MY_KEY\"",
        ))
        .unwrap();
        assert_eq!(named.market.news.key_env(), Some("MY_KEY"));

        // The shipped example documents the section and loads as it is.
        let example = Config::from_toml_str(EXAMPLE).unwrap();
        assert!(example.market.enabled);
        assert_eq!(example.market.news.provider, NewsProviderKind::None);
        assert!(EXAMPLE.contains("[market.news]"));
        assert!(EXAMPLE.contains("FINNHUB_API_KEY") && EXAMPLE.contains("ALPHAVANTAGE_API_KEY"));
    }

    #[test]
    fn market_section_is_validated() {
        for bad in [
            "[market]\nhorizon_minutes = 5",
            "[market]\nhorizon_minutes = 5000",
            "[market]\nevent_blackout_minutes = -1",
            "[market]\nevent_blackout_minutes = 500",
            "[market]\nrate_limit_per_minute = 0",
            "[market]\ncalendar_min_importance = \"critical\"",
            "[market]\nsurprise = 1",
            "[market.news]\nprovider = \"bloomberg\"",
            "[market.news]\ncache_minutes = 0",
            "[market.news]\ncache_minutes = 1000",
            "[market.news]\nprovider = \"finnhub\"\napi_key_env = \"\"",
            // A key pasted where the variable name belongs must not be accepted.
            "[market.news]\nprovider = \"finnhub\"\napi_key_env = \"d1x-secret=key\"",
            "[market.view]\nbias_threshold = nan",
        ] {
            assert!(Config::from_toml_str(&minimal(bad)).is_err(), "{bad}");
        }
    }

    fn minimal(extra: &str) -> String {
        format!(
            r#"
            {extra}
            [llm]
            default = "local"
            [[llm.providers]]
            name = "local"
            kind = "ollama"
            model = "m"
            "#
        )
    }

    #[test]
    fn only_a_local_default_with_warm_up_on_is_warmed_up() {
        let local = Config::from_toml_str(&minimal("")).unwrap();
        assert_eq!(
            local.llm.warm_up_provider().map(|p| p.name.as_str()),
            Some("local")
        );
        let off = Config::from_toml_str(&minimal("").replace(
            "default = \"local\"",
            "default = \"local\"
warm_up = false",
        ))
        .unwrap();
        assert!(off.llm.warm_up_provider().is_none());
        let cloud = Config::from_toml_str(
            r#"
            [llm]
            default = "claude"
            [[llm.providers]]
            name = "claude"
            kind = "anthropic"
            model = "m"
            "#,
        )
        .unwrap();
        assert!(cloud.llm.warm_up_provider().is_none());
    }

    #[test]
    fn voicestudio_is_off_by_default_and_must_be_on_this_machine() {
        let voice = |extra: &str| {
            minimal(&format!(
                "[voice]\nwhisper_model = 'w'\npiper_exe = 'p'\npiper_voice = 'v'\n{extra}"
            ))
        };
        let c = Config::from_toml_str(&voice("")).unwrap();
        let v = c.voice.as_ref().unwrap();
        assert_eq!(v.tts_engine, TtsEngine::Piper);
        assert!(v.voicestudio.is_none());

        let c = Config::from_toml_str(&voice(
            "tts_engine = 'voicestudio'\n[voice.voicestudio]\n[voice.voicestudio.profiles]\njarvis = 'Jarvis'",
        ))
        .unwrap();
        let v = c.voice.as_ref().unwrap();
        assert_eq!(v.tts_engine, TtsEngine::Voicestudio);
        let vs = v.voicestudio.as_ref().unwrap();
        assert_eq!(vs.url, "http://127.0.0.1:3900");
        assert_eq!(
            vs.profiles.get("jarvis").map(String::as_str),
            Some("Jarvis")
        );
        assert!(v.to_voice_config().is_ok());

        let remote = Config::from_toml_str(&voice(
            "tts_engine = 'voicestudio'\n[voice.voicestudio]\nurl = 'http://10.0.0.5:3900'",
        ))
        .unwrap();
        assert!(remote.voice.unwrap().to_voice_config().is_err());
    }

    #[test]
    fn notes_are_on_by_default_and_read_notes_folder() {
        let c = Config::from_toml_str(&minimal("")).unwrap();
        assert!(c.notes.enabled);
        assert_eq!(c.notes.dir, PathBuf::from("notes"));
        assert_eq!(c.notes.limits().max_file_bytes, 256 * 1024);
        let off = Config::from_toml_str(&minimal("[notes]\nenabled = false\ndir = 'x'")).unwrap();
        assert!(!off.notes.enabled);
        assert!(Config::from_toml_str(&minimal("[notes]\nfolder = 'x'")).is_err());
    }

    #[test]
    fn example_config_is_valid() {
        let c = Config::from_toml_str(EXAMPLE).unwrap();
        assert_eq!(c.mode, Mode::Demo);
        assert_eq!(c.llm.providers.len(), 3);
        assert_eq!(
            c.symbols.aliases.get("dow jones").map(String::as_str),
            Some("US30")
        );
        assert_eq!(c.symbols.watchlist.len(), 6);
    }

    #[test]
    fn mode_defaults_to_demo_and_rejects_unknown() {
        assert_eq!(
            Config::from_toml_str(&minimal("")).unwrap().mode,
            Mode::Demo
        );
        assert_eq!(
            Config::from_toml_str(&minimal("mode = \"live\""))
                .unwrap()
                .mode,
            Mode::Live
        );
        assert!(Config::from_toml_str(&minimal("mode = \"paper\"")).is_err());
    }

    #[test]
    fn aliases_are_lowercased() {
        let text = r#"
            [symbols.aliases]
            "Dow  Jones" = "US30"
            GOLD = "XAUUSD"
            [llm]
            default = "local"
            [[llm.providers]]
            name = "local"
            kind = "ollama"
            model = "m"
        "#;
        let c = Config::from_toml_str(text).unwrap();
        assert!(c.symbols.aliases.contains_key("dow jones"));
        assert!(c.symbols.aliases.contains_key("gold"));
    }

    #[test]
    fn bridge_endpoint_must_be_loopback() {
        let with = |endpoint: &str| {
            Config::from_toml_str(&format!(
                "[bridge]\nendpoint = \"{endpoint}\"\n[llm]\ndefault = \"l\"\n\
                 [[llm.providers]]\nname = \"l\"\nkind = \"ollama\"\nmodel = \"m\""
            ))
        };
        let err = with("tcp://192.168.1.5:5555").unwrap_err().to_string();
        assert!(err.contains("loopback"), "{err}");
        assert!(with("tcp://localhost:5555").is_ok());
        assert!(with("tcp://[::1]:5555").is_ok());
        assert!(with("tcp://127.0.0.1:5555").is_ok());
        assert!(with("http://127.0.0.1:5555").is_err());
    }

    #[test]
    fn needs_a_provider_and_a_valid_default() {
        let none = "[llm]\ndefault = \"x\"";
        assert!(
            Config::from_toml_str(none)
                .unwrap_err()
                .to_string()
                .contains("at least one")
        );
        let wrong = minimal("").replace("default = \"local\"", "default = \"nope\"");
        assert!(
            Config::from_toml_str(&wrong)
                .unwrap_err()
                .to_string()
                .contains("llm.default")
        );
    }

    fn bind_config(server: &str, auth: &str) -> anyhow::Result<Config> {
        Config::from_toml_str(&format!(
            "[server]\n{server}\n[auth]\n{auth}\n[llm]\ndefault = \"l\"\n\
             [[llm.providers]]\nname = \"l\"\nkind = \"ollama\"\nmodel = \"m\""
        ))
    }

    #[test]
    fn non_loopback_bind_needs_tls_and_auth() {
        let tls = "tls_cert = \"c.pem\"\ntls_key = \"k.pem\"";
        // Loopback is always fine, with or without auth or TLS.
        assert!(bind_config("bind = \"127.0.0.1:0\"", "enabled = false").is_ok());
        assert!(bind_config("bind = \"[::1]:8787\"", "enabled = true").is_ok());
        // Off loopback: neither, TLS only, auth only are all refused.
        for (server, auth) in [
            ("bind = \"0.0.0.0:8787\"".to_owned(), "enabled = true"),
            (format!("bind = \"0.0.0.0:8787\"\n{tls}"), "enabled = false"),
            ("bind = \"192.168.1.5:8787\"".to_owned(), "enabled = true"),
        ] {
            let err = bind_config(&server, auth).unwrap_err();
            assert!(err.to_string().contains("HTTPS and login"), "{err}");
        }
        // TLS and auth: allowed.
        assert!(bind_config(&format!("bind = \"0.0.0.0:8787\"\n{tls}"), "enabled = true").is_ok());
    }

    #[test]
    fn tls_files_must_come_as_a_pair() {
        let err = bind_config("tls_cert = \"c.pem\"", "enabled = true").unwrap_err();
        assert!(err.to_string().contains("together"), "{err}");
    }

    #[test]
    fn auth_defaults_and_cookie_secure_rules() {
        let c = Config::from_toml_str(&minimal("")).unwrap();
        assert!(c.auth.enabled);
        assert_eq!(c.auth.session_hours, 12);
        assert!(!c.cookie_secure(), "plain HTTP on loopback: no Secure");
        let tls = "[server]\ntls_cert = \"c\"\ntls_key = \"k\"";
        assert!(
            Config::from_toml_str(&minimal(tls))
                .unwrap()
                .cookie_secure()
        );
        let proxy = "[auth]\ncookie_secure = true";
        assert!(
            Config::from_toml_str(&minimal(proxy))
                .unwrap()
                .cookie_secure()
        );
        assert!(Config::from_toml_str(&minimal("[auth]\nsession_hours = 0")).is_err());
    }

    #[test]
    fn voice_section_needs_paths_only_when_enabled() {
        let off = Config::from_toml_str(&minimal("[voice]\nenabled = false")).unwrap();
        assert!(off.voice.unwrap().to_voice_config().unwrap().is_none());
        // Enabled without paths is not a config error (the server starts with voice off and
        // says why); asking for the engine config is.
        let c = Config::from_toml_str(&minimal("[voice]\nenabled = true")).unwrap();
        let err = c.voice.unwrap().to_voice_config().unwrap_err();
        assert!(format!("{err:#}").contains("whisper_model"), "{err:#}");
        let c = Config::from_toml_str(&minimal(
            "[voice]\nwhisper_model = \"w.bin\"\npiper_exe = \"p.exe\"\npiper_voice = \"v.onnx\"",
        ))
        .unwrap();
        let v = c.voice.unwrap().to_voice_config().unwrap().unwrap();
        assert_eq!(v.language, "en");
        assert!((v.max_seconds - 30.0).abs() < f32::EPSILON);
    }

    // ---- the shared bridge key (BR-1) ------------------------------------------------------

    const KEY_A: &str = "0f0e0d0c0b0a09080706050403020100ffeeddccbbaa99887766554433221100";
    const KEY_B: &str = "a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0";

    fn config_with_key_file(contents: &str) -> (std::path::PathBuf, Config) {
        let dir = std::env::temp_dir().join(format!("jarvis-keycfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bridge.key");
        std::fs::write(&path, contents).unwrap();
        let toml = minimal(&format!("[bridge]\nkey_file = '{}'", path.display()));
        (path, Config::from_toml_str(&toml).unwrap())
    }

    fn remove_key_dir(path: &std::path::Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn the_bridge_key_comes_from_the_file() {
        let (path, c) = config_with_key_file(&format!("{KEY_A}\n"));
        assert_eq!(c.bridge.key_file.as_deref(), Some(path.as_path()));
        assert_eq!(c.bridge_key_from(None).unwrap().expose_hex(), KEY_A);
        // An empty or blank environment variable counts as unset.
        assert_eq!(c.bridge_key_from(Some("  ")).unwrap().expose_hex(), KEY_A);
        remove_key_dir(&path);
    }

    #[test]
    fn the_environment_key_wins_over_the_file() {
        let (_, c) = config_with_key_file(&format!("{KEY_A}\n"));
        assert_eq!(c.bridge_key_from(Some(KEY_B)).unwrap().expose_hex(), KEY_B);
        // A broken environment key is an error, not a silent fall back to the file.
        let err = c.bridge_key_from(Some("not-a-key")).unwrap_err();
        assert!(format!("{err:#}").contains("JARVIS_BRIDGE_KEY"), "{err:#}");
    }

    #[test]
    fn without_a_key_the_server_says_how_to_get_one() {
        let c = Config::from_toml_str(&minimal("")).unwrap();
        assert!(c.bridge.key_file.is_none());
        let err = format!("{:#}", c.bridge_key_from(None).unwrap_err());
        assert!(err.contains("no bridge key"), "{err}");
        assert!(err.contains("install-ea.ps1"), "{err}");
        assert!(
            err.contains("key_file") && err.contains("JARVIS_BRIDGE_KEY"),
            "{err}"
        );
    }

    #[test]
    fn a_missing_or_malformed_key_file_is_refused_without_echoing_it() {
        let c =
            Config::from_toml_str(&minimal("[bridge]\nkey_file = 'no/such/bridge.key'")).unwrap();
        let err = format!("{:#}", c.bridge_key_from(None).unwrap_err());
        assert!(err.contains("cannot read the bridge key file"), "{err}");
        assert!(err.contains("install-ea.ps1"), "{err}");

        let wrong_chars = "zz".repeat(32);
        let zeros = "00".repeat(32);
        for bad in ["", "tooshort", wrong_chars.as_str(), zeros.as_str()] {
            let (path, c) = config_with_key_file(bad);
            let err = format!("{:#}", c.bridge_key_from(None).unwrap_err());
            remove_key_dir(&path);
            assert!(err.contains("install-ea.ps1"), "{err}");
            if bad.len() > 8 {
                assert!(
                    !err.contains(bad),
                    "the file content must not appear in the error"
                );
            }
        }
    }

    #[test]
    fn missing_file_explains_how_to_fix() {
        let err = Config::load_from(std::path::Path::new("definitely/not/here.toml")).unwrap_err();
        assert!(err.to_string().contains("jarvis.example.toml"), "{err}");
    }

    // ---- [server] limits (NET-1, NET-2) -------------------------------------------------------

    #[test]
    fn server_limits_have_documented_defaults() {
        let c = Config::from_toml_str(&minimal("")).unwrap();
        let s = &c.server;
        assert_eq!(s.header_read_timeout_secs, 10);
        assert_eq!(s.request_timeout_secs, 30);
        assert_eq!(s.long_request_timeout_secs, 200);
        assert_eq!(s.max_connections, 256);
        assert_eq!(s.chat_rate_limit_per_minute, 20);
        assert_eq!(s.transcribe_rate_limit_per_minute, 20);
        // The example file spells the same numbers out.
        let e = Config::from_toml_str(EXAMPLE).unwrap();
        assert_eq!(e.server.header_read_timeout_secs, 10);
        assert_eq!(e.server.request_timeout_secs, 30);
        // Raised for Claude 3D builds (spec-model-uploads, live check).
        assert_eq!(e.server.long_request_timeout_secs, 300);
        assert_eq!(e.server.max_connections, 256);
        assert_eq!(e.server.chat_rate_limit_per_minute, 20);
        assert_eq!(e.server.transcribe_rate_limit_per_minute, 20);
    }

    #[test]
    fn server_limits_are_range_checked() {
        let with = |line: &str| Config::from_toml_str(&minimal(&format!("[server]\n{line}")));
        for ok in [
            "header_read_timeout_secs = 1",
            "request_timeout_secs = 600\nlong_request_timeout_secs = 600",
            "long_request_timeout_secs = 3600",
            "max_connections = 1",
            "chat_rate_limit_per_minute = 10000",
            "transcribe_rate_limit_per_minute = 1",
        ] {
            assert!(with(ok).is_ok(), "{ok}");
        }
        for bad in [
            "header_read_timeout_secs = 0",
            "header_read_timeout_secs = 121",
            "request_timeout_secs = 0",
            "request_timeout_secs = 601",
            "long_request_timeout_secs = 29",
            "long_request_timeout_secs = 3601",
            "request_timeout_secs = 100\nlong_request_timeout_secs = 50",
            "max_connections = 0",
            "chat_rate_limit_per_minute = 0",
            "chat_rate_limit_per_minute = 10001",
            "transcribe_rate_limit_per_minute = 0",
        ] {
            assert!(with(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn slow_providers_are_reported_against_the_chat_timeout() {
        let c = Config::from_toml_str(&format!("{}\ntimeout_secs = 300", minimal(""))).unwrap();
        assert_eq!(c.providers_slower_than_chat_timeout(), ["local"]);
        let c = Config::from_toml_str(&minimal("")).unwrap();
        assert!(c.providers_slower_than_chat_timeout().is_empty());
    }
}

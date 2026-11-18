//! `jarvis-api preflight` (FL-4): run every live chain once and print a verdict per check, a
//! latency budget and a summary. Real calls, no mocks: the failures that hurt are the ones
//! where every unit test passes and a real chain is dead.
//!
//! The model, voice and loop checks run in this process (no login needed) with the same
//! prompt, tools and engines the server uses. The served-files checks go over HTTP to the
//! running server. The web-bundle checks read the built files on disk.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jarvis_llm::{
    ChatRequest, LlmProvider, Message, ProviderKind, build_provider, effective_base_url,
};
use jarvis_persona::PersonaRegistry;
use serde::Serialize;
use serde_json::{Value, json};

use crate::agent::assemble_prompt;
use crate::broker::book_from_config;
use crate::config::{Config, NewsProviderKind};
use crate::egress::{EgressLog, EgressProvider, is_loopback, non_loopback_hosts, url_target};
use crate::symbols::SymbolResolver;
use crate::tools::tool_specs;
use crate::voice::Voice;

/// The sentence the voice loop speaks and expects back.
pub const LOOP_PHRASE: &str = "Please show my open positions on gold.";
/// How much of [`LOOP_PHRASE`] the ears must hear back.
pub const LOOP_MIN_OVERLAP: f64 = 0.8;
/// The question the model check asks (no tool is needed to answer it).
const CHAT_QUESTION: &str = "In one short sentence, what can you help me with?";
/// Per-request limit for the HTTP checks.
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// Values shorter than this in `.env` are not treated as secrets (flags, ports).
const MIN_SECRET_LEN: usize = 8;

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Warn => "WARN",
            Verdict::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub id: u8,
    pub name: &'static str,
    pub verdict: Verdict,
    pub detail: String,
}

/// Seconds measured in this run; `None` when that stage did not run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Latency {
    pub hear: Option<f64>,
    pub think: Option<f64>,
    pub speak: Option<f64>,
}

impl Latency {
    pub fn total(&self) -> Option<f64> {
        let parts = [self.hear, self.think, self.speak];
        parts
            .iter()
            .any(Option::is_some)
            .then(|| parts.iter().flatten().sum())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
    pub latency: Latency,
}

impl Report {
    pub fn push(
        &mut self,
        id: u8,
        name: &'static str,
        verdict: Verdict,
        detail: impl Into<String>,
    ) {
        self.checks.push(Check {
            id,
            name,
            verdict,
            detail: detail.into(),
        });
    }

    pub fn count(&self, verdict: Verdict) -> usize {
        self.checks.iter().filter(|c| c.verdict == verdict).count()
    }

    /// `LATENCY  hear 0.8 s  think 2.4 s  speak 0.6 s  total 3.8 s` (`n/a` for a stage that
    /// did not run).
    pub fn latency_line(&self) -> String {
        let f = |v: Option<f64>| v.map_or_else(|| "n/a".to_owned(), |s| format!("{s:.1} s"));
        let l = &self.latency;
        format!(
            "LATENCY  hear {}  think {}  speak {}  total {}",
            f(l.hear),
            f(l.think),
            f(l.speak),
            f(l.total())
        )
    }

    pub fn summary_line(&self) -> String {
        format!(
            "SUMMARY  {} pass, {} fail, {} warn",
            self.count(Verdict::Pass),
            self.count(Verdict::Fail),
            self.count(Verdict::Warn)
        )
    }

    /// 1 when any check failed, else 0.
    pub fn exit_code(&self) -> i32 {
        i32::from(self.count(Verdict::Fail) > 0)
    }

    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for c in &self.checks {
            let _ = writeln!(
                out,
                "{}  {:>2} {:<16} {}",
                c.verdict.label(),
                c.id,
                c.name,
                c.detail
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", self.latency_line());
        let _ = writeln!(out, "{}", self.summary_line());
        out
    }

    pub fn to_json(&self) -> Value {
        json!({
            "checks": self.checks,
            "latency": {
                "hear": self.latency.hear,
                "think": self.latency.think,
                "speak": self.latency.speak,
                "total": self.latency.total(),
            },
            "summary": {
                "pass": self.count(Verdict::Pass),
                "fail": self.count(Verdict::Fail),
                "warn": self.count(Verdict::Warn),
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Pure checks
// ---------------------------------------------------------------------------

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The share of `expected`'s words that appear in `heard`, from 0 to 1 (1 when `expected` has
/// no words).
pub fn word_overlap(expected: &str, heard: &str) -> f64 {
    let want = words(expected);
    if want.is_empty() {
        return 1.0;
    }
    let got = words(heard);
    let hits = want.iter().filter(|w| got.contains(w)).count();
    hits as f64 / want.len() as f64
}

/// Whether Ollama's `/api/tags` answer lists `model` (`qwen3.5:9b`; a name without a tag
/// means `:latest`).
pub fn ollama_has_model(tags: &Value, model: &str) -> bool {
    let norm = |m: &str| {
        let m = m.trim().to_lowercase();
        if m.contains(':') {
            m
        } else {
            format!("{m}:latest")
        }
    };
    let want = norm(model);
    tags.get("models")
        .and_then(Value::as_array)
        .is_some_and(|list| {
            list.iter().any(|m| {
                ["name", "model"]
                    .iter()
                    .filter_map(|k| m.get(*k).and_then(Value::as_str))
                    .any(|n| norm(n) == want)
            })
        })
}

/// What kind of file a bundle scan reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Html,
    Css,
    Script,
}

impl FileKind {
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "html" | "htm" => Some(Self::Html),
            "css" => Some(Self::Css),
            "js" | "mjs" => Some(Self::Script),
            _ => None,
        }
    }

    /// Text right before a URL that means "the page loads this".
    fn load_markers(self) -> &'static [&'static str] {
        match self {
            Self::Html => &["src=", "<link", "srcset=", "url(", "import(", "from"],
            Self::Css => &["url(", "@import"],
            Self::Script => &[
                "import(",
                "from",
                "import",
                "importScripts(",
                "fetch(",
                "new Worker(",
                "src=",
            ],
        }
    }
}

/// Hosts a bundle may name in a load without it reaching the network: XML namespaces are
/// identifiers, not requests.
const NAMESPACE_HOSTS: &[&str] = &["www.w3.org", "w3.org"];

/// URLs in `text` that the page would load from another machine: an `http(s)://` URL right
/// after a load marker (`src=`, `import(`, `url(`, ...) whose host is neither loopback nor an
/// XML namespace. A URL that is only text (an error message, a link) is not a load.
pub fn external_loads(text: &str, kind: FileKind) -> Vec<String> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(rel) = text[at..].find("http") {
        let start = at + rel;
        at = start + 4;
        let rest = &text[start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            continue;
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || "\"'`()<>,;\\".contains(c))
            .unwrap_or(rest.len());
        let url = &rest[..end];
        let Some((_, host, _)) = url_target(url) else {
            continue;
        };
        if is_loopback(&host) || NAMESPACE_HOSTS.contains(&host.as_str()) {
            continue;
        }
        let before_start = text[..start]
            .char_indices()
            .rev()
            .nth(39)
            .map_or(0, |(i, _)| i);
        let before = text[before_start..start].to_ascii_lowercase();
        let before = before.trim_end_matches(['"', '\'', '`', ' ', '(']);
        let is_load = kind.load_markers().iter().any(|m| {
            let m = m.trim_end_matches('(');
            before.ends_with(m)
                || before.rsplit('<').next().is_some_and(|tag| {
                    kind == FileKind::Html && m == "<link" && tag.starts_with("link")
                })
        });
        if is_load && !found.iter().any(|f| f == url) {
            found.push(url.to_owned());
        }
    }
    found
}

/// `KEY=value` pairs from a `.env` file whose value is long enough to be a secret.
pub fn env_secrets(env_text: &str) -> Vec<(String, String)> {
    env_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("export ").unwrap_or(l).split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            (k.trim().to_owned(), v.to_owned())
        })
        .filter(|(k, v)| !k.is_empty() && v.chars().count() >= MIN_SECRET_LEN)
        .collect()
}

/// The names (never the values) of the secrets that appear in `text`.
pub fn leaked_secrets(text: &str, secrets: &[(String, String)]) -> Vec<String> {
    secrets
        .iter()
        .filter(|(_, v)| text.contains(v.as_str()))
        .map(|(k, _)| k.clone())
        .collect()
}

/// Files the server must never hand to a browser, with text that proves it did.
const PRIVATE_PATHS: &[(&str, &str)] = &[
    ("/config/jarvis.toml", "[llm]"),
    ("/jarvis.toml", "[llm]"),
    ("/.env", "JARVIS_"),
    ("/jarvis.db", "SQLite format 3"),
    ("/data/jarvis.db", "SQLite format 3"),
    ("/bridge.key", "-----"),
];

/// Every regular file under `dir` (no symlinks followed), at most `limit`.
fn walk(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                out.push(e.path());
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

/// Checks 8 and 9 over the built web files in `dir`.
pub fn check_bundle(dir: &Path, secrets: &[(String, String)], report: &mut Report) {
    if !dir.is_dir() {
        let why = format!(
            "no built web files at {} (run `pnpm -C web build`)",
            dir.display()
        );
        report.push(8, "web: no CDN", Verdict::Warn, why.clone());
        report.push(9, "web: no secrets", Verdict::Warn, why);
        return;
    }
    let files = walk(dir, 5_000);
    let mut loads = Vec::new();
    let mut leaks = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue; // binary (images, fonts)
        };
        let rel = path.strip_prefix(dir).unwrap_or(path).display().to_string();
        if let Some(kind) = FileKind::of(path) {
            for url in external_loads(&text, kind) {
                loads.push(format!("{rel}: {url}"));
            }
        }
        for name in leaked_secrets(&text, secrets) {
            leaks.push(format!("{name} in {rel}"));
        }
    }
    if loads.is_empty() {
        report.push(
            8,
            "web: no CDN",
            Verdict::Pass,
            format!("{} files, nothing loaded from another machine", files.len()),
        );
    } else {
        report.push(8, "web: no CDN", Verdict::Fail, loads.join("; "));
    }
    if leaks.is_empty() {
        report.push(
            9,
            "web: no secrets",
            Verdict::Pass,
            format!("none of the {} .env values appear", secrets.len()),
        );
    } else {
        leaks.sort();
        leaks.dedup();
        report.push(9, "web: no secrets", Verdict::Fail, leaks.join("; "));
    }
}

/// An Ollama model that Ollama runs on its own servers (`gpt-oss:120b-cloud`,
/// `minimax-m3:cloud`): it is addressed at 127.0.0.1, but the prompt leaves the machine.
pub fn is_ollama_cloud_model(model: &str) -> bool {
    let tag = model
        .rsplit_once(':')
        .map_or("", |(_, t)| t)
        .to_ascii_lowercase();
    tag == "cloud" || tag.ends_with("-cloud")
}

/// "Local mode": the default model runs on this machine and news is off.
pub fn is_local_mode(config: &Config) -> bool {
    let news_off = !config.market.enabled || config.market.news.provider == NewsProviderKind::None;
    let model_local = config
        .llm
        .providers
        .iter()
        .find(|p| p.name == config.llm.default)
        .filter(|p| !(p.kind == ProviderKind::Ollama && is_ollama_cloud_model(&p.model)))
        .and_then(|p| url_target(&effective_base_url(p)))
        .is_some_and(|(_, host, _)| is_loopback(&host));
    news_off && model_local
}

/// Check 10 from the hosts contacted during this run.
pub fn check_egress(local_mode: bool, hosts: &[String], report: &mut Report) {
    match (hosts.is_empty(), local_mode) {
        (true, _) => report.push(
            10,
            "egress",
            Verdict::Pass,
            "no host other than this machine was contacted",
        ),
        (false, true) => report.push(
            10,
            "egress",
            Verdict::Fail,
            format!("local mode, but contacted: {}", hosts.join(", ")),
        ),
        (false, false) => report.push(
            10,
            "egress",
            Verdict::Warn,
            format!(
                "cloud mode (the default model or news is remote); contacted: {}",
                hosts.join(", ")
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// Options of `jarvis-api preflight`.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub config: Option<PathBuf>,
    /// The running server's base URL (default: from `[server] bind`).
    pub api: Option<String>,
    /// Skip the model check (and so the "think" figure).
    pub skip_chat: bool,
}

/// Run every check. Never panics on a broken environment: each failure is a FAIL or WARN line.
pub async fn run(opts: Options) -> Report {
    let mut report = Report::default();
    let config = match Config::load_with(opts.config.as_deref()) {
        Ok(c) => {
            let path = opts.config.clone().unwrap_or_else(Config::path);
            report.push(
                1,
                "config",
                Verdict::Pass,
                format!("loaded {}", path.display()),
            );
            c
        }
        Err(e) => {
            report.push(1, "config", Verdict::Fail, format!("{e:#}"));
            return report;
        }
    };
    let log = EgressLog::new();
    let api = opts.api.clone().unwrap_or_else(|| api_base(&config));
    let http = http_client(&api);

    check_api(&http, &api, &mut report).await;
    check_ollama(&http, &config, &log, &mut report).await;

    let personas = PersonaRegistry::load_dir(&config.personas.dir, &config.personas.default)
        .unwrap_or_else(|_| crate::state::builtin_personas());
    let persona = personas.default_persona().clone();

    if opts.skip_chat {
        report.push(4, "model", Verdict::Warn, "skipped (--skip-chat)");
    } else {
        check_chat(&config, &persona, &log, &mut report).await;
    }

    check_voice(&config, &personas, &persona.id, &mut report).await;
    check_private_files(&http, &api, &mut report).await;

    let static_dir = config
        .server
        .static_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("web/build"));
    let secrets = std::fs::read_to_string(".env")
        .map(|t| env_secrets(&t))
        .unwrap_or_default();
    check_bundle(&static_dir, &secrets, &mut report);
    check_assistant(
        &config,
        &static_dir,
        |name| std::env::var(name).ok(),
        chrono::Utc::now(),
        &mut report,
    );
    if secrets.is_empty() {
        // Nothing to look for: a pass would prove nothing.
        if let Some(c) = report
            .checks
            .iter_mut()
            .find(|c| c.id == 9 && c.verdict == Verdict::Pass)
        {
            c.verdict = Verdict::Warn;
            c.detail = "no .env with secrets in this folder; run the preflight from the \
                        repository root"
                .to_owned();
        }
    }

    let mut hosts = non_loopback_hosts(&log.since(0));
    // The running server's own log too, when it answers without a login.
    if let Some(server_hosts) = server_egress(&http, &api).await {
        hosts.extend(server_hosts);
        hosts.sort();
        hosts.dedup();
    }
    check_egress(is_local_mode(&config), &hosts, &mut report);
    report.checks.sort_by_key(|c| c.id);
    report
}

/// The client for the HTTP checks. A self-signed certificate is accepted only when the server
/// is this machine; a server elsewhere (`--api`) must present a valid one.
fn http_client(api: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(accepts_self_signed(api))
        .no_proxy()
        .build()
        .unwrap_or_default()
}

/// True for a loopback `api` URL.
pub fn accepts_self_signed(api: &str) -> bool {
    url_target(api).is_some_and(|(_, host, _)| is_loopback(&host))
}

/// The server's address as a URL, with a wildcard bind turned into loopback.
pub fn api_base(config: &Config) -> String {
    let scheme = if config.tls_enabled() {
        "https"
    } else {
        "http"
    };
    match config.bind_addr() {
        Ok(addr) => {
            let host = if addr.ip().is_unspecified() || addr.ip().is_loopback() {
                "127.0.0.1".to_owned()
            } else {
                match addr.ip() {
                    std::net::IpAddr::V6(v6) => format!("[{v6}]"),
                    ip => ip.to_string(),
                }
            };
            format!("{scheme}://{host}:{}", addr.port())
        }
        Err(_) => format!("{scheme}://127.0.0.1:8787"),
    }
}

async fn check_api(http: &reqwest::Client, api: &str, report: &mut Report) {
    let url = format!("{api}/api/health");
    match http.get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            report.push(2, "api", Verdict::Pass, format!("{api} is up"));
        }
        Ok(r) => report.push(
            2,
            "api",
            Verdict::Fail,
            format!("{url} answered {}", r.status()),
        ),
        Err(_) => report.push(
            2,
            "api",
            Verdict::Fail,
            format!("{api} does not answer; start it with scripts\\run.ps1"),
        ),
    }
}

async fn check_ollama(
    http: &reqwest::Client,
    config: &Config,
    log: &Arc<EgressLog>,
    report: &mut Report,
) {
    let default = config
        .llm
        .providers
        .iter()
        .find(|p| p.name == config.llm.default);
    // The default provider when it is Ollama, else the first Ollama one.
    let Some(p) = default
        .filter(|d| d.kind == ProviderKind::Ollama)
        .or_else(|| {
            config
                .llm
                .providers
                .iter()
                .find(|p| p.kind == ProviderKind::Ollama)
        })
    else {
        report.push(3, "ollama", Verdict::Warn, "no Ollama provider configured");
        return;
    };
    let on_fail = if default.is_some_and(|d| d.kind == ProviderKind::Ollama) {
        Verdict::Fail
    } else {
        Verdict::Warn
    };
    let base = effective_base_url(p);
    let url = format!("{base}/api/tags");
    let started = Instant::now();
    let answer = http.get(&url).send().await;
    log.record(
        &url,
        "preflight:ollama",
        answer.is_ok(),
        started.elapsed().as_millis() as u64,
    );
    let tags = match answer {
        Ok(r) if r.status().is_success() => r.json::<Value>().await.unwrap_or(Value::Null),
        _ => {
            report.push(
                3,
                "ollama",
                on_fail,
                format!("Ollama does not answer at {base}; start it with `ollama serve`"),
            );
            return;
        }
    };
    if is_ollama_cloud_model(&p.model) {
        report.push(
            3,
            "ollama",
            Verdict::Warn,
            format!(
                "{} is an Ollama cloud model: Ollama sends the prompt to its own servers, which the \
                 outbound log cannot see. Pick a local model for local mode.",
                p.model
            ),
        );
    } else if ollama_has_model(&tags, &p.model) {
        report.push(
            3,
            "ollama",
            Verdict::Pass,
            format!("{} is pulled ({base})", p.model),
        );
    } else {
        report.push(
            3,
            "ollama",
            on_fail,
            format!("{} is not pulled; run: ollama pull {}", p.model, p.model),
        );
    }
}

async fn check_chat(
    config: &Config,
    persona: &jarvis_persona::PersonaPack,
    log: &Arc<EgressLog>,
    report: &mut Report,
) {
    let Some(p) = config
        .llm
        .providers
        .iter()
        .find(|p| p.name == config.llm.default)
    else {
        report.push(4, "model", Verdict::Fail, "llm.default names no provider");
        return;
    };
    let provider: Arc<dyn LlmProvider> = match build_provider(p) {
        Ok(b) => EgressProvider::wrap(Arc::from(b), &effective_base_url(p), &p.name, log.clone()),
        Err(e) => {
            report.push(4, "model", Verdict::Fail, format!("{}: {e}", p.name));
            return;
        }
    };
    // The prompt and tools the server sends, so the timing is the real one.
    let mut config = config.clone();
    let book = book_from_config(&config);
    config.canonicalize_symbols(|n| book.canonical_loose(n));
    let resolver = SymbolResolver::with_book(&config.symbols.aliases, book);
    let now = chrono::Utc::now();
    let request = |now: chrono::DateTime<chrono::Utc>| ChatRequest {
        system: Some(assemble_prompt(
            persona,
            &resolver.alias_list(),
            config.notes.enabled,
            &now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            &now.format("%Y-%m-%d %H:%M:%S").to_string(),
        )),
        messages: vec![Message::User {
            text: CHAT_QUESTION.to_owned(),
        }],
        tools: tool_specs(config.market.enabled, config.notes.enabled),
        max_tokens: None,
        temperature: Some(0.2),
        effort: None,
        cache_system: false,
    };
    // Twice: the first may load the model; the second shows the cached-prompt speed.
    let mut times = Vec::new();
    let mut last = None;
    for i in 0..2 {
        let started = Instant::now();
        match provider
            .chat(&request(now + chrono::Duration::seconds(i)))
            .await
        {
            Ok(r) => {
                times.push(started.elapsed().as_secs_f64());
                last = Some(r);
            }
            Err(e) => {
                report.push(4, "model", Verdict::Fail, format!("{}: {e}", p.name));
                return;
            }
        }
    }
    let Some(r) = last else { return };
    let answered =
        r.text.as_deref().is_some_and(|t| !t.trim().is_empty()) || !r.tool_calls.is_empty();
    let tokens = r
        .usage
        .map(|u| {
            format!(
                ", {} prompt and {} answer tokens",
                u.input_tokens, u.output_tokens
            )
        })
        .unwrap_or_default();
    report.latency.think = times.last().copied();
    if answered {
        report.push(
            4,
            "model",
            Verdict::Pass,
            format!(
                "{} {} answered in {:.1} s (first turn {:.1} s){tokens}",
                p.name, p.model, times[1], times[0]
            ),
        );
    } else {
        report.push(
            4,
            "model",
            Verdict::Fail,
            format!(
                "{} returned an empty answer ({:?}); a thinking model may have used its whole \
                 budget: raise max_tokens",
                p.name, r.stop_reason
            ),
        );
    }
}

async fn check_voice(
    config: &Config,
    personas: &PersonaRegistry,
    persona: &str,
    report: &mut Report,
) {
    let voice_config = match config.voice.as_ref().map(|v| v.to_voice_config()) {
        Some(Ok(Some(vc))) => vc,
        Some(Err(e)) => {
            let why = format!("voice config unusable: {e:#}");
            report.push(5, "voice out", Verdict::Warn, why.clone());
            report.push(6, "voice loop", Verdict::Warn, why);
            return;
        }
        _ => {
            report.push(5, "voice out", Verdict::Warn, "voice is off in the config");
            report.push(6, "voice loop", Verdict::Warn, "voice is off in the config");
            return;
        }
    };
    let voice = Voice::from_section(config.voice.as_ref(), personas.list()).await;
    let started = Instant::now();
    let wav = match voice.synthesize_for(persona, LOOP_PHRASE).await {
        Ok(wav) => wav,
        Err(e) => {
            report.push(
                5,
                "voice out",
                Verdict::Warn,
                format!("text-to-speech: {e}"),
            );
            report.push(6, "voice loop", Verdict::Warn, "needs text-to-speech");
            return;
        }
    };
    let speak = started.elapsed().as_secs_f64();
    if jarvis_voice::decode_wav(&wav, voice_config.max_seconds).is_err() {
        report.push(
            5,
            "voice out",
            Verdict::Warn,
            "the voice returned something that is not a WAV",
        );
        report.push(6, "voice loop", Verdict::Warn, "needs a working voice");
        return;
    }
    report.latency.speak = Some(speak);
    report.push(
        5,
        "voice out",
        Verdict::Pass,
        format!("{} KB of WAV in {speak:.1} s", wav.len() / 1024),
    );

    let started = Instant::now();
    match voice.transcribe_wav(wav).await {
        Ok(heard) => {
            let hear = started.elapsed().as_secs_f64();
            report.latency.hear = Some(hear);
            let overlap = word_overlap(LOOP_PHRASE, &heard);
            let verdict = if overlap >= LOOP_MIN_OVERLAP {
                Verdict::Pass
            } else {
                Verdict::Fail
            };
            report.push(
                6,
                "voice loop",
                verdict,
                format!(
                    "heard \"{heard}\" ({:.0}% of the words) in {hear:.1} s",
                    overlap * 100.0
                ),
            );
        }
        Err(e) => report.push(
            6,
            "voice loop",
            Verdict::Warn,
            format!("speech-to-text: {e}"),
        ),
    }
}

async fn check_private_files(http: &reqwest::Client, api: &str, report: &mut Report) {
    let mut served = Vec::new();
    let mut reached = false;
    for (path, marker) in PRIVATE_PATHS {
        let Ok(r) = http.get(format!("{api}{path}")).send().await else {
            continue;
        };
        reached = true;
        if !r.status().is_success() {
            continue;
        }
        let body = r.bytes().await.unwrap_or_default();
        if String::from_utf8_lossy(&body).contains(marker) {
            served.push(*path);
        }
    }
    if !reached {
        report.push(
            7,
            "private files",
            Verdict::Fail,
            "the server did not answer",
        );
    } else if served.is_empty() {
        report.push(
            7,
            "private files",
            Verdict::Pass,
            "config, .env, database and bridge key are not served",
        );
    } else {
        report.push(
            7,
            "private files",
            Verdict::Fail,
            format!("served to a browser: {}", served.join(", ")),
        );
    }
}

/// Non-loopback hosts in the running server's log, when it answers without a login.
async fn server_egress(http: &reqwest::Client, api: &str) -> Option<Vec<String>> {
    let r = http.get(format!("{api}/api/egress")).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: Value = r.json().await.ok()?;
    Some(
        v.get("non_loopback")?
            .as_array()?
            .iter()
            .filter_map(|h| h.as_str().map(str::to_owned))
            .collect(),
    )
}

/// Check 11: Assistant mode (spec-assistant-mode). Never calls Claude (that costs money): it
/// looks at the configuration, the API key's presence, this month's spending and the 3D model
/// sandbox page. Missing pieces are WARN: Trading mode works without any of them.
pub fn check_assistant(
    config: &Config,
    static_dir: &Path,
    env: impl Fn(&str) -> Option<String>,
    now: chrono::DateTime<chrono::Utc>,
    report: &mut Report,
) {
    const ID: u8 = 11;
    const NAME: &str = "assistant";
    let a = &config.assistant;
    if !a.enabled {
        report.push(
            ID,
            NAME,
            Verdict::Warn,
            "turned off ([assistant] enabled = false)",
        );
        return;
    }
    let Some(provider) = config.llm.providers.iter().find(|p| p.name == a.provider) else {
        report.push(
            ID,
            NAME,
            Verdict::Warn,
            format!("no [[llm.providers]] entry is named '{}'", a.provider),
        );
        return;
    };
    let key_set = match &provider.api_key_env {
        None => true,
        Some(var) => env(var).is_some_and(|v| !v.trim().is_empty()),
    };
    if !key_set {
        report.push(
            ID,
            NAME,
            Verdict::Warn,
            format!(
                "{} · {}: {} is not set, so Assistant mode cannot answer",
                provider.name,
                provider.model,
                provider.api_key_env.as_deref().unwrap_or("the API key")
            ),
        );
        return;
    }
    if !static_dir.join("sandbox/model.html").is_file() {
        report.push(
            ID,
            NAME,
            Verdict::Warn,
            format!(
                "no 3D sandbox page at {} (run `pnpm -C web build`)",
                static_dir.join("sandbox/model.html").display()
            ),
        );
        return;
    }
    let spend =
        crate::spend::SpendMeter::new(Some(a.spend_file.clone()), a.monthly_budget_usd, a.prices())
            .status(now);
    let verdict = if spend.blocked {
        Verdict::Warn
    } else {
        Verdict::Pass
    };
    report.push(
        ID,
        NAME,
        verdict,
        format!(
            "{} · {} ready; Claude this month ${:.2} of a ${:.2} cap{}",
            provider.name,
            provider.model,
            spend.spent_usd,
            spend.budget_usd,
            if spend.blocked { " (reached)" } else { "" }
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_config(extra: &str) -> Config {
        let dir = std::env::temp_dir().join(format!("arc-pf-{}", uuid::Uuid::new_v4()));
        Config::from_toml_str(&format!(
            r#"
[llm]
default = "claude"
[[llm.providers]]
name = "claude"
kind = "anthropic"
model = "claude-opus-5-5"
api_key_env = "ARC_TEST_KEY"
[assistant]
spend_file = '{}'
{extra}
"#,
            dir.join("spend.json").display()
        ))
        .unwrap()
    }

    fn verdict_of(config: &Config, key: Option<&str>, site: &Path) -> (Verdict, String) {
        let mut report = Report::default();
        let key = key.map(str::to_owned);
        check_assistant(
            config,
            site,
            |_| key.clone(),
            chrono::Utc::now(),
            &mut report,
        );
        let c = report.checks.pop().unwrap();
        (c.verdict, c.detail)
    }

    #[test]
    fn the_assistant_check_never_fails_and_says_what_is_missing() {
        let site = std::env::temp_dir().join(format!("arc-site-{}", uuid::Uuid::new_v4()));
        let config = assistant_config("");
        let (v, d) = verdict_of(&config, None, &site);
        assert_eq!(v, Verdict::Warn);
        assert!(d.contains("ARC_TEST_KEY is not set"), "{d}");
        let (v, d) = verdict_of(&config, Some("sk-x"), &site);
        assert_eq!(v, Verdict::Warn);
        assert!(d.contains("sandbox page"), "{d}");
        std::fs::create_dir_all(site.join("sandbox")).unwrap();
        std::fs::write(site.join("sandbox/model.html"), "<!doctype html>").unwrap();
        let (v, d) = verdict_of(&config, Some("sk-x"), &site);
        assert_eq!(v, Verdict::Pass, "{d}");
        assert!(
            d.contains("claude-opus-5-5") && d.contains("$20.00 cap"),
            "{d}"
        );
        assert!(!d.contains("sk-x"), "the key never appears");
        let (v, _) = verdict_of(&assistant_config("enabled = false"), Some("sk-x"), &site);
        assert_eq!(v, Verdict::Warn);
        let _ = std::fs::remove_dir_all(site);
    }

    #[test]
    fn word_overlap_counts_expected_words() {
        assert_eq!(
            word_overlap(LOOP_PHRASE, "please show my open positions on gold"),
            1.0
        );
        assert!(word_overlap(LOOP_PHRASE, "Please show my open positions on cold.") > 0.8);
        assert!(word_overlap(LOOP_PHRASE, "show positions") < 0.5);
        assert_eq!(word_overlap(LOOP_PHRASE, ""), 0.0);
        assert_eq!(word_overlap("", "anything"), 1.0);
    }

    #[test]
    fn ollama_tags_match_with_and_without_a_tag() {
        let tags = json!({"models": [{"name": "qwen3.5:9b"}, {"model": "gemma2:latest"}]});
        assert!(ollama_has_model(&tags, "qwen3.5:9b"));
        assert!(ollama_has_model(&tags, "QWEN3.5:9B"));
        assert!(ollama_has_model(&tags, "gemma2"));
        assert!(!ollama_has_model(&tags, "qwen3.5:4b"));
        assert!(!ollama_has_model(&json!({}), "gemma2"));
    }

    #[test]
    fn external_loads_are_found_and_text_urls_are_not() {
        let html = r#"<script src="https://cdn.jsdelivr.net/npm/x@1/x.js"></script>
            <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter">
            <a href="https://github.com/ninad-k/Arc-MT">source</a>
            <svg xmlns="http://www.w3.org/2000/svg"></svg>
            <img src="http://127.0.0.1:8787/icon.png">"#;
        assert_eq!(
            external_loads(html, FileKind::Html),
            [
                "https://cdn.jsdelivr.net/npm/x@1/x.js",
                "https://fonts.googleapis.com/css2?family=Inter"
            ]
        );
        let js = r#"import("https://esm.sh/three@0.160.0");const e="https://svelte.dev/e/x";
            import x from "https://unpkg.com/y@1";fetch("https://api.example.com/v1")"#;
        assert_eq!(
            external_loads(js, FileKind::Script),
            [
                "https://esm.sh/three@0.160.0",
                "https://unpkg.com/y@1",
                "https://api.example.com/v1"
            ]
        );
        let css =
            "@import url(https://fonts.example.com/a.css); a{background:url('https://x.io/b.png')}";
        assert_eq!(external_loads(css, FileKind::Css).len(), 2);
        assert!(external_loads("/* see https://example.com */", FileKind::Css).is_empty());
    }

    #[test]
    fn env_secrets_skip_short_values_and_comments_and_strip_quotes() {
        let env = "# comment\nJARVIS_JWT_SECRET=\"abcdefghijklmnop\"\nRUST_LOG=info\n\
                   export ANTHROPIC_API_KEY='sk-ant-1234567890'\nEMPTY=\n";
        let s = env_secrets(env);
        assert_eq!(
            s,
            [
                (
                    "JARVIS_JWT_SECRET".to_owned(),
                    "abcdefghijklmnop".to_owned()
                ),
                (
                    "ANTHROPIC_API_KEY".to_owned(),
                    "sk-ant-1234567890".to_owned()
                ),
            ]
        );
        assert_eq!(
            leaked_secrets("var k='sk-ant-1234567890'", &s),
            ["ANTHROPIC_API_KEY"]
        );
        assert!(leaked_secrets("nothing here", &s).is_empty());
    }

    #[test]
    fn bundle_check_reads_files_and_reports() {
        let dir = std::env::temp_dir().join(format!("jarvis-preflight-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("_app")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<script src=\"/_app/a.js\"></script>",
        )
        .unwrap();
        std::fs::write(dir.join("_app/a.js"), "const k='secret-value-1';").unwrap();
        let secrets = vec![("API_KEY".to_owned(), "secret-value-1".to_owned())];
        let mut report = Report::default();
        check_bundle(&dir, &secrets, &mut report);
        assert_eq!(
            report.checks[0].verdict,
            Verdict::Pass,
            "{:?}",
            report.checks
        );
        assert_eq!(report.checks[1].verdict, Verdict::Fail);
        assert!(report.checks[1].detail.contains("API_KEY"));
        assert!(!report.checks[1].detail.contains("secret-value-1"));

        let mut missing = Report::default();
        check_bundle(&dir.join("nope"), &secrets, &mut missing);
        assert!(missing.checks.iter().all(|c| c.verdict == Verdict::Warn));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn egress_fails_only_in_local_mode() {
        let hosts = vec!["api.anthropic.com".to_owned()];
        for (local, hosts, want) in [
            (true, vec![], Verdict::Pass),
            (true, hosts.clone(), Verdict::Fail),
            (false, hosts, Verdict::Warn),
        ] {
            let mut r = Report::default();
            check_egress(local, &hosts, &mut r);
            assert_eq!(r.checks[0].verdict, want);
        }
    }

    #[test]
    fn report_lines_and_exit_code() {
        let mut r = Report::default();
        r.push(1, "config", Verdict::Pass, "ok");
        r.push(5, "voice out", Verdict::Warn, "off");
        assert_eq!(r.exit_code(), 0);
        assert_eq!(r.summary_line(), "SUMMARY  1 pass, 0 fail, 1 warn");
        assert_eq!(
            r.latency_line(),
            "LATENCY  hear n/a  think n/a  speak n/a  total n/a"
        );
        r.latency = Latency {
            hear: Some(0.8),
            think: Some(2.4),
            speak: Some(0.6),
        };
        assert_eq!(
            r.latency_line(),
            "LATENCY  hear 0.8 s  think 2.4 s  speak 0.6 s  total 3.8 s"
        );
        r.push(2, "api", Verdict::Fail, "down");
        assert_eq!(r.exit_code(), 1);
        assert_eq!(r.to_json()["summary"]["fail"], 1);
        assert!(r.render_text().contains("FAIL   2 api"));
    }

    fn config(toml: &str) -> Config {
        Config::from_toml_str(toml).unwrap()
    }

    #[test]
    fn local_mode_needs_a_loopback_model_and_no_news() {
        let local = r#"
            [llm]
            default = "local"
            [[llm.providers]]
            name = "local"
            kind = "ollama"
            model = "m"
        "#;
        assert!(is_local_mode(&config(local)));
        assert!(!is_local_mode(&config(&local.replace(
            "model = \"m\"",
            "model = \"m\"\nbase_url = \"http://192.168.1.5:11434\""
        ))));
        let cloud = r#"
            [llm]
            default = "claude"
            [[llm.providers]]
            name = "claude"
            kind = "anthropic"
            model = "m"
        "#;
        assert!(!is_local_mode(&config(cloud)));
        assert!(!is_local_mode(&config(
            &local.replace("model = \"m\"", "model = \"minimax-m3:cloud\"")
        )));
    }

    #[test]
    fn self_signed_certificates_are_accepted_on_this_machine_only() {
        assert!(accepts_self_signed("https://127.0.0.1:8787"));
        assert!(accepts_self_signed("https://localhost:8787"));
        assert!(!accepts_self_signed("https://arc.example.com"));
        assert!(!accepts_self_signed("https://192.168.1.4:8787"));
        assert!(!accepts_self_signed("not a url"));
    }

    #[test]
    fn ollama_cloud_models_are_recognised() {
        for cloud in ["minimax-m3:cloud", "gpt-oss:120b-cloud", "x:CLOUD"] {
            assert!(is_ollama_cloud_model(cloud), "{cloud}");
        }
        for local in ["qwen3.5:9b", "cloud-model", "qwen2.5:7b", "cloudy"] {
            assert!(!is_ollama_cloud_model(local), "{local}");
        }
    }

    #[test]
    fn api_base_follows_the_configured_bind() {
        let c = config(
            r#"
            [server]
            bind = "127.0.0.1:9000"
            [llm]
            default = "l"
            [[llm.providers]]
            name = "l"
            kind = "ollama"
            model = "m"
        "#,
        );
        assert_eq!(api_base(&c), "http://127.0.0.1:9000");
    }

    /// The real thing against the running stack (`scripts\\run.ps1` first), from the repository
    /// root: `cargo test -p jarvis-api --lib preflight::tests::live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs the running stack, Ollama and the models"]
    async fn live_preflight_against_the_running_stack() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::env::set_current_dir(&root).unwrap();
        let report = run(Options::default()).await;
        println!("{}", report.render_text());
        assert_eq!(report.exit_code(), 0, "{}", report.render_text());
    }
}

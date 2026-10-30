# Spec: market views (intraday analysis)

Status: approved 2026-10-30. Decision record: ADR-0019.

## Objective

A trader asks "what's your view on gold for today?" (typed or spoken) and gets an intraday
view built from the broker's own prices, a technical analysis computed in code, the economic
calendar and, when a news provider is configured, headline sentiment:

- a bias (bullish, bearish or neutral) with a confidence from 0 to 100;
- when there is a setup: an entry zone, a stop loss and two take-profit levels sized from ATR;
- the reasons (each factor with its signal), key levels, upcoming high-impact events and the
  news tone;
- "stand aside" when a high-impact event for the instrument's currencies is close, or when the
  spread is too wide for the current volatility.

A view is information. It never places or proposes a trade by itself. "Prepare trade" turns a
view into the normal preview, which goes through every limit and needs the usual confirmation.
Every view is stored and later scored against what the price did, so the trader can see how
reliable the views are before relying on them.

Out of scope: automatic trading, signals pushed without a request, backtesting UI, stocks.

## Success criteria

1. `get_rates` and `get_calendar` work against the demo terminal (EA 0.5.0) and the mock.
2. Every indicator matches a hand-computed reference within 1e-9 on fixed fixtures.
3. `analyze` is deterministic: the same input gives the same view (golden tests).
4. The model can call `get_market_view`, `get_economic_calendar` and `get_news`; every figure
   it repeats from them passes the grounding check.
5. A view never creates a preview by itself; `POST /api/market/prepare` needs an explicit
   volume and goes through `Trading::propose` with all limits.
6. With no news provider configured everything works; the news section says "not configured".
7. Views are journaled and scored (TP first, SL first, expired, bias right or wrong).
8. `scripts/check.ps1` stays green: fmt, clippy, tests, web check/unit/build/e2e, audits,
   gitleaks, semgrep.

## Architecture and contracts

```
MT5 EA ── get_rates / get_calendar ──► bridge-client ──► api (market.rs)
                                                          │  converts bars, events
                                       jarvis-market ◄────┤  analyze(ViewInput) -> MarketView
                                       jarvis-news   ◄────┘  NewsService::summary(...)
api ── tools (get_market_view, get_economic_calendar, get_news), /api/market/*, journal
web ── Market view panel, Prepare trade dialog, track record
```

### 1. Bridge protocol (additive; protocol version stays 2, EA 0.5.0)

`get_rates`

```json
{"cmd": "get_rates", "args": {"symbol": "XAUUSD_", "timeframe": "M15", "count": 300}}
```

- `timeframe`: one of `M1 M5 M15 M30 H1 H4 D1`. `count`: 1 to 1000 (`RATES_COUNT_MAX`),
  default 300.
- Reply `data`: `{"symbol", "timeframe", "digits", "bars": [{"time", "open", "high", "low",
  "close", "tick_volume", "spread"}]}`. Oldest first. `time` is the bar open time in broker
  server time (unix seconds, as MT5 reports it). The last bar is the current, still forming one.
- Fewer bars than asked is not an error. History not loaded yet: `ok:false` with the MT5 error
  code as retcode; the client retries once after 500 ms.

`get_calendar`

```json
{"cmd": "get_calendar", "args": {"from": 1793370000, "to": 1793456400, "currencies": ["USD", "EUR"], "min_importance": "moderate"}}
```

- `from`/`to`: broker server time, unix seconds; range at most 14 days. `currencies` optional
  (empty or missing = all). `min_importance`: `low`, `moderate` (default) or `high`.
- Reply `data`: `{"events": [{"time", "currency", "country", "event", "importance", "actual",
  "forecast", "previous", "unit"}]}`, sorted by time, at most 500. `actual`/`forecast`/`previous`
  are numbers or `null` when MT5 has no value. `importance`: `low|moderate|high`.
- Uses MT5's built-in economic calendar (no external service).

Rust types in `crates/bridge-client/src/protocol.rs`: `Timeframe`, `RatesQuery`, `Bar`,
`Rates`, `Importance`, `CalendarQuery`, `CalendarEvent`, `Calendar`; client methods
`get_rates(RatesQuery)` and `get_calendar(CalendarQuery)`. The mock serves deterministic
synthetic bars (seeded random walk per symbol and timeframe, anchored to the mock price) and
a canned calendar.

### 2. `jarvis-market` crate (pure, no I/O, no bridge dependency)

Time-zone agnostic: every time is unix seconds on one clock (the API passes broker server
time).

```rust
pub struct Bar { pub time: i64, pub open: f64, pub high: f64, pub low: f64, pub close: f64, pub tick_volume: u64, pub spread: i64 }
pub enum Timeframe { M5, M15, H1, H4, D1 }            // frames the view uses
pub mod indicators;  // sma, ema, rsi (Wilder), macd, atr (Wilder), bollinger, vwap, pivots, swing_levels
pub struct EventInput { pub time: i64, pub currency: String, pub title: String, pub importance: Importance }
pub struct NewsInput { pub score: f64 /* -1..1, already in the instrument's direction */, pub count: u32 }
pub struct ViewInput {
    pub symbol: String, pub digits: u32, pub point: f64,
    pub bid: f64, pub ask: f64, pub now: i64,
    pub frames: BTreeMap<Timeframe, Vec<Bar>>,   // M5 M15 H1 H4: 300 bars, D1: 30 bars
    pub currencies: Vec<String>,                 // e.g. XAUUSD -> ["USD"], EURUSD -> ["EUR","USD"]
    pub events: Vec<EventInput>,
    pub news: Option<NewsInput>,
}
pub struct ViewConfig { .. }  // Default = the numbers below; serde so [market] can override
pub fn analyze(input: &ViewInput, cfg: &ViewConfig) -> MarketView;
```

Signals use closed bars only (the forming last bar is dropped) and the live bid/ask.

Scoring (defaults in `ViewConfig`): factors, each from -1 to +1 times its weight; score is the
sum clamped to -100..100.

| Factor | Weight | Rule |
|---|---|---|
| H4 trend | 25 | EMA20 vs EMA50 and close vs EMA50: both up +1, both down -1, mixed 0 |
| H1 trend | 20 | same on H1 |
| M15 momentum | 15 | MACD(12,26,9) histogram sign, ±0.5 more if it grew in that direction over 3 bars (clamped) |
| M15 RSI(14) | 10 | > 70: -1 (stretched), 55..70: +0.5, 45..55: 0, 30..45: -0.5, < 30: +1 (stretched) |
| VWAP | 10 | mid above today's VWAP +1, below -1 (session = server day) |
| Daily pivot | 10 | mid above classic pivot of the previous D1 bar +1, below -1 |
| News | 10 | `news.score` when present, else factor omitted |

- Bias: score ≥ 20 bullish, ≤ -20 bearish, else neutral.
- Confidence: |score|, capped at 50 when H4 and H1 trends disagree, rounded to an integer.
- Stand aside (no setup, bias still reported): a `high` event for one of `currencies` within
  30 minutes before or after `now`; or spread (ask - bid) > 25% of ATR(14) on M15.
- Setup (only when bias is not neutral and not standing aside), with `a` = ATR(14) on H1 and
  `m` = ATR(14) on M15. Buy: entry zone `[ask - 0.5m, ask]`, stop `entry_mid - 1.0a`, TP1
  `entry_mid + 1.5a`, TP2 the nearest swing resistance above TP1 within 3a, else
  `entry_mid + 2.5a`. Sell mirrors it with bid and supports. All prices rounded to `digits`.
  `risk_reward` = TP1 distance / stop distance, two decimals.
- `valid_until` = `now` + 240 minutes.
- `summary`: a deterministic sentence built from the bias, confidence, the two strongest factors
  and the setup (no model text).

`MarketView` (serde, snake_case) is the JSON below minus the fields the API adds
(`id`, `broker_symbol`, `generated_at`, `valid_until` as ISO UTC, `news.headlines`, event
`time` as ISO UTC and `minutes_away`).

### 3. `jarvis-news` crate

```rust
pub struct Headline { pub title: String, pub source: String, pub url: String, pub published_at: DateTime<Utc>, pub tone: f64 }
pub struct NewsQuery { pub canonical: String, pub terms: Vec<String>, pub base: Option<String>, pub quote: Option<String>, pub since: DateTime<Utc>, pub limit: usize }
#[async_trait] pub trait NewsProvider: Send + Sync { fn name(&self) -> &'static str; async fn fetch(&self, q: &NewsQuery) -> Result<Vec<Headline>, NewsError>; }
pub struct Finnhub;       // GET https://finnhub.io/api/v1/news?category=forex|general|crypto, key in the X-Finnhub-Token header
pub struct AlphaVantage;  // GET https://www.alphavantage.co/query?function=NEWS_SENTIMENT, uses its own sentiment scores
pub struct NewsService;   // provider + 10-minute cache per query + relevance filter + direction mapping
pub struct NewsSummary { pub score: f64, pub label: String, pub count: u32, pub headlines: Vec<Headline> }
impl NewsService { pub async fn summary(&self, q: &NewsQuery) -> Result<NewsSummary, NewsError>; }
```

- Relevance: a headline counts when its title or summary contains one of `terms` (canonical,
  aliases, keywords, currency names) as a whole word, case-insensitive.
- Tone without provider scores: a small finance lexicon (positive and negative word lists,
  negation within three words before), -1..1 per headline.
- Direction: for a pair with `base`/`quote`, a headline about the base keeps its tone, about
  the quote flips it, about both counts 0. Other instruments keep the tone.
- Score: mean of directed tones, most recent 24 hours, newer headlines weigh more (half-life
  6 hours). Label: `negative` ≤ -0.35 < `slightly negative` ≤ -0.1 < `neutral` < 0.1 ≤
  `slightly positive` < 0.35 ≤ `positive`.
- API keys never appear in logs, errors or URLs that are logged; Alpha Vantage needs the key
  in the query string, so its request URL is never logged.
- `wiremock` tests for both providers; lexicon and direction unit tests.

### 4. API (`crates/api`)

Config:

```toml
[market]
enabled = true
horizon_minutes = 240
event_blackout_minutes = 30
calendar_min_importance = "moderate"
[market.news]
provider = "none"            # none | finnhub | alphavantage
api_key_env = "FINNHUB_API_KEY"
cache_minutes = 10
```

Tools (new `ReadTool` variants, still read-only):

- `get_market_view {symbol}`: the view JSON (also journaled).
- `get_economic_calendar {hours_ahead?: 1..168 = 24, currencies?: [..]}`.
- `get_news {symbol}`: summary and up to 5 headlines.

HTTP:

| Method | Path | Body / query | Reply |
|---|---|---|---|
| GET | `/api/market/view` | `symbol` | `MarketViewDto` |
| GET | `/api/market/calendar` | `hours` (1..168), `currencies` (comma list) | `{events: [...]}` |
| GET | `/api/market/news` | `symbol` | `{configured, provider, summary}` |
| GET | `/api/market/journal` | `limit` (≤ 200), `symbol` | `{stats, views: [...]}` |
| POST | `/api/market/prepare` | `{view_id, volume, order_type: "market" or "limit"}` | `PreviewDto` (same as other previews) |

`MarketViewDto`:

```json
{
  "id": "uuid", "symbol": "XAUUSD", "broker_symbol": "XAUUSD_",
  "generated_at": "2026-10-30T13:50:00Z", "valid_until": "2026-10-30T17:50:00Z",
  "price": {"bid": 3925.09, "ask": 3925.22},
  "bias": "bullish", "confidence": 62, "score": 62,
  "stand_aside": false, "stand_aside_reason": null,
  "setup": {"side": "buy", "entry_low": 3921.4, "entry_high": 3925.22, "stop_loss": 3911.0,
            "take_profit": 3944.1, "take_profit_2": 3958.0, "risk_reward": 1.5,
            "atr_h1": 12.4, "atr_m15": 5.1},
  "timeframes": [{"timeframe": "H4", "trend": "up", "ema20": 0, "ema50": 0, "rsi": 0, "macd_hist": 0, "atr": 0}],
  "levels": {"pivot": 0, "r1": 0, "r2": 0, "s1": 0, "s2": 0, "prev_day_high": 0, "prev_day_low": 0,
             "vwap": 0, "support": [0], "resistance": [0]},
  "factors": [{"name": "H4 trend", "signal": "bullish", "points": 25, "detail": "EMA20 above EMA50, close above EMA50"}],
  "events": [{"time": "2026-10-30T14:30:00Z", "minutes_away": 40, "currency": "USD", "title": "Nonfarm Payrolls", "importance": "high"}],
  "news": {"configured": true, "score": 0.2, "label": "slightly positive", "count": 7,
           "headlines": [{"title": "", "source": "", "url": "", "published_at": "", "tone": 0.4}]},
  "summary": "Bullish intraday bias (62/100): ...",
  "disclaimer": "Computed from price data, indicators, the economic calendar and headlines. It is not financial advice and no outcome is guaranteed."
}
```

Prepare: the view must exist, be unexpired and have a setup; `volume` is required (no default,
rule 9); `market` uses the setup's stop and TP1, `limit` places at `entry_low` (buy) or
`entry_high` (sell). It builds an `OrderIntent` and calls `Trading::propose` with
`ProposeSource::Ui`; all gate limits apply. It never confirms.

Journal: migration `0007_market_views.sql` stores each view (id, created_at, symbol,
broker_symbol, bias, confidence, score, stand_aside, side, entry, stop, tp, price, valid_until,
view_json) plus outcome columns (`outcome`: `open | tp_hit | sl_hit | expired | no_setup`,
`evaluated_at`, `bias_correct`, `move_atr`). A task every 5 minutes scores open views from M5
bars: TP before SL → `tp_hit`; SL first, or both in one bar → `sl_hit`; past `valid_until` →
`expired`. Views without a setup become `no_setup` at `valid_until`. For every view,
`bias_correct` = the close at `valid_until` moved at least 0.25 ATR(H1) in the bias direction
(neutral: moved less than that). Stats: totals per outcome, TP rate among setups, bias accuracy.

Prompt: the rule "do not give investment recommendations" becomes: for a view or a buy/sell
opinion, call `get_market_view` and relay its bias, confidence, levels and main reasons in two
or three sentences; say it is a computed analysis and not a guarantee; never present it as
certain; never propose a trade from a view unless the user asks for that trade with a volume.

Audit: each view request is audited (`kind = market_view`, symbol, bias, confidence).

### 5. Web (`web/`)

- A **Market view** panel in the HUD: symbol picker (watchlist plus free text), Analyze button,
  and a card with the bias pill, confidence meter, stand-aside banner, setup (entry zone, stop,
  TP1, TP2, R:R), per-timeframe trend table, key levels, upcoming events with minutes away,
  news tone with headlines (links open in a new tab, `rel="noopener noreferrer"`), factors,
  summary, disclaimer, and when it expires.
- **Prepare trade** (only with a setup): a dialog with a required volume and market or limit;
  `POST /api/market/prepare`, then the existing confirmation modal.
- **Track record**: stats and the latest views with outcomes (`GET /api/market/journal`).
- Mock mode serves all of it. Accessible (labels, focus, contrast), works at 375 px.

## Commands

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm -C web check; pnpm -C web test; pnpm -C web build; pnpm -C web test:e2e
powershell -File scripts/compile-ea.ps1
powershell -File scripts/check.ps1 -SkipWhisper
```

## Boundaries

- Always: the gate decides every trade; tests for every rule above; numbers come from code.
- Ask first: new paid services, changing limits or confirmation behaviour.
- Never: a view that proposes or executes by itself; API keys in logs, URLs or the repo.

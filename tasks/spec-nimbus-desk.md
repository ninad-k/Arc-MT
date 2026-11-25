# Spec: Nimbus Desk (a nine-agent trading desk, as a third mode of Arc MT)

Status: **approved 2026-11-24 with the defaults for the open questions** (separate $20 Desk cap; Scout scans
the watchlist plus `[desk] universe`; Friday only; Sentinel at 0.5% risk, stop required, 2 open Desk
positions, no new trades in the last 10 minutes before the US close). Branch: `nimbus-desk`. Plan:
[plan-nimbus-desk.md](plan-nimbus-desk.md). The journal is kept in the audit log (new kinds) rather than a
new `desk_journal` table, decided while planning.

Source: the user's request ("implement https://www.cloud9markets.com/nimbus-desk.html as a separate project
based on Arc MT") and the page itself, a high-level build guide for a voice-driven "AI trading desk": one
master agent that talks to the user and routes each command to eight specialists, a dark HUD with an orb and
nine agent seats, and a single gated path for money. The page publishes no protocol, no risk rules and no data
model, so every such detail below is our design.

Choices the user made on 2026-11-24:

- **Placement:** new crates in the Arc MT workspace, and a new route set in the existing web app.
- **Execution:** the Pilot agent trades **through the Arc MT MT5 bridge**, using the existing preview,
  15-second token and confirm flow.
- **Brain:** **Claude by default** for the master agent (paid, per use; see Cost).
- **Scope:** the full HUD and **all nine agents at MVP depth**. TradingView control, TimesFM forecasting, the
  Electron shell and hand tracking are out of scope (stubbed or replaced, see below).

## Objective

A single trader opens **Desk** mode, types or says one command ("what's moving today?", "give me the full
desk view on NVDA", "size a long on NVDA with a stop at 118"), and gets one spoken answer from the master agent,
built from the specialists' findings. The HUD shows which agents worked, what each found, the market pulse, the
US session clock and the calendar. A trade happens only after the risk agent approves it **and** the trader
confirms the usual preview.

### The desk

| Seat | Agent id | Job (MVP) | Data |
|---|---|---|---|
| Master | `friday` | Takes the command, calls the specialists as tools, merges the results, answers. Does no specialist work. | Claude (or any configured provider) |
| Intelligence | `atlas` (Macro) | Fed and rates context: next FOMC and high-impact US events, macro headline tone, index moves | MT5 economic calendar, Finnhub general news, broker index symbols |
| Intelligence | `capitol` (Smart money) | Insider buying and selling for a symbol (net shares, notable trades, last 90 days); congressional trades when the key allows | Finnhub insider transactions (free); congressional trading (premium, shown as "—" otherwise) |
| Intelligence | `scout` (Recon) | Today's movers across the scan universe: % change, range, relative volume | MT5 bars for the configured universe |
| Analysis | `athena` (Analyst) | Fundamentals snapshot (P/E, margins, 52-week range) and recent filings, plus chart structure: supply and demand zones, liquidity levels | Finnhub basic financials and filings, `jarvis-market` views on MT5 bars |
| Analysis | `chartist` (Technician) | Drives the desk chart: switches symbol and timeframe, draws levels (entry, stop, targets, zones, forecast band) | MT5 bars; draws in the app's own chart |
| Analysis | `oracle` (Quant) | Answers in probabilities: chance of closing above or below a level within N bars, and an expected range | Historical returns from MT5 bars (a statistical model in code, in place of TimesFM) |
| Execution | `sentinel` (Risk) | Sizes the position from risk per trade and stop distance, checks Desk rules on top of the gate limits, and **approves or vetoes** with reasons | Account equity, symbol specs, open positions, gate limits |
| Execution | `pilot` (Execution) | Turns an **approved** plan into the normal Arc MT preview (15 s token). Cannot run without a Sentinel approval id | `jarvis-gate`, the existing propose path |
| Execution | `ledger` (Book) | Journals every desk decision (command, agents called, findings, approvals, vetoes, previews, outcomes) and reports system health | SQLite, bridge, provider and news status |

### Acceptance criteria

1. A **Trading | Assistant | Desk** switch. Desk mode is reachable from the top bar and needs the same login.
2. The Desk HUD shows: the orb at the centre with nine agent seats around it (idle, working, done, error,
   offline); on the left the market pulse (quote rings, red when down), the task queue and the signal log; on
   the right the US session clock (pre-market, open, after-hours, closed, with a countdown, computed in New York
   time with daylight saving), the calendar (FOMC dots from the MT5 calendar, monthly options expiry on the
   third Friday) and agent load; a chart panel the Chartist drives; the transcript and composer.
3. Every number shown or spoken comes from a feed or from code. **Missing data shows as "—"**, never as an
   invented value. Agent results carry their source and timestamp.
4. Friday answers a desk command by calling one or more specialists (as tools) and replying with a synthesis;
   each specialist call appears in the signal log and lights its seat.
5. **Money moves through one gated path:** analysis agents have no write path; `pilot` needs a fresh
   `sentinel` approval id for the same symbol, side and volume; the preview then goes through the existing gate
   limits, kill switch, 15-second token and confirm. No Desk tool sends a trade. A Sentinel veto is final for
   that plan.
6. Ledger records every step in a `desk_journal` table and in the existing audit log, and the journal is
   viewable in the HUD.
7. Voice works as in Trading mode (Whisper in, Piper out). Only Friday speaks.
8. Claude spend for Desk mode is capped per month (shared with or alongside the Assistant cap) and shown in
   the HUD; at the cap Desk mode says so and stops calling Claude. The provider can be switched to Ollama.
9. The web app's mock mode (`pnpm dev:mock`) runs the whole Desk HUD with no backend.

## Design (summary; an ADR records it)

- **`crates/desk`** (new library crate, `jarvis-desk`): the `DeskAgent` trait (id, describe, a typed request,
  a typed `Finding` with source and time), the nine agents, the Sentinel rules, the session clock and the
  calendar maths. Pure where it can be: agents take data through small traits (`Quotes`, `Bars`, `Calendar`,
  `Fundamentals`, `Insiders`), so tests use fixtures and never the network.
- **`crates/api`**: a `desk` module that owns the Desk agent loop (a sibling of `assistant.rs`), implements
  those data traits over the bridge client, `jarvis-news` (extended with the Finnhub insider, filings and
  basic-financials endpoints) and `jarvis-market`, routes under `/api/desk/*`, events over the existing
  WebSocket, and the `desk_journal` migration. Pilot calls the same internal propose function the trading
  tools use, so previews, limits and confirm are not duplicated.
- **Integration points checked in the code:** Pilot builds a `ProposeRequest` and calls `Trading::propose`
  (the same function the trading tools and the Prepare-trade button use), so the gate, sizing
  (`sizing::size_by_risk`), token and `POST /api/confirm/{token}` are reused unchanged. A Pilot call from
  Friday uses the LLM source, so the existing intent cross-check against the trader's words applies; the
  "Send to Pilot" button on an approved plan card uses the UI source. Desk turns reuse the `SpendMeter`
  reserve/settle pattern from Assistant mode. Desk gets its own Finnhub client (the news crate only reads
  headlines today), wrapped for the egress log like `EgressNews`. Desk events are a new variant of the
  WebSocket `Event` enum, and journal steps are new `AuditKind` variants.
- **"Each agent is its own service"** is met as one module per agent behind one trait, each with its own
  config section and status, so an agent can be added or disabled without touching the others. Separate
  processes are not worth it for one trader on one PC (recorded in the ADR).
- **Web**: a `desk` route set and components (`DeskHud`, `AgentRing`, `MarketPulse`, `SignalLog`,
  `TaskQueue`, `SessionClock`, `DeskCalendar`, `AgentLoad`, `DeskChart` drawn in SVG, `Journal`), reusing the
  Orb, Transcript, Composer, ConfirmModal and voice state. Brand accent is gold (the Friday palette).

### Out of scope (this build)

TradingView control through MCP (the chart is our own), TimesFM (a statistical model in code instead), the
Electron shell (the Tauri app and PWA already exist), MediaPipe hand tracking, per-agent voices, crypto and
futures specifics, and automatic or scheduled trading.

## Cost

- **Building and testing costs nothing:** tests use mocks and fixtures and never call Claude or Finnhub.
- **Using Desk mode on Claude is billed to the user's Anthropic key.** Default model `claude-opus-5-5`,
  $4 per million input tokens and $20 per million output tokens (Anthropic price list, cached 2026-11-03).
  **Estimate:** a desk turn with 2 to 4 specialist calls is about 10,000 to 20,000 input and 1,000 to 2,000
  output tokens, so about **$0.06 to $0.12 a turn**, less with prompt caching. At 20 turns a trading day that
  is about **$25 to $50 a month**; the monthly cap (default **$20**, proposed) stops it there.
- **Finnhub:** the free tier covers news, insider transactions, filings and basic financials (60 calls a
  minute). Congressional trading is a premium endpoint and stays off unless the user has it.
- Nothing is billed until the user runs Desk mode with a Claude key; I will not run it against Claude
  without approval.

## Tech stack

Unchanged from Arc MT: Rust 1.88+ (edition 2024), Axum, SQLx and SQLite, `jarvis-llm`; SvelteKit 2 with
Svelte 5 runes, Tailwind 4, Vitest and Playwright. **No new dependencies planned** (New York time is computed
from the US daylight-saving rule in code rather than adding `chrono-tz`).

## Commands

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm -C web check
pnpm -C web test
pnpm -C web test:e2e
pnpm -C web dev:mock          # Desk HUD with no backend
pwsh scripts/check.ps1        # everything
```

## Project structure

```
crates/desk/src/        lib.rs, agent.rs, agents/{atlas,capitol,scout,athena,chartist,oracle,sentinel,pilot,ledger}.rs,
                        session.rs (NY clock), calendar.rs (FOMC, opex), data.rs (data traits), types.rs
crates/desk/tests/      fixtures and contract tests per agent
crates/api/src/         desk.rs (agent loop), desk_routes.rs, desk_data.rs (trait impls); migration for desk_journal
crates/news/src/        finnhub.rs extended: insider transactions, filings, basic financials
web/src/routes/desk/    the Desk page
web/src/lib/desk/       components, state, mock data
docs/                   desk-mode.md (user guide), adr/0023-nimbus-desk.md; README, architecture, api, configuration updated
config/jarvis.example.toml  [desk] section
```

## Code style

Follow CONTRIBUTING.md: `thiserror` in libraries, no `unwrap()`/`expect()` outside tests, money rounded to the
symbol's step and digits, Conventional Commits with scope `desk` (new) or the crate touched.

```rust
/// What a specialist found. Every number carries where it came from.
pub struct Finding {
    pub agent: AgentId,
    pub summary: String,            // one or two sentences for Friday to use
    pub data: serde_json::Value,    // typed per agent, `null` fields mean "no data" (shown as "—")
    pub sources: Vec<Source>,       // feed name and time of each input
}
```

## Testing strategy

- **Unit tests** (in `crates/desk`): each agent on fixtures, including the "no data" path; Sentinel sizing and
  every veto rule; the NY session clock across both daylight-saving changes, weekends and holidays; opex and
  FOMC extraction.
- **Safety tests** (in `crates/api/tests`, mock bridge): Pilot without an approval fails; an approval for a
  different symbol, side or volume fails; an expired approval fails; a vetoed plan cannot be previewed; the
  kill switch and gate limits still apply; no Desk tool reaches a bridge write; the spend cap stops Claude calls.
- **Agent loop tests** with a scripted LLM provider (no network), as the existing agent tests do.
- **Web:** Vitest for the clock, calendar and formatting helpers ("—" for missing); a Playwright smoke test of
  Desk mode against the mock backend.

## Boundaries

- **Always:** keep core rules 1 to 10 of SPEC.md true in Desk mode; tests before each commit; docs and the
  example config in the same change.
- **Ask first:** any new dependency; changes to gate, tokens, kill switch, auth or bridge write code (this
  plan reuses them unchanged); running anything that calls Claude or places a demo trade.
- **Never:** a Desk tool that writes to MT5; trading on a real account in tests; committing `.env` or keys;
  presenting a forecast or view as advice.

## Success criteria

- All commands above pass on the branch.
- In mock mode, the Desk HUD shows all panels, and a scripted command lights the right seats and fills the log.
- Against the mock bridge, a full "size and buy" flow produces a Sentinel approval, a Pilot preview and a
  confirm, and each of the safety tests above fails if its rule is removed.

## Open questions

1. **Spend cap:** a separate Desk cap (proposed $20 a month) or one cap shared with Assistant mode?
2. **Scan universe for Scout:** default to the watchlist plus a list in `[desk] universe` (proposed: the
   broker's US stock and index symbols you add), or something else?
3. **Persona:** Friday only (as on the page), or also Jarvis as the Desk master?
4. **Risk defaults for Sentinel:** proposed 0.5% of equity at risk per trade, a stop required, at most 2 open
   Desk positions, no new trades in the last 10 minutes before the US close. Change any of these?

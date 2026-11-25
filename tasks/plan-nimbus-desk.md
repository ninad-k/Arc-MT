# Plan: Nimbus Desk (Desk mode)

Spec: [spec-nimbus-desk.md](spec-nimbus-desk.md), approved 2026-11-24 with the defaults for its open
questions:
- a separate Desk spend cap of $20 a month;
- Scout scans the watchlist plus `[desk] universe`;
- Friday only as the Desk master;
- Sentinel: 0.5% risk per trade, a stop required, at most 2 open Desk positions, no new trades in the last
  10 minutes before the US close.

## Decisions made while planning

- **`crates/desk` (`jarvis-desk`) is pure**: no I/O, no async. It holds the types, the session clock, the
  calendar maths and the nine agents' logic as functions over plain inputs (`jarvis_market::Bar`, calendar
  events, insider rows, fundamentals). The API gathers the data and calls them. Tests need no network and no
  bridge.
- **The Desk loop lives in `crates/api/src/desk.rs`**, a sibling of `assistant.rs`, and is reached through
  `POST /api/chat` with `mode: "desk"`, so the web keeps one chat call and the existing confirm modal.
  Friday's tools are `ask_<agent>`; none writes.
- **The journal is the audit log.** New `AuditKind`s (`DeskFinding`, `SentinelVerdict`, `PilotPreview`)
  are written for each step and Ledger reads them back with `Audit::recent`. No new table: the audit log is
  already append-only and has a page.
- **Sentinel approvals** are kept in memory (`ApprovalStore`): one-time ids, valid 120 seconds, bound to
  symbol, side, volume, stop and target. Pilot consumes one and calls `Trading::propose`; the 15-second
  token, the gate limits and the kill switch then apply as for any trade.
- **Sizing** reuses `sizing::size_by_risk`; Sentinel adds the Desk rules on top.
- **Finnhub data** (insider transactions, basic financials, filings) is a new `FinnhubData` client in
  `jarvis-news`, sharing its HTTP plumbing, with each call recorded in the egress log.
- **New York time** is computed from the US daylight-saving rule (second Sunday of March, first Sunday of
  November) with a NYSE holiday table for 2026 and 2027, which config can extend.

## Tasks

### Phase 1: the desk crate

- [x] **T1 Crate, types, session clock, calendar** (M)
  - Acceptance: `crates/desk` in the workspace; `AgentId` (nine ids), `Finding`, `Source`, `SeatState`;
    `session::status(utc) -> SessionStatus {phase, next_change, countdown}` for pre-market, open,
    after-hours and closed, with DST and holidays; `calendar::opex(year, month)`, `calendar::month_marks`
    (FOMC days taken from calendar events, opex days).
  - Verify: unit tests across both DST changes, a holiday, a weekend, half-hour boundaries.
- [x] **T2 Intelligence and analysis agents** (M)
  - Acceptance: `scout::movers`, `oracle::forecast` (empirical return distribution, P(above/below level in N
    bars), expected range), `athena::structure` (supply and demand zones, liquidity levels from swings) and
    `athena::fundamentals`, `capitol::insiders` (net shares and value, notable trades, 90 days), `atlas::macro`
    (next FOMC, high-impact US events, index moves, headline tone), `chartist::chart` (bars plus levels to
    draw). Each returns a `Finding` whose missing values are `null`.
  - Verify: fixture tests per agent, including empty input.
- [x] **T3 Sentinel and Pilot rules** (S)
  - Acceptance: `sentinel::review(plan, context) -> Verdict` (approve with reasons, or veto with reasons):
    stop required and on the right side, risk at or under the cap, open Desk positions under the cap, not in
    the last 10 minutes before the close or when the market is closed for a US stock; `ApprovalStore` issue,
    take-once, expiry.
  - Verify: one test per rule; a taken approval cannot be taken again; an expired one fails.

### Checkpoint: crate
- [x] `cargo test -p jarvis-desk`, clippy and fmt clean.

### Phase 2: the API

- [x] **T4 Finnhub data client** (S): `FinnhubData::{insider_transactions, basic_financials, filings}` with
  wiremock tests, key redaction and the loopback-only `http` rule.
- [x] **T5 Desk config, state and loop** (L)
  - Acceptance: `[desk]` config (enabled, provider, effort, spend file and cap, universe, risk settings);
    `Desk` in `AppState`; `mode: "desk"` in `/api/chat` returning the usual body plus `desk` (findings,
    seats, plans, chart); data gathering over the bridge, `MarketService` and `FinnhubData`; spend
    reserve and settle; audit kinds; a `DeskActivity` WebSocket event.
  - Verify: agent-loop tests with `ScriptedLlm` and the mock bridge.
- [x] **T6 Desk routes and safety tests** (M)
  - Acceptance: `GET /api/desk/state` (clock, calendar, pulse, seats, journal, spend, brain),
    `POST /api/desk/plans/{id}/pilot` (UI source). Safety tests: no approval, wrong or expired approval,
    vetoed plan, kill switch, gate limits, no bridge write without confirm, the cap stops Claude calls.

### Checkpoint: API
- [x] `cargo test --workspace`, clippy and fmt clean; a size-and-buy flow passes against the mock bridge.

### Phase 3: the web

- [x] **T7 Desk types, API client, state and mock** (M): `#lib/desk/*`, mock data for `dev:mock`.
- [x] **T8 HUD components** (L): agent ring around the orb, market pulse, task queue, signal log, session
  clock, calendar, agent load, SVG chart, plan cards, journal; "—" for missing values.
- [x] **T9 Mode switch and route** (S): `Trading | Assistant | Desk`, the Desk page, voice and confirm reused;
  Vitest for helpers and a Playwright smoke test.

### Checkpoint: web
- [x] `pnpm -C web check` and `test` pass; the HUD checked in the browser in mock mode (desktop and phone).
- [ ] `pnpm -C web test:e2e`: `e2e/desk.spec.ts` is written but not run here (Playwright's Chromium for 1.65 is
  not installed: `pnpm -C web exec playwright install chromium`).

### Phase 4: docs and finish

- [x] **T10 Docs**: ADR-0023, `docs/desk-mode.md`, `docs/api.md`, `docs/configuration.md`,
  `config/jarvis.example.toml`, README, architecture, CHANGELOG.
- [x] **T11 Review**: code review pass, then commit on `nimbus-desk`.

## Changes after code review

- Pilot re-checks the Desk rules (positions, session, kill switch) before asking for a preview.
- `Trading::propose` uses `cross_check_sized`, so a risk the trader states is compared with the plan's risk.
- Desk tickets are rebuilt from the audit log at start and after missed events.
- US-session symbols include any stock the instrument catalog does not list as FX, metal, index, energy or crypto.
- FOMC detection skips minutes, speeches and projections; a long turn is never dropped from history.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Broker symbol names differ from Finnhub tickers (`NVDA.US` vs `NVDA`) | Med | Finnhub gets the canonical name without broker decoration; missing data shows "—" |
| Friday quotes a figure no agent produced | High | Reuse the trading grounding check on Desk replies, with one retry |
| Pilot used without an explicit trade request | High | Pilot from Friday uses the LLM source and the intent cross-check; the button uses the UI source |
| Claude cost overruns | Med | Reserve-and-settle spend meter with its own monthly cap |

# ADR-0023: Desk mode, a nine-agent trading desk with one gated path for money

- Status: Accepted
- Date: 2026-11-24

## Context

The user asked for the "Nimbus Desk" build guide (cloud9markets.com) as a separate project based on Arc MT. The
guide describes a voice-driven trading desk: a master agent talks to the trader and routes each command to
eight specialists, shown on a dark HUD with an orb, nine seats, a market pulse, a session clock and a calendar.
Its one hard rule is that money moves through one gated path: analysis cannot execute, and execution cannot
approve itself. The guide publishes no protocol, risk rules or data model.

The user chose (spec-nimbus-desk, 2026-11-24):
- new crates in this workspace and a new route set in the existing web app;
- the Pilot agent trades through the Arc MT MT5 bridge;
- Claude by default for the master agent;
- the full HUD and all nine agents at MVP depth, with TradingView, TimesFM, Electron and hand tracking left out;
- a separate $20 monthly Desk cap, Friday only, and Sentinel at 0.5% risk, a stop required, two open Desk
  positions and no new trades in the last 10 minutes before the US close.

## Decision

- **A third mode, Desk**, next to Trading and Assistant. `POST /api/chat {"mode": "desk"}` runs its own agent loop
  (`crates/api/src/desk.rs`). Friday's only tools are the specialists, `ask_<agent>`.
- **The specialists' logic is pure** (`crates/desk`, `jarvis-desk`): functions over bars, calendar events, insider
  rows and fundamentals, with no I/O. The API gathers the data (bridge, `MarketService`, a new Finnhub data
  client in `jarvis-news`) and calls them. "Each agent is its own service" is met as one module per agent behind
  one shape (`Finding`), not as separate processes: one trader on one PC gains nothing from nine processes.
- **One gated path for money**, enforced in code:
  - Sentinel sizes the plan with the existing `sizing::size_by_risk` and checks the Desk rules on top of the gate.
    It returns an approval id, one-time, bound to the plan, valid two minutes. A veto is final.
  - Pilot needs that id. It calls `Trading::propose`, the same function the trading tools and the Prepare-trade
    button use, so the gate limits, the kill switch and the 15-second confirm token apply unchanged. Before
    that, Pilot checks the Desk rules again (positions opened since, the session, the kill switch): a plan that
    would now be vetoed is not previewed. A preview larger than the approved volume is cancelled.
  - Pilot from Friday uses the LLM source, so the existing intent cross-check applies: the trader must have asked
    to buy or sell in their own words. `propose` now calls `cross_check_sized`, so a risk the trader stated must
    match the plan's risk (Trading mode always passes no risk, so its check is unchanged). The plan card's
    "Send to Pilot" button uses the UI source.
  - No Desk tool writes. Only `POST /api/confirm/{token}` does, as before.
- **Grounding**: Friday's reply goes through the trading grounding check. After one retry, an ungrounded reply is
  replaced by the specialists' own summaries.
- **Missing data is `null`** in every finding and "—" in the HUD; each finding lists its sources.
- **The journal is the audit log**: every Desk row is written with persona `desk`, plus new kinds `desk_finding`,
  `sentinel_verdict` and `pilot_preview`. Ledger reads them back.
- **Cost**: a Desk `SpendMeter` with its own cap and spend file, the reserve-and-settle pattern of ADR-0022. Only an
  Anthropic provider is metered; Desk can run on Ollama at no cost.
- **Open Desk positions** are the tickets of executed Pilot previews, followed from `trade_executed` events and
  rebuilt from the audit log (`pilot_preview` and `trade_result` rows) at start and after missed events.
- **US-session symbols**: those in `[desk] universe`, and, when the instrument catalog is loaded, any symbol that
  is not one of its FX, metal, index, energy or crypto instruments (a stock).
- **New York time** comes from the US daylight-saving rule in code, with NYSE holidays for 2026 and 2027 that
  config can extend. No time zone dependency is added.

## Consequences

- SPEC core rules 1 to 10 hold in Desk mode, with tests: `crates/api/tests/desk.rs` covers the specialist-only tool
  list, approval reuse, expiry, vetoes, the kill switch, a gate limit Sentinel does not know, the intent
  cross-check, the spend cap and grounding.
- Two new endpoints, `GET /api/desk/state`, `GET /api/desk/pulse`, and `POST /api/desk/plans/{id}/pilot`, plus a
  `desk_activity` WebSocket event.
- Oracle's probabilities describe past returns, not a forecast model; the HUD and Friday say so.
- Capitol's congressional trades need Finnhub's premium plan; on a free key they show "—".
- Desk positions are found through the audit log rather than a comment or magic number on Desk orders, which
  would change the bridge write path. A trade closed and reopened by hand is not a Desk position.

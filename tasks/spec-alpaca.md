# Spec: Alpaca as a second venue (US stocks, paper only), in Trading and Desk modes

Status: **approved 2026-11-25 with the defaults for the open questions** (100 shares and $10,000 per order, 5 open
positions, $500 daily loss; `[alpaca] symbols` plus Desk's universe; regular session only; built in slices).
Branch: `alpaca` (from `nimbus-desk`), its own PR. Plan: [plan-alpaca.md](plan-alpaca.md).

Source: the user's request ("connect Alpaca to this project, use the settings from D:\Projects\ReyCompass") and
their choices on 2026-11-25:
- **Scope:** market data **and** trading, in **both** Trading and Desk modes.
- **Keys:** copied from ReyCompass's `.env` into Arc MT's `.env` as `ALPACA_TRADING_URL`, `ALPACA_KEY_ID` and
  `ALPACA_SECRET_KEY`. The URL is Alpaca's paper host.
- **Branch:** a new PR on top of Desk mode.

ReyCompass's adapter (`src/reycompass/venues/alpaca.py`, endpoints checked against docs.alpaca.markets on
2026-11-09) is the reference for endpoints and safety: paper by default, the base URL *is* the environment
(Alpaca's responses carry no paper/live flag), keys in the `APCA-API-KEY-ID` / `APCA-API-SECRET-KEY` headers
over https only, and `client_order_id` for idempotent submits.

## Objective

The trader can quote, chart, size, preview, confirm, see and close **US stock** trades on an **Alpaca paper**
account, from Trading mode (Jarvis or Friday) or from Desk mode (Sentinel, then Pilot). MT5 keeps working as
today. Every Arc MT safety rule holds for Alpaca exactly as for MT5.

## Design

- **A venue is where a symbol trades:** `mt5` (default) or `alpaca`. A symbol goes to Alpaca when it is in
  `[alpaca] symbols`, or when the trader names the venue ("buy 10 NVDA on Alpaca"). Everything else stays on
  MT5. One symbol has exactly one venue at a time, so "NVDA" is never ambiguous.
- **One broker interface, two implementations.** Trading's reads (ping, account, positions, orders, price,
  symbol specs, today's history) and its single write path (`WriteSink::send_write`) become a `Venue` trait.
  `BridgeClient` implements it as today; a new `AlpacaClient` (`crates/alpaca`, `jarvis-alpaca`) implements
  it by mapping Alpaca's REST shapes onto the same protocol types. The gate, previews, tokens, the kill
  switch, the intent cross-check, batch double-confirm and the audit log are reused, not copied.
- **Mapping to the shared types** (decided here; the gate's maths then works unchanged):
  - volume = shares; `volume_step` 1 (whole shares), `volume_min` 1; contract size 1; tick size $0.01, tick
    value $0.01 per share, so P/L and risk sizing come out in dollars.
  - tickets: Alpaca ids are UUIDs, so Arc MT gives each position and order a stable 64-bit ticket (a hash of
    the asset or order id) and keeps the map.
  - orders: market or limit, `time_in_force` day. A stop loss and/or take profit becomes Alpaca's `oto` or
    `bracket` order. `client_order_id` is the preview id plus the command index, so a write whose outcome is
    unknown is looked up, never re-sent (ADR-0013 for Alpaca).
  - closes: `DELETE /v2/positions/{symbol}` (full) or with `qty` (partial). Cancels: `DELETE /v2/orders/{id}`.
  - modify (move SL or TP) is **not** supported on Alpaca in this release: the gate refuses it with a clear
    message. Closing and re-opening works.
- **Limits per venue.** The existing `[limits]` apply to MT5 as today. Alpaca gets its own section:
  `max_shares` per order, `max_notional_usd` per order, `max_positions`, `max_daily_loss_usd` and its own
  allowed-symbol list (`[alpaca] symbols`). Trading hours come from Alpaca's market clock (`/v2/clock`), so
  no Alpaca order is previewed while the US market is closed (unless `[alpaca] extended_hours`, off by
  default). The kill switch blocks both venues.
- **Paper only, enforced three times:** Alpaca is disabled at start-up (with the reason in `/api/status`)
  unless the trading URL host is `paper-api.alpaca.markets`; the client refuses every write to any other
  host; and `[alpaca] mode` accepts only `paper`. Live Alpaca is out of scope and needs its own ADR.
- **Data:** quotes and bars from `data.alpaca.markets` (`/v2/stocks/{symbol}/quotes/latest`,
  `/v2/stocks/{symbol}/bars`) on the free `iex` feed. Desk's Scout, Athena, Oracle, Chartist and the pulse
  use them for Alpaca symbols; Atlas keeps the MT5 calendar.
- **UI:** the watchlist, positions, pending orders and the account strip show a venue tag; the account strip
  shows both accounts (equity, P/L) side by side. Previews say "on Alpaca (paper)". Close and cancel buttons
  work for Alpaca rows through the same preview and confirm.
- **Status and preflight:** `/api/status` gains `alpaca: {enabled, connected, paper, account_status,
  market_open, error}`; `jarvis-api preflight` adds an Alpaca check (account read only, no order). Every
  Alpaca request is recorded in the outbound-connection log with purpose `alpaca`.

## Acceptance criteria

1. With the keys set and the paper URL, `/api/status` reports Alpaca connected and paper; with a live URL,
   Alpaca is disabled with the reason and nothing is sent to it.
2. "What's NVDA at?" in Trading mode returns Alpaca's quote for an Alpaca symbol, labelled with its venue.
3. "Buy 5 NVDA with a stop at 118" previews a 5-share bracket order on Alpaca with estimated P/L at the stop,
   and only `POST /api/confirm/{token}` sends it; the 15-second token, kill switch, limits and cross-check
   apply. Desk mode does the same through Sentinel and Pilot, with sizing in whole shares.
4. Alpaca positions and orders appear in the HUD with their venue; close, partial close and cancel work
   through previews.
5. A timed-out Alpaca submit is resolved by `client_order_id` and never sent twice.
6. MT5 behaviour, tests and limits are unchanged.

## Testing

- `crates/alpaca`: a wiremock Alpaca (trading and data hosts) for every endpoint used, the shape mapping,
  ticket hashing, paper-host refusal, key redaction, and unknown-outcome lookup.
- `crates/api/tests/alpaca.rs`: the safety rules against the Alpaca mock: no write without confirm, kill
  switch, Alpaca limits, market-closed refusal, venue routing, a live URL refused at start-up and at write.
- Web: Vitest for venue labels; a Playwright smoke test in mock mode (Alpaca position row, close preview).
- No test calls the real Alpaca. An opt-in `#[ignore]` test reads the paper account only (no orders).

## Cost

Free: Alpaca paper trading and the IEX data feed cost nothing, and tests use mocks. No new paid service.

## Boundaries

- **Always:** paper only; every Alpaca write through preview and confirm; docs, ADR-0024 and the example
  config in the same PR.
- **Ask first:** live Alpaca; fractional shares; options or crypto on Alpaca; any new dependency beyond the
  HTTP client already in the tree (reqwest).
- **Never:** print or commit the keys; send an order in a test.

## Open questions (my proposed defaults in brackets)

1. **Alpaca limits:** [max 100 shares and $10,000 notional per order, 5 open positions, $500 daily loss.]
2. **Which symbols go to Alpaca:** [`[alpaca] symbols` lists them; Desk's `[desk] universe` is added to it by
   default, so Desk's US stocks trade on Alpaca.]
3. **Extended hours:** [off: Alpaca orders only in the regular session.]
4. **Size:** this is larger than Desk mode (a venue abstraction in the trading core plus a new crate and UI).
   [Build it in slices: Alpaca client and data first, then Trading-mode previews and confirm, then Desk and
   the UI, each slice tested and committed.]

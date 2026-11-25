# Plan: Alpaca as a second venue

Spec: [spec-alpaca.md](spec-alpaca.md), approved 2026-11-25 with the defaults for its open questions.

## Decisions made while planning

- **A second `Trading` instance for Alpaca**, not a rewrite of the first. `Trading` reads the broker through a
  small `BrokerReads` trait (ping, account, positions, orders, price, symbol, history) that `BridgeClient` and a
  new `AlpacaVenue` implement, and writes through `WriteSink`, which `AlpacaVenue` also implements. The Alpaca
  instance gets the `[alpaca]` limits and its own token store, and shares the kill switch (`KillSwitch` clones
  share state), the audit log and the event bus. So the gate, previews, tokens, batch confirm, cross-check and
  unknown-outcome handling are the same code for both venues.
- **Routing:** a symbol whose canonical name is in `[alpaca] symbols` (plus `[desk] universe` when
  `include_desk_universe`) goes to Alpaca. Confirm, cancel and read-back find the instance that holds the token.
  Close and cancel by ticket find the instance whose positions or orders hold the ticket.
- **MT5-only checks are skipped for Alpaca:** broker identity (symbol maps) and the MT5 account-switch check.
  `ModeMonitor` for Alpaca expects demo, and the Alpaca venue always reports demo (paper), so a live URL can never
  look like a demo account; the paper check happens before that.
- **Tickets:** FNV-1a 64-bit hash of the Alpaca position's asset id or order id, kept in a map for the reverse
  lookup.
- **Daily P/L:** Alpaca's `equity - last_equity` minus open P/L, as ReyCompass does.

## Slices

- [ ] **S1 `crates/alpaca` (`jarvis-alpaca`)**: config, paper-host rule, https rule (http only for loopback
  tests), client for account, clock, positions, orders, order by client id, submit, cancel, close position,
  asset, latest quote, bars; typed errors with `maybe_sent`; key redaction. Wiremock tests.
- [ ] **S2 Venue in the trading core**: `BrokerReads`, `AlpacaVenue` mapping to protocol types and commands,
  second `Trading`, routing for propose, confirm, cancel, read-back, close and cancel by ticket; kill switch on
  both. Safety tests against a wiremock Alpaca.
- [ ] **S3 Reads and Desk**: `[alpaca]` config, `/api/status` alpaca block, positions, orders, account and
  watchlist merged with a `venue` field, read tools for the model, Desk data agents and Pilot on Alpaca, egress
  logging, preflight check.
- [ ] **S4 Web**: venue tags, both accounts in the account strip, Alpaca rows close and cancel, mock data,
  Vitest and a Playwright smoke test.
- [ ] **S5 Docs**: ADR-0024, `docs/alpaca.md`, api, configuration, example config, README, CHANGELOG.

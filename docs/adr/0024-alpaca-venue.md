# ADR-0024: Alpaca as a second venue, paper only, behind the same gate

- Status: Accepted
- Date: 2026-11-25

## Context

The user asked to connect Alpaca to Arc MT with the settings of their ReyCompass project, for market data and
trading in both Trading and Desk modes (spec-alpaca, approved 2026-11-25). ReyCompass uses Alpaca for US stocks
on a paper account. Alpaca's responses carry no paper/live flag: the base URL is the environment.

Arc MT's safety model (SPEC core rules) was built around one broker: one gate, one preview and token flow, one
confirm path that writes to MT5. A second broker must not get a second, weaker path.

## Decision

- **A venue abstraction in the trading core.** `Trading` reads its broker through `BrokerReads` (ping,
  account, positions, orders, price, symbol, history) and writes through `WriteSink`. `BridgeClient`
  implements both as before. `AlpacaVenue` implements them for Alpaca by mapping its REST shapes onto the
  bridge protocol types:
  - volume = whole shares; contract size 1; tick size and tick value $0.01, so P/L and risk come out in dollars;
  - 64-bit tickets hashed from Alpaca's ids;
  - a stop and/or target becomes a bracket or one-triggers-other order;
  - today's realised P/L is the day change minus open P/L.
- **A second `Trading` instance for Alpaca.** It has the `[alpaca]` limits (shares, dollars per order, open
  positions, daily loss) and its own token store. It shares the kill switch, the audit log and the event bus.
  The gate, previews, tokens, batch confirm, intent cross-check, re-check at confirm and unknown-outcome
  handling are the same code for both venues.
- **Routing:** a `VenueRouter` sends:
  - a proposal by symbol (`[alpaca] symbols`, plus `[desk] universe` by default) or by ticket;
  - confirm, cancel and read-back by the token's owner.
  - "Close everything" stays on MT5; Alpaca positions are closed by symbol or ticket.
- **Paper only, enforced three times:**
  - Alpaca is off at start-up unless the trading URL is `https://paper-api.alpaca.markets`;
  - the client refuses every write to any other host;
  - `[alpaca] mode` accepts only `paper`.
  - In addition, the venue reports a demo account only for the paper host, and the Alpaca instance expects demo.
- **Not supported on Alpaca in this release:** moving a stop or target (refused at preview), stop entry
  orders, fractional shares, extended hours unless configured.
- **Unknown outcomes (ADR-0013):** each submit carries a fresh `client_order_id`. A failure after the request
  may have arrived is looked up by that id. If it is not found, the outcome is recorded as unknown and never
  re-sent.
- **The write-site test** (`tests/trading.rs`) still makes `trading.rs` the only caller of writes.
  `alpaca_venue.rs` is named as Alpaca's only write sink: it may translate the write commands it is handed,
  but it must not call a bridge write. The Alpaca client's write methods may not be called anywhere else, and
  `send_write` stays in `trading.rs`.

## Consequences

- SPEC core rules 1 to 10 hold for Alpaca with tests (`tests/alpaca.rs`):
  - no write without confirm;
  - Alpaca limits, market hours and the kill switch;
  - a live URL turns Alpaca off;
  - closing by ticket;
  - an unclear submit is never sent twice;
  - modify is refused.
- Two instances mean two token stores. Engaging the kill switch revokes the tokens of both.
- Open Desk positions are counted on both venues.
- Live Alpaca, fractional shares, options and crypto on Alpaca each need their own decision.

# Alpaca (US stocks, paper trading)

Arc MT can trade US stocks on an **Alpaca paper account** next to MetaTrader 5. Alpaca is a second *venue*: a
symbol listed in `[alpaca] symbols` trades on Alpaca, everything else stays on MT5. Every Alpaca trade goes
through the same preview, 15-second confirm, limits and kill switch as an MT5 trade.

Spec: [tasks/spec-alpaca.md](../tasks/spec-alpaca.md). Decision record: [ADR-0024](adr/0024-alpaca-venue.md).

> Paper only. Arc MT refuses any Alpaca URL other than `https://paper-api.alpaca.markets`. Not financial advice.

## Setup

1. Put the paper keys in `.env` (Alpaca dashboard, Paper Trading, API keys):

   ```
   ALPACA_TRADING_URL=https://paper-api.alpaca.markets
   ALPACA_KEY_ID=...
   ALPACA_SECRET_KEY=...
   ```

2. List the stocks to trade on Alpaca in `config/jarvis.toml`:

   ```toml
   [alpaca]
   symbols = ["NVDA", "AAPL", "MSFT"]
   ```

   Desk mode's `[desk] universe` is added to it by default (`include_desk_universe = true`).

3. Restart Arc MT. `GET /api/status` shows `alpaca.connected = true` and `paper = true`, and the account panel
   shows the Alpaca paper account. Without the keys Alpaca is off and says why.

## What works

- **Prices:** "what's NVDA at?" asks Alpaca, on the free IEX feed (`feed = "sip"` needs a subscription).
- **Charts:** Desk's Scout, Athena, Oracle, Chartist and the market pulse read Alpaca's bars for Alpaca symbols.
- **Orders:** "buy 5 NVDA with a stop at 118 and a target at 130" previews a whole-share market order with
  the stop and target as an Alpaca bracket order. A limit order needs a price.
- **Closing:** close and partial close from the position row or by voice. Cancel works on pending orders.
- **Desk mode:** Sentinel sizes a plan from Alpaca's quote and account (whole shares). Pilot sends it to Alpaca.

## Limits (`[alpaca]`)

| Key | Default | Effect |
|---|---|---|
| `max_shares` | `100` | Shares per order at most. |
| `max_notional_usd` | `10000` | Order value (shares x price) at most. |
| `max_positions` | `5` | Open Alpaca positions at most. |
| `max_daily_loss_usd` | `500` | No new orders once today's realised loss reaches it. |
| `extended_hours` | `false` | Off: no Alpaca order while the US market is closed. |

The kill switch blocks both venues, and engaging it revokes the open previews of both. `[limits]` stays MT5's.

## Not supported yet

- Moving a stop or target on an open Alpaca position: close it and open it again.
- Stop entry orders and fractional shares.
- Live accounts.
- "Close everything" closes MT5 positions only. Close Alpaca positions by symbol or ticket.

## Cost

Free. Alpaca paper trading and the IEX data feed cost nothing.

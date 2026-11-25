# Desk mode (Nimbus Desk)

Desk mode is a trading desk of nine agents. You talk to **Friday**, the master. She hands your question to
eight specialists, puts their findings together, and answers in a few sentences. The HUD shows which seats
worked, what they found, the market pulse, the US session and the calendar.

Switch to it with **Trading | Assistant | Desk** in the top bar. Spec: [tasks/spec-nimbus-desk.md](../tasks/spec-nimbus-desk.md).
Decision record: [ADR-0023](adr/0023-nimbus-desk.md).

> Not financial advice. Views and probabilities are information. Trade on a demo account until you have tested
> the whole flow. You are responsible for every trade you confirm.

## The seats

| Seat | Group | What it does | Data |
|---|---|---|---|
| Friday | Master | Routes your question, merges the answers, talks to you. Does no specialist work. | Claude by default |
| Atlas | Intelligence | Next FOMC decision, upcoming high-impact US events, index moves, headline tone | MT5 calendar and bars, news provider |
| Capitol | Intelligence | Insider buying and selling over 90 days; congressional trades | Finnhub (congress needs the premium plan) |
| Scout | Intelligence | Today's biggest movers across the scan list: change, range, relative volume | MT5 daily bars |
| Athena | Analysis | P/E, margins, 52-week range, recent filings; supply and demand zones and liquidity levels | Finnhub, MT5 H1 bars |
| Chartist | Analysis | Switches the desk chart and draws zones, a plan's entry, stop and target, and a forecast band | MT5 bars |
| Oracle | Analysis | Chance of ending above or below a level, and the middle-80% range, from past returns | MT5 bars |
| Sentinel | Execution | Sizes a trade from risk and stop, then approves or vetoes it | Account, positions, symbol specs |
| Pilot | Execution | Turns an approved plan into the normal trade preview | The Arc MT gate |
| Ledger | Execution | The desk journal and system health | Audit log |

Every number comes from a feed or from code. When data is missing, the HUD shows "—" and Friday says "no data".

## Trading from the desk

1. Ask for a trade with a stop, for example "size a long on NVDA with a stop at 118".
2. **Sentinel** sizes it at the live price (0.5% of equity at risk by default) and checks the Desk rules:
   - a stop is required, on the losing side of the entry;
   - a target, if you give one, must be on the winning side;
   - the risk must be at or under the Desk cap;
   - fewer than two Desk positions may be open;
   - no new trades in the last 10 minutes before the US close;
   - US stocks trade only while the US market is open (symbols in `[desk] universe`, and any symbol the instrument
     catalog does not list as FX, a metal, an index, energy or crypto);
   - nothing while the kill switch is engaged.
3. An approved plan appears as a card under **The book**, valid for two minutes. A veto is final; ask again with
   a different plan.
4. **Pilot** checks the rules again (positions opened since, the session, the kill switch) and prepares the preview, either when you said "buy" or "sell" in your own words, or when you press
   **Send to Pilot** on the card. The preview then goes through the usual gate limits, kill switch and 15-second
   confirmation. Nothing is sent to MT5 until you confirm, by button or by saying "confirm".

## Setup

- **Brain**: `[desk] provider` names an `[[llm.providers]]` entry. The default is `claude`, which needs
  `ANTHROPIC_API_KEY`. Set it to your local Ollama provider to run Desk mode for free.
- **Finnhub** (optional, free tier): set `FINNHUB_API_KEY` in `.env` for insiders, fundamentals and filings.
  Without it, Capitol shows as offline and Athena gives chart structure only.
- **Scan list**: Scout scans your watchlist plus `[desk] universe`. Add the broker's US stock symbols you trade.
- **Indices**: `[desk] indices` (default `US500`, `NAS100`) feed Atlas and the market pulse.

All settings: [configuration.md](configuration.md#desk).

## Cost

Desk mode on Claude Opus 5.5 is billed to your Anthropic key at $4 per million input tokens and $20 per million
output tokens. A desk question with two to four specialist calls is about $0.06 to $0.12 (an estimate). The Desk
has its own monthly cap (`[desk] monthly_budget_usd`, default $20): at the cap Friday says so and sends nothing
more that month. The HUD shows this month's total under **Agent load**. Finnhub's free tier costs nothing.

## Not included

TradingView control, TimesFM forecasting (Oracle uses past returns instead), an Electron shell (the Tauri app and
the PWA already exist), hand tracking and per-agent voices. Only Friday speaks.

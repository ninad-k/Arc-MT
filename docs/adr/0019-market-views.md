# ADR-0019: Market views computed in code, with the model only reading them back

- Status: Accepted
- Date: 2026-10-30

## Context

Traders asked Arc MT for a view: "what's your view on gold for today?", "any buy setup on EURUSD?". Until now the assistant could read prices, the account and history, and propose a trade you had asked for, but it could not say anything about direction. The system prompt told it not to give investment recommendations, and it refused.

A language model that is allowed to give opinions on its own is a poor source of them. It can state a level that it never read, do arithmetic wrong on 300 bars, and give a different answer to the same question a minute later. A confident wrong number in a trading tool is worse than no number. The grounding check (LLM-2) catches figures that no tool reported, but it cannot make the model's reasoning about bars correct.

The safety rules in [SPEC.md](../../SPEC.md) must stay as they are. Rule 1 says the model can only call read tools and `propose_*` tools. Rules 2 and 3 say a trade needs a preview and a confirmation. Rule 9 says an unclear request is asked about, not guessed (a volume is never assumed). A view must not become a way around any of them. Two facts make this easier. A view needs nothing that can write, so it is a read tool. And the trade that follows from a view can use the existing preview, gate and confirmation path unchanged.

Other facts shaped the design:

- The broker's own bars are the right data. They are the prices the trader gets, with the broker's spreads and server clock. The EA already runs inside the terminal, and MT5 has a built-in economic calendar. Adding two read commands (`get_rates`, `get_calendar`, EA 0.5.0, protocol version 2 unchanged) reaches both without a new service.
- News can add context, but headline sentiment is weak evidence, and every news provider needs a key and has a small free tier. It has to be optional.
- A tool that offers direction invites over-trust. It needs honesty built in: a disclaimer, limits on confidence, a way to stand aside, and a record of how its views turned out.

## Decision

- **The analysis is deterministic code.** A new crate, `jarvis-market`, takes bars on M5, M15, H1, H4 and D1, the live bid and ask, the calendar events and an optional news score, and returns a view. It has no I/O and no clock. The same input always gives the same output, which is tested with golden files. Indicators (EMA, RSI, MACD, ATR, Bollinger, VWAP, classic pivots, fractal swing levels) are pure functions checked against hand-computed values.
- **A simple, readable scoring model.** Seven factors (H4 trend 25, H1 trend 20, M15 momentum 15, M15 RSI 10, VWAP 10, daily pivot 10, news 10) give a score from -100 to +100. A bias needs 20 points. Confidence is the absolute score, capped at 50 when H4 and H1 disagree. Every point is listed on the view with its reason, and every weight is a field of a config struct. See [market-view.md](../market-view.md#how-the-score-works).
- **A view stands aside when it should.** A high-impact event for the instrument's currencies within 30 minutes, or a spread above 25% of the M15 ATR, removes the setup. The bias is still reported.
- **Setups are sized from volatility.** Entry zone, stop and two targets come from the H1 and M15 ATR, with the second target snapped to a swing level when one is close. A view is valid for 4 hours.
- **The model only narrates numbers from tools.** Three new read tools, `get_market_view`, `get_economic_calendar` and `get_news`, return the data. The prompt rule "do not give investment recommendations" is replaced by: for a view or a buy or sell opinion, call `get_market_view`, relay the bias, confidence, setup levels and main reasons in two or three sentences, mention upcoming high-impact events and the reason for standing aside, say it is a computed analysis and not a guarantee, never present it as certain, never add a level or reason the tool did not give, and never propose a trade from a view unless the user asks for that trade with a volume. Every figure in the reply goes through the existing grounding check. The `summary` line of a view is built by code, not by the model. Headline titles reach the model as one short line each, without links, and are marked as untrusted text.
- **The economic calendar comes from MT5.** `get_calendar` uses the terminal's built-in calendar. No external calendar service is added.
- **External news is optional and carries a small weight.** `jarvis-news` has two providers, Finnhub and Alpha Vantage, off by default (`provider = "none"`). A word list or the provider's own scores give the tone, a currency pair's tone points in the pair's direction, and the whole news factor is worth at most 10 points of 100. Results are cached for 10 minutes per instrument. Keys live in `.env`, are sent only to the provider, and never appear in logs, errors or logged URLs.
- **Views never propose by themselves.** A view creates no preview. **Prepare trade** (`POST /api/market/prepare`) turns an unexpired view with a setup into the normal proposal, and only when the user gives a volume and an order type. It goes through `Trading::propose` with all limits and ends in the usual preview and confirmation. Take profit 2 is not sent.
- **Views are journaled and scored.** Every view is stored (migration `0007_market_views.sql`) and each request is audited as `market_view`. A background task scores views from M5 bars: TP first, SL first (also when one bar touches both), expired, or no setup, plus whether the bias was right by at least 0.25 H1 ATR at the end of its validity. The stats are shown next to the views. See [Track record](../market-view.md#track-record).

## Consequences

- Views are reproducible, testable and explainable. A reviewer can recompute any figure, and a user can see why the bias is what it is.
- The model cannot invent a level, because the numbers are not its to produce, and the grounding check blocks a reply that repeats a number no tool gave.
- The trade path is unchanged. A view is a read, Prepare trade uses the existing gate, and nothing is sent without the usual confirmation. Core rules 1 to 10 hold without exception.
- The EA must be at 0.5.0 for views. An older EA answers `retcode -2` (unknown command), and the user has to reinstall it. The protocol version stays 2, so everything else keeps working.
- The scoring model is heuristic. The weights are chosen, not fitted, and indicator signals describe the past. A view can be confidently wrong. This is the main risk of the feature.
- **Over-trust.** A bias with a number invites users to treat it as a forecast. The mitigations are part of the decision: every view carries a disclaimer; confidence is capped when the higher timeframes disagree and says nothing about probability; the view stands aside near events and wide spreads; news weighs little; views are never pushed; a volume and a confirmation are always needed to act; the model is told to call a view computed and uncertain; and the track record shows the hit rate, including the break-even TP rate that the default sizing implies. The remaining risk is a user who ignores all of it. The documentation says plainly that the views are not advice and that demo comes first.
- **Outside calls.** News providers are the first outbound HTTPS calls in the data path other than a cloud LLM. Keys are secrets, so the rules for key handling are written into [SECURITY.md](../../SECURITY.md): a Finnhub key goes in a request header, an Alpha Vantage key must go in the query string, and its request URLs are never logged. Errors are redacted. A user without a provider sends nothing out.
- **Untrusted text.** Headlines are external text. They are shown as plain text, their links open with `noopener noreferrer`, and they reach the model as tool results. A headline that tries to instruct the model cannot trade, because the model has no write tool, the gate and the confirmation stand between a proposal and MT5, and a new order needs a side in the user's own latest message. It could still try to bend the wording of an answer. The grounding check and the short, number-driven replies limit that.
- **Scoring has limits.** The journal reads bars, not fills. It does not prove that an entry was reachable, it counts a bar that touches both levels as a loss, and early on it has few views. It is a guide to the model, not a backtest.
- **Storage.** The journal adds a table to `data/jarvis.db` with the full view JSON of every request. It grows with use and holds prices and headline titles. The database is already treated as private.
- **Maintenance.** Weights, thresholds and multipliers are product decisions. A change to them changes what users see, so it goes in the changelog and, if it changes the model's shape, in a new ADR.

## Alternatives considered

- **Let the model analyse raw bars (rejected).** It would need hundreds of bars per frame in the prompt, which costs tokens and slows a local model. The answer would not be reproducible, the arithmetic would be unreliable, and the grounding check could only reject numbers, not repair them. A deterministic core is cheaper and can be tested.
- **An external technical-analysis service (rejected).** It sends the user's interest in an instrument to a third party, uses prices from another feed than the broker's, adds a dependency and a failure mode, and hides the method. Using the broker's bars in the same process is simpler and more honest.
- **A paid data feed (rejected for now).** It would add cost, a licence to read, and a feed that disagrees with the broker's prices. The spec makes new paid services an "ask first" item. The free MT5 calendar and the optional free news tiers cover the need.
- **News as a main driver (rejected).** A word list reads headlines too roughly to weigh more than a minor factor, and the free tiers are too small for frequent refreshes. News stays optional at 10 points.
- **Pushing views as alerts or signals (rejected).** It would turn a tool that answers into one that nudges, and it moves the trader toward trades they did not choose. Views are given only on request.
- **Leaving views out (rejected).** Users would ask the model anyway and receive ungrounded opinions, or take their analysis elsewhere. A computed, labelled and scored view serves them better than a refusal, as long as it cannot trade by itself.

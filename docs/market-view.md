# Market view

A **market view** is an intraday read on one instrument: a bias (bullish, bearish or neutral), how strongly the numbers agree, and, when the conditions allow it, an entry zone with a stop loss and two take-profit levels. Arc MT computes it in code from your broker's own price bars, the economic calendar in MT5 and, if you set up a provider, news headlines. The language model only reads the result back to you.

This page covers what a view is and is not, how to ask for one, the data and the scoring model, the indicators, the optional news providers, "Prepare trade", the track record, the limits of all this, and troubleshooting. The HTTP contract is in [api.md](api.md#market-view) and the settings are in [configuration.md](configuration.md#market). The decision record is [ADR-0019](adr/0019-market-views.md).

> **A view is information, not advice.** It comes from indicator rules, not from a forecast, and no outcome is guaranteed. Read [Limits and caveats](#limits-and-caveats) before you rely on one. Use a demo account first.

## What a view is and is not

A view **is**:

- a bias with a confidence from 0 to 100, computed from a fixed set of rules;
- the reasons: each factor with its signal and points, the trend on three timeframes, and the key levels;
- a setup (entry zone, stop, two targets) when there is a bias and no reason to stand aside;
- the upcoming calendar events for the instrument's currencies (moderate and high importance by default), and the tone of recent headlines when a news provider is set up;
- valid for 4 hours, then expired, and saved so it can be scored later (see [Track record](#track-record)).

A view is **not**:

- a prediction. It says what the indicators show now. It does not know what happens next;
- a trade. A view never places an order, never creates a preview and never asks you to confirm anything by itself. To act on one, you use [Prepare trade](#prepare-trade) or ask for the trade yourself, and the usual limits and confirmation apply;
- a signal service. Arc MT does not push views. It gives one only when you ask;
- written by the language model. The model cannot change a number. The wording of the `summary` line is built by code from the numbers, and the model's reply is checked against the same numbers (see [How the assistant answers](#how-the-assistant-answers)).

Out of scope: automatic trading, alerts, a backtesting screen, and stocks.

## How to ask

Type or say the question in chat. Hinglish works the same way as in the rest of Arc MT, but the chat model reads it, so check the answer against the view card.

| You say | What Arc MT does |
|---|---|
| "what's your view on gold for today?" | Computes a view for `XAUUSD` and tells you the bias, confidence, levels and main reasons |
| "any buy setup on EURUSD?" | Computes a view for `EURUSD`. Says whether the bias is bullish and whether a buy setup exists. If it is bearish or neutral, or Arc MT stands aside, it says so. |
| "what news is out for the Nasdaq?" | Shows the headline tone and up to 5 recent headlines for `US100`. Says "not configured" when no provider is set up. |
| "what's on the calendar today?" | Lists the economic events of the next 24 hours, with currency and importance |
| "calendar for the next 3 days, USD only" | The same, for 72 hours and one currency |

The same views are in the web app. Open the **Market view** panel on the HUD, choose a symbol from the watchlist (or type one), and press **Analyze**. The card shows everything in this page: the bias pill, the confidence meter, a stand-aside banner when it applies, the setup, the trend table, the levels, events, news, the factors, the summary, the disclaimer and the expiry time. It works in mock mode too.

Spoken questions go through the same path as typed ones. The reply is read aloud like any other answer.

## The data it uses

| Data | Source | Notes |
|---|---|---|
| Price bars | Your broker, through the EA command `get_rates` | 300 bars each of M5, M15, H1 and H4, and 30 bars of D1. Oldest first. The last bar of each frame is still forming and is dropped. |
| Live price | `get_price` | Bid and ask, used for the entry zone, the spread check and the VWAP and pivot factors |
| Economic calendar | MT5's built-in calendar, through the EA command `get_calendar` | No outside service. The terminal must be connected to the trade server. |
| Headlines | Finnhub or Alpha Vantage, only if you set one up | See [News](#news). Off by default. |

Bars, bid and ask are the broker's own, so the levels in a view are prices you can trade on that account. A different broker has slightly different prices and a different server clock.

All times in the computation are on one clock: the broker's **server time**. Day boundaries (for the VWAP and the daily pivot) are server midnight. The API converts the server time to UTC for display, using the offset it learns from the EA's `ping`.

`get_rates` and `get_calendar` need **EA 0.5.0**. They are described in [protocol.md](protocol.md#get_rates).

### Closed bars only

Signals use **closed bars only**. A bar counts as closed when it has ended by the time of the request. The bar that is still forming is dropped, so an indicator does not change its mind in the middle of a candle. When the market is closed, the last bar has ended and is kept, so a view asked at the weekend describes the last session, with frozen prices. Bars with missing or inconsistent prices (for example a high below the low) are skipped.

### How much history it needs

A factor whose data is missing is left out, and the view lists the reason in its notes. The minimums, in closed bars:

| Factor or step | Needs |
|---|---|
| H4 trend, H1 trend | 50 bars of that frame (the EMA50 has to exist) |
| M15 momentum | 34 closed M15 bars (26 + 9 - 1) |
| M15 RSI | 15 closed M15 bars |
| VWAP | at least 1 closed M5 bar of the current server day |
| Daily pivot | one completed D1 bar (the previous day) |
| ATR (setup size, spread check) | 15 closed bars of that frame, H1 and M15 |

If **fewer than 3** of the technical factors can be computed (the trends, momentum, RSI, VWAP and pivot; news does not count), or there is no valid bid and ask, the view does not guess: the bias is neutral, the confidence is 0, no factors are listed and the summary says there is not enough price data.

## How the score works

Each factor gives a signal from -1 to +1. The signal times the factor's weight is its **points**. The **score** is the sum of the points, clamped to -100 to +100.

| Factor | Weight | Rule |
|---|---|---|
| H4 trend | 25 | Compare EMA20 and EMA50 of the H4 closes, and the last close against EMA50. Both up: +1. Both down: -1. Mixed: 0. |
| H1 trend | 20 | The same rule on H1 |
| M15 momentum | 15 | Sign of the MACD(12,26,9) histogram on M15, worth 0.5. If the histogram has also grown in that direction compared with 3 bars earlier, add 0.5 more. So the signal is 0, +/-0.5 or +/-1. |
| M15 RSI | 10 | RSI(14) on M15. Above 70: -1 (stretched, a fade). 55 to 70: +0.5. 45 to below 55: 0. 30 to below 45: -0.5. Below 30: +1 (stretched, a bounce). |
| VWAP | 10 | Mid price above today's VWAP: +1. Below: -1. Equal: 0. |
| Daily pivot | 10 | Mid price above the classic pivot of the previous day: +1. Below: -1. Equal: 0. |
| News | 10 | The news score (-1 to +1, already in the instrument's direction) when a provider is set up and has data. Otherwise the factor is left out. |

The weights add up to 100. Without news the largest possible score is 90. The mid price is (bid + ask) / 2. The RSI bands, the weights and the other numbers are the defaults of `ViewConfig` in `crates/market/src/config.rs`.

The RSI rule is deliberately contrarian at the extremes: a very high RSI scores against the trend, a very low one scores for a bounce. It follows the usual reading of "overbought" and "oversold", and it is the factor most likely to disagree with the trend factors.

### Bias

| Score | Bias |
|---|---|
| 20 or more | bullish |
| -20 or less | bearish |
| between | neutral |

### Confidence

Confidence is the absolute value of the score, rounded to a whole number, from 0 to 100. It is **capped at 50** when the H4 and H1 trend factors point in opposite directions (one up and one down; a "mixed" trend is not a disagreement). Higher timeframes that fight each other make the view less reliable, so the number cannot say otherwise.

Confidence is a measure of how many rules agree and how much they weigh. It is not a probability of winning. A confidence of 60 does not mean a 60 percent chance.

Worked example. H4 trend up (+25), H1 trend mixed (0), M15 histogram positive but not growing (+7.5), M15 RSI 62 (+5), mid above the VWAP (+10), mid below the daily pivot (-10), no news. The score is 37.5, which is bullish. Confidence is 38. The two strongest factors that support the bias are the H4 trend and the VWAP, and the summary mentions them.

### Stand aside

A view **stands aside** when either condition holds. It still reports the bias, the confidence and the reasons, but it offers **no setup**, and the card and the summary say why.

- **A high-impact event is close.** An event with importance `high` for one of the instrument's currencies is within `event_blackout_minutes` (default 30) before or after now. The currencies come from the instrument: both sides of an FX pair (EURUSD: EUR and USD), the quote currency for a metal (XAUUSD: USD), the home currency of an index (US100: USD, GER40: EUR, UK100: GBP, JP225: JPY), and USD for energy, crypto and anything else.
- **The spread is too wide.** The live spread (ask - bid) is more than 25% of the M15 ATR(14). The spread is a cost you pay at once, and when it is large compared with how far the price moves in a bar, a setup sized from ATR is not worth taking.

When both hold, both reasons are shown. When the spread check cannot run because there is no M15 ATR yet, the view says it was skipped.

### Setup

A setup is offered only when the bias is not neutral and the view is not standing aside. Let `a` be the ATR(14) on H1 and `m` the ATR(14) on M15. All prices are rounded to the symbol's digits.

| Part | Buy | Sell |
|---|---|---|
| Entry zone | from `ask - 0.5 * m` up to the ask | from the bid up to `bid + 0.5 * m` |
| Entry mid | the middle of the zone | the middle of the zone |
| Stop loss | entry mid minus `1.0 * a` | entry mid plus `1.0 * a` |
| Take profit 1 | entry mid plus `1.5 * a` | entry mid minus `1.5 * a` |
| Take profit 2 | the nearest H1 swing resistance above TP1 and within `3 * a` of the entry mid; if there is none, entry mid plus `2.5 * a` | the nearest H1 swing support below TP1 and within `3 * a` of the entry mid; if there is none, entry mid minus `2.5 * a` |

`risk_reward` is the distance to TP1 divided by the distance to the stop, to two decimals. With the default multipliers it is 1.5 every time. It comes from the sizing rule. It says nothing about how likely TP1 is.

The setup is dropped, with a note, when the rounded prices do not keep their order (for example the ATR is too small for the symbol's precision), when the ATR is zero, or when the ATR on H1 or M15 is not available yet.

### Validity

`valid_until` is the time of the view plus `horizon_minutes` (default 240, so 4 hours). After that the view is expired: the card says so, and it can no longer be used for Prepare trade. Ask again for a fresh one. A view is also out of date well before then if the price runs through the stop or past the targets, or if news arrives. Treat the age of a view as part of it.

### The summary line

`summary` is one sentence built by code from the bias, the confidence, the two strongest factors for the bias and the setup. It contains no text from the model. For example:

> Bullish intraday bias, confidence 62 of 100: H4 and H1 trends are up. Buy zone 3921.40 to 3925.22, stop 3911.00, target 3944.10.

When the view stands aside it names the reason and says no setup is offered.

## Indicators and conventions

The periods are part of the model and are not settings. Every indicator is a pure function with tests against hand-computed values (to within 1e-9).

| Indicator | Definition and conventions |
|---|---|
| **EMA** (20 and 50) | Smoothing `k = 2 / (period + 1)`. The first value is seeded with the simple average of the first `period` closes, so the first EMA appears at index `period - 1`. |
| **RSI** (14) | Wilder smoothing. The first average gain and loss are plain means of the first 14 changes, then `avg = (avg * 13 + x) / 14`. `RSI = 100 - 100 / (1 + avg gain / avg loss)`. If there are no losses, RSI is 100 when there are gains and 50 when the series is completely flat. |
| **MACD** (12, 26, 9) | MACD line is EMA12 minus EMA26 of the closes. The signal line is the EMA(9) of the MACD line, seeded with the average of its first 9 values. The histogram is the line minus the signal. |
| **ATR** (14) | True range is the largest of (high - low), the absolute value of (high - previous close) and the absolute value of (low - previous close), so a gap counts as range. Wilder smoothing: the first value is the mean of the first 14 true ranges, then `atr = (atr * 13 + tr) / 14`. |
| **VWAP** | The typical price `(high + low + close) / 3` of each closed M5 bar of the **current server day**, weighted by tick volume. If every volume is zero, the bars are weighted equally. Tick volume counts price changes, not traded size, so on some instruments the VWAP is only a rough guide. |
| **Classic pivots** | From the previous completed D1 bar: `P = (H + L + C) / 3`, `R1 = 2P - L`, `S1 = 2P - H`, `R2 = P + (H - L)`, `S2 = P - (H - L)`. |
| **Swing levels** | Fractals on H1 closes. A bar is a swing high when its high is strictly above the highs of the 3 bars before it and at least as high as the 3 bars after it. A flat top counts once, at its first bar. Swing lows mirror this. Levels closer together than 0.25 H1 ATR are merged into their average. Supports are the swing lows below the mid price and resistances the swing highs above it, nearest first. The view lists up to 3 of each. |

A swing point needs 3 bars after it to be confirmed, so the most recent turn is never a swing level yet.

The view also reports a table with one row each for H4, H1 and M15: the trend (up, down or mixed), EMA20, EMA50, RSI, the MACD histogram and the ATR. And the levels: the daily pivot with R1, R2, S1 and S2, the previous day's high and low, the VWAP, and the swing supports and resistances. A level is empty when its data is missing.

## News

News is **optional and off by default**. With no provider the view works exactly the same, the news factor is left out, and the news section says "not configured".

Headlines have a small weight: 10 points of 100 at most. A view should never hang on them.

### Providers

| Provider | `provider` value | What it sends | Key |
|---|---|---|---|
| [Finnhub](https://finnhub.io) | `finnhub` | The latest market news of a category. It cannot filter by instrument, so Arc MT filters afterwards. The categories are `crypto` for crypto, `forex` for FX pairs, `forex` and `general` for metals, and `general` for indices and energy. | In the `X-Finnhub-Token` request header |
| [Alpha Vantage](https://www.alphavantage.co) | `alphavantage` | The `NEWS_SENTIMENT` feed, with its own sentiment scores. An FX pair takes two requests (one per currency, for example `FOREX:EUR` and `FOREX:USD`), crypto takes one for the coin, and metals, energy and indices read one topic each. | In the query string of the request (Alpha Vantage does not accept it anywhere else) |

### Get a free key

1. Open the provider's site, [finnhub.io](https://finnhub.io) or [alphavantage.co](https://www.alphavantage.co), and register for a free API key. Both sites explain the steps and the terms of the free plan, which are theirs and can change. Check that the plan allows your use.
2. Put the key in the `.env` file next to `config/jarvis.toml`. Never put it in the TOML file or in the repository.

   ```text
   FINNHUB_API_KEY=your-key-here
   # or
   ALPHAVANTAGE_API_KEY=your-key-here
   ```

3. Choose the provider in `config/jarvis.toml`:

   ```toml
   [market.news]
   provider = "finnhub"             # none | finnhub | alphavantage
   api_key_env = "FINNHUB_API_KEY"  # the variable that holds the key; use "ALPHAVANTAGE_API_KEY" for Alpha Vantage
   cache_minutes = 10
   ```

4. Restart `jarvis-api`. Ask "what news is out for gold?". The answer says which provider it used.

`api_key_env` names the variable. The key itself is never in the config file. If you leave `api_key_env` out, Arc MT reads the provider's usual variable (`FINNHUB_API_KEY` or `ALPHAVANTAGE_API_KEY`). The value must be a variable name: a pasted key is rejected at startup. If the variable is missing or empty, Arc MT logs a warning at startup, runs as if no provider were set, and the news section says "not configured". The rest of the view is unaffected. A key that the provider rejects has the same effect.

### Free-tier limits and the cache

Free plans are small. At the time of writing, the free Finnhub plan allows about 60 calls a minute, and the free Alpha Vantage plan about 25 calls a day. The providers change these numbers, so check their sites. Alpha Vantage answers an exhausted quota with a normal HTTP 200 and a notice instead of a feed, and Arc MT treats that as "rate limited".

Arc MT is careful with calls:

- Results are cached **per instrument for `cache_minutes`** (default 10). A second question about the same instrument inside that time costs no call.
- Only one request per instrument is in flight at a time. Several callers asking for the same symbol wait and share one call.
- Each call has a 10 second time limit.
- A view waits at most about 4 seconds for news. If a provider fails (timeout, quota, outage) and a cached summary less than an hour old exists, that summary is used and a warning is logged. If there is none, the news section says "unavailable", the view lists a note, and the view is computed without the news factor.

On Alpha Vantage, an FX pair costs two calls per refresh, so a 25-call day allows only about a dozen refreshes of FX pairs. Use the cache and ask about news when you need it.

### How headlines are scored

1. **Relevance.** A headline counts when its title or summary contains one of the instrument's terms as a whole word, in any case. The terms are the canonical name, its aliases and keywords, and the names of its currencies (for example `gold` for XAUUSD, or `euro`, `ECB` and `eurozone` for EURUSD). A provider that already selected an article for the instrument (Alpha Vantage, by ticker) skips this step. Duplicates (the same URL, or the same title) are dropped.
2. **Window.** Only headlines from the last 24 hours count.
3. **Tone.** Each headline gets a tone from -1 to +1:
   - with Alpha Vantage, the provider's own score is used. For a pair it is the base currency's score minus the quote's, divided by 2. If a headline has no ticker score, its overall score is used, and if it has none at all, the word list below;
   - with Finnhub, and as the fallback, a **word list** gives the tone (see below).
4. **Direction.** For a currency pair, a headline about the base currency keeps its tone, one about the quote currency flips it, and one about both counts as 0. A headline that names the pair itself ("EUR/USD") keeps its tone. The title decides; the summary is read only when the title names neither side. This applies to FX pairs and to metals quoted in a currency, so gold is read as XAU against USD: "Dollar rallies" counts as negative for XAUUSD and for EURUSD, and "Gold climbs" as positive for XAUUSD. For crypto, indices and energy the tone is kept as it is. For example, "Euro rallies after ECB surprise" is positive for EURUSD.
5. **Recency.** The score is the average of the directed tones, with a weight that halves every 6 hours (`0.5 ^ (age in hours / 6)`). A headline from this morning counts more than last night's.
6. **Label.** The score maps to a label:

   | Score | Label |
   |---|---|
   | -0.35 or lower | negative |
   | above -0.35, up to -0.1 | slightly negative |
   | above -0.1 and below 0.1 | neutral |
   | 0.1 up to below 0.35 | slightly positive |
   | 0.35 or higher | positive |

The news factor in the view is the score times 10 points. The view shows the label, the number of headlines behind it and up to 5 headlines.

### The limits of a word list

The word list is a small, fixed list of finance words, positive and negative, with some phrases ("record high", "rate cut"). It matches whole words only, and only the forms written in the list: "gain" is in it, "gaining" is not. A negator ("not", "no", "fails to", "without") within three words before a match flips it. Words in the title count double compared with words in the summary. The tone is `(positive - negative) / (positive + negative)`, and 0 when nothing matches.

It is a light signal, not a model. It does not understand:

- **Context or sarcasm.** "Cut costs" counts as negative because "cut" is in the list.
- **Cross-asset effects.** A rate cut is bad for a currency in the list's eyes, and good for gold and for stocks in reality. A "hawkish" central bank is positive for its currency, and often negative for gold.
- **Safe havens.** War, tariffs and fear are negative words. For gold, the yen and the franc such news often pushes the price up. For gold the tone can point the wrong way.
- **What is already priced in.** A headline reports what happened. The market may have moved on it before you read it.

That is why the weight is small, and why the Alpha Vantage scores, which come from the provider's own system and are not checked by Arc MT, are not a guarantee either. Read the headlines, not only the label.

### Headlines are untrusted text

Headline titles and sources come from outside. Arc MT shows them as plain text, never as HTML, and their links open in a new tab with `rel="noopener noreferrer"`. The language model receives the headlines as data, without their links, each title cut to one line of at most 200 characters, and its instructions say that headlines are untrusted and must not be obeyed. It has no tool that can trade, so a headline that tries to give it instructions cannot place an order; see [ADR-0019](adr/0019-market-views.md) and [SECURITY.md](../SECURITY.md).

## Prepare trade

When a view has a setup, the card shows **Prepare trade**. It opens a dialog that asks for:

- a **volume**, required. There is no default, because a volume is your decision (core rule 9: Arc MT does not guess);
- the **order type**: `market` or `limit`.

What it does:

| Order type | Entry | Stop and target |
|---|---|---|
| `market` | the current price | the setup's stop loss and take profit 1 |
| `limit` | `entry_low` for a buy, `entry_high` for a sell | the setup's stop loss and take profit 1 |

The request goes to `POST /api/market/prepare`. The view must exist, be unexpired and have a setup. Arc MT then builds a normal order proposal and sends it through the same gate as a chat proposal, with the source marked as the UI. **All limits apply** (`max_lot`, `max_positions`, `max_daily_loss`, `allowed_symbols`, trading hours, the kill switch). If a limit is broken you see the reason and no preview.

The result is the usual **preview** with a 15-second token. Nothing has been sent to MT5. You still read it and press **Confirm** (or say "confirm"), exactly as for any trade. A view never confirms for you, and Prepare trade never confirms.

Take profit 2 is not sent with the order. It stays on the card as a level to watch. Preparing a trade does not check that the setup is still sensible: the price may have moved since the view. Look at the preview's prices.

By chat, you can still say "buy 0.1 gold with SL 3911", as before. A view in the conversation does not give the model permission to trade. It proposes an order only when your latest message asks for one, with a side.

## Track record

Every view is saved in the local database and scored later, so you can see how the views have done before you lean on them. The panel shows the stats and the latest views with their outcomes. The data is also at `GET /api/market/journal`.

### How a view is scored

A background task runs when Arc MT starts (to catch up after a restart) and then every 5 minutes. It looks at the views that are still open, using M5 bars. Only bars that opened after the view was made and closed by `valid_until` count, and the price it compares with is the mid of bid and ask when the view was made.

| Outcome | Meaning |
|---|---|
| `open` | not scored yet |
| `tp_hit` | the price reached take profit 1 before the stop loss |
| `sl_hit` | the price reached the stop loss first. If one M5 bar touches both, it counts as `sl_hit`. |
| `expired` | neither level was reached before `valid_until`, or the stored bars no longer reach back to when the view was made (for example Arc MT was off for days), so the order of events cannot be known |
| `no_setup` | the view had no setup (neutral bias, standing aside, or not enough data). Scored at `valid_until`. |

A second result is recorded for **every** view, setup or not: **bias correct**. It is judged once, after `valid_until`: it is true when the last closed M5 bar of the view's life closed at least 0.25 H1 ATR away from the price at creation, in the bias direction. For a neutral view it is true when the price moved less than that either way. If the bars cannot be read back to the creation of the view, the bias is left unscored.

The stats show the number of views per outcome, the number of setups, the number of right and wrong biases, the **TP rate** and the **bias accuracy**. The TP rate is `tp_hit` divided by the setups that have closed (`tp_hit`, `sl_hit` and `expired`), so an expired setup counts as a miss. The bias accuracy is the share of scored views whose bias was right. Both are empty until something has been scored.

### How to read it

- **Wait for a sample.** A handful of views says almost nothing. Ten good views in a row can be luck.
- **Mind the break-even rate.** With the default sizing (stop 1.0, target 1.5), a TP rate of 40% only breaks even before spread, commission, swap and slippage (`1 / (1 + 1.5)`). After costs you need more. A TP rate below that loses money even if it looks like many wins.
- **TP rate is not profit.** The score is mechanical. It reads bars, not fills. It does **not** check that the price reached the entry zone: it scores from the price at the moment of the view, so a view whose entry was never touched can still show as `tp_hit` or `sl_hit`.
- **Bias accuracy has a baseline.** Neutral views count as correct when the price barely moved, which happens often. Look at bullish and bearish views separately in the list.
- **Per instrument and per time of day matter.** A strategy of rules that works on gold in a trend may not on a range-bound pair. Filter the list by symbol (`?symbol=`).
- **It is a record of this account's server, bars and spreads, on views you asked for.** It is not a backtest. Views you did not ask for are not in it.

## Limits and caveats

- **Indicator signals are not predictions.** Trend, momentum and RSI describe the past. Markets can reverse at any bar. A bullish view is no promise that the price rises.
- **The weights are chosen, not fitted.** The model is a transparent set of rules with hand-set weights and thresholds. They have not been optimised on data, and they are not guaranteed to suit your instrument or style. The track record is how you check them.
- **News carries a small weight on purpose.** A word list reads headlines roughly. Gold, the yen and the franc often react the opposite way to the word list.
- **The calendar is only as good as MT5's.** An event that is missing there is missing here. An empty calendar does not mean a quiet day.
- **Data gaps change the result.** A short history, a young symbol or a "max bars in chart" setting that is too low leaves factors out. The view lists what it skipped. With fewer factors the score is less meaningful.
- **A view is a snapshot.** It does not update as the price moves. After 4 hours it expires, and often sooner it is stale.
- **Costs are not in the setup.** Spread, commission, swap and slippage come on top. The spread check only stops the worst cases.
- **Market closed.** At weekends and on holidays you get a view of the last session at the last prices.
- **No guarantee.** Arc MT is provided as is. You are responsible for any trade you confirm.
- **Demo first.** Run the views on a demo account for a few weeks and read the track record before you decide whether they add anything to your own analysis.

### How the assistant answers

When you ask for a view or a buy or sell opinion, the model must call `get_market_view` and relay the bias, the confidence, the levels and the main reasons in two or three sentences. It mentions high-impact events that are coming up, and says why when the view stands aside. It must say that this is a computed analysis and not a guarantee, and it must not present the result as certain. It must not add levels, reasons or numbers that the tool did not give, and if the tool fails it says so instead of giving an opinion from memory. It does not propose a trade from a view unless you ask for that trade, with a volume.

Every figure in the reply goes through the same **grounding check** as other replies: a number that does not appear in a tool result or in your own message is not shown. The model cannot invent a level. See [api.md](api.md#previews-and-confirmation-sprint-2).

## Settings

The keys are in [configuration.md](configuration.md#market). In short:

```toml
[market]
enabled = true
horizon_minutes = 240          # how long a view stays valid
event_blackout_minutes = 30    # stand aside this close to a high-impact event
calendar_min_importance = "moderate"
rate_limit_per_minute = 30     # view and news requests per minute and session

[market.news]
provider = "none"              # none | finnhub | alphavantage
api_key_env = "FINNHUB_API_KEY"
cache_minutes = 10
```

Two more keys are in [configuration.md](configuration.md#market): `rate_limit_per_minute` (default 30, the number of view and news requests per session and minute) and an optional `[market.view]` table. The weights, thresholds and multipliers in this page are the defaults of `ViewConfig` in `crates/market/src/config.rs`, and every one of them can be changed in `[market.view]`. Leave it alone unless you know what you are changing: a changed model no longer matches this page, and the track record then describes a different model. `horizon_minutes` and `event_blackout_minutes` in `[market]` win over the same keys in `[market.view]`.

## Troubleshooting

| What you see | Likely cause | What to do |
|---|---|---|
| `ea_outdated`: "The MetaTrader 5 expert advisor is too old for market views" (the view or the calendar fails) | The EA in MT5 is older than 0.5.0. It answers `get_rates` and `get_calendar` with "unknown command" (`retcode -2`). Only 0.5.0 and newer answer them. | Update the EA: run `scripts/install-ea.ps1` and `scripts/compile-ea.ps1` (see the [README](../README.md#2-install-and-compile-the-ea)), then remove and re-attach JarvisBridge to a chart. The Experts tab must show `JarvisBridge 0.5.0`. |
| The view says price history is not loaded, or `bridge_error` with a 44xx or 4806 code | MT5 has not downloaded the bars for that symbol and timeframe yet. The first request for a symbol often finds none. Arc MT waits 500 ms and asks once more. | Ask again after a few seconds. If it persists, open a chart of that symbol on M5, M15, H1, H4 and D1 in MT5 and scroll back, so the terminal downloads the history. |
| "unknown symbol" | The broker has no symbol under that name. | See [symbol-mapping.md](symbol-mapping.md#troubleshooting). |
| The view lists notes such as "needs 50 closed H4 bars, have 30", or says there is not enough price data | The terminal has less history than the view needs. | Raise "Max bars in chart" in MT5 (Tools, Options, Charts), open the chart and let it download. Ask again. |
| The calendar is empty | MT5's calendar has no events for that range, or the terminal is not connected to its trade server, or the broker's server has no calendar feed. An empty calendar can also be real (a quiet window, a weekend). | Open the Calendar tab in the MT5 Toolbox (Ctrl+T) and compare. If MT5 shows events and Arc MT does not, check the bridge. Until it works, the stand-aside rule cannot see events. |
| Events are an hour or more off | The broker's server clock is not UTC. The API converts from server time to UTC with the offset it learns from the EA. | Compare with the Calendar tab in MT5, which shows server time. Check that the bridge is connected, because the offset is learned from it. |
| News says "not configured" | `provider = "none"` (the default), or the key variable named by `api_key_env` is empty or missing in `.env`, or the provider rejected the key. | Set the provider and the key (see [Get a free key](#get-a-free-key)) and restart. |
| News says "unavailable" (and the view has a note "News was unavailable"), or shows an old summary | The provider is rate limited, took longer than about 4 seconds, or is down. Free plans are small. | Wait. Raise `cache_minutes`. For Alpha Vantage, ask about FX pairs less often. |
| `429 rate_limited` on a view or news request | More than `[market] rate_limit_per_minute` (30) requests in a minute for this session. | Wait for `Retry-After`, or raise the limit. |
| "market views are turned off" (404), and the model has no market tools | `[market] enabled = false`. | Set `enabled = true` and restart. |
| Prepare trade says there is no such view | The view could not be saved in the journal (the error is in the log), or the id is wrong. | Ask for a new view. Check the log if it repeats. |
| The view stands aside and shows no setup | A high-impact event is within 30 minutes, or the spread is wider than 25% of the M15 ATR. | Read the stand-aside reason on the card. Wait for the event to pass or the spread to narrow. Ask again. |
| Bullish or bearish bias but no setup, without a stand-aside banner | The setup could not be sized: no ATR yet, or the rounded levels were not in order. The notes say which. | See the view's notes. Check [How much history it needs](#how-much-history-it-needs). |
| Prepare trade says the view expired or has no setup | The view is older than 4 hours, or it was neutral or standing aside. | Ask for a new view. |
| Prepare trade is refused with a limit message | A trading limit or the kill switch. | The message names the rule. Limits are in [configuration.md](configuration.md#limits). |
| The assistant answers "I couldn't verify that from MetaTrader just now" | The reply contained a figure that is not in any tool result. The grounding check blocked it. | Ask again. The view card in the web app shows the real numbers. |

For logs and general problems, see the [runbook](runbook.md#troubleshooting).

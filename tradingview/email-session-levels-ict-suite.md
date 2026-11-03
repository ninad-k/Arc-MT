# Email draft: Session Levels + ICT Suite

> Before sending, read the note at the bottom about the licence. Attach
> `session-levels-ict-suite.pine` to the email.

---

**Subject:** Session Levels + ICT Suite for TradingView — one indicator for your day trading

Hi all,

I've put together a single TradingView indicator that gives you everything most of us
add to a chart by hand each morning. It's called **Session Levels + ICT Suite**, and it
draws:

- **Session levels** for Asia, London and New York: the session Open, High and Low, plus the
  volume-profile **VAH, POC and VAL** (the 70% value area and its point of control). Each session
  has its own colour, a dot at the open, dotted lines at session start and end, and labels at the
  end of the session so nothing piles up. Today's running session updates live.
- **ICT Concepts** (by LuxAlgo): market structure (MSS / BOS), fair value gaps, order blocks,
  buyside / sellside liquidity, volume imbalances, NWOG / NDOG and killzones.
- **Supply and Demand Visible Range** (by LuxAlgo): supply and demand zones built from the volume
  in the part of the chart you're looking at.
- **Liquidity Swings** (by LuxAlgo): swing highs and lows with the volume that traded at them.

## How to install it (about two minutes)

1. Open TradingView (desktop app or website) and open any chart.
2. At the bottom of the screen click **Pine Editor**.
3. In the editor, click the script name at the top-left, then **Create new → Indicator**.
4. Select everything in the editor (Ctrl+A) and delete it.
5. Open the attached file `session-levels-ict-suite.pine` in Notepad, select all, copy, and paste it
   into the editor.
6. Click **Save** (Ctrl+S) and name it `Session Levels + ICT Suite`.
7. Click **Add to chart**.

From now on you'll find it under **Indicators → My scripts**, so you can add it to any chart with
one click. If you already have ICT Concepts, Supply and Demand Visible Range, Liquidity Swings or a
separate session-levels indicator on the chart, remove them, otherwise everything draws twice.

## Reading the chart

| Colour | Session | Hours (local exchange time) |
|---|---|---|
| Orange | Asia | 09:00–18:00 Tokyo |
| Blue | London | 08:00–17:00 London |
| Pink | New York | 08:00–17:00 New York |

For each session: solid thick line = **POC**, dashed faded lines = **VAH / VAL**, solid lines =
**High / Low**, dotted line and dot = **Open**. Labels read e.g. `London POC 3898.50`. Levels within
a few ticks of each other share one label.

A few ways people use these:

- **VAH / VAL** are the edges of where most of the session's volume traded. Price back inside the
  value area tends to rotate toward the POC; price holding above VAH (or below VAL) suggests
  acceptance of the move.
- **Session High / Low** are where stops tend to sit, so they're common liquidity targets; the ICT
  module marks the same idea as buyside / sellside liquidity.
- The previous session's levels usually matter for the next one: London often reacts to Asia's
  range, and New York to London's.

## Settings worth knowing (gear icon on the indicator)

- **Sessions**: turn each session on or off, change hours and time zone, pick the colours.
- **Days to show** (default 2) and **Extend lines to** (day end, or stop at the session's own end).
- **Session volume profile**: value area % (default 70) and the detail timeframe used to build it.
- **ICT / Supply & Demand / Liquidity Swings**: each has its own group with the same options as the
  original LuxAlgo indicators, plus an **Enable** switch for Supply & Demand and Liquidity Swings.

It works best on 1-hour charts and below (5m and 15m are the sweet spot). On 4-hour or daily charts
the session boundaries get blurry.

This is a charting tool, not a signal service: it draws levels, it doesn't tell you when to trade.
Please paper-trade with it first until you're comfortable with how the levels behave on your
instruments.

Reply to this email if you hit any problems installing it or want a setting changed.

Best regards,
[Your name]

---

## Note before sending (not part of the email)

Three of the four modules are LuxAlgo's open-source indicators, licensed
**CC BY-NC-SA 4.0**. Sharing the combined script by email is allowed as long as:

- it's **non-commercial**: not sold, not bundled with a paid service, and not used as a
  commercial offering to clients;
- the LuxAlgo attribution and licence notices stay in the file (they do); and
- recipients get it under the same terms, so it's worth saying in the email that the script is
  for personal, non-commercial use.

If the recipients are paying clients, or this goes out under a company name as part of a service,
that's commercial use and you'd need LuxAlgo's permission first. Also don't publish it on
TradingView as a public script: TradingView's house rules on reusing other authors' open-source
code are stricter than the licence.

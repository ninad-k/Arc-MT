# The architecture page

`/architecture` (main menu → Architecture) shows how every Desk decision was made and, when no trade was taken,
why. Spec: [tasks/spec-architecture-page.md](../tasks/spec-architecture-page.md). Decision record:
[ADR-0025](adr/0025-architecture-page-and-master-account-lock.md).

> Analysis, not financial advice. Nothing on this page sends an order: trades still need Sentinel, Pilot and
> your 15-second confirm.

## How a decision is made

1. **Signals:** Scout's movers, the market view, the calendar, or your question in Desk mode.
2. **General members:** Atlas, Capitol, Scout, Athena, Chartist and Oracle. Each gives one vote on the
   symbol: bull, bear or neutral, with a confidence from 0 to 100, its evidence and its sources.
   - The votes are calculated by code from what each member found, not by a language model.
   - A member with no data votes neutral at 0 and says why.
   - The rules are in `crates/desk/src/opinion.rs`, version `v1`.
3. **Council** (code):
   - The bull and bear cases list the strongest weighted evidence for each side.
   - The risk seat is Sentinel's checks that need no plan: kill switch, open positions, the last minutes before
     the close, and the US market being closed.
   - The chair is the score: the sum of weight × vote, from −1 to +1. The council passes at |score| ≥ 0.30 with
     at least 3 members taking a side.
4. **Master (Friday):** on a passed decision, "Ask Friday" runs one Desk turn. Friday plans the trade
   through Sentinel or stands aside. The page shows the model, tokens and cost, the exact messages sent (system
   prompt by hash), the reply and the parsed plan.
5. **Execution:** Pilot's preview, the gate (`[limits]`), your confirm and the broker's result.

The **free council scan** runs steps 1 to 3 over the watchlist and `[desk] universe` every
`scan_every_minutes` (default 15) while the US market is open, and when you click "Scan now". It costs
nothing: no language model and no paid data.

## Why no trade

Every decision that did not trade has one stop code: the first stage that stopped it. The funnel counts them.

| Stage | Codes |
|---|---|
| Signal | `market_closed`, `no_data` |
| Members | `members_split` (fewer than 3 took a side) |
| Council | `council_below_threshold`, `sentinel_veto:<rule>` |
| Master | `not_asked` (waiting for Ask Friday), `friday_stood_aside`, `plan_invalid`, `sentinel_veto:<rule>` |
| Execution | `gate_limit:<rule>`, `kill_switch`, `account_mismatch`, `intent_mismatch`, `preview_expired`, `trade_failed:<retcode>`, `outcome_unknown` |

## Reviewing a trade

Open a decision (Decisions tab):

- **Review:** add a setup tag, mistake tags, a regime, a 1 to 5 rating, "take again?" and notes. Each save is
  audited.
- **Export:** the list exports as CSV, or as JSON with the full records.

## Learning

Every decision is checked 4 hours later. A member's vote is a hit when the price moved at least a quarter ATR
its way, a miss when it moved that much the other way, and flat in between.

The Learning tab shows:

- each member's hit rate and Brier skill (0 = a coin), with an 80% interval;
- taken and counterfactual (not traded) results, kept apart;
- what each guard stopped and how far the price moved afterwards, in ATR.

**Weights.** Code proposes new weights:

- They move toward the members that scored better, shrunk toward equal weights while data is thin.
- Each weight stays between 0.08 and 0.33 and moves at most 0.05 per review.
- A proposal is offered only when it does no worse on the newest 30% of decisions.
- A member needs 30 scored votes before its weight changes.

**Run review** sends these statistics, and nothing else, to Claude Sonnet 5.5. Claude explains them and may
suggest guard changes from a list of values the code offers. The code then checks the answer:

- a value not on the list is rejected;
- a figure that is not in the statistics is rejected;
- rejected items are listed with the reason.

**Applying a change:**

- Re-weighting and tightening a guard apply in one click when there are 30 closed decisions.
- Loosening a guard needs all of these:
  - 100 closed decisions;
  - 30 resolved stops by that guard that missed moves on average;
  - a second confirmation;
  - your authenticator code, when login is on.
- Each change is a new version, with rollback.

Cost: about $0.03 a review (estimate; Sonnet 5.5 is $2 per million input tokens and $10 per million output
tokens). It is capped at `[learning] monthly_budget_usd` (default $2) and runs only when you click.

## Master account lock

The Desk trades on MT5 only while MT5 is logged into the expected account (default IC Markets 53107904):

- The lock is checked when Pilot previews and again when you confirm.
- The chip in the page header shows the state. Click it to change the number: type it twice, plus your
  authenticator code when login is on. No MT5 password is needed.
- `0` turns the lock off.
- Trading mode and Alpaca are not affected.

The EA reports the account number to the backend from version 0.7.0. Recompile
`mql5/Experts/JarvisMT/JarvisBridge.mq5` in MetaEditor once and re-attach it. With an older EA the lock shows
"unverified" and the Desk will not trade on MT5.

The browser only ever sees the number you set and whether MT5 matches it. The language model sees neither.

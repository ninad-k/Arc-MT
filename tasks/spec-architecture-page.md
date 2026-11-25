# Spec: Architecture page (decision pipeline, why-no-trade, learning loop, master account lock)

Status: **approved 2026-11-25** with these answers to the open questions:

- Council in code only.
- Account lock for the Desk master only, with SPEC rule 8 amended as described.
- Learning review on Sonnet 5.5, cost approved: about $0.03 per review, manual runs only, $2 monthly cap.

Build approved. Wireframe: [wireframe-architecture.html](wireframe-architecture.html).
Plan: [plan-architecture-page.md](plan-architecture-page.md).

The request (2026-11-25), in short:

- one page that shows each decision from start to finish: the signals, then every general member's opinion,
  then every council member's opinion, then the master's final proposal (the exact prompt) that would execute;
- when no trade is taken, the page says why;
- a self-learning loop: Claude reads the track record and suggests new weights and guard changes, which the
  user applies with one click;
- every sub-bot can be clicked to see its details;
- professional design in the current theme;
- ready for manual analysis of every trade;
- the master bot is locked to IC Markets account **53107904**, and the number can be changed from the frontend
  without a password, because MT5 is already logged in.

## Objective

Make every Desk decision inspectable and explainable, including the ones that end in no trade. Turn the track
record into concrete, reviewable improvements without giving the LLM any write access.

Users: the account owner, reviewing live during the session and in depth afterwards.

## Research summary (sources in the research notes of this session)

- **Multi-agent trading frameworks:** TradingAgents (analysts, then a bull/bear debate, a trader and a risk
  team) and ai-hedge-fund show each role's reasoning as text transcripts with an approval step. Neither has a
  structured visual explanation, so this page fills that gap.
- **Visual idioms:**
  - An **attrition funnel** answers "why no trade" fastest and scales to many decisions.
  - **Decision cards on a vertical timeline** read best for one decision's reasoning.
  - **Consensus bars** and **contribution waterfalls** suit 6 to 10 members.
  - A node graph is useful only as a small, static topology map. Sankey diagrams and graph libraries add cost
    without helping a trader read one decision, so the map is hand-drawn SVG (no new dependency).
- **Trade journals** (Tradervue, Edgewonk, TradeZella): per-trade setup and mistake tags, a rating, notes, a
  chart snapshot, saved filters, a weekly review and CSV export.
- **AI audit practice:** an immutable record per decision with:
  - the model and version, prompt hash and full prompt, an input snapshot, the raw and parsed output;
  - deterministic guard results as separate pass/fail rows, visually distinct from LLM opinions;
  - the human action.
- **Learning with small samples** (tens of trades):
  - Score each opinion with a proper scoring rule (Brier).
  - Use a multiplicative-weights (Hedge) update, shrunk toward equal weights.
  - Bound the step, set minimum samples and a cooldown, and require a walk-forward holdout check.
  - Count skipped signals' outcomes separately (counterfactual), never pooled silently with taken trades.
  - Tightening a guard needs less evidence than loosening one. Never loosen automatically.
  - The LLM explains and ranks. Code computes every number and validates every proposal.

## Design

### Tiers (definitions, since neither repo had them)

1. **Signals:** what starts a decision.
   - Scout movers.
   - Market-view setups (bias plus setup).
   - Atlas calendar or tone events.
   - A user question in Desk.
2. **General members** (6), each giving one structured **Opinion**:
   - Atlas (macro), Capitol (insiders and Congress), Scout (momentum), Athena (fundamentals and supply/demand),
     Chartist (levels and band), Oracle (probabilities).
   - Each Opinion has stance (bull, bear or neutral), confidence 0 to 100, horizon, evidence lines, sources, and
     the agent's raw data.
   - Opinions are computed **in code** from each agent's existing data, so they are free, deterministic and
     testable. Today only the market view has a stance, so the mapping per agent is new (plan S1). An example:
     Oracle `p_up` 0.64 becomes bull, confidence 28.
3. **Council** (4 seats):
   - **Bull case** and **Bear case**: the strongest weighted evidence for each side, assembled in code from the
     members' opinions.
   - **Risk seat**: Sentinel's rules, which can veto.
   - **Chair**: the council score. Score = Σ wᵢ · sign(stanceᵢ) · confᵢ/100, from −1 to +1. The council passes
     when |score| ≥ `min_council_score` (default 0.30) and at least 3 members are non-neutral.
4. **Master: Friday.** For a council-passed decision, Friday's proposal shows:
   - the model;
   - the exact prompt: system prompt hash, the context sent, and the tool results;
   - the raw output and the parsed plan (side, entry, stop, target, rationale);
   - Sentinel's sizing.

   The page also shows "Friday stood aside" with Friday's reason.
5. **Execution:** Pilot preview, then the gate (`[limits]`), then the 15 s human confirm, then the broker result.
   The rules are unchanged: nothing executes without the user's confirm.

A **free council scan**:
- runs the members and the council in code over the watchlist plus `[desk] universe`;
- runs every 15 minutes during the US session, and when the user clicks "Scan now";
- needs no LLM and no new paid data.

So the funnel shows decisions even when nobody asked Friday. Friday (paid) runs on a council-passed decision
only when the user clicks **Ask Friday**. An optional **auto-ask** setting is off by default and counts against
the Desk $20 cap.

### Why no trade: one stop code per decision

Each decision that does not end in a filled trade carries exactly one `stop_code`, the first stage that
stopped it, plus a detail line. The stop codes are:

| Stage | stop codes |
|---|---|
| Signal | `market_closed`, `pre_close_cutoff`, `event_blackout`, `budget_exhausted`, `no_data` |
| Members | `members_split` (fewer than 3 non-neutral), `stale_data` |
| Council | `council_below_threshold`, `sentinel_veto:<rule>` |
| Master | `not_asked` (waiting for Ask Friday), `friday_stood_aside`, `plan_invalid` |
| Execution | `gate_limit:<rule>`, `kill_switch`, `account_mismatch`, `intent_mismatch`, `preview_expired`, `user_declined`, `trade_failed:<retcode>`, `outcome_unknown` |

Each decision page shows the code on a banner ("Stopped at Council: score +0.18 below 0.30"). The funnel counts
decisions by stop code. Execution-stage codes map one-to-one from the existing `TradingError`, `PilotError` and
Sentinel reasons, so no reason is invented.

### Persistence (new SQLite migrations; the audit log stays the source of truth for actions)

- `decisions`:
  - id, created_at, session_id, symbol, venue, trigger, stage_reached, status (open, traded, stopped),
    stop_code, stop_detail;
  - council_score, threshold, weights_version;
  - plan_id, preview_id, ticket;
  - master_json (model, prompt hash, prompt, raw output, parsed plan, usage);
  - outcome fields filled later (result_r, pnl, mfe/mae, closed_at).
- `decision_opinions`: decision_id, tier (member or council), agent, stance, confidence, weight, contribution,
  evidence_json, sources_json, data_json. This fixes today's gap, where the Finding `data` is not stored.
- `decision_reviews`: the manual analysis.
  - setup tag, mistake tags, regime tag, rating 1 to 5, "take again?" (yes or no), notes.
  - This is the one editable table, and each edit is audited.
- Opinion outcomes (columns on `decision_opinions` in the build, not a separate table): resolved at the horizon with the market journal's rules (move ≥ 0.25 ATR in the stance
  direction; tp/sl first touch). Each row has `counterfactual = true` for decisions that were not traded.
- `config_versions`: versioned snapshots of the council weights and Desk guards (who, when, why, proposal id).
  Rollback restores a snapshot.
- Sentinel plans and Pilot previews stay in memory, as today, but their outcome is copied into the decision row,
  so nothing the page needs is lost on restart.

### Learning loop (human-in-the-loop; Claude has no write access)

1. **Code** computes the statistics on demand ("Run review") or weekly:
   - per member: n resolved, hit rate, Brier skill vs 0.5 (BSS = 1 − Brier/0.25) with a bootstrap 80%
     interval, and taken vs counterfactual shown separately;
   - per guard: how often it stopped a decision, and the counterfactual R of what it stopped.
2. **Code** proposes weights with a shrunk Hedge update:
   - raw weight ∝ exp(η · nᵢ/(nᵢ+50) · skillᵢ) with η = 0.5;
   - mixed with equal weights by λ = min(1, n/100);
   - clamped to [0.5/N, 2/N] and moving at most ±0.05 per review;
   - only offered when it does not worsen a 70/30 walk-forward holdout.
3. **Claude** (Sonnet 5.5) receives the statistics as JSON and returns proposals in a fixed schema:
   {id, type: weight or guard, param, old, new, rationale, evidence_refs, confidence, direction: tighten or
   loosen}. It writes the explanation and may suggest guard changes. Code then:
   - rejects any number not present in or derivable from the input;
   - enforces the bounds above;
   - labels each proposal **tighten** or **loosen**.
4. **One click:**
   - Weight changes and tightening guard changes apply with one click and a 15 s undo.
   - Loosening a guard (a higher risk %, more positions, a lower threshold, a later cutoff) needs n ≥ 100, a
     holdout pass, a second confirm, and TOTP when auth is on.
   - Every apply writes a `config_versions` row and an audit row. Rollback is one click.
5. **Gates on proposing at all:**
   - n ≥ 30 resolved opinions per member before any weight proposal;
   - n ≥ 30 decisions before any guard proposal;
   - at most one review per week or per 20 new resolved decisions.

   Before then, the Learning tab shows the progress ("12 of 30 resolved").
6. **Scope of tunables:**
   - Included: council weights, `min_council_score`, and the Desk guards (`risk_percent`,
     `max_open_positions`, `no_new_trades_before_close_minutes`, `min_reward_risk`, new and default 1.5).
   - Advisory text only, never applied here: `[limits]` (still changed in Settings with TOTP) and the
     market-view weights.

### Master account lock (IC Markets 53107904)

- **EA change:**
  - `JarvisBridge.mq5` adds the `ACCOUNT_LOGIN` number to the ping reply. In the build this is an optional
    field (EA 0.7.0) and the protocol version is unchanged, so older EAs still connect.
  - The user recompiles the EA once in MetaEditor, since it is a manual step in MT5.
  - An old EA reports no login, so the lock shows "unverified" and blocks Desk trading.
- **Backend:**
  - It stores the expected login in `[desk] master_account` (default `53107904`, broker label "IC Markets"),
    overridable from the UI and stored in SQLite.
  - On every Pilot preview and again at confirm, it compares the expected login with the reported one. A
    mismatch stops the decision with `account_mismatch`.
  - The Pilot path checks before preview and at confirm (re-check), so a switch in between is caught too.
- **Frontend:**
  - Settings → Desk → "Master account" field, also opened from the lock chip on the page.
  - The chip shows "IC Markets ••7904 · locked ✓", or "mismatch" or "unverified".
  - Changing the number needs no password. It asks for a typed confirmation, plus TOTP when auth is on, as
    other security settings do. It is audited.
- **SPEC rule 8 amendment (ADR-0025):**
  - The rule says the login never reaches the LLM or the browser.
  - The amendment: the backend may receive the login from the EA only to compare it. The browser sees only the
    expected number the user typed (masked by default) and match/mismatch, never the reported number. The LLM
    sees neither.
  - Name and server rules are unchanged. A test asserts no login in any LLM tool result or `/api` response
    other than the user's own expected value.

## Acceptance criteria

1. `/architecture` opens from the main menu and has three tabs: **Pipeline** (live map plus the latest
   decision), **Decisions** (funnel plus table plus filters), **Learning** (scorecards, proposals, versions).
2. The map shows all tiers. Clicking any node (each member, each council seat, Friday, Sentinel, Pilot, the
   gate) opens a drawer with:
   - role, inputs and data sources;
   - its last opinion or verdict, current weight and skill;
   - recent opinions and its config.
3. Every decision shows each member's stance, confidence, evidence and sources; the council's score with each
   member's contribution; and, when Friday was asked, the exact prompt, the model, and the raw and parsed
   output.
4. Every decision that did not trade shows exactly one stop code and a plain-English reason. The funnel's
   counts add up.
5. The free scan runs with no LLM call, verified by a test with no provider configured.
6. A learning review sends Claude the statistics only (no keys, logins or account numbers). A proposal with a
   number not in the input is rejected. A loosening proposal cannot be applied in one click. Apply and rollback
   each write a version and an audit row.
7. With the master account set to 53107904, a Pilot preview on another login is refused with
   `account_mismatch`. The browser never receives the reported login. The LLM never receives any login.
8. Decisions export to CSV and JSON (with the full record), and each has a review form (tags, rating, notes).
9. The page uses the existing tokens (void, deck, panel, line, ink, up, down, warn, violet, accent), works from
   1280 px down to 390 px, and passes keyboard and screen-reader checks (drawer focus trap, labelled controls).

## Testing

- Rust:
  - opinion mappers per agent;
  - council score and pass rule;
  - stop-code mapping from every `TradingError`, `PilotError` and Sentinel rule;
  - the shrunk-Hedge update (bounds, shrinkage, step cap);
  - proposal validation (unknown numbers, loosening);
  - version apply and rollback;
  - the account-lock compare at preview and confirm;
  - rule-8 leak tests.
- Web: Vitest for formatters, funnel math and stop-code text. Playwright e2e on `?mock=architecture` (CI only,
  since Chromium is not installed locally).
- Claude: a recorded response fixture, so no test calls the API.

## Cost (needs your approval before anything paid runs)

| Item | Price | Estimate |
|---|---|---|
| Council scan, members, funnel, page | free (code + existing free data feeds) | $0 |
| Learning review on **Sonnet 5.5** (`claude-sonnet-5-5`) | $2 / MTok input, $10 / MTok output (list price, claude-api reference) | ~6k in + ~2k out = **~$0.03 per review** (estimate) |
| Weekly review | | **~$0.13 / month** (estimate); daily would be ~$0.95 / month |
| Ask Friday (existing Desk cost, Opus 5.5 $4 / $20) | unchanged | ~$0.06-0.12 per ask, inside the Desk $20 cap |

The learning review gets its own cap, `[learning] monthly_budget_usd = 2`. It runs only when you click "Run
review" or on the weekly schedule if you turn that on. Auto-ask Friday is off by default.

## Boundaries

- Always:
  - the LLM keeps read and propose tools only;
  - the confirm stays human;
  - code computes the numbers;
  - every config change is versioned and audited.
- Ask first: loosening any guard; changing `[limits]`; turning on auto-ask or the weekly review.
- Never:
  - automatic config changes;
  - Claude writing config;
  - the reported MT5 login in the browser, or any login in the LLM;
  - pooling counterfactual and taken outcomes in one number without labelling them.

## Open questions (my proposed defaults in brackets)

1. Council seats: [Bull case, Bear case, Risk (Sentinel), Chair, all in code, no extra LLM call]. An LLM-argued
   bull/bear debate would cost about $0.05 per decision.
2. Account-lock scope: [Desk/master bot only], or every MT5 trade including Trading mode.
3. SPEC rule 8 amendment as described (the backend compares the login and never shows or forwards it): [yes].
4. Claude cost for the learning review: [Sonnet 5.5, ~$0.03 per review, $2 monthly cap, manual "Run review"
   only].

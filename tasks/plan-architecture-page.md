# Plan: Architecture page

Spec: [spec-architecture-page.md](spec-architecture-page.md), approved 2026-11-25 (council in code, lock on the Desk master only, manual Sonnet 5.5 reviews).
Wireframe: [wireframe-architecture.html](wireframe-architecture.html).

Branch: `feat/architecture-page` from `main`. One PR, built in the slices below. Each slice ends green (fmt,
clippy, tests, web check) before the next starts.

## Slices

- [ ] **S1 Opinions (crates/desk, pure):**
  - Add the `Opinion {stance, confidence, horizon, evidence, sources}` type.
  - Add one mapper per member (Atlas, Capitol, Scout, Athena, Chartist, Oracle) from its existing `Finding.data`.
  - Unit tests per mapper, covering edge cases: missing data gives neutral 0, and confidence is clamped.
- [ ] **S2 Council (crates/desk, pure):**
  - Weights, score, contributions and the pass rule.
  - Bull and bear cases (top weighted evidence).
  - A Sentinel seat wrapper.
  - The `StopCode` enum and its text.
  - Tests: score math, ties, fewer than 3 non-neutral, veto precedence.
- [ ] **S3 Decision records (crates/api):**
  - Migrations: `decisions`, `decision_opinions`, `decision_reviews`, `opinion_outcomes`, `config_versions`.
  - A `decisions.rs` store.
  - Stop-code mapping from `TradingError`, `PilotError` and Sentinel reasons, with an exhaustive test over every
    variant.
  - Desk turns, Pilot and confirm write and update decision rows.
- [ ] **S4 Free council scan:**
  - Runs the members, then the council, then a decision row with a stop code.
  - Every 15 min in the US session, plus `POST /api/architecture/scan`.
  - A test with no LLM provider configured proves it never calls one.
  - "Ask Friday" on a passed decision (`POST /api/architecture/decisions/{id}/ask`) records the master prompt,
    model, raw output and parsed plan.
- [ ] **S5 Read API:**
  - `GET /api/architecture/map` (nodes, weights, skill, last opinion).
  - `GET /api/architecture/decisions` (filters: date, symbol, stage, stop code, stance, tag).
  - `GET /api/architecture/decisions/{id}` and `GET /api/architecture/funnel`.
  - CSV and JSON export.
  - `PUT /api/architecture/decisions/{id}/review` (audited).
- [ ] **S6 Web, Pipeline tab:**
  - The `/architecture` route and a menu entry.
  - The hand-drawn SVG tier map, with a node drawer (focus trap, Esc).
  - Latest-decision strip and stop banner.
  - The `?mock=architecture` fixture.
- [ ] **S7 Web, Decisions tab:**
  - Funnel (click to filter), table with a stage strip, filters and saved filters.
  - The decision detail route: timeline cards, consensus bar, contribution waterfall, master prompt viewer,
    guard rows, review form, export.
- [ ] **S8 Outcomes + statistics:**
  - Resolve opinions at the horizon with the market-journal rules (taken and counterfactual flagged).
  - Per-member Brier skill with a bootstrap interval, and per-guard stop counts and counterfactual R.
  - The shrunk-Hedge weight proposal, with bounds, step cap, minimum n and the holdout check.
- [ ] **S9 Learning loop:**
  - `[learning]` config and a spend cap.
  - A Claude Sonnet 5.5 review call that sends the statistics JSON and receives proposals JSON, with a recorded
    fixture in tests.
  - Proposal validation: number provenance, bounds, the tighten/loosen label.
  - Apply and rollback through `config_versions`. Loosening needs n ≥ 100, the holdout, a second confirm and TOTP.
  - The Learning tab UI.
- [ ] **S10 Master account lock:**
  - ADR-0025, with SPEC rule 8 amended.
  - EA: `ACCOUNT_LOGIN` in the ping reply, and the protocol version bumped.
  - Bridge-client: `PingInfo.login: Option<u64>`, kept out of every serialised view.
  - `[desk] master_account = 53107904` and a UI override (audited, typed confirm, TOTP when auth is on).
  - The check at Pilot preview and at confirm.
  - The lock chip and the Settings field.
  - Leak tests: no login in LLM tool results or `/api` responses.
- [ ] **S11 Docs + review:**
  - `docs/architecture-page.md`, the config example and the CHANGELOG.
  - A code review (agent-skills `code-review-and-quality`) and a security review of S9 and S10.
  - The full local check, then the PR.

## Manual step for you

After S10, recompile `mql5/Experts/JarvisMT/JarvisBridge.mq5` in MetaEditor and re-attach the EA, so it reports
the account login. Until then the lock shows "unverified" and Desk will not trade.

## Risks

- **Mapping agent data to a stance is a judgement call.** The mappers are simple, documented and versioned, and
  the learning loop scores them, so a bad mapper loses weight.
- **Small samples.** No proposal is made below the minimum n, and the tab shows progress instead.
- **Data rate limits during the 15-min scan.** The universe is small (watchlist + `[desk] universe`). The scan
  reuses the existing caches, and a failing feed gives `no_data`, not a guess.

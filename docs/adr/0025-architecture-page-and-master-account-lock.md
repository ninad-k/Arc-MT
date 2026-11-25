# ADR-0025: The architecture page, a human-approved learning loop, and the Desk master account lock

- Status: Accepted
- Date: 2026-11-25

## Context

The user asked for an architecture page that shows each Desk decision from start to finish: the signals, every
member's opinion, the council, and the master's exact proposal. When no trade is taken, the page must say why.
The user also asked for:

- a self-learning loop on the Claude API that suggests new weights and guard changes, applied with one click;
- per-trade manual analysis;
- the master bot locked to one IC Markets MT5 account (53107904), changeable from the frontend with no password.

The spec (`tasks/spec-architecture-page.md`) was approved with these choices: the council in code, the lock for
the Desk master only, and Sonnet 5.5 for the review with manual runs and a $2 monthly cap.

SPEC core rule 8 says account credentials stay in MT5 and the backend never sends them to the LLM or the
browser. The lock needs the backend to know which account MT5 is on, which it did not before (the EA never sent
the login).

## Decision

- **Opinions and the council are code** (`jarvis-desk::opinion`, `::council`).
  - Each member's finding is mapped to a stance and a confidence by a documented, versioned rule (`v1`).
  - The council score is a weighted sum. A pass needs |score| ≥ `min_council_score` and at least
    `min_non_neutral` members taking a side.
  - Sentinel's plan-free checks act as the risk seat.
  - The free council scan uses no language model. Friday, the paid step, runs only on "Ask Friday".
- **One decision record per decision** (`decisions` and related tables), updated as the decision moves:
  - It carries one stop code when it did not trade: the first stage that stopped it.
  - Execution codes map one-to-one from `TradingError`, `PilotError` and Sentinel's rule ids. Sentinel's veto
    now carries the rule id behind each reason.
  - The audit table stays the record of every action.
- **Learning: code computes, Claude explains, the trader applies.**
  - Statistics: Brier skill per member and counterfactual guard outcomes, with taken and counterfactual
    outcomes kept apart.
  - Weight proposal: a shrunk Hedge update with bounds, a step cap and a walk-forward holdout.
  - Claude receives statistics only and has no tools. Its answer is checked:
    - a guard value must be one of the candidates the code offered;
    - every figure in a rationale must come from the statistics.
  - Tightening and re-weighting apply in one click when the evidence gates pass.
  - Loosening needs 100 closed decisions, a counterfactual record that favours it, a second confirmation and,
    with login on, an authenticator code. Rolling back to looser guards needs the same.
  - Every change is a new `config_versions` row and an audit row.
- **Master account lock, and rule 8 amended:**
  - From 0.7.0 the EA adds `login` (`ACCOUNT_LOGIN`) to its `ping` reply. The field is optional, so older EAs
    still work and the lock then reads "unverified". The protocol version is unchanged.
  - The backend compares that number with the expected account (`[desk] master_account`, default 53107904,
    changeable in the UI). It checks before Pilot previews an MT5 trade and again at confirm.
  - A mismatch or "unverified" refuses the Desk's MT5 trades (`account_mismatch`). Trading mode and Alpaca are
    not affected.
  - The browser sees only the expected number the trader typed and the state. The reported number is in no
    response.
  - The LLM sees neither number.
  - Changing the number needs no MT5 password. It needs the number typed twice, an authenticator code when
    login is on, and an audit row with masked numbers.
  - Rule 8 now reads: credentials stay in MT5; the backend may read the account number only to compare it for
    the master account lock and never sends it to the LLM; the browser sees only the expected number it set
    and match or mismatch.

## Consequences

- Tests:
  - `crates/api/tests/architecture.rs` asserts the reported number appears in no `/api` response and no LLM
    request.
  - The same file asserts that a switch between preview and confirm is refused, an old EA leaves the lock
    unverified, Claude receives no symbols or account figures, and locked or looser changes cannot apply in one
    click.
- The user must recompile the EA (0.7.0) in MetaEditor once. Until then the Desk does not trade on MT5.
- Sentinel now vetoes a target with reward to risk under `min_reward_risk` (default 1.5). Before, it only warned
  under 1.
- Stance rules are judgement calls. They are versioned and scored, so a weak rule loses weight. Changing a rule
  bumps `MAPPER_VERSION`.
- Counterfactual outcomes use no fill or cost model. The page labels them as indicative.

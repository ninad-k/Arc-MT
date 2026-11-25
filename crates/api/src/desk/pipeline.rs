//! The decision pipeline on the Desk (spec-architecture-page): the free council scan, "Ask
//! Friday" on a decision the council passed, and the decision records of Friday's turns.
//!
//! The scan runs the six members and the council in code: no language model, no new paid data.
//! Friday (the paid step) runs only when the trader asks, and Pilot and the trader's confirm
//! still execute, exactly as before.

use jarvis_desk::council::{self, Stage, StopCode};
use jarvis_desk::opinion::{self, AssetKind, MEMBERS, Opinion, OpinionContext};

use super::*;
use crate::decisions::{NewDecision, Trigger, master_record};

/// Symbols scanned at most.
pub const MAX_SCAN: usize = 20;
/// The members' horizon: Oracle looks four H1 bars ahead.
pub const HORIZON: &str = "4h";

/// Why "Ask Friday" could not start.
#[derive(Debug)]
pub enum AskError {
    /// No decision with this id is waiting for Friday.
    NotWaiting,
    Llm(LlmError),
    Store(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::NotWaiting => f.write_str("this decision is not waiting for Friday"),
            AskError::Llm(e) => write!(f, "{e}"),
            AskError::Store(e) => write!(f, "{e}"),
        }
    }
}

/// The symbol a finding is about, when it is about one.
fn finding_symbol(f: &Finding) -> Option<&str> {
    f.data
        .get("symbol")
        .and_then(Value::as_str)
        .or_else(|| f.data.pointer("/forecast/symbol").and_then(Value::as_str))
}

impl Desk {
    /// Whether the members should read the symbol as a stock, an index or something else.
    pub(super) fn asset_kind(&self, canonical: &str) -> AssetKind {
        if self
            .parts
            .config
            .indices
            .iter()
            .any(|i| i.eq_ignore_ascii_case(canonical))
        {
            return AssetKind::Index;
        }
        if let Some(catalog) = self.parts.trading.broker().book().catalog()
            && catalog.instruments().iter().any(|i| {
                i.canonical.eq_ignore_ascii_case(canonical)
                    && i.class == jarvis_symbols::AssetClass::Index
            })
        {
            return AssetKind::Index;
        }
        if self.trades_in_us_session(canonical) {
            AssetKind::Stock
        } else {
            AssetKind::Other
        }
    }

    fn venue_name(&self, canonical: &str) -> &'static str {
        if self.parts.router.is_alpaca_symbol(canonical) {
            "alpaca"
        } else {
            "mt5"
        }
    }

    /// Each member's opinion on `symbol` from the findings at hand; a member with no finding
    /// for it votes neutral with confidence 0.
    fn opinions_for(
        &self,
        findings: &[Finding],
        ctx: &OpinionContext,
        missing_why: &str,
    ) -> Vec<(Opinion, Option<Value>)> {
        MEMBERS
            .iter()
            .map(|agent| {
                let f = findings.iter().rev().find(|f| {
                    f.agent == *agent
                        && match agent {
                            AgentId::Atlas | AgentId::Scout => true,
                            _ => finding_symbol(f)
                                .is_some_and(|s| s.eq_ignore_ascii_case(&ctx.symbol)),
                        }
                });
                match f.and_then(|f| opinion::opinion(f, ctx).map(|o| (o, f.data.clone()))) {
                    Some((o, data)) => (o, Some(data)),
                    None => (opinion::missing(*agent, ctx, missing_why), None),
                }
            })
            .collect()
    }

    /// The risk seat's checks that need no plan.
    async fn risk_flags(&self, canonical: &str) -> Vec<council::RiskFlag> {
        match self.risk_context(canonical).await {
            Ok(ctx) => council::risk_flags(jarvis_desk::sentinel::precheck(
                &ctx,
                &self.rules(),
                canonical,
            )),
            Err(e) => vec![council::RiskFlag {
                rule: "context".into(),
                reason: format!("Could not read the account: {e}"),
            }],
        }
    }

    /// Claude usage of this turn: the `claude_usage` rows after the turn's utterance.
    async fn turn_usage(&self, sid: Uuid) -> Value {
        let Ok(rows) = self.parts.audit.rows(Some(sid)).await else {
            return Value::Null;
        };
        let start = rows
            .iter()
            .rev()
            .find(|r| r.kind == "utterance")
            .map_or(0, |r| r.id);
        let (mut input, mut output, mut cost) = (0u64, 0u64, 0f64);
        for r in rows
            .iter()
            .filter(|r| r.id > start && r.kind == "claude_usage")
        {
            input += r.payload["input_tokens"].as_u64().unwrap_or(0);
            output += r.payload["output_tokens"].as_u64().unwrap_or(0);
            cost += r.payload["cost_usd"].as_f64().unwrap_or(0.0);
        }
        json!({ "input_tokens": input, "output_tokens": output, "cost_usd": cost })
    }

    /// Record what one Friday turn decided: every Sentinel plan becomes (or completes) a
    /// decision with the turn's master record; an "Ask Friday" turn without a plan for its
    /// symbol records that Friday stood aside.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_turn(
        &self,
        sid: Uuid,
        provider: &dyn LlmProvider,
        system: &str,
        history: &[Message],
        message: &str,
        reply: &str,
        found: &Found,
    ) {
        let ask = lock(&self.asks).remove(&sid);
        if ask.is_none() && found.plans.is_empty() {
            return;
        }
        let usage = self.turn_usage(sid).await;
        let messages = serde_json::to_value(history).unwrap_or(Value::Null);
        let master = master_record(
            provider.name(),
            provider.model(),
            system,
            &messages,
            reply,
            usage,
        );
        let ask_symbol = match &ask {
            Some(id) => self
                .decisions
                .get(id)
                .await
                .ok()
                .flatten()
                .map(|d| d.summary.symbol),
            None => None,
        };
        let mut ask_done = false;
        for plan in &found.plans {
            let symbol = plan.plan.symbol.clone();
            let id = match (&ask, &ask_symbol) {
                (Some(id), Some(s)) if !ask_done && s.eq_ignore_ascii_case(&symbol) => {
                    ask_done = true;
                    id.clone()
                }
                _ => {
                    let ctx = OpinionContext {
                        symbol: symbol.clone(),
                        kind: self.asset_kind(&symbol),
                        price: Some(plan.plan.entry),
                        atr: None,
                        horizon: HORIZON.into(),
                    };
                    let opinions =
                        self.opinions_for(&found.findings, &ctx, "Friday did not ask this member.");
                    let live = self.decisions.live();
                    let ops: Vec<Opinion> = opinions.iter().map(|(o, _)| o.clone()).collect();
                    let verdict = council::deliberate(
                        &ops,
                        &live.weights,
                        &live.guards.council(),
                        self.risk_flags(&symbol).await,
                    );
                    let new = NewDecision {
                        session_id: Some(sid),
                        symbol: symbol.clone(),
                        broker_symbol: Some(plan.plan.broker_symbol.clone()),
                        venue: self.venue_name(&symbol).into(),
                        trigger: Trigger {
                            kind: "question".into(),
                            detail: Some(truncate(message, 200)),
                        },
                        price: Some(plan.plan.entry),
                        atr: None,
                        opinions,
                        council: Some(verdict),
                        early_stop: None,
                    };
                    match self.decisions.create(new).await {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::warn!(error = %e, "could not record the decision");
                            continue;
                        }
                    }
                }
            };
            if let Err(e) = self
                .decisions
                .record_master(&id, &master, Some(plan), reply)
                .await
            {
                tracing::warn!(error = %e, "could not record Friday's proposal");
            }
        }
        if let Some(id) = ask
            && !ask_done
        {
            let done = if found.budget_stopped {
                self.decisions
                    .stop(
                        &id,
                        Stage::Master,
                        &StopCode::BudgetExhausted,
                        &truncate(reply, 300),
                    )
                    .await
            } else {
                self.decisions
                    .record_master(&id, &master, None, reply)
                    .await
            };
            if let Err(e) = done {
                tracing::warn!(error = %e, "could not record that Friday stood aside");
            }
        }
    }

    /// Run the members and the council over the watchlist and `[desk] universe`, and record a
    /// decision per symbol. Free: no language model. Returns the new decision ids.
    pub async fn scan(&self, user: &DeskUser) -> Vec<String> {
        // A scan already running records these symbols; a second one would count them twice.
        let Ok(_one) = self.scan_lock.try_lock() else {
            return Vec::new();
        };
        let _ = self.decisions.settle(Utc::now().timestamp_millis()).await;
        let mut symbols: Vec<String> = Vec::new();
        for s in user
            .watchlist
            .iter()
            .chain(self.parts.config.universe.iter())
        {
            let key = s.trim().to_ascii_uppercase();
            if !key.is_empty() && !symbols.contains(&key) {
                symbols.push(key);
            }
        }
        symbols.truncate(MAX_SCAN);
        let atlas = self.atlas(user).await.ok();
        let scout = self.scout(user).await.ok();
        let session = self.exchange.status(Utc::now());
        let mut ids = Vec::new();
        let mut passed = 0;
        for raw in symbols {
            let resolved = self.resolve(user, &raw);
            let (canonical, broker) = match &resolved {
                Ok((c, b)) => (c.clone(), Some(b.clone())),
                Err(_) => (raw.clone(), None),
            };
            let mut new = NewDecision {
                session_id: None,
                symbol: canonical.clone(),
                broker_symbol: broker.clone(),
                venue: self.venue_name(&canonical).into(),
                trigger: Trigger {
                    kind: "scan".into(),
                    detail: None,
                },
                price: None,
                atr: None,
                opinions: Vec::new(),
                council: None,
                early_stop: None,
            };
            let us = self.trades_in_us_session(&canonical);
            let price = match &broker {
                Some(b) => self.price(b).await.map(|p| p.bid).filter(|p| *p > 0.0),
                None => None,
            };
            if us && session.phase != jarvis_desk::session::Phase::Open {
                new.early_stop = Some((
                    Stage::Signal,
                    StopCode::MarketClosed,
                    format!("The US market is {}.", session.phase.label().to_lowercase()),
                ));
            } else if broker.is_none() || price.is_none() {
                new.early_stop = Some((
                    Stage::Signal,
                    StopCode::NoData,
                    match resolved {
                        Err(e) => e,
                        Ok(_) => "No live price.".into(),
                    },
                ));
            } else {
                let args = json!({ "symbol": canonical });
                let mut findings: Vec<Finding> =
                    atlas.iter().chain(scout.iter()).cloned().collect();
                let kind = self.asset_kind(&canonical);
                if kind == AssetKind::Stock
                    && self.parts.finnhub.is_some()
                    && let Ok(f) = self.capitol(user, &args).await
                {
                    findings.push(f);
                }
                if let Ok(f) = self.athena(user, &args).await {
                    findings.push(f);
                }
                let mut scratch = Found::default();
                if let Ok(f) = self.chartist(user, &args, &mut scratch).await {
                    findings.push(f);
                }
                let oracle_args =
                    json!({ "symbol": canonical, "timeframe": "H1", "horizon_bars": 4 });
                if let Ok(f) = self.oracle(user, &oracle_args).await {
                    findings.push(f);
                }
                let atr = findings
                    .iter()
                    .find(|f| f.agent == AgentId::Athena)
                    .and_then(|f| f.data.pointer("/structure/atr"))
                    .and_then(Value::as_f64);
                let ctx = OpinionContext {
                    symbol: canonical.clone(),
                    kind,
                    price,
                    atr,
                    horizon: HORIZON.into(),
                };
                let opinions =
                    self.opinions_for(&findings, &ctx, "No data came back for this member.");
                let ops: Vec<Opinion> = opinions.iter().map(|(o, _)| o.clone()).collect();
                let live = self.decisions.live();
                let verdict = council::deliberate(
                    &ops,
                    &live.weights,
                    &live.guards.council(),
                    self.risk_flags(&canonical).await,
                );
                passed += usize::from(verdict.passed);
                new.price = price;
                new.atr = atr;
                new.opinions = opinions;
                new.council = Some(verdict);
            }
            match self.decisions.create(new).await {
                Ok(id) => ids.push(id),
                Err(e) => tracing::warn!(error = %e, "could not record a scan decision"),
            }
        }
        self.signal(
            AgentId::Friday,
            "info",
            format!(
                "Council scan: {} symbols, {passed} passed and wait for Ask Friday.",
                ids.len()
            ),
        );
        ids
    }

    /// Judge the opinions of decisions older than the horizon against the H1 bars that
    /// followed. Free. Returns how many decisions were resolved.
    pub async fn resolve_outcomes(&self) -> usize {
        use jarvis_desk::learning::{HORIZON_MS, MoveAfter, move_after};
        let now = Utc::now().timestamp_millis();
        let Ok(list) = self.decisions.unresolved(now - HORIZON_MS, 100).await else {
            return 0;
        };
        let mut mt5_offset: Option<Option<i64>> = None;
        let mut bars: HashMap<String, Option<Vec<Bar>>> = HashMap::new();
        let mut done = 0;
        for d in list {
            let (Some(broker), Some(price)) = (d.broker_symbol.clone(), d.price) else {
                let _ = self.decisions.resolve(&d.id, None, now).await;
                done += 1;
                continue;
            };
            let offset = if self.parts.router.is_alpaca_symbol(&d.symbol) {
                Some(0)
            } else {
                if mt5_offset.is_none() {
                    // The bars are on the broker server's clock; whole quarter hours.
                    mt5_offset = Some(self.parts.bridge.ping().await.ok().map(|p| {
                        let raw = p.server_time - Utc::now().timestamp();
                        (raw as f64 / 900.0).round() as i64 * 900
                    }));
                }
                mt5_offset.flatten()
            };
            let Some(offset) = offset else { continue };
            if !bars.contains_key(&broker) {
                let b = self
                    .bars(&broker, Timeframe::H1, 400)
                    .await
                    .ok()
                    .map(|b| b.0);
                bars.insert(broker.clone(), b);
            }
            let Some(Some(series)) = bars.get(&broker) else {
                continue;
            };
            let t0 = d.created_ms / 1000 + offset;
            let resolved = match move_after(series, t0, price, d.atr) {
                MoveAfter::Move(m) => Some(Some(m)),
                MoveAfter::Unjudgeable => Some(None),
                MoveAfter::NotYet => None,
            };
            if let Some(m) = resolved
                && self.decisions.resolve(&d.id, m, now).await.is_ok()
            {
                done += 1;
            }
        }
        done
    }

    /// Ask Friday for a plan on a decision the council passed. Runs one Desk turn (paid, inside
    /// the Desk budget); its outcome is recorded on the decision.
    pub async fn ask_friday(
        &self,
        provider: &dyn LlmProvider,
        metered: bool,
        user: &DeskUser,
        decision_id: &str,
    ) -> Result<DeskOutcome, AskError> {
        let detail = self
            .decisions
            .get(decision_id)
            .await
            .map_err(|e| AskError::Store(e.to_string()))?
            .filter(|d| d.summary.status == "waiting")
            .ok_or(AskError::NotWaiting)?;
        let sid = Uuid::new_v4();
        if !self
            .decisions
            .mark_asked(decision_id, sid)
            .await
            .map_err(|e| AskError::Store(e.to_string()))?
        {
            return Err(AskError::NotWaiting);
        }
        lock(&self.asks).insert(sid, decision_id.to_owned());
        let s = &detail.summary;
        let message = format!(
            "The council passed {} ({}, score {:+.2}, threshold {:.2}). Ask the members you need, \
             then either propose one plan through Sentinel with a stop and a target, or stand \
             aside and say why in one or two sentences.",
            s.symbol,
            s.side.as_deref().unwrap_or("no side"),
            s.score.unwrap_or(0.0),
            s.threshold.unwrap_or(0.0),
        );
        match self
            .run_turn(provider, metered, user, Some(sid), &message, false)
            .await
        {
            Ok(outcome) => Ok(outcome),
            Err(e) => {
                lock(&self.asks).remove(&sid);
                if let Err(err) = self.decisions.reset_waiting(decision_id).await {
                    tracing::warn!(error = %err, "could not put the decision back to waiting");
                }
                Err(AskError::Llm(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_name_their_symbol() {
        let a = Finding::new(AgentId::Athena, "s", json!({ "symbol": "NVDA" }));
        let o = Finding::new(
            AgentId::Oracle,
            "s",
            json!({ "forecast": { "symbol": "EURUSD" } }),
        );
        let s = Finding::new(AgentId::Scout, "s", json!({ "movers": [] }));
        assert_eq!(finding_symbol(&a), Some("NVDA"));
        assert_eq!(finding_symbol(&o), Some("EURUSD"));
        assert_eq!(finding_symbol(&s), None);
    }
}

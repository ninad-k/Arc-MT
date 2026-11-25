//! Alpaca as a venue for `Trading` (spec-alpaca, ADR-0024): Alpaca's REST shapes mapped onto the bridge
//! protocol types, so the gate, previews, tokens and confirm work unchanged.
//!
//! Mapping:
//! - volume = whole shares; contract size 1; tick size and tick value $0.01, so P/L and risk sizing come
//!   out in dollars. Prices have 2 digits.
//! - tickets: [`jarvis_alpaca::ticket_for`] of the asset id (positions) or order id (orders), remembered
//!   for the way back.
//! - an order with a stop and/or target becomes an Alpaca bracket or one-triggers-other order.
//! - today's realised P/L is Alpaca's day change (equity - last equity) minus open P/L, returned as one
//!   synthetic deal so `Trading`'s daily-loss sum works as for MT5.
//! - the trade mode is `Demo` only for the paper host, else `Real`, so the Alpaca `Trading` (which expects
//!   demo) refuses a live endpoint even before the client's own paper check.
//!
//! Errors keep ADR-0013: a write that may have reached Alpaca and could not be looked up by its
//! `client_order_id` is a `Timeout` (unknown outcome, never re-sent); one that never left is a
//! "cannot connect" transport error (not sent).

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use jarvis_alpaca::{AlpacaClient, AlpacaError, NewOrder, ticket_for};
use jarvis_bridge_client::BridgeError;
use jarvis_bridge_client::protocol::{
    self as wire, Account, Command, Deal, DealEntry, DealType, ExecType, History, HistoryQuery,
    OrderType, Orders, PingInfo, Positions, Price, Side, SymbolInfo, TradeMode, TradeResult,
};
use uuid::Uuid;

use crate::egress::EgressLog;
use crate::trading::WriteSink;
use crate::venue::BrokerReads;

const RETCODE_REJECTED: i64 = 10006;
const RETCODE_INVALID_VOLUME: i64 = 10014;
const RETCODE_TRADE_DISABLED: i64 = 10017;
const RETCODE_INVALID: i64 = 10013;
/// A volume larger than any limit allows; the gate's `max_lot` (Alpaca `max_shares`) is what binds.
const VOLUME_MAX: f64 = 1_000_000.0;

#[derive(Debug, Clone)]
enum TicketRef {
    Position(String),
    Order(String),
}

pub struct AlpacaVenue {
    client: AlpacaClient,
    egress: Arc<EgressLog>,
    extended_hours: bool,
    timeout_ms: u64,
    tickets: Mutex<HashMap<u64, TicketRef>>,
}

fn whole_shares(volume: f64) -> Result<u64, BridgeError> {
    if volume.is_finite() && volume >= 1.0 && volume.fract().abs() < 1e-9 && volume <= VOLUME_MAX {
        Ok(volume.round() as u64)
    } else {
        Err(BridgeError::Remote {
            message: format!(
                "Alpaca trades whole shares; {volume} is not a whole number of shares"
            ),
            retcode: RETCODE_INVALID_VOLUME,
        })
    }
}

fn iso_unix(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|t| t.timestamp())
        .unwrap_or_else(|_| chrono::Utc::now().timestamp())
}

impl AlpacaVenue {
    pub fn new(
        client: AlpacaClient,
        egress: Arc<EgressLog>,
        extended_hours: bool,
        timeout_ms: u64,
    ) -> Self {
        Self {
            client,
            egress,
            extended_hours,
            timeout_ms,
            tickets: Mutex::new(HashMap::new()),
        }
    }

    pub fn client(&self) -> &AlpacaClient {
        &self.client
    }

    pub fn is_paper(&self) -> bool {
        self.client.is_paper()
    }

    /// The `alpaca` block of `/api/status`: reachable, paper, market open.
    pub async fn status_view(&self, symbols: &[String]) -> serde_json::Value {
        match self.timed(false, self.client.clock()).await {
            Ok(clock) => serde_json::json!({
                "enabled": true, "reason": null, "connected": true,
                "paper": self.client.is_paper(), "market_open": clock.is_open,
                "error": null, "symbols": symbols,
            }),
            Err(e) => serde_json::json!({
                "enabled": true, "reason": null, "connected": false,
                "paper": self.client.is_paper(), "market_open": null,
                "error": e.to_string(), "symbols": symbols,
            }),
        }
    }

    /// `GET /api/alpaca/account`: the paper account and the market clock. Never an error: a failure
    /// is reported in `error`.
    pub async fn account_view(&self) -> serde_json::Value {
        let (account, clock) = tokio::join!(
            self.timed(false, self.client.account()),
            self.timed(false, self.client.clock())
        );
        let (open, next_open, next_close) = match &clock {
            Ok(c) => (
                Some(c.is_open),
                Some(c.next_open.clone()),
                Some(c.next_close.clone()),
            ),
            Err(_) => (None, None, None),
        };
        match account {
            Ok(a) => serde_json::json!({
                "enabled": true,
                "currency": if a.currency.is_empty() { "USD".to_owned() } else { a.currency.clone() },
                "equity": a.equity, "cash": a.cash,
                "day_pl": match (a.equity, a.last_equity) {
                    (Some(e), Some(l)) => Some(((e - l) * 100.0).round() / 100.0),
                    _ => None,
                },
                "buying_power": a.buying_power,
                "market_open": open, "next_open": next_open, "next_close": next_close,
                "blocked": a.blocked_reason(), "error": null,
            }),
            Err(e) => serde_json::json!({ "enabled": true, "error": e.to_string(),
                                          "market_open": open }),
        }
    }

    /// Recent bars for a chart, oldest first, in the market crate's shape. `timeframe` is Arc MT's
    /// (`M5` .. `D1`).
    pub async fn bars(
        &self,
        symbol: &str,
        timeframe: &str,
        count: u32,
    ) -> Result<Vec<jarvis_market::Bar>, String> {
        // Alpaca timeframe, and calendar days per bar (a 6.5-hour session, weekends and holidays).
        let (tf, days_per_bar) = match timeframe {
            "M5" => ("5Min", 5.0 / 390.0 * 1.5),
            "M15" => ("15Min", 15.0 / 390.0 * 1.5),
            "M30" => ("30Min", 30.0 / 390.0 * 1.5),
            "H1" => ("1Hour", 60.0 / 390.0 * 1.5),
            "H4" => ("4Hour", 240.0 / 390.0 * 1.5),
            _ => ("1Day", 1.5),
        };
        let days = (f64::from(count) * days_per_bar).ceil() as i64 + 5;
        let start = (chrono::Utc::now() - chrono::Duration::days(days))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let bars = self
            .timed(true, self.client.bars(symbol, tf, &start, count))
            .await
            .map_err(|e| e.to_string())?;
        Ok(bars
            .into_iter()
            .map(|b| jarvis_market::Bar {
                time: b.time,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                tick_volume: b.volume.max(0.0) as u64,
                spread: 0,
            })
            .collect())
    }

    /// Whether `ticket` is an Alpaca position or order seen since start.
    pub fn knows_ticket(&self, ticket: u64) -> bool {
        self.lookup(ticket).is_some()
    }

    fn remember(&self, ticket: u64, r: TicketRef) {
        let mut map = self.tickets.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() > 5000 {
            map.clear();
        }
        map.insert(ticket, r);
    }

    fn lookup(&self, ticket: u64) -> Option<TicketRef> {
        self.tickets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&ticket)
            .cloned()
    }

    /// One Alpaca call, recorded in the outbound-connection log.
    async fn timed<T, F>(&self, data_host: bool, fut: F) -> Result<T, AlpacaError>
    where
        F: Future<Output = Result<T, AlpacaError>>,
    {
        let started = Instant::now();
        let out = fut.await;
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let url = if data_host {
            self.client.data_url()
        } else {
            self.client.trading_url()
        };
        self.egress.record(url, "alpaca", out.is_ok(), ms);
        out
    }

    fn bridge_error(&self, e: AlpacaError) -> BridgeError {
        match e {
            AlpacaError::Transport {
                maybe_sent: true, ..
            } => BridgeError::Timeout(self.timeout_ms),
            AlpacaError::Transport { message, .. } => {
                BridgeError::Transport(format!("cannot connect to Alpaca: {message}"))
            }
            AlpacaError::RateLimited => {
                BridgeError::Transport("cannot connect to Alpaca: rate limited, try again".into())
            }
            AlpacaError::NotPaper => BridgeError::Remote {
                message: "the Alpaca endpoint is not paper trading; orders are refused".into(),
                retcode: RETCODE_TRADE_DISABLED,
            },
            AlpacaError::Auth => BridgeError::Remote {
                message: "Alpaca rejected the API keys".into(),
                retcode: RETCODE_REJECTED,
            },
            AlpacaError::NotFound => BridgeError::Remote {
                message: "Alpaca: not found".into(),
                retcode: RETCODE_INVALID,
            },
            AlpacaError::Api { message, .. } => BridgeError::Remote {
                message: format!("Alpaca: {message}"),
                retcode: RETCODE_REJECTED,
            },
            AlpacaError::Config(m) | AlpacaError::Decode(m) => {
                BridgeError::Protocol(format!("Alpaca: {m}"))
            }
        }
    }

    fn order_result(&self, o: &jarvis_alpaca::Order, what: &str) -> TradeResult {
        let ticket = ticket_for(&o.id);
        self.remember(ticket, TicketRef::Order(o.id.clone()));
        let rejected = o.is_rejected();
        TradeResult {
            retcode: if rejected {
                RETCODE_REJECTED
            } else {
                wire::TRADE_RETCODE_DONE
            },
            retcode_description: format!("Alpaca {what}: {}", o.status),
            order: ticket,
            deal: 0,
            volume: o.filled_qty.filter(|q| *q > 0.0).or(o.qty).unwrap_or(0.0),
            price: o.filled_avg_price.unwrap_or(0.0),
            comment: o.id.clone(),
        }
    }

    /// Submit, resolving an uncertain outcome by the client order id (never by sending again).
    async fn submit(&self, order: NewOrder) -> Result<TradeResult, BridgeError> {
        match self.timed(false, self.client.submit_order(&order)).await {
            Ok(o) => Ok(self.order_result(&o, "order")),
            Err(e) if e.maybe_sent() => {
                match self
                    .timed(
                        false,
                        self.client.order_by_client_id(&order.client_order_id),
                    )
                    .await
                {
                    Ok(Some(o)) => Ok(self.order_result(&o, "order")),
                    _ => Err(BridgeError::Timeout(self.timeout_ms)),
                }
            }
            Err(e) => Err(self.bridge_error(e)),
        }
    }

    async fn position_symbol(&self, ticket: u64) -> Result<String, BridgeError> {
        if let Some(TicketRef::Position(s)) = self.lookup(ticket) {
            return Ok(s);
        }
        self.get_positions(None).await?;
        match self.lookup(ticket) {
            Some(TicketRef::Position(s)) => Ok(s),
            _ => Err(BridgeError::Remote {
                message: format!("no Alpaca position with ticket {ticket}"),
                retcode: RETCODE_INVALID,
            }),
        }
    }

    async fn order_id(&self, ticket: u64) -> Result<String, BridgeError> {
        if let Some(TicketRef::Order(id)) = self.lookup(ticket) {
            return Ok(id);
        }
        self.get_orders(None).await?;
        match self.lookup(ticket) {
            Some(TicketRef::Order(id)) => Ok(id),
            _ => Err(BridgeError::Remote {
                message: format!("no open Alpaca order with ticket {ticket}"),
                retcode: RETCODE_INVALID,
            }),
        }
    }
}

#[async_trait]
impl BrokerReads for AlpacaVenue {
    async fn ping(&self) -> Result<PingInfo, BridgeError> {
        let clock = self
            .timed(false, self.client.clock())
            .await
            .map_err(|e| self.bridge_error(e))?;
        Ok(PingInfo {
            protocol_version: wire::PROTOCOL_VERSION,
            ea_version: "alpaca".to_owned(),
            trade_mode: if self.client.is_paper() {
                TradeMode::Demo
            } else {
                TradeMode::Real
            },
            terminal_connected: true,
            trade_allowed: self.client.is_paper(),
            server_time: iso_unix(&clock.timestamp),
        })
    }

    async fn get_account(&self) -> Result<Account, BridgeError> {
        let a = self
            .timed(false, self.client.account())
            .await
            .map_err(|e| self.bridge_error(e))?;
        let (Some(cash), Some(equity)) = (a.cash, a.equity) else {
            return Err(BridgeError::Protocol(
                "Alpaca: the account has no cash or equity figure".into(),
            ));
        };
        Ok(Account {
            currency: if a.currency.is_empty() {
                "USD".to_owned()
            } else {
                a.currency.clone()
            },
            balance: cash,
            equity,
            profit: a.last_equity.map_or(0.0, |l| equity - l),
            margin: a.initial_margin.unwrap_or(0.0),
            free_margin: a.buying_power.unwrap_or(0.0),
            margin_level: 0.0,
            leverage: 1,
            trade_mode: if self.client.is_paper() {
                TradeMode::Demo
            } else {
                TradeMode::Real
            },
        })
    }

    async fn get_positions(&self, symbol: Option<&str>) -> Result<Positions, BridgeError> {
        let rows = self
            .timed(false, self.client.positions())
            .await
            .map_err(|e| self.bridge_error(e))?;
        let mut positions = Vec::new();
        for p in rows {
            let (Some(qty), Some(avg)) = (p.qty, p.avg_entry_price) else {
                return Err(BridgeError::Protocol(format!(
                    "Alpaca: unreadable position for {}",
                    p.symbol
                )));
            };
            if qty == 0.0 {
                continue;
            }
            let ticket = ticket_for(&p.asset_id);
            self.remember(ticket, TicketRef::Position(p.symbol.clone()));
            if symbol.is_some_and(|s| !s.eq_ignore_ascii_case(&p.symbol)) {
                continue;
            }
            positions.push(wire::Position {
                ticket,
                symbol: p.symbol.clone(),
                side: if p.side == "short" || qty < 0.0 {
                    Side::Sell
                } else {
                    Side::Buy
                },
                volume: qty.abs(),
                price_open: avg,
                price_current: p.current_price.unwrap_or(avg),
                sl: None,
                tp: None,
                profit: p.unrealized_pl.unwrap_or(0.0),
                swap: 0.0,
                magic: 0,
                comment: "alpaca".to_owned(),
                time: 0,
            });
        }
        Ok(Positions { positions })
    }

    async fn get_orders(&self, symbol: Option<&str>) -> Result<Orders, BridgeError> {
        let rows = self
            .timed(false, self.client.open_orders())
            .await
            .map_err(|e| self.bridge_error(e))?;
        let mut orders = Vec::new();
        for o in rows {
            let ticket = ticket_for(&o.id);
            self.remember(ticket, TicketRef::Order(o.id.clone()));
            if symbol.is_some_and(|s| !s.eq_ignore_ascii_case(&o.symbol)) {
                continue;
            }
            // Pending entries only: a resting limit or stop. Market orders fill at once.
            let buy = o.side == "buy";
            let order_type = match (o.order_type.as_str(), buy) {
                ("limit", true) => OrderType::BuyLimit,
                ("limit", false) => OrderType::SellLimit,
                ("stop", true) => OrderType::BuyStop,
                ("stop", false) => OrderType::SellStop,
                ("stop_limit", true) => OrderType::BuyStopLimit,
                ("stop_limit", false) => OrderType::SellStopLimit,
                _ => continue,
            };
            let legs = o.legs.clone().unwrap_or_default();
            let sl = legs
                .iter()
                .find(|l| l.order_type == "stop")
                .and_then(|l| l.stop_price);
            let tp = legs
                .iter()
                .find(|l| l.order_type == "limit")
                .and_then(|l| l.limit_price);
            let qty = o.qty.unwrap_or(0.0);
            orders.push(wire::Order {
                ticket,
                symbol: o.symbol.clone(),
                order_type,
                volume_initial: qty,
                volume_current: (qty - o.filled_qty.unwrap_or(0.0)).max(0.0),
                price_open: o.limit_price.or(o.stop_price).unwrap_or(0.0),
                sl,
                tp,
                time_setup: o.created_at.as_deref().map_or(0, iso_unix),
                comment: "alpaca".to_owned(),
            });
        }
        Ok(Orders { orders })
    }

    async fn get_price(&self, symbol: &str) -> Result<Price, BridgeError> {
        let q = self
            .timed(true, self.client.latest_quote(symbol))
            .await
            .map_err(|e| self.bridge_error(e))?;
        Ok(Price {
            symbol: symbol.to_owned(),
            bid: q.bid,
            ask: q.ask,
            last: 0.0,
            spread_points: ((q.ask - q.bid) / 0.01).round() as i64,
            digits: 2,
            time: q.time,
        })
    }

    async fn get_symbol(&self, symbol: &str) -> Result<SymbolInfo, BridgeError> {
        let asset = self
            .timed(false, self.client.asset(symbol))
            .await
            .map_err(|e| self.bridge_error(e))?;
        let clock = self
            .timed(false, self.client.clock())
            .await
            .map_err(|e| self.bridge_error(e))?;
        let open = clock.is_open || self.extended_hours;
        Ok(SymbolInfo {
            symbol: asset.symbol.clone(),
            description: asset.name.clone(),
            digits: 2,
            point: 0.01,
            volume_min: 1.0,
            volume_max: VOLUME_MAX,
            volume_step: 1.0,
            contract_size: 1.0,
            tick_value: 0.01,
            tick_size: 0.01,
            stops_level: 0,
            // Closed market, a halted or non-tradable asset: no new trade (the gate refuses).
            trade_allowed: asset.tradable
                && asset.status.eq_ignore_ascii_case("active")
                && open
                && self.client.is_paper(),
            currency_profit: "USD".to_owned(),
            spread: None,
            swap_long: None,
            swap_short: None,
            swap_mode: None,
            trade_mode: None,
            sessions: None,
            trade_open_now: Some(clock.is_open),
        })
    }

    async fn get_history(&self, query: HistoryQuery) -> Result<History, BridgeError> {
        // Alpaca reports no realised-today figure: the day change minus open P/L is it.
        let account = self
            .timed(false, self.client.account())
            .await
            .map_err(|e| self.bridge_error(e))?;
        let positions = self.get_positions(None).await?;
        let open: f64 = positions.positions.iter().map(|p| p.profit).sum();
        let realized = match (account.equity, account.last_equity) {
            (Some(e), Some(l)) => (e - l) - open,
            _ => {
                return Err(BridgeError::Protocol(
                    "Alpaca: the account has no equity figures for today's P/L".into(),
                ));
            }
        };
        let now = chrono::Utc::now().timestamp();
        Ok(History {
            deals: vec![Deal {
                ticket: 1,
                order: 0,
                position_id: 0,
                symbol: String::new(),
                deal_type: DealType::Buy,
                entry: DealEntry::Out,
                volume: 0.0,
                price: 0.0,
                profit: (realized * 100.0).round() / 100.0,
                commission: 0.0,
                swap: 0.0,
                time: now.clamp(query.from, query.to.max(query.from)),
                comment: "alpaca realised P/L today".to_owned(),
            }],
            truncated: false,
        })
    }
}

#[async_trait]
impl WriteSink for AlpacaVenue {
    async fn send_write(&self, command: &Command) -> Result<TradeResult, BridgeError> {
        if !self.client.is_paper() {
            return Err(self.bridge_error(AlpacaError::NotPaper));
        }
        match command {
            Command::OrderSend(a) => {
                let shares = whole_shares(a.volume)?;
                let limit = match a.exec {
                    ExecType::Market => None,
                    ExecType::Limit => a.price,
                    ExecType::Stop => {
                        return Err(BridgeError::Remote {
                            message: "stop entry orders are not supported on Alpaca yet".into(),
                            retcode: RETCODE_INVALID,
                        });
                    }
                };
                // Unique per write, and known before sending, so an unclear outcome can be looked up.
                let coid = format!("arc-{}", Uuid::new_v4().simple());
                let mut order = NewOrder::new(
                    &a.symbol,
                    shares,
                    a.side == Side::Buy,
                    limit,
                    a.sl,
                    a.tp,
                    &coid,
                );
                // Alpaca takes extended-hours only for limit day orders without legs.
                order.extended_hours =
                    self.extended_hours && limit.is_some() && order.order_class.is_none();
                self.submit(order).await
            }
            Command::PositionClose(a) => {
                let symbol = self.position_symbol(a.ticket).await?;
                match self
                    .timed(false, self.client.close_position(&symbol, None))
                    .await
                {
                    Ok(o) => Ok(self.order_result(&o, "close")),
                    Err(e) => Err(self.bridge_error(e)),
                }
            }
            Command::PositionClosePartial(a) => {
                let shares = whole_shares(a.volume)?;
                let symbol = self.position_symbol(a.ticket).await?;
                match self
                    .timed(false, self.client.close_position(&symbol, Some(shares)))
                    .await
                {
                    Ok(o) => Ok(self.order_result(&o, "partial close")),
                    Err(e) => Err(self.bridge_error(e)),
                }
            }
            Command::OrderCancel(a) => {
                let id = self.order_id(a.ticket).await?;
                self.timed(false, self.client.cancel_order(&id))
                    .await
                    .map_err(|e| self.bridge_error(e))?;
                Ok(TradeResult {
                    retcode: wire::TRADE_RETCODE_DONE,
                    retcode_description: "Alpaca order cancelled".to_owned(),
                    order: a.ticket,
                    deal: 0,
                    volume: 0.0,
                    price: 0.0,
                    comment: id,
                })
            }
            Command::PositionModify(_) => Err(BridgeError::Remote {
                message: "moving a stop loss or take profit is not supported on Alpaca yet".into(),
                retcode: RETCODE_INVALID,
            }),
            other => Err(BridgeError::Protocol(format!(
                "{} is not a write command",
                other.name()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whole_shares_are_sent() {
        assert_eq!(whole_shares(5.0).unwrap(), 5);
        assert!(whole_shares(0.5).is_err());
        assert!(whole_shares(1.25).is_err());
        assert!(whole_shares(f64::NAN).is_err());
        assert!(whole_shares(0.0).is_err());
    }
}

//! Where a symbol trades (spec-alpaca, ADR-0024): MT5 through the bridge, or Alpaca (US stocks, paper).
//!
//! `Trading` reads its broker through [`BrokerReads`] and writes through `WriteSink`, so one gate, one
//! preview and token flow and one confirm path serve both venues. `BridgeClient` implements both
//! traits as before; `AlpacaVenue` (`alpaca_venue.rs`) implements them for Alpaca.

use async_trait::async_trait;
use jarvis_bridge_client::BridgeClient;
use jarvis_bridge_client::BridgeError;
use jarvis_bridge_client::protocol::{
    Account, History, HistoryQuery, Orders, PingInfo, Positions, Price, SymbolInfo,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VenueKind {
    Mt5,
    Alpaca,
}

impl VenueKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            VenueKind::Mt5 => "mt5",
            VenueKind::Alpaca => "alpaca",
        }
    }
}

/// Everything `Trading` reads from a broker before and after a write.
#[async_trait]
pub trait BrokerReads: Send + Sync {
    async fn ping(&self) -> Result<PingInfo, BridgeError>;
    async fn get_account(&self) -> Result<Account, BridgeError>;
    async fn get_positions(&self, symbol: Option<&str>) -> Result<Positions, BridgeError>;
    async fn get_orders(&self, symbol: Option<&str>) -> Result<Orders, BridgeError>;
    async fn get_price(&self, symbol: &str) -> Result<Price, BridgeError>;
    async fn get_symbol(&self, symbol: &str) -> Result<SymbolInfo, BridgeError>;
    async fn get_history(&self, query: HistoryQuery) -> Result<History, BridgeError>;
}

#[async_trait]
impl BrokerReads for BridgeClient {
    async fn ping(&self) -> Result<PingInfo, BridgeError> {
        BridgeClient::ping(self).await
    }
    async fn get_account(&self) -> Result<Account, BridgeError> {
        BridgeClient::get_account(self).await
    }
    async fn get_positions(&self, symbol: Option<&str>) -> Result<Positions, BridgeError> {
        BridgeClient::get_positions(self, symbol).await
    }
    async fn get_orders(&self, symbol: Option<&str>) -> Result<Orders, BridgeError> {
        BridgeClient::get_orders(self, symbol).await
    }
    async fn get_price(&self, symbol: &str) -> Result<Price, BridgeError> {
        BridgeClient::get_price(self, symbol).await
    }
    async fn get_symbol(&self, symbol: &str) -> Result<SymbolInfo, BridgeError> {
        BridgeClient::get_symbol(self, symbol).await
    }
    async fn get_history(&self, query: HistoryQuery) -> Result<History, BridgeError> {
        BridgeClient::get_history(self, query).await
    }
}

/// Alpaca's half of the router.
pub struct AlpacaRoute {
    pub trading: std::sync::Arc<crate::trading::Trading>,
    pub venue: std::sync::Arc<crate::alpaca_venue::AlpacaVenue>,
    /// Canonical names (upper case) that trade on Alpaca.
    pub symbols: std::collections::BTreeSet<String>,
}

/// Picks the `Trading` instance for a proposal, a token or a ticket. MT5 unless the symbol is an
/// Alpaca symbol or the ticket is an Alpaca position or order.
pub struct VenueRouter {
    mt5: std::sync::Arc<crate::trading::Trading>,
    alpaca: Option<AlpacaRoute>,
}

impl VenueRouter {
    pub fn new(mt5: std::sync::Arc<crate::trading::Trading>, alpaca: Option<AlpacaRoute>) -> Self {
        Self { mt5, alpaca }
    }

    pub fn mt5(&self) -> &std::sync::Arc<crate::trading::Trading> {
        &self.mt5
    }

    pub fn alpaca(&self) -> Option<&AlpacaRoute> {
        self.alpaca.as_ref()
    }

    /// Every venue's trading instance, MT5 first.
    pub fn all(&self) -> Vec<&std::sync::Arc<crate::trading::Trading>> {
        let mut v = vec![&self.mt5];
        if let Some(a) = &self.alpaca {
            v.push(&a.trading);
        }
        v
    }

    pub fn is_alpaca_symbol(&self, symbol: &str) -> bool {
        self.alpaca
            .as_ref()
            .is_some_and(|a| a.symbols.contains(&symbol.trim().to_ascii_uppercase()))
    }

    /// The venue a symbol trades on.
    pub fn venue_of_symbol(&self, symbol: &str) -> VenueKind {
        if self.is_alpaca_symbol(symbol) {
            VenueKind::Alpaca
        } else {
            VenueKind::Mt5
        }
    }

    /// The instance that issued `token` (MT5 when none did: it reports the token unknown).
    pub fn for_token(&self, token: &str) -> &std::sync::Arc<crate::trading::Trading> {
        match &self.alpaca {
            Some(a) if a.trading.holds_token(token) => &a.trading,
            _ => &self.mt5,
        }
    }

    /// Whether `ticket` is an Alpaca position or order, reading Alpaca once if it is not known yet.
    async fn alpaca_ticket(&self, ticket: u64) -> bool {
        let Some(a) = &self.alpaca else { return false };
        if a.venue.knows_ticket(ticket) {
            return true;
        }
        let _ = a.venue.get_positions(None).await;
        let _ = a.venue.get_orders(None).await;
        a.venue.knows_ticket(ticket)
    }

    /// The instance a proposal goes to.
    pub async fn for_action(
        &self,
        action: &jarvis_gate::ProposedAction,
    ) -> &std::sync::Arc<crate::trading::Trading> {
        use jarvis_gate::ProposedAction as A;
        let Some(a) = &self.alpaca else {
            return &self.mt5;
        };
        let alpaca = match action {
            A::Order(o) => self.is_alpaca_symbol(&o.symbol),
            A::Modify(m) => self.alpaca_ticket(m.ticket).await,
            A::Close(c) => match (c.tickets.first(), &c.symbol) {
                (Some(t), _) => self.alpaca_ticket(*t).await,
                (None, Some(s)) => self.is_alpaca_symbol(s),
                // "Close everything" stays on MT5; Alpaca positions are closed by symbol or ticket.
                (None, None) => false,
            },
            A::Cancel(c) => match (c.tickets.first(), &c.symbol) {
                (Some(t), _) => self.alpaca_ticket(*t).await,
                (None, Some(s)) => self.is_alpaca_symbol(s),
                (None, None) => false,
            },
        };
        if alpaca { &a.trading } else { &self.mt5 }
    }
}

//! Nimbus Desk (spec-nimbus-desk, ADR-0023): a trading desk of nine agents behind one master.
//!
//! Friday, the master, takes the trader's command and calls the specialists as tools; this crate
//! holds what each specialist computes, as pure functions over plain inputs. The API gathers the
//! data (MT5 bars and calendar, Finnhub) and calls them, so nothing here does I/O, reads a clock
//! or draws a random number.
//!
//! - Intelligence: [`atlas`] (macro), [`capitol`] (insiders and congress), [`scout`] (movers).
//! - Analysis: [`athena`] (fundamentals and chart structure), [`chartist`] (what to draw),
//!   [`oracle`] (probabilities from past returns).
//! - Execution: [`sentinel`] (sizing checks, approve or veto), then Pilot and Ledger, which live
//!   in the API because they use the trading service and the audit log.
//!
//! The architecture page's pipeline: [`opinion`] turns each member's finding into a vote and
//! [`council`] weighs the votes (spec-architecture-page).
//!
//! Plus the US session clock ([`session`]) and the desk calendar ([`calendar`]).
//!
//! Missing data is `None` (`null` in JSON, shown as "—"), never a made-up number.

pub mod athena;
pub mod atlas;
pub mod calendar;
pub mod capitol;
pub mod chartist;
pub mod council;
pub mod learning;
pub mod opinion;
pub mod oracle;
pub mod scout;
pub mod sentinel;
pub mod session;
mod types;

pub use types::{AgentId, Finding, SeatState, Source, round_to};

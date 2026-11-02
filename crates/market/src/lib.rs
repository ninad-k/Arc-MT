//! Indicators and intraday market views computed from price bars.
//!
//! [`analyze`] turns closed bars of five frames (M5, M15, H1, H4, D1), the live
//! bid/ask, the economic calendar and optional headline sentiment into a [`MarketView`]:
//! a bias with a confidence, an ATR-sized setup when one is justified, and the reasons.
//!
//! The model in short:
//!
//! - Seven factors, each a signal from -1 to +1 times a weight: H4 trend (25), H1 trend
//!   (20), M15 MACD momentum (15), M15 RSI (10), price against the session VWAP (10),
//!   price against the daily pivot (10) and news tone (10). A factor whose inputs are
//!   missing is left out and listed in [`MarketView::notes`].
//! - The score is the sum of the points, clamped to -100..100. At +-20 or more the bias
//!   is bullish or bearish; confidence is the absolute score, capped at 50 when the H4
//!   and H1 trends point in opposite directions.
//! - The view stands aside (no setup) near a high-impact event for the instrument's
//!   currencies, or when the spread is wide compared with the M15 ATR.
//! - A setup is an entry zone at the live price, a stop and two targets sized from the
//!   H1 ATR, with the second target snapped to a swing level when one is close.
//!
//! Everything is a pure function of its input: no clock, no I/O, no randomness. Times
//! are unix seconds on one clock chosen by the caller. See `docs/market-view.md` for
//! the full description.

mod analyze;
mod config;
pub mod indicators;
mod types;
mod view;

pub use analyze::analyze;
pub use config::ViewConfig;
pub use types::{Bar, EventInput, Importance, NewsInput, Timeframe, ViewInput};
pub use view::{
    Bias, DISCLAIMER, Factor, Levels, MarketView, NewsView, Price, Setup, Side, TimeframeSummary,
    Trend, ViewEvent,
};

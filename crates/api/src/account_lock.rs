//! The Desk master account lock (ADR-0025, spec-architecture-page).
//!
//! The master bot (Friday, through Sentinel and Pilot) may trade on MT5 only while MT5 is logged
//! into the expected account. The EA reports the account number in its `ping` reply; this module
//! compares it with the expected one, and the rest of the system only ever learns the result.
//!
//! - The reported number stays in the backend: no API response returns a [`PingInfo`] whole,
//!   and [`LockView`] carries the expected number (which the trader typed) and the state only.
//! - Nothing here goes to the language model.
//! - `0` turns the lock off. Alpaca trades are not MT5 trades and are not affected.
//!
//! [`PingInfo`]: jarvis_bridge_client::protocol::PingInfo

use std::sync::RwLock;

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockState {
    /// MT5 reports the expected account.
    Locked,
    /// MT5 reports another account.
    Mismatch,
    /// The EA does not report the account (older than 0.7.0) or MT5 is not reachable.
    Unverified,
    /// No lock (expected account 0).
    Off,
}

impl LockState {
    pub fn allows_trading(self) -> bool {
        matches!(self, LockState::Locked | LockState::Off)
    }
}

/// What the browser may see.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LockView {
    /// The number the trader set (0 = off). Never the number MT5 reported.
    pub expected: u64,
    pub masked: String,
    pub broker: String,
    pub state: LockState,
    /// Plain words for the chip and the refusal.
    pub message: String,
}

/// `••••7904`: the last four digits.
pub fn mask(login: u64) -> String {
    if login == 0 {
        return "off".into();
    }
    let s = login.to_string();
    let tail: String = s.chars().skip(s.len().saturating_sub(4)).collect();
    format!("••••{tail}")
}

pub fn compare(expected: u64, reported: Option<u64>) -> LockState {
    match (expected, reported) {
        (0, _) => LockState::Off,
        (_, None) => LockState::Unverified,
        (e, Some(r)) if e == r => LockState::Locked,
        _ => LockState::Mismatch,
    }
}

pub struct AccountLock {
    pool: SqlitePool,
    current: RwLock<(u64, String)>,
}

impl AccountLock {
    pub fn new(pool: SqlitePool, default_login: u64, default_broker: String) -> Self {
        Self {
            pool,
            current: RwLock::new((default_login, default_broker)),
        }
    }

    /// Load the trader's setting, if one was saved.
    pub async fn load(&self) -> anyhow::Result<()> {
        if let Some(r) = sqlx::query("SELECT expected_login, broker FROM account_lock WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?
        {
            let login: i64 = r.try_get("expected_login")?;
            *self.current.write().unwrap_or_else(|p| p.into_inner()) =
                (u64::try_from(login).unwrap_or(0), r.try_get("broker")?);
        }
        Ok(())
    }

    pub fn expected(&self) -> (u64, String) {
        self.current
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Save the expected account (0 turns the lock off).
    pub async fn set(&self, login: u64, broker: &str, actor: Option<&str>) -> anyhow::Result<()> {
        let stored = i64::try_from(login)?;
        sqlx::query(
            "INSERT INTO account_lock (id, expected_login, broker, updated_at, actor)
             VALUES (1, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET expected_login = excluded.expected_login,
                broker = excluded.broker, updated_at = excluded.updated_at, actor = excluded.actor",
        )
        .bind(stored)
        .bind(broker)
        .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))
        .bind(actor)
        .execute(&self.pool)
        .await?;
        *self.current.write().unwrap_or_else(|p| p.into_inner()) = (login, broker.to_owned());
        Ok(())
    }

    /// The view for the browser, given what MT5 reported (`None`: unknown).
    pub fn view(&self, reported: Option<u64>) -> LockView {
        let (expected, broker) = self.expected();
        let state = compare(expected, reported);
        let message = match state {
            LockState::Locked => format!("Locked to {broker} {}.", mask(expected)),
            LockState::Mismatch => format!(
                "MT5 is logged into a different account than {broker} {}. The Desk will not trade on MT5.",
                mask(expected)
            ),
            LockState::Unverified => format!(
                "Cannot confirm MT5 is on {broker} {}: the EA does not report the account (update it to 0.7.0) or MT5 is not reachable. The Desk will not trade on MT5.",
                mask(expected)
            ),
            LockState::Off => "No master account lock.".into(),
        };
        LockView {
            expected,
            masked: mask(expected),
            broker,
            state,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_and_compares() {
        assert_eq!(mask(53_107_904), "••••7904");
        assert_eq!(mask(12), "••••12");
        assert_eq!(mask(0), "off");
        assert_eq!(compare(53_107_904, Some(53_107_904)), LockState::Locked);
        assert_eq!(compare(53_107_904, Some(1)), LockState::Mismatch);
        assert_eq!(compare(53_107_904, None), LockState::Unverified);
        assert_eq!(compare(0, None), LockState::Off);
        assert!(!LockState::Unverified.allows_trading());
        assert!(LockState::Off.allows_trading());
    }
}

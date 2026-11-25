use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The nine seats of the desk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentId {
    /// The master: talks to the trader, routes to the others, does no specialist work.
    Friday,
    Atlas,
    Capitol,
    Scout,
    Athena,
    Chartist,
    Oracle,
    Sentinel,
    Pilot,
    Ledger,
}

impl AgentId {
    /// Every seat, the master first, then the specialists by group.
    pub const ALL: [AgentId; 10] = [
        AgentId::Friday,
        AgentId::Atlas,
        AgentId::Capitol,
        AgentId::Scout,
        AgentId::Athena,
        AgentId::Chartist,
        AgentId::Oracle,
        AgentId::Sentinel,
        AgentId::Pilot,
        AgentId::Ledger,
    ];

    /// The nine specialists around the orb.
    pub const SPECIALISTS: [AgentId; 9] = [
        AgentId::Atlas,
        AgentId::Capitol,
        AgentId::Scout,
        AgentId::Athena,
        AgentId::Chartist,
        AgentId::Oracle,
        AgentId::Sentinel,
        AgentId::Pilot,
        AgentId::Ledger,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            AgentId::Friday => "friday",
            AgentId::Atlas => "atlas",
            AgentId::Capitol => "capitol",
            AgentId::Scout => "scout",
            AgentId::Athena => "athena",
            AgentId::Chartist => "chartist",
            AgentId::Oracle => "oracle",
            AgentId::Sentinel => "sentinel",
            AgentId::Pilot => "pilot",
            AgentId::Ledger => "ledger",
        }
    }

    pub fn parse(s: &str) -> Option<AgentId> {
        AgentId::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// The seat's title on the HUD.
    pub const fn title(self) -> &'static str {
        match self {
            AgentId::Friday => "Master",
            AgentId::Atlas => "Macro",
            AgentId::Capitol => "Smart money",
            AgentId::Scout => "Recon",
            AgentId::Athena => "Analyst",
            AgentId::Chartist => "Technician",
            AgentId::Oracle => "Quant",
            AgentId::Sentinel => "Risk officer",
            AgentId::Pilot => "Execution",
            AgentId::Ledger => "The book",
        }
    }

    /// `"intelligence"`, `"analysis"`, `"execution"` or `"master"`.
    pub const fn group(self) -> &'static str {
        match self {
            AgentId::Friday => "master",
            AgentId::Atlas | AgentId::Capitol | AgentId::Scout => "intelligence",
            AgentId::Athena | AgentId::Chartist | AgentId::Oracle => "analysis",
            AgentId::Sentinel | AgentId::Pilot | AgentId::Ledger => "execution",
        }
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where one input of a finding came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// `"mt5"`, `"finnhub"`, `"fed schedule"`, ...
    pub name: String,
    /// What was read, e.g. `"NVDA D1 bars"`.
    pub what: String,
    /// ISO 8601 UTC time the data is from, when known.
    pub at: Option<String>,
}

impl Source {
    pub fn new(name: impl Into<String>, what: impl Into<String>, at: Option<String>) -> Self {
        Self {
            name: name.into(),
            what: what.into(),
            at,
        }
    }
}

/// What a specialist found. Every number in `data` comes from a source; a `null` means no data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub agent: AgentId,
    /// One or two plain sentences Friday can build on.
    pub summary: String,
    /// The agent's figures, typed per agent.
    pub data: Value,
    pub sources: Vec<Source>,
    /// What was missing, in plain words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl Finding {
    pub fn new(agent: AgentId, summary: impl Into<String>, data: Value) -> Self {
        Self {
            agent,
            summary: summary.into(),
            data,
            sources: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn with_source(mut self, source: Source) -> Self {
        self.sources.push(source);
        self
    }

    pub fn with_sources(mut self, sources: impl IntoIterator<Item = Source>) -> Self {
        self.sources.extend(sources);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// A seat's state on the HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatState {
    Idle,
    Working,
    Done,
    Error,
    /// Turned off in config or missing its data source (for example no Finnhub key).
    Offline,
}

/// Round to `decimals` places, keeping non-finite values out of the output.
pub fn round_to(value: f64, decimals: u32) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let f = 10f64.powi(decimals as i32);
    Some((value * f).round() / f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for a in AgentId::ALL {
            assert_eq!(AgentId::parse(a.as_str()), Some(a));
            let json = serde_json::to_string(&a).unwrap();
            assert_eq!(json, format!("\"{}\"", a.as_str()));
        }
        assert_eq!(AgentId::parse("nobody"), None);
        assert_eq!(AgentId::SPECIALISTS.len(), 9);
    }

    #[test]
    fn rounding_drops_non_finite() {
        assert_eq!(round_to(1.23456, 2), Some(1.23));
        assert_eq!(round_to(f64::NAN, 2), None);
        assert_eq!(round_to(f64::INFINITY, 2), None);
    }
}

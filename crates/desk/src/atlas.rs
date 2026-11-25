//! Atlas (Macro): the Fed, rates and the state of the market.
//!
//! Inputs, each optional: the next FOMC decision day, upcoming high-impact US events from the MT5
//! calendar, today's moves of the index symbols, and the tone of general market headlines.

use jarvis_market::{EventInput, Importance};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{AgentId, Finding, Source};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexMove {
    pub symbol: String,
    pub change_pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tone {
    /// -1 to +1.
    pub score: f64,
    pub label: String,
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MacroInput {
    /// `YYYY-MM-DD` and where it came from.
    pub next_fomc: Option<(String, String)>,
    /// Days from today to the next FOMC decision.
    pub days_to_fomc: Option<i64>,
    /// Upcoming US events (any importance; only high ones are summarised).
    pub events: Vec<EventInput>,
    /// Unix seconds now, on the events' clock, to say how far away they are.
    pub now: i64,
    pub indices: Vec<IndexMove>,
    pub tone: Option<Tone>,
}

pub fn macro_view(input: &MacroInput, at: Option<String>) -> Finding {
    let mut parts = Vec::new();
    match (&input.next_fomc, input.days_to_fomc) {
        (Some((d, _)), Some(0)) => parts.push(format!("FOMC decision today ({d})")),
        (Some((d, _)), Some(n)) => parts.push(format!("next FOMC decision {d}, in {n} days")),
        _ => {}
    }
    let high: Vec<&EventInput> = input
        .events
        .iter()
        .filter(|e| e.importance == Importance::High && e.time >= input.now)
        .collect();
    if let Some(e) = high.first() {
        let hours = (e.time - input.now) as f64 / 3600.0;
        parts.push(format!(
            "next high-impact US event: {} in {:.1} hours",
            e.title, hours
        ));
    }
    let moves: Vec<String> = input
        .indices
        .iter()
        .filter_map(|m| m.change_pct.map(|c| format!("{} {c:+.2}%", m.symbol)))
        .collect();
    if !moves.is_empty() {
        parts.push(format!("indices {}", moves.join(", ")));
    }
    if let Some(t) = &input.tone {
        parts.push(format!(
            "headline tone {} over {} stories",
            t.label, t.count
        ));
    }
    let summary = if parts.is_empty() {
        "No macro data came back.".to_owned()
    } else {
        let mut s = parts.join("; ");
        if let Some(first) = s.get(0..1) {
            s = first.to_uppercase() + &s[1..];
        }
        format!("{s}.")
    };
    let events: Vec<_> = high
        .iter()
        .take(5)
        .map(|e| {
            json!({ "time": e.time, "title": e.title, "currency": e.currency,
                         "hours_away": crate::round_to((e.time - input.now) as f64 / 3600.0, 1) })
        })
        .collect();
    let mut f = Finding::new(
        AgentId::Atlas,
        summary,
        json!({
            "next_fomc": input.next_fomc.as_ref().map(|(d, src)| json!({"date": d, "source": src})),
            "days_to_fomc": input.days_to_fomc,
            "high_impact_events": events,
            "indices": input.indices,
            "tone": input.tone,
        }),
    );
    if let Some((_, src)) = &input.next_fomc {
        f = f.with_source(Source::new(src.clone(), "FOMC dates", None));
    }
    f = f.with_source(Source::new("mt5", "economic calendar", at.clone()));
    if !input.indices.is_empty() {
        f = f.with_source(Source::new("mt5", "index daily bars", at.clone()));
    }
    match &input.tone {
        Some(_) => f = f.with_source(Source::new("finnhub", "general market news", at)),
        None => f = f.with_note("No headline tone (no news key)."),
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_what_it_has() {
        let input = MacroInput {
            next_fomc: Some(("2026-12-09".into(), "fed schedule".into())),
            days_to_fomc: Some(15),
            events: vec![
                EventInput {
                    time: 7200,
                    currency: "USD".into(),
                    title: "Core PCE".into(),
                    importance: Importance::High,
                },
                EventInput {
                    time: 3600,
                    currency: "USD".into(),
                    title: "Jobless claims".into(),
                    importance: Importance::Moderate,
                },
            ],
            now: 0,
            indices: vec![IndexMove {
                symbol: "US500".into(),
                change_pct: Some(-0.42),
            }],
            tone: Some(Tone {
                score: 0.2,
                label: "slightly positive".into(),
                count: 12,
            }),
        };
        let f = macro_view(&input, None);
        assert!(
            f.summary
                .starts_with("Next FOMC decision 2026-12-09, in 15 days")
        );
        assert!(f.summary.contains("Core PCE in 2.0 hours"));
        assert!(!f.summary.contains("Jobless"));
        assert!(f.summary.contains("US500 -0.42%"));
        assert_eq!(f.data["high_impact_events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn nothing_in_nothing_out() {
        let f = macro_view(&MacroInput::default(), None);
        assert_eq!(f.summary, "No macro data came back.");
        assert!(f.data["next_fomc"].is_null());
        assert!(f.data["tone"].is_null());
    }
}

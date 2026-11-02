//! A small finance word list that gives a headline a tone from -1 to +1.
//!
//! This is a light signal, not a model. The rules:
//!
//! - Matching is whole-word and case-insensitive. "gain" is in the list, "gaining" is not:
//!   only the forms written out below count, which keeps false hits down (the list covers the
//!   common headline forms such as "rose", "fell", "surged").
//! - Multi-word phrases ("record high", "rate cut") match as a unit and win over their parts.
//!   Hyphens split words, so "sell-off" and "sell off" are the same phrase.
//! - A negator within three words before a match flips it: "no rate cut" is positive,
//!   "fails to rally" is negative. A sentence end stops the look-back.
//! - Tone is `(pos - neg) / (pos + neg)`, and 0 when nothing matches.
//!
//! The words are read as good or bad for the subject's currency or price. "hawkish" and rate
//! hikes are positive, "dovish" and rate cuts negative. A currency pair gets its direction
//! from the relevance module; cross-asset effects (a rate cut is good for gold and stocks) are
//! out of reach for a word list.

use std::sync::OnceLock;

use crate::text::{BREAK, matches_at, tokens};

const POSITIVE: &[&str] = &[
    "beat",
    "beats",
    "surge",
    "surges",
    "surged",
    "rally",
    "rallies",
    "rallied",
    "gain",
    "gains",
    "gained",
    "rise",
    "rises",
    "rose",
    "jump",
    "jumps",
    "jumped",
    "climb",
    "climbs",
    "climbed",
    "soar",
    "soars",
    "soared",
    "rebound",
    "rebounds",
    "rebounded",
    "strong",
    "stronger",
    "strength",
    "upbeat",
    "bullish",
    "record high",
    "upgrade",
    "upgrades",
    "upgraded",
    "hawkish",
    "rate hike",
    "rate hikes",
    "optimism",
    "optimistic",
    "recovery",
    "boost",
    "boosts",
    "boosted",
    "outperform",
    "outperforms",
];

const NEGATIVE: &[&str] = &[
    "miss",
    "misses",
    "missed",
    "plunge",
    "plunges",
    "plunged",
    "slump",
    "slumps",
    "slumped",
    "fall",
    "falls",
    "fell",
    "drop",
    "drops",
    "dropped",
    "tumble",
    "tumbles",
    "tumbled",
    "sink",
    "sinks",
    "sank",
    "slide",
    "slides",
    "slid",
    "decline",
    "declines",
    "declined",
    "weak",
    "weaker",
    "weakness",
    "bearish",
    "record low",
    "downgrade",
    "downgrades",
    "downgraded",
    "dovish",
    "rate cut",
    "rate cuts",
    "cut",
    "cuts",
    "recession",
    "crisis",
    "selloff",
    "sell off",
    "fear",
    "fears",
    "war",
    "tariff",
    "tariffs",
    "default",
    "slowdown",
    "contraction",
    "turmoil",
];

const NEGATORS: &[&str] = &[
    "not",
    "no",
    "never",
    "without",
    "less",
    "cannot",
    "isn't",
    "wasn't",
    "aren't",
    "weren't",
    "doesn't",
    "didn't",
    "don't",
    "won't",
    "hasn't",
    "haven't",
    "hadn't",
    "fails to",
    "failed to",
    "fail to",
    "unable to",
];

/// How far back, in words, a negator can sit and still flip a match.
const NEGATION_WINDOW: usize = 3;

struct Entry {
    words: Vec<String>,
    positive: bool,
}

struct Tables {
    entries: Vec<Entry>,
    negators: Vec<Vec<String>>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut entries: Vec<Entry> = POSITIVE
            .iter()
            .map(|w| (w, true))
            .chain(NEGATIVE.iter().map(|w| (w, false)))
            .map(|(w, positive)| Entry {
                words: tokens(w),
                positive,
            })
            .collect();
        // Longest phrase first, so "rate cut" wins over "cut".
        entries.sort_by_key(|e| std::cmp::Reverse(e.words.len()));
        Tables {
            entries,
            negators: NEGATORS.iter().map(|w| tokens(w)).collect(),
        }
    })
}

/// Positive and negative matches in a text, after negation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub positive: u32,
    pub negative: u32,
}

impl Counts {
    /// `(pos - neg) / (pos + neg)`, or 0 when there are no matches.
    pub fn tone(self) -> f64 {
        let total = self.positive + self.negative;
        if total == 0 {
            0.0
        } else {
            (f64::from(self.positive) - f64::from(self.negative)) / f64::from(total)
        }
    }
}

fn is_negated(toks: &[String], at: usize) -> bool {
    let t = tables();
    for j in (at.saturating_sub(NEGATION_WINDOW)..at).rev() {
        if toks[j] == BREAK {
            return false;
        }
        // A negator "ends" at j; multi-word ones such as "fails to" end on their last word.
        let hit = t
            .negators
            .iter()
            .any(|n| j + 1 >= n.len() && matches_at(toks, j + 1 - n.len(), n));
        if hit {
            return true;
        }
    }
    false
}

/// Count the positive and negative terms in `text`.
pub fn count(text: &str) -> Counts {
    let toks = tokens(text);
    let t = tables();
    let mut counts = Counts::default();
    let mut i = 0;
    while i < toks.len() {
        match t.entries.iter().find(|e| matches_at(&toks, i, &e.words)) {
            Some(e) => {
                if e.positive != is_negated(&toks, i) {
                    counts.positive += 1;
                } else {
                    counts.negative += 1;
                }
                i += e.words.len();
            }
            None => i += 1,
        }
    }
    counts
}

/// Tone of one text, -1..1.
pub fn tone(text: &str) -> f64 {
    count(text).tone()
}

/// Tone of a headline with its summary. The title counts double: it is the part written to
/// say what happened, while summaries drift into background.
pub fn headline_tone(title: &str, summary: &str) -> f64 {
    let t = count(title);
    let s = count(summary);
    Counts {
        positive: t.positive * 2 + s.positive,
        negative: t.negative * 2 + s.negative,
    }
    .tone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_positive_and_negative() {
        assert_eq!(tone("Euro rallies as data beats forecasts"), 1.0);
        assert_eq!(tone("Stocks plunge on recession fears"), -1.0);
    }

    #[test]
    fn nothing_matches_is_zero() {
        assert_eq!(tone("ECB publishes its meeting calendar"), 0.0);
        assert_eq!(tone(""), 0.0);
    }

    #[test]
    fn mixed_is_the_balance() {
        // rally (+), plunge (-), fears (-): (1 - 2) / 3
        let t = tone("Gold rally fades as stocks plunge on fears");
        assert!((t - (-1.0 / 3.0)).abs() < 1e-12, "{t}");
    }

    #[test]
    fn whole_words_only() {
        // "gain" is listed, "gaining" and "gainful" are not.
        assert_eq!(tone("dollar gain"), 1.0);
        assert_eq!(tone("dollar gaining ground"), 0.0);
        assert_eq!(tone("unfallen"), 0.0);
        assert_eq!(tone("Warsaw index"), 0.0);
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(tone("DOLLAR SURGES"), 1.0);
    }

    #[test]
    fn hyphenated_phrases() {
        assert_eq!(tone("Global sell-off deepens"), -1.0);
        assert_eq!(tone("Global selloff deepens"), -1.0);
        assert_eq!(tone("Dow hits record high"), 1.0);
    }

    #[test]
    fn phrase_beats_its_parts() {
        // One phrase match, not "rate" + "cut".
        let c = count("Traders price a rate cut");
        assert_eq!(
            c,
            Counts {
                positive: 0,
                negative: 1
            }
        );
    }

    #[test]
    fn central_bank_stance() {
        assert_eq!(tone("Fed turns hawkish"), 1.0);
        assert_eq!(tone("ECB sounds dovish"), -1.0);
    }

    #[test]
    fn negation_flips_within_three_words() {
        assert_eq!(tone("Sterling did not rally"), -1.0);
        assert_eq!(tone("No rate cut expected"), 1.0);
        assert_eq!(tone("Yen never weak"), 1.0);
        assert_eq!(tone("Less hawkish tone from the Fed"), -1.0);
        assert_eq!(tone("Company fails to beat estimates"), -1.0);
        assert_eq!(tone("Data isn't weak"), 1.0);
    }

    #[test]
    fn negation_out_of_reach_does_not_flip() {
        // Four words between the negator and the match.
        assert_eq!(tone("not at all what the market expected rally"), 1.0);
    }

    #[test]
    fn negation_stops_at_sentence_end() {
        assert_eq!(tone("Nothing is certain, not so. Dollar rallies"), 1.0);
        assert_eq!(tone("No. Dollar rallies"), 1.0);
    }

    #[test]
    fn title_counts_double() {
        // Title: one positive (x2). Summary: one negative.
        let t = headline_tone("Dollar rallies", "Traders fear a slowdown");
        // positive 2, negative 2 -> 0
        assert_eq!(t, 0.0);
        let t = headline_tone("Dollar rallies", "Analysts fear");
        assert!((t - (2.0 - 1.0) / 3.0).abs() < 1e-12);
    }
}

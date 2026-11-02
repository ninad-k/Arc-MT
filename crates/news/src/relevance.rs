//! Which headlines are about an instrument, and which way they point for a pair.
//!
//! Relevance is a whole-word match of any query term in the title or summary. Direction only
//! applies to a pair with a base and a quote: a headline about the base keeps its tone, one
//! about the quote flips it, one about both counts as 0 (the effects cancel in a word list's
//! eyes), and one that names the pair itself ("EUR/USD") keeps its tone.

use std::sync::OnceLock;

use crate::text::{contains_phrase, matches_at, tokens};

/// Names a headline uses for each currency or base asset: the code, the currency name and its
/// central bank. "dollar" alone means USD; "Australian dollar" and friends are listed as
/// longer phrases and win over it. The extra codes (gold, silver, two coins) let the same
/// direction logic serve XAUUSD, XAGUSD, BTCUSD and ETHUSD.
const TERMS: &[(&str, &[&str])] = &[
    (
        "USD",
        &[
            "USD",
            "US dollar",
            "dollar",
            "greenback",
            "Fed",
            "Federal Reserve",
        ],
    ),
    (
        "EUR",
        &["EUR", "euro", "eurozone", "ECB", "European Central Bank"],
    ),
    (
        "GBP",
        &[
            "GBP",
            "sterling",
            "pound",
            "British pound",
            "BoE",
            "Bank of England",
        ],
    ),
    ("JPY", &["JPY", "yen", "BoJ", "Bank of Japan"]),
    (
        "CHF",
        &["CHF", "franc", "Swiss franc", "SNB", "Swiss National Bank"],
    ),
    (
        "AUD",
        &[
            "AUD",
            "Aussie",
            "Australian dollar",
            "RBA",
            "Reserve Bank of Australia",
        ],
    ),
    (
        "CAD",
        &["CAD", "loonie", "Canadian dollar", "BoC", "Bank of Canada"],
    ),
    (
        "NZD",
        &[
            "NZD",
            "kiwi",
            "New Zealand dollar",
            "RBNZ",
            "Reserve Bank of New Zealand",
        ],
    ),
    (
        "CNH",
        &[
            "CNH",
            "CNY",
            "yuan",
            "renminbi",
            "PBOC",
            "People's Bank of China",
        ],
    ),
    ("XAU", &["XAU", "gold", "bullion"]),
    ("XAG", &["XAG", "silver"]),
    ("BTC", &["BTC", "bitcoin"]),
    ("ETH", &["ETH", "ether", "ethereum"]),
];

/// Names a headline uses for `code` (case-insensitive). Empty for an unknown code.
pub fn currency_terms(code: &str) -> &'static [&'static str] {
    let code = code.trim();
    TERMS
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(code))
        .map_or(&[][..], |(_, terms)| *terms)
}

/// Does the title or summary contain any of `terms` as whole words (case-insensitive)?
pub fn is_relevant(terms: &[String], title: &str, summary: &str) -> bool {
    let mut toks = tokens(title);
    toks.push(crate::text::BREAK.to_string());
    toks.extend(tokens(summary));
    terms.iter().any(|t| {
        let phrase = tokens(t);
        contains_phrase(&toks, &phrase)
    })
}

/// What a headline names, out of a pair's two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mentions {
    pub base: bool,
    pub quote: bool,
    /// The pair itself was named ("EURUSD", "EUR/USD").
    pub pair: bool,
}

struct Entry {
    words: Vec<String>,
    code: &'static str,
}

fn entries() -> &'static [Entry] {
    static TABLE: OnceLock<Vec<Entry>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut all: Vec<Entry> = TERMS
            .iter()
            .flat_map(|(code, terms)| {
                terms.iter().map(|t| Entry {
                    words: tokens(t),
                    code,
                })
            })
            .collect();
        // Longest first: "Australian dollar" must be seen before "dollar".
        all.sort_by_key(|e| std::cmp::Reverse(e.words.len()));
        all
    })
}

fn scan(text: &str, base: &str, quote: &str) -> Mentions {
    let toks = tokens(text);
    let (b, q) = (base.to_ascii_lowercase(), quote.to_ascii_lowercase());
    let joined = format!("{b}{q}");
    let mut m = Mentions::default();
    let mut i = 0;
    while i < toks.len() {
        let tok = toks[i].as_str();
        if !b.is_empty() && !q.is_empty() {
            if tok == joined {
                m.pair = true;
                i += 1;
                continue;
            }
            if tok == b && toks.get(i + 1).is_some_and(|n| *n == q) {
                m.pair = true;
                i += 2;
                continue;
            }
        }
        if let Some(e) = entries().iter().find(|e| matches_at(&toks, i, &e.words)) {
            if e.code.eq_ignore_ascii_case(base) {
                m.base = true;
            } else if e.code.eq_ignore_ascii_case(quote) {
                m.quote = true;
            }
            i += e.words.len();
            continue;
        }
        // A code that is not in the table still counts by its own letters.
        if !b.is_empty() && tok == b {
            m.base = true;
        } else if !q.is_empty() && tok == q {
            m.quote = true;
        }
        i += 1;
    }
    m
}

/// Which side of a pair a headline is about. The title decides; the summary is only read when
/// the title names neither side, because summaries mention the other currency in passing.
pub fn mentions(title: &str, summary: &str, base: &str, quote: &str) -> Mentions {
    let m = scan(title, base, quote);
    if m.base || m.quote || m.pair {
        m
    } else {
        scan(summary, base, quote)
    }
}

/// Apply the pair rule to a tone: base keeps it, quote flips it, both cancel, neither (or the
/// pair named directly) keeps it.
pub fn directed_tone(tone: f64, m: Mentions) -> f64 {
    if m.pair {
        return tone;
    }
    match (m.base, m.quote) {
        (true, true) => 0.0,
        (false, true) => -tone,
        _ => tone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn relevance_is_whole_word_and_case_insensitive() {
        let t = terms(&["gold", "XAUUSD"]);
        assert!(is_relevant(&t, "GOLD hits a record", ""));
        assert!(is_relevant(&t, "Markets wrap", "Spot gold edged up"));
        assert!(is_relevant(&t, "Gold's rally", ""));
        assert!(!is_relevant(&t, "Goldman Sachs raises targets", ""));
        assert!(!is_relevant(&t, "Marigold festival", ""));
        assert!(!is_relevant(&[], "gold", "gold"));
    }

    #[test]
    fn multi_word_terms() {
        let t = terms(&["Federal Reserve"]);
        assert!(is_relevant(&t, "The Federal Reserve holds", ""));
        assert!(!is_relevant(&t, "Federal budget and reserve ratios", ""));
    }

    #[test]
    fn terms_do_not_match_across_title_and_summary() {
        let t = terms(&["rate decision"]);
        assert!(!is_relevant(
            &t,
            "Awaiting the rate",
            "decision on Thursday"
        ));
    }

    #[test]
    fn currency_terms_cover_the_majors() {
        for code in [
            "USD", "EUR", "GBP", "JPY", "CHF", "AUD", "CAD", "NZD", "CNH",
        ] {
            let list = currency_terms(code);
            assert!(list.iter().any(|t| t.eq_ignore_ascii_case(code)), "{code}");
            assert!(list.len() >= 3, "{code}");
        }
        assert!(currency_terms("eur").contains(&"euro"));
        assert!(currency_terms("ZZZ").is_empty());
    }

    fn eurusd(title: &str) -> Mentions {
        mentions(title, "", "EUR", "USD")
    }

    #[test]
    fn base_quote_and_both() {
        assert_eq!(
            eurusd("Euro jumps after ECB"),
            Mentions {
                base: true,
                ..Default::default()
            }
        );
        assert_eq!(
            eurusd("Greenback surges on Fed"),
            Mentions {
                quote: true,
                ..Default::default()
            }
        );
        let both = eurusd("Euro slips as dollar firms");
        assert!(both.base && both.quote);
    }

    #[test]
    fn pair_named_directly() {
        assert!(eurusd("EUR/USD rises to 1.09").pair);
        assert!(eurusd("EURUSD rises").pair);
        assert!(eurusd("EUR USD outlook").pair);
    }

    #[test]
    fn other_dollars_are_not_usd() {
        let m = mentions("Australian dollar jumps", "", "AUD", "USD");
        assert!(m.base && !m.quote);
        let m = mentions("Canadian dollar slips", "", "EUR", "USD");
        assert!(!m.quote && !m.base);
    }

    #[test]
    fn summary_is_a_fallback_only() {
        // Title names the euro, so the dollar in the summary is ignored.
        let m = mentions("Euro rises", "while the dollar slid", "EUR", "USD");
        assert!(m.base && !m.quote);
        // Title names neither, so the summary decides.
        let m = mentions("Markets wrap", "the dollar slid", "EUR", "USD");
        assert!(m.quote && !m.base);
    }

    #[test]
    fn direction_rules() {
        let base = Mentions {
            base: true,
            ..Default::default()
        };
        let quote = Mentions {
            quote: true,
            ..Default::default()
        };
        let both = Mentions {
            base: true,
            quote: true,
            pair: false,
        };
        assert_eq!(directed_tone(0.5, base), 0.5);
        assert_eq!(directed_tone(0.5, quote), -0.5);
        assert_eq!(directed_tone(0.5, both), 0.0);
        assert_eq!(directed_tone(0.5, Mentions::default()), 0.5);
        assert_eq!(directed_tone(-0.5, Mentions { pair: true, ..both }), -0.5);
    }

    #[test]
    fn central_bank_names_map_to_their_currency() {
        assert!(mentions("ECB signals pause", "", "EUR", "USD").base);
        assert!(mentions("Fed signals pause", "", "EUR", "USD").quote);
        assert!(mentions("BoJ intervenes", "", "USD", "JPY").quote);
        assert!(mentions("BoE hikes", "", "GBP", "USD").base);
    }

    #[test]
    fn gold_against_the_dollar() {
        assert!(mentions("Gold climbs", "", "XAU", "USD").base);
        assert!(mentions("Dollar climbs", "", "XAU", "USD").quote);
    }
}

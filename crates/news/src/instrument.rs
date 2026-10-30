//! Rough instrument classes, used to choose a news category or topic.

use crate::NewsQuery;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Crypto,
    Metal,
    FxPair,
    Energy,
    /// Indices and anything else.
    Other,
}

const CRYPTO: &[&str] = &[
    "BTC", "ETH", "LTC", "XRP", "BCH", "ADA", "DOT", "SOL", "DOGE", "BNB", "LINK", "XLM", "TRX",
    "AVAX",
];
const METALS: &[&str] = &["XAU", "XAG", "XPT", "XPD"];
const FIAT: &[&str] = &[
    "USD", "EUR", "GBP", "JPY", "CHF", "AUD", "CAD", "NZD", "CNH", "CNY", "SEK", "NOK", "DKK",
    "PLN", "CZK", "HUF", "TRY", "ZAR", "MXN", "SGD", "HKD",
];
const ENERGY: &[&str] = &["OIL", "BRENT", "WTI", "XTI", "XBR", "NGAS", "NATGAS", "XNG"];

/// Base and quote as upper-case codes: from the query, else a six-letter canonical name.
pub(crate) fn parts(q: &NewsQuery) -> Option<(String, String)> {
    if let (Some(b), Some(qt)) = (q.base.as_deref(), q.quote.as_deref()) {
        let (b, qt) = (b.trim(), qt.trim());
        if !b.is_empty() && !qt.is_empty() {
            return Some((b.to_ascii_uppercase(), qt.to_ascii_uppercase()));
        }
    }
    let c = q.canonical.trim().to_ascii_uppercase();
    (c.len() == 6 && c.is_ascii()).then(|| (c[..3].to_string(), c[3..].to_string()))
}

/// The coin code of a crypto instrument ("BTC" for BTCUSD or an explicit base).
pub(crate) fn crypto_base(q: &NewsQuery) -> Option<String> {
    let canonical = q.canonical.trim().to_ascii_uppercase();
    let base = parts(q).map(|(b, _)| b);
    CRYPTO
        .iter()
        .find(|c| base.as_deref() == Some(**c) || canonical.starts_with(**c))
        .map(|c| (*c).to_string())
}

pub(crate) fn classify(q: &NewsQuery) -> Kind {
    let canonical = q.canonical.trim().to_ascii_uppercase();
    let parts = parts(q);
    let base = parts.as_ref().map(|(b, _)| b.as_str());
    let starts = |list: &[&str]| {
        list.iter()
            .any(|c| base == Some(*c) || canonical.starts_with(c))
    };
    if starts(CRYPTO) {
        Kind::Crypto
    } else if starts(METALS) {
        Kind::Metal
    } else if parts
        .as_ref()
        .is_some_and(|(b, qt)| FIAT.contains(&b.as_str()) && FIAT.contains(&qt.as_str()))
    {
        Kind::FxPair
    } else if ENERGY.iter().any(|e| canonical.contains(e)) {
        Kind::Energy
    } else {
        Kind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(canonical: &str) -> NewsQuery {
        NewsQuery::new(canonical)
    }

    #[test]
    fn classes() {
        assert_eq!(classify(&q("EURUSD")), Kind::FxPair);
        assert_eq!(classify(&q("usdjpy")), Kind::FxPair);
        assert_eq!(classify(&q("XAUUSD")), Kind::Metal);
        assert_eq!(classify(&q("XAGUSD")), Kind::Metal);
        assert_eq!(classify(&q("BTCUSD")), Kind::Crypto);
        assert_eq!(classify(&q("USOIL")), Kind::Energy);
        assert_eq!(classify(&q("BRENT")), Kind::Energy);
        assert_eq!(classify(&q("US500")), Kind::Other);
        assert_eq!(classify(&q("NAS100")), Kind::Other);
    }

    #[test]
    fn explicit_base_and_quote_win() {
        let query = q("GOLD").with_pair("XAU", "USD");
        assert_eq!(classify(&query), Kind::Metal);
        let query = q("SOMETHING").with_pair("GBP", "JPY");
        assert_eq!(classify(&query), Kind::FxPair);
    }
}

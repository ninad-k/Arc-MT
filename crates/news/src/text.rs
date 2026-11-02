//! Word tokenising shared by the lexicon and the relevance checks.
//!
//! Everything is lower case. Sentence ends become a [`BREAK`] token so a negation or a phrase
//! never reaches across sentences. A trailing possessive ("gold's") is dropped so it still
//! matches "gold".

pub(crate) const BREAK: &str = ".";

pub(crate) fn tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &raw) in chars.iter().enumerate() {
        let c = if raw == '\u{2019}' || raw == '\u{2018}' {
            '\''
        } else {
            raw
        };
        if c.is_alphanumeric() {
            cur.extend(c.to_lowercase());
        } else if c == '\'' && !cur.is_empty() {
            cur.push('\'');
        } else {
            push_token(&mut out, &mut cur);
            // "1.09" is a number, "rose. Then" is a sentence end.
            let ends_sentence = matches!(c, '.' | '!' | '?' | ';')
                && chars.get(i + 1).is_none_or(|n| n.is_whitespace());
            if ends_sentence {
                out.push(BREAK.to_string());
            }
        }
    }
    push_token(&mut out, &mut cur);
    out
}

fn push_token(out: &mut Vec<String>, cur: &mut String) {
    if cur.is_empty() {
        return;
    }
    let raw = std::mem::take(cur);
    let t = raw
        .strip_suffix("'s")
        .unwrap_or(&raw)
        .trim_end_matches('\'');
    if !t.is_empty() {
        out.push(t.to_string());
    }
}

/// Does `phrase` match the tokens starting at `at`?
pub(crate) fn matches_at(tokens: &[String], at: usize, phrase: &[String]) -> bool {
    !phrase.is_empty()
        && at + phrase.len() <= tokens.len()
        && tokens[at..at + phrase.len()] == *phrase
}

pub(crate) fn contains_phrase(tokens: &[String], phrase: &[String]) -> bool {
    (0..tokens.len()).any(|i| matches_at(tokens, i, phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_lowercases() {
        assert_eq!(tokens("Euro JUMPS, again"), ["euro", "jumps", "again"]);
    }

    #[test]
    fn hyphen_and_slash_split() {
        assert_eq!(
            tokens("sell-off in EUR/USD"),
            ["sell", "off", "in", "eur", "usd"]
        );
    }

    #[test]
    fn possessive_and_curly_apostrophe() {
        assert_eq!(tokens("Gold\u{2019}s rally"), ["gold", "rally"]);
        assert_eq!(tokens("it isn't"), ["it", "isn't"]);
    }

    #[test]
    fn sentence_end_makes_a_break_but_decimals_do_not() {
        assert_eq!(
            tokens("rose 1.09. Next"),
            ["rose", "1", "09", BREAK, "next"]
        );
    }
}

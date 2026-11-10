//! Deterministic style fixes applied to formatted dictation.
//!
//! Speech models punctuate every pause, and the post-processing model keeps
//! those commas however it is prompted: measured against Wispr Flow on the
//! same transcripts, the output carried about six commas per hundred words to
//! its three and a half. The commas it removes are regular enough to remove
//! here: before a clause joined with "but", "so" or "because", after a
//! sentence opener, and before an "and" that isn't closing a series.

use once_cell::sync::Lazy;
use regex::{Captures, Regex};

static BEFORE_CONJUNCTION: Lazy<Regex> =
    Lazy::new(|| Regex::new(r",(\s+(?:but|so|because)\s)").expect("valid regex"));

static AFTER_OPENER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"((?:^|\n)\s*(?:[-•]\s+|\d+\.\s+)?|[.!?:]\s+)",
        r"(For example|For instance|Ideally|Right now|Okay|Also|Then|So|Yeah|Well|Now|Honestly|",
        r"Basically|Realistically|Actually|Anyway|Anyways|Obviously),(\s)",
    ))
    .expect("valid regex")
});

static BEFORE_AND: Lazy<Regex> = Lazy::new(|| Regex::new(r",(\s+and\s)").expect("valid regex"));

/// How far back in a sentence another comma marks ", and" as the end of a
/// series ("eggs, milk, and bread") rather than a pause between clauses.
const SERIES_WINDOW: usize = 60;

/// Removes the commas that mark a spoken pause rather than a written one.
/// English only: the word lists mean nothing in other languages.
pub fn lighten_commas(text: &str) -> String {
    let text = BEFORE_CONJUNCTION.replace_all(text, "$1");
    let text = AFTER_OPENER.replace_all(&text, "$1$2$3");
    BEFORE_AND
        .replace_all(&text, |caps: &Captures| {
            let whole = caps.get(0).expect("group 0 always exists");
            let head = &text[..whole.start()];
            let sentence_start = head
                .rfind(['.', '!', '?', ':', '\n'])
                .map_or(0, |index| index + 1);
            let sentence = &head[sentence_start..];
            let mut window_start = sentence.len().saturating_sub(SERIES_WINDOW);
            while !sentence.is_char_boundary(window_start) {
                window_start += 1;
            }
            if sentence[window_start..].contains(',') {
                whole.as_str().to_string()
            } else {
                caps[1].to_string()
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::lighten_commas;

    #[test]
    fn pause_commas_before_conjunctions_are_removed() {
        assert_eq!(
            lighten_commas("I like it, but it's slow, so I stopped, because it crashed."),
            "I like it but it's slow so I stopped because it crashed."
        );
    }

    #[test]
    fn commas_after_sentence_openers_are_removed() {
        assert_eq!(
            lighten_commas("For example, we link to listings. Ideally, we show an icon."),
            "For example we link to listings. Ideally we show an icon."
        );
        // Also at the start of a list item.
        assert_eq!(
            lighten_commas("- Okay, do it.\n2. Then, ship."),
            "- Okay do it.\n2. Then ship."
        );
    }

    #[test]
    fn a_series_keeps_its_commas() {
        assert_eq!(
            lighten_commas("We need eggs, milk, and bread."),
            "We need eggs, milk, and bread."
        );
    }

    #[test]
    fn a_lone_comma_before_and_is_a_pause() {
        assert_eq!(
            lighten_commas("We shipped the app, and the team is happy."),
            "We shipped the app and the team is happy."
        );
    }

    #[test]
    fn other_commas_and_mid_sentence_words_are_untouched() {
        let text = "Hey Jordan, thanks. It was fine, which is weird. Do so, now.";
        assert_eq!(lighten_commas(text), text);
        // Multi-byte text before the match must not split a character.
        assert_eq!(
            lighten_commas("Café ünïcödé — naïve, and done."),
            "Café ünïcödé — naïve and done."
        );
    }
}

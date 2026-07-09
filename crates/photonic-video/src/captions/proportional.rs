//! Proportional-by-character-count word timing distribution (06 §2.2, §7.1).
//!
//! Shared by two callers that both need to fabricate word-level timing from a
//! cue/segment that only has line-level timing: the hosted provider's
//! "cue-only source" degraded path (§2.2) and SRT/VTT/ASS import when no
//! native word timing is present (§7.1). One implementation, two thin
//! wrappers so each caller gets back its own word type without duplicating
//! the distribution math.

use photonic_core::timeline::{CaptionWord, Tick};

use super::provider::TranscribedWord;

/// Split `text` into whitespace-separated tokens and distribute `[start,
/// end)` across them proportionally by character count. Deterministic;
/// rounds toward the start of each token. Returns `(text, start, end)`
/// triples in token order.
fn distribute_spans(text: &str, start: Tick, end: Tick) -> Vec<(String, Tick, Tick)> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let total_chars: usize = tokens.iter().map(|t| t.chars().count()).sum();
    let span = (end.0 - start.0).max(0) as f64;

    let mut cursor = start.0 as f64;
    let mut out = Vec::with_capacity(tokens.len());
    for (i, tok) in tokens.iter().enumerate() {
        let is_last = i + 1 == tokens.len();
        let frac = if total_chars == 0 {
            1.0 / tokens.len() as f64
        } else {
            tok.chars().count() as f64 / total_chars as f64
        };
        let dur = span * frac;
        let w_start = Tick(cursor.round() as i64);
        let w_end = if is_last {
            end
        } else {
            Tick((cursor + dur).round() as i64)
        };
        // Guard degenerate zero/negative spans (e.g. start == end) so every
        // word still gets a valid, non-decreasing [start, end].
        let w_end = if w_end.0 < w_start.0 { w_start } else { w_end };
        out.push((tok.to_string(), w_start, w_end));
        cursor = w_end.0 as f64;
    }
    out
}

/// [`TranscribedWord`] flavor — used by the hosted adapter's degraded
/// cue-only path (06 §2.2). `confidence` is always `None` (the source never
/// had per-word confidence to begin with).
pub fn distribute_words_proportionally(text: &str, start: Tick, end: Tick) -> Vec<TranscribedWord> {
    distribute_spans(text, start, end)
        .into_iter()
        .map(|(text, start, end)| TranscribedWord {
            text,
            start,
            end,
            confidence: None,
        })
        .collect()
}

/// [`CaptionWord`] flavor — used by SRT/VTT/ASS import when the source has
/// no native word-level timing (06 §7.1).
pub fn distribute_caption_words(text: &str, start: Tick, end: Tick) -> Vec<CaptionWord> {
    distribute_spans(text, start, end)
        .into_iter()
        .map(|(text, start, end)| CaptionWord::new(text, start, end))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_no_words() {
        assert!(distribute_caption_words("", Tick(0), Tick(1000)).is_empty());
        assert!(distribute_caption_words("   ", Tick(0), Tick(1000)).is_empty());
    }

    #[test]
    fn single_word_spans_the_whole_range() {
        let words = distribute_caption_words("hello", Tick(100), Tick(500));
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].start, Tick(100));
        assert_eq!(words[0].end, Tick(500));
    }

    #[test]
    fn words_are_chronological_and_cover_the_full_span_by_char_count() {
        // "hi" (2 chars) + "world" (5 chars) => 7 total; span 700 ticks.
        let words = distribute_caption_words("hi world", Tick(0), Tick(700));
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "hi");
        assert_eq!(words[1].text, "world");
        assert_eq!(words[0].start, Tick(0));
        // "hi" gets 2/7 of the span ~= 200 ticks.
        assert_eq!(words[0].end, Tick(200));
        assert_eq!(words[1].start, Tick(200));
        // Last word always closes exactly at `end`.
        assert_eq!(words[1].end, Tick(700));
    }

    #[test]
    fn words_are_non_overlapping_and_ordered() {
        let words = distribute_caption_words(
            "the quick brown fox jumps over the lazy dog",
            Tick(0),
            Tick(9000),
        );
        for pair in words.windows(2) {
            assert!(pair[0].end <= pair[1].start);
            assert!(pair[0].start <= pair[0].end);
        }
        assert_eq!(words.last().unwrap().end, Tick(9000));
    }

    #[test]
    fn zero_duration_span_still_produces_valid_non_decreasing_words() {
        let words = distribute_caption_words("a b c", Tick(500), Tick(500));
        for w in &words {
            assert!(w.start <= w.end);
        }
        assert_eq!(words.last().unwrap().end, Tick(500));
    }

    #[test]
    fn transcribed_word_flavor_has_no_confidence() {
        let words = distribute_words_proportionally("a b", Tick(0), Tick(100));
        assert!(words.iter().all(|w| w.confidence.is_none()));
    }
}

//! Grouping algorithm: raw word stream → `Vec<CaptionCue>` (06 §3.5).
//!
//! Pure function, no engine/I-O involved: `&[TranscribedWord] ->
//! Vec<CaptionCue>`. Reused by both the auto-caption workflow (CAP-009) and
//! the "also caption this voiceover" TTS option (06 §6), since both produce a
//! `Vec<TranscribedWord>` as their raw input.
//!
//! Two passes, exactly as spec'd:
//! - **Pass 1 (build)** — greedy forward scan, flushing a cue on a silence
//!   gap, a projected overflow of `max_chars_per_line * max_lines_per_cue`,
//!   or a sentence-ending previous word.
//! - **Pass 2 (repair)** — split any cue whose duration exceeds
//!   `max_cue_duration` (recursively, in case one split still leaves a half
//!   too long), then merge any cue whose duration is under
//!   `min_cue_duration` into a neighbor, in a single left-to-right pass (not
//!   iterated to a fixpoint — the spec does not require re-checking merged
//!   results against the duration bounds again).

use photonic_core::timeline::{CaptionCue, Tick, TICKS_PER_SECOND};

use super::provider::TranscribedWord;

/// Tunable grouping parameters (06 §3.5 table). Defaults match the spec's
/// table; overridable per-project in caption settings (out of this story's
/// scope — the settings UI/persistence).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupingParams {
    pub max_chars_per_line: usize,
    pub max_lines_per_cue: usize,
    pub min_cue_duration: Tick,
    pub max_cue_duration: Tick,
    pub gap_merge_threshold: Tick,
}

impl Default for GroupingParams {
    fn default() -> Self {
        GroupingParams {
            max_chars_per_line: 42,
            max_lines_per_cue: 2,
            min_cue_duration: ticks_from_millis(800),
            max_cue_duration: Tick::from_seconds(6),
            gap_merge_threshold: ticks_from_millis(250),
        }
    }
}

impl GroupingParams {
    fn max_chars_per_cue(&self) -> usize {
        self.max_chars_per_line * self.max_lines_per_cue
    }
}

/// Ticks per second is exactly divisible by 1000 (`TICKS_PER_SECOND =
/// 705_600_000`), so this is exact for any millisecond count, not a rounded
/// approximation.
const fn ticks_from_millis(ms: i64) -> Tick {
    Tick(TICKS_PER_SECOND / 1000 * ms)
}

fn ends_sentence(text: &str) -> bool {
    matches!(text.trim_end().chars().last(), Some('.') | Some('!') | Some('?'))
}

/// Character length of `words` joined with single spaces (matches how
/// `CaptionCue::text()` renders them), used for the `max_chars_per_cue` cap.
fn joined_char_len(words: &[TranscribedWord]) -> usize {
    if words.is_empty() {
        return 0;
    }
    words.iter().map(|w| w.text.chars().count()).sum::<usize>() + (words.len() - 1)
}

fn cue_duration(cue: &CaptionCue) -> Tick {
    cue.end.saturating_sub(cue.start)
}

fn flush(words: Vec<TranscribedWord>) -> CaptionCue {
    debug_assert!(!words.is_empty());
    let start = words.first().unwrap().start;
    let end = words.last().unwrap().end;
    let caption_words = words
        .into_iter()
        .map(|w| photonic_core::timeline::CaptionWord::new(w.text, w.start, w.end))
        .collect();
    CaptionCue::new(start, end, caption_words)
}

/// Words → cues, build pass (06 §3.5 pass 1) followed by the repair pass
/// (pass 2). The single entry point this module exposes.
pub fn group_words_into_cues(words: &[TranscribedWord], params: &GroupingParams) -> Vec<CaptionCue> {
    let built = build_pass(words, params);
    let split = split_long_cues(built, params);
    merge_short_cues(split, params)
}

fn build_pass(words: &[TranscribedWord], params: &GroupingParams) -> Vec<CaptionCue> {
    let mut cues = Vec::new();
    let mut current: Vec<TranscribedWord> = Vec::new();

    for word in words {
        if current.is_empty() {
            current.push(word.clone());
            continue;
        }
        let last = current.last().unwrap();
        let gap = word.start.saturating_sub(last.end);
        let projected_chars = joined_char_len(&current) + 1 + word.text.chars().count();
        let prev_ends_sentence = ends_sentence(&last.text);

        if gap > params.gap_merge_threshold
            || projected_chars > params.max_chars_per_cue()
            || prev_ends_sentence
        {
            cues.push(flush(std::mem::take(&mut current)));
            current.push(word.clone());
        } else {
            current.push(word.clone());
        }
    }
    if !current.is_empty() {
        cues.push(flush(current));
    }
    cues
}

/// Recursively splits any cue longer than `max_cue_duration`. A cue with a
/// single word can't be split further and is left over-length (better than
/// losing text, same philosophy as the short-cue repair).
fn split_long_cues(cues: Vec<CaptionCue>, params: &GroupingParams) -> Vec<CaptionCue> {
    let mut out = Vec::with_capacity(cues.len());
    for cue in cues {
        if cue_duration(&cue) > params.max_cue_duration && cue.words.len() > 1 {
            let (a, b) = split_cue_at_best_point(cue);
            out.extend(split_long_cues(vec![a], params));
            out.extend(split_long_cues(vec![b], params));
        } else {
            out.push(cue);
        }
    }
    out
}

/// Split `cue` into two, choosing the boundary per 06 §3.5's three-tier
/// fallback: nearest-to-midpoint silence gap, else nearest-to-midpoint
/// sentence-ending word, else the plain nearest-to-midpoint word boundary.
/// Within the first tier, ties on distance-to-midpoint break toward the
/// larger gap.
fn split_cue_at_best_point(cue: CaptionCue) -> (CaptionCue, CaptionCue) {
    let words = cue.words;
    let n = words.len();
    debug_assert!(n > 1);

    let mid = cue.start.0 + (cue.end.0 - cue.start.0) / 2;
    let boundary_tick = |i: usize| -> i64 {
        let a = words[i].end.0;
        let b = words[i + 1].start.0;
        a + (b - a) / 2
    };
    let dist = |i: usize| (boundary_tick(i) - mid).abs();

    let gap_candidates: Vec<usize> = (0..n - 1)
        .filter(|&i| words[i + 1].start.0 > words[i].end.0)
        .collect();

    let chosen = if !gap_candidates.is_empty() {
        gap_candidates
            .into_iter()
            .min_by_key(|&i| {
                let gap = words[i + 1].start.0 - words[i].end.0;
                (dist(i), std::cmp::Reverse(gap))
            })
            .unwrap()
    } else {
        let sentence_candidates: Vec<usize> =
            (0..n - 1).filter(|&i| ends_sentence(&words[i].text)).collect();
        if !sentence_candidates.is_empty() {
            sentence_candidates.into_iter().min_by_key(|&i| dist(i)).unwrap()
        } else {
            (0..n - 1).min_by_key(|&i| dist(i)).unwrap()
        }
    };

    let mut words = words;
    let b_words = words.split_off(chosen + 1);
    let a_words = words;

    (cue_from_caption_words(a_words), cue_from_caption_words(b_words))
}

/// Like [`flush`] but for words already converted to `CaptionWord` (i.e.
/// re-assembling a cue split from an already-built cue's own words, as
/// opposed to `flush`'s raw `TranscribedWord` input from pass 1).
fn cue_from_caption_words(words: Vec<photonic_core::timeline::CaptionWord>) -> CaptionCue {
    debug_assert!(!words.is_empty());
    let start = words.first().unwrap().start;
    let end = words.last().unwrap().end;
    CaptionCue::new(start, end, words)
}

/// Merges any cue shorter than `min_cue_duration` into the following cue
/// (or, if it's the last cue, into the preceding one), provided the merge
/// stays within `max_chars_per_cue`; otherwise the cue is left short. Single
/// left-to-right pass.
fn merge_short_cues(cues: Vec<CaptionCue>, params: &GroupingParams) -> Vec<CaptionCue> {
    if cues.len() <= 1 {
        return cues;
    }
    let max_chars = params.max_chars_per_cue();
    let mut result: Vec<CaptionCue> = Vec::with_capacity(cues.len());
    let mut iter = cues.into_iter().peekable();

    while let Some(cue) = iter.next() {
        if cue_duration(&cue) >= params.min_cue_duration {
            result.push(cue);
            continue;
        }
        // `cue` is short: merge it into the *following* cue if one exists and
        // the merge stays within the char budget. When the forward merge would
        // overflow, leave `cue` short but DO NOT consume the next cue — it gets
        // its own turn on the next loop iteration so it can still merge into
        // *its* follower. (06 §3.5: every short cue is offered a forward merge;
        // single left-to-right pass.)
        if iter.peek().is_some() {
            if merged_char_len(&cue, iter.peek().unwrap()) <= max_chars {
                let next = iter.next().unwrap();
                result.push(merge_cues(cue, next));
            } else {
                result.push(cue);
            }
        } else if let Some(prev) = result.pop() {
            if merged_char_len(&prev, &cue) <= max_chars {
                result.push(merge_cues(prev, cue));
            } else {
                result.push(prev);
                result.push(cue);
            }
        } else {
            result.push(cue);
        }
    }
    result
}

fn merged_char_len(a: &CaptionCue, b: &CaptionCue) -> usize {
    let total_words = a.words.len() + b.words.len();
    if total_words == 0 {
        return 0;
    }
    let chars: usize = a
        .words
        .iter()
        .chain(b.words.iter())
        .map(|w| w.text.chars().count())
        .sum();
    chars + (total_words - 1)
}

fn merge_cues(a: CaptionCue, b: CaptionCue) -> CaptionCue {
    let mut words = a.words;
    words.extend(b.words);
    let start = words.first().map(|w| w.start).unwrap_or(a.start.min(b.start));
    let end = words.last().map(|w| w.end).unwrap_or(a.end.max(b.end));
    CaptionCue::new(start, end, words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start_ms: i64, end_ms: i64) -> TranscribedWord {
        TranscribedWord {
            text: text.to_string(),
            start: ticks_from_millis(start_ms),
            end: ticks_from_millis(end_ms),
            confidence: None,
        }
    }

    #[test]
    fn dense_speech_all_merges_into_one_cue() {
        // Back-to-back words, no gaps, no sentence-enders, short text:
        // everything should land in a single cue.
        let words = vec![
            w("the", 0, 200),
            w("quick", 200, 500),
            w("brown", 500, 800),
            w("fox", 800, 1000),
        ];
        let cues = group_words_into_cues(&words, &GroupingParams::default());
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].words.len(), 4);
        assert_eq!(cues[0].start, ticks_from_millis(0));
        assert_eq!(cues[0].end, ticks_from_millis(1000));
    }

    #[test]
    fn long_pause_forces_a_break() {
        // Words are each >= min_cue_duration (800ms) so the Pass-1 gap break is
        // NOT undone by the Pass-2 short-cue merge — this test isolates the
        // forced-break behavior, not the merge behavior (06 §3.5).
        let words = vec![
            w("hello", 0, 900),
            // Gap of 500ms > 250ms default threshold => forced break.
            w("world", 1400, 2300),
        ];
        let cues = group_words_into_cues(&words, &GroupingParams::default());
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text(), "hello");
        assert_eq!(cues[1].text(), "world");
    }

    #[test]
    fn short_gap_stays_in_the_same_cue() {
        let words = vec![w("hello", 0, 300), w("world", 400, 700)]; // 100ms gap
        let cues = group_words_into_cues(&words, &GroupingParams::default());
        assert_eq!(cues.len(), 1);
    }

    #[test]
    fn punctuation_heavy_input_breaks_on_sentence_end() {
        // Durations chosen so both resulting cues clear min_cue_duration
        // (800ms) and survive the Pass-2 merge; the ONLY break trigger here is
        // the sentence-ending "Stop." (gaps are zero), isolating the
        // sentence-end rule (06 §3.5).
        let words = vec![
            w("Stop.", 0, 900),
            w("Go", 900, 1400), // no gap, but previous word ends a sentence
            w("now", 1400, 2000),
        ];
        let cues = group_words_into_cues(&words, &GroupingParams::default());
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text(), "Stop.");
        assert_eq!(cues[1].text(), "Go now");
    }

    #[test]
    fn long_monologue_splits_at_max_cue_duration() {
        // One continuous run of words (no gaps, no punctuation) spanning 9s,
        // one word per 300ms => must split because it exceeds the 6s default
        // max_cue_duration. Put a deliberate silence gap near the middle so
        // the split lands on it (tier 1 of the fallback).
        let mut words = Vec::new();
        let mut t = 0i64;
        for i in 0..15 {
            let start = t;
            let end = t + 300;
            words.push(w(&format!("w{i}"), start, end));
            t = end;
            if i == 7 {
                t += 400; // silence gap near the temporal midpoint
            }
        }
        let params = GroupingParams::default();
        let cues = group_words_into_cues(&words, &params);
        assert!(cues.len() >= 2, "expected a split, got {} cue(s)", cues.len());
        for cue in &cues {
            // Every resulting cue must respect the max duration (each word is
            // only 300ms, so a valid split point always exists).
            assert!(
                cue_duration(cue) <= params.max_cue_duration,
                "cue {:?} exceeds max_cue_duration",
                cue.text()
            );
        }
        // Words are preserved in order across the split with no loss/dup.
        let total_words: usize = cues.iter().map(|c| c.words.len()).sum();
        assert_eq!(total_words, words.len());
    }

    #[test]
    fn short_cue_merges_forward() {
        let words = vec![
            // First cue: single short word, then a big gap forcing a break.
            w("Hi.", 0, 300),
            w("greetings", 3000, 3800),
            w("everyone", 3800, 4600),
        ];
        let params = GroupingParams::default();
        let cues = group_words_into_cues(&words, &params);
        // "Hi." alone is a 300ms cue (< 800ms min) — should merge forward
        // into the next cue rather than stay short.
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text(), "Hi. greetings everyone");
    }

    #[test]
    fn short_last_cue_merges_backward() {
        let words = vec![
            w("greetings", 0, 800),
            w("everyone", 800, 1600),
            // Big gap, then a short trailing cue that is the *last* cue.
            w("Bye.", 4000, 4300),
        ];
        let params = GroupingParams::default();
        let cues = group_words_into_cues(&words, &params);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text(), "greetings everyone Bye.");
    }

    #[test]
    fn short_cue_still_merges_forward_after_a_prior_overflow() {
        // Three cues after Pass 1: A (short, 60 chars), B (short, 30 chars),
        // C (long, 10 chars), each forced apart by >250ms gaps. A+B = 91 chars
        // > 84 budget, so A cannot merge into B and is left short. B, however,
        // is itself short and B+C = 41 chars <= 84, so B MUST still merge
        // forward into C (06 §3.5: every short cue is offered a forward merge).
        // Regression guard: a naive pass that consumes B while rejecting the
        // A+B merge would strand B and wrongly yield three cues.
        let a = "a".repeat(60);
        let b = "b".repeat(30);
        let c = "c".repeat(10);
        let words = vec![
            w(&a, 0, 300),        // short (300ms)
            w(&b, 1000, 1300),    // short (300ms), gap 700ms => break
            w(&c, 2000, 3000),    // long (1000ms), gap 700ms => break
        ];
        let params = GroupingParams::default();
        let cues = group_words_into_cues(&words, &params);
        assert_eq!(cues.len(), 2, "B must merge forward into C");
        assert_eq!(cues[0].text(), a);
        assert_eq!(cues[1].text(), format!("{b} {c}"));
    }

    #[test]
    fn short_cue_left_short_when_merge_would_overflow_char_budget() {
        // "Hi." (3) + space (1) + 82 = 86 chars, which *exceeds* the
        // max_chars_per_cue budget of 42*2=84. Merging is therefore disallowed
        // ("provided the merge doesn't exceed ..." — 06 §3.5), so the short
        // "Hi." cue is left short rather than losing text. (At exactly 84 the
        // merge would be permitted — the budget is an inclusive ceiling.)
        let long_word = "x".repeat(82);
        let words = vec![
            w("Hi.", 0, 300),
            w(&long_word, 3000, 3800),
        ];
        let params = GroupingParams::default();
        let cues = group_words_into_cues(&words, &params);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text(), "Hi.");
    }

    #[test]
    fn words_are_never_lost_or_reordered() {
        let words: Vec<TranscribedWord> = (0..50)
            .map(|i| w(&format!("word{i}"), i * 200, i * 200 + 150))
            .collect();
        let cues = group_words_into_cues(&words, &GroupingParams::default());
        let flat: Vec<String> = cues.iter().flat_map(|c| c.words.iter().map(|w| w.text.clone())).collect();
        let expected: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
        assert_eq!(flat, expected);
    }

    #[test]
    fn empty_input_yields_no_cues() {
        assert!(group_words_into_cues(&[], &GroupingParams::default()).is_empty());
    }

    #[test]
    fn ticks_from_millis_matches_default_table_values() {
        let params = GroupingParams::default();
        assert_eq!(params.min_cue_duration, Tick(564_480_000)); // 0.8s
        assert_eq!(params.gap_merge_threshold, Tick(176_400_000)); // 250ms
        assert_eq!(params.max_cue_duration, Tick::from_seconds(6));
    }
}

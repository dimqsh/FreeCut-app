//! TX-022's **Pace**: a slot length from a time per unit, and the unit
//! counts it needs (TX-023), without layout.
//!
//! Units are counted with a table-free approximation of Unicode text
//! segmentation (no new dependencies): a letter is a grapheme cluster
//! (combining marks, joiners, variation selectors, emoji modifiers, tags
//! and regional-indicator pairs extend it); a word is a run of characters
//! between white space, each CJK ideograph or kana counting alone.
//! Whitespace units take no time (TX-023), so they are not counted. The
//! renderer splits units from the layout, which may differ in rare
//! scripts; only the slot length depends on this count.

use crate::model::{AnimUnit, PerMille};
use crate::time::{FrameRate, Rounding, Tick, TimeError};

use crate::model::AnimationSpec;

/// The units of `text` a per-unit preset moves (TX-023): letters, words,
/// lines (separated by hard breaks) or the block (1 when anything shows).
/// Whitespace is never a unit.
pub fn count_units(text: &str, unit: AnimUnit) -> u32 {
    let n = match unit {
        AnimUnit::Letter => graphemes(text).filter(|g| !is_blank(g)).count(),
        AnimUnit::Word => words(text),
        AnimUnit::Line => text.split('\n').filter(|l| !is_blank(l)).count(),
        AnimUnit::Block => usize::from(!is_blank(text)),
    };
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The slot length for `units` units at `pace` each with `overlap`
/// (TX-022): `units × pace × (1 − overlap) + pace × overlap`, rounded to
/// `rate`'s frames, and never below FX-018's 0.1 s minimum (rounded up to
/// a frame), which is also what text with nothing to reveal gets.
pub fn pace_duration(
    units: u32,
    pace: Tick,
    overlap: PerMille,
    rate: FrameRate,
) -> Result<Tick, TimeError> {
    let min = rate
        .round_to_grid(AnimationSpec::MIN_DURATION, Rounding::Ceil)?
        .max(rate.frame_ticks());
    if units == 0 {
        return Ok(min);
    }
    let ov = i128::from(overlap.get().min(1000));
    let weight = i128::from(units) * (1000 - ov) + ov;
    let raw = i128::from(pace.raw().max(0)) * weight / 1000;
    let raw = Tick::from_raw(i64::try_from(raw).map_err(|_| TimeError::Overflow)?);
    Ok(rate.quantize(raw)?.max(min))
}

/// The unit Pace counts for `spec`: its own unit, or letters (Pace is
/// offered only on letter and word presets, TX-022, and the app stores
/// the effective unit with it). `None` for lines and blocks, which have
/// no Pace.
pub fn pace_unit(spec: &AnimationSpec) -> Option<AnimUnit> {
    match spec.params.unit.unwrap_or(AnimUnit::Letter) {
        u @ (AnimUnit::Letter | AnimUnit::Word) => Some(u),
        AnimUnit::Line | AnimUnit::Block => None,
    }
}

/// `spec` with its duration recomputed from its Pace for `text`
/// (TX-022); unchanged without Pace.
pub fn with_paced_duration(
    spec: &AnimationSpec,
    text: &str,
    rate: FrameRate,
) -> Result<AnimationSpec, TimeError> {
    let (Some(pace), Some(unit)) = (spec.params.pace, pace_unit(spec)) else {
        return Ok(spec.clone());
    };
    let duration = pace_duration(count_units(text, unit), pace, spec.params.overlap, rate)?;
    Ok(AnimationSpec {
        duration,
        ..spec.clone()
    })
}

fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// Approximate extended grapheme clusters.
fn graphemes(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let mut chars = rest.char_indices().peekable();
        let (_, first) = chars.next()?;
        let mut end = first.len_utf8();
        let mut prev = first;
        let mut regional = u32::from(is_regional(first));
        while let Some(&(i, c)) = chars.peek() {
            let joins = extends(c)
                || prev == '\u{200D}'
                || (prev == '\r' && c == '\n')
                || (is_regional(c) && regional % 2 == 1);
            if !joins {
                break;
            }
            if is_regional(c) {
                regional += 1;
            }
            end = i + c.len_utf8();
            prev = c;
            chars.next();
        }
        let (cluster, tail) = rest.split_at(end);
        rest = tail;
        Some(cluster)
    })
}

fn is_regional(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Characters that extend the cluster before them: combining marks of the
/// common scripts, joiners, variation selectors, emoji modifiers and tags.
fn extends(c: char) -> bool {
    const RANGES: &[(u32, u32)] = &[
        (0x0300, 0x036F),
        (0x0483, 0x0489),
        (0x0591, 0x05BD),
        (0x05BF, 0x05C7),
        (0x0610, 0x061A),
        (0x064B, 0x065F),
        (0x0670, 0x0670),
        (0x06D6, 0x06ED),
        (0x0711, 0x0711),
        (0x0730, 0x074A),
        (0x0900, 0x0903),
        (0x093A, 0x094F),
        (0x0951, 0x0957),
        (0x0962, 0x0963),
        (0x0981, 0x0983),
        (0x09BC, 0x09D7),
        (0x0A01, 0x0A03),
        (0x0A3C, 0x0A51),
        (0x0A81, 0x0A83),
        (0x0ABC, 0x0ACD),
        (0x0B01, 0x0B03),
        (0x0B3C, 0x0B57),
        (0x0BBE, 0x0BCD),
        (0x0C00, 0x0C04),
        (0x0C3E, 0x0C56),
        (0x0CBC, 0x0CD6),
        (0x0D00, 0x0D03),
        (0x0D3E, 0x0D4D),
        (0x0E31, 0x0E31),
        (0x0E34, 0x0E3A),
        (0x0E47, 0x0E4E),
        (0x0EB1, 0x0EB1),
        (0x0EB4, 0x0EBC),
        (0x0EC8, 0x0ECD),
        (0x1AB0, 0x1AFF),
        (0x1DC0, 0x1DFF),
        (0x200C, 0x200D),
        (0x20D0, 0x20FF),
        (0x302A, 0x302F),
        (0x3099, 0x309A),
        (0xFE00, 0xFE0F),
        (0xFE20, 0xFE2F),
        (0x1F3FB, 0x1F3FF),
        (0xE0020, 0xE007F),
        (0xE0100, 0xE01EF),
    ];
    let c = u32::from(c);
    RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// Ideographs and kana, which form a word each.
fn stands_alone(c: char) -> bool {
    matches!(
        u32::from(c),
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}

fn words(text: &str) -> usize {
    let mut count = 0;
    let mut in_word = false;
    for c in text.chars() {
        if c.is_whitespace() {
            in_word = false;
        } else if stands_alone(c) {
            count += 1;
            in_word = false;
        } else if !in_word {
            count += 1;
            in_word = true;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_grapheme_clusters_without_whitespace() {
        assert_eq!(count_units("Hello world", AnimUnit::Letter), 10);
        assert_eq!(count_units("e\u{301}te\u{301}", AnimUnit::Letter), 3);
        // A ZWJ family, a skin-toned wave and a flag: one unit each.
        assert_eq!(
            count_units("👨\u{200D}👩\u{200D}👧 👋🏽 🇹🇷", AnimUnit::Letter),
            3
        );
        assert_eq!(count_units("🇹🇷🇩🇪", AnimUnit::Letter), 2);
        assert_eq!(count_units("مرحبا", AnimUnit::Letter), 5);
        assert_eq!(count_units("  \n ", AnimUnit::Letter), 0);
        assert_eq!(count_units("a\r\nb", AnimUnit::Letter), 2);
    }

    #[test]
    fn words_lines_and_blocks() {
        assert_eq!(count_units("straße\nın İzmir. ok", AnimUnit::Word), 4);
        assert_eq!(count_units("  ", AnimUnit::Word), 0);
        assert_eq!(count_units("日本語 abc", AnimUnit::Word), 4);
        assert_eq!(count_units("one\n\ntwo\n", AnimUnit::Line), 2);
        assert_eq!(count_units("x", AnimUnit::Block), 1);
        assert_eq!(count_units(" ", AnimUnit::Block), 0);
    }

    #[test]
    fn pace_follows_tx_022() {
        let r25 = FrameRate::new(25, 1).unwrap();
        let ms = |v: i64| Tick::from_raw(crate::time::TICKS_PER_SECOND / 1000 * v);
        // 10 letters at 0.06 s, no overlap: 0.6 s = 15 frames.
        assert_eq!(
            pace_duration(10, ms(60), PerMille::ZERO, r25).unwrap(),
            r25.frames(15).unwrap()
        );
        // 5 words at 0.15 s, 50 % overlap: 5·0.15·0.5 + 0.075 = 0.45 s,
        // 11.25 frames → 11.
        assert_eq!(
            pace_duration(5, ms(150), PerMille::new(500), r25).unwrap(),
            r25.frames(11).unwrap()
        );
        // Nothing to reveal: the 0.1 s minimum, rounded up to 3 frames.
        let min = pace_duration(0, ms(60), PerMille::ZERO, r25).unwrap();
        assert_eq!(min, r25.frames(3).unwrap());
        assert_eq!(pace_duration(1, ms(20), PerMille::ZERO, r25).unwrap(), min);
        // Full overlap: one pace whatever the count.
        assert_eq!(
            pace_duration(40, ms(400), PerMille::ONE, r25).unwrap(),
            r25.frames(10).unwrap()
        );
    }
}

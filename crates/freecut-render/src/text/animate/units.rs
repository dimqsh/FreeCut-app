//! Animation units from the laid-out text (TX-023): which unit every glyph
//! belongs to, which units take time, and each unit's rest box.

use unicode_segmentation::UnicodeSegmentation;

use super::AnimUnit;
use crate::shape::Rect;
use crate::text::TextLayout;

/// The units of one laid-out text for one [`AnimUnit`].
///
/// **Timed units.** Whitespace takes no time (TX-023): a whitespace-only
/// unit (a space grapheme, an empty line) is folded into the timed unit
/// before it, or the first one when it leads. So is a letter that owns no
/// glyph of its own, drawn by the glyph of the letter before it (a
/// ligature a font forms even with the ligature policy's features off):
/// it moves with that glyph and takes no time of its own. Units are counted in logical
/// (reading) order, from Unicode segmentation of the text for letters and
/// words (the model's Pace counts the same way) and from the layout for
/// lines.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitMap {
    /// The unit used: letters degrade to words in cursive-joining scripts
    /// (Arabic and similar), so joined letters never come apart.
    pub unit: AnimUnit,
    /// How many timed units there are.
    pub count: u32,
    /// Per layout glyph ([`TextLayout::glyphs`]): its timed unit.
    pub glyph_unit: Vec<u32>,
    /// Per layout glyph: whether it belongs to whitespace (no ink, no box).
    pub blank: Vec<bool>,
    /// Per timed unit: the rest box (layout px) of its non-blank glyphs'
    /// advances, from their line's top to its bottom; `None` for a unit
    /// without such glyphs.
    pub boxes: Vec<Option<Rect>>,
    /// Per timed unit: the layout line of its first non-blank glyph (its
    /// line for Spread and Word rise, TX-056); `None` with its box.
    pub lines: Vec<Option<u32>>,
}

/// Whether `c` belongs to a script whose letters join (cursive scripts).
pub fn is_joining_script(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0600..=0x06FF // Arabic
            | 0x0700..=0x074F // Syriac
            | 0x0750..=0x077F // Arabic Supplement
            | 0x07C0..=0x07FF // N'Ko
            | 0x0840..=0x085F // Mandaic
            | 0x0860..=0x086F // Syriac Supplement
            | 0x0870..=0x08FF // Arabic Extended-B, -A
            | 0x1800..=0x18AF // Mongolian
            | 0xA840..=0xA87F // Phags-pa
            | 0xFB50..=0xFDFF // Arabic Presentation Forms-A
            | 0xFE70..=0xFEFF // Arabic Presentation Forms-B
            | 0x10AC0..=0x10AFF // Manichaean
            | 0x10B80..=0x10BAF // Psalter Pahlavi
            | 0x10D00..=0x10D3F // Hanifi Rohingya
            | 0x10F30..=0x10F6F // Sogdian
            | 0x10FB0..=0x10FDF // Chorasmian
            | 0x1E900..=0x1E95F // Adlam
    )
}

/// Whether `s` is whitespace only (a stray CR included).
pub(crate) fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// For each item of `blank`, the index of the timed item at or before it
/// (the first timed one for leading blanks), and the number of timed
/// items.
fn fold_blanks(blank: &[bool]) -> (Vec<u32>, u32) {
    let mut timed = 0u32;
    let map = blank
        .iter()
        .map(|&b| {
            if !b {
                timed += 1;
            }
            timed.saturating_sub(1)
        })
        .collect();
    (map, timed)
}

impl UnitMap {
    /// The units of `layout`, laid out from `text` (the display text whose
    /// byte offsets the glyphs' clusters refer to), for `unit`.
    pub fn new(layout: &TextLayout, text: &str, unit: AnimUnit) -> UnitMap {
        let unit = if unit == AnimUnit::Grapheme && text.chars().any(is_joining_script) {
            AnimUnit::Word
        } else {
            unit
        };
        let graphemes: Vec<&str> = text.graphemes(true).collect();
        let grapheme_blank: Vec<bool> = graphemes.iter().map(|g| is_blank(g)).collect();
        let blank: Vec<bool> = layout
            .glyphs
            .iter()
            .map(|g| {
                grapheme_blank
                    .get(g.unit.grapheme as usize)
                    .copied()
                    .unwrap_or(true)
            })
            .collect();
        let (glyph_unit, count): (Vec<u32>, u32) = match unit {
            AnimUnit::Grapheme => {
                let mut owns_glyph = vec![false; graphemes.len()];
                for g in &layout.glyphs {
                    if let Some(o) = owns_glyph.get_mut(g.unit.grapheme as usize) {
                        *o = true;
                    }
                }
                let untimed: Vec<bool> = grapheme_blank
                    .iter()
                    .zip(&owns_glyph)
                    .map(|(&blank, &owns)| blank || !owns)
                    .collect();
                let (map, count) = fold_blanks(&untimed);
                let units = layout
                    .glyphs
                    .iter()
                    .map(|g| map.get(g.unit.grapheme as usize).copied().unwrap_or(0))
                    .collect();
                (units, count)
            }
            AnimUnit::Word => {
                let words = text.unicode_words().count() as u32;
                let any_ink = blank.iter().any(|b| !b);
                // Text without any word (only punctuation) is one unit.
                let count = if words == 0 {
                    u32::from(any_ink)
                } else {
                    words
                };
                let units = layout
                    .glyphs
                    .iter()
                    .map(|g| g.unit.word.min(count.saturating_sub(1)))
                    .collect();
                (units, count)
            }
            AnimUnit::Line => {
                let mut line_blank = vec![true; layout.lines.len()];
                for (g, &b) in layout.glyphs.iter().zip(&blank) {
                    if !b && let Some(l) = line_blank.get_mut(g.unit.line as usize) {
                        *l = false;
                    }
                }
                let (map, count) = fold_blanks(&line_blank);
                let units = layout
                    .glyphs
                    .iter()
                    .map(|g| map.get(g.unit.line as usize).copied().unwrap_or(0))
                    .collect();
                (units, count)
            }
        };
        let mut boxes: Vec<Option<Rect>> = vec![None; count as usize];
        let mut lines: Vec<Option<u32>> = vec![None; count as usize];
        for ((g, &u), &b) in layout.glyphs.iter().zip(&glyph_unit).zip(&blank) {
            if b {
                continue;
            }
            let Some(line) = layout.lines.get(g.unit.line as usize) else {
                continue;
            };
            let r = Rect::new(
                g.origin.0,
                line.top,
                g.origin.0 + g.advance.max(0.0),
                line.bottom,
            );
            if let Some(slot) = boxes.get_mut(u as usize) {
                *slot = Some(slot.map_or(r, |s| s.union(&r)));
            }
            if let Some(slot @ None) = lines.get_mut(u as usize) {
                *slot = Some(g.unit.line);
            }
        }
        UnitMap {
            unit,
            count,
            glyph_unit,
            blank,
            boxes,
            lines,
        }
    }

    /// The glyph indices of timed unit `u`.
    pub fn glyphs_of(&self, u: u32) -> impl Iterator<Item = usize> + '_ {
        self.glyph_unit
            .iter()
            .enumerate()
            .filter(move |(_, v)| **v == u)
            .map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_fold_into_the_unit_before_them() {
        let (map, count) = fold_blanks(&[true, false, true, false, false, true]);
        assert_eq!(map, [0, 0, 0, 1, 2, 2]);
        assert_eq!(count, 3);
        assert_eq!(fold_blanks(&[true, true]), (vec![0, 0], 0));
    }

    #[test]
    fn cursive_scripts_are_detected() {
        assert!("مرحبا".chars().all(is_joining_script));
        assert!(!"Hello שלום".chars().any(is_joining_script));
    }
}

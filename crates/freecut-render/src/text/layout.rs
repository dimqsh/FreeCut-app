//! Layout with parley. Nothing parley-specific leaves this module: the
//! built layout is flattened into per-glyph items, decoration runs and
//! cluster boxes, which the rasterizer and the public [`TextLayout`] use.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use parley::fontique::{Collection, SourceCache};
use parley::{
    Alignment, AlignmentOptions, BreakReason, FontContext, FontData, FontFamily, FontFamilyName,
    FontFeatures, FontStyle, FontWeight, Layout, LayoutContext, LineHeight, OverflowWrap,
    PositionedLayoutItem, StyleProperty, TextWrapMode, WhiteSpaceCollapse,
};
use unicode_segmentation::UnicodeSegmentation;

use super::fonts::DEFAULT_FAMILY;
use super::{TextAlign, TextSpec};

/// Where text fell back to other fonts (TX-006, TX-007).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FallbackReport {
    /// The requested family isn't installed; Noto Sans was used instead.
    pub family_missing: bool,
    /// Families other than the requested one that supplied glyphs, sorted.
    pub fallback_families: Vec<String>,
    /// Characters no available font has (drawn as the font's missing-glyph
    /// box), in text order, deduplicated.
    pub missing_chars: Vec<char>,
}

/// One laid-out line, in layout px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineInfo {
    /// Top and bottom of the line box.
    pub top: f32,
    pub bottom: f32,
    pub baseline: f32,
    /// Left edge of the line's content after alignment.
    pub left: f32,
    /// Width of the line's content (trailing spaces and the letter spacing
    /// after the last cluster excluded; the box width for a justified
    /// line).
    pub width: f32,
}

/// Identifies a font face: the font file's blob id and its index in a
/// collection. Stable while the font set (registry generation) is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphFont {
    pub blob: u64,
    pub index: u32,
}

/// The animation units a glyph belongs to (TX-023), as indices counted
/// over the whole text in logical order:
/// - `grapheme`: the extended grapheme cluster (UAX #29) containing the
///   glyph's cluster, counting every cluster, spaces and line breaks
///   included;
/// - `word`: the UAX #29 word (a segment with at least one alphanumeric
///   character); glyphs between words (spaces, punctuation) belong to the
///   word before them (the first word for leading ones);
/// - `line`: the laid-out line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct UnitIds {
    pub grapheme: u32,
    pub word: u32,
    pub line: u32,
}

/// One positioned glyph of the layout, at rest (before curve or motion).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    pub font: GlyphFont,
    /// Font size in layout px.
    pub size: f32,
    pub glyph_id: u16,
    /// Pen position on the baseline, in layout px (box coordinates).
    pub origin: (f32, f32),
    pub advance: f32,
    /// Byte offset of the glyph's cluster in [`TextSpec::text`].
    pub cluster: u32,
    pub unit: UnitIds,
}

/// The measured text block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextLayout {
    /// Box size in layout px (see the module docs of [`crate::text`]).
    pub size: (f32, f32),
    pub lines: Vec<LineInfo>,
    /// Every glyph, in visual order line by line.
    pub glyphs: Vec<PlacedGlyph>,
    /// The family the text was set in (Noto Sans when the requested one is
    /// missing).
    pub family: String,
    pub fallback: FallbackReport,
    /// Whether the paragraph reads right to left (its base direction).
    pub rtl: bool,
    /// The text's font size in layout px: the em that per-unit animation
    /// lengths ([`crate::text::animate::GlyphAnim`]) measure in.
    pub em: f32,
}

/// A glyph ready to rasterize.
#[derive(Debug, Clone)]
pub(super) struct GlyphItem {
    pub font: FontData,
    pub size: f32,
    pub embolden: bool,
    pub skew: f32,
    pub coords: Arc<[i16]>,
    pub id: u16,
    /// Rest pen position (alignment applied), layout px.
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    pub cluster: u32,
    pub unit: UnitIds,
}

/// A stretch of one line drawn with one font's decoration metrics.
#[derive(Debug, Clone, Copy)]
pub(super) struct DecoRun {
    pub line: u32,
    pub x0: f32,
    pub x1: f32,
    pub baseline: f32,
    /// (top, thickness) in layout px, y down, for underline and strike.
    pub underline: (f32, f32),
    pub strike: (f32, f32),
}

/// The advance box of one non-blank grapheme cluster (per-character boxes).
#[derive(Debug, Clone, Copy)]
pub(super) struct ClusterBox {
    /// Byte offset of the cluster in the text.
    pub cluster: u32,
    pub line: u32,
    pub x0: f32,
    pub x1: f32,
}

pub(super) struct Built {
    pub layout: Layout<()>,
    pub lines: Vec<LineInfo>,
    pub items: Vec<GlyphItem>,
    pub decorations: Vec<DecoRun>,
    pub clusters: Vec<ClusterBox>,
    /// x-height of each line (layout px), for lower-half boxes.
    pub x_heights: Vec<f32>,
    pub box_width: f32,
    pub rtl: bool,
    family_missing: bool,
    family: String,
}

pub(super) struct Engine {
    fonts: FontContext,
    layouts: LayoutContext<()>,
    /// Family names of fonts seen in layouts, by (blob id, index).
    names: HashMap<(u64, u32), String>,
}

/// Grapheme and word indices by byte offset.
struct Units {
    /// Start byte of every grapheme cluster, ascending.
    graphemes: Vec<usize>,
    /// Start byte of every word, ascending.
    words: Vec<usize>,
}

impl Units {
    fn new(text: &str) -> Units {
        Units {
            graphemes: text.grapheme_indices(true).map(|(i, _)| i).collect(),
            words: text.unicode_word_indices().map(|(i, _)| i).collect(),
        }
    }

    /// Index of the last start at or before `byte`.
    fn index(starts: &[usize], byte: usize) -> u32 {
        let n = starts.partition_point(|&s| s <= byte);
        n.saturating_sub(1) as u32
    }

    fn ids(&self, byte: usize, line: u32) -> UnitIds {
        UnitIds {
            grapheme: Units::index(&self.graphemes, byte),
            word: Units::index(&self.words, byte),
            line,
        }
    }
}

impl Engine {
    pub fn new(collection: Collection) -> Engine {
        Engine {
            fonts: FontContext {
                collection,
                source_cache: SourceCache::default(),
            },
            layouts: LayoutContext::new(),
            names: HashMap::new(),
        }
    }

    pub fn build(&mut self, spec: &TextSpec) -> Built {
        let found = self
            .fonts
            .collection
            .family_by_name(&spec.family)
            .map(|f| f.name().to_owned());
        let family_missing = found.is_none();
        let family = found.unwrap_or_else(|| DEFAULT_FAMILY.to_owned());
        let mut stack = vec![FontFamilyName::Named(Cow::Owned(family.clone()))];
        if !family.eq_ignore_ascii_case(DEFAULT_FAMILY) {
            stack.push(FontFamilyName::Named(Cow::Borrowed(DEFAULT_FAMILY)));
        }

        // A stray CR (CRLF is normalized upstream) must not act as a second
        // break or shape as a missing glyph.
        let text: Cow<'_, str> = if spec.text.contains('\r') {
            Cow::Owned(spec.text.replace('\r', " "))
        } else {
            Cow::Borrowed(&spec.text)
        };
        let wrap = spec.wrap_width();
        let letter_spacing = spec.letter_spacing_px();
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, &text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Owned(
            stack,
        ))));
        builder.push_default(StyleProperty::FontSize(spec.size()));
        builder.push_default(StyleProperty::FontWeight(if spec.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        }));
        builder.push_default(StyleProperty::FontStyle(if spec.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }));
        builder.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(
            spec.spacing(),
        )));
        // Parley adds the spacing after every atom (never inside a joined
        // run or a ligature); the gap after a line's last cluster is taken
        // out of the line width below.
        builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
        // The ligature policy: optional ligatures off while letters move
        // one by one or are spaced apart (`rlig`, `calt` and complex
        // scripts' shaping stay on).
        if !spec.forms_ligatures() {
            builder.push_default(StyleProperty::FontFeatures(FontFeatures::Source(
                Cow::Borrowed(r#""liga" 0, "clig" 0, "dlig" 0"#),
            )));
        }
        builder.push_default(StyleProperty::WhiteSpaceCollapse(
            WhiteSpaceCollapse::Preserve,
        ));
        builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::BreakWord));
        // Always `Wrap`: without a box the line width is unlimited, so only
        // explicit breaks end lines, and trailing spaces still hang (they
        // don't count for width or alignment, TX-005).
        builder.push_default(StyleProperty::TextWrapMode(TextWrapMode::Wrap));
        let mut layout = builder.build(&text);
        layout.break_all_lines(wrap);
        // Parley sets every line flush left (or justifies it); we align
        // ourselves so trailing spaces never count (TX-005; parley keeps
        // preserved spaces before a break when they fit).
        let justify = spec.align == TextAlign::Justify && wrap.is_some();
        layout.align(
            if justify {
                Alignment::Justify
            } else {
                Alignment::Left
            },
            AlignmentOptions {
                align_when_overflowing: true,
                last_line_alignment: None,
            },
        );
        let rtl = layout.is_rtl();
        let mut lines = Vec::with_capacity(layout.len());
        let mut extents = Vec::with_capacity(layout.len());
        let mut justified = Vec::with_capacity(layout.len());
        for line in layout.lines() {
            let m = line.metrics();
            // Trailing whitespace in visual order: on the right of an LTR
            // paragraph's lines, on the left of an RTL one's.
            let mut clusters = Vec::new();
            for run in line.runs() {
                clusters.extend(run.visual_clusters().map(|c| {
                    let blank = text
                        .get(c.text_range())
                        .is_none_or(|t| t.chars().all(char::is_whitespace));
                    (blank, c.advance())
                }));
            }
            // How many clusters at the logical end are blank.
            let blank_end = if rtl {
                clusters.iter().take_while(|(blank, _)| *blank).count()
            } else {
                clusters
                    .iter()
                    .rev()
                    .take_while(|(blank, _)| *blank)
                    .count()
            };
            let content = if rtl {
                &clusters[blank_end..]
            } else {
                &clusters[..clusters.len() - blank_end]
            };
            // Summed from the clusters (not `advance - trailing`), so text
            // with and without trailing spaces measures exactly the same.
            // Letter spacing follows every cluster visually, so the line's
            // visually last one carries a gap that isn't content.
            let mut width: f32 = content.iter().map(|(_, a)| *a).sum();
            if !content.is_empty() {
                width = (width - letter_spacing).max(0.0);
            }
            let start = m.offset
                + if rtl {
                    clusters[..blank_end].iter().map(|(_, a)| *a).sum::<f32>()
                } else {
                    0.0
                };
            extents.push((start, width));
            justified.push(justify && line.break_reason() == BreakReason::Regular);
            lines.push(LineInfo {
                top: m.block_min_coord,
                bottom: m.block_max_coord,
                baseline: m.baseline,
                left: 0.0,
                width,
            });
        }
        let box_width =
            wrap.unwrap_or_else(|| lines.iter().map(|l| l.width).fold(0.0_f32, f32::max));
        let mut shifts = Vec::with_capacity(lines.len());
        for ((line, (start, width)), justified) in lines.iter_mut().zip(extents).zip(justified) {
            if justified {
                // Parley spread the line over the box; keep its positions.
                line.left = 0.0;
                line.width = box_width;
                shifts.push(0.0);
                continue;
            }
            let left = match spec.align {
                TextAlign::Left | TextAlign::Justify => 0.0,
                TextAlign::Center => (box_width - width) / 2.0,
                TextAlign::Right => box_width - width,
            };
            line.left = left;
            shifts.push(left - start);
        }
        let units = Units::new(&text);
        let mut built = Built {
            layout,
            lines,
            items: Vec::new(),
            decorations: Vec::new(),
            clusters: Vec::new(),
            x_heights: Vec::new(),
            box_width,
            rtl,
            family_missing,
            family,
        };
        flatten(
            &mut built,
            &shifts,
            &text,
            &units,
            spec.size(),
            letter_spacing,
        );
        built
    }

    pub fn describe(&mut self, built: &Built, spec: &TextSpec) -> TextLayout {
        let height = built.lines.last().map_or(0.0, |l| l.bottom);
        TextLayout {
            size: (built.box_width.max(0.0), height.max(0.0)),
            lines: built.lines.clone(),
            glyphs: built
                .items
                .iter()
                .map(|g| PlacedGlyph {
                    font: GlyphFont {
                        blob: g.font.data.id(),
                        index: g.font.index,
                    },
                    size: g.size,
                    glyph_id: g.id,
                    origin: (g.x, g.y),
                    advance: g.advance,
                    cluster: g.cluster,
                    unit: g.unit,
                })
                .collect(),
            family: built.family.clone(),
            fallback: self.fallback(built, spec),
            rtl: built.rtl,
            em: spec.size(),
        }
    }

    fn fallback(&mut self, built: &Built, spec: &TextSpec) -> FallbackReport {
        let mut families = Vec::new();
        let mut missing = Vec::new();
        for line in built.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let font = run.run().font();
                let key = (font.data.id(), font.index);
                let name = self
                    .names
                    .entry(key)
                    .or_insert_with(|| family_name(font.data.data(), font.index));
                if !name.eq_ignore_ascii_case(&built.family) && !name.is_empty() {
                    families.push(name.clone());
                }
                for cluster in run.run().clusters() {
                    if cluster.glyphs().any(|g| g.id == 0) {
                        let range = cluster.text_range();
                        if let Some(text) = spec.text.get(range) {
                            missing.extend(text.chars().filter(|c| !c.is_whitespace()));
                        }
                    }
                }
            }
        }
        families.sort();
        families.dedup();
        let mut seen = std::collections::HashSet::new();
        missing.retain(|c| seen.insert(*c));
        FallbackReport {
            family_missing: built.family_missing,
            fallback_families: families,
            missing_chars: missing,
        }
    }
}

/// Fills `built`'s glyph items, decoration runs, cluster boxes and
/// x-heights from its parley layout.
fn flatten(
    built: &mut Built,
    shifts: &[f32],
    text: &str,
    units: &Units,
    em: f32,
    letter_spacing: f32,
) {
    let mut items = Vec::new();
    let mut decorations = Vec::new();
    let mut boxes = Vec::new();
    let mut x_heights = Vec::new();
    for (index, (line, shift)) in built.layout.lines().zip(shifts.iter().copied()).enumerate() {
        let line_no = index as u32;
        let info = built.lines[index];
        let mut x_height = None;
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let r = run.run();
            let font = r.font();
            let metrics = r.font_metrics();
            x_height = x_height.or(metrics.x_height.filter(|h| *h > 0.0));
            let synthesis = r.synthesis();
            let coords: Arc<[i16]> = r.normalized_coords().iter().map(|c| c.to_bits()).collect();
            // The cluster (byte offset) of every glyph, in the visual order
            // `positioned_glyphs` yields them.
            let mut owners = Vec::new();
            let mut pen = run.offset() + shift;
            for c in r.visual_clusters() {
                let start = c.text_range().start;
                owners.extend(c.glyphs().map(|_| start));
                let blank = text
                    .get(c.text_range())
                    .is_none_or(|t| t.chars().all(char::is_whitespace));
                if !blank {
                    let width = (c.advance() - letter_spacing).max(0.0);
                    boxes.push(ClusterBox {
                        cluster: start as u32,
                        line: line_no,
                        x0: pen,
                        x1: pen + width,
                    });
                }
                pen += c.advance();
            }
            let fallback_owner = r.text_range().start;
            for (i, g) in run.positioned_glyphs().enumerate() {
                let Ok(id) = u16::try_from(g.id) else {
                    continue;
                };
                let cluster = owners.get(i).copied().unwrap_or(fallback_owner);
                items.push(GlyphItem {
                    font: font.clone(),
                    size: r.font_size(),
                    embolden: synthesis.embolden(),
                    skew: synthesis.skew().unwrap_or(0.0),
                    coords: Arc::clone(&coords),
                    id,
                    x: g.x + shift,
                    y: g.y,
                    advance: g.advance,
                    cluster: cluster as u32,
                    unit: units.ids(cluster, line_no),
                });
            }
            // Decorations cover the run, clipped to the line's content so
            // trailing spaces and the last letter gap stay bare.
            let x0 = (run.offset() + shift).max(info.left);
            let x1 = (run.offset() + shift + run.advance()).min(info.left + info.width);
            if x1 > x0 {
                let fallback = em / 18.0;
                let thick = |size: f32| if size > 0.0 { size } else { fallback };
                let baseline = run.baseline();
                // Font offsets are y up: the top of the stroke.
                decorations.push(DecoRun {
                    line: line_no,
                    x0,
                    x1,
                    baseline,
                    underline: (
                        baseline - metrics.underline_offset,
                        thick(metrics.underline_size),
                    ),
                    strike: (
                        baseline - metrics.strikethrough_offset,
                        thick(metrics.strikethrough_size),
                    ),
                });
            }
        }
        x_heights.push(x_height.unwrap_or(0.5 * em));
    }
    built.items = items;
    built.decorations = decorations;
    built.clusters = boxes;
    built.x_heights = x_heights;
}

/// The typographic (or legacy) family name of a font, "" when unreadable.
fn family_name(data: &[u8], index: u32) -> String {
    use swash::StringId;
    let Some(font) = swash::FontRef::from_index(data, index as usize) else {
        return String::new();
    };
    let strings = font.localized_strings();
    [StringId::TypographicFamily, StringId::Family]
        .into_iter()
        .find_map(|id| {
            strings
                .find_by_id(id, Some("en"))
                .or_else(|| strings.find_by_id(id, None))
        })
        .map(|s| s.chars().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_count_graphemes_and_words() {
        let text = "Hi, wo\u{301}rld!\nok";
        let u = Units::new(text);
        // "o" + combining acute is one grapheme.
        assert_eq!(u.ids(5, 0).grapheme, 5);
        assert_eq!(u.ids(6, 0).grapheme, 5);
        assert_eq!(u.ids(0, 0).word, 0);
        // The comma and space belong to the first word.
        assert_eq!(u.ids(2, 0).word, 0);
        assert_eq!(u.ids(4, 0).word, 1);
        let ok = text.find("ok").unwrap();
        assert_eq!(u.ids(ok, 1).word, 2);
        assert_eq!(u.ids(ok, 1).line, 1);
    }
}

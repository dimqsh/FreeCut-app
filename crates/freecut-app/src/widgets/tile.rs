//! The media tile and its grid (ui-concept §13.23): auto-filled columns
//! that always fill the width, 16:9 cover-cropped thumbnails on the kind's
//! well, duration badges, hover / selected states and the rubber band.
//!
//! Every rule with a number in it is a pure function ([`grid_layout`],
//! [`cover_uv`], [`short_duration`], [`rubber_band_hits`]); the painter
//! only calls them.

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Shape, Stroke, StrokeKind, TextureId, Vec2};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick};

use super::side_list::one_line;
use crate::theme::{self, Type, fonts, icons};
use crate::waveform::Peaks;

/// Space for the name under a thumbnail: 4 pt gap and a 14 pt line.
pub const NAME_BAND: f32 = 18.0;
/// The hover-only buttons in a thumbnail's corners.
pub const CORNER_BUTTON: f32 = 18.0;

/// The grid's columns and tile width for a `width` pt wide area with
/// tiles at least `min` wide and `gap` apart: as many columns as fit
/// (at least one), widened so they fill the width exactly.
pub fn grid_layout(width: f32, min: f32, gap: f32) -> (usize, f32) {
    let width = width.max(0.0);
    let cols = ((width + gap) / (min + gap)).floor().max(1.0);
    let tile = ((width - (cols - 1.0) * gap) / cols).max(0.0);
    // `cols` is at least 1 and at most width / gap: small.
    (cols as usize, tile)
}

/// The height of one grid row (thumbnail and name) plus the gap below it.
pub fn row_pitch(tile_w: f32, gap: f32) -> f32 {
    tile_w * 9.0 / 16.0 + NAME_BAND + gap
}

/// The cell of tile `index` in a grid of `cols` columns of `tile_w`,
/// `gap` apart, with the first cell at `origin`.
pub fn cell_rect(origin: Pos2, index: usize, cols: usize, tile_w: f32, gap: f32) -> Rect {
    let cols = cols.max(1);
    let (row, col) = (index / cols, index % cols);
    let min = origin
        + Vec2::new(
            col as f32 * (tile_w + gap),
            row as f32 * row_pitch(tile_w, gap),
        );
    Rect::from_min_size(min, Vec2::new(tile_w, tile_w * 9.0 / 16.0 + NAME_BAND))
}

/// The part of a texture (as UV coordinates) that **covers** a frame:
/// centred, the longer axis cropped, the whole texture when the aspects
/// match. Aspects are width / height.
pub fn cover_uv(tex_aspect: f32, frame_aspect: f32) -> Rect {
    let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if !(tex_aspect > 0.0 && frame_aspect > 0.0) || !tex_aspect.is_finite() {
        return full;
    }
    if tex_aspect > frame_aspect {
        // Wider than the frame: keep the full height, crop the sides.
        let w = frame_aspect / tex_aspect;
        Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0))
    } else if tex_aspect < frame_aspect {
        let h = tex_aspect / frame_aspect;
        Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0))
    } else {
        full
    }
}

/// A duration badge: `m:ss`, or `h:mm:ss` from an hour, to the nearest
/// second; no frames.
pub fn short_duration(t: Tick) -> String {
    let ticks = t.raw().max(0);
    let secs =
        ticks / TICKS_PER_SECOND + i64::from(ticks % TICKS_PER_SECOND >= TICKS_PER_SECOND / 2);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A still's badge: its file extension in capitals ("PNG"), else "Image".
pub fn still_format(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.len() <= 5)
        .map_or_else(|| "Image".to_owned(), str::to_uppercase)
}

/// The tiles a rubber band touches, by index.
pub fn rubber_band_hits(band: Rect, tiles: &[Rect]) -> Vec<usize> {
    tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| band.intersects(**t))
        .map(|(i, _)| i)
        .collect()
}

/// The thumbnail part of a tile's cell.
pub fn thumb_rect(cell: Rect) -> Rect {
    Rect::from_min_size(cell.min, Vec2::new(cell.width(), cell.width() * 9.0 / 16.0))
}

/// The hover-only button in a thumbnail's top-right corner.
pub fn corner_button_rect(thumb: Rect) -> Rect {
    Rect::from_min_size(
        Pos2::new(thumb.right() - 4.0 - CORNER_BUTTON, thumb.top() + 4.0),
        Vec2::splat(CORNER_BUTTON),
    )
}

/// What a thumbnail shows, best first.
pub enum Picture<'a> {
    /// A poster texture of `size` pixels, cover-cropped.
    Texture(TextureId, Vec2),
    /// An audio item's waveform across the thumbnail.
    Wave(&'a Peaks, Tick),
    /// A kind glyph, centred.
    Glyph(&'a str),
    /// Custom content painted by the caller (the Add text sample).
    Custom,
}

/// One tile's content and state.
pub struct TileSpec<'a> {
    pub name: &'a str,
    /// The kind colour (`clip.*`); the well and the waveform derive from
    /// it.
    pub mark: Color32,
    pub picture: Picture<'a>,
    /// Bottom-right: a duration or a still's format.
    pub badge: Option<String>,
    /// Top-left: a word such as "Extracted".
    pub tag: Option<&'a str>,
    pub offline: bool,
    pub selected: bool,
    pub hovered: bool,
    /// Whether the corner button is under the pointer (`danger` for a
    /// remove button, `accent` for an add button).
    pub corner: Option<Corner>,
}

/// The hover-only corner button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    Remove { hot: bool },
    Add { hot: bool },
}

/// Paints a tile into `cell` (thumbnail on top, name under it).
pub fn paint_tile(ui: &egui::Ui, painter: &egui::Painter, cell: Rect, spec: &TileSpec<'_>) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let thumb = thumb_rect(cell);
    let well = theme::surface::clip_well(spec.mark, look.kind);
    painter.rect_filled(thumb, theme::R_MD, well);
    let clipped = painter.with_clip_rect(thumb.intersect(painter.clip_rect()));
    match spec.picture {
        Picture::Texture(tex, size) => {
            let uv = cover_uv(
                size.x / size.y.max(1.0),
                thumb.width() / thumb.height().max(1.0),
            );
            // Rounded image corners: `rect_filled` with a texture brush.
            clipped.add(
                egui::epaint::RectShape::filled(thumb, theme::R_MD, Color32::WHITE)
                    .with_texture(tex, uv),
            );
        }
        Picture::Wave(peaks, duration) => {
            let band = thumb.shrink2(Vec2::new(6.0, thumb.height() * 0.22));
            waveform(
                &clipped,
                band,
                peaks,
                duration,
                theme::surface::clip_wave(spec.mark, look.kind),
            );
        }
        Picture::Glyph(glyph) => {
            clipped.text(
                thumb.center(),
                Align2::CENTER_CENTER,
                glyph,
                fonts::icon_font(ui.ctx(), 24.0, false),
                theme::surface::clip_wave(spec.mark, look.kind).gamma_multiply(0.7),
            );
        }
        Picture::Custom => {}
    }
    if spec.offline {
        clipped.rect_filled(thumb, theme::R_MD, p.overlay_badge);
    }
    let micro = Type::Micro.style(ui.style()).resolve(ui.style());
    let pill = |at: Pos2, align: Align2, text: &str, color: Color32| {
        let galley = clipped.layout_no_wrap(text.to_owned(), micro.clone(), color);
        let size = galley.size() + Vec2::new(7.0, 2.0);
        let rect = align.anchor_size(at, size);
        clipped.rect_filled(rect, theme::R_XS, p.overlay_badge);
        clipped.galley(rect.min + Vec2::new(3.5, 1.0), galley, color);
    };
    if spec.offline {
        pill(
            thumb.left_top() + Vec2::new(4.0, 4.0),
            Align2::LEFT_TOP,
            "Offline",
            p.danger,
        );
    } else if let Some(tag) = spec.tag {
        pill(
            thumb.left_top() + Vec2::new(4.0, 4.0),
            Align2::LEFT_TOP,
            tag,
            Color32::WHITE,
        );
    }
    if let Some(badge) = &spec.badge {
        pill(
            thumb.right_bottom() - Vec2::new(4.0, 4.0),
            Align2::RIGHT_BOTTOM,
            badge,
            Color32::WHITE,
        );
    }
    if let Some(corner) = spec.corner {
        let r = corner_button_rect(thumb);
        let (glyph, fill) = match corner {
            Corner::Remove { hot } => (icons::CLOSE, if hot { p.danger } else { p.overlay_badge }),
            Corner::Add { hot } => (icons::PLUS, if hot { p.accent } else { p.overlay_badge }),
        };
        let ink = match corner {
            Corner::Remove { hot: true } | Corner::Add { hot: true } => {
                theme::surface::label_on(fill)
            }
            _ => Color32::WHITE,
        };
        clipped.rect_filled(r, theme::R_XS, fill);
        clipped.text(
            r.center(),
            Align2::CENTER_CENTER,
            glyph,
            fonts::icon_font(ui.ctx(), 10.0, false),
            ink,
        );
    }
    // The outline: rest hairline, hover strong, selected 2 pt accent.
    let stroke = if spec.selected {
        Stroke::new(2.0, p.accent)
    } else if spec.hovered {
        Stroke::new(1.0, p.border_strong)
    } else {
        Stroke::new(1.0, p.border_subtle)
    };
    painter.rect_stroke(thumb, theme::R_MD, stroke, StrokeKind::Inside);
    let name_color = if spec.selected {
        p.text_primary
    } else {
        p.text_secondary
    };
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = one_line(ui, spec.name, font, name_color, cell.width());
    painter.galley(
        Pos2::new(cell.left(), thumb.bottom() + 4.0),
        galley,
        name_color,
    );
}

/// The whole item's waveform as bars across `band`, one per 3 pt.
pub fn waveform(
    painter: &egui::Painter,
    band: Rect,
    peaks: &Peaks,
    duration: Tick,
    color: Color32,
) {
    let columns = (band.width() / 3.0).floor().max(1.0) as i64;
    let total = i128::from(duration.raw().max(0));
    let at = |k: i64| {
        // k / columns of the way through; fits i64 because it is at most
        // `duration`.
        let t = total * i128::from(k) / i128::from(columns);
        Tick::from_raw(i64::try_from(t).unwrap_or(i64::MAX))
    };
    let cy = band.center().y;
    let half = band.height() / 2.0;
    let step = band.width() / columns as f32;
    let mut shapes = Vec::new();
    for k in 0..columns {
        if let Some((lo, hi)) = peaks.range(at(k), at(k + 1)) {
            let amp = hi.abs().max(lo.abs()).clamp(0.0, 1.0);
            let h = (amp * half).max(1.0);
            let x = band.left() + k as f32 * step + step / 2.0;
            shapes.push(Shape::line_segment(
                [Pos2::new(x, cy - h), Pos2::new(x, cy + h)],
                Stroke::new((step - 1.0).clamp(1.0, 2.0), color),
            ));
        }
    }
    painter.extend(shapes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_columns_fill_the_width_exactly() {
        for (min, gap) in [(96.0, 8.0), (80.0, 6.0)] {
            for width in [0.0, 50.0, 96.0, 150.0, 200.0, 233.0, 361.5, 999.0] {
                let (cols, tile) = grid_layout(width, min, gap);
                assert!(cols >= 1);
                let used = cols as f32 * tile + (cols as f32 - 1.0) * gap;
                if width >= min {
                    assert!((used - width).abs() < 0.01, "{width}: {used}");
                    assert!(tile >= min - 0.001, "{width}: {tile}");
                    // One more column wouldn't fit at the minimum.
                    let more = (cols as f32 + 1.0) * min + cols as f32 * gap;
                    assert!(more > width, "{width}");
                }
            }
        }
        assert_eq!(grid_layout(233.0, 96.0, 8.0).0, 2);
        assert_eq!(grid_layout(95.0, 96.0, 8.0), (1, 95.0));
    }

    #[test]
    fn cells_step_by_tile_and_pitch() {
        let (cols, w) = (3, 100.0);
        let a = cell_rect(Pos2::ZERO, 4, cols, w, 8.0);
        assert_eq!(a.min, Pos2::new(108.0, row_pitch(w, 8.0)));
        assert!((a.height() - (56.25 + NAME_BAND)).abs() < 1e-4);
    }

    #[test]
    fn cover_crops_the_longer_axis_around_the_centre() {
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        assert_eq!(cover_uv(16.0 / 9.0, 16.0 / 9.0), full);
        assert_eq!(cover_uv(0.0, 16.0 / 9.0), full);
        // A square poster in a 16:9 frame: full width, the middle 9/16.
        let uv = cover_uv(1.0, 16.0 / 9.0);
        assert!((uv.width() - 1.0).abs() < 1e-6);
        assert!((uv.height() - 9.0 / 16.0).abs() < 1e-6);
        assert!((uv.center().y - 0.5).abs() < 1e-6);
        // A 2.39:1 frame grab in 16:9: the middle of the width.
        let uv = cover_uv(2.39, 16.0 / 9.0);
        assert!((uv.height() - 1.0).abs() < 1e-6);
        assert!(uv.width() < 1.0 && uv.min.x > 0.0);
        assert!((uv.center().x - 0.5).abs() < 1e-6);
        for uv in [cover_uv(0.3, 1.7), cover_uv(5.0, 1.7)] {
            assert!(full.contains_rect(uv), "{uv:?}");
        }
    }

    #[test]
    fn durations_round_to_the_second() {
        let ms = |n: i64| Tick::from_raw(TICKS_PER_SECOND / 1000 * n);
        assert_eq!(short_duration(Tick::ZERO), "0:00");
        assert_eq!(short_duration(ms(499)), "0:00");
        assert_eq!(short_duration(ms(500)), "0:01");
        assert_eq!(short_duration(ms(22_000)), "0:22");
        assert_eq!(short_duration(ms(59_400)), "0:59");
        assert_eq!(short_duration(ms(59_600)), "1:00");
        assert_eq!(
            short_duration(Tick::from_secs(754).unwrap_or(Tick::ZERO)),
            "12:34"
        );
        assert_eq!(
            short_duration(Tick::from_secs(3_600 + 5 * 60 + 9).unwrap_or(Tick::ZERO)),
            "1:05:09"
        );
        assert_eq!(short_duration(Tick::from_raw(-5)), "0:00");
    }

    #[test]
    fn stills_show_their_format() {
        assert_eq!(still_format("/a/b/photo.png"), "PNG");
        assert_eq!(still_format("C:\\x\\y.JpEg"), "JPEG");
        assert_eq!(still_format("/a/b/noext"), "Image");
    }

    #[test]
    fn rubber_band_selects_what_it_touches() {
        let tiles: Vec<Rect> = (0..6)
            .map(|i| cell_rect(Pos2::ZERO, i, 3, 100.0, 8.0))
            .collect();
        let band = Rect::from_two_pos(Pos2::new(50.0, 10.0), Pos2::new(150.0, 20.0));
        assert_eq!(rubber_band_hits(band, &tiles), [0, 1]);
        let tall = Rect::from_two_pos(Pos2::new(250.0, 0.0), Pos2::new(260.0, 200.0));
        assert_eq!(rubber_band_hits(tall, &tiles), [2, 5]);
        // In the gap between tiles: nothing.
        let gap = Rect::from_two_pos(Pos2::new(101.0, 0.0), Pos2::new(107.0, 50.0));
        assert!(rubber_band_hits(gap, &tiles).is_empty());
    }
}

//! The timeline's geometry (ui-concept §13.26): where its bands sit, how
//! tall a lane is, where a clip, its name strip, its sound band and its
//! filmstrip tiles go, the ruler's tick interval, the playhead's head, the
//! wheel zoom and the edge autoscroll.
//!
//! Every rule with a number in it is a pure function here so it can be
//! tested without a window; `timeline.rs` only paints what these return.

use eframe::egui::{Pos2, Rangef, Rect, Vec2};
use freecut_timeline::model::TrackKind;
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick};

use crate::theme::Metrics;

/// The narrowest a zoomed timeline gets, and the widest, in points per
/// second.
pub const MIN_PPS: f32 = 2.0;
pub const MAX_PPS: f32 = 2_000.0;
/// What one zoom button press (and `+` / `-`) multiplies the zoom by.
pub const ZOOM_STEP: f32 = 1.25;
/// The labelled ruler ticks are at least this far apart.
pub const MIN_LABEL_SPACING: f32 = 88.0;
/// The edge zones where a drag scrolls the view along.
pub const AUTOSCROLL_ZONE: f32 = 40.0;
/// The most a drag in an edge zone scrolls per frame.
pub const AUTOSCROLL_STEP: f32 = 12.0;
/// A thin scrollbar's gutter (§13.24).
pub const SCROLLBAR: f32 = 8.0;
/// Grab area either side of the header column's hairline splitter.
pub const SPLITTER_GRAB: f32 = 3.0;
/// The gap kept above and below a clip inside its lane.
pub const CLIP_INSET: f32 = 4.0;
/// Names (and kind glyphs) hide on clips narrower than this.
pub const NAME_MIN_WIDTH: f32 = 32.0;
/// Badges show on clips wider than this.
pub const BADGE_MIN_WIDTH: f32 = 60.0;
/// The most filmstrip tiles one clip paints.
pub const MAX_TILES: usize = 120;

/// The bands of the timeline panel, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineRects {
    /// The sequence tab strip (the panel's header).
    pub tabs: Rect,
    pub toolbar: Rect,
    /// The header column's part of the ruler row: a plain block.
    pub corner: Rect,
    /// The ruler over the lanes.
    pub ruler: Rect,
    /// The track header column below the ruler.
    pub headers: Rect,
    /// The lanes below the ruler.
    pub lanes: Rect,
    /// The vertical thin scrollbar at the header column's right edge, clear
    /// of the splitter.
    pub vbar: Rect,
    /// The horizontal thin scrollbar along the bottom of the lanes.
    pub hbar: Rect,
    /// The hairline splitter between the header column and the lanes,
    /// with its grab area.
    pub splitter: Rect,
}

/// Splits the panel `rect` into its bands for a header column `header_w`
/// wide (already clamped, see [`clamp_header_width`]).
pub fn timeline_layout(rect: Rect, m: &Metrics, header_w: f32) -> TimelineRects {
    let x = rect.left() + header_w;
    let tabs_bottom = (rect.top() + m.tl_tabs).min(rect.bottom());
    let toolbar_bottom = (tabs_bottom + m.tl_toolbar).min(rect.bottom());
    let ruler_bottom = (toolbar_bottom + m.tl_ruler).min(rect.bottom());
    let tabs = Rect::from_x_y_ranges(rect.x_range(), Rangef::new(rect.top(), tabs_bottom));
    let toolbar = Rect::from_x_y_ranges(rect.x_range(), Rangef::new(tabs_bottom, toolbar_bottom));
    let ruler_y = Rangef::new(toolbar_bottom, ruler_bottom);
    // The horizontal scrollbar has a strip of its own under the lanes, so
    // it never covers the last lane's clips.
    let body_y = Rangef::new(ruler_bottom, (rect.bottom() - SCROLLBAR).max(ruler_bottom));
    let corner = Rect::from_x_y_ranges(Rangef::new(rect.left(), x), ruler_y);
    let ruler = Rect::from_x_y_ranges(Rangef::new(x, rect.right()), ruler_y);
    let headers = Rect::from_x_y_ranges(Rangef::new(rect.left(), x), body_y);
    let lanes = Rect::from_x_y_ranges(Rangef::new(x, rect.right()), body_y);
    // Clear of the splitter's grab area.
    let vbar = Rect::from_x_y_ranges(
        Rangef::new(x - SCROLLBAR - 2.0, x - 2.0),
        Rangef::new(body_y.min + 4.0, (body_y.max - 4.0).max(body_y.min + 4.0)),
    );
    let hbar = Rect::from_x_y_ranges(
        lanes.x_range(),
        Rangef::new(lanes.bottom(), rect.bottom().max(lanes.bottom())),
    );
    let splitter = Rect::from_x_y_ranges(
        Rangef::new(x - SPLITTER_GRAB, x + SPLITTER_GRAB),
        Rangef::new(toolbar_bottom, rect.bottom()),
    );
    TimelineRects {
        tabs,
        toolbar,
        corner,
        ruler,
        headers,
        lanes,
        vbar,
        hbar,
        splitter,
    }
}

/// The header column's width: `want`, kept between the density's minimum
/// and 40 % of the panel (the minimum wins in a very narrow panel).
pub fn clamp_header_width(want: f32, panel_w: f32, m: &Metrics) -> f32 {
    let max = (panel_w * 0.4).max(m.tl_header_min);
    want.clamp(m.tl_header_min, max)
}

/// A lane's height choice from its header's `...` menu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LaneSize {
    /// By what the track holds.
    #[default]
    Auto,
    Small,
    Medium,
    Large,
}

impl LaneSize {
    pub const ALL: [LaneSize; 4] = [
        LaneSize::Auto,
        LaneSize::Small,
        LaneSize::Medium,
        LaneSize::Large,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LaneSize::Auto => "Auto",
            LaneSize::Small => "Small",
            LaneSize::Medium => "Medium",
            LaneSize::Large => "Large",
        }
    }
}

/// A lane's height: `size`, or for *Auto* by content: the main track
/// Large, other video and image tracks and audio Medium, text, sticker,
/// effect and adjustment tracks Small, an empty track Medium (so a title,
/// an overlay, the main video and its sound fit a 1440 × 900 window's
/// timeline).
pub fn lane_height(
    kind: TrackKind,
    is_main: bool,
    empty: bool,
    size: LaneSize,
    m: &Metrics,
) -> f32 {
    let [small, medium, large] = m.lanes;
    match size {
        LaneSize::Small => small,
        LaneSize::Medium => medium,
        LaneSize::Large => large,
        LaneSize::Auto if empty => medium,
        LaneSize::Auto if is_main => large,
        LaneSize::Auto => match kind {
            TrackKind::Video | TrackKind::Audio => medium,
            TrackKind::Text | TrackKind::Sticker | TrackKind::Effect | TrackKind::Adjustment => {
                small
            }
        },
    }
}

/// Where a clip from `x0` to `x1` sits in `lane`: 4 pt in from the lane's
/// top and bottom, 1 pt short of its end so cuts show a gap, at least 2 pt
/// wide.
pub fn clip_rect(lane: Rect, x0: f32, x1: f32) -> Rect {
    let right = if x1 - x0 >= 4.0 { x1 - 1.0 } else { x1 };
    let top = lane.top() + CLIP_INSET;
    let bottom = (lane.bottom() - CLIP_INSET).max(top + 1.0);
    Rect::from_min_max(Pos2::new(x0, top), Pos2::new(right.max(x0 + 2.0), bottom))
}

/// How a clip's kind is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipShape {
    /// Video, image and audio: a coloured name strip over a dark body.
    TwoTone,
    /// Text, stickers, effects and adjustments: the whole clip coloured.
    AllStrip,
}

/// The name strip and the body of a two-tone clip at `r`; the strip
/// takes the whole clip when it is too short for a body.
pub fn strip_and_body(r: Rect, strip: f32) -> (Rect, Rect) {
    let split = (r.top() + strip).min(r.bottom());
    let top = Rect::from_x_y_ranges(r.x_range(), Rangef::new(r.top(), split));
    let body = Rect::from_x_y_ranges(r.x_range(), Rangef::new(split, r.bottom()));
    (top, body)
}

/// The height of the sound band along the bottom of a video clip's body
/// `body` pt tall: 30 % of it, between 10 and 28 pt, never taller than the
/// body.
pub fn sound_band_height(body: f32) -> f32 {
    (0.3 * body).clamp(10.0, 28.0).min(body.max(0.0))
}

/// The filmstrip tiles of a clip whose frame area is `frames`, for a source
/// of `aspect` (width / height), clipped to `visible`: each tile is as tall
/// as the area, `aspect` times as wide (at least 4 pt), counted from the
/// clip's left edge; only tiles on screen are returned (at most
/// [`MAX_TILES`]), each with the x of the middle of its visible part, so a
/// short or trimmed clip still shows its own frame.
pub fn filmstrip_tiles(frames: Rect, visible: Rect, aspect: f32) -> Vec<(Rect, f32)> {
    let shown = frames.intersect(visible);
    if frames.height() < 1.0 || shown.width() <= 0.0 || shown.height() <= 0.0 {
        return Vec::new();
    }
    let w = (frames.height() * aspect.max(0.01)).max(4.0);
    let first = ((shown.left() - frames.left()) / w).floor().max(0.0) as usize;
    let mut out = Vec::new();
    let mut k = first;
    loop {
        let left = frames.left() + k as f32 * w;
        if left >= shown.right() || out.len() >= MAX_TILES {
            break;
        }
        let tile =
            Rect::from_min_size(Pos2::new(left, frames.top()), Vec2::new(w, frames.height()));
        let part = tile.intersect(shown);
        if part.width() > 0.0 {
            out.push((tile, part.center().x));
        }
        k += 1;
    }
    out
}

/// The x of each waveform bar's left edge across `[left, right)`: bars
/// `width` wide on a `pitch` grid anchored at `anchor` (the clip's left
/// edge, so bars don't swim while the view scrolls).
pub fn wave_bars(anchor: f32, left: f32, right: f32, pitch: f32) -> Vec<f32> {
    if right <= left || pitch <= 0.0 {
        return Vec::new();
    }
    let first = ((left - anchor) / pitch).floor();
    let mut x = anchor + first * pitch;
    let mut out = Vec::new();
    while x < right {
        if x + pitch > left {
            out.push(x);
        }
        x += pitch;
    }
    out
}

/// Full scale at −1 dBFS: bar parts at or beyond it are "near clipping".
pub const HOT_LEVEL: f32 = 0.891_250_9;

/// Splits one bar of peak amplitude `amp` (0..=1 of full scale, after the
/// clip's gain) drawn `half` pt either side of `cy` into its normal part
/// and, when it reaches −1 dBFS, the hot parts beyond that level: returns
/// `(top, bottom, hot)` where `hot` is the distance from the centre at which
/// the hot part starts.
pub fn bar_extent(amp: f32, cy: f32, half: f32) -> (f32, f32, Option<f32>) {
    let a = amp.clamp(0.0, 1.0);
    let reach = (a * half).max(0.5);
    let hot = (a >= HOT_LEVEL).then_some(HOT_LEVEL * half);
    (cy - reach, cy + reach, hot)
}

/// The zoom after a wheel or pinch `factor` at `pps` points per second:
/// at most ×2 or ÷2 per event, within [`MIN_PPS`]..[`MAX_PPS`].
pub fn zoom_wheel(pps: f32, factor: f32) -> f32 {
    let f = if factor.is_finite() {
        factor.clamp(0.5, 2.0)
    } else {
        1.0
    };
    (pps * f).clamp(MIN_PPS, MAX_PPS)
}

/// The scroll (seconds at the lanes' left edge) that keeps the time under
/// the pointer, `dx` pt into the lanes, in place when the zoom goes from
/// `old` to `new` points per second. Never before 0.
pub fn zoom_about(scroll: f64, old: f32, new: f32, dx: f32) -> f64 {
    let at = scroll + f64::from(dx) / f64::from(old);
    (at - f64::from(dx) / f64::from(new)).max(0.0)
}

/// How far (in points, signed) a drag at `x` scrolls the view this frame:
/// nothing outside the 40 pt zones at the lanes' left and right edges,
/// up to 12 pt deeper in, towards that edge.
pub fn autoscroll_step(x: f32, lanes: Rangef) -> f32 {
    let depth = |d: f32| (d / AUTOSCROLL_ZONE).clamp(0.0, 1.0);
    let into_left = lanes.min + AUTOSCROLL_ZONE - x;
    let into_right = x - (lanes.max - AUTOSCROLL_ZONE);
    if lanes.span() <= 2.0 * AUTOSCROLL_ZONE {
        return 0.0;
    }
    if into_left > 0.0 {
        -AUTOSCROLL_STEP * depth(into_left).max(0.2)
    } else if into_right > 0.0 {
        AUTOSCROLL_STEP * depth(into_right).max(0.2)
    } else {
        0.0
    }
}

/// The playhead's pentagon head at `x`: an 11 pt-wide block 7 pt tall at
/// the top of the ruler, ending in a 5 pt point on the line. Clockwise from
/// the top left; the last point is the apex.
pub fn playhead_head(x: f32, ruler: Rect) -> [Pos2; 5] {
    let top = ruler.top() + 1.0;
    let half = 5.5;
    [
        Pos2::new(x - half, top),
        Pos2::new(x + half, top),
        Pos2::new(x + half, top + 7.0),
        Pos2::new(x, top + 12.0),
        Pos2::new(x - half, top + 7.0),
    ]
}

/// How opaque a ruler label spanning `span` is with the playhead's head
/// at `head`: hidden while the head ([`playhead_head`], 11 pt wide)
/// covers any of it, fully shown from [`LABEL_CLEARANCE`] beyond the head's
/// edge, and fading in between, so the head never sits on a timecode.
pub fn label_opacity(span: Rangef, head: f32) -> f32 {
    let half = 5.5;
    let gap = (span.min - (head + half)).max((head - half) - span.max);
    (gap / LABEL_CLEARANCE).clamp(0.0, 1.0)
}

/// The clear space between the playhead's head and a fully shown ruler
/// label.
pub const LABEL_CLEARANCE: f32 = 8.0;

/// The labelled ruler interval: the first of 1, 2, 5, 10 frames, 1, 2, 5,
/// 10, 15, 30 s, 1, 2, 5, 10, 30 min and 1 h whose spacing at `pps` points
/// per second is at least [`MIN_LABEL_SPACING`].
pub fn tick_interval(pps: f32, rate: FrameRate) -> Tick {
    let frame = rate.frame_ticks();
    let frames = [1, 2, 5, 10].map(|n| frame.checked_mul_int(n).unwrap_or(frame));
    let secs = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1_800, 3_600]
        .map(|s| Tick::from_raw(s * TICKS_PER_SECOND));
    let wide = |t: &Tick| t.as_secs_f64() * f64::from(pps) >= f64::from(MIN_LABEL_SPACING);
    frames
        .into_iter()
        .chain(secs)
        .find(wide)
        .unwrap_or(Tick::from_raw(3_600 * TICKS_PER_SECOND))
}

/// How many unlabelled ticks sit between two labelled ones.
pub const MINOR_TICKS: u32 = 4;

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact geometry

    use super::*;
    use crate::theme::Density;

    fn m() -> Metrics {
        Density::Comfortable.metrics()
    }

    #[test]
    fn the_bands_stack_and_the_header_column_splits_the_ruler() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(1000.0, 400.0));
        let r = timeline_layout(rect, &m(), 176.0);
        assert_eq!(r.tabs.height(), 32.0);
        assert_eq!(r.toolbar.top(), r.tabs.bottom());
        assert_eq!(r.toolbar.height(), 40.0);
        assert_eq!(r.ruler.height(), 28.0);
        assert_eq!(r.corner.right(), r.ruler.left());
        assert_eq!(r.ruler.left(), 186.0);
        assert_eq!(r.lanes.left(), 186.0);
        assert_eq!(r.headers.right(), r.lanes.left());
        assert_eq!(r.lanes.top(), 20.0 + 32.0 + 40.0 + 28.0);
        // The scrollbar's strip is under the lanes, not over them.
        assert_eq!(r.lanes.bottom(), 420.0 - SCROLLBAR);
        assert_eq!(r.headers.bottom(), r.lanes.bottom());
        assert_eq!(r.hbar.height(), SCROLLBAR);
        assert_eq!(r.hbar.top(), r.lanes.bottom());
        assert_eq!(r.hbar.bottom(), 420.0);
        assert_eq!(r.vbar.right(), r.headers.right() - 2.0);
        assert!(
            r.vbar.center().x + 2.0 <= r.splitter.left(),
            "the thumb stays clear of the splitter"
        );
        assert_eq!(r.vbar.width(), SCROLLBAR);
        // A panel too short for its bands squeezes the lanes to nothing.
        let flat = Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 60.0));
        let r = timeline_layout(flat, &m(), 176.0);
        assert!(r.lanes.height() <= 0.0);
        assert_eq!(r.toolbar.bottom(), 60.0);
    }

    #[test]
    fn the_header_column_stays_between_its_minimum_and_forty_percent() {
        assert_eq!(clamp_header_width(176.0, 1400.0, &m()), 176.0);
        assert_eq!(clamp_header_width(80.0, 1400.0, &m()), 140.0);
        assert_eq!(clamp_header_width(900.0, 1000.0, &m()), 400.0);
        // A narrow panel: the minimum wins.
        assert_eq!(clamp_header_width(176.0, 200.0, &m()), 140.0);
        let k = Density::Compact.metrics();
        assert_eq!(clamp_header_width(10.0, 1000.0, &k), 120.0);
    }

    #[test]
    fn auto_lanes_follow_what_the_track_holds() {
        let m = m();
        let h = |kind, main, empty| lane_height(kind, main, empty, LaneSize::Auto, &m);
        assert_eq!(h(TrackKind::Video, true, false), 96.0);
        assert_eq!(h(TrackKind::Video, false, false), 72.0, "an overlay");
        // A title, an overlay, the main video and its sound fit the
        // lanes of a 1440 × 900 window (about 240 pt) up to the sound's
        // start.
        let above_sound = h(TrackKind::Text, false, false)
            + h(TrackKind::Video, false, false)
            + h(TrackKind::Video, true, false);
        assert!(above_sound + 24.0 <= 240.0, "{above_sound}");
        assert_eq!(h(TrackKind::Audio, false, false), 72.0);
        for kind in [
            TrackKind::Text,
            TrackKind::Sticker,
            TrackKind::Effect,
            TrackKind::Adjustment,
        ] {
            assert_eq!(h(kind, false, false), 40.0);
        }
        assert_eq!(h(TrackKind::Text, false, true), 72.0, "empty: Medium");
        assert_eq!(h(TrackKind::Video, true, true), 72.0);
        // A chosen size wins over the content.
        assert_eq!(
            lane_height(TrackKind::Audio, false, false, LaneSize::Small, &m),
            40.0
        );
        assert_eq!(
            lane_height(TrackKind::Text, false, false, LaneSize::Large, &m),
            96.0
        );
        let k = Density::Compact.metrics();
        let sizes = [LaneSize::Small, LaneSize::Medium, LaneSize::Large]
            .map(|s| lane_height(TrackKind::Video, false, false, s, &k));
        assert_eq!(sizes, [34.0, 60.0, 80.0]);
    }

    #[test]
    fn clips_sit_inside_their_lane_with_a_gap_at_cuts() {
        let lane = Rect::from_min_size(Pos2::new(0.0, 100.0), Vec2::new(800.0, 96.0));
        let r = clip_rect(lane, 50.0, 150.0);
        assert_eq!((r.top(), r.bottom()), (104.0, 192.0));
        assert_eq!((r.left(), r.right()), (50.0, 149.0));
        // The next clip starting at the cut leaves a 1 pt gap.
        let next = clip_rect(lane, 150.0, 250.0);
        assert_eq!(next.left() - r.right(), 1.0);
        // A sliver stays at least 2 pt wide.
        let sliver = clip_rect(lane, 50.0, 50.5);
        assert_eq!(sliver.width(), 2.0);
        let (strip, body) = strip_and_body(r, 18.0);
        assert_eq!(strip.height(), 18.0);
        assert_eq!(body.top(), strip.bottom());
        assert_eq!(body.bottom(), r.bottom());
        // A clip shorter than the strip is all strip.
        let short = Rect::from_min_size(Pos2::ZERO, Vec2::new(40.0, 12.0));
        let (strip, body) = strip_and_body(short, 18.0);
        assert_eq!(strip, short);
        assert_eq!(body.height(), 0.0);
    }

    #[test]
    fn the_sound_band_is_thirty_percent_of_the_body_within_limits() {
        assert_eq!(sound_band_height(70.0), 21.0);
        assert_eq!(sound_band_height(20.0), 10.0);
        assert_eq!(sound_band_height(200.0), 28.0);
        assert_eq!(sound_band_height(6.0), 6.0, "never taller than the body");
        assert_eq!(sound_band_height(0.0), 0.0);
    }

    #[test]
    fn filmstrip_tiles_cover_only_what_shows() {
        let frames = Rect::from_min_size(Pos2::new(100.0, 0.0), Vec2::new(1000.0, 45.0));
        let visible = Rect::from_min_max(Pos2::new(300.0, -10.0), Pos2::new(500.0, 100.0));
        let tiles = filmstrip_tiles(frames, visible, 16.0 / 9.0);
        let w = 80.0;
        assert!(!tiles.is_empty());
        for (tile, mid) in &tiles {
            assert!((tile.width() - w).abs() < 1e-3);
            assert_eq!(tile.height(), 45.0);
            assert!(tile.right() > 300.0 && tile.left() < 500.0, "on screen");
            assert!(*mid >= tile.left().max(300.0) && *mid <= tile.right().min(500.0));
        }
        // Tiles are counted from the clip's left edge.
        assert_eq!((tiles[0].0.left() - 100.0) % w, 0.0);
        assert_eq!(tiles.len(), 3, "260..340, 340..420, 420..500");
        // The first tile is partly off screen: its time is under the middle
        // of the part that shows.
        assert_eq!(tiles[0].1, 320.0);
        // A clip narrower than one tile still gets a tile, at its middle.
        let short = Rect::from_min_size(Pos2::new(100.0, 0.0), Vec2::new(30.0, 45.0));
        let tiles = filmstrip_tiles(short, Rect::EVERYTHING, 16.0 / 9.0);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].1, 115.0, "the middle of the clip");
        let clipped = filmstrip_tiles(
            short,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(110.0, 50.0)),
            1.0,
        );
        assert_eq!(clipped[0].1, 105.0);
        // Nothing on screen, or no room: no tiles.
        let away = Rect::from_min_max(Pos2::new(2000.0, 0.0), Pos2::new(2100.0, 50.0));
        assert!(filmstrip_tiles(frames, away, 1.0).is_empty());
        let flat = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 0.5));
        assert!(filmstrip_tiles(flat, Rect::EVERYTHING, 1.0).is_empty());
        // A very long clip zoomed in still paints a bounded number.
        let long = Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0e6, 4.0));
        assert_eq!(
            filmstrip_tiles(long, Rect::EVERYTHING, 0.1).len(),
            MAX_TILES
        );
    }

    #[test]
    fn waveform_bars_are_on_a_fixed_pitch_from_the_clip_edge() {
        let bars = wave_bars(100.0, 100.0, 112.0, 3.0);
        assert_eq!(bars, vec![100.0, 103.0, 106.0, 109.0]);
        // Scrolled: the visible range starts mid-clip, bars stay on the grid.
        let bars = wave_bars(100.0, 104.0, 110.0, 3.0);
        assert_eq!(bars, vec![103.0, 106.0, 109.0]);
        assert!(wave_bars(0.0, 10.0, 10.0, 3.0).is_empty());
        let (top, bottom, hot) = bar_extent(0.5, 50.0, 20.0);
        assert_eq!((top, bottom, hot), (40.0, 60.0, None));
        let (_, _, hot) = bar_extent(0.95, 50.0, 20.0);
        assert!((hot.unwrap() - HOT_LEVEL * 20.0).abs() < 1e-4);
        // −1 dBFS exactly counts as hot.
        assert!(bar_extent(HOT_LEVEL, 0.0, 10.0).2.is_some());
        // Silence still draws a hairline.
        let (top, bottom, _) = bar_extent(0.0, 50.0, 20.0);
        assert_eq!(bottom - top, 1.0);
    }

    #[test]
    fn wheel_zoom_is_bounded_and_keeps_the_time_under_the_pointer() {
        assert_eq!(zoom_wheel(100.0, 1.1), 110.0);
        assert_eq!(zoom_wheel(100.0, 10.0), 200.0, "at most twice per event");
        assert_eq!(zoom_wheel(100.0, 0.01), 50.0, "at most half");
        assert_eq!(zoom_wheel(1_900.0, 2.0), MAX_PPS);
        assert_eq!(zoom_wheel(3.0, 0.5), MIN_PPS);
        assert_eq!(zoom_wheel(100.0, f32::NAN), 100.0);
        for (scroll, old, new, dx) in [(3.0, 60.0, 90.0, 250.0), (5.0, 200.0, 40.0, 33.0)] {
            let before = scroll + f64::from(dx) / f64::from(old);
            let s = zoom_about(scroll, old, new, dx);
            let after = s + f64::from(dx) / f64::from(new);
            let px = ((after - before) * f64::from(new)).abs();
            assert!(px <= 0.5, "{px}");
        }
        // Never scrolls before zero.
        assert_eq!(zoom_about(0.0, 60.0, 30.0, 100.0), 0.0);
    }

    #[test]
    fn dragging_near_an_edge_scrolls_towards_it() {
        let lanes = Rangef::new(200.0, 1000.0);
        assert_eq!(autoscroll_step(500.0, lanes), 0.0);
        assert_eq!(autoscroll_step(240.0, lanes), 0.0, "the zone's edge");
        assert!(autoscroll_step(230.0, lanes) < 0.0);
        assert!(autoscroll_step(990.0, lanes) > 0.0);
        assert_eq!(autoscroll_step(150.0, lanes), -AUTOSCROLL_STEP);
        assert_eq!(autoscroll_step(1100.0, lanes), AUTOSCROLL_STEP);
        // Deeper in scrolls faster.
        assert!(autoscroll_step(995.0, lanes) > autoscroll_step(965.0, lanes));
        // Lanes too narrow for two zones never scroll.
        assert_eq!(autoscroll_step(10.0, Rangef::new(0.0, 60.0)), 0.0);
    }

    #[test]
    fn a_ruler_label_fades_out_under_the_playhead_head() {
        let label = Rangef::new(104.0, 160.0);
        assert_eq!(label_opacity(label, 130.0), 0.0, "head on the label");
        assert_eq!(label_opacity(label, 100.0), 0.0, "head on its edge");
        assert_eq!(label_opacity(label, 90.0), 1.0, "clear to the left");
        assert_eq!(label_opacity(label, 174.0), 1.0, "clear to the right");
        let near = label_opacity(label, 94.5);
        assert!(near > 0.0 && near < 1.0, "fading: {near}");
    }

    #[test]
    fn the_playhead_head_is_a_pentagon_pointing_at_the_line() {
        let ruler = Rect::from_min_size(Pos2::new(0.0, 100.0), Vec2::new(500.0, 28.0));
        let head = playhead_head(250.0, ruler);
        let xs: Vec<f32> = head.iter().map(|p| p.x).collect();
        let width = xs.iter().copied().fold(f32::MIN, f32::max)
            - xs.iter().copied().fold(f32::MAX, f32::min);
        assert_eq!(width, 11.0);
        assert_eq!(head[3], Pos2::new(250.0, 113.0), "apex on the line");
        assert!(head.iter().all(|p| ruler.contains(*p)), "inside the ruler");
        assert_eq!(head[2].y - head[1].y, 7.0, "the block is 7 pt tall");
        assert_eq!(head[3].y - head[2].y, 5.0, "then a 5 pt point");
    }

    #[test]
    fn ruler_labels_take_the_first_interval_at_least_88_pt_apart() {
        let rate = FrameRate::new(30, 1).unwrap();
        let frame = rate.frame_ticks();
        let secs = |s: i64| Tick::from_raw(s * TICKS_PER_SECOND);
        // 30 fps at 3000 pt/s: a frame is 100 pt.
        assert_eq!(tick_interval(3_000.0, rate), frame);
        assert_eq!(
            tick_interval(1_500.0, rate),
            frame.checked_mul_int(2).unwrap()
        );
        assert_eq!(tick_interval(100.0, rate), secs(1));
        assert_eq!(tick_interval(60.0, rate), secs(2));
        assert_eq!(tick_interval(10.0, rate), secs(10));
        assert_eq!(tick_interval(2.0, rate), secs(60));
        assert_eq!(tick_interval(0.01, rate), secs(3_600));
        // Monotonic: zooming in never widens the interval.
        let mut last = Tick::MAX;
        for k in 0..200 {
            let pps = 0.5 * 1.07f32.powi(k);
            let step = tick_interval(pps, rate);
            assert!(step <= last, "{pps}");
            assert!(
                step.as_secs_f64() * f64::from(pps) >= 88.0 || step == secs(3_600),
                "{pps}"
            );
            last = step;
        }
    }
}

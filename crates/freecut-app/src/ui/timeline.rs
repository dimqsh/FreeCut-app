//! The timeline panel (TL-002, TL-012, TL-013, TL-015, TL-018, TL-020,
//! TL-028, TL-042; ui-concept §13.26): the sequence tab, the toolbar
//! (UI-011), the ruler, the track header column, the banded lanes with
//! their clips, the playhead, the thin scrollbars, the drag gestures and
//! the clip context menu (UI-035). Two-tone clips (video, pictures, audio)
//! have a name strip over a body holding a filmstrip and bar waveforms;
//! text, stickers, effects and adjustments are all strip. Clips with sound
//! carry their volume line and keyframe dots (AU-008, AU-009), and linked
//! sound that slipped shows an out-of-sync badge (TL-030).
//!
//! Gestures plan an edit on every pointer move and draw the plan's preview
//! as the ghost; nothing changes until the pointer is released, when the
//! final request is planned again and committed as one undo step. The
//! geometry is in `timeline_layout`; this file paints and handles input.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Layout, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, UiBuilder, Vec2,
};
use freecut_timeline::edit::{
    Edge, EditRequest, InsertReq, InsertTarget, MoveReq, NewClip, Plan, TrimReq,
};
use freecut_timeline::history::{Coalesce, GestureId};
use freecut_timeline::ids::{ClipId, MediaId, SeqId, TrackId};
use freecut_timeline::model::{
    Clip, ClipAudio, ClipContent, Document, EmbeddedAudio, GainDb, GeneratedKind, MarkerRef,
    MediaKind, MediaSource, PaletteColor, Pan, Sequence, Track, TrackAudio, TrackHeader, TrackKind,
    expected_start,
};
use freecut_timeline::snap::SnapIndex;
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange, Timecode};

use super::assets::AssetDrag;
use super::text::insert_text;
use super::timeline_layout::{self as layout, ClipShape, LaneSize, TimelineRects};
use super::timeline_markers::{self as markers, EditOutcome, Flag, MarkerEdit};
use super::timeline_toolbar::{self, ToolAction, ToolbarInput};
use super::timeline_tools::{FlagGrab, FlagSpot, TimelineTool, ToolState, blade_time};
use crate::clip_keys::{self, LinePart, VisibleKey};
use crate::clip_sound::{
    ClipCommand, SoundCommand, format_db, gain_db, mute_command, sound_command,
};
use crate::clip_streams::{self, AddStreamState};
use crate::session::Project;
use crate::skim::Skim;
use crate::theme::{self, Look, Metrics, Type, icons, surface};
use crate::thumbs::{
    FilmKey, FilmSource, Filmstrips, bucket_step, bucket_time, display_aspect, height_bucket,
};
use crate::timeline_commands::Pick;
use crate::waveform::{PeakState, Waveforms, column_source_range, peak_key};
use crate::widgets::icon_button::IconButton;
use crate::widgets::thin_scrollbar::{Axis, thin_scrollbar};

const EDGE_GRAB: f32 = 6.0;
const SNAP_PX: f32 = 8.0;
/// Default length of stills and generated items (TL-011).
const STILL_SECONDS: i64 = 5;
/// Waveform bars: 2 pt wide on a 3 pt pitch.
const BAR_W: f32 = 2.0;
const BAR_PITCH: f32 = 3.0;
/// The track header's buttons.
const HEADER_CHIP: f32 = 24.0;
const HEADER_GLYPH: f32 = 14.0;

/// View state of the timeline panel (not part of the document or history).
#[derive(Debug)]
pub struct TimelineView {
    /// Zoom.
    pub px_per_sec: f32,
    /// Leftmost visible time, in seconds (UI edge only).
    pub scroll_secs: f64,
    /// How far the track stack is scrolled, in points from its top.
    pub track_scroll: f32,
    pub snapping: bool,
    /// The pointer tool (TL-024): view state, Select when a project opens.
    pub tool: ToolState,
    /// *Select linked clips together* (TL-057): a per-user preference kept
    /// beside snapping, on by default.
    pub linked_selection: bool,
    /// The colour of the last marker added, for *next colour* (TL-037).
    pub last_marker_color: Option<PaletteColor>,
    /// Hover skim (TL-014): a per-user preference the app keeps in step
    /// with the settings file.
    pub skim: Skim,
    /// The time being skimmed, for the thin skim line (set by the app).
    pub skim_at: Option<Tick>,
    /// The sequence is playing (set by the app each frame).
    pub playing: bool,
    /// The take being recorded (AU-027), drawn as a growing red clip (set
    /// by the app).
    pub live_take: Option<LiveTake>,
    /// Where the toolbar's Record button was drawn: the recorder panel
    /// hangs above it.
    pub record_button: Rect,
    /// What the Record button shows (set by the app).
    pub record: timeline_toolbar::RecordButton,
    /// The marker editor popover, while open.
    marker_edit: Option<MarkerEdit>,
    /// The stack was scrolled to show a live take's new-track lane once;
    /// after that the user scrolls freely.
    take_lane_shown: bool,
    /// The header column's width set with its splitter; the density's
    /// default until then.
    header_w: Option<f32>,
    /// Lane heights picked in the track headers' menus: view state, not
    /// saved (ui-concept §18 #9).
    lane_size: HashMap<TrackId, LaneSize>,
    /// The name being typed in a track's menu.
    rename: Option<(TrackId, String)>,
    drag: Drag,
    /// What the open context menu is for.
    menu: Option<MenuFor>,
    /// The selected volume keyframe dots (AU-008).
    keys: Option<KeySelection>,
    /// Where the panel was drawn last frame (headless snapshots aim at it).
    shown: Rect,
}

/// Volume keyframe dots selected on one clip (AU-008): Delete removes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySelection {
    pub clip: ClipId,
    /// The keys' source times.
    pub times: Vec<Tick>,
}

impl TimelineView {
    /// The selected dots that still exist on a selected clip, if any.
    pub fn selected_keys(&self, project: &Project) -> Option<KeySelection> {
        let keys = self.keys.as_ref()?;
        if !project.selection.contains(&keys.clip) {
            return None;
        }
        let audio = project.doc().clip(keys.clip)?.audio.active()?.clone();
        let times: Vec<Tick> = keys
            .times
            .iter()
            .copied()
            .filter(|t| audio.gain.key_at(*t).is_some())
            .collect();
        (!times.is_empty()).then_some(KeySelection {
            clip: keys.clip,
            times,
        })
    }

    pub fn clear_keys(&mut self) {
        self.keys = None;
    }

    /// Ends a volume drag whose release the panel didn't see (it was
    /// hidden mid-drag). Call once per frame.
    pub fn settle(&mut self, project: Option<&mut Project>, pointer_down: bool) {
        if pointer_down {
            return;
        }
        if let Drag::Volume { gesture, .. } = self.drag {
            if let Some(p) = project {
                p.end_gesture(gesture);
            }
            self.drag = Drag::None;
        }
    }

    /// Where the panel was drawn last frame.
    #[cfg(test)]
    pub(crate) fn shown(&self) -> Rect {
        self.shown
    }

    /// Opens the marker editor on `r`, hanging from `anchor`.
    #[cfg(test)]
    pub(crate) fn open_marker_editor(&mut self, doc: &Document, r: MarkerRef, anchor: Pos2) {
        self.marker_edit = MarkerEdit::open(doc, r, anchor);
    }

    /// The lane height picked for `track`.
    fn size_of(&self, track: TrackId) -> LaneSize {
        self.lane_size.get(&track).copied().unwrap_or_default()
    }
}

impl Default for TimelineView {
    fn default() -> Self {
        TimelineView {
            px_per_sec: 60.0,
            scroll_secs: 0.0,
            track_scroll: 0.0,
            snapping: true,
            tool: ToolState::default(),
            linked_selection: true,
            last_marker_color: None,
            skim: Skim::default(),
            skim_at: None,
            playing: false,
            live_take: None,
            record_button: Rect::NOTHING,
            record: timeline_toolbar::RecordButton::default(),
            marker_edit: None,
            take_lane_shown: false,
            header_w: None,
            lane_size: HashMap::new(),
            rename: None,
            drag: Drag::None,
            menu: None,
            keys: None,
            shown: Rect::NOTHING,
        }
    }
}

/// What a right-click opened a menu for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuFor {
    Clip,
    Marker(MarkerRef),
    /// The ruler's floor away from flags.
    Ruler,
}

#[derive(Debug)]
enum Drag {
    None,
    Playhead,
    /// The hand tool pans the view (TL-024).
    Pan {
        origin: Pos2,
        scroll_secs: f64,
        track_scroll: f32,
    },
    /// A marker flag (or a range's end) being dragged: committed on
    /// release as one `MoveMarker` (or `EditMarker` for the end).
    Marker {
        marker: MarkerRef,
        end: bool,
        start_tick: Tick,
        /// The marker's start (or end) in sequence time when grabbed.
        from: Tick,
        /// For a range end: the marker's start.
        anchor: Tick,
        snap: SnapIndex,
        to: Tick,
    },
    Move {
        clips: Vec<ClipId>,
        start_tick: Tick,
        start_row: usize,
        snap: SnapIndex,
        edges: [Tick; 2],
        request: Option<EditRequest>,
        plan: Option<Result<Plan, String>>,
    },
    Trim {
        clip: ClipId,
        edge: Edge,
        start_tick: Tick,
        snap: SnapIndex,
        edge_at: Tick,
        /// Linked partners trimmed with it on release (TL-057).
        partners: Vec<ClipId>,
        request: Option<EditRequest>,
        plan: Option<Result<Plan, String>>,
    },
    /// A drag of a clip's volume line or one of its keyframe dots (AU-008):
    /// committed as it goes, all of it one undo entry.
    Volume {
        clip: ClipId,
        gesture: GestureId,
        grab: VolumeGrab,
        /// Where the press was.
        start: Pos2,
        /// The line band's height: pixels to slider travel.
        height: f32,
        /// The level being set, for the pill.
        shown: Option<GainDb>,
    },
}

impl Drag {
    /// A drag that follows the pointer in time, so the view scrolls along
    /// near the lanes' edges.
    fn scrolls_along(&self) -> bool {
        matches!(
            self,
            Drag::Playhead | Drag::Move { .. } | Drag::Trim { .. } | Drag::Marker { .. }
        )
    }
}

/// What a volume drag holds.
#[derive(Debug, Clone)]
enum VolumeGrab {
    /// The static volume, or the keys around the segment grabbed.
    Line(LinePart),
    /// A keyframe dot: its index, and its sequence time and value when
    /// grabbed.
    Dot {
        index: usize,
        seq: Tick,
        gain: GainDb,
    },
}

/// A clip's volume line as laid out this frame, for hit tests.
#[derive(Debug, Clone)]
struct LineGeom {
    clip: Arc<Clip>,
    audio: ClipAudio,
    band: Rect,
    /// Selected and on an unlocked track: the line and dots can be dragged.
    editable: bool,
    dots: Vec<(VisibleKey, Pos2)>,
}

/// What a press on a volume line hit.
#[derive(Debug, Clone, Copy)]
enum VolumeHit {
    Dot(VisibleKey),
    /// The line, at this source time.
    Line(Tick),
}

/// The heights of −∞ (bottom) and the top of the volume scale in a band.
fn line_span(band: Rect) -> (f32, f32) {
    (band.top() + 1.0, band.bottom() - 1.0)
}

/// Hit radius of a keyframe dot, in pixels: larger than the dot, so it is
/// easy to catch on small tracks.
const DOT_HIT: f32 = 7.0;

/// The volume line or dot of an editable clip under `p`; dots first.
fn volume_hit(lines: &[LineGeom], sc: Scale, p: Pos2) -> Option<(usize, VolumeHit)> {
    for (i, line) in lines.iter().enumerate() {
        if !line.editable || !line.band.expand(DOT_HIT).contains(p) {
            continue;
        }
        let dots: Vec<(f32, f32)> = line.dots.iter().map(|(_, d)| (d.x, d.y)).collect();
        if let Some(d) = clip_keys::hit_dot(&dots, (p.x, p.y), DOT_HIT) {
            return Some((i, VolumeHit::Dot(line.dots[d].0)));
        }
    }
    for (i, line) in lines.iter().enumerate() {
        if !line.editable || p.x < line.band.left() || p.x > line.band.right() {
            continue;
        }
        let at = sc.tick(p.x).clamp(line.clip.start(), line.clip.end());
        let db = clip_keys::envelope_db_at(&line.clip, &line.audio, at);
        let (top, bottom) = line_span(line.band);
        let y = clip_keys::y_of_db(db, top, bottom);
        if (p.y - y).abs() <= clip_keys::line_tolerance(line.band.height())
            && let Some(source) = line.clip.source_time_at(at)
        {
            return Some((i, VolumeHit::Line(source)));
        }
    }
    None
}

/// The keyframe dots of a clip's volume line in `band`.
fn volume_dots(clip: &Clip, audio: &ClipAudio, band: Rect, sc: Scale) -> Vec<(VisibleKey, Pos2)> {
    let (top, bottom) = line_span(band);
    clip_keys::visible_keys(clip, audio)
        .into_iter()
        .filter_map(|key| {
            let value = audio.gain.keys().get(key.index)?.value;
            let pos = Pos2::new(sc.x(key.seq), clip_keys::y_of_gain(value, top, bottom));
            Some((key, pos))
        })
        .collect()
}

/// One drawn row: a track in display order (top to bottom).
#[derive(Debug, Clone, Copy)]
struct Row {
    track: TrackId,
    kind: TrackKind,
    is_main: bool,
    top: f32,
    height: f32,
    /// The first audio track: the line between the bands sits above it.
    first_audio: bool,
}

/// The rows of `seq` from `top` down: the visual tracks from the top one
/// to the main track, then the audio tracks, each as tall as its lane.
fn rows(seq: &Sequence, top: f32, view: &TimelineView, m: &Metrics) -> Vec<Row> {
    let mut out = Vec::new();
    let mut y = top;
    let height = |t: &Track, main: bool| {
        layout::lane_height(t.kind, main, t.is_empty(), view.size_of(t.id), m)
    };
    for (i, t) in seq.visual_tracks().iter().enumerate().rev() {
        let h = height(t, i == 0);
        out.push(Row {
            track: t.id,
            kind: t.kind,
            is_main: i == 0,
            top: y,
            height: h,
            first_audio: false,
        });
        y += h;
    }
    for (i, t) in seq.audio_tracks().iter().enumerate() {
        let h = height(t, false);
        out.push(Row {
            track: t.id,
            kind: t.kind,
            is_main: false,
            top: y,
            height: h,
            first_audio: i == 0,
        });
        y += h;
    }
    out
}

/// A row's lane across `lanes`.
fn lane_rect(lanes: Rect, row: &Row) -> Rect {
    Rect::from_x_y_ranges(
        lanes.x_range(),
        egui::Rangef::new(row.top, row.top + row.height),
    )
}

/// Time ↔ pixel mapping for one frame of drawing (floats at the UI edge).
#[derive(Debug, Clone, Copy)]
struct Scale {
    x0: f32,
    scroll: f64,
    pps: f32,
}

impl Scale {
    fn of(view: &TimelineView, x0: f32) -> Scale {
        Scale {
            x0,
            scroll: view.scroll_secs,
            pps: view.px_per_sec,
        }
    }

    fn x(self, t: Tick) -> f32 {
        self.x0 + ((t.as_secs_f64() - self.scroll) * f64::from(self.pps)) as f32
    }

    fn tick(self, x: f32) -> Tick {
        let secs = self.scroll + f64::from(x - self.x0) / f64::from(self.pps);
        Tick::from_raw((secs * TICKS_PER_SECOND as f64).round() as i64)
    }

    /// The time `w` points span.
    fn span(self, w: f32) -> Tick {
        Tick::from_raw((f64::from(w / self.pps) * TICKS_PER_SECOND as f64).round() as i64)
    }
}

/// Where the parts of a clip go.
#[derive(Debug, Clone, Copy)]
struct ClipParts {
    rect: Rect,
    shape: ClipShape,
    /// The name strip (the whole clip when all strip).
    strip: Rect,
    /// Below the strip (empty when all strip).
    body: Rect,
    /// The filmstrip area of video and pictures.
    frames: Option<Rect>,
    /// Where the sound and the volume line go, when the clip plays sound.
    band: Option<Rect>,
}

/// Lays out `clip` in `lane`.
fn clip_parts(clip: &Clip, lane: Rect, sc: Scale, m: &Metrics) -> ClipParts {
    let rect = layout::clip_rect(lane, sc.x(clip.start()), sc.x(clip.end()));
    let shape = clip_shape(&clip.content);
    if shape == ClipShape::AllStrip {
        return ClipParts {
            rect,
            shape,
            strip: rect,
            body: Rect::from_min_max(rect.left_bottom(), rect.right_bottom()),
            frames: None,
            band: None,
        };
    }
    let (strip, body) = layout::strip_and_body(rect, m.clip_strip);
    let has_body = body.height() > 0.0;
    let (frames, band) = match (&clip.content, &clip.audio) {
        (
            ClipContent::Media {
                kind: MediaKind::Audio,
                ..
            },
            _,
        ) => (None, has_body.then_some(body)),
        (
            ClipContent::Media {
                kind: MediaKind::Video,
                ..
            },
            EmbeddedAudio::Active(_),
        ) if has_body => {
            let h = layout::sound_band_height(body.height());
            let split = body.bottom() - h;
            let frames =
                Rect::from_x_y_ranges(body.x_range(), egui::Rangef::new(body.top(), split));
            let band =
                Rect::from_x_y_ranges(body.x_range(), egui::Rangef::new(split, body.bottom()));
            (Some(frames), Some(band))
        }
        _ => (has_body.then_some(body), None),
    };
    ClipParts {
        rect,
        shape,
        strip,
        body,
        frames,
        band,
    }
}

fn clip_shape(content: &ClipContent) -> ClipShape {
    match content {
        ClipContent::Media { .. } | ClipContent::Image { .. } => ClipShape::TwoTone,
        ClipContent::Text(_) | ClipContent::Generated { .. } => ClipShape::AllStrip,
    }
}

/// The kind glyph of an all-strip clip.
fn clip_glyph(content: &ClipContent) -> &'static str {
    match content {
        ClipContent::Text(_) => icons::TEXT,
        ClipContent::Generated { kind } => match kind {
            GeneratedKind::Sticker => icons::STICKER,
            GeneratedKind::Effect => icons::EFFECT,
            GeneratedKind::Adjustment => icons::ADJUST,
        },
        ClipContent::Media {
            kind: MediaKind::Audio,
            ..
        } => icons::WAVEFORM,
        ClipContent::Media { .. } => icons::VIDEO,
        ClipContent::Image { .. } => icons::IMAGE,
    }
}

/// The icon of a track kind in its header.
fn track_glyph(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => icons::VIDEO,
        TrackKind::Text => icons::TEXT,
        TrackKind::Sticker => icons::STICKER,
        TrackKind::Effect => icons::EFFECT,
        TrackKind::Adjustment => icons::ADJUST,
        TrackKind::Audio => icons::WAVEFORM,
    }
}

/// A clip on screen this frame.
#[derive(Debug, Clone)]
struct ClipDraw {
    clip: Arc<Clip>,
    parts: ClipParts,
    selected: bool,
    /// On a locked track: veiled.
    locked: bool,
    /// Switched off or on a hidden track: dimmed.
    dimmed: bool,
    /// Its lane's colour, for the veils.
    lane: Color32,
    /// Its volume line, by index into the frame's lines.
    line: Option<usize>,
}

/// A take being recorded (AU-027), as the timeline draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveTake {
    /// Where the clip will start.
    pub start: Tick,
    /// What was recorded after the start so far.
    pub length: Tick,
    /// The track it lands on; `None`: a new audio track (drawn below the
    /// tracks).
    pub track: Option<TrackId>,
    /// Peak levels (0..=1) spread evenly over `length`, oldest first.
    pub peaks: Vec<f32>,
}

/// What the app should do after the panel ran.
#[derive(Debug, Default)]
pub struct TimelineOutput {
    pub status: Option<String>,
    pub seeked: bool,
    /// A command picked from the clip context menu.
    pub command: Option<ClipCommand>,
    /// A text clip was double-clicked (TX-002): edit its content.
    pub edit_text: bool,
    /// A toolbar button (UI-011) that runs one of the app's commands.
    pub action: Option<ToolAction>,
    /// The frame under the pointer over the ruler or the lanes, for hover
    /// skim (TL-014); `None` outside them or during a gesture.
    pub pointer_time: Option<Tick>,
    /// The blade's cut line under the pointer (TL-024).
    pub blade_time: Option<Tick>,
    /// A gesture (a clip, edge, marker or playhead drag) is running.
    pub dragging: bool,
    /// In Match's pick mode (CL-020), a clip was clicked: the clip and the
    /// sequence time of the frame under the pointer.
    pub match_reference: Option<(ClipId, Tick)>,
}

/// Draws the timeline and handles its gestures. `mixer_open` checks the
/// toolbar's Mixer button.
pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    waves: &Waveforms,
    films: &mut Filmstrips,
    mixer_open: bool,
) -> TimelineOutput {
    let mut out = TimelineOutput::default();
    let look = theme::look(ui.ctx());
    let (p, m) = (look.palette, look.metrics);
    let full = ui.available_rect_before_wrap();
    ui.advance_cursor_after_rect(full);
    view.shown = full;
    let header_w =
        layout::clamp_header_width(view.header_w.unwrap_or(m.tl_header), full.width(), &m);
    let r = layout::timeline_layout(full, &m, header_w);

    super::timeline_tabs::show(ui, r.tabs);
    let can_delete = !project.selection.is_empty()
        || !project.markers.is_empty()
        || view.selected_keys(project).is_some();
    let input = ToolbarInput {
        lanes_width: r.lanes.width(),
        mixer_open,
        can_delete,
    };
    timeline_toolbar::show(ui, r.toolbar, project, view, &input, &mut out);

    let body = r.ruler.union(r.lanes);
    let response = ui.interact(
        body,
        ui.id().with("timeline-lanes"),
        Sense::click_and_drag(),
    );
    scroll_and_zoom(ui, view, &r);

    let rate = project.doc().settings().frame_rate;
    let quantize = |t: Tick| rate.quantize(t).unwrap_or(t).max(Tick::ZERO);

    // The document to draw: the gesture's ghost if it has one.
    let doc: Document = match &view.drag {
        Drag::Move {
            plan: Some(Ok(p)), ..
        }
        | Drag::Trim {
            plan: Some(Ok(p)), ..
        } => p.preview().clone(),
        // Cheap: the document is Arc-structured.
        _ => project.doc().clone(),
    };
    let refused = matches!(
        &view.drag,
        Drag::Move {
            plan: Some(Err(_)),
            ..
        } | Drag::Trim {
            plan: Some(Err(_)),
            ..
        }
    );
    let Some(seq) = doc.sequence(doc.root()) else {
        return out;
    };
    // The track stack scrolls; it can't scroll past its end.
    let stack: f32 = rows(seq, 0.0, view, &m).iter().map(|r| r.height).sum();
    // The scrollbar has its own strip under the lanes (see the layout).
    // A take recorded onto a new track adds its lane below the stack:
    // scrollable, and scrolled into view once when the take starts.
    let take_lane = view.live_take.as_ref().is_some_and(|t| t.track.is_none());
    let content_h = stack + if take_lane { m.lanes[1] } else { 0.0 };
    let max_scroll = (content_h - r.lanes.height()).max(0.0);
    if take_lane && !view.take_lane_shown {
        view.track_scroll = max_scroll;
    }
    view.take_lane_shown = take_lane;
    view.track_scroll = view.track_scroll.clamp(0.0, max_scroll);
    let rows = rows(seq, r.lanes.top() - view.track_scroll, view, &m);
    // Dots stay selected only while their clip is.
    if view
        .keys
        .as_ref()
        .is_some_and(|k| !project.selection.contains(&k.clip))
    {
        view.keys = None;
    }
    let sc = Scale::of(view, r.lanes.left());
    let tick_per_px = TICKS_PER_SECOND as f64 / f64::from(view.px_per_sec);
    // TL-030: every out-of-sync badge in one pass.
    let offsets = freecut_timeline::edit::sync_offsets(&doc, doc.root());
    let frame = rate.frame_ticks();

    // Lay out the clips on screen and their volume lines.
    let mut draws: Vec<ClipDraw> = Vec::new();
    let mut lines: Vec<LineGeom> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let lane = lane_rect(r.lanes, row);
        if !lane.intersects(r.lanes) {
            continue;
        }
        let Some((_, track)) = seq.find_track(row.track) else {
            continue;
        };
        let flags = track.header.flags;
        for clip in track.clips() {
            let (x0, x1) = (sc.x(clip.start()), sc.x(clip.end()));
            if x1 < r.lanes.left() || x0 > r.lanes.right() {
                continue;
            }
            let parts = clip_parts(clip, lane, sc, &m);
            let selected = project.selection.contains(&clip.id);
            let line = match (parts.band, clip.audio.active()) {
                (Some(band), Some(audio)) if matches!(clip.content, ClipContent::Media { .. }) => {
                    lines.push(LineGeom {
                        clip: Arc::clone(clip),
                        audio: audio.clone(),
                        band,
                        // AU-008: a locked track's line is visible, not
                        // editable.
                        editable: selected && !flags.locked && band.height() >= 6.0,
                        dots: volume_dots(clip, audio, band, sc),
                    });
                    Some(lines.len() - 1)
                }
                _ => None,
            };
            draws.push(ClipDraw {
                clip: Arc::clone(clip),
                parts,
                selected,
                locked: flags.locked,
                dimmed: !clip.enabled || flags.hidden,
                lane: if i % 2 == 0 { p.bg_lane } else { p.bg_lane_alt },
                line,
            });
        }
    }

    let hit = |p: Pos2| -> Option<(ClipId, Option<Edge>)> {
        if !r.lanes.contains(p) {
            return None;
        }
        let row = rows
            .iter()
            .find(|r| p.y >= r.top && p.y < r.top + r.height)?;
        let (_, track) = seq.find_track(row.track)?;
        let t = sc.tick(p.x);
        let clip = track.clips().iter().find(|c| {
            let (x0, x1) = (sc.x(c.start()), sc.x(c.end()));
            p.x >= x0 - 2.0
                && p.x <= x1 + 2.0
                && (c.range.contains(t)
                    || (p.x - x0).abs() < EDGE_GRAB
                    || (p.x - x1).abs() < EDGE_GRAB)
        })?;
        let (x0, x1) = (sc.x(clip.start()), sc.x(clip.end()));
        let edge = if (p.x - x0).abs() < EDGE_GRAB && x1 - x0 > 3.0 * EDGE_GRAB {
            Some(Edge::In)
        } else if (p.x - x1).abs() < EDGE_GRAB && x1 - x0 > 3.0 * EDGE_GRAB {
            Some(Edge::Out)
        } else {
            None
        };
        Some((clip.id, edge))
    };
    let row_index_at = |y: f32| -> usize {
        rows.iter()
            .position(|r| y < r.top + r.height)
            .unwrap_or(rows.len().saturating_sub(1))
    };

    // Marker flags (TL-037): timeline markers on the ruler floor, clip
    // markers on their clips' strips; a dragged one where it would land.
    let dragged_marker = match &view.drag {
        Drag::Marker {
            marker, end, to, ..
        } => Some((*marker, *end, *to)),
        _ => None,
    };
    let flags = layout_flags(seq, project, &draws, sc, &r, dragged_marker);
    let tool = view.tool.active;
    let flag_under = |pos: Pos2| -> Option<(usize, FlagGrab)> {
        let (i, grab) = markers::flag_at(&flags, pos)?;
        // Clip flags take presses only with the Select tool.
        (flags[i].ruler || tool == TimelineTool::Select).then_some((i, grab))
    };
    let hover_flag = match (response.hover_pos(), &view.drag) {
        (Some(pos), Drag::None) => flag_under(pos).map(|(i, _)| i),
        _ => None,
    };

    // What the pointer is over: a volume line, else a clip.
    let hover = response
        .hover_pos()
        .filter(|p| r.lanes.contains(*p) && hover_flag.is_none());
    let (hover_line, hover_clip) = match (hover, &view.drag) {
        (Some(pos), Drag::None) if tool != TimelineTool::Select => (None, hit(pos).map(|(c, _)| c)),
        (Some(pos), Drag::None) => match volume_hit(&lines, sc, pos) {
            Some((i, _)) => (Some(i), None),
            None => (None, hit(pos).map(|(c, _)| c)),
        },
        _ => (None, None),
    };
    let held_line = match &view.drag {
        Drag::Volume { clip, shown, .. } => lines
            .iter()
            .position(|l| l.clip.id == *clip)
            .map(|i| (i, *shown)),
        _ => None,
    };

    // --- painting ---
    let painter = ui.painter_at(full);
    let head = sc.x(project.playhead);
    let head = (head >= r.lanes.left() - 0.5 && head <= r.lanes.right()).then_some(head);
    let grid = draw_ruler(
        &painter.with_clip_rect(r.ruler),
        r.ruler,
        sc,
        rate,
        head,
        &look,
    );
    painter.rect_filled(r.corner, 0.0, p.bg_ruler);
    painter.hline(
        r.corner.x_range(),
        r.corner.bottom() - 0.5,
        Stroke::new(1.0, p.border_subtle),
    );
    let lanes_painter = painter.with_clip_rect(r.lanes);
    for (i, row) in rows.iter().enumerate() {
        let lane = lane_rect(r.lanes, row);
        if !lane.intersects(r.lanes) {
            continue;
        }
        let fill = if i % 2 == 0 { p.bg_lane } else { p.bg_lane_alt };
        lanes_painter.rect_filled(lane, 0.0, fill);
        lanes_painter.hline(
            lane.x_range(),
            lane.bottom() - 0.5,
            Stroke::new(1.0, p.border_subtle),
        );
    }
    // Below the last track: the panel's well.
    if let Some(last) = rows.last() {
        let below = Rect::from_x_y_ranges(
            r.lanes.x_range(),
            egui::Rangef::new(
                last.top + last.height,
                r.lanes.bottom().max(last.top + last.height),
            ),
        );
        lanes_painter.rect_filled(below, 0.0, p.bg_well);
    }
    for x in &grid {
        lanes_painter.vline(*x, r.lanes.y_range(), Stroke::new(1.0, p.timeline_grid));
    }
    // The line between the visual and the audio tracks.
    if let Some(row) = rows.iter().find(|r| r.first_audio) {
        painter.with_clip_rect(r.lanes.union(r.headers)).hline(
            r.headers.left()..=r.lanes.right(),
            row.top + 0.5,
            Stroke::new(1.0, p.border_strong),
        );
    }

    let ctx = ClipCtx {
        painter: &lanes_painter,
        look: &look,
        doc: &doc,
        project: &*project,
        waves,
        sc,
        frame,
        lanes: r.lanes,
        ppp: ui.ctx().pixels_per_point(),
    };
    let latest = ui.input(|i| i.pointer.latest_pos());
    for d in &draws {
        let line = d.line.map(|i| {
            let held = held_line.filter(|(h, _)| *h == i);
            // The level by the pointer while the line is held or hovered.
            let pill = match (held, hover_line == Some(i), latest) {
                (Some((_, Some(g))), _, Some(pos)) => Some((format_db(gain_db(g)), pos.x)),
                (None, true, Some(pos)) => {
                    let at = sc.tick(pos.x);
                    let db = clip_keys::envelope_db_at(&lines[i].clip, &lines[i].audio, at);
                    Some((format_db(db), pos.x))
                }
                _ => None,
            };
            (&lines[i], held.is_some() || hover_line == Some(i), pill)
        });
        let picked: &[Tick] = view
            .keys
            .as_ref()
            .filter(|k| k.clip == d.clip.id)
            .map_or(&[], |k| k.times.as_slice());
        let sync = offsets
            .get(&d.clip.id)
            .map(|o| clip_streams::sync_badge(o, frame));
        draw_clip(
            &ctx,
            films,
            d,
            line,
            picked,
            hover_clip == Some(d.clip.id),
            sync,
        );
    }

    // TL-029: a tick at the start of each attached item points down to its
    // owner; an item that lost its place (a locked track) shows it in the
    // warning colour.
    for d in &draws {
        let Some(a) = d.clip.attach else { continue };
        let rect = d.parts.rect;
        if rect.width() < 10.0 {
            continue;
        }
        let color = if expected_start(seq, a) == Some(d.clip.start()) {
            p.text_secondary
        } else {
            p.warning
        };
        let (x, y) = (rect.left() + 1.0, rect.bottom());
        lanes_painter.add(Shape::convex_polygon(
            vec![
                Pos2::new(x, y - 2.0),
                Pos2::new(x + 6.0, y - 2.0),
                Pos2::new(x, y + 3.5),
            ],
            color,
            Stroke::new(1.0, Color32::from_black_alpha(80)),
        ));
    }
    // Clip markers on their strips.
    for (i, f) in flags.iter().enumerate().filter(|(_, f)| !f.ruler) {
        markers::paint_flag(&lanes_painter, f, &p, hover_flag == Some(i));
    }

    // The blade's cut line (TL-024): at the snapped frame under the
    // pointer, with its timecode; a line through every lane with Shift.
    let snapping_now = view.snapping && !ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
    let threshold = (f64::from(SNAP_PX) * tick_per_px) as u64;
    let blade_at = match (tool, hover, &view.drag) {
        (TimelineTool::Blade, Some(pos), Drag::None) => hit(pos).and_then(|(clip, _)| {
            let index = snapping_now
                .then(|| SnapIndex::build(seq, Some(project.playhead), &BTreeSet::new()));
            blade_time(
                seq,
                clip,
                sc.tick(pos.x),
                index.as_ref().map(|i| (i, threshold)),
                rate,
            )
            .map(|t| (clip, t))
        }),
        _ => None,
    };
    if let (Some((_, t)), Some(pos)) = (blade_at, hover) {
        let x = sc.x(t).round() + 0.5;
        // Only the hovered lane is cut unless Shift is held, so the line
        // covers just that lane; with Shift it spans every lane.
        let every_track = ui.input(|i| i.modifiers.shift);
        let span = rows
            .iter()
            .find(|row| pos.y >= row.top && pos.y < row.top + row.height)
            .filter(|_| !every_track)
            .map_or(r.lanes.y_range(), |row| {
                let top = row.top.max(r.lanes.top());
                let bottom = (row.top + row.height).min(r.lanes.bottom());
                egui::Rangef::new(top, bottom.max(top))
            });
        lanes_painter.vline(x, span, Stroke::new(1.0, p.accent));
        let tc =
            Timecode::from_tick(t, rate, false).map_or_else(|_| String::new(), |tc| tc.to_string());
        // Beside the pointer, so it never sits on a clip's name strip at the
        // top of the lanes.
        let pill_y = (pos.y + 14.0)
            .min(r.lanes.bottom() - 18.0)
            .max(r.lanes.top() + 2.0);
        markers::paint_time_pill(&lanes_painter, Pos2::new(x + 6.0, pill_y), &tc, &p);
    }

    // Media dragged in from the asset panel: where it will land.
    if response.dnd_hover_payload::<AssetDrag>().is_some()
        && let Some(pos) = response.hover_pos()
        && r.lanes.contains(pos)
    {
        let span = rows
            .iter()
            .find(|row| pos.y >= row.top && pos.y < row.top + row.height)
            .map_or(r.lanes, |row| lane_rect(r.lanes, row));
        lanes_painter.rect_filled(span, 0.0, p.overlay_selection());
        let x = sc.x(quantize(sc.tick(pos.x)));
        lanes_painter.vline(x, span.y_range(), Stroke::new(2.0, p.accent));
    }
    if refused {
        painter.rect_stroke(r.lanes, 0.0, Stroke::new(2.0, p.danger), StrokeKind::Inside);
    }

    // The selected clip's keyframes on the ruler floor.
    let diamonds = ruler_keys(&doc, project, sc, r.ruler);
    let ruler_painter = painter.with_clip_rect(r.ruler);
    for (x, _) in &diamonds {
        let c = Pos2::new(*x, r.ruler.bottom() - 5.5);
        let d = 4.5;
        ruler_painter.add(Shape::convex_polygon(
            vec![
                c + Vec2::new(0.0, -d),
                c + Vec2::new(d, 0.0),
                c + Vec2::new(0.0, d),
                c + Vec2::new(-d, 0.0),
            ],
            p.accent,
            Stroke::new(1.0, p.bg_ruler),
        ));
    }

    // Timeline markers on the ruler floor; a hovered or selected one draws
    // its line down through the lanes.
    for (i, f) in flags.iter().enumerate().filter(|(_, f)| f.ruler) {
        if f.selected || hover_flag == Some(i) {
            lanes_painter.vline(
                f.spot.x.round() + 0.5,
                r.lanes.y_range(),
                Stroke::new(1.0, surface::palette(f.color)),
            );
        }
        markers::paint_flag(&ruler_painter, f, &p, hover_flag == Some(i));
    }
    if let Some(i) = hover_flag {
        let text = markers::tooltip(&flags, i);
        if !text.is_empty() {
            response.clone().on_hover_text_at_pointer(text);
        }
    }

    // The take being recorded (AU-027): a red clip growing from its start.
    if let Some(take) = &view.live_take {
        let lane = take
            .track
            .and_then(|t| rows.iter().find(|row| row.track == t))
            .map(|row| lane_rect(r.lanes, row))
            .unwrap_or_else(|| {
                // The new track goes below the others: drawn there, and
                // kept in view when the stack is taller than the lanes.
                let below = rows.last().map_or(r.lanes.top(), |l| l.top + l.height);
                let top = below.min(r.lanes.bottom() - m.lanes[1]).max(r.lanes.top());
                Rect::from_x_y_ranges(r.lanes.x_range(), top..=top + m.lanes[1])
            });
        if take.track.is_none() {
            // The new track's lane, over whatever it covers.
            lanes_painter.rect_filled(lane, 0.0, p.bg_lane_alt);
            lanes_painter.hline(
                lane.x_range(),
                lane.top() + 0.5,
                Stroke::new(1.0, p.border_strong),
            );
        }
        draw_live_take(&lanes_painter, &look, take, lane, sc);
    }
    // The hover-skim line (TL-014): 1 pt, quieter than the playhead.
    if let Some(t) = view.skim_at {
        let x = sc.x(t).round() + 0.5;
        if x >= r.lanes.left() && x <= r.lanes.right() {
            painter.with_clip_rect(body).vline(
                x,
                egui::Rangef::new(r.ruler.top(), r.lanes.bottom()),
                Stroke::new(1.0, p.text_tertiary),
            );
        }
    }

    draw_headers(ui, project, view, seq, &rows, &r, &mut out);
    header_splitter(ui, view, &r, full);

    // The playhead over everything in the lanes.
    let ph_x = sc.x(project.playhead);
    if ph_x >= r.lanes.left() - 0.5 && ph_x <= r.lanes.right() {
        let over = painter.with_clip_rect(body);
        over.vline(
            ph_x,
            egui::Rangef::new(r.ruler.top(), r.lanes.bottom()),
            Stroke::new(1.0, p.playhead),
        );
        over.add(Shape::convex_polygon(
            layout::playhead_head(ph_x, r.ruler).to_vec(),
            p.playhead,
            Stroke::new(1.0, p.bg_ruler),
        ));
    }

    // Thin scrollbars: the track stack and time.
    let mut track_scroll = view.track_scroll;
    thin_scrollbar(
        ui,
        ui.id().with("timeline-vbar"),
        r.vbar,
        Axis::Vertical,
        content_h,
        r.lanes.height(),
        &mut track_scroll,
    );
    view.track_scroll = track_scroll;
    let pps = view.px_per_sec;
    let view_secs = f64::from(r.lanes.width() / pps);
    let length = seq.duration().as_secs_f64();
    let content_secs = (length + view_secs * 0.25).max(view.scroll_secs + view_secs);
    let mut offset = (view.scroll_secs * f64::from(pps)) as f32;
    if thin_scrollbar(
        ui,
        ui.id().with("timeline-hbar"),
        r.hbar,
        Axis::Horizontal,
        (content_secs * f64::from(pps)) as f32,
        r.lanes.width(),
        &mut offset,
    ) {
        view.scroll_secs = f64::from(offset / pps).max(0.0);
    }

    // --- gestures ---
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    // Over a clip on a locked track, the blade and the hand show "not
    // allowed" (TL-024).
    let on_locked = |pos: Pos2| {
        hit(pos).is_some_and(|(clip, _)| {
            seq.find_clip(clip)
                .is_some_and(|(_, t)| t.header.flags.locked)
        })
    };
    let picking = project.match_pick.is_some();
    if let Some(pos) = response.hover_pos()
        && picking
        && r.lanes.contains(pos)
    {
        // CL-020's pick mode: a frame of a clip is the target.
        let icon = if hit(pos).is_some() {
            CursorIcon::Crosshair
        } else {
            CursorIcon::NotAllowed
        };
        ui.ctx().set_cursor_icon(icon);
    } else if response.hover_pos().is_some()
        && matches!(view.drag, Drag::None)
        && hover_flag.is_some()
    {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    } else if let Some(pos) = response.hover_pos()
        && matches!(view.drag, Drag::None)
        && r.lanes.contains(pos)
        && tool != TimelineTool::Select
    {
        let icon = if on_locked(pos) {
            CursorIcon::NotAllowed
        } else if tool == TimelineTool::Blade {
            CursorIcon::Crosshair
        } else {
            CursorIcon::Grab
        };
        ui.ctx().set_cursor_icon(icon);
    } else if let Some(pos) = response.hover_pos()
        && matches!(view.drag, Drag::None)
        && r.lanes.contains(pos)
    {
        match volume_hit(&lines, sc, pos) {
            Some((_, VolumeHit::Dot(_))) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
            Some((_, VolumeHit::Line(_))) => ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical),
            None => {
                if let Some((_, Some(_))) = hit(pos) {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                }
            }
        }
    }
    if matches!(view.drag, Drag::Volume { .. } | Drag::Pan { .. }) {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }

    // Press: decide what the gesture is (none while Match picks a frame).
    if response.drag_started()
        && !picking
        && let Some(pos) = response.interact_pointer_pos()
    {
        let live = project.doc().clone();
        let live_seq = live.sequence(live.root());
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
        let alt = ui.input(|i| i.modifiers.alt);
        if let Some((i, grab)) = flag_under(origin)
            && let Some(live_seq) = live_seq
            && let Some(drag) = marker_drag(
                &live,
                live_seq,
                &flags[i],
                grab,
                sc.tick(origin.x),
                project.playhead,
            )
        {
            project.markers = BTreeSet::from([flags[i].r]);
            view.drag = drag;
        } else if r.ruler.contains(pos) {
            view.drag = Drag::Playhead;
        } else if tool == TimelineTool::Hand {
            view.drag = Drag::Pan {
                origin,
                scroll_secs: view.scroll_secs,
                track_scroll: view.track_scroll,
            };
        } else if tool == TimelineTool::Blade {
            // A blade drag does nothing; the click cuts.
        } else if let Some((i, grabbed)) = volume_hit(&lines, sc, origin) {
            // AU-008: the volume line or a keyframe dot of a selected clip.
            let line = &lines[i];
            let clip = line.clip.id;
            let grab = match grabbed {
                VolumeHit::Dot(k) => {
                    let kept = view
                        .keys
                        .as_ref()
                        .is_some_and(|s| s.clip == clip && s.times.contains(&k.at));
                    if !kept {
                        view.keys = Some(KeySelection {
                            clip,
                            times: vec![k.at],
                        });
                    }
                    VolumeGrab::Dot {
                        index: k.index,
                        seq: k.seq,
                        gain: line
                            .audio
                            .gain
                            .keys()
                            .get(k.index)
                            .map_or(GainDb::ZERO, |key| key.value),
                    }
                }
                VolumeHit::Line(at) => VolumeGrab::Line(clip_keys::line_part(&line.audio.gain, at)),
            };
            view.drag = Drag::Volume {
                clip,
                gesture: project.begin_gesture(),
                grab,
                start: origin,
                height: line_span(line.band).1 - line_span(line.band).0,
                shown: None,
            };
        } else if let (Some((clip, edge)), Some(live_seq)) = (hit(pos), live_seq) {
            if !project.selection.contains(&clip) {
                project.pick(clip, Pick::Replace, view.linked_selection, alt);
            }
            let mut exclude: BTreeSet<ClipId> = BTreeSet::new();
            match edge {
                Some(edge) => {
                    exclude.insert(clip);
                    // TL-057: partners whose same edge is at the same time
                    // trim with it (not with Alt, nor with the option off).
                    let partners = if view.linked_selection && !alt {
                        freecut_timeline::select::linked_edges(live_seq, clip, edge)
                    } else {
                        Vec::new()
                    };
                    exclude.extend(partners.iter().copied());
                    let snap = SnapIndex::build(live_seq, Some(project.playhead), &exclude);
                    let edge_at = live.clip(clip).map_or(Tick::ZERO, |c| match edge {
                        Edge::In => c.start(),
                        Edge::Out => c.end(),
                    });
                    view.drag = Drag::Trim {
                        clip,
                        edge,
                        start_tick: sc.tick(pos.x),
                        snap,
                        edge_at,
                        partners,
                        request: None,
                        plan: None,
                    };
                }
                None => {
                    let clips = project.selection.clone();
                    exclude.extend(clips.iter().copied());
                    let start = clips
                        .iter()
                        .filter_map(|id| live.clip(*id))
                        .map(|c| c.start())
                        .min()
                        .unwrap_or(Tick::ZERO);
                    let end = clips
                        .iter()
                        .filter_map(|id| live.clip(*id))
                        .map(|c| c.end())
                        .max()
                        .unwrap_or(Tick::ZERO);
                    view.drag = Drag::Move {
                        clips,
                        start_tick: sc.tick(pos.x),
                        start_row: row_index_at(pos.y),
                        snap: SnapIndex::build(live_seq, Some(project.playhead), &exclude),
                        edges: [start, end],
                        request: None,
                        plan: None,
                    };
                }
            }
        } else {
            view.drag = Drag::Playhead;
        }
    }
    // Move: re-plan from the total delta since the press.
    let mut view_keys_moved: Option<(ClipId, Tick)> = None;
    if response.dragged()
        && let Some(p) = pointer
    {
        let snapping = view.snapping && !ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        let pointer_tick = sc.tick(p.x);
        let row_now = row_index_at(p.y);
        let seq_id = project.doc().root();
        if let Drag::Pan {
            origin,
            scroll_secs,
            track_scroll,
        } = view.drag
        {
            let d = p - origin;
            view.scroll_secs = (scroll_secs - f64::from(d.x / view.px_per_sec)).max(0.0);
            view.track_scroll = (track_scroll - d.y).max(0.0);
        }
        match &mut view.drag {
            Drag::Playhead => {
                project.playhead = quantize(pointer_tick);
                out.seeked = true;
            }
            Drag::Marker {
                end,
                start_tick,
                from,
                anchor,
                snap,
                to,
                ..
            } => {
                let mut delta = pointer_tick.checked_sub(*start_tick).unwrap_or(Tick::ZERO);
                if snapping && let Some(s) = snap.snap(&[*from], delta, threshold) {
                    delta = s.delta;
                }
                let t = quantize(from.checked_add(delta).unwrap_or(*from));
                *to = if *end { t.max(*anchor) } else { t };
            }
            Drag::Pan { .. } => {}
            Drag::Move {
                clips,
                start_tick,
                start_row,
                snap,
                edges,
                request,
                plan,
            } => {
                let mut delta = pointer_tick.checked_sub(*start_tick).unwrap_or(Tick::ZERO);
                if snapping && let Some(s) = snap.snap(edges, delta, threshold) {
                    delta = s.delta;
                }
                let shift =
                    i32::try_from(*start_row).unwrap_or(0) - i32::try_from(row_now).unwrap_or(0);
                let req = EditRequest::Move(MoveReq {
                    seq: seq_id,
                    clips: clips.clone(),
                    delta,
                    track_shift: shift,
                });
                if request.as_ref() != Some(&req) {
                    *plan = Some(
                        project
                            .editor
                            .plan(&project.ctx(), &req)
                            .map_err(|e| crate::session::describe_edit_error(&e)),
                    );
                    *request = Some(req);
                }
            }
            Drag::Trim {
                clip,
                edge,
                start_tick,
                snap,
                edge_at,
                request,
                plan,
                ..
            } => {
                let mut delta = pointer_tick.checked_sub(*start_tick).unwrap_or(Tick::ZERO);
                if snapping && let Some(s) = snap.snap(&[*edge_at], delta, threshold) {
                    delta = s.delta;
                }
                let req = EditRequest::Trim(TrimReq {
                    seq: seq_id,
                    clip: *clip,
                    edge: *edge,
                    delta,
                    ripple: false,
                });
                if request.as_ref() != Some(&req) {
                    *plan = Some(
                        project
                            .editor
                            .plan(&project.ctx(), &req)
                            .map_err(|e| crate::session::describe_edit_error(&e)),
                    );
                    *request = Some(req);
                }
            }
            Drag::Volume {
                clip,
                gesture,
                grab,
                start,
                height,
                shown,
            } => {
                let du = f64::from((start.y - p.y) / height.max(1.0));
                let coalesce = Coalesce::Gesture(*gesture);
                let result = match grab {
                    VolumeGrab::Line(part) => {
                        *shown = Some(match part {
                            LinePart::Static(g) => clip_keys::shifted(*g, du),
                            LinePart::Keys(keys) => keys
                                .first()
                                .map_or(GainDb::ZERO, |k| clip_keys::shifted(k.2, du)),
                        });
                        clip_keys::shift_line(project, *clip, part, du, coalesce)
                    }
                    VolumeGrab::Dot { index, seq, gain } => {
                        let g = clip_keys::shifted(*gain, du);
                        *shown = Some(g);
                        let moved = pointer_tick
                            .checked_sub(sc.tick(start.x))
                            .unwrap_or(Tick::ZERO);
                        let live = project.doc().clip(*clip).cloned();
                        let to = live.as_ref().and_then(|c| {
                            let t = quantize(seq.checked_add(moved).unwrap_or(*seq))
                                .clamp(c.start(), c.end());
                            c.source_time_at(t)
                        });
                        match to {
                            Some(to) => clip_keys::move_key(
                                project, *clip, *index, to, g, coalesce,
                            )
                            .map(|at| {
                                view_keys_moved = Some((*clip, at));
                            }),
                            None => Ok(()),
                        }
                    }
                };
                if let Err(e) = result {
                    out.status = Some(e);
                }
            }
            Drag::None => {}
        }
    }
    if let Some((clip, at)) = view_keys_moved {
        view.keys = Some(KeySelection {
            clip,
            times: vec![at],
        });
    }

    // Release: commit the final request (planned again against the live
    // document, in case anything changed during the drag).
    if response.drag_stopped() {
        match std::mem::replace(&mut view.drag, Drag::None) {
            Drag::Move {
                request: Some(req), ..
            } => {
                if let Err(e) = project.apply(&req, Coalesce::Never) {
                    out.status = Some(e);
                }
            }
            Drag::Trim {
                request: Some(req),
                partners,
                ..
            } => {
                if let Err(e) = commit_trim(project, &req, &partners) {
                    out.status = Some(e);
                }
            }
            Drag::Marker {
                marker,
                end,
                from,
                anchor,
                to,
                ..
            } if to != from => {
                let result = if end {
                    project.edit_marker(
                        marker,
                        freecut_timeline::edit::MarkerPatch {
                            duration: to.checked_sub(anchor).ok(),
                            ..Default::default()
                        },
                    )
                } else {
                    project.move_marker(marker, to)
                };
                if let Err(e) = result {
                    out.status = Some(e);
                }
            }
            Drag::Volume { gesture, .. } => project.end_gesture(gesture),
            _ => {}
        }
    }

    // Click: select a keyframe dot or add one (AU-008), select a clip, or
    // move the playhead.
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        let (shift, alt, command) =
            ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.modifiers.command));
        let on_flag = flag_under(p);
        let on_volume = (r.lanes.contains(p) && tool == TimelineTool::Select && on_flag.is_none())
            .then(|| volume_hit(&lines, sc, p))
            .flatten();
        let handled = match on_volume {
            Some((i, VolumeHit::Dot(k))) => {
                let clip = lines[i].clip.id;
                match &mut view.keys {
                    Some(sel) if sel.clip == clip && shift => {
                        if let Some(at) = sel.times.iter().position(|t| *t == k.at) {
                            sel.times.remove(at);
                        } else {
                            sel.times.push(k.at);
                        }
                    }
                    _ => {
                        view.keys = Some(KeySelection {
                            clip,
                            times: vec![k.at],
                        });
                    }
                }
                true
            }
            Some((i, VolumeHit::Line(_))) if alt => {
                // The new key lands on a frame of the clip.
                let c = &lines[i].clip;
                let clip = c.id;
                let at = c
                    .source_time_at(quantize(sc.tick(p.x)).clamp(c.start(), c.end()))
                    .unwrap_or(Tick::ZERO);
                match clip_keys::add_key(project, clip, at) {
                    Ok(at) => {
                        view.keys = Some(KeySelection {
                            clip,
                            times: vec![at],
                        });
                    }
                    Err(e) => out.status = Some(e),
                }
                true
            }
            _ => {
                view.keys = None;
                false
            }
        };
        if handled {
            // A dot or the line took the click.
        } else if r.lanes.contains(p) && picking {
            // CL-020's pick mode: the clicked clip's frame is the match
            // reference; the selection stays.
            match hit(p) {
                Some((clip, _)) => out.match_reference = Some((clip, quantize(sc.tick(p.x)))),
                None => {
                    out.status = Some("Click a frame of a clip, or press Esc to stop".to_owned());
                }
            }
        } else if let Some((i, _)) = on_flag {
            // A flag selects its marker (TL-037); Shift adds or removes.
            let m = flags[i].r;
            if shift || command {
                if !project.markers.remove(&m) {
                    project.markers.insert(m);
                }
            } else {
                project.markers = BTreeSet::from([m]);
                project.selection.clear();
            }
        } else if r.lanes.contains(p) && tool == TimelineTool::Blade {
            // TL-024: cut the clicked clip at the snapped frame; Shift cuts
            // every unlocked track. Gaps, edges and locked tracks do nothing.
            let index = snapping_now
                .then(|| SnapIndex::build(seq, Some(project.playhead), &BTreeSet::new()));
            if let Some((clip, _)) = hit(p)
                && let Some(t) = blade_time(
                    seq,
                    clip,
                    sc.tick(p.x),
                    index.as_ref().map(|i| (i, threshold)),
                    rate,
                )
                && let Err(e) = project.blade(clip, t, shift)
            {
                out.status = Some(e);
            }
        } else if r.lanes.contains(p) && tool == TimelineTool::Hand {
            // The hand only pans.
        } else if r.ruler.contains(p) {
            // A keyframe diamond takes the click to its own time.
            let key = diamonds
                .iter()
                .find(|(x, _)| (x - p.x).abs() <= 7.0)
                .map(|(_, t)| *t);
            project.playhead = key.unwrap_or_else(|| quantize(sc.tick(p.x)));
            out.seeked = true;
        } else if r.lanes.contains(p) {
            // TL-057: a clip brings its linked partners while the option is
            // on; Alt picks one alone.
            let linked = view.linked_selection;
            match hit(p) {
                Some((clip, _)) if shift || command => {
                    project.pick(clip, Pick::Toggle, linked, alt);
                }
                Some((clip, _)) => project.pick(clip, Pick::Replace, linked, alt),
                None => {
                    project.selection.clear();
                    project.markers.clear();
                    project.playhead = quantize(sc.tick(p.x));
                    out.seeked = true;
                }
            }
        }
    }

    // Double-click on a flag opens the marker editor (TL-037).
    if response.double_clicked()
        && let Some(p) = response.interact_pointer_pos()
        && let Some((i, _)) = flag_under(p)
    {
        let f = &flags[i];
        view.marker_edit = MarkerEdit::open(project.doc(), f.r, Pos2::new(f.spot.x, f.floor));
    }
    // Double-click on a text clip (TX-002): the playhead moves into it (to
    // its start when outside) and its content opens for editing.
    if response.double_clicked()
        && tool == TimelineTool::Select
        && let Some(p) = response.interact_pointer_pos()
        && flag_under(p).is_none()
        && r.lanes.contains(p)
        && let Some((clip, _)) = hit(p)
        && let Some(c) = project.doc().clip(clip).cloned()
        && matches!(c.content, ClipContent::Text(_))
    {
        project.selection = vec![clip];
        if !c.range.contains(project.playhead) {
            project.playhead = c.start();
            out.seeked = true;
        }
        out.edit_text = true;
    }

    // Right-click: the clip menu (UI-035). An unselected clip is selected
    // first (added to the selection with Cmd/Ctrl); a selected one keeps
    // the whole selection.
    if response.secondary_clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        if let Some((i, _)) = flag_under(p) {
            // A marker's menu (TL-037); it selects the marker first.
            let m = flags[i].r;
            if !project.markers.contains(&m) {
                project.markers = BTreeSet::from([m]);
            }
            view.menu = Some(MenuFor::Marker(m));
        } else if r.ruler.contains(p) {
            view.menu = Some(MenuFor::Ruler);
        } else {
            let clip = hit(p).map(|(clip, _)| clip);
            view.menu = clip.map(|_| MenuFor::Clip);
            if let Some(clip) = clip
                && !project.selection.contains(&clip)
            {
                let pick = if ui.input(|i| i.modifiers.command) {
                    Pick::Toggle
                } else {
                    Pick::Replace
                };
                let alt = ui.input(|i| i.modifiers.alt);
                project.pick(clip, pick, view.linked_selection, alt);
            }
        }
    }
    if let Some(menu) = view.menu {
        let selection = project.selection.clone();
        let anchor = |m: MarkerRef| {
            flags
                .iter()
                .find(|f| f.r == m)
                .map_or(r.ruler.left_bottom(), |f| Pos2::new(f.spot.x, f.floor))
        };
        let mut edit = None;
        let shown = response.context_menu(|ui| match menu {
            MenuFor::Clip => {
                clip_menu(ui, project, &selection, &mut out);
                if let [clip] = selection.as_slice() {
                    let has = project
                        .doc()
                        .clip(*clip)
                        .is_some_and(|c| !c.markers.is_empty());
                    if has {
                        ui.separator();
                        if ui.button("Delete markers on this clip").clicked() {
                            if let Err(e) = project.delete_markers(
                                freecut_timeline::edit::MarkerSelector::OnClip(*clip),
                            ) {
                                out.status = Some(e);
                            }
                            ui.close();
                        }
                    }
                }
            }
            MenuFor::Marker(m) => {
                if ui.button("Edit marker…").clicked() {
                    edit = Some(m);
                    ui.close();
                }
                marker_menu(ui, project, m, &mut out);
            }
            MenuFor::Ruler => {
                let any = project.has_timeline_markers();
                if ui
                    .add_enabled(any, egui::Button::new("Delete all timeline markers"))
                    .clicked()
                {
                    if let Err(e) =
                        project.delete_markers(freecut_timeline::edit::MarkerSelector::AllTimeline)
                    {
                        out.status = Some(e);
                    }
                    ui.close();
                }
            }
        });
        if let Some(m) = edit {
            view.marker_edit = MarkerEdit::open(project.doc(), m, anchor(m));
        }
        if shown.is_none() {
            view.menu = None;
        }
    }

    // The marker editor popover (TL-037): one EditMarker when it closes.
    if let Some(edit) = &mut view.marker_edit {
        match markers::editor(ui.ctx(), edit, rate) {
            EditOutcome::Open => {}
            EditOutcome::Discard => view.marker_edit = None,
            EditOutcome::Commit => {
                if let Some(m) = markers::find_marker(project.doc(), edit.marker) {
                    let patch = edit.patch(&m, rate);
                    if !markers::patch_is_empty(&patch)
                        && let Err(e) = project.edit_marker(edit.marker, patch)
                    {
                        out.status = Some(e);
                    }
                }
                view.marker_edit = None;
            }
        }
    }

    // An item dropped from the asset panel.
    if let Some(payload) = response.dnd_release_payload::<AssetDrag>()
        && let Some(p) = response.hover_pos().or(pointer)
    {
        let hit = rows
            .iter()
            .find(|r| p.y >= r.top && p.y < r.top + r.height)
            .map(|r| r.track);
        let at = quantize(sc.tick(p.x.max(r.lanes.left())));
        let result = match *payload {
            AssetDrag::Media(media) => {
                let visual = doc
                    .library()
                    .get(media)
                    .is_some_and(|m| m.info.has_video() || m.info.is_still());
                insert_media(project, media, drop_target(seq, hit, visual), at)
            }
            // TX-001: at the drop point, on the track under the pointer
            // (TL's rules move it to the nearest Text track, or make one).
            AssetDrag::Text => {
                let target = hit.map_or(InsertTarget::FirstFree, InsertTarget::Track);
                insert_text(project, target, at).map(|_| ())
            }
        };
        if let Err(e) = result {
            out.status = Some(e);
        }
    }
    // Hover skim (TL-014) and the blade's preview (TL-024): the frame under
    // the pointer, past the end showing the last frame.
    out.dragging = !matches!(view.drag, Drag::None);
    if !out.dragging {
        let end = seq.duration();
        out.pointer_time = response
            .hover_pos()
            .filter(|pos| body.contains(*pos))
            .map(|pos| quantize(sc.tick(pos.x)).min(end));
        out.blade_time = blade_at.map(|(_, t)| t);
    }
    out
}

/// The take being recorded: a `danger` clip with the levels so far as bars.
fn draw_live_take(painter: &egui::Painter, look: &Look, take: &LiveTake, lane: Rect, sc: Scale) {
    let p = look.palette;
    let x0 = sc.x(take.start);
    let end = take.start.checked_add(take.length).unwrap_or(take.start);
    let x1 = sc.x(end).max(x0 + 2.0);
    let rect = Rect::from_x_y_ranges(x0..=x1, lane.top() + 2.0..=lane.bottom() - 2.0);
    painter.rect_filled(rect, theme::R_XS, p.danger.gamma_multiply(0.28));
    painter.rect_stroke(
        rect,
        theme::R_XS,
        Stroke::new(1.5, p.danger),
        StrokeKind::Inside,
    );
    let n = take.peaks.len();
    if n > 0 && rect.width() > BAR_PITCH {
        let mid = rect.center().y;
        let half = (rect.height() / 2.0 - 4.0).max(1.0);
        let mut x = rect.left() + 2.0;
        while x + BAR_W < rect.right() - 1.0 {
            let f = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let i = ((f * n as f32) as usize).min(n - 1);
            let h = (take.peaks[i].clamp(0.0, 1.0) * half).max(0.5);
            painter.rect_filled(
                Rect::from_x_y_ranges(x..=x + BAR_W, mid - h..=mid + h),
                0.0,
                p.danger,
            );
            x += BAR_PITCH;
        }
    }
    if rect.width() > 60.0 {
        painter.text(
            rect.left_top() + Vec2::new(5.0, 3.0),
            Align2::LEFT_TOP,
            "Recording",
            FontId::proportional(11.0),
            p.text_primary,
        );
    }
}

/// A marker's context menu after *Edit marker…*: delete it, or every
/// marker of its colour (TL-037).
fn marker_menu(ui: &mut egui::Ui, project: &mut Project, m: MarkerRef, out: &mut TimelineOutput) {
    use freecut_timeline::edit::MarkerSelector;
    if ui.button("Delete marker").clicked() {
        if let Err(e) = project.delete_markers(MarkerSelector::Refs(vec![m])) {
            out.status = Some(e);
        }
        ui.close();
    }
    let color = markers::find_marker(project.doc(), m).map(|mk| mk.color);
    if let Some(color) = color
        && ui
            .button(format!(
                "Delete all {} markers",
                crate::timeline_commands::color_name(color).to_lowercase()
            ))
            .clicked()
    {
        if let Err(e) = project.delete_markers(MarkerSelector::Color(color)) {
            out.status = Some(e);
        }
        ui.close();
    }
}

/// Commits a trim, and with linked partners (TL-057) trims each of them by
/// the same amount in the same undo entry.
fn commit_trim(
    project: &mut Project,
    req: &EditRequest,
    partners: &[ClipId],
) -> Result<(), String> {
    let EditRequest::Trim(main) = req else {
        return project.apply(req, Coalesce::Never).map(|_| ());
    };
    if partners.is_empty() {
        return project.apply(req, Coalesce::Never).map(|_| ());
    }
    let gesture = project.begin_gesture();
    let result = project.apply(req, Coalesce::Gesture(gesture)).map(|_| ());
    if result.is_ok() {
        for partner in partners {
            let req = EditRequest::Trim(TrimReq {
                clip: *partner,
                ..main.clone()
            });
            // A partner the trim can't reach stays as it is.
            let _ = project.apply(&req, Coalesce::Gesture(gesture));
        }
    }
    project.end_gesture(gesture);
    result
}

/// Starts dragging marker flag `f` (its start, or a range's end), from
/// pointer time `pointer`.
fn marker_drag(
    doc: &Document,
    seq: &Sequence,
    f: &Flag,
    grab: FlagGrab,
    pointer: Tick,
    playhead: Tick,
) -> Option<Drag> {
    let m = markers::find_marker(doc, f.r)?;
    let start = match f.r {
        MarkerRef::Timeline(_) => m.at.start(),
        MarkerRef::Clip(clip, _) => doc.clip(clip)?.marker_start(&m)?,
    };
    let end = matches!(grab, FlagGrab::End(_)) && matches!(f.r, MarkerRef::Timeline(_));
    let from = if end { m.at.end() } else { start };
    Some(Drag::Marker {
        marker: f.r,
        end,
        start_tick: pointer,
        from,
        anchor: start,
        snap: SnapIndex::build(seq, Some(playhead), &BTreeSet::new()),
        to: from,
    })
}

/// The flags to draw this frame: timeline markers on the ruler floor and
/// the shown markers of clips at least 24 pt wide; `dragged` moves one.
fn layout_flags(
    seq: &Sequence,
    project: &Project,
    draws: &[ClipDraw],
    sc: Scale,
    r: &TimelineRects,
    dragged: Option<(MarkerRef, bool, Tick)>,
) -> Vec<Flag> {
    let mut flags = Vec::new();
    let place = |mref: MarkerRef, start: Tick, end: Option<Tick>| -> (f32, Option<f32>) {
        match dragged {
            Some((d, false, to)) if d == mref => {
                let shift = to.checked_sub(start).unwrap_or(Tick::ZERO);
                (
                    sc.x(to),
                    end.map(|e| sc.x(e.checked_add(shift).unwrap_or(e))),
                )
            }
            Some((d, true, to)) if d == mref => (sc.x(start), Some(sc.x(to))),
            _ => (sc.x(start), end.map(|e| sc.x(e))),
        }
    };
    for m in &seq.markers {
        let mref = MarkerRef::Timeline(m.id);
        let end = (!m.at.is_point()).then(|| m.at.end());
        let (x, end_x) = place(mref, m.at.start(), end);
        if x > r.ruler.right() + 2.0 || end_x.unwrap_or(x) + markers::CLIP_FLAG_W < r.ruler.left() {
            continue;
        }
        flags.push(Flag {
            r: mref,
            spot: FlagSpot { x, end_x },
            color: m.color,
            name: m.name.clone(),
            floor: r.ruler.bottom() - 1.0,
            top: r.ruler.bottom() - 16.0,
            ruler: true,
            selected: project.markers.contains(&mref),
            clip: None,
        });
    }
    for d in draws {
        let clip = &d.clip;
        if clip.markers.is_empty() || d.parts.rect.width() < markers::CLIP_FLAGS_MIN_WIDTH {
            continue;
        }
        let floor = if d.parts.shape == ClipShape::AllStrip {
            (d.parts.rect.top() + 12.0).min(d.parts.rect.bottom())
        } else {
            d.parts.strip.bottom()
        } - 1.0;
        for m in &clip.markers {
            let Some(start) = clip.marker_start(m) else {
                continue;
            };
            let mref = MarkerRef::Clip(clip.id, m.id);
            let end = (!m.at.is_point())
                .then(|| clip.sequence_time_of(m.at.end()))
                .flatten()
                .map(|e| e.min(clip.end()));
            let (x, end_x) = place(mref, start, end);
            flags.push(Flag {
                r: mref,
                spot: FlagSpot { x, end_x },
                color: m.color,
                name: m.name.clone(),
                floor,
                top: floor - 10.0,
                ruler: false,
                selected: project.markers.contains(&mref),
                clip: Some(d.parts.rect.intersect(r.lanes)),
            });
        }
    }
    flags
}

/// The clip context menu's items for the current selection (UI-035's mvp
/// list plus TL-030 and AU-038). Unavailable items stay visible, disabled,
/// with the reason on hover.
fn clip_menu(ui: &mut egui::Ui, project: &Project, selection: &[ClipId], out: &mut TimelineOutput) {
    let doc = project.doc();
    let mut item =
        |ui: &mut egui::Ui, label: &str, shortcut: &str, command: Result<ClipCommand, &str>| {
            let button = egui::Button::new(label).shortcut_text(shortcut);
            match command {
                Ok(command) => {
                    if ui.add(button).clicked() {
                        out.command = Some(command);
                        ui.close();
                    }
                }
                Err(reason) => {
                    ui.add_enabled(false, button).on_disabled_hover_text(reason);
                }
            }
        };
    item(ui, "Split at playhead", "S", Ok(ClipCommand::Split));
    item(ui, "Delete", "Del", Ok(ClipCommand::Delete));
    ui.separator();
    match mute_command(doc, selection) {
        Ok(true) => item(ui, "Mute audio", "", Ok(ClipCommand::SetMuted(true))),
        Ok(false) => item(ui, "Unmute audio", "", Ok(ClipCommand::SetMuted(false))),
        Err(reason) => item(ui, "Mute audio", "", Err(reason)),
    }
    match sound_command(doc, selection) {
        SoundCommand::Detach(ready) => {
            item(ui, "Detach audio", "", ready.map(|()| ClipCommand::Detach))
        }
        SoundCommand::Restore => item(ui, "Restore audio", "", Ok(ClipCommand::Restore)),
    }
    // TL-030: offered while something selected is out of sync.
    if clip_streams::can_resync(doc, doc.root(), selection) {
        item(ui, "Resync", "", Ok(ClipCommand::Resync));
    }
    stream_menus(ui, project, selection, out);
}

/// "Sound source ▸" and "Add stream as clip ▸" (AU-038).
fn stream_menus(
    ui: &mut egui::Ui,
    project: &Project,
    selection: &[ClipId],
    out: &mut TimelineOutput,
) {
    let doc = project.doc();
    if let Some(picker) = clip_streams::selection_picker(doc, selection) {
        match picker.disabled {
            Some(reason) => {
                ui.add_enabled(
                    false,
                    egui::Button::new(format!("Sound source {}", crate::theme::icons::CARET_RIGHT)),
                )
                .on_disabled_hover_text(reason);
            }
            None => {
                ui.menu_button("Sound source", |ui| {
                    for (i, choice) in picker.choices.iter().enumerate() {
                        if let Some(reason) = choice.blocked {
                            ui.add_enabled(
                                false,
                                egui::RadioButton::new(i == picker.current, &choice.label),
                            )
                            .on_disabled_hover_text(reason);
                            continue;
                        }
                        if ui.radio(i == picker.current, &choice.label).clicked() {
                            if i != picker.current {
                                out.command = Some(ClipCommand::SetStream(choice.sel));
                            }
                            ui.close();
                        }
                    }
                });
            }
        }
    }
    if !clip_streams::offers_add_stream(doc, selection) {
        return;
    }
    ui.menu_button("Add stream as clip", |ui| {
        for entry in clip_streams::add_stream_menu(&project.editor, selection) {
            match entry.state {
                AddStreamState::Add => {
                    if ui
                        .button(&entry.label)
                        .on_hover_text("Put this stream on its own audio clip, linked to the video")
                        .clicked()
                    {
                        out.command = Some(ClipCommand::AddStream(entry.ordinal));
                        ui.close();
                    }
                }
                AddStreamState::AlreadyPlaying(Ok(())) => {
                    let label = format!("{} (playing): Detach audio", entry.label);
                    if ui
                        .button(label)
                        .on_hover_text(
                            "The clip already plays this stream; detach its audio instead",
                        )
                        .clicked()
                    {
                        out.command = Some(ClipCommand::Detach);
                        ui.close();
                    }
                }
                AddStreamState::AlreadyPlaying(Err(reason)) => {
                    let label = format!("{} (playing)", entry.label);
                    ui.add_enabled(false, egui::Button::new(label))
                        .on_disabled_hover_text(reason);
                }
                AddStreamState::Unavailable(reason) => {
                    ui.add_enabled(false, egui::Button::new(&entry.label))
                        .on_disabled_hover_text(reason);
                }
            }
        }
    });
}

/// Where a library item dropped on the timeline lands (TL-012): the track
/// under the pointer, else a new track. While the sequence holds no clips, a
/// picture or video goes to the main track wherever it is dropped, so the
/// first clip never ends up on an overlay above an empty main track.
fn drop_target(seq: &Sequence, hit: Option<TrackId>, visual: bool) -> InsertTarget {
    match (seq.main_track(), hit) {
        (Some(main), _) if visual && seq.duration() == Tick::ZERO => InsertTarget::Track(main.id),
        (_, Some(track)) => InsertTarget::Track(track),
        (_, None) => InsertTarget::NewTrack,
    }
}

/// Places a library item on the timeline as one undo step.
pub fn insert_media(
    project: &mut Project,
    media: MediaId,
    target: InsertTarget,
    at: Tick,
) -> Result<(), String> {
    let doc = project.doc();
    let item = doc
        .library()
        .get(media)
        .ok_or("That media is no longer in the project")?;
    let clip = if item.info.is_still() {
        NewClip::image(
            media,
            Tick::from_secs(STILL_SECONDS).map_err(|e| e.to_string())?,
        )
    } else {
        let kind = if item.info.has_video() {
            MediaKind::Video
        } else {
            MediaKind::Audio
        };
        let source = TimeRange::new(Tick::ZERO, item.info.duration).map_err(|e| e.to_string())?;
        NewClip::media(media, kind, source)
    };
    let req = EditRequest::Insert(InsertReq {
        seq: doc.root(),
        target,
        at,
        clips: vec![clip],
    });
    project.apply(&req, Coalesce::Never).map(|_| ())
}

/// The asset panels' "add at playhead" (UI-005): a picture or video goes
/// on the main track at the playhead (TL-012, at the nearest cut when the
/// track is magnetic); sound goes on the first audio track that is free
/// there (AU-030). The new clip becomes selected; the playhead stays.
pub fn add_at_playhead(project: &mut Project, media: MediaId) -> Result<(), String> {
    let doc = project.doc();
    let item = doc
        .library()
        .get(media)
        .ok_or("That media is no longer in the project")?;
    let visual = item.info.has_video() || item.info.is_still();
    let main = doc
        .sequence(doc.root())
        .and_then(|s| s.main_track())
        .map(|t| t.id);
    let target = match main {
        Some(main) if visual => InsertTarget::Track(main),
        _ => InsertTarget::FirstFree,
    };
    let at = project.playhead;
    insert_media(project, media, target, at)
}

/// The wheel over the timeline (ui-concept §13.26): a plain wheel scrolls
/// the tracks, a horizontal wheel or `Shift` + wheel pans time, `Ctrl` /
/// `Cmd` + wheel or a pinch zooms about the pointer. A drag near the lanes'
/// left or right edge scrolls the view along.
fn scroll_and_zoom(ui: &egui::Ui, view: &mut TimelineView, r: &TimelineRects) {
    let area = r.lanes.union(r.headers).union(r.ruler);
    if ui.rect_contains_pointer(area) {
        let (zoom, scroll, pointer) =
            ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
        if (zoom - 1.0).abs() > f32::EPSILON {
            let old = view.px_per_sec;
            let new = layout::zoom_wheel(old, zoom);
            let dx = pointer.map_or(0.0, |p| (p.x - r.lanes.left()).max(0.0));
            view.scroll_secs = layout::zoom_about(view.scroll_secs, old, new, dx);
            view.px_per_sec = new;
        } else {
            if scroll.x != 0.0 {
                view.scroll_secs -= f64::from(scroll.x / view.px_per_sec);
            }
            if scroll.y != 0.0 {
                view.track_scroll -= scroll.y;
            }
        }
        view.scroll_secs = view.scroll_secs.max(0.0);
    }
    if view.drag.scrolls_along()
        && let Some(pos) = ui.input(|i| {
            i.pointer
                .primary_down()
                .then(|| i.pointer.latest_pos())
                .flatten()
        })
    {
        let step = layout::autoscroll_step(pos.x, r.lanes.x_range());
        if step != 0.0 {
            view.scroll_secs = (view.scroll_secs + f64::from(step / view.px_per_sec)).max(0.0);
            ui.ctx().request_repaint();
        }
    }
}

/// The ruler (ui-concept §13.26): labelled ticks on the first interval of
/// [`layout::tick_interval`]'s ladder, four small ticks between them. A
/// label the playhead's head (at `head`) covers fades out under it.
/// Returns the labelled ticks' x, for the grid lines through the lanes.
fn draw_ruler(
    painter: &egui::Painter,
    ruler: Rect,
    sc: Scale,
    rate: FrameRate,
    head: Option<f32>,
    look: &Look,
) -> Vec<f32> {
    let p = look.palette;
    painter.rect_filled(ruler, 0.0, p.bg_ruler);
    painter.hline(
        ruler.x_range(),
        ruler.bottom() - 0.5,
        Stroke::new(1.0, p.border_subtle),
    );
    let step = layout::tick_interval(sc.pps, rate);
    let step_px = step.as_secs_f64() * f64::from(sc.pps);
    let minor = Tick::from_raw(step.raw() / i64::from(layout::MINOR_TICKS + 1));
    let minor_px = minor.as_secs_f64() * f64::from(sc.pps);
    let first = (sc.scroll / step.as_secs_f64()).floor().max(0.0) as i64;
    let font = FontId::monospace(10.0);
    let tick_h = 8.0;
    let mut out = Vec::new();
    let mut k = first;
    while let Ok(t) = step.checked_mul_int(k) {
        let x = sc.x(t);
        if x > ruler.right() + 1.0 {
            break;
        }
        if minor_px >= 6.0 {
            for j in 1..=layout::MINOR_TICKS {
                let mx = x + (f64::from(j) * minor_px) as f32;
                if mx >= ruler.left() && mx <= ruler.right() {
                    painter.vline(
                        mx,
                        egui::Rangef::new(ruler.bottom() - 4.0, ruler.bottom()),
                        Stroke::new(1.0, p.border_subtle),
                    );
                }
            }
        }
        if x >= ruler.left() - step_px as f32 {
            painter.vline(
                x,
                egui::Rangef::new(ruler.bottom() - tick_h, ruler.bottom()),
                Stroke::new(1.0, p.border_strong),
            );
            let label = Timecode::from_tick(t, rate, false)
                .map_or_else(|_| String::new(), |tc| tc.to_string());
            let galley = painter.layout_no_wrap(label, font.clone(), p.text_tertiary);
            let span = egui::Rangef::new(x + 4.0, x + 4.0 + galley.size().x);
            let opacity = head.map_or(1.0, |h| layout::label_opacity(span, h));
            if opacity > 0.0 {
                let y = ruler.top() + (ruler.height() - tick_h) / 2.0 - galley.size().y / 2.0;
                painter.galley(
                    Pos2::new(span.min, y),
                    galley,
                    p.text_tertiary.gamma_multiply(opacity),
                );
            }
            if x >= ruler.left() {
                out.push(x);
            }
        }
        k += 1;
    }
    out
}

/// The keyframes of the one selected clip, at the ruler's x: (x, time).
fn ruler_keys(doc: &Document, project: &Project, sc: Scale, ruler: Rect) -> Vec<(f32, Tick)> {
    let [id] = project.selection.as_slice() else {
        return Vec::new();
    };
    let Some(clip) = doc.clip(*id) else {
        return Vec::new();
    };
    let mut times: Vec<Tick> = clip
        .keys
        .tracks()
        .iter()
        .flat_map(|t| t.keys().iter().map(|k| k.at))
        .filter_map(|at| clip.sequence_time_of(at))
        .filter(|t| clip.range.contains(*t) || *t == clip.end())
        .collect();
    times.sort_unstable();
    times.dedup();
    times
        .into_iter()
        .map(|t| (sc.x(t), t))
        .filter(|(x, _)| *x >= ruler.left() - 5.0 && *x <= ruler.right() + 5.0)
        .collect()
}

/// What painting a clip needs.
struct ClipCtx<'a> {
    /// Clipped to the lanes.
    painter: &'a egui::Painter,
    look: &'a Look,
    doc: &'a Document,
    project: &'a Project,
    waves: &'a Waveforms,
    sc: Scale,
    frame: Tick,
    lanes: Rect,
    ppp: f32,
}

/// A clip's volume line as painted: the line, whether it is hovered or
/// held, and the pill's text with the pointer's x.
type LineLook<'a> = (&'a LineGeom, bool, Option<(String, f32)>);

/// Paints one clip (ui-concept §13.26): two-tone or all strip, its
/// filmstrip, sound and volume line, the In / Out shading, its name, glyph
/// and badges, the hover or selection outline, and the veil of a locked or
/// hidden track.
fn draw_clip(
    cx: &ClipCtx<'_>,
    films: &mut Filmstrips,
    d: &ClipDraw,
    line: Option<LineLook<'_>>,
    picked: &[Tick],
    hovered: bool,
    sync: Option<String>,
) {
    let p = cx.look.palette;
    let painter = cx.painter;
    let clip = &d.clip;
    let parts = d.parts;
    let r = parts.rect;
    let mark = clip_color(cx.doc, &clip.content);
    let well = surface::clip_well(mark, cx.look.kind);
    match parts.shape {
        ClipShape::AllStrip => {
            painter.rect_filled(r, theme::R_SM, mark);
        }
        ClipShape::TwoTone => {
            painter.rect_filled(r, theme::R_SM, well);
            let radius = if parts.body.height() > 0.0 {
                CornerRadius {
                    nw: theme::R_SM,
                    ne: theme::R_SM,
                    sw: 0,
                    se: 0,
                }
            } else {
                CornerRadius::same(theme::R_SM)
            };
            painter.rect_filled(parts.strip, radius, mark);
        }
    }
    let offline = clip
        .content
        .asset()
        .is_some_and(|a| cx.project.offline.contains(&a));
    if let Some(frames) = parts.frames {
        draw_filmstrip(cx, films, clip, frames, r, mark, offline);
    }
    let audio = clip.audio.active();
    if let (Some(band), Some(audio)) = (parts.band, audio) {
        let video = matches!(
            clip.content,
            ClipContent::Media {
                kind: MediaKind::Video,
                ..
            }
        );
        if video {
            // `clip.well` darkened: a step down from the frames.
            let darker = if cx.look.kind == theme::ThemeKind::Light {
                0.08
            } else {
                0.3
            };
            let ground = well.lerp_to_gamma(Color32::BLACK, darker);
            let radius = CornerRadius {
                nw: 0,
                ne: 0,
                sw: theme::R_SM,
                se: theme::R_SM,
            };
            painter.rect_filled(band, radius, ground);
            painter.hline(
                band.x_range(),
                band.top() + 0.5,
                Stroke::new(1.0, p.border_subtle),
            );
        }
        if !offline {
            draw_waves(cx, clip, audio, band, mark);
        }
    }
    // TX-022 / FX-018: the In and Out play over shaded ramps.
    let ink = surface::label_on(mark);
    for (slot, a, b) in crate::anim::shaded_regions(clip, cx.frame) {
        let (xa, xb) = (cx.sc.x(a).max(r.left()), cx.sc.x(b).min(r.right()));
        if xb - xa < 1.0 {
            continue;
        }
        let region = Rect::from_min_max(Pos2::new(xa, r.top()), Pos2::new(xb, r.bottom()));
        painter.rect_filled(region, theme::R_SM, Color32::from_black_alpha(56));
        let (from, to) = match slot {
            freecut_timeline::model::Slot::Exit => (region.left_top(), region.right_bottom()),
            _ => (region.left_bottom(), region.right_top()),
        };
        painter.line_segment([from, to], Stroke::new(1.0, ink.gamma_multiply(0.6)));
    }
    if let (Some((geom, hot, pill)), Some(audio)) = (line, audio) {
        draw_volume(cx, clip, audio, geom, well, hot, picked, pill);
    }
    draw_label(cx, d, mark, offline, sync);
    if d.locked || d.dimmed {
        let veil = if d.locked && d.dimmed { 0.65 } else { 0.5 };
        painter.rect_filled(r.expand(0.5), theme::R_SM, d.lane.gamma_multiply(veil));
    }
    if d.selected {
        painter.rect_stroke(
            r,
            theme::R_SM,
            Stroke::new(2.0, p.clip_selected),
            StrokeKind::Inside,
        );
    } else if hovered {
        painter.rect_stroke(
            r,
            theme::R_SM,
            Stroke::new(1.0, p.border_strong),
            StrokeKind::Inside,
        );
    }
}

/// The filmstrip of a video or picture clip in `frames`: one frame per
/// tile from the background job, the nearest frame while a tile's own is
/// on its way, and the kind glyph while none is.
fn draw_filmstrip(
    cx: &ClipCtx<'_>,
    films: &mut Filmstrips,
    clip: &Clip,
    frames: Rect,
    r: Rect,
    mark: Color32,
    offline: bool,
) {
    let Some(asset) = clip.content.asset() else {
        return;
    };
    let item = cx.doc.library().get(asset);
    let aspect = item.and_then(|i| display_aspect(i)).unwrap_or(16.0 / 9.0);
    let still = item.is_some_and(|i| i.info.is_still());
    let tiles = layout::filmstrip_tiles(frames, cx.lanes, aspect);
    let height_px = height_bucket(frames.height(), cx.ppp);
    let step = bucket_step(cx.sc.span(frames.height() * aspect));
    let round_bottom = (frames.bottom() - r.bottom()).abs() < 0.5;
    let mut shown = false;
    for (tile, mid) in tiles {
        if offline {
            break;
        }
        let at = if still {
            Tick::ZERO
        } else {
            let t = cx.sc.tick(mid).clamp(clip.start(), clip.end());
            bucket_time(clip.source_time_at(t).unwrap_or(Tick::ZERO), step)
        };
        let key = FilmKey {
            media: asset,
            height_px,
            at,
        };
        let source = || {
            let path = cx.project.media_paths.get(&asset)?.clone();
            FilmSource::of(item?, path)
        };
        let Some((texture, _)) = films.frame(key, step, source) else {
            continue;
        };
        shown = true;
        let part = tile.intersect(frames);
        if part.width() <= 0.0 {
            continue;
        }
        let u0 = (part.left() - tile.left()) / tile.width();
        let u1 = (part.right() - tile.left()) / tile.width();
        let corner = |at_edge: bool| {
            if round_bottom && at_edge {
                theme::R_SM
            } else {
                0
            }
        };
        let radius = CornerRadius {
            nw: 0,
            ne: 0,
            sw: corner(part.left() <= r.left() + 0.5),
            se: corner(part.right() >= r.right() - 0.5),
        };
        cx.painter.add(
            egui::epaint::RectShape::filled(part, radius, Color32::WHITE).with_texture(
                texture,
                Rect::from_min_max(Pos2::new(u0, 0.0), Pos2::new(u1, 1.0)),
            ),
        );
    }
    let visible = frames.intersect(cx.lanes);
    if !shown && visible.width() > 40.0 && visible.height() > 16.0 {
        let glyph = if still { icons::IMAGE } else { icons::VIDEO };
        let size = 20.0f32.min(visible.height() - 4.0);
        cx.painter.text(
            visible.center(),
            Align2::CENTER_CENTER,
            glyph,
            theme::fonts::icon_font(cx.painter.ctx(), size, false),
            mark.gamma_multiply(0.3).lerp_to_gamma(mark, 0.3),
        );
    }
}

/// The waveform in `band` (ui-concept §13.26): bars 2 pt wide on a 3 pt
/// pitch, symmetric about the band's middle, in `clip.wave`, their parts at
/// or above −1 dBFS in `warning`; the height follows the clip's volume and
/// fades. Muted clips show grey bars; offline media none.
fn draw_waves(cx: &ClipCtx<'_>, clip: &Clip, audio: &ClipAudio, band: Rect, mark: Color32) {
    let ClipContent::Media {
        asset,
        source,
        stream,
        ..
    } = &clip.content
    else {
        return;
    };
    let p = cx.look.palette;
    let wave = if audio.muted {
        p.text_tertiary.gamma_multiply(0.5)
    } else {
        surface::clip_wave(mark, cx.look.kind)
    };
    let Some(key) = peak_key(cx.doc, *asset, *stream) else {
        return;
    };
    let visible = band.intersect(cx.lanes);
    match cx.waves.get(key) {
        Some(PeakState::Ready(peaks)) => {
            let cy = band.center().y;
            let half = (band.height() / 2.0 - 1.0).max(0.5);
            let mut shapes = Vec::new();
            for x in layout::wave_bars(band.left(), visible.left(), visible.right(), BAR_PITCH) {
                let span = column_source_range(
                    clip.range,
                    source.start(),
                    cx.sc.tick(x),
                    cx.sc.tick(x + BAR_PITCH),
                );
                let Some((lo, hi)) = span.and_then(|(a, b)| peaks.range(a, b)) else {
                    continue;
                };
                let db = clip_keys::envelope_db_at(clip, audio, cx.sc.tick(x + BAR_PITCH / 2.0));
                let gain = if db.is_finite() {
                    10f64.powf(db / 20.0) as f32
                } else {
                    0.0
                };
                let amp = lo.abs().max(hi.abs()) * gain;
                let (top, bottom, hot) = layout::bar_extent(amp, cy, half);
                let bar = |y0: f32, y1: f32, color: Color32| {
                    Shape::rect_filled(
                        Rect::from_min_max(Pos2::new(x, y0), Pos2::new(x + BAR_W, y1)),
                        0.0,
                        color,
                    )
                };
                match hot {
                    Some(h) if !audio.muted => {
                        shapes.push(bar(cy - h, cy + h, wave));
                        shapes.push(bar(top, cy - h, p.warning));
                        shapes.push(bar(cy + h, bottom, p.warning));
                    }
                    _ => shapes.push(bar(top, bottom, wave)),
                }
            }
            cx.painter.extend(shapes);
        }
        Some(PeakState::Failed(reason)) => {
            // AU-009: distinct from the offline pattern, with the cause.
            cx.painter.with_clip_rect(visible).text(
                Pos2::new(band.left().max(visible.left()) + 6.0, band.center().y),
                Align2::LEFT_CENTER,
                format!("Waveform unavailable: {reason}"),
                FontId::proportional(10.0),
                wave,
            );
        }
        Some(PeakState::Pending) | None => {
            cx.painter.hline(
                visible.x_range(),
                band.center().y,
                Stroke::new(1.0, wave.gamma_multiply(0.5)),
            );
        }
    }
}

/// The volume line over a clip's sound (AU-008, AU-009): the volume and
/// keyframes times the fades, one point per two pixel columns, the fade
/// ramps shaded above it and a grey veil when the clip is muted (AU-007);
/// 1 pt at 70 %, 2 pt when hovered or held, with the level in a pill at
/// its left end. Keyframe dots are 7 pt, `picked` ones in the accent.
#[allow(clippy::too_many_arguments)]
fn draw_volume(
    cx: &ClipCtx<'_>,
    clip: &Clip,
    audio: &ClipAudio,
    geom: &LineGeom,
    well: Color32,
    hot: bool,
    picked: &[Tick],
    pill: Option<(String, f32)>,
) {
    let p = cx.look.palette;
    let painter = cx.painter;
    let band = geom.band;
    let visible = band.intersect(cx.lanes);
    let (top, bottom) = line_span(band);
    if audio.muted {
        painter.rect_filled(band, 0.0, Color32::from_black_alpha(56));
    }
    // White on dark bodies, near-black on light ones.
    let ink = surface::label_on(well);
    let other = if ink == Color32::WHITE {
        surface::LABEL_DARK
    } else {
        Color32::WHITE
    };
    let x0 = band.left().max(visible.left()).floor();
    let x1 = band.right().min(visible.right()).ceil();
    let mut points = Vec::new();
    let mut shade = Vec::new();
    let mut x = x0;
    while x <= x1 {
        let t = cx.sc.tick(x).clamp(clip.start(), clip.end());
        let y = clip_keys::y_of_db(clip_keys::envelope_db_at(clip, audio, t), top, bottom);
        if clip_keys::fades_at(clip, audio, t) < 1.0 {
            shade.push(Shape::line_segment(
                [Pos2::new(x, top), Pos2::new(x, y)],
                Stroke::new(2.0, Color32::from_black_alpha(102)),
            ));
        }
        points.push(Pos2::new(x, y));
        x += 2.0;
    }
    painter.extend(shade);
    let stroke = if hot {
        Stroke::new(2.0, ink)
    } else {
        Stroke::new(1.0, ink.gamma_multiply(0.7))
    };
    if points.len() >= 2 {
        painter.add(Shape::line(points, stroke));
    }
    for (key, pos) in &geom.dots {
        if !visible.expand(4.0).contains(*pos) {
            continue;
        }
        let fill = if picked.contains(&key.at) {
            p.accent
        } else {
            ink
        };
        painter.circle(*pos, 3.5, fill, Stroke::new(1.0, other.gamma_multiply(0.8)));
    }
    if let Some((text, px)) = pill {
        let style = cx.painter.ctx().global_style();
        let font = Type::Caption.style(&style).resolve(&style);
        let galley = painter.layout_no_wrap(text, font, other);
        let size = Vec2::new(galley.size().x + 10.0, 16.0);
        let t = cx.sc.tick(px).clamp(clip.start(), clip.end());
        let y = clip_keys::y_of_db(clip_keys::envelope_db_at(clip, audio, t), top, bottom);
        // Above the line, or under it when there's no room above.
        let y = if y - 20.0 >= cx.lanes.top() {
            y - 20.0
        } else {
            y + 4.0
        };
        let x = (px + 10.0).min(visible.right() - size.x - 2.0);
        let pill = Rect::from_min_size(Pos2::new(x, y), size);
        painter.rect_filled(pill, theme::R_SM, ink);
        painter.galley(
            Pos2::new(pill.left() + 5.0, pill.center().y - galley.size().y / 2.0),
            galley,
            other,
        );
    }
}

/// A badge at a clip's strip end.
enum Badge {
    Text(String, Color32),
    Icon(&'static str),
}

/// A clip's name (with the kind glyph on all-strip clips) and its badges
/// at the strip's right end: the out-of-sync offset (TL-030), offline, a
/// missing LUT (CL-017), detached sound, muted, In / Out / Loop animation,
/// keyframes.
fn draw_label(cx: &ClipCtx<'_>, d: &ClipDraw, mark: Color32, offline: bool, sync: Option<String>) {
    let p = cx.look.palette;
    let clip = &d.clip;
    let r = d.parts.rect;
    let strip = d.parts.strip;
    if r.width() < layout::NAME_MIN_WIDTH || strip.height() < 10.0 {
        return;
    }
    let ink = surface::label_on(mark);
    let ctx = cx.painter.ctx();
    let style = ctx.global_style();
    let caption = Type::Caption.style(&style).resolve(&style);
    let micro = Type::Micro.style(&style).resolve(&style);
    let visible = r.intersect(cx.lanes);
    // Badges right to left from the strip's end.
    let mut badges = Vec::new();
    if let Some(offset) = sync {
        badges.push(Badge::Text(offset, p.danger));
    }
    if offline {
        badges.push(Badge::Text("Offline".to_owned(), p.danger));
    }
    // CL-017: a LUT whose file can't be found grades nothing.
    if let Some(slot) = clip.color.as_ref().and_then(|c| c.lut.as_ref())
        && cx.project.luts.is_missing(&slot.lut)
    {
        badges.push(Badge::Text("LUT missing".to_owned(), p.danger));
    }
    match &clip.audio {
        EmbeddedAudio::Detached(_) => badges.push(Badge::Icon(icons::LINK)),
        EmbeddedAudio::Active(a) if a.muted => badges.push(Badge::Icon(icons::SPEAKER_MUTED)),
        _ => {}
    }
    if !clip.animations.is_empty() {
        badges.push(Badge::Icon(icons::LENS_MOTION));
    }
    if !clip.keys.is_empty() {
        badges.push(Badge::Icon(icons::DIAMOND));
    }
    let chip_h = (strip.height() - 5.0).min(16.0);
    let mut right = visible.right().min(strip.right()) - 3.0;
    if r.width() > layout::BADGE_MIN_WIDTH && chip_h >= 10.0 {
        for badge in badges {
            let (galley, fill) = match badge {
                Badge::Text(text, fill) => (
                    cx.painter
                        .layout_no_wrap(text, micro.clone(), surface::label_on(fill)),
                    fill,
                ),
                Badge::Icon(icon) => (
                    cx.painter.layout_no_wrap(
                        icon.to_owned(),
                        theme::fonts::icon_font(ctx, 11.0, false),
                        Color32::WHITE,
                    ),
                    Color32::from_black_alpha(64),
                ),
            };
            let w = (galley.size().x + 8.0).max(chip_h);
            let chip = Rect::from_min_size(
                Pos2::new(right - w, strip.center().y - chip_h / 2.0),
                Vec2::new(w, chip_h),
            );
            if chip.left() < visible.left() + 40.0 {
                break;
            }
            cx.painter.rect_filled(chip, theme::R_XS, fill);
            cx.painter
                .galley(chip.center() - galley.size() / 2.0, galley, ink);
            right = chip.left() - 3.0;
        }
    }
    // The name from the visible left edge, so it stays in view while the
    // clip scrolls.
    let mut x = visible.left().max(r.left()) + 6.0;
    if d.parts.shape == ClipShape::AllStrip {
        cx.painter.text(
            Pos2::new(x, strip.center().y),
            Align2::LEFT_CENTER,
            clip_glyph(&clip.content),
            theme::fonts::icon_font(ctx, 12.0, false),
            ink,
        );
        x += 16.0;
    }
    let room = right - x - 2.0;
    if room < 8.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        clip_label(cx.doc, &clip.content, clip.name.as_deref()),
        egui::TextFormat::simple(caption, ink),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = cx.painter.layout_job(job);
    cx.painter.galley(
        Pos2::new(x, strip.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
}

/// The track header column (ui-concept §13.26): per row the kind icon and
/// name, then eye, speaker, solo and lock buttons and a `...` menu; on
/// lanes of Medium height and taller the buttons sit on a second line.
fn draw_headers(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    seq: &Sequence,
    rows: &[Row],
    r: &TimelineRects,
    out: &mut TimelineOutput,
) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let area = r.headers;
    let painter = ui.painter_at(area);
    let doc = project.doc().clone();
    let magnet = seq.flags.main_magnet;
    for row in rows {
        let rect = Rect::from_min_size(
            Pos2::new(area.left(), row.top),
            Vec2::new(area.width(), row.height),
        );
        if !rect.intersects(area) {
            continue;
        }
        painter.hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, p.border_subtle),
        );
        let Some((_, track)) = seq.find_track(row.track) else {
            continue;
        };
        let name = track_name(&doc, doc.root(), track.id);
        header_row(
            ui, &painter, project, view, track, row, rect, area, &name, magnet, out,
        );
    }
}

/// The buttons of one track header, in order: (glyph, tooltip, checked,
/// what a click does).
enum HeaderToggle {
    Hide,
    Mute,
    Solo,
    Lock,
}

#[allow(clippy::too_many_arguments)]
fn header_row(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    project: &mut Project,
    view: &mut TimelineView,
    track: &Track,
    row: &Row,
    rect: Rect,
    area: Rect,
    name: &str,
    magnet: bool,
    out: &mut TimelineOutput,
) {
    let look = theme::look(ui.ctx());
    let (p, m) = (look.palette, look.metrics);
    let left = rect.left() + 8.0;
    let right = rect.right() - 4.0 - layout::SCROLLBAR - 2.0;
    let tall = row.height >= m.lanes[1] - 0.5;
    let line_h = HEADER_CHIP;
    let line_top = if tall {
        rect.top() + 6.0
    } else {
        rect.center().y - line_h / 2.0
    };
    let line_y = line_top + line_h / 2.0;

    // `...` at the right end of the name line.
    let menu_rect = Rect::from_min_size(
        Pos2::new(right - HEADER_CHIP, line_top),
        Vec2::splat(HEADER_CHIP),
    );
    let menu = button_at(
        ui,
        menu_rect,
        area,
        ("track-menu", track.id),
        IconButton::new(icons::DOTS_THREE, "Track options: name, height, delete")
            .glyph(HEADER_GLYPH)
            .side(HEADER_CHIP),
    );
    egui::Popup::menu(&menu)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| track_menu(ui, project, view, track, row.is_main, name, out));

    // The toggles: withholding ones are checked.
    let flags = track.header.flags;
    let mut toggles = Vec::new();
    if row.kind != TrackKind::Audio {
        toggles.push(HeaderToggle::Hide);
    }
    if row.kind.carries_sound() {
        toggles.push(HeaderToggle::Mute);
        toggles.push(HeaderToggle::Solo);
    }
    toggles.push(HeaderToggle::Lock);
    let n = toggles.len() as f32;
    let width = n * HEADER_CHIP + (n - 1.0) * 2.0;
    let second_line = tall && line_top + 2.0 * line_h + 4.0 <= rect.bottom();
    let (mut x, y) = if second_line {
        (left - 2.0, line_top + line_h + 2.0)
    } else {
        (menu_rect.left() - 4.0 - width, line_top)
    };
    for toggle in toggles {
        let solo = project.solo().contains(&track.id);
        let (icon, tip, checked) = match toggle {
            HeaderToggle::Hide if flags.hidden => (icons::EYE_SLASH, "Show track", true),
            HeaderToggle::Hide => (icons::EYE, "Hide track", false),
            HeaderToggle::Mute if flags.muted => (icons::SPEAKER_MUTED, "Unmute track", true),
            HeaderToggle::Mute => (icons::SPEAKER, "Mute track", false),
            HeaderToggle::Solo => (
                icons::SOLO,
                "Solo: hear only this track (Shift-click to add or remove). Preview only",
                solo,
            ),
            HeaderToggle::Lock if flags.locked => (icons::LOCK, "Unlock track", true),
            HeaderToggle::Lock => (icons::LOCK_OPEN, "Lock track", false),
        };
        let at = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(HEADER_CHIP));
        x += HEADER_CHIP + 2.0;
        let salt = (
            "track-toggle",
            track.id,
            match toggle {
                HeaderToggle::Hide => 0,
                HeaderToggle::Mute => 1,
                HeaderToggle::Solo => 2,
                HeaderToggle::Lock => 3,
            },
        );
        let button = IconButton::new(icon, tip)
            .glyph(HEADER_GLYPH)
            .side(HEADER_CHIP)
            .checked(checked);
        if !button_at(ui, at, area, salt, button).clicked() {
            continue;
        }
        let mut new_flags = flags;
        match toggle {
            HeaderToggle::Hide => new_flags.hidden = !new_flags.hidden,
            HeaderToggle::Mute => new_flags.muted = !new_flags.muted,
            HeaderToggle::Lock => new_flags.locked = !new_flags.locked,
            HeaderToggle::Solo => {
                // Solo is view state rather than a header flag (AU-012).
                let add = ui.input(|i| i.modifiers.shift);
                let next = super::mixer::solo_after_click(project.solo(), track.id, add);
                project.set_solo(next);
                continue;
            }
        }
        let req = EditRequest::SetTrackHeader {
            seq: project.doc().root(),
            track: track.id,
            header: TrackHeader {
                name: track.header.name.clone(),
                flags: new_flags,
            },
        };
        if let Err(e) = project.apply(&req, Coalesce::Never) {
            out.status = Some(e);
        }
    }

    // The kind icon and the name, cut before the buttons on its line.
    let name_right = if second_line {
        menu_rect.left() - 6.0
    } else {
        menu_rect.left() - 8.0 - width
    };
    let clipped = painter.with_clip_rect(
        Rect::from_min_max(
            Pos2::new(left, rect.top()),
            Pos2::new(name_right.max(left), rect.bottom()),
        )
        .intersect(area),
    );
    // The compact volume readout (AU-013) on tracks that carry sound:
    // always on audio tracks, on video tracks once changed.
    let readout = (second_line
        && row.kind.carries_sound()
        && (row.kind == TrackKind::Audio || track.mix != TrackAudio::default()))
    .then(|| {
        let mut text = super::mixer::track_readout(track.mix.gain);
        if track.mix.pan != Pan::CENTRE {
            text.push(' ');
            text.push_str(&super::mixer::pan_label(f64::from(track.mix.pan.percent())));
        }
        text
    });
    let micro = Type::Micro.style(ui.style()).resolve(ui.style());
    let mut text_right = name_right;
    if let Some(text) = readout {
        let galley = painter.layout_no_wrap(text, micro, p.text_tertiary);
        let at = Pos2::new(name_right - galley.size().x, line_y - galley.size().y / 2.0);
        if at.x > left + 60.0 {
            text_right = at.x - 6.0;
            painter
                .with_clip_rect(area)
                .galley(at, galley, p.text_tertiary);
        }
    }
    let body = egui::TextStyle::Body.resolve(ui.style());
    // The kind icon gives way to the name when the room is short.
    let name_w = painter
        .layout_no_wrap(name.to_owned(), body.clone(), p.text_primary)
        .size()
        .x;
    let with_glyph = text_right - (left + 20.0) >= name_w.min(64.0);
    let name_x = if with_glyph {
        clipped.text(
            Pos2::new(left + 7.0, line_y),
            Align2::CENTER_CENTER,
            track_glyph(row.kind),
            theme::fonts::icon_font(ui.ctx(), HEADER_GLYPH, false),
            p.text_tertiary,
        );
        left + 20.0
    } else {
        left
    };
    let mut job = egui::text::LayoutJob::single_section(
        name.to_owned(),
        egui::TextFormat::simple(body, p.text_primary),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: (text_right - name_x - if row.is_main && magnet { 18.0 } else { 0.0 }).max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = painter.layout_job(job);
    let name_w = galley.size().x;
    clipped.galley(
        Pos2::new(name_x, line_y - galley.size().y / 2.0),
        galley,
        p.text_primary,
    );
    // The main track's magnet (TL-005), when it is on.
    if row.is_main && magnet {
        clipped.text(
            Pos2::new(name_x + name_w + 10.0, line_y),
            Align2::CENTER_CENTER,
            icons::MAGNET,
            theme::fonts::icon_font(ui.ctx(), 12.0, false),
            p.text_tertiary,
        );
    }
}

/// An icon button placed at `rect`, clipped to `clip`, with an id from
/// `salt` (stable while rows come and go).
fn button_at(
    ui: &mut egui::Ui,
    rect: Rect,
    clip: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
    button: IconButton<'_>,
) -> egui::Response {
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .id_salt(salt)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.set_clip_rect(clip.intersect(ui.clip_rect()));
    button.show(&mut child)
}

/// A track header's `...` menu: its name, its lane height (view state)
/// and Delete track.
fn track_menu(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    track: &Track,
    is_main: bool,
    shown_name: &str,
    out: &mut TimelineOutput,
) {
    ui.set_min_width(190.0);
    ui.label(egui::RichText::new("Name").weak());
    if view.rename.as_ref().is_none_or(|(t, _)| *t != track.id) {
        view.rename = Some((track.id, track.header.name.clone().unwrap_or_default()));
    }
    if let Some((_, text)) = &mut view.rename {
        let field = ui.add(
            egui::TextEdit::singleline(text)
                .hint_text(shown_name)
                .desired_width(f32::INFINITY),
        );
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if field.lost_focus() && enter {
            let name = text.trim();
            let req = EditRequest::SetTrackHeader {
                seq: project.doc().root(),
                track: track.id,
                header: TrackHeader {
                    name: (!name.is_empty()).then(|| name.to_owned()),
                    flags: track.header.flags,
                },
            };
            if track.header.name.as_deref().unwrap_or("") != name
                && let Err(e) = project.apply(&req, Coalesce::Never)
            {
                out.status = Some(e);
            }
            view.rename = None;
            ui.close();
        }
    }
    ui.separator();
    ui.label(egui::RichText::new("Height").weak());
    let current = view.size_of(track.id);
    for size in LaneSize::ALL {
        if ui.radio(current == size, size.label()).clicked() {
            if size == LaneSize::Auto {
                view.lane_size.remove(&track.id);
            } else {
                view.lane_size.insert(track.id, size);
            }
            ui.close();
        }
    }
    ui.separator();
    let delete = egui::Button::new(crate::widgets::icon_then_text(
        ui,
        icons::DELETE,
        "Delete track",
    ));
    let response = ui.add_enabled(!is_main, delete);
    if is_main {
        response.on_disabled_hover_text("The main track can't be deleted");
    } else if response.clicked() {
        let req = EditRequest::RemoveTrack {
            seq: project.doc().root(),
            track: track.id,
        };
        if let Err(e) = project.apply(&req, Coalesce::Never) {
            out.status = Some(e);
        }
        view.lane_size.remove(&track.id);
        view.rename = None;
        ui.close();
    }
}

/// The 1 pt hairline between the header column and the lanes: drag to
/// resize the column, double-click for its default width. Accent while
/// hovered or dragged.
fn header_splitter(ui: &mut egui::Ui, view: &mut TimelineView, r: &TimelineRects, full: Rect) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let id = ui.id().with("timeline-header-splitter");
    let response = ui.interact(r.splitter, id, Sense::click_and_drag());
    let hot = response.hovered() || response.dragged();
    if hot {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        view.header_w = Some(layout::clamp_header_width(
            pos.x - full.left(),
            full.width(),
            &look.metrics,
        ));
    }
    if response.double_clicked() {
        view.header_w = None;
    }
    let t = ui
        .ctx()
        .animate_bool_with_time(id.with("hot"), hot, ui.style().animation_time);
    let color = p.border_subtle.lerp_to_gamma(p.accent, t);
    let x = r.lanes.left() - 0.5;
    ui.painter().vline(
        x,
        egui::Rangef::new(r.ruler.top(), r.lanes.bottom()),
        Stroke::new(1.0, color),
    );
}

/// A track's name: its own, else its kind and number within its band
/// ("Main", "Video 1", "Audio 2"), counted from the main track up and from
/// the top audio track down.
pub fn track_name(doc: &Document, seq: SeqId, track: TrackId) -> String {
    let Some(sequence) = doc.sequence(seq) else {
        return "Track".to_owned();
    };
    let Some((slot, t)) = sequence.find_track(track) else {
        return "Track".to_owned();
    };
    if let Some(name) = &t.header.name {
        return name.clone();
    }
    if slot.is_main() {
        return "Main".to_owned();
    }
    let kind = match t.kind {
        TrackKind::Video => "Video",
        TrackKind::Text => "Text",
        TrackKind::Sticker => "Sticker",
        TrackKind::Effect => "Effect",
        TrackKind::Adjustment => "Adjustment",
        TrackKind::Audio => "Audio",
    };
    let number = if t.kind.is_audio() {
        sequence
            .audio_tracks()
            .iter()
            .position(|x| x.id == track)
            .map(|i| i + 1)
    } else {
        // Index 0 is the main track.
        sequence.visual_tracks().iter().position(|x| x.id == track)
    };
    number.map_or_else(|| kind.to_owned(), |n| format!("{kind} {n}"))
}

/// A clip's colour, by what it plays (ui-concept §12.3; AU-001: audio is
/// coloured by origin, so extracted sound reads apart from imported files).
fn clip_color(doc: &Document, content: &ClipContent) -> Color32 {
    match content {
        ClipContent::Media {
            kind: MediaKind::Video,
            ..
        }
        | ClipContent::Image { .. } => surface::CLIP_VIDEO,
        ClipContent::Media {
            kind: MediaKind::Audio,
            asset,
            ..
        } => {
            let extracted = doc
                .library()
                .get(*asset)
                .is_some_and(|m| matches!(m.source, MediaSource::AudioOf { .. }));
            if extracted {
                surface::CLIP_AUDIO_EXTRACTED
            } else {
                surface::CLIP_AUDIO
            }
        }
        ClipContent::Text(_) => surface::CLIP_TEXT,
        ClipContent::Generated { kind } => match kind {
            GeneratedKind::Sticker => surface::CLIP_ELEMENT,
            GeneratedKind::Effect => surface::CLIP_EFFECT,
            GeneratedKind::Adjustment => surface::CLIP_ADJUST,
        },
    }
}

/// The name shown on a clip: its own name, else the first line of a text
/// clip ("(empty text)" when it has none, TX-002), else the media's name.
pub fn clip_label(doc: &Document, content: &ClipContent, name: Option<&str>) -> String {
    if let Some(n) = name {
        return n.to_owned();
    }
    if let ClipContent::Text(data) = content {
        return text_label(data.content.as_str());
    }
    match content.asset().and_then(|a| doc.library().get(a)) {
        Some(item) => media_name(item),
        None => match content {
            ClipContent::Generated { kind } => format!("{kind:?}"),
            _ => "Missing media".to_owned(),
        },
    }
}

/// A text clip's timeline label: its first non-blank line, trimmed.
pub fn text_label(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map_or_else(|| "(empty text)".to_owned(), str::to_owned)
}

/// A library item's display name: its own name, else the file name.
pub fn media_name(item: &freecut_timeline::model::MediaItem) -> String {
    item.display_name.clone().unwrap_or_else(|| {
        std::path::Path::new(&item.path.absolute)
            .file_name()
            .map_or_else(
                || item.path.absolute.clone(),
                |n| n.to_string_lossy().into_owned(),
            )
    })
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Modifiers};
    use freecut_timeline::model::{ProjectSettings, TextClipData};
    use std::collections::BTreeSet;

    use super::*;
    use crate::ui::drag::tests::{Frames, button, gain, modifiers, project_with_a_sound};

    /// Where `clip`'s volume line crosses sequence time `at`, in a timeline
    /// drawn at `rect` with `view`.
    pub(crate) fn line_point(
        project: &Project,
        view: &TimelineView,
        rect: Rect,
        clip: ClipId,
        at: Tick,
    ) -> Pos2 {
        let doc = project.doc();
        let seq = doc.sequence(doc.root()).unwrap();
        let m = Look::default().metrics;
        let header_w =
            layout::clamp_header_width(view.header_w.unwrap_or(m.tl_header), rect.width(), &m);
        let bands = layout::timeline_layout(rect, &m, header_w);
        let rows = rows(seq, bands.lanes.top() - view.track_scroll, view, &m);
        let (_, track) = seq.find_clip(clip).unwrap();
        let row = rows.iter().find(|r| r.track == track.id).unwrap();
        let c = doc.clip(clip).unwrap();
        let sc = Scale::of(view, bands.lanes.left());
        let parts = clip_parts(c, lane_rect(bands.lanes, row), sc, &m);
        let (top, bottom) = line_span(parts.band.expect("a clip with sound"));
        let audio = c.audio.active().unwrap();
        let db = clip_keys::envelope_db_at(c, audio, at);
        Pos2::new(sc.x(at), clip_keys::y_of_db(db, top, bottom))
    }

    /// No filmstrip frames in these tests.
    struct NoFrames;

    impl crate::thumbs::FrameDecoder for NoFrames {
        fn decode(
            &mut self,
            _: &crate::thumbs::FilmRequest,
        ) -> Result<crate::thumbs::FrameImage, String> {
            Err("no frames in tests".into())
        }
    }

    /// Draws the timeline for one frame; returns where it was drawn.
    pub(crate) fn timeline_frame(
        frames: &mut Frames,
        project: &mut Project,
        view: &mut TimelineView,
        waves: &Waveforms,
        events: Vec<Event>,
    ) -> (Rect, TimelineOutput) {
        let mut rect = Rect::NOTHING;
        let mut out = TimelineOutput::default();
        let mut films = Filmstrips::with_decoder(
            Box::new(|| Box::new(NoFrames) as Box<dyn crate::thumbs::FrameDecoder>),
            crate::thumbs::BUDGET,
            || {},
        );
        frames.run(events, |ui| {
            rect = ui.available_rect_before_wrap();
            out = show(ui, project, view, waves, &mut films, false);
        });
        let down = frames.ctx.input(|i| i.pointer.any_down());
        view.settle(Some(project), down);
        (rect, out)
    }

    fn secs(n: f64) -> Tick {
        Tick::from_raw((n * TICKS_PER_SECOND as f64).round() as i64)
    }

    fn keys(project: &Project, clip: ClipId) -> Vec<(Tick, GainDb)> {
        project
            .doc()
            .clip(clip)
            .unwrap()
            .audio
            .active()
            .unwrap()
            .gain
            .keys()
            .iter()
            .map(|k| (k.at, k.value))
            .collect()
    }

    /// Presses at `from`, drags to `to` in a few frames and releases.
    fn drag(
        frames: &mut Frames,
        project: &mut Project,
        view: &mut TimelineView,
        waves: &Waveforms,
        from: Pos2,
        to: Pos2,
    ) {
        let f = |frames: &mut Frames, project: &mut Project, view: &mut TimelineView, e| {
            timeline_frame(frames, project, view, waves, e);
        };
        f(
            frames,
            project,
            view,
            vec![Event::PointerMoved(from), button(from, true)],
        );
        for step in 1..=4 {
            let p = from + (to - from) * (step as f32 / 4.0);
            f(frames, project, view, vec![Event::PointerMoved(p)]);
        }
        f(
            frames,
            project,
            view,
            vec![Event::PointerMoved(to), button(to, false)],
        );
        f(frames, project, view, Vec::new());
    }

    #[test]
    fn dragging_the_volume_line_and_its_dots_is_one_undo_entry_each() {
        let (mut project, clip) = project_with_a_sound();
        let mut view = TimelineView::default();
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let entries = project.editor.history().len();

        // The line without keys: the static volume, one entry.
        let at = line_point(&project, &view, rect, clip, secs(1.0));
        drag(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            at,
            at + Vec2::new(0.0, 8.0),
        );
        let lowered = gain(&project, clip);
        assert!(lowered < GainDb::ZERO, "{lowered:?}");
        assert!(!project.editor.gesture_open(), "the drag ended");
        assert_eq!(project.editor.history().len(), entries + 1);
        project.undo().unwrap();
        assert_eq!(gain(&project, clip), GainDb::ZERO);

        // Alt/Option-click on the line adds a key on a frame, selected.
        let at = line_point(&project, &view, rect, clip, secs(2.0));
        let alt = |p: Pos2, down: bool| {
            vec![
                modifiers(Modifiers::ALT),
                Event::PointerMoved(p),
                Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: down,
                    modifiers: Modifiers::ALT,
                },
            ]
        };
        timeline_frame(&mut frames, &mut project, &mut view, &waves, alt(at, true));
        timeline_frame(&mut frames, &mut project, &mut view, &waves, alt(at, false));
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![modifiers(Modifiers::NONE)],
        );
        let k = keys(&project, clip);
        assert_eq!(k.len(), 1, "{k:?}");
        let rate = project.doc().settings().frame_rate;
        assert!(rate.is_on_grid(k[0].0), "on a frame");
        assert_eq!(k[0].1, GainDb::ZERO, "the line's value there");
        assert_eq!(
            view.selected_keys(&project),
            Some(KeySelection {
                clip,
                times: vec![k[0].0]
            })
        );
        assert_eq!(project.selection, [clip], "the clip stays selected");

        // Drag the dot later and lower: one entry, still selected.
        let entries = project.editor.history().len();
        let dot = line_point(&project, &view, rect, clip, k[0].0);
        drag(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            dot,
            dot + Vec2::new(30.0, 3.0),
        );
        assert_eq!(project.editor.history().len(), entries + 1);
        let moved = keys(&project, clip);
        assert!(moved[0].0 > k[0].0, "later: {moved:?}");
        assert!(moved[0].1 < GainDb::ZERO, "lower: {moved:?}");
        assert_eq!(
            view.selected_keys(&project).map(|s| s.times),
            Some(vec![moved[0].0])
        );

        // A click on the clip away from the line clears the dot selection.
        let dot = line_point(&project, &view, rect, clip, moved[0].0);
        let away = Pos2::new(dot.x - 60.0, dot.y - 14.0);
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::PointerMoved(away), button(away, true)],
        );
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![button(away, false)],
        );
        assert_eq!(view.selected_keys(&project), None);
        assert_eq!(project.selection, [clip]);
    }

    #[test]
    fn text_clips_are_labelled_by_their_first_line() {
        assert_eq!(text_label("Your text"), "Your text");
        assert_eq!(text_label("\n  Title  \nsecond line"), "Title");
        assert_eq!(text_label(""), "(empty text)");
        assert_eq!(text_label(" \n\t"), "(empty text)");
        let doc = Project::new(ProjectSettings::new_default().unwrap())
            .unwrap()
            .doc()
            .clone();
        let content = ClipContent::Text(std::sync::Arc::new(TextClipData::with_content("Hi")));
        assert_eq!(clip_label(&doc, &content, None), "Hi");
        assert_eq!(clip_label(&doc, &content, Some("Named")), "Named");
    }

    #[test]
    fn a_picture_dropped_on_an_empty_timeline_goes_to_the_main_track() {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let doc = project.doc().clone();
        let seq = doc.sequence(doc.root()).unwrap();
        let main = seq.main_track().unwrap().id;
        assert_eq!(drop_target(seq, None, true), InsertTarget::Track(main));
        // Audio still gets its own track.
        assert_eq!(drop_target(seq, None, false), InsertTarget::NewTrack);

        // Once anything is on the timeline, the pointer decides.
        let text = NewClip::text(TextClipData::default(), Tick::from_secs(1).unwrap());
        project
            .apply(
                &EditRequest::Insert(InsertReq {
                    seq: doc.root(),
                    target: InsertTarget::NewTrack,
                    at: Tick::ZERO,
                    clips: vec![text],
                }),
                Coalesce::Never,
            )
            .unwrap();
        let doc = project.doc().clone();
        let seq = doc.sequence(doc.root()).unwrap();
        assert_eq!(drop_target(seq, None, true), InsertTarget::NewTrack);
        assert_eq!(
            drop_target(seq, Some(main), true),
            InsertTarget::Track(main)
        );
    }

    /// ui-concept §12.3: clip fills come from the editing-surface tokens,
    /// not from colours of the timeline's own.
    #[test]
    fn clips_are_coloured_by_the_surface_tokens() {
        let (project, clip) = project_with_a_sound();
        let doc = project.doc();
        let audio = &doc.clip(clip).unwrap().content;
        assert_eq!(clip_color(doc, audio), surface::CLIP_AUDIO);
        let text = ClipContent::Text(std::sync::Arc::new(TextClipData::default()));
        assert_eq!(clip_color(doc, &text), surface::CLIP_TEXT);
        let sticker = ClipContent::Generated {
            kind: GeneratedKind::Sticker,
        };
        assert_eq!(clip_color(doc, &sticker), surface::CLIP_ELEMENT);
    }

    /// Presses and releases the primary button at `at`.
    fn click(
        frames: &mut Frames,
        project: &mut Project,
        view: &mut TimelineView,
        waves: &Waveforms,
        at: Pos2,
    ) {
        timeline_frame(
            frames,
            project,
            view,
            waves,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        timeline_frame(frames, project, view, waves, vec![button(at, false)]);
        timeline_frame(frames, project, view, waves, Vec::new());
    }

    /// The header's icon buttons keep their behaviour: a lock is one undo
    /// entry; the toolbar's zoom is view state with no entry.
    #[test]
    fn header_and_toolbar_buttons_keep_their_commands() {
        let (mut project, _) = project_with_a_sound();
        let mut view = TimelineView::default();
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let m = Look::default().metrics;
        let bands = layout::timeline_layout(rect, &m, m.tl_header);
        let doc = project.doc().clone();
        let seq = doc.sequence(doc.root()).unwrap();
        let rows = rows(seq, bands.lanes.top(), &view, &m);
        let audio = rows.iter().find(|r| r.kind == TrackKind::Audio).unwrap();
        let locked = |p: &Project| {
            p.doc()
                .sequence(p.doc().root())
                .unwrap()
                .find_track(audio.track)
                .unwrap()
                .1
                .header
                .flags
                .locked
        };
        // An audio lane is Medium: mute, solo and lock on the second line.
        let lock = Pos2::new(
            rect.left() + 6.0 + 2.0 * (HEADER_CHIP + 2.0) + HEADER_CHIP / 2.0,
            audio.top + 6.0 + HEADER_CHIP + 2.0 + HEADER_CHIP / 2.0,
        );
        let entries = project.editor.history().len();
        click(&mut frames, &mut project, &mut view, &waves, lock);
        assert!(locked(&project));
        assert_eq!(project.editor.history().len(), entries + 1);
        project.undo().unwrap();
        assert!(!locked(&project));

        // Zoom in: the second button from the toolbar's right end, after
        // the `⋯` menu and its rule when something folded.
        let chip = m.icon_chip;
        let fold = timeline_toolbar::fold_toolbar(
            bands.toolbar.width(),
            timeline_toolbar::Widths { chip, count: 0.0 },
        );
        let more = if fold.folded() {
            chip + 4.0 + 1.0 + 4.0
        } else {
            0.0
        };
        let zoom_in = Pos2::new(
            bands.toolbar.right() - 8.0 - more - chip - 4.0 - chip / 2.0,
            bands.toolbar.center().y,
        );
        let entries = project.editor.history().len();
        click(&mut frames, &mut project, &mut view, &waves, zoom_in);
        assert_eq!(view.px_per_sec, 60.0 * layout::ZOOM_STEP);
        assert_eq!(project.editor.history().len(), entries);
    }

    /// Ctrl / Cmd + wheel zooms about the pointer; a plain wheel scrolls
    /// the tracks only when they overflow.
    #[test]
    fn the_wheel_zooms_about_the_pointer() {
        let (mut project, _) = project_with_a_sound();
        let mut view = TimelineView {
            scroll_secs: 1.0,
            ..TimelineView::default()
        };
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let m = Look::default().metrics;
        let bands = layout::timeline_layout(rect, &m, m.tl_header);
        let at = Pos2::new(bands.lanes.left() + 200.0, bands.lanes.top() + 20.0);
        let before = Scale::of(&view, bands.lanes.left()).tick(at.x);
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::PointerMoved(at), Event::Zoom(1.5)],
        );
        assert_eq!(view.px_per_sec, 90.0);
        let after = Scale::of(&view, bands.lanes.left()).tick(at.x);
        let off = (after.as_secs_f64() - before.as_secs_f64()).abs() * 90.0;
        assert!(off <= 0.5, "{off} pt");
        // Nothing overflows: the tracks stay put.
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -40.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            }],
        );
        assert_eq!(view.track_scroll, 0.0);
    }

    /// The middle of `clip`'s lane at sequence time `at`, in a timeline
    /// drawn at `rect` with `view`.
    pub(crate) fn clip_point(
        project: &Project,
        view: &TimelineView,
        rect: Rect,
        clip: ClipId,
        at: Tick,
    ) -> Pos2 {
        let doc = project.doc();
        let seq = doc.sequence(doc.root()).unwrap();
        let m = Look::default().metrics;
        let header_w =
            layout::clamp_header_width(view.header_w.unwrap_or(m.tl_header), rect.width(), &m);
        let bands = layout::timeline_layout(rect, &m, header_w);
        let rows = rows(seq, bands.lanes.top() - view.track_scroll, view, &m);
        let (_, track) = seq.find_clip(clip).unwrap();
        let row = rows.iter().find(|r| r.track == track.id).unwrap();
        let sc = Scale::of(view, bands.lanes.left());
        let lane = lane_rect(bands.lanes, row);
        // Below the name strip, away from the volume line's band.
        Pos2::new(
            sc.x(at),
            lane.top() + layout::CLIP_INSET + m.clip_strip + 4.0,
        )
    }

    /// Where timeline marker flags stand: the x of `at` and the ruler's
    /// flag height.
    pub(crate) fn ruler_point(view: &TimelineView, rect: Rect, at: Tick) -> Pos2 {
        let m = Look::default().metrics;
        let header_w =
            layout::clamp_header_width(view.header_w.unwrap_or(m.tl_header), rect.width(), &m);
        let bands = layout::timeline_layout(rect, &m, header_w);
        let sc = Scale::of(view, bands.lanes.left());
        Pos2::new(sc.x(at) + 3.0, bands.ruler.bottom() - 8.0)
    }

    /// A click at `at` holding `mods`.
    fn click_with(
        frames: &mut Frames,
        project: &mut Project,
        view: &mut TimelineView,
        waves: &Waveforms,
        at: Pos2,
        mods: Modifiers,
    ) {
        let press = |down: bool| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: down,
            modifiers: mods,
        };
        timeline_frame(
            frames,
            project,
            view,
            waves,
            vec![modifiers(mods), Event::PointerMoved(at), press(true)],
        );
        timeline_frame(frames, project, view, waves, vec![press(false)]);
        timeline_frame(
            frames,
            project,
            view,
            waves,
            vec![modifiers(Modifiers::NONE)],
        );
    }

    fn span(project: &Project, clip: ClipId) -> (Tick, Tick) {
        let c = project.doc().clip(clip).unwrap();
        (c.start(), c.end())
    }

    #[test]
    fn the_blade_cuts_where_clicked_and_shift_cuts_every_track() {
        use crate::timeline_commands::tests::talking_head;
        let (mut project, [a, _, title]) = talking_head();
        let mut view = TimelineView {
            snapping: false,
            ..TimelineView::default()
        };
        view.tool.set(TimelineTool::Blade);
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let entries = project.editor.history().len();
        let at = clip_point(&project, &view, rect, a, secs(2.0));
        click(&mut frames, &mut project, &mut view, &waves, at);
        assert_eq!(project.editor.history().len(), entries + 1);
        assert_eq!(span(&project, a), (Tick::ZERO, secs(2.0)));
        assert_eq!(span(&project, title), (secs(1.0), secs(3.0)), "one clip");
        assert_eq!(project.playhead, Tick::ZERO, "a blade click doesn't seek");
        project.undo().unwrap();

        // Shift: every unlocked track at that time.
        click_with(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            at,
            Modifiers::SHIFT,
        );
        assert_eq!(span(&project, a), (Tick::ZERO, secs(2.0)));
        assert_eq!(span(&project, title), (secs(1.0), secs(2.0)));
        assert_eq!(project.editor.history().len(), entries + 1);

        // On an edge or a gap: nothing.
        let edge = clip_point(&project, &view, rect, a, secs(2.0));
        let entries = project.editor.history().len();
        click(&mut frames, &mut project, &mut view, &waves, edge);
        let gap = Pos2::new(edge.x + 600.0, edge.y);
        click(&mut frames, &mut project, &mut view, &waves, gap);
        assert_eq!(project.editor.history().len(), entries);
    }

    #[test]
    fn the_hand_pans_and_never_selects() {
        use crate::timeline_commands::tests::talking_head;
        let (mut project, [a, _, _]) = talking_head();
        let mut view = TimelineView {
            scroll_secs: 2.0,
            ..TimelineView::default()
        };
        view.tool.set(TimelineTool::Hand);
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let at = clip_point(&project, &view, rect, a, secs(3.0));
        click(&mut frames, &mut project, &mut view, &waves, at);
        assert!(project.selection.is_empty(), "clicks are inert");
        drag(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            at,
            at + Vec2::new(60.0, 0.0),
        );
        assert!(
            (view.scroll_secs - 1.0).abs() < 1e-6,
            "{}",
            view.scroll_secs
        );
        assert!(project.selection.is_empty());
    }

    #[test]
    fn a_linked_clip_brings_its_partner_unless_alt_is_held() {
        use crate::timeline_commands::tests::talking_head;
        let (mut project, [a, _, _]) = talking_head();
        project.selection = vec![a];
        let req = EditRequest::DetachAudio {
            seq: project.doc().root(),
            clips: vec![a],
        };
        project.apply(&req, Coalesce::Never).unwrap();
        let sound = project.selection[0];
        project.selection.clear();
        let mut view = TimelineView::default();
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let video = clip_point(&project, &view, rect, a, secs(1.5));
        click(&mut frames, &mut project, &mut view, &waves, video);
        assert_eq!(project.selection, vec![a, sound]);
        let audio = clip_point(&project, &view, rect, sound, secs(1.5));
        click_with(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            audio,
            Modifiers::ALT,
        );
        assert_eq!(project.selection, vec![sound], "Alt picks one");
        view.linked_selection = false;
        click(&mut frames, &mut project, &mut view, &waves, video);
        assert_eq!(project.selection, vec![a], "TL-030's rule with it off");

        // Dragging the video with the option on moves its sound too.
        view.linked_selection = true;
        click(&mut frames, &mut project, &mut view, &waves, video);
        let entries = project.editor.history().len();
        drag(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            audio,
            audio + Vec2::new(30.0, 0.0),
        );
        assert_eq!(project.editor.history().len(), entries + 1);
        let offsets = freecut_timeline::edit::sync_offsets(project.doc(), project.doc().root());
        assert!(!offsets.contains_key(&sound), "still in sync: {offsets:?}");
    }

    #[test]
    fn ruler_flags_select_move_and_open_the_editor() {
        use crate::timeline_commands::tests::talking_head;
        let (mut project, _) = talking_head();
        project.playhead = secs(2.0);
        let m = project.add_marker(None).unwrap();
        project.markers.clear();
        project.playhead = Tick::ZERO;
        let mut view = TimelineView {
            snapping: false,
            ..TimelineView::default()
        };
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let flag = ruler_point(&view, rect, secs(2.0));
        click(&mut frames, &mut project, &mut view, &waves, flag);
        assert_eq!(project.markers, BTreeSet::from([m]), "selected");
        assert_eq!(project.playhead, Tick::ZERO, "a flag click doesn't seek");

        // Dragging it 1 s later: one MoveMarker.
        let entries = project.editor.history().len();
        drag(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            flag,
            flag + Vec2::new(60.0, 0.0),
        );
        assert_eq!(project.editor.history().len(), entries + 1);
        let start = |p: &Project| {
            p.doc()
                .sequence(p.doc().root())
                .unwrap()
                .markers
                .get(m.id())
                .unwrap()
                .at
                .start()
        };
        assert_eq!(start(&project), secs(3.0));

        // A double-click opens the editor; Esc drops it unchanged. (Wait
        // first, so the clicks before don't make it a triple click.)
        for _ in 0..40 {
            timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        }
        let flag = ruler_point(&view, rect, secs(3.0));
        click(&mut frames, &mut project, &mut view, &waves, flag);
        click(&mut frames, &mut project, &mut view, &waves, flag);
        assert_eq!(view.marker_edit.as_ref().map(|e| e.marker), Some(m));
        let entries = project.editor.history().len();
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![crate::ui::drag::tests::key(
                egui::Key::Escape,
                Modifiers::NONE,
            )],
        );
        assert!(view.marker_edit.is_none());
        assert_eq!(project.editor.history().len(), entries);
    }

    /// TL-014: the panel reports the frame under the pointer over the
    /// ruler and lanes (the last frame past the end), nothing outside
    /// them, and nothing while a clip is dragged; the blade reports its cut
    /// line whatever the skim mode.
    #[test]
    fn skim_reads_the_frame_under_the_pointer_but_not_during_a_drag() {
        use crate::timeline_commands::tests::talking_head;
        let (mut project, [a, _, _]) = talking_head();
        let mut view = TimelineView::default();
        let waves = Waveforms::new(|| {});
        let mut frames = Frames::new();
        let (rect, _) = timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let at = clip_point(&project, &view, rect, a, secs(1.0));
        let (_, out) = timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::PointerMoved(at)],
        );
        assert_eq!(out.pointer_time, Some(secs(1.0)));
        assert!(!out.dragging);
        assert_eq!(out.blade_time, None, "the Select tool has no cut line");
        // Past the sequence's end: its last frame.
        let far = Pos2::new(rect.right() - 20.0, at.y);
        let (_, out) = timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::PointerMoved(far)],
        );
        let end = project
            .doc()
            .sequence(project.doc().root())
            .unwrap()
            .duration();
        assert_eq!(out.pointer_time, Some(end));
        // Over the track headers: not the lanes.
        let header = Pos2::new(rect.left() + 10.0, at.y);
        let (_, out) = timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::PointerMoved(header)],
        );
        assert_eq!(out.pointer_time, None);
        // A clip drag.
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        let mut during = Vec::new();
        for step in 1..=3 {
            let p = at + Vec2::new(10.0 * step as f32, 0.0);
            let (_, out) = timeline_frame(
                &mut frames,
                &mut project,
                &mut view,
                &waves,
                vec![Event::PointerMoved(p)],
            );
            during.push((out.dragging, out.pointer_time));
        }
        assert!(during.iter().any(|(d, _)| *d), "the drag started");
        assert!(
            during.iter().filter(|(d, _)| *d).all(|(_, t)| t.is_none()),
            "{during:?}"
        );
        let end_at = at + Vec2::new(30.0, 0.0);
        timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![button(end_at, false)],
        );
        // The blade's cut line, with skim off.
        view.tool.set(TimelineTool::Blade);
        assert!(!view.skim.is_on());
        let b = clip_point(&project, &view, rect, a, secs(1.0));
        timeline_frame(&mut frames, &mut project, &mut view, &waves, Vec::new());
        let (_, out) = timeline_frame(
            &mut frames,
            &mut project,
            &mut view,
            &waves,
            vec![Event::PointerMoved(b)],
        );
        assert!(out.blade_time.is_some());
    }
}

//! The Stage panel (EX-001..EX-004, EX-006, EX-019, AU-010; ui-concept
//! §9, rev. 2): a panel header with the output readout and the zoom
//! select, the canvas on its surround, and the transport bar with play
//! exactly centred, the scrubbable timecode field, the speaker (preview
//! mute, with the master volume beside it), the preview resolution and
//! *Fit*.
//!
//! The view (zoom and pan) is view state, never an edit: `Ctrl` / `Cmd` +
//! wheel or a pinch zooms about the pointer, the wheel pans while zoomed,
//! a middle-button drag pans, and a double-click on the surround fits
//! again. Every number of it is a pure function ([`fit_scale`],
//! [`canvas_rect`], [`zoom_about`], [`clamp_pan`], [`transport_layout`]).
//!
//! On the canvas (TR-007 subset): a click selects the topmost visual clip
//! under the pointer (or clears the selection on empty canvas), a drag
//! moves it (Shift keeps one axis, Esc cancels), the selected clips get a
//! thin outline, and a double-click on a text clip edits its content in the
//! inspector. A single selected text clip shows its text handles
//! (`super::stage`): side handles that set the box width (TX-005; the
//! opposite edge stays, Alt keeps the centre) and, with a tailed bubble,
//! the tail-tip handle (TX-055). Each handle drag is one undo entry and Esc
//! cancels it. Handles for scale and rotation come later. While playing,
//! nothing on the canvas reacts and no outline is drawn.

use eframe::egui::{
    self, Color32, DragValue, Key, PointerButton, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2,
};
use freecut_timeline::edit::{EditRequest, TextLayoutEdit, TextStyleEdit, TransformEdit};
use freecut_timeline::glam::DVec2;
use freecut_timeline::history::{Coalesce, GestureId};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    CanvasPos, ClipContent, Document, GainDb, MasterAudio, RefPx, TailTip, TextClipData,
};
use freecut_timeline::plan::ref_px_scale;
use freecut_timeline::time::Tick;

use crate::clip_sound::{format_db, gain_db, gain_from_db, parse_db};
use crate::preview::LayerBox;
use crate::session::Project;
use crate::theme::{self, ThemeKind, icons, surface};
use crate::ui::canvas::{
    CanvasView, canvas_to_screen, corners, hit_test, moved_position, screen_to_canvas,
};
use crate::ui::drag::DragGesture;
use crate::ui::props::{self, Edit};
use crate::ui::stage::{self, Handle, TextHandles, WidthDrag};
use crate::widgets::icon_button::IconButton;
use crate::widgets::panel::pane_header;
use crate::widgets::text_then_icon;
use crate::widgets::timecode::timecode_field;

/// How near (pt on screen) a press must be to grab a handle.
const HANDLE_REACH: f32 = 10.0;
/// Zoom limits, as displayed percentages of the canvas's own pixels
/// (§9.1: 10 % to 800 %).
const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 8.0;
/// How much of the canvas always stays inside the surround while panned.
const KEEP_VISIBLE: f32 = 64.0;
/// Space between the transport's three groups.
const GROUP_GAP: f32 = 12.0;

/// How the Stage shows the canvas: fitted to the panel (the default) or
/// at a zoom with a pan. View state: never saved, never an undo entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageView {
    /// Follows the panel's size.
    pub fitted: bool,
    /// Points per canvas pixel while not fitted.
    pub zoom: f32,
    /// The canvas centre's offset from the surround's centre, pt.
    pub pan: Vec2,
}

impl Default for StageView {
    fn default() -> StageView {
        StageView {
            fitted: true,
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl StageView {
    /// Points per canvas pixel in a surround of `room` for a canvas of
    /// `out` pixels.
    pub fn scale(&self, room: Vec2, out: (u32, u32)) -> f32 {
        if self.fitted {
            fit_scale(room, out)
        } else {
            self.zoom
        }
    }
}

/// The scale (pt per canvas pixel) that fits a canvas of `out` pixels in
/// `room`.
pub fn fit_scale(room: Vec2, out: (u32, u32)) -> f32 {
    let (w, h) = (out.0.max(1) as f32, out.1.max(1) as f32);
    (room.x / w).min(room.y / h).max(1e-4)
}

/// Where the canvas is drawn: centred and fitted in `room`, or at the
/// view's zoom with its pan.
pub fn canvas_rect(room: Rect, out: (u32, u32), view: &StageView) -> Rect {
    let scale = view.scale(room.size(), out);
    let size = Vec2::new(out.0.max(1) as f32, out.1.max(1) as f32) * scale;
    let centre = if view.fitted {
        room.center()
    } else {
        room.center() + view.pan
    };
    Rect::from_center_size(centre, size)
}

/// The zoom factor of one wheel event of `dy` pt (§9.1): `exp(−dy / 400)`,
/// at most ×2 or ÷2.
pub fn wheel_factor(dy: f32) -> f32 {
    (-dy / 400.0).exp().clamp(0.5, 2.0)
}

/// `view` zoomed by `factor` (clamped to ×2 / ÷2 per event) about the
/// screen point `pointer`: the canvas point under the pointer stays put.
/// The total zoom stays within 10 %–800 % of the canvas's pixels at
/// `ppp` physical pixels per point.
pub fn zoom_about(
    view: &StageView,
    room: Rect,
    out: (u32, u32),
    pointer: Pos2,
    factor: f32,
    ppp: f32,
) -> StageView {
    let old = canvas_rect(room, out, view);
    let scale = view.scale(room.size(), out);
    let ppp = ppp.max(0.1);
    let (lo, hi) = (ZOOM_MIN / ppp, ZOOM_MAX / ppp);
    let zoom = (scale * factor.clamp(0.5, 2.0)).clamp(lo.min(scale), hi.max(scale));
    let k = zoom / scale;
    let centre = pointer - (pointer - old.center()) * k;
    let zoomed = StageView {
        fitted: false,
        zoom,
        pan: centre - room.center(),
    };
    StageView {
        pan: clamp_pan(&zoomed, room, out),
        ..zoomed
    }
}

/// The view's pan, kept so that at least 64 pt of the canvas (or all of
/// it, if smaller) stays inside `room`.
pub fn clamp_pan(view: &StageView, room: Rect, out: (u32, u32)) -> Vec2 {
    if view.fitted {
        return Vec2::ZERO;
    }
    let size = Vec2::new(out.0.max(1) as f32, out.1.max(1) as f32) * view.zoom;
    let axis = |pan: f32, half_room: f32, s: f32| {
        let keep = KEEP_VISIBLE.min(s);
        // The centre's offset may reach the room's edge minus what stays
        // visible, plus half the canvas.
        let limit = (half_room - keep + s / 2.0).max(0.0);
        pan.clamp(-limit, limit)
    };
    Vec2::new(
        axis(view.pan.x, room.width() / 2.0, size.x),
        axis(view.pan.y, room.height() / 2.0, size.y),
    )
}

/// The transport's three groups in `bar` (§9.1): the centre group of
/// `centre_w` sits exactly on the bar's centre whenever the side groups of
/// `left_w` and `right_w` fit beside it; otherwise it moves just enough to
/// clear them. Returns (left, centre, right).
pub fn transport_layout(bar: Rect, left_w: f32, centre_w: f32, right_w: f32) -> (Rect, Rect, Rect) {
    let ideal = bar.center().x - centre_w / 2.0;
    let earliest = bar.left() + left_w + GROUP_GAP;
    let latest = bar.right() - right_w - GROUP_GAP - centre_w;
    let x = if earliest > latest {
        earliest
    } else {
        ideal.clamp(earliest, latest)
    };
    let centre = Rect::from_min_max(
        Pos2::new(x, bar.top()),
        Pos2::new(x + centre_w, bar.bottom()),
    );
    let left = Rect::from_min_max(
        bar.min,
        Pos2::new((x - GROUP_GAP).max(bar.left()), bar.bottom()),
    );
    let right = Rect::from_min_max(
        Pos2::new((centre.right() + GROUP_GAP).min(bar.right()), bar.top()),
        bar.max,
    );
    (left, centre, right)
}

/// What the transport shows in a narrow Stage: the sequence's length,
/// *Fit*, the speaker's master-volume chevron and the preview resolution
/// fold away in that order (the timecode, the five transport buttons and
/// the speaker always stay; the master volume is also in the Mixer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportFold {
    pub total: bool,
    pub resolution: bool,
    pub fit: bool,
    pub chevron: bool,
}

/// Widths (pt) of the transport's parts, for [`transport_fold`].
#[derive(Debug, Clone, Copy)]
pub struct TransportWidths {
    pub timecode: f32,
    pub total: f32,
    pub centre: f32,
    pub speaker: f32,
    /// The master-volume chevron beside the speaker (no gap between).
    pub chevron: f32,
    pub resolution: f32,
    pub fit: f32,
    /// Between items inside a group.
    pub spacing: f32,
}

impl TransportWidths {
    /// The left and right groups' widths with `fold`.
    pub fn sides(&self, fold: TransportFold) -> (f32, f32) {
        let extra = |on: bool, w: f32| if on { self.spacing + w } else { 0.0 };
        (
            self.timecode + extra(fold.total, self.total),
            self.speaker
                + if fold.chevron { self.chevron } else { 0.0 }
                + extra(fold.resolution, self.resolution)
                + extra(fold.fit, self.fit),
        )
    }
}

/// The parts that fit in a transport `width` pt wide. The length, *Fit*
/// and then the chevron give way so that play can stay centred; the
/// resolution only when the parts don't fit side by side at all.
pub fn transport_fold(width: f32, w: &TransportWidths) -> TransportFold {
    let centred = |fold: TransportFold| {
        let (left, right) = w.sides(fold);
        2.0 * left.max(right) + w.centre + 2.0 * GROUP_GAP <= width
    };
    let fits = |fold: TransportFold| {
        let (left, right) = w.sides(fold);
        left + w.centre + right + 2.0 * GROUP_GAP <= width
    };
    let all = TransportFold {
        total: true,
        resolution: true,
        fit: true,
        chevron: true,
    };
    let steps = [
        all,
        TransportFold {
            total: false,
            ..all
        },
        TransportFold {
            total: false,
            fit: false,
            ..all
        },
        TransportFold {
            total: false,
            fit: false,
            chevron: false,
            ..all
        },
    ];
    steps
        .into_iter()
        .find(|f| centred(*f))
        .or_else(|| steps.into_iter().find(|f| fits(*f)))
        .unwrap_or(TransportFold {
            total: false,
            resolution: false,
            fit: false,
            chevron: false,
        })
}

/// Which of go-to-start, previous frame, play, next frame and go-to-end
/// can act with the playhead on frame `frame` of a sequence whose end is
/// frame `last`: nothing goes before the first frame or past the end.
pub fn transport_enabled(frame: i64, last: i64) -> [bool; 5] {
    let start = frame > 0;
    let end = frame < last;
    [start, start, last > 0, end, end]
}

/// Preview resolution (EX-006): what the Stage renders at. Changes only
/// the preview; exports always render at full quality.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PreviewResolution {
    /// The smaller of the canvas and the Stage's pixels (recommended).
    #[default]
    Auto,
    Full,
    Half,
    Quarter,
}

impl PreviewResolution {
    pub const ALL: [PreviewResolution; 4] = [
        PreviewResolution::Auto,
        PreviewResolution::Full,
        PreviewResolution::Half,
        PreviewResolution::Quarter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PreviewResolution::Auto => "Auto",
            PreviewResolution::Full => "Full",
            PreviewResolution::Half => "Half",
            PreviewResolution::Quarter => "Quarter",
        }
    }

    /// The dim detail beside the label.
    pub fn detail(self) -> &'static str {
        match self {
            PreviewResolution::Auto => "recommended",
            PreviewResolution::Full => "1:1",
            PreviewResolution::Half => "1:2",
            PreviewResolution::Quarter => "1:4",
        }
    }

    /// The largest frame to render for a canvas of `out` pixels shown on
    /// `screen` physical pixels (the preview never renders above the
    /// canvas size).
    pub fn wanted(self, out: (u32, u32), screen: (u32, u32)) -> (u32, u32) {
        let scaled = |d: u32| match self {
            PreviewResolution::Auto | PreviewResolution::Full => d,
            PreviewResolution::Half => d / 2,
            PreviewResolution::Quarter => d / 4,
        };
        let (w, h) = match self {
            PreviewResolution::Auto => (screen.0.min(out.0), screen.1.min(out.1)),
            _ => (scaled(out.0), scaled(out.1)),
        };
        (w.max(16), h.max(16))
    }
}

/// A drag on the canvas in progress.
#[derive(Debug)]
struct CanvasDrag {
    clip: ClipId,
    /// Where the press was, in canvas pixels.
    start: DVec2,
    /// The clip's position when the drag began.
    origin: CanvasPos,
    gesture: GestureId,
    what: Dragging,
}

/// What a canvas drag moves.
#[derive(Debug)]
enum Dragging {
    /// The clip (TR-007): the position last committed.
    Move { last: CanvasPos },
    /// A side handle (TX-005), on the box as it was when the drag began.
    /// `grab` is the handle's offset from the press (canvas px): the handle
    /// follows the pointer without jumping to it.
    Width {
        handle: Handle,
        start: LayerBox,
        grab: DVec2,
        last: Option<WidthDrag>,
    },
    /// The bubble's tail tip (TX-055).
    Tail {
        start: LayerBox,
        size: RefPx,
        grab: DVec2,
        last: Option<TailTip>,
    },
}

/// Player state that outlives a frame.
#[derive(Debug, Default)]
pub struct PlayerState {
    drag: Option<CanvasDrag>,
    /// The Stage's zoom and pan.
    pub view: StageView,
    /// What the preview renders at (EX-006).
    pub resolution: PreviewResolution,
}

impl PlayerState {
    /// Ends a canvas drag whose release was never seen (the panel went
    /// away). Call once per frame.
    pub fn settle(&mut self, project: Option<&mut Project>, pointer_down: bool) {
        if pointer_down {
            return;
        }
        if let (Some(drag), Some(p)) = (self.drag.take(), project) {
            p.end_gesture(drag.gesture);
        }
    }

    /// Forgets a drag of a project that is no longer open, and fits the
    /// view again.
    pub fn reset(&mut self) {
        self.drag = None;
        self.view = StageView::default();
    }
}

#[derive(Debug, Default)]
pub struct PlayerOutput {
    pub toggle_play: bool,
    pub step: i64,
    pub to_start: bool,
    pub to_end: bool,
    /// The speaker button: flip preview mute.
    pub toggle_mute: bool,
    /// The size the preview should be rendered at, in physical pixels.
    pub wanted: (u32, u32),
    /// Why an edit made here didn't happen.
    pub status: Option<String>,
    /// A text clip was double-clicked: edit its content (TX-002).
    pub edit_text: bool,
    /// The timecode field asked to move the playhead here (not an edit).
    pub seek: Option<Tick>,
    /// Where the canvas was drawn (tests aim at it).
    #[cfg(test)]
    pub canvas: Option<Rect>,
    /// Where the timecode field was drawn (tests aim at it).
    #[cfg(test)]
    pub timecode: Option<Rect>,
}

/// The sound state the player shows.
#[derive(Debug, Clone, Default)]
pub struct SoundStatus {
    pub muted: bool,
    /// Why nothing can be heard (no device), if so.
    pub problem: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    master_drag: &mut DragGesture,
    state: &mut PlayerState,
    view: &CanvasView<'_>,
    frame: Option<&TextureHandle>,
    playing: bool,
    sound: &SoundStatus,
) -> PlayerOutput {
    let mut out = PlayerOutput::default();
    let look = theme::look(ui.ctx());
    let m = look.metrics;
    let settings = *project.doc().settings();
    let out_px = (settings.width, settings.height);
    let readout = format!(
        "{} × {} · {}",
        settings.width, settings.height, settings.frame_rate
    );
    let name = project.name();
    let ppp = ui.ctx().pixels_per_point();
    let card = ui.available_rect_before_wrap();
    let header_h = m.panel_header.min(card.height() / 3.0);
    let transport_h = m.transport.min(card.height() / 3.0);
    let header_rect = Rect::from_min_size(card.min, Vec2::new(card.width(), header_h));
    let room = Rect::from_min_max(
        Pos2::new(card.left(), header_rect.bottom()),
        Pos2::new(
            card.right(),
            (card.bottom() - transport_h).max(header_rect.bottom() + 16.0),
        ),
    );
    let bar = Rect::from_min_max(Pos2::new(card.left(), room.bottom()), card.max);
    // The canvas keeps at least the surround margin around it.
    let inner_room = room.shrink(m.surround_margin);

    // Header: the project's name, the output readout, the zoom select.
    let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(header_rect));
    pane_header(&mut header_ui, &name, Some(&readout), |ui| {
        zoom_select(ui, state, inner_room, out_px, ppp);
    });

    // The surround and the canvas.
    let response = ui.interact(room, ui.id().with("stage-room"), Sense::click_and_drag());
    stage_view_input(ui, &response, state, inner_room, out_px);
    let canvas = canvas_rect(inner_room, out_px, &state.view);
    #[cfg(test)]
    {
        out.canvas = Some(canvas);
    }
    let painter = ui.painter_at(room);
    painter.rect_filled(room, 0.0, surface::PLAYER_SURROUND);
    painter.rect_filled(canvas, 0.0, Color32::BLACK);
    if let Some(tex) = frame {
        painter.image(
            tex.id(),
            canvas,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    // The surround is dark in every theme: its ring is dark's
    // `border.strong` (white 16 %), or High contrast's white.
    let ring = if look.kind == ThemeKind::HighContrast {
        look.palette.border_strong
    } else {
        theme::TOKENS.dark.border_strong
    };
    painter.rect_stroke(
        canvas,
        0.0,
        Stroke::new(1.0, ring),
        egui::StrokeKind::Outside,
    );
    if playing {
        if let Some(drag) = state.drag.take() {
            project.end_gesture(drag.gesture);
        }
    } else {
        canvas_input(ui, &response, project, state, view, canvas, &mut out);
        draw_outlines(&painter, &look, project, view, canvas);
        draw_text_handles(&painter, project, view, canvas);
    }
    if response.double_clicked()
        && let Some(p) = response.interact_pointer_pos()
        && !canvas.contains(p)
    {
        state.view = StageView::default();
    }
    let on_screen = (
        (canvas.width() * ppp).round().max(16.0) as u32,
        (canvas.height() * ppp).round().max(16.0) as u32,
    );
    out.wanted = state.resolution.wanted(out_px, on_screen);

    transport(
        ui,
        bar,
        project,
        master_drag,
        state,
        playing,
        sound,
        &mut out,
    );
    ui.advance_cursor_after_rect(card);
    out
}

/// The zoom select in the header (§9.1): *Fit*, 50 %, 100 %, 200 %,
/// 400 % of the canvas's own pixels.
fn zoom_select(ui: &mut egui::Ui, state: &mut PlayerState, room: Rect, out: (u32, u32), ppp: f32) {
    let current = if state.view.fitted {
        "Fit".to_owned()
    } else {
        format!("{:.0} %", state.view.zoom * ppp * 100.0)
    };
    let button = egui::Button::new(text_then_icon(ui, &current, icons::CARET_DOWN));
    egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        if ui.selectable_label(state.view.fitted, "Fit").clicked() {
            state.view = StageView::default();
        }
        for percent in [50.0_f32, 100.0, 200.0, 400.0] {
            let zoom = percent / 100.0 / ppp.max(0.1);
            let on = !state.view.fitted && (state.view.zoom - zoom).abs() < 1e-4;
            if ui.selectable_label(on, format!("{percent:.0} %")).clicked() {
                let zoomed = StageView {
                    fitted: false,
                    zoom,
                    pan: Vec2::ZERO,
                };
                state.view = StageView {
                    pan: clamp_pan(&zoomed, room, out),
                    ..zoomed
                };
            }
        }
    });
}

/// Zoom and pan over the surround: `Ctrl` / `Cmd` + wheel or a pinch
/// zooms about the pointer, the wheel pans while zoomed, a middle-button
/// drag pans.
fn stage_view_input(
    ui: &egui::Ui,
    response: &egui::Response,
    state: &mut PlayerState,
    room: Rect,
    out: (u32, u32),
) {
    let ppp = ui.ctx().pixels_per_point();
    if response.hovered()
        && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
    {
        let (zoom, pinch, scroll) = ui.input(|i| {
            (
                i.zoom_delta(),
                i.multi_touch().is_some(),
                i.smooth_scroll_delta,
            )
        });
        if (zoom - 1.0).abs() > 1e-4 {
            // egui turns Ctrl / Cmd + wheel into `exp(dy / 200)`; the Stage
            // zooms by `exp(−dy / 400)` per wheel distance (§9.1). A pinch
            // keeps its own proportion.
            let factor = if pinch {
                zoom
            } else {
                wheel_factor(-200.0 * zoom.ln())
            };
            state.view = zoom_about(&state.view, room, out, pointer, factor, ppp);
        } else if !state.view.fitted && scroll != Vec2::ZERO {
            state.view.pan += scroll;
            state.view.pan = clamp_pan(&state.view, room, out);
        }
    }
    if response.dragged_by(PointerButton::Middle) {
        if state.view.fitted {
            state.view = StageView {
                fitted: false,
                zoom: fit_scale(room.size(), out),
                pan: Vec2::ZERO,
            };
        }
        state.view.pan += response.drag_delta();
        state.view.pan = clamp_pan(&state.view, room, out);
    }
}

/// The transport bar (§9.1): the timecode on the left, the five transport
/// buttons exactly centred, the speaker, the preview resolution and *Fit*
/// on the right.
#[allow(clippy::too_many_arguments)]
fn transport(
    ui: &mut egui::Ui,
    bar: Rect,
    project: &mut Project,
    master_drag: &mut DragGesture,
    state: &mut PlayerState,
    playing: bool,
    sound: &SoundStatus,
    out: &mut PlayerOutput,
) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let m = look.metrics;
    ui.painter().hline(
        bar.x_range(),
        bar.top() + 0.5,
        Stroke::new(1.0, p.border_subtle),
    );
    let inner = bar.shrink2(Vec2::new(10.0, 0.0));
    let rate = project.doc().settings().frame_rate;
    let fps = rate.nominal_fps();
    let end = project
        .doc()
        .sequence(project.doc().root())
        .map_or(Tick::ZERO, freecut_timeline::model::Sequence::duration);
    let last = rate.frame_index(end).max(0);
    let frame = rate.frame_index(project.playhead).clamp(0, last);

    // Measure the side groups so the centre group can sit on the centre.
    let small = egui::TextStyle::Small.resolve(ui.style());
    let total = format!("/ {}", crate::widgets::timecode::format_tc(last, fps));
    let total_w = ui
        .painter()
        .layout_no_wrap(total.clone(), small.clone(), p.text_tertiary)
        .size()
        .x;
    let chip = m.icon_chip;
    let widths = TransportWidths {
        timecode: crate::widgets::timecode::SIZE.x,
        total: total_w,
        centre: 5.0 * chip + 4.0 * 2.0,
        speaker: chip,
        chevron: CHEVRON_W,
        resolution: resolution_width(ui),
        fit: chip,
        spacing: 6.0,
    };
    let fold = transport_fold(inner.width(), &widths);
    let (left_w, right_w) = widths.sides(fold);
    let (left, centre, right) = transport_layout(inner, left_w, widths.centre, right_w);
    let group =
        |rect: Rect, layout: egui::Layout| egui::UiBuilder::new().max_rect(rect).layout(layout);

    #[cfg(test)]
    {
        let h = m.control_height;
        out.timecode = Some(Rect::from_min_size(
            Pos2::new(left.left(), left.center().y - h / 2.0),
            Vec2::new(crate::widgets::timecode::SIZE.x, h),
        ));
    }
    // Left: the timecode field and the sequence's length.
    ui.scope_builder(
        group(left, egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(to) = timecode_field(
                ui,
                egui::Id::new("freecut-stage-timecode"),
                frame,
                last,
                fps,
            ) && let Ok(t) = rate.frame_start(to)
            {
                out.seek = Some(t.min(end));
            }
            if fold.total {
                ui.add(egui::Label::new(
                    egui::RichText::new(total)
                        .font(small)
                        .color(p.text_tertiary),
                ));
            }
        },
    );

    // Centre: start, previous, play / pause, next, end.
    let enabled = transport_enabled(frame, last);
    ui.scope_builder(
        group(centre, egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let start_tip = if enabled[0] {
                "Go to start (Home)"
            } else {
                "Go to start (Home): already at the start"
            };
            if IconButton::new(icons::GO_START, start_tip)
                .enabled(enabled[0])
                .show(ui)
                .clicked()
            {
                out.to_start = true;
            }
            let prev_tip = if enabled[1] {
                "Previous frame (←)"
            } else {
                "Previous frame (←): this is the first frame"
            };
            if IconButton::new(icons::CARET_LEFT, prev_tip)
                .enabled(enabled[1])
                .show(ui)
                .clicked()
            {
                out.step = -1;
            }
            let (glyph, tip) = if playing {
                (icons::PAUSE, "Pause (Space)")
            } else {
                (icons::PLAY, "Play (Space)")
            };
            if IconButton::new(glyph, tip)
                .glyph(18.0)
                .accent(true)
                .enabled(enabled[2] || playing)
                .show(ui)
                .clicked()
            {
                out.toggle_play = true;
            }
            let next_tip = if enabled[3] {
                "Next frame (→)"
            } else {
                "Next frame (→): this is the end"
            };
            if IconButton::new(icons::CARET_RIGHT, next_tip)
                .enabled(enabled[3])
                .show(ui)
                .clicked()
            {
                out.step = 1;
            }
            let end_tip = if enabled[4] {
                "Go to end (End)"
            } else {
                "Go to end (End): already at the end"
            };
            if IconButton::new(icons::GO_END, end_tip)
                .enabled(enabled[4])
                .show(ui)
                .clicked()
            {
                out.to_end = true;
            }
        },
    );

    // Right, from the far edge: Fit, the resolution, the speaker.
    ui.scope_builder(
        group(right, egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let fitted = state.view.fitted;
            let fit_tip = if fitted {
                "Fit the canvas to the Stage: it already fits"
            } else {
                "Fit the canvas to the Stage (or double-click the surround)"
            };
            if fold.fit
                && IconButton::new(icons::FIT, fit_tip)
                    .enabled(!fitted)
                    .show(ui)
                    .clicked()
            {
                state.view = StageView::default();
            }
            if fold.resolution {
                resolution_select(ui, state);
            }
            ui.spacing_mut().item_spacing.x = 0.0;
            out.status = speaker(ui, project, master_drag, sound, fold.chevron, out);
        },
    );
}

/// The resolution select's width: its widest label, the caret and the
/// padding, so it doesn't change width with the choice (§13.20).
fn resolution_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let widest = PreviewResolution::ALL
        .iter()
        .map(|r| {
            ui.painter()
                .layout_no_wrap(r.label().to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    widest + 12.0 + 8.0 + 2.0 * ui.spacing().button_padding.x
}

/// Preview resolution (EX-006): Auto, Full, Half, Quarter, with their
/// ratios as dim details and a note that export is unaffected.
fn resolution_select(ui: &mut egui::Ui, state: &mut PlayerState) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let width = resolution_width(ui);
    let label = text_then_icon(ui, state.resolution.label(), icons::CARET_DOWN);
    let button = egui::Button::new(label).min_size(Vec2::new(width, look.metrics.icon_chip));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(180.0);
        for r in PreviewResolution::ALL {
            let on = state.resolution == r;
            let text = egui::text::LayoutJob::default();
            let mut job = text;
            job.append(
                r.label(),
                0.0,
                egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), p.text_primary),
            );
            job.append(
                r.detail(),
                8.0,
                egui::TextFormat::simple(
                    egui::TextStyle::Small.resolve(ui.style()),
                    p.text_tertiary,
                ),
            );
            if ui.selectable_label(on, job).clicked() {
                state.resolution = r;
            }
        }
        ui.separator();
        ui.label(
            egui::RichText::new("Changes only the preview. Exports always render at full quality.")
                .text_style(egui::TextStyle::Small)
                .color(p.text_tertiary),
        );
    });
    response.on_hover_text("Preview resolution: lower plays heavy timelines more smoothly");
}

/// The master-volume chevron's width.
const CHEVRON_W: f32 = 16.0;

/// The speaker (preview mute, EX-019) and, beside it unless a narrow
/// transport folded it (`chevron`), a chevron that opens the master
/// volume (AU-010). Returns why a master volume edit failed.
fn speaker(
    ui: &mut egui::Ui,
    project: &mut Project,
    master_drag: &mut DragGesture,
    sound: &SoundStatus,
    chevron: bool,
    out: &mut PlayerOutput,
) -> Option<String> {
    let mut status = None;
    // Right to left: the chevron first, then the speaker.
    if chevron {
        status = master_chevron(ui, project, master_drag);
    }
    match &sound.problem {
        Some(problem) => {
            IconButton::new(icons::SPEAKER_MUTED, problem.as_str())
                .enabled(false)
                .show(ui);
        }
        None => {
            let (glyph, tip) = if sound.muted {
                (
                    icons::SPEAKER_MUTED,
                    "Unmute preview sound (export is unaffected)",
                )
            } else {
                (icons::SPEAKER, "Mute preview sound (export is unaffected)")
            };
            if IconButton::new(glyph, tip)
                .checked(sound.muted)
                .show(ui)
                .clicked()
            {
                out.toggle_mute = true;
            }
        }
    }
    status
}

/// The chevron beside the speaker that opens the master volume (AU-010).
/// Returns why a master volume edit failed.
fn master_chevron(
    ui: &mut egui::Ui,
    project: &mut Project,
    master_drag: &mut DragGesture,
) -> Option<String> {
    let mut status = None;
    let mut caret = egui::text::LayoutJob::default();
    caret.append(
        icons::CARET_DOWN,
        0.0,
        egui::TextFormat::simple(
            theme::fonts::icon_font(ui.ctx(), 12.0, false),
            Color32::PLACEHOLDER,
        ),
    );
    let chevron = egui::Button::new(caret)
        .frame_when_inactive(false)
        .min_size(Vec2::new(
            CHEVRON_W,
            theme::look(ui.ctx()).metrics.icon_chip,
        ));
    let (chevron, _) = egui::containers::menu::MenuButton::from_button(chevron).ui(ui, |ui| {
        ui.horizontal(|ui| {
            status = master_volume(ui, project, master_drag);
        });
    });
    chevron.on_hover_text("Master volume");
    status
}

/// The master volume field (AU-010): −∞, then −60 to +12 dB, saved with
/// the project. Typed values accept dB or a percentage like the clip
/// Volume field; a drag is one undo entry. Returns why an edit failed.
fn master_volume(
    ui: &mut egui::Ui,
    project: &mut Project,
    drag: &mut DragGesture,
) -> Option<String> {
    let seq = project.doc().root();
    let gain = project
        .doc()
        .sequence(seq)
        .map_or(GainDb::ZERO, |s| s.master.gain);
    let mut db = gain_db(gain);
    ui.weak("Master");
    let field = ui
        .add(
            DragValue::new(&mut db)
                .speed(0.1)
                .update_while_editing(false)
                .custom_formatter(|v, _| format_db(v))
                .custom_parser(parse_db),
        )
        .on_hover_text("Master volume: the whole mix, −∞ or −60 to +12 dB. Saved with the project");
    let mut status = None;
    let wanted = MasterAudio::clamp_gain(gain_from_db(db));
    if field.changed() && wanted != gain {
        let coalesce = drag.coalesce(project, &field);
        let req = EditRequest::SetMasterGain { seq, gain: wanted };
        status = project.apply(&req, coalesce).err();
    }
    drag.finish(project, &field);
    status
}

/// Screen ↔ canvas mapping for the canvas drawn in `rect`.
fn to_canvas(rect: Rect, view: &CanvasView<'_>, p: Pos2) -> DVec2 {
    screen_to_canvas(
        DVec2::new(f64::from(p.x), f64::from(p.y)),
        DVec2::new(f64::from(rect.min.x), f64::from(rect.min.y)),
        DVec2::new(f64::from(rect.width()), f64::from(rect.height())),
        view.canvas,
    )
}

/// Whether the clip can be picked on the canvas: not on a locked track
/// (TR-007; hidden tracks and disabled clips are never in the frame).
fn pickable(doc: &Document, clip: ClipId) -> bool {
    doc.find_clip(clip)
        .and_then(|loc| doc.sequence(loc.seq)?.find_track(loc.track))
        .is_some_and(|(_, track)| !track.is_locked())
}

/// Clicks, double-clicks and move drags on the canvas.
fn canvas_input(
    ui: &egui::Ui,
    response: &egui::Response,
    project: &mut Project,
    state: &mut PlayerState,
    view: &CanvasView<'_>,
    canvas: Rect,
    out: &mut PlayerOutput,
) {
    let hit = |project: &Project, p: Pos2| {
        let doc = project.doc();
        hit_test(view.layers, to_canvas(canvas, view, p), |l| {
            pickable(doc, l.clip)
        })
    };
    if response.double_clicked()
        && let Some(p) = response.interact_pointer_pos()
        && let Some(clip) = hit(project, p)
    {
        project.selection = vec![clip];
        let is_text = project
            .doc()
            .clip(clip)
            .is_some_and(|c| matches!(c.content, ClipContent::Text(_)));
        out.edit_text = is_text;
    } else if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
        && !on_handle(project, view, canvas, p)
    {
        match hit(project, p) {
            Some(clip) => project.selection = vec![clip],
            None => project.selection.clear(),
        }
    }

    if response.drag_started_by(PointerButton::Primary)
        && state.drag.is_none()
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
    {
        let at = to_canvas(canvas, view, press);
        let grabbed = handle_under(project, view, canvas, press);
        if let Some((clip, layer, size, handle, point)) = grabbed
            && let Some(origin) = shown_position(project, clip)
        {
            let grab = point - at;
            let what = match handle {
                Handle::Tail => Dragging::Tail {
                    start: layer,
                    size,
                    grab,
                    last: None,
                },
                side => Dragging::Width {
                    handle: side,
                    start: layer,
                    grab,
                    last: None,
                },
            };
            state.drag = Some(CanvasDrag {
                clip,
                start: at,
                origin,
                gesture: project.begin_gesture(),
                what,
            });
        } else if let Some(clip) = hit(project, press)
            && let Some(origin) = shown_position(project, clip)
        {
            if !project.selection.contains(&clip) {
                project.selection = vec![clip];
            }
            state.drag = Some(CanvasDrag {
                clip,
                start: at,
                origin,
                gesture: project.begin_gesture(),
                what: Dragging::Move { last: origin },
            });
        }
    }
    let Some(drag) = &mut state.drag else {
        return;
    };
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        // TR-007: Esc puts the clip back, with no undo entry.
        let gesture = drag.gesture;
        state.drag = None;
        out.status = project.cancel_gesture(gesture).err();
        return;
    }
    if response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let pointer = to_canvas(canvas, view, p);
        if let Err(e) = drag_to(project, drag, view, pointer, shift, alt) {
            out.status = Some(e);
        }
    }
    if response.drag_stopped()
        && let Some(drag) = state.drag.take()
    {
        project.end_gesture(drag.gesture);
    }
}

/// `clip`'s position as the Stage shows it: its keyed value at the
/// playhead (TR-027), so a drag starts from where the clip is drawn.
fn shown_position(project: &Project, clip: ClipId) -> Option<CanvasPos> {
    let c = project.doc().clip(clip)?;
    Some(props::shown_transform(c, project.playhead).position)
}

/// Applies one frame of `drag` with the pointer at canvas point
/// `pointer`, merged into the drag's gesture. Keyed properties take a key
/// at the playhead, as the inspector's rows do (TR-027).
fn drag_to(
    project: &mut Project,
    drag: &mut CanvasDrag,
    view: &CanvasView<'_>,
    pointer: DVec2,
    shift: bool,
    alt: bool,
) -> Result<(), String> {
    let seq = project.doc().root();
    let clips = vec![drag.clip];
    let coalesce = Coalesce::Gesture(drag.gesture);
    let scale = ref_px_scale(view.canvas);
    let clip = drag.clip;
    let apply =
        |project: &mut Project, edit: Edit| match props::apply(project, &[clip], edit, coalesce) {
            Some(refused) => Err(refused),
            None => Ok(()),
        };
    match &mut drag.what {
        Dragging::Move { last } => {
            let wanted = moved_position(drag.origin, pointer - drag.start, view.canvas, shift);
            if wanted != *last {
                apply(project, Edit::Transform(TransformEdit::Position(wanted)))?;
                *last = wanted;
            }
        }
        Dragging::Width {
            handle,
            start,
            grab,
            last,
        } => {
            let at = pointer + *grab;
            let Some(wanted) = stage::width_drag(start, *handle, at, alt, scale) else {
                return Ok(());
            };
            if Some(wanted) == *last {
                return Ok(());
            }
            let req = EditRequest::SetTextLayout {
                seq,
                clips,
                edit: TextLayoutEdit::BoxWidth(Some(wanted.width)),
            };
            project.apply(&req, coalesce)?;
            // The opposite edge stays put (TX-005): the same entry moves
            // the clip by half the change.
            let (x, y) = drag.origin.to_px(view.canvas.0, view.canvas.1);
            let position = CanvasPos::from_px(
                x + wanted.shift.x,
                y + wanted.shift.y,
                view.canvas.0,
                view.canvas.1,
            );
            if shown_position(project, drag.clip) != Some(position) {
                apply(project, Edit::Transform(TransformEdit::Position(position)))?;
            }
            *last = Some(wanted);
        }
        Dragging::Tail {
            start,
            size,
            grab,
            last,
        } => {
            let Some(tip) = stage::tail_tip_at(start, pointer + *grab, scale, *size) else {
                return Ok(());
            };
            if Some(tip) != *last {
                apply(project, Edit::Style(TextStyleEdit::BubbleTailTip(tip)))?;
                *last = Some(tip);
            }
        }
    }
    Ok(())
}

/// The single selected text clip, its box in the latest frame and its
/// data, when it can be edited on the canvas (not on a locked track).
fn selected_text<'v>(
    project: &Project,
    view: &CanvasView<'v>,
) -> Option<(ClipId, &'v LayerBox, std::sync::Arc<TextClipData>)> {
    let [clip] = project.selection.as_slice() else {
        return None;
    };
    let doc = project.doc();
    let data = match &doc.clip(*clip)?.content {
        ClipContent::Text(data) => data.clone(),
        _ => return None,
    };
    if !pickable(doc, *clip) {
        return None;
    }
    let layers: &'v [LayerBox] = view.layers;
    let layer = layers.iter().rev().find(|l| l.clip == *clip)?;
    Some((*clip, layer, data))
}

/// The handle of the selected text clip under screen point `p`: its clip,
/// box, font size and which handle.
fn handle_under(
    project: &Project,
    view: &CanvasView<'_>,
    canvas: Rect,
    p: Pos2,
) -> Option<(ClipId, LayerBox, RefPx, Handle, DVec2)> {
    let (clip, layer, data) = selected_text(project, view)?;
    let reach =
        f64::from(HANDLE_REACH) * f64::from(view.canvas.0) / f64::from(canvas.width().max(1.0));
    let handles = text_handles(view, layer, &data);
    let handle = stage::handle_at(&handles, to_canvas(canvas, view, p), reach)?;
    let point = stage::handle_point(&handles, handle)?;
    Some((clip, *layer, data.style.size, handle, point))
}

/// Whether screen point `p` is on a handle (a click there keeps the
/// selection, even outside the box).
fn on_handle(project: &Project, view: &CanvasView<'_>, canvas: Rect, p: Pos2) -> bool {
    handle_under(project, view, canvas, p).is_some()
}

fn text_handles(view: &CanvasView<'_>, layer: &LayerBox, data: &TextClipData) -> TextHandles {
    stage::text_handles(layer, data, ref_px_scale(view.canvas))
}

/// The selected text clip's handles: white squares with a dark rim on the
/// box's side edges, a white dot on the tail tip (ui-concept §9.1).
fn draw_text_handles(
    painter: &egui::Painter,
    project: &Project,
    view: &CanvasView<'_>,
    canvas: Rect,
) {
    let Some((_, layer, data)) = selected_text(project, view) else {
        return;
    };
    let handles = text_handles(view, layer, &data);
    let min = DVec2::new(f64::from(canvas.min.x), f64::from(canvas.min.y));
    let size = DVec2::new(f64::from(canvas.width()), f64::from(canvas.height()));
    let screen = |p: DVec2| {
        let s = canvas_to_screen(p, min, size, view.canvas);
        Pos2::new(s.x as f32, s.y as f32)
    };
    let rim = Stroke::new(1.0, surface::LABEL_DARK);
    for side in handles.sides.into_iter().flatten() {
        let rect = Rect::from_center_size(screen(side), Vec2::splat(10.0));
        painter.rect(rect, 1.0, Color32::WHITE, rim, egui::StrokeKind::Outside);
    }
    if let Some(tail) = handles.tail {
        painter.circle(screen(tail), 6.0, Color32::WHITE, rim);
    }
}

/// A 1.5 pt `accent` outline (over a soft dark underlay, so it reads on
/// any footage) around each selected clip's box in the latest frame
/// (ui-concept §9.1).
fn draw_outlines(
    painter: &egui::Painter,
    look: &theme::Look,
    project: &Project,
    view: &CanvasView<'_>,
    canvas: Rect,
) {
    let min = DVec2::new(f64::from(canvas.min.x), f64::from(canvas.min.y));
    let size = DVec2::new(f64::from(canvas.width()), f64::from(canvas.height()));
    let stroke = Stroke::new(1.5, look.palette.accent);
    let shadow = Stroke::new(3.0, Color32::from_black_alpha(110));
    for clip in &project.selection {
        let Some(layer) = view.layer(*clip) else {
            continue;
        };
        let points: Vec<Pos2> = corners(layer)
            .iter()
            .map(|c| {
                let s = canvas_to_screen(*c, min, size, view.canvas);
                Pos2::new(s.x as f32, s.y as f32)
            })
            .collect();
        painter.add(egui::Shape::closed_line(points.clone(), shadow));
        painter.add(egui::Shape::closed_line(points, stroke));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Modifiers, Pos2, Rect, Vec2};
    use freecut_timeline::glam::{DMat3, DVec2};
    use freecut_timeline::model::ProjectSettings;
    use freecut_timeline::plan::clip_placement;

    use super::*;
    use crate::preview::LayerBox;
    use crate::ui::drag::tests::{Frames, button, key};

    struct Rig {
        frames: Frames,
        project: Project,
        state: PlayerState,
        master: DragGesture,
        clip: ClipId,
        layers: Vec<LayerBox>,
        /// Where the canvas was drawn on screen.
        canvas: Rect,
    }

    impl Rig {
        fn new() -> Rig {
            let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
            let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
            project.selection.clear();
            let size = (300.0, 100.0);
            let layers = vec![LayerBox {
                clip,
                track: project.doc().find_clip(clip).unwrap().track,
                to_canvas: clip_placement(&Default::default(), (1920, 1080))
                    * DMat3::from_translation(-DVec2::new(size.0, size.1) / 2.0),
                size,
                opacity: 1.0,
            }];
            Rig {
                frames: Frames::new(),
                project,
                state: PlayerState::default(),
                master: DragGesture::default(),
                clip,
                layers,
                canvas: Rect::NOTHING,
            }
        }

        fn frame(&mut self, events: Vec<Event>) -> PlayerOutput {
            let mut out = PlayerOutput::default();
            let view = CanvasView {
                canvas: (1920, 1080),
                layers: &self.layers,
            };
            let (project, state, master, canvas) = (
                &mut self.project,
                &mut self.state,
                &mut self.master,
                &mut self.canvas,
            );
            self.frames.run(events, |ui| {
                out = show(
                    ui,
                    project,
                    master,
                    state,
                    &view,
                    None,
                    false,
                    &SoundStatus::default(),
                );
            });
            *canvas = out.canvas.unwrap_or(Rect::NOTHING);
            let down = self.frames.ctx.input(|i| i.pointer.any_down());
            self.state.settle(Some(&mut self.project), down);
            out
        }

        fn position(&self) -> CanvasPos {
            self.project
                .doc()
                .clip(self.clip)
                .unwrap()
                .transform
                .position
        }

        /// Presses at the canvas centre and moves right by `dx` points in
        /// steps.
        fn drag_right(&mut self, dx: f32) -> Pos2 {
            let start = self.canvas.center();
            self.frame(vec![Event::PointerMoved(start), button(start, true)]);
            let mut at = start;
            for step in 1..=4 {
                at = start + Vec2::new(dx * step as f32 / 4.0, 0.0);
                self.frame(vec![Event::PointerMoved(at)]);
            }
            at
        }
    }

    #[test]
    fn a_canvas_drag_moves_the_clip_in_one_undo_entry() {
        let mut rig = Rig::new();
        rig.frame(Vec::new());
        let steps = rig.project.editor.history().len();
        let end = rig.drag_right(100.0);
        rig.frame(vec![button(end, false)]);
        rig.frame(Vec::new());

        assert_eq!(
            rig.project.selection,
            [rig.clip],
            "the dragged clip is selected"
        );
        let (x, y) = rig.position().to_px(1920, 1080);
        let expected = 100.0 * 1920.0 / f64::from(rig.canvas.width());
        assert!((x - expected).abs() < 1.0, "x {x}, expected {expected}");
        assert!(y.abs() < 1e-9);
        assert_eq!(rig.project.editor.history().len(), steps + 1);
        assert!(!rig.project.editor.gesture_open());
        rig.project.undo().unwrap();
        assert_eq!(rig.position(), CanvasPos::CENTER, "one undo: back home");
    }

    #[test]
    fn esc_cancels_a_canvas_drag_without_an_undo_entry() {
        let mut rig = Rig::new();
        rig.frame(Vec::new());
        let steps = rig.project.editor.history().len();
        let end = rig.drag_right(80.0);
        assert_ne!(rig.position(), CanvasPos::CENTER, "moving live");
        rig.frame(vec![key(Key::Escape, Modifiers::NONE)]);
        assert_eq!(rig.position(), CanvasPos::CENTER);
        rig.frame(vec![
            Event::PointerMoved(end + Vec2::new(30.0, 0.0)),
            button(end, false),
        ]);
        assert_eq!(
            rig.position(),
            CanvasPos::CENTER,
            "the rest of the drag is ignored"
        );
        assert_eq!(rig.project.editor.history().len(), steps);
    }

    impl Rig {
        /// The screen point of canvas point `p`.
        fn screen(&self, p: DVec2) -> Pos2 {
            let min = DVec2::new(f64::from(self.canvas.min.x), f64::from(self.canvas.min.y));
            let size = DVec2::new(
                f64::from(self.canvas.width()),
                f64::from(self.canvas.height()),
            );
            let s = canvas_to_screen(p, min, size, (1920, 1080));
            Pos2::new(s.x as f32, s.y as f32)
        }

        /// Canvas pixels per screen point.
        fn per_point(&self) -> f64 {
            1920.0 / f64::from(self.canvas.width())
        }

        fn text(&self) -> TextClipData {
            self.project
                .doc()
                .clip(self.clip)
                .unwrap()
                .content
                .text()
                .unwrap()
                .as_ref()
                .clone()
        }

        /// Presses at `from` and drags by `delta` points in four steps.
        fn drag(&mut self, from: Pos2, delta: Vec2, modifiers: Modifiers) -> Pos2 {
            self.frame(vec![Event::PointerMoved(from), button(from, true)]);
            let mut at = from;
            for step in 1..=4 {
                at = from + delta * step as f32 / 4.0;
                self.frame(vec![
                    Event::ModifiersChanged(modifiers),
                    Event::PointerMoved(at),
                ]);
            }
            at
        }
    }

    #[test]
    fn a_side_handle_sets_a_fixed_width_and_keeps_the_other_edge() {
        let mut rig = Rig::new();
        rig.project.selection = vec![rig.clip];
        rig.frame(Vec::new());
        let steps = rig.project.editor.history().len();
        // The right handle sits mid-edge: the box is 300 px wide.
        let right = rig.screen(DVec2::new(960.0 + 150.0, 540.0));
        let end = rig.drag(right, Vec2::new(50.0, 0.0), Modifiers::NONE);
        rig.frame(vec![button(end, false)]);
        rig.frame(Vec::new());
        let grown = 50.0 * rig.per_point();
        let width = rig.text().layout.box_width.unwrap().as_f64();
        assert!((width - (300.0 + grown)).abs() < 0.02, "width {width}");
        // The left edge stayed: the centre moved by half the growth.
        let (x, y) = rig.position().to_px(1920, 1080);
        assert!((x - grown / 2.0).abs() < 1.0, "x {x}");
        assert!(y.abs() < 1e-9);
        assert_eq!(rig.project.editor.history().len(), steps + 1, "one entry");
        assert_eq!(rig.project.selection, [rig.clip], "still selected");
        rig.project.undo().unwrap();
        assert_eq!(rig.text().layout.box_width, None, "one undo: auto again");
        assert_eq!(rig.position(), CanvasPos::CENTER);

        // Alt resizes around the centre: no move.
        let left = rig.screen(DVec2::new(960.0 - 150.0, 540.0));
        let end = rig.drag(left, Vec2::new(-30.0, 0.0), Modifiers::ALT);
        rig.frame(vec![button(end, false)]);
        let width = rig.text().layout.box_width.unwrap().as_f64();
        assert!((width - (300.0 + 60.0 * rig.per_point())).abs() < 0.02);
        assert_eq!(rig.position(), CanvasPos::CENTER);
    }

    /// S1: a press a few points off a handle grabs it without a jump: the
    /// width grows by the pointer's travel only, and the tail tip moves by
    /// it.
    #[test]
    fn a_handle_grabbed_off_centre_follows_the_pointer_without_a_jump() {
        use freecut_timeline::model::TextLayerKind;

        let mut rig = Rig::new();
        rig.project.selection = vec![rig.clip];
        rig.frame(Vec::new());
        // 8 pt right of the right handle (within its reach).
        let right = rig.screen(DVec2::new(960.0 + 150.0, 540.0)) + Vec2::new(8.0, 0.0);
        let end = rig.drag(right, Vec2::new(50.0, 0.0), Modifiers::NONE);
        rig.frame(vec![button(end, false)]);
        let width = rig.text().layout.box_width.unwrap().as_f64();
        let grown = 50.0 * rig.per_point();
        assert!((width - (300.0 + grown)).abs() < 0.05, "width {width}");
        rig.project.undo().unwrap();

        let req = EditRequest::SetTextStyle {
            seq: rig.project.doc().root(),
            clips: vec![rig.clip],
            edit: TextStyleEdit::SetLayer {
                layer: TextLayerKind::Bubble,
                on: true,
            },
        };
        rig.project.apply(&req, Coalesce::Never).unwrap();
        rig.frame(Vec::new());
        let before = rig.text().style.bubble.value.spec.tail.value.tip;
        let tip = DVec2::new(before.x.as_f64(), before.y.as_f64()) + DVec2::new(960.0, 540.0);
        let off = rig.screen(tip) + Vec2::new(-6.0, 5.0);
        let end = rig.drag(off, Vec2::new(-20.0, 10.0), Modifiers::NONE);
        rig.frame(vec![button(end, false)]);
        let after = rig.text().style.bubble.value.spec.tail.value.tip;
        let moved = DVec2::new(
            after.x.as_f64() - before.x.as_f64(),
            after.y.as_f64() - before.y.as_f64(),
        );
        let want = DVec2::new(-20.0, 10.0) * rig.per_point();
        assert!(
            (moved - want).length() < 0.05,
            "moved {moved:?}, want {want:?}"
        );
    }

    #[test]
    fn esc_cancels_a_handle_drag_and_a_click_on_one_keeps_the_selection() {
        let mut rig = Rig::new();
        rig.project.selection = vec![rig.clip];
        rig.frame(Vec::new());
        let steps = rig.project.editor.history().len();
        let right = rig.screen(DVec2::new(960.0 + 150.0, 540.0));
        rig.drag(right, Vec2::new(40.0, 0.0), Modifiers::NONE);
        assert!(rig.text().layout.box_width.is_some(), "live");
        rig.frame(vec![key(Key::Escape, Modifiers::NONE)]);
        assert_eq!(rig.text().layout.box_width, None);
        assert_eq!(rig.position(), CanvasPos::CENTER);
        assert_eq!(rig.project.editor.history().len(), steps);
        rig.frame(vec![button(right, false)]);
        // A click on the handle (outside the box) keeps the selection.
        let outside = rig.screen(DVec2::new(960.0 + 155.0, 540.0));
        rig.frame(vec![Event::PointerMoved(outside), button(outside, true)]);
        rig.frame(vec![button(outside, false)]);
        assert_eq!(rig.project.selection, [rig.clip]);
    }

    #[test]
    fn the_tail_handle_moves_the_tip_in_one_entry() {
        use freecut_timeline::model::TextLayerKind;

        let mut rig = Rig::new();
        let req = EditRequest::SetTextStyle {
            seq: rig.project.doc().root(),
            clips: vec![rig.clip],
            edit: TextStyleEdit::SetLayer {
                layer: TextLayerKind::Bubble,
                on: true,
            },
        };
        rig.project.apply(&req, Coalesce::Never).unwrap();
        rig.project.selection = vec![rig.clip];
        rig.frame(Vec::new());
        let steps = rig.project.editor.history().len();
        let before = rig.text().style.bubble.value.spec.tail.value.tip;
        // 1080 p: one ref px is one canvas px; the box centre is the
        // canvas centre.
        let tip = DVec2::new(before.x.as_f64(), before.y.as_f64()) + DVec2::new(960.0, 540.0);
        let end = rig.drag(rig.screen(tip), Vec2::new(-20.0, 10.0), Modifiers::NONE);
        rig.frame(vec![button(end, false)]);
        let after = rig.text().style.bubble.value.spec.tail.value.tip;
        let moved = DVec2::new(
            after.x.as_f64() - before.x.as_f64(),
            after.y.as_f64() - before.y.as_f64(),
        );
        let want = DVec2::new(-20.0, 10.0) * rig.per_point();
        assert!(
            (moved - want).length() < 0.05,
            "moved {moved:?}, want {want:?}"
        );
        assert_eq!(rig.position(), CanvasPos::CENTER, "the clip stays");
        assert_eq!(rig.project.editor.history().len(), steps + 1);
        rig.project.undo().unwrap();
        assert_eq!(rig.text().style.bubble.value.spec.tail.value.tip, before);
    }

    #[test]
    fn dragging_a_clip_with_position_keys_keys_it_at_the_playhead() {
        use freecut_timeline::edit::KeyEdit;
        use freecut_timeline::model::{PropPath, PropValue, TransformProp};

        // TR-027: Position keyed at 0 s (centre) and 1 s (300 px right);
        // the playhead sits on the first key.
        let mut rig = Rig::new();
        let prop = PropPath::Transform(TransformProp::Position);
        let later = CanvasPos::from_px(300.0, 0.0, 1920, 1080);
        for (at, value) in [
            (Tick::ZERO, CanvasPos::CENTER),
            (Tick::from_secs(1).unwrap(), later),
        ] {
            let req = EditRequest::EditKeys {
                seq: rig.project.doc().root(),
                clips: vec![rig.clip],
                prop,
                edit: KeyEdit::Add {
                    at,
                    value: Some(PropValue::Position(value)),
                },
            };
            rig.project.apply(&req, Coalesce::Never).unwrap();
        }
        rig.frame(Vec::new());
        let steps = rig.project.editor.history().len();
        let end = rig.drag_right(100.0);
        rig.frame(vec![button(end, false)]);

        let clip = rig.project.doc().clip(rig.clip).unwrap().clone();
        let shown = props::shown_transform(&clip, Tick::ZERO).position;
        let (x, _) = shown.to_px(1920, 1080);
        let expected = 100.0 * rig.per_point();
        assert!(
            (x - expected).abs() < 1.0,
            "shown x {x}, expected {expected}"
        );
        assert_eq!(rig.position(), CanvasPos::CENTER, "the static value stays");
        let keys = crate::prop_keys::shown_keys(&clip, prop);
        assert_eq!(keys.len(), 2, "the key at the playhead moved, no new one");
        assert_eq!(keys[1].value, PropValue::Position(later));
        assert_eq!(rig.project.editor.history().len(), steps + 1);
        rig.project.undo().unwrap();
        let clip = rig.project.doc().clip(rig.clip).unwrap().clone();
        assert_eq!(
            props::shown_transform(&clip, Tick::ZERO).position,
            CanvasPos::CENTER
        );
    }

    #[test]
    fn clicks_select_the_clip_under_the_pointer_or_clear_the_selection() {
        let mut rig = Rig::new();
        rig.frame(Vec::new());
        let centre = rig.canvas.center();
        rig.frame(vec![Event::PointerMoved(centre), button(centre, true)]);
        rig.frame(vec![button(centre, false)]);
        assert_eq!(rig.project.selection, [rig.clip]);
        let corner = rig.canvas.left_top() + Vec2::new(5.0, 5.0);
        rig.frame(vec![Event::PointerMoved(corner), button(corner, true)]);
        rig.frame(vec![button(corner, false)]);
        assert!(rig.project.selection.is_empty());
    }

    #[test]
    fn a_fitted_canvas_is_centred_and_fills_one_axis() {
        let room = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(800.0, 300.0));
        let view = StageView::default();
        let c = canvas_rect(room, (1920, 1080), &view);
        assert!((c.center() - room.center()).length() < 1e-3);
        assert!((c.height() - 300.0).abs() < 1e-3, "{c:?}");
        assert!(c.width() <= room.width());
        assert!((fit_scale(room.size(), (1920, 1080)) - 300.0 / 1080.0).abs() < 1e-6);
        // Zoomed: the pan moves the centre.
        let zoomed = StageView {
            fitted: false,
            zoom: 0.5,
            pan: Vec2::new(30.0, -10.0),
        };
        let c = canvas_rect(room, (1920, 1080), &zoomed);
        assert_eq!(c.size(), Vec2::new(960.0, 540.0));
        assert!((c.center() - (room.center() + Vec2::new(30.0, -10.0))).length() < 1e-3);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let room = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 450.0));
        let out = (1920, 1080);
        let mut view = StageView::default();
        let pointer = Pos2::new(500.0, 200.0);
        for factor in [1.3, 1.9, 0.7, 3.0] {
            let before = canvas_rect(room, out, &view);
            let next = zoom_about(&view, room, out, pointer, factor, 2.0);
            let after = canvas_rect(room, out, &next);
            // The canvas point under the pointer, in canvas pixels.
            let at = |c: Rect| (pointer - c.min) / c.size() * Vec2::new(1920.0, 1080.0);
            assert!(
                (at(before) - at(after)).length() < 0.5 * 1920.0 / after.width().max(1.0) + 0.5
            );
            assert!(!next.fitted);
            // At most ×2 per event.
            let ratio = after.width() / before.width();
            assert!((0.5 - 1e-4..=2.0 + 1e-4).contains(&ratio), "{ratio}");
            view = next;
        }
        // 10 %–800 % of the canvas's pixels at 2 px per point.
        let mut v = view;
        for _ in 0..20 {
            v = zoom_about(&v, room, out, pointer, 2.0, 2.0);
        }
        assert!((v.zoom * 2.0 - 8.0).abs() < 1e-4, "{}", v.zoom);
        for _ in 0..40 {
            v = zoom_about(&v, room, out, pointer, 0.5, 2.0);
        }
        assert!(v.zoom * 2.0 >= 0.1 - 1e-4, "{}", v.zoom);
        assert!((wheel_factor(200.0) - (-0.5f32).exp()).abs() < 1e-6);
        assert_eq!(wheel_factor(-10_000.0), 2.0);
        assert_eq!(wheel_factor(10_000.0), 0.5);
    }

    #[test]
    fn panning_keeps_64_pt_of_the_canvas_inside() {
        let room = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 450.0));
        let out = (1920, 1080);
        let far = StageView {
            fitted: false,
            zoom: 1.0,
            pan: Vec2::new(10_000.0, -10_000.0),
        };
        let pan = clamp_pan(&far, room, out);
        let c = canvas_rect(room, out, &StageView { pan, ..far });
        let overlap = c.intersect(room);
        assert!((overlap.width() - 64.0).abs() < 1e-3, "{overlap:?}");
        assert!((overlap.height() - 64.0).abs() < 1e-3, "{overlap:?}");
        assert_eq!(clamp_pan(&StageView::default(), room, out), Vec2::ZERO);
    }

    #[test]
    fn play_stays_centred_and_side_groups_never_overlap_it() {
        let bar = Rect::from_min_size(Pos2::new(100.0, 0.0), Vec2::new(700.0, 44.0));
        for left_w in [120.0, 200.0, 250.0] {
            let (left, centre, right) = transport_layout(bar, left_w, 138.0, 160.0);
            assert!((centre.center().x - bar.center().x).abs() < 1e-3);
            assert!(left.right() <= centre.left() && centre.right() <= right.left());
            assert!(left.width() >= left_w);
        }
        // Too narrow: the centre moves aside instead of overlapping.
        let narrow = Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 44.0));
        let (left, centre, right) = transport_layout(narrow, 230.0, 138.0, 40.0);
        assert!(left.right() <= centre.left() && centre.right() <= right.left());
        assert!(left.width() >= 230.0 - 1e-3);
    }

    #[test]
    fn a_narrow_transport_folds_the_length_then_fit_then_the_resolution() {
        let w = TransportWidths {
            timecode: 116.0,
            total: 90.0,
            centre: 138.0,
            speaker: 26.0,
            chevron: 16.0,
            resolution: 90.0,
            fit: 26.0,
            spacing: 6.0,
        };
        let all = transport_fold(700.0, &w);
        assert!(all.total && all.resolution && all.fit && all.chevron);
        let (l, r) = w.sides(all);
        assert!(l + w.centre + r + 24.0 <= 700.0);
        let f = transport_fold(520.0, &w);
        assert!(!f.total && f.resolution && f.fit && f.chevron, "{f:?}");
        let f = transport_fold(440.0, &w);
        assert!(!f.total && f.resolution && !f.fit && f.chevron, "{f:?}");
        // The chevron folds before the resolution, so play stays centred
        // and the volume group doesn't crowd it.
        let f = transport_fold(420.0, &w);
        assert!(!f.total && f.resolution && !f.fit && !f.chevron, "{f:?}");
        let (l, r) = w.sides(f);
        assert!(2.0 * l.max(r) + w.centre + 2.0 * GROUP_GAP <= 420.0);
        let f = transport_fold(300.0, &w);
        assert!(!f.total && !f.resolution && !f.fit && !f.chevron, "{f:?}");
    }

    #[test]
    fn transport_buttons_grey_out_at_the_ends() {
        assert_eq!(transport_enabled(0, 90), [false, false, true, true, true]);
        assert_eq!(transport_enabled(45, 90), [true; 5]);
        assert_eq!(transport_enabled(90, 90), [true, true, true, false, false]);
        assert_eq!(transport_enabled(0, 0), [false; 5]);
    }

    #[test]
    fn preview_resolution_scales_the_render_not_the_display() {
        let out = (1920, 1080);
        let screen = (800, 450);
        assert_eq!(PreviewResolution::Auto.wanted(out, screen), (800, 450));
        assert_eq!(
            PreviewResolution::Auto.wanted(out, (4000, 2250)),
            (1920, 1080)
        );
        assert_eq!(PreviewResolution::Full.wanted(out, screen), (1920, 1080));
        assert_eq!(PreviewResolution::Half.wanted(out, screen), (960, 540));
        assert_eq!(PreviewResolution::Quarter.wanted(out, screen), (480, 270));
        assert_eq!(
            PreviewResolution::Quarter.wanted((40, 40), screen),
            (16, 16)
        );
    }

    /// Dragging the timecode field moves the playhead a frame per 3 pt and
    /// never adds an undo entry; a typed timecode seeks on Enter.
    #[test]
    fn the_timecode_field_scrubs_and_seeks_without_undo_entries() {
        let mut rig = Rig::new();
        let rate = rig.project.doc().settings().frame_rate;
        let out = rig.frame(Vec::new());
        assert!(out.seek.is_none());
        let steps = rig.project.editor.history().len();
        let field = out.timecode.unwrap().center();
        let mut seek = None;
        let mut apply = |rig: &mut Rig, events: Vec<Event>| {
            let out = rig.frame(events);
            if let Some(t) = out.seek {
                rig.project.playhead = t;
                seek = Some(t);
            }
        };
        apply(
            &mut rig,
            vec![Event::PointerMoved(field), button(field, true)],
        );
        for step in 1..=5 {
            let at = field + Vec2::new(6.0 * step as f32, 0.0);
            apply(&mut rig, vec![Event::PointerMoved(at)]);
        }
        apply(&mut rig, vec![button(field + Vec2::new(30.0, 0.0), false)]);
        // 30 pt at 3 pt per frame, less egui's drag threshold.
        let frame = rate.frame_index(seek.unwrap());
        assert!((7..=10).contains(&frame), "frame {frame}");
        assert_eq!(rig.project.editor.history().len(), steps, "no undo entry");

        // A click types; Enter seeks to the typed time.
        rig.frame(vec![Event::PointerMoved(field), button(field, true)]);
        rig.frame(vec![button(field, false)]);
        rig.frame(Vec::new());
        let mut seek = None;
        for events in [
            vec![Event::Text("00:00:01:00".to_owned())],
            vec![key(Key::Enter, Modifiers::NONE)],
        ] {
            let out = rig.frame(events);
            seek = seek.or(out.seek);
        }
        let nominal = i64::from(rate.nominal_fps());
        assert_eq!(rate.frame_index(seek.unwrap()), nominal, "one second in");
        assert_eq!(rig.project.editor.history().len(), steps);
    }
}

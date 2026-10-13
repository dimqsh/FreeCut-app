//! The In / Loop / Out timing strip (ui-concept §13.11, TX-022): a 28 pt
//! bar standing for the clip, In at the left, Out at the right and Loop
//! between, each with its preset's name when it fits. Clicking a segment
//! selects that slot; dragging the join after In or before Out asks for a
//! new length (the caller snaps and commits it, one undo entry per drag;
//! `Esc` during the drag reports [`StripOutput::cancelled`]).
//!
//! Segments are told apart by shape as well as colour: the selected slot
//! is filled with `accent` at 30 %, Loop carries a wavy line, an empty slot
//! is drawn hollow, and In and Out shortened to fit the clip are hatched.

use eframe::egui::{
    self, CursorIcon, Id, Key, Modifiers, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    WidgetInfo, WidgetType,
};
use freecut_timeline::model::Slot;

use crate::anim::StripLayout;
use crate::theme::{self, Type};

/// What the strip reports.
#[derive(Debug, Default)]
pub struct StripOutput {
    /// A segment was clicked.
    pub picked: Option<Slot>,
    /// A join is being dragged to this strip fraction, with the join's
    /// response (for the caller's undo coalescing).
    pub dragged: Option<(Slot, f64, Response)>,
    /// A join that didn't move this frame, to end its drag on release.
    pub idle: Vec<Response>,
    /// `Esc` stopped a join drag this frame.
    pub cancelled: bool,
}

/// One strip.
#[derive(Debug)]
pub struct TimingStrip<'a> {
    pub id: Id,
    pub layout: StripLayout,
    pub selected: Slot,
    /// The preset names of In, Loop and Out, when set.
    pub names: [Option<&'a str>; 3],
}

const HEIGHT: f32 = 28.0;
const JOIN: f32 = 8.0;

fn slot_index(slot: Slot) -> usize {
    match slot {
        Slot::Entrance => 0,
        Slot::Continuous => 1,
        Slot::Exit => 2,
    }
}

impl TimingStrip<'_> {
    pub fn show(self, ui: &mut egui::Ui) -> StripOutput {
        let mut out = StripOutput::default();
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let width = ui.available_width().max(60.0);
        let (rect, bar) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), Sense::click());
        let l = &self.layout;
        let x_of = |f: f64| rect.left() + (f as f32) * rect.width();
        let segs = [
            (Slot::Entrance, 0.0, l.in_end, l.in_set),
            (Slot::Continuous, l.in_end, l.out_start, l.loop_set),
            (Slot::Exit, l.out_start, 1.0, l.out_set),
        ];
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect.expand(2.0));
            for (slot, a, b, set) in segs {
                let r = Rect::from_min_max(
                    egui::pos2(x_of(a), rect.top()),
                    egui::pos2(x_of(b).max(x_of(a) + 1.0), rect.bottom()),
                );
                let selected = slot == self.selected;
                let fill = if selected {
                    p.accent.gamma_multiply(0.3)
                } else if set {
                    p.bg_well
                } else {
                    egui::Color32::TRANSPARENT
                };
                painter.rect_filled(r, theme::R_SM, fill);
                let border = if set {
                    Stroke::new(1.0, p.border_control)
                } else {
                    // An empty slot: a hollow, dashed-looking outline.
                    Stroke::new(1.0, p.text_tertiary)
                };
                painter.rect_stroke(r.shrink(0.5), theme::R_SM, border, StrokeKind::Inside);
                if slot == Slot::Continuous && set {
                    wave(&painter, r, p.text_tertiary);
                }
                if l.shortened && set && slot != Slot::Continuous {
                    hatch(&painter, r, p.warning.gamma_multiply(0.6));
                }
                let name = self.names[slot_index(slot)];
                let label = match (slot, name) {
                    (_, Some(n)) => n.to_owned(),
                    (Slot::Entrance, None) => "In".to_owned(),
                    (Slot::Continuous, None) => "Loop".to_owned(),
                    (Slot::Exit, None) => "Out".to_owned(),
                };
                let font = Type::Caption.style(ui.style()).resolve(ui.style());
                let color = if set { p.text_primary } else { p.text_tertiary };
                let galley = painter.layout_no_wrap(label, font, color);
                if galley.size().x + 6.0 <= r.width() {
                    painter.galley(r.center() - galley.size() / 2.0, galley, color);
                }
            }
            painter.rect_stroke(
                rect,
                theme::R_SM,
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Inside,
            );
        }
        let name = |s: Slot| match s {
            Slot::Entrance => "In",
            Slot::Continuous => "Loop",
            Slot::Exit => "Out",
        };
        let info = format!("Animation timing: {} selected", name(self.selected));
        bar.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, &info));

        let mut on_join = false;
        for (slot, f) in l.joins() {
            let x = x_of(f);
            let hit =
                Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(JOIN, HEIGHT));
            let join_id = self.id.with(("join", slot_index(slot)));
            let r = ui.interact(hit, join_id, Sense::drag());
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                on_join = true;
            }
            let color = if r.dragged() || r.hovered() {
                p.accent
            } else {
                p.text_secondary
            };
            ui.painter().line_segment(
                [
                    egui::pos2(x, rect.top() + 3.0),
                    egui::pos2(x, rect.bottom() - 3.0),
                ],
                Stroke::new(2.0, color),
            );
            let tip = match slot {
                Slot::Exit => "Drag to change the Out length",
                _ => "Drag to change the In length",
            };
            let r = r.on_hover_text(tip);
            if (r.dragged() || r.drag_stopped())
                && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                ui.ctx().stop_dragging();
                out.cancelled = true;
                continue;
            }
            match r.interact_pointer_pos() {
                Some(pos) if r.dragged() && r.drag_delta().x != 0.0 => {
                    let frac = f64::from((pos.x - rect.left()) / rect.width().max(1.0));
                    out.dragged = Some((slot, frac, r));
                }
                _ => out.idle.push(r),
            }
        }
        if !on_join
            && bar.clicked()
            && let Some(pos) = bar.interact_pointer_pos()
        {
            let frac = f64::from((pos.x - rect.left()) / rect.width().max(1.0));
            out.picked = Some(l.slot_at(frac));
        }
        out
    }
}

/// Loop's wavy line (shape, not colour alone).
fn wave(painter: &egui::Painter, r: Rect, color: egui::Color32) {
    if r.width() < 12.0 {
        return;
    }
    let y = r.bottom() - 5.0;
    let n = ((r.width() - 8.0) / 2.0).max(2.0) as usize;
    let points: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let x = r.left() + 4.0 + i as f32 * 2.0;
            egui::pos2(x, y + 2.0 * ((x - r.left()) / 4.0).sin())
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(1.0, color)));
}

/// Diagonal hatching for In and Out shortened to fit the clip (TX-022).
fn hatch(painter: &egui::Painter, r: Rect, color: egui::Color32) {
    let p = painter.with_clip_rect(r.intersect(painter.clip_rect()));
    let mut x = r.left() - r.height();
    while x < r.right() {
        p.line_segment(
            [
                egui::pos2(x, r.bottom()),
                egui::pos2(x + r.height(), r.top()),
            ],
            Stroke::new(1.0, color),
        );
        x += 6.0;
    }
}

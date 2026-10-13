//! Toasts (UI-040, ui-concept §13.12): short notes in the bottom-right
//! corner, 16 pt from the window's right edge and above the status line.
//!
//! - At most three show at once; the rest wait in order. The same message
//!   again is folded into the toast already there, with a count.
//! - Info leaves after 4 s, success after 3 s, warning after 8 s; errors
//!   stay until closed. Pointing at a toast pauses its clock.
//! - A toast arrives over `dur.slow` rising 8 pt and leaves over
//!   `dur.exit` falling 8 pt ([`toast_offset`]); with Reduce motion both
//!   are instant.
//! - Each toast is `e2`, `r.lg`, at most 360 pt wide and at least 32 pt
//!   tall: icon, message (`type.body`), up to two text buttons and ×.
//!   Errors carry a 3 pt `danger` bar on the left edge.
//!
//! Other parts of the app raise toasts through [`Toasts::push`]; none
//! draws its own.

use std::collections::VecDeque;
use std::path::PathBuf;

use eframe::egui::{self, Align, Align2, Color32, Frame, Id, Margin, Order, Sense, Shadow, Stroke};

use crate::theme::{self, fonts, icons, motion};
use crate::widgets::icon_button::IconButton;

/// How many toasts show at once.
pub const MAX_VISIBLE: usize = 3;
/// A toast's widest.
pub const MAX_WIDTH: f32 = 360.0;
/// How far a toast rises when it arrives and falls when it leaves.
pub const TRAVEL: f32 = 8.0;
/// The gap between stacked toasts and from the window's right edge.
pub const GAP: f32 = 8.0;
pub const EDGE: f32 = 16.0;

/// What a toast is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

/// How long a toast of `kind` stays, in seconds; `None`: until closed.
pub fn hold_time(kind: ToastKind) -> Option<f64> {
    match kind {
        ToastKind::Info => Some(4.0),
        ToastKind::Success => Some(3.0),
        ToastKind::Warning => Some(8.0),
        ToastKind::Error => None,
    }
}

/// Where a toast is drawn `t` (0..=1) into its arrival (`entering`) or its
/// exit: `(how far below its place, opacity)`. Arriving it rises 8 pt with
/// an ease-out; leaving it falls 8 pt with an ease-in.
pub fn toast_offset(t: f32, entering: bool) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    if entering {
        let e = motion::ease_out(t);
        ((1.0 - e) * TRAVEL, e)
    } else {
        let e = motion::ease_in(t);
        (e * TRAVEL, 1.0 - e)
    }
}

/// What a toast's button asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastAction {
    /// Show the Export sheet (a finished or failed export).
    ShowExport,
    /// Save the project again (a failed save).
    RetrySave,
    /// Show the file in the system's file manager.
    Reveal(PathBuf),
    /// Place a voiceover take that was kept but not placed (AU-027).
    PlaceTake(PathBuf),
    /// Open the recorder panel on its Takes list (AU-027).
    ShowTakes,
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    kind: ToastKind,
    message: String,
    /// How many times the same message came.
    count: u32,
    actions: Vec<(String, ToastAction)>,
    /// Seconds it has shown, not counting time under the pointer.
    shown: f64,
    /// The arrival's progress, 0..=1.
    arrive: f32,
    /// The exit's progress, once leaving.
    leave: Option<f32>,
}

/// The toasts of the window.
#[derive(Debug, Default)]
pub struct Toasts {
    toasts: VecDeque<Toast>,
    next: u64,
}

impl Toasts {
    /// Raises a toast. The same message and kind as a toast already there
    /// is folded into it: its count goes up and its clock starts again.
    pub fn push(&mut self, kind: ToastKind, message: impl Into<String>) {
        self.push_with(kind, message, Vec::new());
    }

    /// [`Toasts::push`] with up to two buttons.
    pub fn push_with(
        &mut self,
        kind: ToastKind,
        message: impl Into<String>,
        mut actions: Vec<(String, ToastAction)>,
    ) {
        let message = message.into();
        if message.trim().is_empty() {
            return;
        }
        actions.truncate(2);
        if let Some(same) = self
            .toasts
            .iter_mut()
            .find(|t| t.kind == kind && t.message == message && t.leave.is_none())
        {
            same.count += 1;
            same.shown = 0.0;
            same.actions = actions;
            return;
        }
        self.next += 1;
        self.toasts.push_back(Toast {
            id: self.next,
            kind,
            message,
            count: 1,
            actions,
            shown: 0.0,
            arrive: 0.0,
            leave: None,
        });
    }

    /// The messages showing or waiting, oldest first.
    #[cfg(test)]
    pub fn messages(&self) -> Vec<&str> {
        self.toasts.iter().map(|t| t.message.as_str()).collect()
    }

    /// Advances the clocks by `dt` seconds (`hovered`: the toast id under
    /// the pointer) and drops the toasts that finished leaving. Only the
    /// first [`MAX_VISIBLE`] run.
    fn advance(&mut self, dt: f32, slow: f32, exit: f32, hovered: Option<u64>) {
        for t in self.toasts.iter_mut().take(MAX_VISIBLE) {
            if let Some(leave) = &mut t.leave {
                *leave = if exit <= 0.0 { 1.0 } else { *leave + dt / exit };
                continue;
            }
            t.arrive = if slow <= 0.0 {
                1.0
            } else {
                (t.arrive + dt / slow).min(1.0)
            };
            if hovered != Some(t.id) {
                t.shown += f64::from(dt);
            }
            if hold_time(t.kind).is_some_and(|hold| t.shown >= hold) {
                t.leave = Some(0.0);
            }
        }
        self.toasts.retain(|t| t.leave.is_none_or(|l| l < 1.0));
    }

    fn close(&mut self, id: u64) {
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.leave.get_or_insert(0.0);
        }
    }

    /// Whether nothing is showing or waiting.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// Draws the toasts with their bottom edge `bottom` pt above the
    /// window's bottom, and returns the buttons pressed.
    pub fn show(&mut self, ctx: &egui::Context, bottom: f32) -> Vec<ToastAction> {
        let mut actions = Vec::new();
        if self.toasts.is_empty() {
            return actions;
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let hovered = ctx.data(|d| d.get_temp::<u64>(hover_id()));
        self.advance(
            dt,
            motion::duration(ctx, motion::SLOW),
            motion::duration(ctx, motion::EXIT),
            hovered,
        );
        let look = theme::look(ctx);
        let p = look.palette;
        let window = ctx.content_rect();
        let mut y = window.bottom() - bottom;
        let mut now_hovered = None;
        let mut closed = Vec::new();
        // The newest showing toast sits lowest.
        let visible: Vec<Toast> = self.toasts.iter().take(MAX_VISIBLE).cloned().collect();
        for toast in visible.iter().rev() {
            let (drop, opacity) = match toast.leave {
                Some(l) => toast_offset(l, false),
                None => toast_offset(toast.arrive, true),
            };
            let height_id = Id::new(("toast-height", toast.id));
            let height = ctx.data(|d| d.get_temp::<f32>(height_id)).unwrap_or(40.0);
            let anchor = egui::pos2(window.right() - EDGE, y + drop);
            let area = egui::Area::new(Id::new(("toast", toast.id)))
                .order(Order::Tooltip)
                .pivot(Align2::RIGHT_BOTTOM)
                .fixed_pos(anchor)
                .interactable(true);
            let response = area.show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                let frame = Frame::NONE
                    .fill(p.bg_card)
                    .corner_radius(theme::R_LG)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .shadow(Shadow {
                        offset: [0, 4],
                        blur: 12,
                        spread: 0,
                        color: Color32::from_black_alpha(p.shadow_e2),
                    })
                    .inner_margin(Margin {
                        left: 12,
                        right: 6,
                        top: 4,
                        bottom: 4,
                    });
                let inner = frame.show(ui, |ui| {
                    ui.set_max_width(MAX_WIDTH - 18.0);
                    ui.set_min_height(24.0);
                    toast_body(ui, toast, &mut actions, &mut closed);
                });
                let rect = inner.response.rect;
                if toast.kind == ToastKind::Error {
                    let bar = egui::Rect::from_min_size(
                        rect.min + egui::vec2(0.5, 8.0),
                        egui::vec2(3.0, rect.height() - 16.0),
                    );
                    ui.painter().rect_filled(bar, 1.5, p.danger);
                }
                (rect, ui.rect_contains_pointer(rect))
            });
            let (rect, pointed) = response.inner;
            ctx.data_mut(|d| d.insert_temp(height_id, rect.height()));
            if pointed {
                now_hovered = Some(toast.id);
            }
            y -= height + GAP;
        }
        ctx.data_mut(|d| match now_hovered {
            Some(id) => {
                d.insert_temp(hover_id(), id);
            }
            None => {
                d.remove::<u64>(hover_id());
            }
        });
        for id in closed {
            self.close(id);
        }
        ctx.request_repaint();
        actions
    }
}

fn hover_id() -> Id {
    Id::new("freecut-toast-hovered")
}

/// The icon and its colour for `kind`.
fn mark(kind: ToastKind, p: &theme::Palette) -> (&'static str, Color32) {
    match kind {
        ToastKind::Info => (icons::INFO, p.text_tertiary),
        ToastKind::Success => (icons::CHECK_CIRCLE, p.success),
        ToastKind::Warning => (icons::WARNING, p.warning),
        ToastKind::Error => (icons::ERROR, p.danger),
    }
}

fn toast_body(
    ui: &mut egui::Ui,
    toast: &Toast,
    actions: &mut Vec<ToastAction>,
    closed: &mut Vec<u64>,
) {
    let p = theme::look(ui.ctx()).palette;
    let (icon, colour) = mark(toast.kind, &p);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            egui::RichText::new(icon)
                .font(fonts::icon_font(ui.ctx(), 15.0, false))
                .color(colour),
        );
        let text = if toast.count > 1 {
            format!("{} (×{})", toast.message, toast.count)
        } else {
            toast.message.clone()
        };
        // Leave room for the buttons and ×.
        let button_font = egui::TextStyle::Button.resolve(ui.style());
        let pad = ui.spacing().button_padding.x * 2.0 + 8.0;
        let buttons: f32 = toast
            .actions
            .iter()
            .map(|(label, _)| {
                ui.painter()
                    .layout_no_wrap(label.clone(), button_font.clone(), Color32::WHITE)
                    .size()
                    .x
                    + pad
            })
            .sum::<f32>()
            + 24.0
            + 8.0;
        let room = (MAX_WIDTH - 18.0 - 23.0 - buttons).max(120.0);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::simple(text, font, p.text_primary, room);
        job.halign = Align::Min;
        let galley = ui.painter().layout_job(job);
        ui.label(galley);
        for (label, action) in &toast.actions {
            let r = ui.add(
                egui::Button::new(
                    egui::RichText::new(label)
                        .text_style(egui::TextStyle::Button)
                        .color(p.accent_text),
                )
                .frame(false)
                .sense(Sense::click()),
            );
            if r.clicked() {
                actions.push(action.clone());
                closed.push(toast.id);
            }
        }
        if IconButton::new(icons::CLOSE, "Close")
            .glyph(12.0)
            .side(24.0)
            .show(ui)
            .clicked()
        {
            closed.push(toast.id);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_times_follow_ui_040() {
        assert_eq!(hold_time(ToastKind::Info), Some(4.0));
        assert_eq!(hold_time(ToastKind::Success), Some(3.0));
        assert_eq!(hold_time(ToastKind::Warning), Some(8.0));
        assert_eq!(hold_time(ToastKind::Error), None);
    }

    #[test]
    fn toasts_rise_in_and_fall_out() {
        assert_eq!(toast_offset(0.0, true), (TRAVEL, 0.0));
        assert_eq!(toast_offset(1.0, true), (0.0, 1.0));
        assert_eq!(toast_offset(0.0, false), (0.0, 1.0));
        assert_eq!(toast_offset(1.0, false), (TRAVEL, 0.0));
        // Ease-out arrives quickly, ease-in leaves slowly at first.
        assert!(toast_offset(0.5, true).0 < TRAVEL / 2.0);
        assert!(toast_offset(0.5, false).0 < TRAVEL / 2.0);
        // Arriving takes dur.slow, leaving the longer dur.exit.
        const { assert!(motion::EXIT > motion::SLOW) };
    }

    #[test]
    fn repeats_fold_and_at_most_three_run() {
        let mut toasts = Toasts::default();
        toasts.push(ToastKind::Info, "Snapping off");
        toasts.push(ToastKind::Info, "Snapping off");
        assert_eq!(toasts.messages(), ["Snapping off"]);
        assert_eq!(toasts.toasts[0].count, 2);
        for n in 0..4 {
            toasts.push(ToastKind::Error, format!("Error {n}"));
        }
        assert_eq!(toasts.toasts.len(), 5);
        // Five seconds later the info left; the errors stay.
        for _ in 0..60 {
            toasts.advance(0.1, motion::SLOW, motion::EXIT, None);
        }
        assert_eq!(
            toasts.messages(),
            ["Error 0", "Error 1", "Error 2", "Error 3"]
        );
        // The fourth waits: its clock hasn't started.
        assert_eq!(toasts.toasts[3].arrive, 0.0);
        toasts.close(toasts.toasts[0].id);
        for _ in 0..5 {
            toasts.advance(0.1, motion::SLOW, motion::EXIT, None);
        }
        assert_eq!(toasts.messages(), ["Error 1", "Error 2", "Error 3"]);
        assert!(toasts.toasts[2].arrive > 0.0);
        toasts.push(ToastKind::Info, "  ");
        assert_eq!(toasts.toasts.len(), 3, "blank messages are dropped");
    }

    #[test]
    fn pointing_at_a_toast_pauses_it() {
        let mut toasts = Toasts::default();
        toasts.push(ToastKind::Success, "Saved");
        let id = toasts.toasts[0].id;
        for _ in 0..50 {
            toasts.advance(0.1, 0.0, 0.0, Some(id));
        }
        assert_eq!(toasts.messages(), ["Saved"]);
        for _ in 0..35 {
            toasts.advance(0.1, 0.0, 0.0, None);
        }
        assert!(toasts.is_empty());
    }
}

//! Sheets (ui-concept §13.28): modal sheets (new project, confirmations,
//! About), the frame the export sheet and Preferences share, and the parts
//! they are made of: the 44 pt header with a centred title and a close
//! button, form rows with an 88 pt eyebrow label column and hairlines,
//! fact lists, choice chips and the three button kinds (primary in
//! `accent`, secondary on `bg.field`, ghost).
//!
//! - **Frame:** `bg.panel`, `r.xl`, a 1 pt `border.strong` outline and the
//!   `e3` shadow, over the `overlay.scrim`.
//! - **Width:** `min(want, window − 32)` ([`modal_width`]).
//! - **Footer:** a hairline, then the buttons right-aligned with the
//!   primary one last.

use eframe::egui::{
    self, Align, Align2, Color32, Frame, Id, Layout, Margin, Modal, Response, Sense, Shadow,
    Stroke, StrokeKind, UiBuilder, Vec2, WidgetInfo, WidgetType,
};

use super::icon_button::{IconButton, keyboard_focus};
use crate::theme::{self, Look, Type, fonts, icons};

/// A modal sheet's header height.
pub const HEADER: f32 = 44.0;
/// The export sheet's header height.
pub const LARGE_HEADER: f32 = 48.0;
/// A form row's label column.
pub const FORM_LABEL: f32 = 88.0;
/// A modal sheet's default width.
pub const MODAL_WIDTH: f32 = 600.0;
/// The large primary button's height (the export footer).
pub const PRIMARY_LARGE: f32 = 34.0;

/// A sheet's width in a window `window_w` wide: what it asks for, but 16 pt
/// of window on each side at least (never below 200 pt or the window).
pub fn modal_width(window_w: f32, want: f32) -> f32 {
    want.min(window_w - 32.0).max(200.0_f32.min(window_w))
}

/// The sheet's frame: `bg.panel`, `r.xl`, `border.strong`, `e3`.
pub fn sheet_frame(look: &Look) -> Frame {
    Frame::NONE
        .fill(look.palette.bg_panel)
        .corner_radius(theme::R_XL)
        .stroke(Stroke::new(1.0, look.palette.border_strong))
        .shadow(Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: Color32::from_black_alpha(look.palette.shadow_e3),
        })
}

/// The padding of a sheet's body and footer: 20 pt at the sides (16
/// Compact), 16 above and below.
pub fn body_margin(look: &Look) -> Margin {
    let side = (look.metrics.side_inset + 6.0) as i8;
    Margin {
        left: side,
        right: side,
        top: 16,
        bottom: 16,
    }
}

/// A modal sheet. Build with [`Sheet::new`], then [`Sheet::show`].
#[derive(Debug, Clone, Copy)]
pub struct Sheet<'a> {
    id: Id,
    title: &'a str,
    icon: Option<&'a str>,
    width: f32,
    header: f32,
    closable: bool,
    offset: f32,
    opacity: f32,
}

/// What a sheet reports.
#[derive(Debug)]
pub struct SheetResponse<R> {
    pub inner: R,
    /// The close button, `Esc` or a click on the scrim (when closable).
    pub close: bool,
}

impl<'a> Sheet<'a> {
    pub fn new(id: Id, title: &'a str) -> Sheet<'a> {
        Sheet {
            id,
            title,
            icon: None,
            width: MODAL_WIDTH,
            header: HEADER,
            closable: true,
            offset: 0.0,
            opacity: 1.0,
        }
    }

    /// The width asked for (see [`modal_width`]).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// A glyph before the title.
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The header's height (48 for the export sheet).
    pub fn header_height(mut self, height: f32) -> Self {
        self.header = height;
        self
    }

    /// Whether the close button shows and `Esc` or the scrim close it.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Moves the sheet down by `offset` and fades it to `opacity` (the
    /// export sheet's slide).
    pub fn slide(mut self, offset: f32, opacity: f32) -> Self {
        self.offset = offset;
        self.opacity = opacity;
        self
    }

    pub fn show<R>(
        self,
        ctx: &egui::Context,
        add: impl FnOnce(&mut SheetUi<'_>) -> R,
    ) -> SheetResponse<R> {
        let look = theme::look(ctx);
        let window = ctx.content_rect();
        let width = modal_width(window.width(), self.width);
        let area =
            Modal::default_area(self.id).anchor(Align2::CENTER_CENTER, Vec2::new(0.0, self.offset));
        let mut header_close = false;
        let modal = Modal::new(self.id)
            .area(area)
            .frame(sheet_frame(&look).multiply_with_opacity(self.opacity))
            .backdrop_color(look.palette.scrim.gamma_multiply(self.opacity))
            .show(ctx, |ui| {
                ui.multiply_opacity(self.opacity);
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                header_close = header(ui, self.title, self.icon, self.header, self.closable);
                let mut sheet = SheetUi { ui, look };
                add(&mut sheet)
            });
        let close = self.closable && (header_close || modal.should_close());
        SheetResponse {
            inner: modal.inner,
            close,
        }
    }
}

/// The inside of a sheet below its header.
pub struct SheetUi<'u> {
    pub ui: &'u mut egui::Ui,
    look: Look,
}

impl std::fmt::Debug for SheetUi<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SheetUi").finish_non_exhaustive()
    }
}

impl SheetUi<'_> {
    /// The body, with the sheet's padding.
    pub fn body<R>(&mut self, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let look = self.look;
        Frame::NONE
            .inner_margin(body_margin(&look))
            .show(self.ui, |ui| {
                ui.spacing_mut().item_spacing.y = look.metrics.row_gap;
                add(ui)
            })
            .inner
    }

    /// The body in a scroll area at most `max_height` tall.
    pub fn scroll_body<R>(&mut self, max_height: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let look = self.look;
        egui::ScrollArea::vertical()
            .max_height(max_height.max(80.0))
            .auto_shrink([false, true])
            .show(self.ui, |ui| {
                Frame::NONE
                    .inner_margin(body_margin(&look))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = look.metrics.row_gap;
                        add(ui)
                    })
                    .inner
            })
            .inner
    }

    /// The footer: a hairline, then `add` laid out right to left (so the
    /// primary button, added first, ends up last).
    pub fn footer<R>(&mut self, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let look = self.look;
        hairline(self.ui);
        let margin = Margin {
            top: 12,
            bottom: 14,
            ..body_margin(&look)
        };
        Frame::NONE
            .inner_margin(margin)
            .show(self.ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), add)
                    .inner
            })
            .inner
    }
}

/// A full-width 1 pt `border.subtle` hairline taking 1 pt of height.
pub fn hairline(ui: &mut egui::Ui) {
    let p = theme::look(ui.ctx()).palette;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.border_subtle);
}

/// A sheet header `height` tall: the title (`type.label`) centred, with an
/// optional glyph before it, the close button at the right and a hairline
/// below. Returns whether close was clicked.
pub fn header(
    ui: &mut egui::Ui,
    title: &str,
    icon: Option<&str>,
    height: f32,
    closable: bool,
) -> bool {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let font = Type::Label.style(ui.style()).resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), font, p.text_primary);
    let icon_w = if icon.is_some() { 16.0 + 8.0 } else { 0.0 };
    let total = icon_w + galley.size().x;
    let left = rect.center().x - total / 2.0;
    if let Some(icon) = icon {
        ui.painter().text(
            egui::pos2(left + 8.0, rect.center().y),
            Align2::CENTER_CENTER,
            icon,
            fonts::icon_font(ui.ctx(), 16.0, false),
            p.accent_text,
        );
    }
    ui.painter().galley(
        egui::pos2(left + icon_w, rect.center().y - galley.size().y / 2.0),
        galley,
        p.text_primary,
    );
    let mut close = false;
    if closable {
        let side = look.metrics.icon_chip;
        let at = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 10.0 - side / 2.0, rect.center().y),
            egui::vec2(side, side),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(at)
                .layout(Layout::left_to_right(Align::Center)),
        );
        close = IconButton::new(icons::CLOSE, "Close (Esc)")
            .glyph(14.0)
            .show(&mut child)
            .clicked();
    }
    let line = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - 1.0),
        rect.right_bottom(),
    );
    ui.painter().rect_filled(line, 0.0, p.border_subtle);
    close
}

/// `text` in capitals as a form eyebrow: `type.micro`, +0.8 pt tracking,
/// `text.tertiary`.
pub fn eyebrow(ui: &egui::Ui, text: &str) -> egui::text::LayoutJob {
    let p = theme::look(ui.ctx()).palette;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: Type::Micro.style(ui.style()).resolve(ui.style()),
            color: p.text_tertiary,
            extra_letter_spacing: 0.8,
            ..egui::TextFormat::default()
        },
    );
    job
}

/// A form row: the eyebrow `label` in an 88 pt column, `add` beside it,
/// 10 pt above and below, and a hairline under it unless it is the `last`.
pub fn form_row<R>(
    ui: &mut egui::Ui,
    label: &str,
    last: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let job = eyebrow(ui, label);
    let width = ui.available_width();
    ui.add_space(10.0);
    let inner = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let galley = ui.painter().layout_job(job);
            let control_h = theme::look(ui.ctx()).metrics.control_height;
            let (cell, _) =
                ui.allocate_exact_size(egui::vec2(FORM_LABEL, control_h), Sense::hover());
            ui.painter().galley(
                egui::pos2(cell.left(), cell.center().y - galley.size().y / 2.0),
                galley,
                Color32::PLACEHOLDER,
            );
            ui.allocate_ui_with_layout(
                egui::vec2(width - FORM_LABEL, control_h),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                    add(ui)
                },
            )
            .inner
        })
        .inner;
    ui.add_space(10.0);
    if !last {
        hairline(ui);
    }
    inner
}

/// The kind of a text button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// `accent` fill, `text.on-accent` label: the one action a sheet is for.
    Primary,
    /// `bg.field` fill.
    Secondary,
    /// No fill until hovered.
    Ghost,
    /// Destructive: a `danger` label on `bg.field`, a `danger` tint on hover.
    Danger,
}

/// The fills and label colour of a button of `kind`, for `t` of hover.
fn button_colours(look: &Look, kind: ButtonKind, enabled: bool, t: f32) -> (Color32, Color32) {
    let p = look.palette;
    if !enabled {
        let fill = match kind {
            ButtonKind::Ghost => Color32::TRANSPARENT,
            _ => p.bg_field,
        };
        return (fill, p.text_disabled);
    }
    match kind {
        ButtonKind::Primary => (p.accent.lerp_to_gamma(p.accent_hover, t), p.text_on_accent),
        ButtonKind::Secondary => (p.bg_field.lerp_to_gamma(p.bg_hover, t), p.text_primary),
        ButtonKind::Ghost => (
            Color32::TRANSPARENT.lerp_to_gamma(p.bg_field, t),
            p.text_secondary.lerp_to_gamma(p.text_primary, t),
        ),
        ButtonKind::Danger => (p.bg_field.lerp_to_gamma(p.bg_hover, t), p.danger),
    }
}

/// A text button: `label` with an optional glyph before it, at least
/// `min_width` wide and `height` tall (`None`: the control height),
/// `r.md`. Disabled buttons still answer hover, for their reason.
pub fn button(
    ui: &mut egui::Ui,
    kind: ButtonKind,
    icon: Option<&str>,
    label: &str,
    min_width: f32,
    height: Option<f32>,
    enabled: bool,
) -> Response {
    let look = theme::look(ui.ctx());
    let enabled = enabled && ui.is_enabled();
    let height = height.unwrap_or(look.metrics.control_height);
    let font = if height >= PRIMARY_LARGE {
        Type::Label.style(ui.style()).resolve(ui.style())
    } else {
        egui::TextStyle::Button.resolve(ui.style())
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let glyph = font.size + 2.0;
    let icon_w = if icon.is_some() { glyph + 6.0 } else { 0.0 };
    let pad = if height >= PRIMARY_LARGE { 16.0 } else { 12.0 };
    let width = (icon_w + galley.size().x + 2.0 * pad).max(min_width);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), sense);
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            enabled && response.hovered(),
            ui.style().animation_time,
        );
        let (mut fill, ink) = button_colours(&look, kind, enabled, t);
        if enabled && response.is_pointer_button_down_on() && kind != ButtonKind::Primary {
            fill = look.palette.bg_active;
        }
        ui.painter().rect_filled(rect, theme::R_MD, fill);
        if kind == ButtonKind::Danger && enabled && t > 0.0 {
            let edge = look.palette.danger.gamma_multiply(t);
            ui.painter().rect_stroke(
                rect,
                theme::R_MD,
                Stroke::new(1.0, edge),
                StrokeKind::Inside,
            );
        } else {
            super::control_outline(ui, rect, theme::R_MD);
        }
        let total = icon_w + galley.size().x;
        let left = rect.center().x - total / 2.0;
        if let Some(icon) = icon {
            ui.painter().text(
                egui::pos2(left + glyph / 2.0, rect.center().y),
                Align2::CENTER_CENTER,
                icon,
                fonts::icon_font(ui.ctx(), glyph, false),
                ink,
            );
        }
        ui.painter().galley(
            egui::pos2(left + icon_w, rect.center().y - galley.size().y / 2.0),
            galley,
            ink,
        );
        if keyboard_focus(ui, &response) {
            super::focus_ring(ui, rect, theme::R_MD);
        }
    }
    let name = label.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &name));
    response
}

/// A choice chip (§13.9) in a sheet: a pill the control height tall with an
/// optional aspect glyph (`(w, h)` drawn as a small frame) before the
/// label. Rest `bg.field`; hover `bg.hover`; chosen `overlay.selection`
/// with an `accent` outline and `accent.text`.
pub fn choice_chip(
    ui: &mut egui::Ui,
    label: &str,
    aspect: Option<(f32, f32)>,
    selected: bool,
) -> Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let font = Type::Caption.style(ui.style()).resolve(ui.style());
    let ink = if selected {
        p.accent_text
    } else {
        p.text_secondary
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let glyph_w = if aspect.is_some() { 14.0 + 6.0 } else { 0.0 };
    let height = look.metrics.control_height;
    let size = egui::vec2(glyph_w + galley.size().x + 24.0, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered(),
            ui.style().animation_time,
        );
        let radius = (height / 2.0).round() as u8;
        let fill = if selected {
            p.overlay_selection()
        } else {
            p.bg_field.lerp_to_gamma(p.bg_hover, t)
        };
        ui.painter().rect_filled(rect, radius, fill);
        if selected {
            ui.painter()
                .rect_stroke(rect, radius, Stroke::new(1.0, p.accent), StrokeKind::Inside);
        } else if look.kind == theme::ThemeKind::HighContrast {
            super::control_outline(ui, rect, radius);
        } else {
            // A hairline, so a chip still reads on a `bg.field` note box.
            let edge = p.border_subtle.lerp_to_gamma(p.border_strong, t);
            ui.painter()
                .rect_stroke(rect, radius, Stroke::new(1.0, edge), StrokeKind::Inside);
        }
        let ink = if selected {
            ink
        } else {
            ink.lerp_to_gamma(p.text_primary, t)
        };
        let mut x = rect.left() + 12.0;
        if let Some((w, h)) = aspect {
            let glyph = aspect_glyph(egui::pos2(x + 7.0, rect.center().y), w, h, 12.0);
            ui.painter()
                .rect_stroke(glyph, 1.5, Stroke::new(1.5, ink), StrokeKind::Inside);
            x += glyph_w;
        }
        ui.painter().galley(
            egui::pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            ink,
        );
        if keyboard_focus(ui, &response) {
            super::focus_ring(ui, rect, radius);
        }
    }
    let name = label.to_owned();
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &name));
    response
}

/// The frame of an aspect glyph: a `w` : `h` rectangle fitting a `side`
/// square centred on `centre` (never thinner than 4 pt).
pub fn aspect_glyph(centre: egui::Pos2, w: f32, h: f32, side: f32) -> egui::Rect {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let size = if w >= h {
        egui::vec2(side, (side * h / w).max(4.0))
    } else {
        egui::vec2((side * w / h).max(4.0), side)
    };
    egui::Rect::from_center_size(centre, size)
}

/// A fact list (§13.28's summary): rows on `bg.card` separated by
/// hairlines, labels in `text.secondary`, values in `text.primary`, cut
/// with "…" when long.
pub fn fact_list(ui: &mut egui::Ui, facts: &[(&str, String)]) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let row_h = 30.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, row_h * facts.len() as f32),
        Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(rect, theme::R_MD, p.bg_card);
    super::control_outline(ui, rect, theme::R_MD);
    let label_font = egui::TextStyle::Small.resolve(ui.style());
    let value_font = egui::TextStyle::Small.resolve(ui.style());
    for (i, (label, value)) in facts.iter().enumerate() {
        let top = rect.top() + row_h * i as f32;
        let row = egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(width, row_h));
        if i > 0 {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(row.min, egui::vec2(width, 1.0)),
                0.0,
                p.border_subtle,
            );
        }
        let label_g =
            ui.painter()
                .layout_no_wrap((*label).to_owned(), label_font.clone(), p.text_secondary);
        let label_w = label_g.size().x;
        ui.painter().galley(
            egui::pos2(row.left() + 10.0, row.center().y - label_g.size().y / 2.0),
            label_g,
            p.text_secondary,
        );
        let room = (width - label_w - 32.0).max(16.0);
        let value_g =
            super::side_list::one_line(ui, value, value_font.clone(), p.text_primary, room);
        ui.painter().galley(
            egui::pos2(
                row.right() - 10.0 - value_g.size().x,
                row.center().y - value_g.size().y / 2.0,
            ),
            value_g,
            p.text_primary,
        );
    }
}

/// A section title in a sheet: `type.label` with an optional help badge
/// after it, `trailing` at the far end, 32 pt tall.
pub fn section_title<R>(
    ui: &mut egui::Ui,
    id: Id,
    title: &str,
    help: Option<&str>,
    trailing: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let p = theme::look(ui.ctx()).palette;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, 32.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, 32.0));
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(
                egui::RichText::new(title)
                    .text_style(Type::Label.style(ui.style()))
                    .color(p.text_primary),
            );
            if let Some(help) = help {
                super::help::help_badge(ui, id.with("help"), help);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), trailing)
                .inner
        },
    )
    .inner
}

/// A 4 pt progress bar across `ui`: a `bg.well` track and an `accent`
/// fill (`success` once `done`).
pub fn progress_bar(ui: &mut egui::Ui, fraction: f32, done: bool) {
    let p = theme::look(ui.ctx()).palette;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, p.bg_well);
    let fill = if done { p.success } else { p.accent };
    let w = rect.width() * fraction.clamp(0.0, 1.0);
    if w > 0.0 {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(w, rect.height())),
            2.0,
            fill,
        );
    }
}

/// A note box: `bg.field` with a 3 pt bar in `colour` along the left edge,
/// `icon` in `colour` and the text in `text.primary`, wrapping.
pub fn note_box<R>(
    ui: &mut egui::Ui,
    icon: &str,
    colour: Color32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let p = theme::look(ui.ctx()).palette;
    let response = Frame::NONE
        .fill(p.bg_field)
        .corner_radius(theme::R_MD)
        .inner_margin(Margin {
            left: 12,
            right: 10,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    egui::RichText::new(icon)
                        .font(fonts::icon_font(ui.ctx(), 14.0, false))
                        .color(colour),
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    add(ui)
                })
                .inner
            })
            .inner
        });
    let r = response.response.rect;
    let bar = egui::Rect::from_min_size(r.min, egui::vec2(3.0, r.height()));
    ui.painter().rect_filled(
        bar,
        egui::CornerRadius {
            nw: theme::R_MD,
            sw: theme::R_MD,
            ne: 0,
            se: 0,
        },
        colour,
    );
    response.inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_width_keeps_16_pt_of_window_each_side() {
        assert_eq!(modal_width(1440.0, 600.0), 600.0);
        assert_eq!(modal_width(500.0, 600.0), 468.0);
        assert_eq!(modal_width(150.0, 600.0), 150.0);
        assert_eq!(modal_width(220.0, 600.0), 200.0);
    }

    #[test]
    fn aspect_glyphs_fit_their_square() {
        let c = egui::pos2(10.0, 10.0);
        let wide = aspect_glyph(c, 16.0, 9.0, 12.0);
        assert_eq!(wide.width(), 12.0);
        assert!((wide.height() - 6.75).abs() < 1e-4);
        let tall = aspect_glyph(c, 9.0, 16.0, 12.0);
        assert_eq!(tall.height(), 12.0);
        let cinema = aspect_glyph(c, 64.0, 27.0, 12.0);
        assert!(cinema.height() >= 4.0);
        assert_eq!(
            aspect_glyph(c, 1.0, 1.0, 12.0).size(),
            egui::vec2(12.0, 12.0)
        );
        assert_eq!(wide.center(), c);
    }

    #[test]
    fn primary_buttons_use_the_accent_and_disabled_ones_do_not() {
        let look = Look::default();
        let p = look.palette;
        assert_eq!(
            button_colours(&look, ButtonKind::Primary, true, 0.0),
            (p.accent, p.text_on_accent)
        );
        assert_eq!(
            button_colours(&look, ButtonKind::Primary, true, 1.0).0,
            p.accent_hover
        );
        assert_eq!(
            button_colours(&look, ButtonKind::Primary, false, 1.0),
            (p.bg_field, p.text_disabled)
        );
        assert_eq!(
            button_colours(&look, ButtonKind::Ghost, true, 0.0).0,
            Color32::TRANSPARENT
        );
    }
}

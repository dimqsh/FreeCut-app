//! FreeCut's own widgets (ui-concept §13, UI-009): the property section
//! and row, the number field with its unit segment, the slider with field,
//! the segmented control, the select, the colour field, the help and reset
//! badges, the toggle switch, the keyframe navigator and the empty state,
//! with the panel shell and the asset browser's parts. They paint with the
//! theme tokens ([`crate::theme::look`]) and take explicit ids, so their
//! state survives selection changes (ui-concept §16.3).

pub mod card;
pub mod color_wheel;
pub mod colour;
pub mod curve_editor;
pub mod empty;
pub mod field;
pub mod gradient;
pub mod help;
pub mod icon_button;
pub mod import_button;
pub mod keyframe;
pub mod mask_shape;
pub mod panel;
pub mod row;
pub mod segmented;
pub mod select;
pub mod sheet;
pub mod shelf_tab;
pub mod side_list;
pub mod slider;
pub mod stepper;
pub mod thin_scrollbar;
pub mod tile;
pub mod timecode;
pub mod timing_strip;
pub mod toggle;

use eframe::egui::{self, Align2, Rect, Response, Sense, Stroke, StrokeKind};

use crate::theme::{self, fonts};

/// The focus ring: `accent`, 2 pt (3 pt in High contrast), 1 pt outside
/// `rect` (ui-concept §15).
pub fn focus_ring(ui: &egui::Ui, rect: Rect, radius: u8) {
    let p = theme::look(ui.ctx()).palette;
    ui.painter().rect_stroke(
        rect.expand(1.0),
        radius,
        Stroke::new(p.focus_width, p.accent),
        StrokeKind::Outside,
    );
}

/// High contrast's border around a frameless control (UI-014: "visible
/// borders on every control"); nothing in the other themes.
pub fn control_outline(ui: &egui::Ui, rect: Rect, radius: u8) {
    let border = theme::look(ui.ctx()).control_border();
    if border != Stroke::NONE && ui.is_rect_visible(rect) {
        ui.painter()
            .rect_stroke(rect, radius, border, StrokeKind::Inside);
    }
}

/// A label of `text` followed by `icon` in the icon font, for buttons and
/// selects. Icons never go through the text font: Inter has glyphs of its
/// own at some of Phosphor's private-use codepoints. Both parts take the
/// widget's text colour.
pub fn text_then_icon(ui: &egui::Ui, text: &str, icon: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font = egui::TextStyle::Button.resolve(ui.style());
    let size = font.size;
    job.append(
        text,
        0.0,
        egui::TextFormat::simple(font, egui::Color32::PLACEHOLDER),
    );
    job.append(
        icon,
        6.0,
        egui::TextFormat::simple(
            fonts::icon_font(ui.ctx(), size, false),
            egui::Color32::PLACEHOLDER,
        ),
    );
    job
}

/// `icon` in the icon font, then `text`, for menu items and buttons
/// (icons never go through the text font; see [`text_then_icon`]).
pub fn icon_then_text(ui: &egui::Ui, icon: &str, text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font = egui::TextStyle::Button.resolve(ui.style());
    let size = font.size;
    job.append(
        icon,
        0.0,
        egui::TextFormat {
            valign: egui::Align::Center,
            ..egui::TextFormat::simple(
                fonts::icon_font(ui.ctx(), size + 1.0, false),
                egui::Color32::PLACEHOLDER,
            )
        },
    );
    job.append(
        text,
        8.0,
        egui::TextFormat {
            valign: egui::Align::Center,
            ..egui::TextFormat::simple(font, egui::Color32::PLACEHOLDER)
        },
    );
    job
}

/// A square icon button (`size` pt, hit area at least 24 pt) with a
/// tooltip, painted in the context's text colour. Returns its response.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, size: f32, tooltip: &str) -> Response {
    let side = size.max(24.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), Sense::click());
    let look = theme::look(ui.ctx());
    let p = look.palette;
    if ui.is_rect_visible(rect) {
        let color = if !ui.is_enabled() {
            p.text_disabled
        } else if response.hovered() || response.has_focus() {
            p.text_primary
        } else {
            p.text_secondary
        };
        if response.hovered() && ui.is_enabled() {
            ui.painter().rect_filled(rect, theme::R_SM, p.bg_hover);
        }
        control_outline(ui, rect, theme::R_SM);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            fonts::icon_font(ui.ctx(), size * 0.75, false),
            color,
        );
        if response.has_focus() {
            focus_ring(ui, rect, theme::R_SM);
        }
    }
    let label = tooltip.to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    response.on_hover_text(tooltip)
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Context, Pos2, RawInput, Shape};

    use super::segmented::{Segment, Segmented};
    use super::*;
    use crate::theme::{Density, TOKENS, ThemeKind};

    /// The rectangle strokes one frame of `draw` paints under `kind`.
    fn strokes(kind: ThemeKind, draw: impl FnMut(&mut egui::Ui)) -> Vec<Stroke> {
        let ctx = Context::default();
        theme::apply(&ctx, &TOKENS, kind, Density::Comfortable);
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))),
            ..RawInput::default()
        };
        let mut draw = draw;
        let mut output = ctx.run_ui(input, |ui| draw(ui));
        // Nothing renders here: drop the font atlas uploads.
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) => Some(r.stroke),
                _ => None,
            })
            .collect()
    }

    /// UI-014: in High contrast the frameless controls (icon buttons, the
    /// segmented track) show a `border.control` outline; not elsewhere.
    #[test]
    fn frameless_controls_get_a_border_in_high_contrast_only() {
        let draw = |ui: &mut egui::Ui| {
            icon_button(ui, crate::theme::icons::DELETE, 24.0, "Delete");
            let items = [Segment::new(None, "A"), Segment::new(None, "B")];
            Segmented::new(egui::Id::new("seg"), &items, 0).show(ui);
        };
        let border = Stroke::new(1.0, TOKENS.high_contrast.border_control);
        let hc = strokes(ThemeKind::HighContrast, draw);
        assert!(hc.iter().filter(|s| **s == border).count() >= 2, "{hc:?}");
        let dark = strokes(ThemeKind::Dark, draw);
        let dark_border = Stroke::new(1.0, TOKENS.dark.border_control);
        assert!(!dark.contains(&dark_border), "{dark:?}");
    }
}

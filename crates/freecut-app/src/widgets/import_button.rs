//! The Import button (ui-concept §13.27): 28 pt tall, `r.md`, a 1 pt
//! dashed `border.strong` outline with solid corners that says "drop files
//! here"; accent outline and `bg.field` on hover; dimmed and not clickable
//! with "Importing… (n)" while imports run. Where the toolbar is too narrow
//! for its label it shows the glyph alone, the label in its tooltip.

use eframe::egui::{self, Align2, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2};

use super::focus_ring;
use super::icon_button::keyboard_focus;
use crate::theme::{self, fonts, icons};

const SIDE_PADDING: f32 = 16.0;
const GLYPH: f32 = 14.0;
const GLYPH_GAP: f32 = 8.0;
const DASH: f32 = 4.0;

/// The label: `label`, or "Importing… (n)" while `busy` imports run.
pub fn import_label(label: &str, busy: usize) -> String {
    if busy > 0 {
        format!("Importing… ({busy})")
    } else {
        label.to_owned()
    }
}

/// The straight runs of a rounded rectangle's outline (top, right,
/// bottom, left), between its corner arcs: the parts that are dashed.
pub fn straight_runs(rect: Rect, r: f32) -> [[Pos2; 2]; 4] {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    [
        [
            rect.left_top() + Vec2::new(r, 0.0),
            rect.right_top() - Vec2::new(r, 0.0),
        ],
        [
            rect.right_top() + Vec2::new(0.0, r),
            rect.right_bottom() - Vec2::new(0.0, r),
        ],
        [
            rect.right_bottom() - Vec2::new(r, 0.0),
            rect.left_bottom() + Vec2::new(r, 0.0),
        ],
        [
            rect.left_bottom() - Vec2::new(0.0, r),
            rect.left_top() + Vec2::new(0.0, r),
        ],
    ]
}

/// A rounded rectangle outline with dashed sides and solid corners.
fn dashed_outline(painter: &egui::Painter, rect: Rect, r: f32, stroke: Stroke) {
    for run in straight_runs(rect, r) {
        painter.extend(Shape::dashed_line(&run, stroke, DASH, DASH));
    }
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let corners = [
        (rect.left_top() + Vec2::new(r, r), std::f32::consts::PI),
        (
            rect.right_top() + Vec2::new(-r, r),
            1.5 * std::f32::consts::PI,
        ),
        (rect.right_bottom() + Vec2::new(-r, -r), 0.0),
        (
            rect.left_bottom() + Vec2::new(r, -r),
            0.5 * std::f32::consts::PI,
        ),
    ];
    for (centre, start) in corners {
        let points: Vec<Pos2> = (0..=6)
            .map(|i| {
                let a = start + std::f32::consts::FRAC_PI_2 * i as f32 / 6.0;
                centre + r * Vec2::new(a.cos(), a.sin())
            })
            .collect();
        painter.add(Shape::line(points, stroke));
    }
}

/// The Import button's width with a label `label_w` wide, or with the
/// glyph alone (`icon_only`): then square-ish, the glyph with 8 pt beside.
pub fn import_width(label_w: f32, icon_only: bool) -> f32 {
    if icon_only {
        GLYPH + 2.0 * ICON_PADDING
    } else {
        SIDE_PADDING * 2.0 + GLYPH + GLYPH_GAP + label_w
    }
}

/// The icon-only button's padding either side of the glyph.
const ICON_PADDING: f32 = 8.0;

/// The width of the Import button's label `label` (or its busy label).
pub fn import_label_width(ui: &egui::Ui, label: &str, busy: usize) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(import_label(label, busy), font, egui::Color32::WHITE)
        .size()
        .x
}

/// The Import button. Returns its response (never clicked while busy).
/// `icon_only` draws the glyph alone (a narrow toolbar).
pub fn import_button(ui: &mut egui::Ui, label: &str, busy: usize, icon_only: bool) -> Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let text = import_label(label, busy);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.clone(), font, p.text_secondary);
    let size = Vec2::new(
        import_width(galley.size().x, icon_only),
        look.metrics.control_height,
    );
    let sense = if busy > 0 {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let mut painter = ui.painter().clone();
        if busy > 0 {
            painter.multiply_opacity(0.4);
        }
        let hot = busy == 0 && response.hovered();
        let t = ui.ctx().animate_bool_with_time(
            response.id.with("hot"),
            hot,
            ui.style().animation_time,
        );
        painter.rect_filled(rect, theme::R_MD, p.bg_panel);
        if t > 0.0 {
            painter.rect_filled(rect, theme::R_MD, p.bg_field.gamma_multiply(t));
        }
        let outline = if hot { p.accent } else { p.border_strong };
        dashed_outline(
            &painter,
            rect.shrink(0.5),
            f32::from(theme::R_MD),
            Stroke::new(1.0, outline),
        );
        let ink = if hot {
            p.text_primary
        } else {
            p.text_secondary
        };
        let x = if icon_only {
            rect.left() + ICON_PADDING
        } else {
            rect.left() + SIDE_PADDING
        };
        painter.text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            icons::IMPORT,
            fonts::icon_font(ui.ctx(), GLYPH, false),
            ink,
        );
        if !icon_only {
            painter.galley(
                Pos2::new(
                    x + GLYPH + GLYPH_GAP,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                ink,
            );
        }
        // High contrast needs no extra outline: the dashes are white.
        if keyboard_focus(ui, &response) {
            focus_ring(ui, rect, theme::R_MD);
        }
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, busy == 0, &text));
    if icon_only && busy > 0 {
        return response.on_hover_text(text);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_imports_change_the_label() {
        assert_eq!(import_label("Import", 0), "Import");
        assert_eq!(import_label("Import", 3), "Importing… (3)");
    }

    #[test]
    fn dashes_stop_where_the_corners_start() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 28.0));
        let runs = straight_runs(rect, 7.0);
        assert_eq!(runs[0], [Pos2::new(7.0, 0.0), Pos2::new(93.0, 0.0)]);
        assert_eq!(runs[1], [Pos2::new(100.0, 7.0), Pos2::new(100.0, 21.0)]);
        // A radius larger than half the height is capped.
        let runs = straight_runs(rect, 40.0);
        assert_eq!(runs[1][0], runs[1][1]);
    }
}

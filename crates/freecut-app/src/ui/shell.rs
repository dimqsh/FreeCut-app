//! The window shell around the panels (ui-concept §3.1–§3.5, rev. 2): the
//! slim top bar on the ground, the save state next to the project name,
//! the accent Export button, the status line, and the floating panels'
//! gutters with their resize lines.
//!
//! The panels themselves are egui `Panel`s whose frame is the ground with
//! gutter margins ([`crate::widgets::panel::ground_frame`]); inside each,
//! [`crate::widgets::panel::show_panel`] draws the rounded card. The stock
//! resize line is silenced ([`quiet_stock_resize_lines`]) and replaced by
//! the gutter's 2 pt accent line ([`gutter_feedback`]).

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Rangef, Rect, Response, Sense, Stroke, Vec2,
};

use crate::theme::{self, Look, Palette, fonts, icons};
use crate::widgets::panel;

/// Room kept free on the left of the top bar for macOS's window buttons,
/// where the content runs under a transparent title bar (§3.5).
pub const MAC_WINDOW_BUTTONS: f32 = 76.0;

/// The top bar's left inset: the window buttons' room on macOS, a small
/// margin elsewhere (native decorations stay there for now).
pub fn top_bar_left_inset(macos: bool) -> f32 {
    if macos { MAC_WINDOW_BUTTONS } else { 8.0 }
}

/// Pressing on empty top-bar space and moving more than 4 pt starts a
/// window drag (§3.5); a smaller wobble is still a click.
pub fn drag_should_start(delta: Vec2) -> bool {
    delta.length() > 4.0
}

/// PM-012's save states, as the top bar shows them next to the project
/// name (UI-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    Saved,
    Saving,
    Unsaved,
    Failed,
}

impl SaveState {
    /// The state of a project: `None` for a new project with nothing in
    /// it to save yet.
    pub fn of(saving: bool, dirty: bool, failed: bool, has_file: bool) -> Option<SaveState> {
        if saving {
            Some(SaveState::Saving)
        } else if failed {
            Some(SaveState::Failed)
        } else if dirty {
            Some(SaveState::Unsaved)
        } else if has_file {
            Some(SaveState::Saved)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SaveState::Saved => "Saved",
            SaveState::Saving => "Saving…",
            SaveState::Unsaved => "Unsaved changes",
            SaveState::Failed => "Save failed — Retry",
        }
    }

    /// `text.tertiary` for Saved / Saving…, `warning` for unsaved
    /// changes, `danger` for a failed save (§3.5).
    pub fn colour(self, p: &Palette) -> Color32 {
        match self {
            SaveState::Saved | SaveState::Saving => p.text_tertiary,
            SaveState::Unsaved => p.warning,
            SaveState::Failed => p.danger,
        }
    }
}

/// `name` cut in the middle with an ellipsis when it has more than `max`
/// characters (UI-012: very long project names).
pub fn middle_ellipsis(name: &str, max: usize) -> String {
    let count = name.chars().count();
    if count <= max || max < 3 {
        return name.to_owned();
    }
    let keep = max - 1;
    let head = keep.div_ceil(2);
    let tail = keep - head;
    let start: String = name.chars().take(head).collect();
    let end: String = name.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

/// The project name and its save state (§3.5). A failed save is a button
/// that retries; returns whether it was clicked.
pub fn project_name(
    ui: &mut egui::Ui,
    name: &str,
    path: Option<&str>,
    state: Option<SaveState>,
    failure: Option<&str>,
) -> bool {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    // About 6.5 pt per character of 12 pt Inter; the state needs about
    // 110 pt beside the name.
    let max_chars = ((ui.available_width() - 110.0) / 6.5).max(8.0) as usize;
    let shown = middle_ellipsis(name, max_chars.min(48));
    let label = ui.add(
        egui::Label::new(
            egui::RichText::new(shown)
                .text_style(egui::TextStyle::Body)
                .color(p.text_secondary),
        )
        .selectable(false),
    );
    label.on_hover_text(path.unwrap_or(name));
    let Some(state) = state else { return false };
    ui.add_space(6.0 - ui.spacing().item_spacing.x);
    let text = egui::RichText::new(state.label())
        .text_style(egui::TextStyle::Small)
        .color(state.colour(&p));
    if state == SaveState::Failed {
        let response = ui
            .add(egui::Button::new(text).frame_when_inactive(false))
            .on_hover_text(failure.unwrap_or("The last save failed: click to try again"));
        response.clicked()
    } else {
        ui.add(egui::Label::new(text).selectable(false));
        false
    }
}

/// The accent Export button (§13.18's filled variant): 26 pt tall,
/// `r.sm`, `accent` fill fading to `accent.hover`, the export glyph and
/// the label in `text.on-accent`. Disabled at 40 %.
pub fn export_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let height = look.metrics.icon_chip;
    let font = egui::FontId::new(
        12.0,
        theme::Type::Label
            .style(ui.style())
            .resolve(ui.style())
            .family,
    );
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, p.text_on_accent);
    let width = 10.0 + 12.0 + 6.0 + galley.size().x + 10.0;
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
        let mut fill = p.accent.lerp_to_gamma(p.accent_hover, t);
        let mut ink = p.text_on_accent;
        if !enabled {
            fill = fill.gamma_multiply(0.4);
            ink = ink.gamma_multiply(0.6);
        }
        let painter = ui.painter();
        painter.rect_filled(rect, theme::R_SM, fill);
        let glyph_x = rect.left() + 10.0 + 6.0;
        painter.text(
            egui::pos2(glyph_x, rect.center().y),
            Align2::CENTER_CENTER,
            icons::EXPORT,
            fonts::icon_font(ui.ctx(), 12.0, false),
            ink,
        );
        painter.galley_with_override_text_color(
            egui::pos2(
                rect.left() + 10.0 + 12.0 + 6.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            ink,
        );
        if crate::widgets::icon_button::keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(ui, rect, theme::R_SM);
        }
    }
    let text = label.to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &text));
    response
}

/// Silences egui's own resize line on the panels shown next (the gutter
/// draws its own, [`gutter_feedback`]), returning the style to restore
/// inside each panel's body and after the panels.
pub fn quiet_stock_resize_lines(ui: &mut egui::Ui) -> Arc<egui::Style> {
    let saved = Arc::clone(ui.style());
    let w = &mut ui.style_mut().visuals.widgets;
    w.hovered.fg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::NONE;
    saved
}

/// The edge of a panel that moves when it is resized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// A left panel's right edge.
    Right,
    /// A right panel's left edge.
    Left,
    /// A bottom panel's top edge.
    Top,
}

/// Where the gutter line of a panel whose outer rect (ground margin
/// included) is `outer` and whose card is `card` goes: `(vertical,
/// across, along)`.
pub fn gutter_geometry(edge: Edge, outer: Rect, card: Rect) -> (bool, f32, Rangef) {
    match edge {
        Edge::Right => (true, outer.right(), card.y_range()),
        Edge::Left => (true, outer.left(), card.y_range()),
        Edge::Top => (false, outer.top(), card.x_range()),
    }
}

/// The resize feedback of the gutter beside the panel `panel_id`: the
/// resize cursor, the 2 pt accent line fading in on hover (`dur.fast`) and
/// staying while dragging (§13.17), and a double-click that resets the
/// split to its default (UI-002).
pub fn gutter_feedback(ui: &egui::Ui, panel_id: Id, edge: Edge, outer: Rect, card: Rect) {
    let ctx = ui.ctx();
    // egui's panel registers its resize handle under this id.
    let handle = ctx.read_response(panel_id.with("__resize"));
    let (hovered, dragged, double) = handle.as_ref().map_or((false, false, false), |r| {
        (r.hovered(), r.dragged(), r.double_clicked())
    });
    if double {
        ctx.data_mut(|d| d.remove::<egui::containers::panel::PanelState>(panel_id));
        ctx.request_repaint();
    }
    if hovered || dragged {
        ctx.set_cursor_icon(match edge {
            Edge::Top => CursorIcon::ResizeVertical,
            Edge::Left | Edge::Right => CursorIcon::ResizeHorizontal,
        });
    }
    let t = ctx.animate_bool_with_time(
        panel_id.with("gutter-line"),
        hovered || dragged,
        ui.style().animation_time,
    );
    let look: Look = theme::look(ctx);
    let (vertical, across, along) = gutter_geometry(edge, outer, card);
    panel::gutter_line(ui.painter(), vertical, across, along, &look, t);
}

/// Whether the widget with keyboard focus lies inside `card` (the
/// focused place, which shows the focus line).
pub fn holds_focus(ctx: &egui::Context, card: Rect) -> bool {
    ctx.memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| card.contains_rect(r.rect))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::TOKENS;

    #[test]
    fn save_states_follow_pm_012() {
        assert_eq!(
            SaveState::of(true, true, true, true),
            Some(SaveState::Saving)
        );
        assert_eq!(
            SaveState::of(false, true, true, true),
            Some(SaveState::Failed),
            "a failed save stays red until the next successful one"
        );
        assert_eq!(
            SaveState::of(false, true, false, false),
            Some(SaveState::Unsaved)
        );
        assert_eq!(
            SaveState::of(false, false, false, true),
            Some(SaveState::Saved)
        );
        assert_eq!(SaveState::of(false, false, false, false), None);
        let p = TOKENS.dark;
        assert_eq!(SaveState::Saved.colour(&p), p.text_tertiary);
        assert_eq!(SaveState::Saving.colour(&p), p.text_tertiary);
        assert_eq!(SaveState::Unsaved.colour(&p), p.warning);
        assert_eq!(SaveState::Failed.colour(&p), p.danger);
    }

    #[test]
    fn the_top_bar_keeps_room_for_the_window_buttons_and_drags_after_4_pt() {
        assert_eq!(top_bar_left_inset(true), 76.0);
        assert_eq!(top_bar_left_inset(false), 8.0);
        assert!(!drag_should_start(Vec2::new(3.0, 2.0)));
        assert!(drag_should_start(Vec2::new(4.0, 1.0)));
    }

    #[test]
    fn long_names_are_cut_in_the_middle() {
        assert_eq!(middle_ellipsis("Beach trip", 20), "Beach trip");
        let cut = middle_ellipsis("A very long project name for the summer", 15);
        assert_eq!(cut.chars().count(), 15);
        assert!(
            cut.starts_with("A very ") && cut.ends_with("summer"),
            "{cut}"
        );
        assert!(cut.contains('…'));
    }

    #[test]
    fn the_gutter_line_runs_along_the_moving_edge() {
        let outer = Rect::from_min_max(egui::pos2(0.0, 40.0), egui::pos2(300.0, 600.0));
        let card = Rect::from_min_max(egui::pos2(8.0, 40.0), egui::pos2(296.0, 596.0));
        assert_eq!(
            gutter_geometry(Edge::Right, outer, card),
            (true, 300.0, card.y_range())
        );
        assert_eq!(
            gutter_geometry(Edge::Top, outer, card),
            (false, 40.0, card.x_range())
        );
    }
}

//! The select (ui-concept §13.20): FreeCut's own drop-down, replacing
//! egui's `ComboBox` in the panels.
//!
//! - **Trigger:** 28 pt tall (24 Compact), `r.md`, `bg.field` (`bg.hover`
//!   while hovered or open), the current option in `type.body` 8 pt from
//!   the left and a 12 pt caret 6 pt from the right. It is as wide as its
//!   widest option ([`select_natural_width`]), so it doesn't change width
//!   when the choice changes; in a form row it stretches across the rest.
//! - **List:** opens 4 pt below with the right edges aligned, at least as
//!   wide as the trigger: `bg.card`, a 1 pt `border.strong` outline, `r.md`,
//!   4 pt padding, the popup shadow. Rows are 26 pt, `r.sm`; the row under
//!   the pointer or the arrow keys is `bg.active`; the current option has
//!   an `accent.text` check on the right and optional detail text before
//!   it; rows that can't be picked are `text.disabled` and the cursor skips
//!   them ([`select_step`]). At most 240 pt tall, then it scrolls, opening
//!   on the current option.
//! - **Keyboard:** on the trigger `Down`, `Up`, `Enter` or `Space` open; in
//!   the list the arrows, `Home` and `End` move, `Enter` / `Space` pick,
//!   `Esc` closes; both give focus back to the trigger. A pick with the
//!   pointer or a click outside closes without focusing the trigger, so the
//!   app's shortcuts work again at once (while the list is open it owns the
//!   keyboard, [`crate::app`]'s shortcuts stand back).

use eframe::egui::{
    self, Align2, Color32, Frame, Id, Key, Margin, Modifiers, Popup, PopupCloseBehavior, RectAlign,
    Response, Sense, Stroke, StrokeKind, WidgetInfo, WidgetType, text::LayoutJob,
};

use crate::theme::{self, fonts, icons};

/// Trigger padding: 8 pt before the text, 6 pt after the caret.
const PAD_LEFT: f32 = 8.0;
const PAD_RIGHT: f32 = 6.0;
/// The caret glyph's size and the gap before it.
const CARET: f32 = 12.0;
const CARET_GAP: f32 = 6.0;
/// A list row's height and the list's tallest height.
pub const ROW: f32 = 26.0;
const LIST_MAX: f32 = 240.0;

/// The trigger width that fits every option: the widest option's text,
/// the paddings and the caret.
pub fn select_natural_width(option_widths: &[f32], caret: f32, padding: (f32, f32)) -> f32 {
    let widest = option_widths.iter().copied().fold(0.0_f32, f32::max);
    padding.0 + widest + CARET_GAP + caret + padding.1
}

/// Where `key` moves the list's cursor from `at` among options that can
/// be picked (`enabled`): `Down` / `Up` to the next / previous enabled row,
/// stopping at the ends (no wrap), `Home` / `End` to the first / last.
/// With no cursor yet, `Down` starts at the first and `Up` at the last.
/// `None` for other keys or when nothing can be picked.
pub fn select_step(at: Option<usize>, enabled: &[bool], key: Key) -> Option<usize> {
    let first = enabled.iter().position(|e| *e)?;
    let last = enabled.iter().rposition(|e| *e)?;
    let at = at.filter(|i| *i < enabled.len());
    match (key, at) {
        (Key::Home, _) | (Key::ArrowDown, None) => Some(first),
        (Key::End, _) | (Key::ArrowUp, None) => Some(last),
        (Key::ArrowDown, Some(i)) => Some(
            (i + 1..enabled.len())
                .find(|j| enabled[*j])
                .unwrap_or(i.max(first).min(last)),
        ),
        (Key::ArrowUp, Some(i)) => Some(
            (0..i)
                .rev()
                .find(|j| enabled[*j])
                .unwrap_or(i.max(first).min(last)),
        ),
        _ => None,
    }
}

/// One option of a select.
#[derive(Debug, Clone, Copy)]
pub struct SelectOption<'a> {
    pub label: &'a str,
    /// Right-aligned extra text (`type.small`, `text.tertiary`).
    pub detail: Option<&'a str>,
    /// Why it can't be picked: shown disabled with this as its tooltip.
    pub blocked: Option<&'a str>,
}

impl<'a> SelectOption<'a> {
    pub fn new(label: &'a str) -> SelectOption<'a> {
        SelectOption {
            label,
            detail: None,
            blocked: None,
        }
    }

    pub fn blocked(mut self, reason: Option<&'a str>) -> Self {
        self.blocked = reason;
        self
    }
}

/// What a select reports.
#[derive(Debug)]
pub struct SelectOutput {
    /// An option was picked this frame (perhaps the current one).
    pub picked: Option<usize>,
    /// The trigger.
    pub response: Response,
}

/// A select over `options` showing `current`.
#[derive(Debug)]
pub struct Select<'a> {
    id: Id,
    options: &'a [SelectOption<'a>],
    current: Option<usize>,
    text: Option<String>,
    fill: bool,
}

impl<'a> Select<'a> {
    pub fn new(id: Id, options: &'a [SelectOption<'a>], current: Option<usize>) -> Select<'a> {
        Select {
            id,
            options,
            current,
            text: None,
            fill: false,
        }
    }

    /// Show this on the trigger instead of the current option's label (the
    /// mixed dash, "Preset (Ease out)").
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Stretch across the available width (form rows).
    pub fn fill_width(mut self) -> Self {
        self.fill = true;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> SelectOutput {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let widths: Vec<f32> = self
            .options
            .iter()
            .map(|o| text_width(ui, o.label, &font))
            .chain(self.text.iter().map(|t| text_width(ui, t, &font)))
            .collect();
        let natural = select_natural_width(&widths, CARET, (PAD_LEFT, PAD_RIGHT));
        let width = if self.fill {
            ui.available_width().max(48.0)
        } else {
            natural.min(ui.available_width().max(48.0))
        };
        let shown = self.text.clone().unwrap_or_else(|| {
            self.current
                .and_then(|i| self.options.get(i))
                .map_or_else(String::new, |o| o.label.to_owned())
        });
        let popup_id = self.id.with("list");
        let open = Popup::is_id_open(ui.ctx(), popup_id);
        let response = trigger(ui, self.id, &shown, width, open, false);

        // Keyboard on the closed trigger opens the list.
        let mut command = response.clicked().then_some(!open);
        if !open && response.has_focus() {
            let pressed = ui.input_mut(|i| {
                [Key::ArrowDown, Key::ArrowUp, Key::Enter, Key::Space]
                    .into_iter()
                    .any(|k| i.consume_key(Modifiers::NONE, k))
            });
            if pressed {
                command = Some(true);
            }
        }
        if command == Some(true) {
            // The cursor starts on the current option.
            ui.data_mut(|d| d.insert_temp(popup_id.with("cursor"), self.current));
            ui.data_mut(|d| d.insert_temp(popup_id.with("scroll"), true));
        }
        let picked = list(ui, &response, popup_id, command, self.options, self.current);
        SelectOutput { picked, response }
    }
}

fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x
}

/// The trigger of a select: `text`, a caret, `width` × the control height.
pub fn trigger(
    ui: &mut egui::Ui,
    id: Id,
    text: &str,
    width: f32,
    open: bool,
    danger: bool,
) -> Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let height = look.metrics.control_height;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let enabled = ui.is_enabled();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    if ui.is_rect_visible(rect) {
        let lit = ui.ctx().animate_bool_with_time(
            id.with("lit"),
            enabled && (response.hovered() || open),
            ui.style().animation_time,
        );
        let opacity = if enabled { 1.0 } else { 0.4 };
        let mut fill = p.bg_field.lerp_to_gamma(p.bg_hover, lit);
        if danger {
            fill = fill.lerp_to_gamma(p.danger, 0.18);
        }
        let painter = ui.painter();
        painter.rect_filled(rect, theme::R_MD, fill.gamma_multiply(opacity));
        let text_color = if !enabled {
            p.text_disabled
        } else if danger {
            p.danger
        } else {
            p.text_primary
        };
        let room = (rect.width() - PAD_LEFT - CARET_GAP - CARET - PAD_RIGHT).max(0.0);
        let galley = painter.layout_job(single_line(ui, text, room, text_color));
        let at = egui::pos2(
            rect.left() + PAD_LEFT,
            rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(at, galley, text_color);
        painter.text(
            egui::pos2(rect.right() - PAD_RIGHT - CARET / 2.0, rect.center().y),
            Align2::CENTER_CENTER,
            icons::CARET_DOWN,
            fonts::icon_font(ui.ctx(), CARET, false),
            p.text_secondary.gamma_multiply(opacity),
        );
        if open {
            painter.rect_stroke(
                rect,
                theme::R_MD,
                Stroke::new(1.0, p.border_strong),
                StrokeKind::Inside,
            );
        } else {
            super::control_outline(ui, rect, theme::R_MD);
        }
        if super::icon_button::keyboard_focus(ui, &response) {
            super::focus_ring(ui, rect, theme::R_MD);
        }
    }
    let label = text.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, enabled, &label));
    response
}

/// `text` on one line cut with an ellipsis at `width`.
fn single_line(ui: &egui::Ui, text: &str, width: f32, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(
        text.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    job
}

/// The list's frame (§13.20): `bg.card`, `border.strong`, `r.md`, 4 pt
/// padding, the popup shadow.
pub fn list_frame(ui: &egui::Ui) -> Frame {
    let p = theme::look(ui.ctx()).palette;
    Frame::new()
        .fill(p.bg_card)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(theme::R_MD)
        .inner_margin(Margin::same(4))
        .shadow(ui.visuals().popup_shadow)
}

/// Opens, shows and closes the list under `anchor`. Returns the option
/// picked this frame. Closing from the keyboard (`Enter` / `Space` pick,
/// `Esc`) gives the trigger focus; closing with the pointer doesn't.
fn list(
    ui: &mut egui::Ui,
    anchor: &Response,
    popup_id: Id,
    command: Option<bool>,
    options: &[SelectOption<'_>],
    current: Option<usize>,
) -> Option<usize> {
    let set = command.map(egui::SetOpenCommand::Bool);
    let enabled: Vec<bool> = options.iter().map(|o| o.blocked.is_none()).collect();
    let cursor_id = popup_id.with("cursor");
    let mut picked = None;
    let mut by_key = false;
    let mut close = false;
    let width = anchor.rect.width();
    let shown = Popup::new(popup_id, ui.ctx().clone(), anchor.rect, anchor.layer_id)
        .open_memory(set)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .align(RectAlign::BOTTOM_END)
        .gap(4.0)
        .width(width)
        .frame(list_frame(ui))
        .show(|ui| {
            ui.set_min_width((width - 8.0).max(0.0));
            let mut cursor: Option<usize> = ui.data(|d| d.get_temp(cursor_id)).flatten();
            // Arrows, Home and End move the cursor; Enter or Space picks.
            let (keys, pick) = ui.input_mut(|i| {
                let keys: Vec<Key> = [Key::ArrowDown, Key::ArrowUp, Key::Home, Key::End]
                    .into_iter()
                    .flat_map(|k| {
                        std::iter::repeat_n(k, i.count_and_consume_key(Modifiers::NONE, k))
                    })
                    .collect();
                let pick = i.consume_key(Modifiers::NONE, Key::Enter)
                    || i.consume_key(Modifiers::NONE, Key::Space);
                (keys, pick)
            });
            let moved = !keys.is_empty();
            for key in keys {
                cursor = select_step(cursor, &enabled, key).or(cursor);
            }
            if pick && let Some(i) = cursor.filter(|i| enabled.get(*i) == Some(&true)) {
                picked = Some(i);
                by_key = true;
            }
            let scroll_once = ui
                .data_mut(|d| d.remove_temp::<bool>(popup_id.with("scroll")))
                .unwrap_or(false);
            egui::ScrollArea::vertical()
                .max_height(LIST_MAX)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, option) in options.iter().enumerate() {
                        let response = row(
                            ui,
                            popup_id.with(i),
                            option,
                            current == Some(i),
                            cursor == Some(i),
                        );
                        if response.hovered() && option.blocked.is_none() {
                            cursor = Some(i);
                        }
                        if response.clicked() && option.blocked.is_none() {
                            picked = Some(i);
                        }
                        if (scroll_once && current == Some(i)) || (moved && cursor == Some(i)) {
                            response.scroll_to_me(None);
                        }
                    }
                });
            ui.data_mut(|d| d.insert_temp(cursor_id, cursor));
        });
    if picked.is_some() {
        close = true;
    }
    if let Some(shown) = shown
        && (close || shown.response.should_close())
    {
        Popup::close_id(ui.ctx(), popup_id);
        if by_key || escaped(ui) {
            anchor.request_focus();
        }
    }
    picked
}

/// `Esc` was pressed this frame (it closes an open list).
fn escaped(ui: &egui::Ui) -> bool {
    ui.input(|i| i.key_pressed(Key::Escape))
}

/// One row of a select list: the label, its detail, and the check on the
/// current option.
pub fn row(
    ui: &mut egui::Ui,
    id: Id,
    option: &SelectOption<'_>,
    current: bool,
    cursor: bool,
) -> Response {
    let p = theme::look(ui.ctx()).palette;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW), Sense::hover());
    let enabled = option.blocked.is_none();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if enabled && (cursor || response.hovered()) {
            painter.rect_filled(rect, theme::R_SM, p.bg_active);
        }
        let color = if enabled {
            p.text_primary
        } else {
            p.text_disabled
        };
        let mut right = rect.right() - 8.0;
        // The check column is always kept, so labels and details line up.
        if current {
            painter.text(
                egui::pos2(right - 6.0, rect.center().y),
                Align2::CENTER_CENTER,
                icons::CHECK,
                fonts::icon_font(ui.ctx(), 12.0, false),
                p.accent_text,
            );
        }
        right -= 12.0 + 8.0;
        if let Some(detail) = option.detail {
            let galley = painter.layout_no_wrap(
                detail.to_owned(),
                egui::TextStyle::Small.resolve(ui.style()),
                p.text_tertiary,
            );
            let at = egui::pos2(
                right - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            );
            right = at.x - 8.0;
            painter.galley(at, galley, p.text_tertiary);
        }
        let room = (right - rect.left() - 8.0).max(0.0);
        let galley = painter.layout_job(single_line(ui, option.label, room, color));
        let at = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, color);
    }
    let label = option.label.to_owned();
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, current, &label)
    });
    match option.blocked {
        Some(reason) => response.on_hover_text(reason),
        None => response,
    }
}

/// A select of `options` keyed by `id` in a popup anchored under
/// `anchor`, with custom contents (the font picker's search box): opens on
/// a click of `anchor` (or the keyboard, like [`Select`]), and stays open
/// until `add` returns `true` or the user clicks outside or presses `Esc`.
pub fn custom_list(
    ui: &mut egui::Ui,
    anchor: &Response,
    min_width: f32,
    add: impl FnOnce(&mut egui::Ui) -> bool,
) {
    let popup_id = anchor.id.with("list");
    let mut command = anchor
        .clicked()
        .then(|| !Popup::is_id_open(ui.ctx(), popup_id));
    if !Popup::is_id_open(ui.ctx(), popup_id) && anchor.has_focus() {
        let pressed = ui.input_mut(|i| {
            [Key::ArrowDown, Key::Enter, Key::Space]
                .into_iter()
                .any(|k| i.consume_key(Modifiers::NONE, k))
        });
        if pressed {
            command = Some(true);
        }
    }
    let mut close = false;
    let shown = Popup::new(popup_id, ui.ctx().clone(), anchor.rect, anchor.layer_id)
        .open_memory(command.map(egui::SetOpenCommand::Bool))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .align(RectAlign::BOTTOM_END)
        .gap(4.0)
        .width(min_width.max(anchor.rect.width()))
        .frame(list_frame(ui))
        .show(|ui| {
            ui.set_min_width((min_width.max(anchor.rect.width()) - 8.0).max(0.0));
            close = add(ui);
        });
    if let Some(shown) = shown
        && (close || shown.response.should_close())
    {
        Popup::close_id(ui.ctx(), popup_id);
        // Only a keyboard close gives the trigger focus back (see `list`).
        if escaped(ui) || ui.input(|i| i.key_pressed(Key::Enter)) {
            anchor.request_focus();
        }
    }
}

/// Where a list of `size` opens under a trigger at `rect`.
#[cfg(test)]
fn list_rect_below(rect: egui::Rect, size: egui::Vec2) -> egui::Rect {
    let (pivot, at) = RectAlign::BOTTOM_END.pivot_pos(&rect, 4.0);
    pivot.anchor_size(at, size)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Pos2, Rect};

    use super::*;
    use crate::ui::drag::tests::{Frames, button, key};

    #[test]
    fn the_trigger_fits_the_widest_option() {
        let w = select_natural_width(&[30.0, 72.5, 10.0], 12.0, (8.0, 6.0));
        assert_eq!(w, 8.0 + 72.5 + 6.0 + 12.0 + 6.0);
        assert_eq!(select_natural_width(&[], 12.0, (8.0, 6.0)), 32.0);
    }

    #[test]
    fn the_cursor_clamps_and_skips_rows_that_cannot_be_picked() {
        let enabled = [true, false, true, true, false];
        assert_eq!(select_step(Some(0), &enabled, Key::ArrowDown), Some(2));
        assert_eq!(select_step(Some(2), &enabled, Key::ArrowUp), Some(0));
        assert_eq!(
            select_step(Some(3), &enabled, Key::ArrowDown),
            Some(3),
            "no wrap"
        );
        assert_eq!(
            select_step(Some(0), &enabled, Key::ArrowUp),
            Some(0),
            "no wrap"
        );
        assert_eq!(select_step(None, &enabled, Key::ArrowDown), Some(0));
        assert_eq!(select_step(None, &enabled, Key::ArrowUp), Some(3));
        assert_eq!(select_step(Some(2), &enabled, Key::Home), Some(0));
        assert_eq!(select_step(Some(0), &enabled, Key::End), Some(3));
        assert_eq!(select_step(Some(0), &enabled, Key::Enter), None);
        assert_eq!(select_step(Some(0), &[false, false], Key::ArrowDown), None);
        assert_eq!(select_step(Some(9), &enabled, Key::ArrowDown), Some(0));
    }

    #[test]
    fn the_list_opens_below_with_the_right_edges_aligned() {
        let trigger = Rect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(120.0, 28.0));
        let list = list_rect_below(trigger, egui::vec2(180.0, 100.0));
        assert_eq!(list.right(), trigger.right());
        assert_eq!(list.top(), trigger.bottom() + 4.0);
    }

    /// Opens with the keyboard, moves down past a blocked row, picks with
    /// Enter; Esc closes without picking.
    #[test]
    fn the_keyboard_opens_moves_picks_and_esc_closes() {
        let options = [
            SelectOption::new("Linear"),
            SelectOption::new("Smooth").blocked(Some("not here")),
            SelectOption::new("Fast"),
        ];
        let mut frames = Frames::new();
        let mut picked = Vec::new();
        let id = Id::new("select");
        let frame = |frames: &mut Frames, events: Vec<Event>, picked: &mut Vec<usize>| {
            frames.run(events, |ui| {
                if let Some(i) = Select::new(id, &options, Some(0)).show(ui).picked {
                    picked.push(i);
                }
            });
        };
        frame(&mut frames, Vec::new(), &mut picked);
        let at = frames.ctx.read_response(id).unwrap().rect.center();
        // A click opens it.
        frame(
            &mut frames,
            vec![Event::PointerMoved(at), button(at, true)],
            &mut picked,
        );
        frame(&mut frames, vec![button(at, false)], &mut picked);
        frame(&mut frames, Vec::new(), &mut picked);
        assert!(Popup::is_id_open(&frames.ctx, id.with("list")));
        // Esc closes without a pick.
        frame(
            &mut frames,
            vec![key(Key::Escape, Modifiers::NONE)],
            &mut picked,
        );
        frame(&mut frames, Vec::new(), &mut picked);
        assert!(!Popup::is_id_open(&frames.ctx, id.with("list")));
        assert!(picked.is_empty());
        // Focused (Esc gave focus back): Down opens, Down moves past the
        // blocked row, Enter picks.
        frame(
            &mut frames,
            vec![key(Key::ArrowDown, Modifiers::NONE)],
            &mut picked,
        );
        frame(&mut frames, Vec::new(), &mut picked);
        assert!(Popup::is_id_open(&frames.ctx, id.with("list")));
        frame(
            &mut frames,
            vec![key(Key::ArrowDown, Modifiers::NONE)],
            &mut picked,
        );
        frame(
            &mut frames,
            vec![key(Key::Enter, Modifiers::NONE)],
            &mut picked,
        );
        frame(&mut frames, Vec::new(), &mut picked);
        assert_eq!(picked, [2]);
        assert!(!Popup::is_id_open(&frames.ctx, id.with("list")));
        assert!(
            frames.ctx.memory(|m| m.has_focus(id)),
            "a keyboard pick keeps the focus on the trigger"
        );
    }

    /// A pick with the pointer, or a click outside, closes the list without
    /// focusing the trigger: the app's shortcuts (which stand back while a
    /// widget has focus) work again at once.
    #[test]
    fn a_pointer_close_leaves_the_keyboard_to_the_app() {
        let options = [SelectOption::new("Linear"), SelectOption::new("Fast")];
        let mut frames = Frames::new();
        let mut picked = Vec::new();
        let id = Id::new("select");
        let frame = |frames: &mut Frames, events: Vec<Event>, picked: &mut Vec<usize>| {
            frames.run(events, |ui| {
                if let Some(i) = Select::new(id, &options, Some(0)).show(ui).picked {
                    picked.push(i);
                }
            });
        };
        let click = |frames: &mut Frames, at: Pos2, picked: &mut Vec<usize>| {
            frame(
                frames,
                vec![Event::PointerMoved(at), button(at, true)],
                picked,
            );
            frame(frames, vec![button(at, false)], picked);
            frame(frames, Vec::new(), picked);
        };
        frame(&mut frames, Vec::new(), &mut picked);
        let at = frames.ctx.read_response(id).unwrap().rect.center();
        let list = id.with("list");
        let focused = |frames: &Frames| frames.ctx.memory(|m| m.focused());

        // Opened and picked with the pointer.
        click(&mut frames, at, &mut picked);
        assert!(Popup::is_id_open(&frames.ctx, list));
        let row = frames.ctx.read_response(list.with(1usize)).unwrap().rect;
        click(&mut frames, row.center(), &mut picked);
        assert_eq!(picked, [1]);
        assert!(!Popup::is_id_open(&frames.ctx, list));
        assert_eq!(focused(&frames), None, "after a pointer pick");

        // Opened, then dismissed by a click outside.
        click(&mut frames, at, &mut picked);
        assert!(Popup::is_id_open(&frames.ctx, list));
        click(&mut frames, Pos2::new(700.0, 500.0), &mut picked);
        assert!(!Popup::is_id_open(&frames.ctx, list));
        assert_eq!(focused(&frames), None, "after a click outside");
        assert_eq!(picked, [1]);
    }
}

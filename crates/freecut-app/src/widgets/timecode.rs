//! The Stage's timecode field (ui-concept §13.25): the playhead as
//! `HH:MM:SS:FF` in a 116 × 28 pt field. Dragging across it scrubs at
//! 3 pt per frame (`Shift` ×10, `Alt` ×0.1); a click types a timecode that
//! `Enter` (or leaving the field) seeks to and `Esc` drops; `Up` / `Down`
//! step one frame while typing. Seeking is not an edit: nothing goes on the
//! undo stack.
//!
//! Times are frame indices here (integers, CLAUDE.md's rational rule); the
//! caller turns them into ticks.

use eframe::egui::{self, Align2, Id, Key, Sense};

use crate::theme;

/// Points of drag per frame.
pub const PT_PER_FRAME: f32 = 3.0;
/// The field's size (§13.25).
pub const SIZE: egui::Vec2 = egui::vec2(116.0, 28.0);

/// `frame` at `fps` nominal frames per second as `HH:MM:SS:FF`
/// (non-drop, UI-019's format). Negative frames show as zero.
pub fn format_tc(frame: i64, fps: u32) -> String {
    let fps = i64::from(fps.max(1));
    let n = frame.max(0);
    let ff = n % fps;
    let secs = n / fps;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        ff
    )
}

/// The frame index a typed timecode names: `HH:MM:SS:FF`, or its tail
/// (`MM:SS:FF`, `SS:FF`, `FF`), fields separated by `:`, `;` or `.`.
/// Minutes and seconds must be below 60 and frames below `fps`; anything
/// else is `None` (the old value stays).
pub fn parse_tc(text: &str, fps: u32) -> Option<i64> {
    let fps = i64::from(fps.max(1));
    let fields: Vec<&str> = text.trim().split([':', ';', '.']).collect();
    if fields.is_empty() || fields.len() > 4 {
        return None;
    }
    let mut numbers = Vec::with_capacity(4);
    for f in &fields {
        if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        numbers.push(f.parse::<i64>().ok()?);
    }
    // Pad on the left: "12:05" is seconds and frames.
    while numbers.len() < 4 {
        numbers.insert(0, 0);
    }
    let [h, m, s, f] = [numbers[0], numbers[1], numbers[2], numbers[3]];
    if fields.len() > 1 && f >= fps {
        return None;
    }
    if (fields.len() > 2 && s >= 60) || (fields.len() > 3 && m >= 60) {
        return None;
    }
    h.checked_mul(3600)?
        .checked_add(m.checked_mul(60)?)?
        .checked_add(s)?
        .checked_mul(fps)?
        .checked_add(f)
}

/// The frame a scrub that started on `start` reaches after `travel` pt
/// of modifier-weighted drag, kept in `0..=last`.
pub fn scrub_frame(start: i64, travel: f32, last: i64) -> i64 {
    let frames = (travel / PT_PER_FRAME).trunc() as i64;
    start.saturating_add(frames).clamp(0, last.max(0))
}

/// The drag weight of the held modifiers.
fn weight(modifiers: egui::Modifiers) -> f32 {
    if modifiers.shift {
        10.0
    } else if modifiers.alt {
        0.1
    } else {
        1.0
    }
}

#[derive(Debug, Clone, Default)]
struct FieldState {
    /// The text being typed, while typing.
    typing: Option<String>,
    /// The frame a scrub started on and its weighted travel so far.
    scrub: Option<(i64, f32)>,
}

/// Shows the field for the playhead at `frame` of `0..=last`; returns the
/// frame to seek to, if the user asked for one this frame.
pub fn timecode_field(ui: &mut egui::Ui, id: Id, frame: i64, last: i64, fps: u32) -> Option<i64> {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let mut state: FieldState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut seek = None;
    let size = egui::vec2(SIZE.x, look.metrics.control_height);
    let mono = egui::TextStyle::Monospace.resolve(ui.style());

    if let Some(mut text) = state.typing.take() {
        let edit_id = id.with("typing");
        let (up, down) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            )
        });
        let typed = parse_tc(&text, fps).unwrap_or(frame);
        if up || down {
            let to = (typed + if up { 1 } else { -1 }).clamp(0, last.max(0));
            text = format_tc(to, fps);
            seek = Some(to);
        }
        let response = ui.add_sized(
            size,
            egui::TextEdit::singleline(&mut text)
                .id(edit_id)
                .font(mono)
                .margin(egui::vec2(8.0, 4.0))
                .char_limit(16),
        );
        let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if escape {
            // Esc drops the typing; the playhead stays.
        } else if response.lost_focus() || enter {
            if let Some(to) = parse_tc(&text, fps) {
                seek = Some(to.clamp(0, last.max(0)));
            }
        } else {
            state.typing = Some(text);
        }
        if state.typing.is_some() && !response.has_focus() && !response.gained_focus() {
            response.request_focus();
        }
        ui.data_mut(|d| d.insert_temp(id, state));
        return seek;
    }

    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.drag_started() {
        state.scrub = Some((frame, 0.0));
    }
    if response.dragged()
        && let Some((start, travel)) = state.scrub.as_mut()
    {
        let w = ui.input(|i| weight(i.modifiers));
        *travel += response.drag_delta().x * w;
        let to = scrub_frame(*start, *travel, last);
        if to != frame {
            seek = Some(to);
        }
    }
    if response.drag_stopped() {
        state.scrub = None;
    }
    if response.clicked() {
        // Typing starts with the whole timecode selected.
        let text = format_tc(frame, fps);
        let mut edit = egui::text_edit::TextEditState::default();
        edit.cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
        edit.store(ui.ctx(), id.with("typing"));
        state.typing = Some(text);
        ui.memory_mut(|m| m.request_focus(id.with("typing")));
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(
            id.with("hover"),
            response.hovered() || response.dragged(),
            ui.style().animation_time,
        );
        let fill = p.bg_field.lerp_to_gamma(p.bg_hover, t);
        ui.painter().rect_filled(rect, theme::R_MD, fill);
        crate::widgets::control_outline(ui, rect, theme::R_MD);
        ui.painter().text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            Align2::LEFT_CENTER,
            format_tc(frame, fps),
            mono,
            p.text_primary,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Playhead timecode")
    });
    response.on_hover_text("Playhead: drag to scrub (Shift ×10, Alt fine), click to type a time");
    ui.data_mut(|d| d.insert_temp(id, state));
    seek
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecodes_round_trip_at_common_rates() {
        for fps in [24, 25, 30, 60] {
            for frame in [
                0,
                1,
                fps as i64 - 1,
                fps as i64,
                3_600 * fps as i64 + 17,
                123_456,
            ] {
                let text = format_tc(frame, fps);
                assert_eq!(parse_tc(&text, fps), Some(frame), "{text} at {fps}");
            }
        }
        // 23.976 and 29.97 count nominal 24 / 30 frames (non-drop).
        assert_eq!(format_tc(24, 24), "00:00:01:00");
        assert_eq!(format_tc(30 * 61 + 5, 30), "00:01:01:05");
    }

    #[test]
    fn short_and_invalid_timecodes() {
        assert_eq!(parse_tc("12", 30), Some(12));
        assert_eq!(parse_tc("2:05", 30), Some(65));
        assert_eq!(parse_tc("1:00:00", 25), Some(60 * 25));
        assert_eq!(parse_tc("00;00;01;00", 30), Some(30));
        assert_eq!(parse_tc("00:00:01:30", 30), None, "frames past the rate");
        assert_eq!(parse_tc("00:61:00:00", 30), None);
        assert_eq!(parse_tc("abc", 30), None);
        assert_eq!(parse_tc("", 30), None);
        assert_eq!(parse_tc("1::2", 30), None);
        assert_eq!(parse_tc("1:2:3:4:5", 30), None);
    }

    #[test]
    fn scrubbing_moves_a_frame_every_3_pt_within_the_sequence() {
        assert_eq!(scrub_frame(10, 2.9, 100), 10);
        assert_eq!(scrub_frame(10, 3.0, 100), 11);
        assert_eq!(scrub_frame(10, -9.5, 100), 7);
        assert_eq!(scrub_frame(10, 3000.0, 100), 100);
        assert_eq!(scrub_frame(10, -3000.0, 100), 0);
        assert_eq!(weight(egui::Modifiers::SHIFT), 10.0);
        assert_eq!(weight(egui::Modifiers::ALT), 0.1);
    }
}

//! Marker flags on the timeline (TL-037): timeline markers stand on the
//! ruler floor as 9 pt flags (a range adds a 4 pt bar along the floor);
//! clip markers are 7 pt flags on the clip's name strip. Selected flags get
//! a 1 pt accent ring. Double-clicking a flag opens the editor popover:
//! name, note, the 8 colours and the duration, applied on `Enter` or when
//! the popover closes as one `EditMarker`; `Esc` drops the changes.

use eframe::egui::{self, Color32, Key, Pos2, Rect, Shape, Stroke, Vec2};
use freecut_timeline::edit::MarkerPatch;
use freecut_timeline::model::{
    Document, MAX_MARKER_NAME_CHARS, MAX_MARKER_NOTE_CHARS, Marker, MarkerRef, PaletteColor,
};
use freecut_timeline::time::{FrameRate, Tick};

use super::timeline_tools::{FLAG_W, FlagGrab, FlagSpot, flag_hit};
use crate::theme::{self, Palette, surface};
use crate::timeline_commands::color_name;
use crate::widgets::timecode::timecode_field;

/// A clip marker's flag width (TL-037: 7 pt).
pub const CLIP_FLAG_W: f32 = 7.0;
/// Clips narrower than this show no marker flags.
pub const CLIP_FLAGS_MIN_WIDTH: f32 = 24.0;

/// One flag as laid out this frame.
#[derive(Debug, Clone)]
pub struct Flag {
    pub r: MarkerRef,
    pub spot: FlagSpot,
    pub color: PaletteColor,
    pub name: String,
    /// The y the pole stands on, and the top of the flag's hit area.
    pub floor: f32,
    pub top: f32,
    /// A timeline marker on the ruler (else a clip marker on its strip).
    pub ruler: bool,
    pub selected: bool,
    /// Where a clip flag may be drawn (its clip, clipped to the lanes).
    pub clip: Option<Rect>,
}

impl Flag {
    fn width(&self) -> f32 {
        if self.ruler { FLAG_W } else { CLIP_FLAG_W }
    }
}

/// The flag under `pos` and what it grabs: the last drawn wins.
pub fn flag_at(flags: &[Flag], pos: Pos2) -> Option<(usize, FlagGrab)> {
    for (i, f) in flags.iter().enumerate().rev() {
        if pos.y < f.top || pos.y > f.floor + 1.0 {
            continue;
        }
        let spot = if f.ruler {
            f.spot
        } else {
            // Clip flags are narrower: shift the hit test to their width.
            FlagSpot {
                x: f.spot.x - (FLAG_W - CLIP_FLAG_W) / 2.0,
                end_x: None,
            }
        };
        match flag_hit(&[spot], pos.x) {
            Some(FlagGrab::Start(_)) => return Some((i, FlagGrab::Start(0))),
            Some(FlagGrab::End(_)) => return Some((i, FlagGrab::End(0))),
            None => {}
        }
    }
    None
}

/// The tooltip of flag `i`: its name, and the names of every marker
/// stacked at the same place.
pub fn tooltip(flags: &[Flag], i: usize) -> String {
    let Some(f) = flags.get(i) else {
        return String::new();
    };
    let names: Vec<&str> = flags
        .iter()
        .filter(|o| o.ruler == f.ruler && (o.spot.x - f.spot.x).abs() < 1.0)
        .map(|o| o.name.as_str())
        .collect();
    let named: Vec<String> = names
        .iter()
        .map(|n| {
            if n.is_empty() {
                "(unnamed marker)".to_owned()
            } else {
                (*n).to_owned()
            }
        })
        .collect();
    named.join("\n")
}

/// Paints `flag` (its range bar, pole and pennant) with `painter`, which
/// is clipped to the ruler or the lanes.
pub fn paint_flag(painter: &egui::Painter, flag: &Flag, p: &Palette, hot: bool) {
    let painter = match flag.clip {
        Some(clip) => painter.with_clip_rect(painter.clip_rect().intersect(clip)),
        None => painter.clone(),
    };
    let color = surface::palette(flag.color);
    let w = flag.width();
    let x = flag.spot.x.round() + 0.5;
    if let Some(end) = flag.spot.end_x {
        let bar = Rect::from_min_max(
            Pos2::new(x, flag.floor - 4.0),
            Pos2::new(end.max(x + 1.0), flag.floor),
        );
        painter.rect_filled(bar, 1.0, color.gamma_multiply(0.7));
    }
    let h = if flag.ruler { 13.0 } else { 9.0 };
    let top = flag.floor - h;
    painter.vline(
        x,
        egui::Rangef::new(top, flag.floor),
        Stroke::new(1.0, color),
    );
    let pennant = h * 0.55;
    let shape = vec![
        Pos2::new(x, top),
        Pos2::new(x + w, top + pennant / 2.0),
        Pos2::new(x, top + pennant),
    ];
    let edge = if hot {
        Stroke::new(1.0, p.text_primary)
    } else {
        Stroke::new(1.0, Color32::from_black_alpha(90))
    };
    painter.add(Shape::convex_polygon(shape, color, edge));
    if flag.selected {
        let ring = Rect::from_min_max(
            Pos2::new(x - 2.0, top - 2.0),
            Pos2::new(x + w + 2.0, flag.floor + 1.0),
        );
        painter.rect_stroke(
            ring,
            theme::R_XS,
            Stroke::new(1.0, p.accent),
            egui::StrokeKind::Outside,
        );
    }
}

/// The editor popover's state: the values being typed, applied as one
/// `EditMarker` when it closes.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkerEdit {
    pub marker: MarkerRef,
    /// The flag it hangs from.
    pub anchor: Pos2,
    pub name: String,
    pub note: String,
    pub color: PaletteColor,
    /// The duration in frames (0 is a point).
    pub frames: i64,
    /// Set on the frame it opened, so the opening click doesn't close it.
    fresh: bool,
}

/// What the popover asks for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutcome {
    Open,
    /// `Enter` or a click outside: apply.
    Commit,
    /// `Esc`: drop the changes.
    Discard,
}

/// The marker `r` points at, from the root sequence or its clip.
pub fn find_marker(doc: &Document, r: MarkerRef) -> Option<Marker> {
    match r {
        MarkerRef::Timeline(id) => doc.sequence(doc.root())?.markers.get(id).cloned(),
        MarkerRef::Clip(clip, id) => doc.clip(clip)?.markers.get(id).cloned(),
    }
}

impl MarkerEdit {
    /// The editor for marker `r`, showing its current values.
    pub fn open(doc: &Document, r: MarkerRef, anchor: Pos2) -> Option<MarkerEdit> {
        let m = find_marker(doc, r)?;
        let rate = doc.settings().frame_rate;
        Some(MarkerEdit {
            marker: r,
            anchor,
            name: m.name,
            note: m.note,
            color: m.color,
            frames: rate.frame_index(m.at.duration()).max(0),
            fresh: true,
        })
    }

    /// The patch that turns `m` into what was typed: only changed fields.
    pub fn patch(&self, m: &Marker, rate: FrameRate) -> MarkerPatch {
        let duration = rate.frame_start(self.frames.max(0)).unwrap_or(Tick::ZERO);
        MarkerPatch {
            name: (self.name != m.name).then(|| self.name.clone()),
            note: (self.note != m.note).then(|| self.note.clone()),
            color: (self.color != m.color).then_some(self.color),
            duration: (duration != m.at.duration()).then_some(duration),
        }
    }
}

/// Whether a patch changes nothing.
pub fn patch_is_empty(patch: &MarkerPatch) -> bool {
    patch.name.is_none()
        && patch.note.is_none()
        && patch.color.is_none()
        && patch.duration.is_none()
}

/// Shows the editor popover under its flag.
pub fn editor(ctx: &egui::Context, edit: &mut MarkerEdit, rate: FrameRate) -> EditOutcome {
    let look = theme::look(ctx);
    let p = look.palette;
    let id = egui::Id::new(("freecut-marker-editor", edit.marker));
    let mut outcome = EditOutcome::Open;
    let mut note_focused = false;
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(edit.anchor + Vec2::new(-12.0, 6.0))
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(260.0);
                ui.label(egui::RichText::new("Marker").strong());
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Name").small().color(p.text_secondary));
                let name = ui.add(
                    egui::TextEdit::singleline(&mut edit.name)
                        .char_limit(MAX_MARKER_NAME_CHARS)
                        .desired_width(f32::INFINITY),
                );
                if edit.fresh {
                    name.request_focus();
                }
                ui.label(egui::RichText::new("Note").small().color(p.text_secondary));
                note_focused = ui
                    .add(
                        egui::TextEdit::multiline(&mut edit.note)
                            .char_limit(MAX_MARKER_NOTE_CHARS)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    )
                    .has_focus();
                ui.label(
                    egui::RichText::new("Colour")
                        .small()
                        .color(p.text_secondary),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for c in PaletteColor::ALL {
                        swatch(ui, c, &mut edit.color);
                    }
                });
                ui.label(
                    egui::RichText::new("Duration (0 is a point)")
                        .small()
                        .color(p.text_secondary),
                );
                let last = rate.frame_index(Tick::from_secs(24 * 3600).unwrap_or(Tick::ZERO));
                if let Some(frames) = timecode_field(
                    ui,
                    id.with("duration"),
                    edit.frames,
                    last,
                    rate.nominal_fps(),
                ) {
                    edit.frames = frames.max(0);
                }
            });
        });
    let (enter, escape, clicked_outside) = ctx.input(|i| {
        let pressed_outside = i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| !area.response.rect.contains(pos));
        (
            i.key_pressed(Key::Enter) && !i.modifiers.shift,
            i.key_pressed(Key::Escape),
            pressed_outside,
        )
    });
    if escape {
        outcome = EditOutcome::Discard;
    } else if (enter && !note_focused) || (clicked_outside && !edit.fresh) {
        outcome = EditOutcome::Commit;
    }
    edit.fresh = false;
    outcome
}

/// One colour swatch: a 18 pt dot, ringed when chosen.
fn swatch(ui: &mut egui::Ui, c: PaletteColor, chosen: &mut PaletteColor) {
    let p = theme::look(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::click());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 7.0, surface::palette(c));
    if *chosen == c {
        painter.circle_stroke(rect.center(), 9.0, Stroke::new(1.5, p.text_primary));
    } else if response.hovered() {
        painter.circle_stroke(rect.center(), 9.0, Stroke::new(1.0, p.border_strong));
    }
    if response.clicked() {
        *chosen = c;
    }
    let name = color_name(c);
    let selected = *chosen == c;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, name)
    });
    response.on_hover_text(name);
}

/// A tiny label of the time under the blade, drawn beside its line.
pub fn paint_time_pill(painter: &egui::Painter, at: Pos2, text: &str, p: &Palette) {
    let font = egui::FontId::monospace(10.0);
    let galley = painter.layout_no_wrap(text.to_owned(), font, Color32::WHITE);
    let rect = Rect::from_min_size(at, galley.size() + Vec2::new(8.0, 4.0));
    painter.rect_filled(rect, theme::R_XS, p.overlay_badge);
    painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use super::*;
    use crate::timeline_commands::tests::{s, talking_head};

    fn flag(x: f32, name: &str, ruler: bool) -> Flag {
        Flag {
            r: MarkerRef::Timeline(freecut_timeline::ids::MarkerId::from_raw(
                core::num::NonZeroU64::new(x as u64 + 1).unwrap(),
            )),
            spot: FlagSpot { x, end_x: None },
            color: PaletteColor::Blue,
            name: name.to_owned(),
            floor: 28.0,
            top: 14.0,
            ruler,
            selected: false,
            clip: None,
        }
    }

    #[test]
    fn stacked_flags_share_one_tooltip_and_presses_need_the_flag_height() {
        let flags = [
            flag(10.0, "Intro", true),
            flag(10.0, "", true),
            flag(50.0, "Out", true),
        ];
        assert_eq!(tooltip(&flags, 0), "Intro\n(unnamed marker)");
        assert_eq!(tooltip(&flags, 2), "Out");
        assert_eq!(
            flag_at(&flags, Pos2::new(12.0, 20.0)).map(|(i, _)| i),
            Some(1),
            "the one drawn last"
        );
        assert_eq!(flag_at(&flags, Pos2::new(12.0, 5.0)), None, "above it");
        assert_eq!(flag_at(&flags, Pos2::new(30.0, 20.0)), None);
    }

    #[test]
    fn the_editor_patches_only_what_changed() {
        let (mut p, _) = talking_head();
        p.playhead = s(1);
        let r = p.add_marker(None).unwrap();
        let doc = p.doc().clone();
        let rate = doc.settings().frame_rate;
        let mut edit = MarkerEdit::open(&doc, r, Pos2::ZERO).unwrap();
        assert_eq!(edit.name, "Marker 1");
        let m = find_marker(&doc, r).unwrap();
        assert!(patch_is_empty(&edit.patch(&m, rate)));
        edit.name = "Hook".into();
        edit.frames = 30;
        edit.color = PaletteColor::Green;
        let patch = edit.patch(&m, rate);
        assert_eq!(patch.name.as_deref(), Some("Hook"));
        assert_eq!(patch.note, None);
        assert_eq!(patch.color, Some(PaletteColor::Green));
        assert_eq!(patch.duration, Some(rate.frame_start(30).unwrap()));
        p.edit_marker(r, patch).unwrap();
        let m = find_marker(p.doc(), r).unwrap();
        assert_eq!(m.name, "Hook");
        assert_eq!(m.at.duration(), rate.frame_start(30).unwrap());
    }
}

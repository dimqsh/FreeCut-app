//! The segmented control (ui-concept §13.4): equal-width items in a
//! `bg.well` track with a hairline edge (`r.md`), inside a 3 pt inset; one
//! `bg.hover` **thumb** (`r.sm`) slides to the chosen item over `dur.base`
//! ([`segment_thumb_rect`]). Labels are `text.primary` on the chosen or
//! hovered item and `text.secondary` otherwise; icons go through the icon
//! font, never the text font. It is one focus stop with radio-group
//! semantics: the arrow keys move the selection, `Home` and `End` jump to
//! the ends ([`key_step`]). Used for the lens strip, the Detail switch,
//! alignment, case and the In / Loop / Out picker.

use eframe::egui::{
    self, Event, EventFilter, Id, Key, Rect, Response, Sense, Stroke, StrokeKind, TextStyle,
    WidgetInfo, WidgetType, text::LayoutJob,
};

use crate::theme::{self, fonts};

/// One item: an icon, a label, or both.
#[derive(Debug, Clone, Copy)]
pub struct Segment<'a> {
    pub icon: Option<&'a str>,
    pub label: &'a str,
    /// Extra hover text; icon-only items always show their label.
    pub tooltip: Option<&'a str>,
}

impl<'a> Segment<'a> {
    pub fn new(icon: Option<&'a str>, label: &'a str) -> Segment<'a> {
        Segment {
            icon,
            label,
            tooltip: None,
        }
    }
}

/// Where `key` moves the selection of a control with `len` items, from
/// `selected`: Left/Up to the previous item and Right/Down to the next,
/// wrapping around (radio-group semantics); Home and End to the ends.
/// `None` for other keys and empty controls.
pub fn key_step(selected: usize, len: usize, key: Key) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let selected = selected.min(len - 1);
    match key {
        Key::ArrowLeft | Key::ArrowUp => Some((selected + len - 1) % len),
        Key::ArrowRight | Key::ArrowDown => Some((selected + 1) % len),
        Key::Home => Some(0),
        Key::End => Some(len - 1),
        _ => None,
    }
}

/// The track's inset around the items.
pub const INSET: f32 = 3.0;

/// `dur.base` (§12.8): the thumb's slide.
const DUR_BASE: f32 = 0.16;

/// The thumb's rectangle on item `i` of `n` equal items in `track`, inside
/// the 3 pt inset. An empty control has no thumb (an empty rect).
#[cfg(test)]
pub fn segment_thumb_rect(track: Rect, i: usize, n: usize) -> Rect {
    if n == 0 {
        return Rect::NOTHING;
    }
    let rects = segment_rects(track, &vec![1.0; n]);
    rects[i.min(n - 1)]
}

/// The items' rectangles inside `track`'s 3 pt inset, as wide as their
/// `weights` share of it (equal weights: equal items; the lens strip's
/// words: each as wide as its word asks, the spare room shared out).
pub fn segment_rects(track: Rect, weights: &[f32]) -> Vec<Rect> {
    let inner = track.shrink(INSET);
    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    let n = weights.len().max(1) as f32;
    let mut x = inner.left();
    weights
        .iter()
        .map(|w| {
            let share = if total > 0.0 {
                w.max(0.0) / total
            } else {
                1.0 / n
            };
            let width = inner.width() * share;
            let rect = Rect::from_min_size(
                egui::pos2(x, inner.top()),
                egui::vec2(width, inner.height()),
            );
            x += width;
            rect
        })
        .collect()
}

/// What the control reports.
#[derive(Debug)]
pub struct SegmentedOutput {
    /// A different item was picked this frame.
    pub picked: Option<usize>,
    pub response: Response,
}

/// A segmented control.
#[derive(Debug)]
pub struct Segmented<'a> {
    id: Id,
    items: &'a [Segment<'a>],
    selected: usize,
    icons_only: bool,
    words_or_icons: bool,
    /// Words only, each item as wide as its word (set by `show`).
    words: bool,
    fill_width: bool,
    width: Option<f32>,
    height: Option<f32>,
}

impl<'a> Segmented<'a> {
    pub fn new(id: Id, items: &'a [Segment<'a>], selected: usize) -> Segmented<'a> {
        Segmented {
            id,
            items,
            selected,
            icons_only: false,
            words_or_icons: false,
            words: false,
            fill_width: false,
            width: None,
            height: None,
        }
    }

    /// Words alone, each item as wide as its word asks, when they fit;
    /// icons alone (with the words as tooltips) when they don't (the lens
    /// strip, ui-concept §6.1, §13.10).
    pub fn words_or_icons(mut self) -> Self {
        self.words_or_icons = true;
        self
    }

    /// A fixed width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Items show their icons only, with the label as tooltip.
    pub fn icons_only(mut self, icons_only: bool) -> Self {
        self.icons_only = icons_only;
        self
    }

    /// The control takes the whole available width.
    pub fn fill_width(mut self) -> Self {
        self.fill_width = true;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// An item's label: its icon in the icon font (Fill when `selected`
    /// and icon-only), then its words unless icon-only.
    fn job(
        &self,
        ui: &egui::Ui,
        item: &Segment<'_>,
        font: &egui::FontId,
        selected: bool,
        color: egui::Color32,
    ) -> LayoutJob {
        let mut job = LayoutJob::default();
        if let Some(icon) = item.icon.filter(|_| !self.words) {
            let filled = self.icons_only && selected;
            let size = if self.icons_only {
                font.size + 4.0
            } else {
                font.size + 2.0
            };
            job.append(
                icon,
                0.0,
                egui::TextFormat::simple(fonts::icon_font(ui.ctx(), size, filled), color),
            );
            job.sections
                .last_mut()
                .iter_mut()
                .for_each(|s| s.format.valign = egui::Align::Center);
        }
        if item.icon.is_none() || !self.icons_only {
            let gap = if item.icon.is_some() && !self.words {
                5.0
            } else {
                0.0
            };
            job.append(
                item.label,
                gap,
                egui::TextFormat {
                    valign: egui::Align::Center,
                    ..egui::TextFormat::simple(font.clone(), color)
                },
            );
        }
        job
    }

    /// The width the control asks for as configured (equal items).
    pub fn natural(&self, ui: &egui::Ui) -> f32 {
        let font = TextStyle::Body.resolve(ui.style());
        self.natural_width(ui, &font, 8.0)
    }

    fn natural_width(&self, ui: &egui::Ui, font: &egui::FontId, pad: f32) -> f32 {
        let widest = self
            .items
            .iter()
            .map(|item| {
                ui.painter()
                    .layout_job(self.job(ui, item, font, false, egui::Color32::WHITE))
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max);
        (widest + 2.0 * pad) * self.items.len() as f32 + 2.0 * INSET
    }

    pub fn show(mut self, ui: &mut egui::Ui) -> SegmentedOutput {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let len = self.items.len();
        let font = TextStyle::Body.resolve(ui.style());
        let pad = 8.0;
        let mut weights = vec![1.0; len];
        let mut natural = self.natural_width(ui, &font, pad);
        if self.words_or_icons && !self.icons_only {
            self.words = true;
            // Words sit closer: 6 pt each side.
            weights = self
                .items
                .iter()
                .map(|item| {
                    let job = self.job(ui, item, &font, false, egui::Color32::WHITE);
                    ui.painter().layout_job(job).size().x + 12.0
                })
                .collect();
            natural = weights.iter().sum::<f32>() + 2.0 * INSET;
            if natural > ui.available_width() {
                self.words = false;
                self.icons_only = true;
                weights = vec![1.0; len];
                natural = self.natural_width(ui, &font, pad);
            }
        }
        let height = self.height.unwrap_or(look.metrics.control_height);
        let width = match self.width {
            Some(w) => w,
            None if self.fill_width => ui.available_width().max(natural.min(ui.available_width())),
            None => natural,
        };
        let (track, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let response = ui.interact(track, self.id, Sense::FOCUSABLE);
        let mut picked = None;

        // Keyboard: the arrows belong to this control while it has focus.
        if response.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.id,
                    EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..EventFilter::default()
                    },
                );
            });
            let keys: Vec<Key> = ui.input_mut(|i| {
                let mut keys = Vec::new();
                i.events.retain(|e| match e {
                    Event::Key {
                        key:
                            k @ (Key::ArrowLeft
                            | Key::ArrowRight
                            | Key::ArrowUp
                            | Key::ArrowDown
                            | Key::Home
                            | Key::End),
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none() => {
                        keys.push(*k);
                        false
                    }
                    _ => true,
                });
                keys
            });
            let mut at = self.selected;
            for key in keys {
                if let Some(next) = key_step(at, len, key) {
                    at = next;
                }
            }
            if at != self.selected {
                picked = Some(at);
            }
        }

        let enabled = ui.is_enabled();
        let painter = ui.painter().clone();
        if ui.is_rect_visible(track) {
            painter.rect(
                track,
                theme::R_MD,
                p.bg_well,
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Inside,
            );
        }
        // High contrast: the track's edge (bg.well is the panel's black).
        super::control_outline(ui, track, theme::R_MD);
        let shown = picked.unwrap_or(self.selected);
        let rects = segment_rects(track, &weights);
        // The thumb slides to the chosen item (instant when it first shows).
        if let Some(target) = rects.get(shown) {
            let x =
                ui.ctx()
                    .animate_value_with_time(self.id.with("thumb"), target.left(), DUR_BASE);
            let w =
                ui.ctx()
                    .animate_value_with_time(self.id.with("thumb-w"), target.width(), DUR_BASE);
            let thumb =
                Rect::from_min_size(egui::pos2(x, target.top()), egui::vec2(w, target.height()));
            if ui.is_rect_visible(thumb) {
                // Light: `bg.hover` hardly differs from the well, so the
                // thumb is lifted to `bg.card` with a hairline.
                if look.kind == theme::ThemeKind::Light {
                    painter.rect(
                        thumb,
                        theme::R_SM,
                        p.bg_card,
                        Stroke::new(1.0, p.border_subtle),
                        StrokeKind::Inside,
                    );
                } else {
                    painter.rect_filled(thumb, theme::R_SM, p.bg_hover);
                }
                // High contrast: the thumb has an edge of its own.
                super::control_outline(ui, thumb, theme::R_SM);
            }
        }
        for (i, item) in self.items.iter().enumerate() {
            let rect = rects[i];
            let item_response = ui.interact(rect, self.id.with(i), Sense::CLICK);
            if item_response.clicked() {
                response.request_focus();
                if i != self.selected {
                    picked = Some(i);
                }
            }
            let selected = i == shown;
            let hovered = item_response.hovered() && enabled;
            let color = if !enabled {
                p.text_disabled
            } else if selected || hovered {
                p.text_primary
            } else {
                p.text_secondary
            };
            let galley = painter.layout_job(self.job(ui, item, &font, selected, color));
            let at = rect.center() - galley.size() / 2.0;
            painter.galley(at, galley, color);
            let tip = match (self.icons_only, item.tooltip) {
                (true, Some(t)) => Some(format!("{} — {t}", item.label)),
                (true, None) => Some(item.label.to_owned()),
                (false, t) => t.map(str::to_owned),
            };
            if let Some(tip) = tip {
                item_response.on_hover_text(tip);
            }
        }
        if super::icon_button::keyboard_focus(ui, &response) {
            super::focus_ring(ui, track, theme::R_MD);
        }
        let current = self
            .items
            .get(shown)
            .map_or_else(String::new, |i| i.label.to_owned());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::RadioGroup, enabled, &current));
        if picked.is_some() {
            ui.ctx().request_repaint();
        }
        SegmentedOutput { picked, response }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    #[test]
    fn the_thumb_sits_on_its_item_inside_the_inset() {
        let track = Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(306.0, 28.0));
        let first = segment_thumb_rect(track, 0, 3);
        assert_eq!(first.left(), 13.0);
        assert_eq!(first.top(), 3.0);
        assert_eq!(first.height(), 22.0);
        assert_eq!(first.width(), 100.0);
        let last = segment_thumb_rect(track, 2, 3);
        assert_eq!(last.right(), track.right() - 3.0);
        assert_eq!(segment_thumb_rect(track, 9, 3), last, "clamped");
        assert_eq!(segment_thumb_rect(track, 0, 0), Rect::NOTHING);
        // Weighted: items share the inner width by their weights.
        let rects = segment_rects(track, &[1.0, 3.0]);
        assert_eq!(rects[0].width(), 75.0);
        assert_eq!(rects[1].left(), rects[0].right());
        assert_eq!(rects[1].right(), track.right() - 3.0);
    }

    use eframe::egui::{Event, Modifiers, Pos2};

    use super::*;
    use crate::ui::drag::tests::{Frames, button, key};

    #[test]
    fn arrow_keys_move_like_a_radio_group_and_wrap() {
        assert_eq!(key_step(0, 3, Key::ArrowRight), Some(1));
        assert_eq!(key_step(1, 3, Key::ArrowDown), Some(2));
        assert_eq!(key_step(2, 3, Key::ArrowRight), Some(0), "wraps forward");
        assert_eq!(key_step(0, 3, Key::ArrowLeft), Some(2), "wraps back");
        assert_eq!(key_step(2, 3, Key::ArrowUp), Some(1));
        assert_eq!(key_step(1, 3, Key::Home), Some(0));
        assert_eq!(key_step(1, 3, Key::End), Some(2));
        assert_eq!(key_step(1, 3, Key::Enter), None);
        assert_eq!(key_step(0, 0, Key::ArrowRight), None, "no items");
        assert_eq!(key_step(9, 3, Key::ArrowLeft), Some(1), "clamped first");
        assert_eq!(key_step(0, 1, Key::ArrowRight), Some(0));
    }

    #[test]
    fn a_click_focuses_the_control_and_then_the_arrows_pick() {
        let items = [
            Segment::new(None, "Left"),
            Segment::new(None, "Center"),
            Segment::new(None, "Right"),
        ];
        let mut selected = 0;
        let mut frames = Frames::new();
        let mut rect = Rect::NOTHING;
        let mut frame = |frames: &mut Frames, events: Vec<Event>, selected: &mut usize| {
            frames.run(events, |ui| {
                let out = Segmented::new(Id::new("seg"), &items, *selected).show(ui);
                rect = out.response.rect;
                if let Some(i) = out.picked {
                    *selected = i;
                }
            });
            rect
        };
        let rect = frame(&mut frames, Vec::new(), &mut selected);
        // Arrows do nothing before the control has focus.
        frame(
            &mut frames,
            vec![key(Key::ArrowRight, Modifiers::NONE)],
            &mut selected,
        );
        assert_eq!(selected, 0);
        let third = Pos2::new(rect.right() - 6.0, rect.center().y);
        frame(
            &mut frames,
            vec![Event::PointerMoved(third), button(third, true)],
            &mut selected,
        );
        frame(&mut frames, vec![button(third, false)], &mut selected);
        assert_eq!(selected, 2, "clicked Right");
        frame(
            &mut frames,
            vec![key(Key::ArrowRight, Modifiers::NONE)],
            &mut selected,
        );
        assert_eq!(selected, 0, "wrapped to Left");
        frame(
            &mut frames,
            vec![
                key(Key::ArrowRight, Modifiers::NONE),
                key(Key::ArrowRight, Modifiers::NONE),
                key(Key::ArrowLeft, Modifiers::NONE),
            ],
            &mut selected,
        );
        assert_eq!(selected, 1);
        frame(
            &mut frames,
            vec![key(Key::End, Modifiers::NONE)],
            &mut selected,
        );
        assert_eq!(selected, 2);
    }
}

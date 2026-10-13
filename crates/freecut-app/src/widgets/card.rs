//! The property card (ui-concept §6.2, §13.1), drawn as a full-width
//! **section**: no radius and no gap; a 1 pt `border.subtle` hairline above
//! every section but the first; a header band (36 pt, 30 Compact) in
//! `bg.card` that turns `bg.field` on hover over `dur.fast` and folds the
//! section on a click. Left to right the band holds the title at the side
//! inset (so titles line up with row labels), the chevron after it, then
//! — at the far end, 2 pt apart — the summary while folded (caption,
//! `text.tertiary`, ellipsis), the help badge, the reset badge, the
//! optional layer's switch and the `...` menu. Badges, the switch and the
//! menu take their own clicks, so they never fold the section. The body
//! sits 14 pt (10 Compact) in from both sides, 4 pt below the band and
//! 8 pt above the next hairline; advanced rows hide behind *More (n)*.
//!
//! The card doesn't own its open state: the inspector keeps a
//! [`CardMemory`] per (selection kind, card) in workspace state (UI-008),
//! so the state survives selection changes. A switched-off card collapses
//! to its header and its summary says so; its values stay.

use eframe::egui::{
    self, Align, Align2, Frame, Id, Key, Layout, Margin, Modifiers, Sense, Stroke, TextStyle,
    UiBuilder, WidgetInfo, WidgetType,
};

use super::help::{help_badge, reset_badge};
use super::toggle::toggle;
use crate::theme::{self, Type, fonts, icons};

/// A card's remembered state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardMemory {
    pub open: bool,
    /// *More (n)* is expanded.
    pub more: bool,
}

impl CardMemory {
    pub fn new(open: bool) -> CardMemory {
        CardMemory { open, more: false }
    }
}

type MenuFn<'a> = Box<dyn FnOnce(&mut egui::Ui) + 'a>;

/// A property card.
pub struct PropertyCard<'a> {
    id: Id,
    title: &'a str,
    summary: String,
    keyed: bool,
    switch: Option<(bool, &'a str)>,
    menu: Option<MenuFn<'a>>,
    help: Option<&'a str>,
    reset: Option<(bool, &'a str)>,
}

impl std::fmt::Debug for PropertyCard<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PropertyCard")
            .field("id", &self.id)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// What a card reports.
#[derive(Debug)]
pub struct CardOutput<R> {
    /// The body's result, when the body was drawn.
    pub inner: Option<R>,
    /// The switch was flipped to this state.
    pub switched: Option<bool>,
    /// The reset badge was clicked.
    pub reset: bool,
}

impl<'a> PropertyCard<'a> {
    pub fn new(id: Id, title: &'a str) -> PropertyCard<'a> {
        PropertyCard {
            id,
            title,
            summary: String::new(),
            keyed: false,
            switch: None,
            menu: None,
            help: None,
            reset: None,
        }
    }

    /// A help badge explaining the section.
    pub fn help(mut self, text: &'a str) -> Self {
        self.help = Some(text);
        self
    }

    /// A reset badge: `dirty` when something differs from the defaults
    /// (dimmed otherwise), `tooltip` saying what it resets.
    pub fn reset(mut self, dirty: bool, tooltip: &'a str) -> Self {
        self.reset = Some((dirty, tooltip));
        self
    }

    /// The chip shown while the card is collapsed ("2 px · black").
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// The card's properties have keyframes: the collapsed chip starts with
    /// a diamond (ui-concept §13.6, shape not colour alone).
    pub fn keyed(mut self, keyed: bool) -> Self {
        self.keyed = keyed;
        self
    }

    /// An on/off switch in the header, with its tooltip. Off collapses the
    /// card.
    pub fn switch(mut self, on: bool, tooltip: &'a str) -> Self {
        self.switch = Some((on, tooltip));
        self
    }

    /// The `...` menu's contents.
    pub fn menu(mut self, menu: impl FnOnce(&mut egui::Ui) + 'a) -> Self {
        self.menu = Some(Box::new(menu));
        self
    }

    pub fn show<R>(
        self,
        ui: &mut egui::Ui,
        memory: &mut CardMemory,
        body: impl FnOnce(&mut egui::Ui, &mut CardMemory) -> R,
    ) -> CardOutput<R> {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let m = look.metrics;
        let inset = m.card_padding;
        let mut switched = None;
        let mut reset = false;
        let mut inner = None;

        // A hairline above every section but the first in its stack.
        let first = ui.cursor().top() <= ui.max_rect().top() + 0.5;
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, m.card_header), Sense::hover());
        let header = ui.interact(rect, self.id.with("header"), Sense::click());
        // Keyboard: Enter/Space toggle (a click), Right/Left expand and
        // collapse.
        if header.has_focus() {
            // The arrows act on the section instead of moving focus.
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    header.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        ..egui::EventFilter::default()
                    },
                );
            });
            let (right, left) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowRight),
                    i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                )
            });
            if right {
                memory.open = true;
            }
            if left {
                memory.open = false;
            }
        }
        if header.clicked() {
            memory.open = !memory.open;
        }
        let on = self.switch.is_none_or(|(on, _)| on);
        let expanded = memory.open && on;
        let title = self.title.to_owned();
        header.widget_info(|| {
            WidgetInfo::selected(WidgetType::CollapsingHeader, true, expanded, &title)
        });

        let painter = ui.painter().clone();
        let lit = ui.ctx().animate_bool_with_time(
            self.id.with("band"),
            header.hovered(),
            ui.style().animation_time,
        );
        painter.rect_filled(rect, 0.0, p.bg_card.lerp_to_gamma(p.bg_field, lit));
        if !first {
            painter.hline(
                rect.x_range(),
                rect.top() + 0.5,
                Stroke::new(1.0, p.border_subtle),
            );
        }
        let title_font = Type::Label.style(ui.style()).resolve(ui.style());
        let color = if ui.is_enabled() {
            p.text_primary
        } else {
            p.text_secondary
        };
        let title_galley = painter.layout_no_wrap(self.title.to_owned(), title_font, color);
        let title_w = title_galley.size().x;
        let title_at = egui::pos2(
            rect.left() + inset,
            rect.center().y - title_galley.size().y / 2.0,
        );
        painter.galley(title_at, title_galley, color);
        let chevron = if expanded {
            icons::CARET_DOWN
        } else {
            icons::CARET_RIGHT
        };
        painter.text(
            egui::pos2(title_at.x + title_w + 10.0, rect.center().y),
            Align2::CENTER_CENTER,
            chevron,
            fonts::icon_font(ui.ctx(), 12.0, false),
            p.text_secondary,
        );
        if super::icon_button::keyboard_focus(ui, &header) {
            super::focus_ring(ui, rect.shrink(2.0), theme::R_SM);
        }

        // The far end: menu, switch, reset, help, then the summary.
        let mut controls = rect.shrink2(egui::vec2(inset - 2.0, 0.0));
        controls.min.x = title_at.x + title_w + 20.0;
        let mut strip = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id.with("actions"))
                .max_rect(controls)
                .layout(Layout::right_to_left(Align::Center)),
        );
        strip.spacing_mut().item_spacing.x = 2.0;
        if let Some(menu) = self.menu {
            strip.push_id(self.id.with("menu"), |ui| {
                let button = egui::Button::new(
                    egui::RichText::new(icons::DOTS_THREE)
                        .font(fonts::icon_font(ui.ctx(), 14.0, false))
                        .color(p.text_secondary),
                )
                .frame(false)
                .min_size(egui::vec2(24.0, 24.0));
                let response = egui::containers::menu::MenuButton::from_button(button)
                    .ui(ui, menu)
                    .0
                    .on_hover_text("More actions");
                super::control_outline(ui, response.rect, theme::R_SM);
            });
        }
        if let Some((on, tooltip)) = self.switch {
            let mut state = on;
            let response = toggle(&mut strip, self.id.with("switch"), &mut state, self.title)
                .on_hover_text(tooltip);
            if response.changed() {
                switched = Some(state);
            }
        }
        if let Some((dirty, tooltip)) = self.reset {
            reset = reset_badge(&mut strip, self.id.with("reset"), dirty, tooltip);
        }
        if let Some(text) = self.help {
            help_badge(&mut strip, self.id.with("help"), text);
        }
        if !expanded && (self.keyed || !self.summary.is_empty()) {
            strip.add_space(6.0);
            let response = summary_chip(&mut strip, &self.summary, self.keyed);
            if self.keyed {
                response.on_hover_text("This section has keyframes");
            }
        }

        if expanded {
            let frame = Frame::new().inner_margin(Margin {
                left: inset as i8,
                right: inset as i8,
                top: 4,
                bottom: 8,
            });
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = m.row_gap;
                inner = Some(body(ui, memory));
            });
        }
        CardOutput {
            inner,
            switched,
            reset,
        }
    }
}

/// The summary of a folded section (ui-concept §6.2): `type.caption`
/// with tabular figures in `text.tertiary`, cut short with an ellipsis; a
/// leading diamond (in the icon font) when the section has keyframes.
pub fn summary_chip(ui: &mut egui::Ui, text: &str, keyed: bool) -> egui::Response {
    let p = theme::look(ui.ctx()).palette;
    let font = Type::Caption.style(ui.style()).resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    if keyed {
        job.append(
            icons::DIAMOND,
            0.0,
            egui::TextFormat::simple(fonts::icon_font(ui.ctx(), font.size, false), p.accent_text),
        );
    }
    job.append(
        text,
        if keyed { 4.0 } else { 0.0 },
        egui::TextFormat::simple(font, p.text_tertiary),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width().max(0.0));
    ui.add(egui::Label::new(job).truncate())
}

/// The card's advanced rows (ui-concept §6.2): shown in place behind
/// *More (n)* with the Detail switch on Essentials, always with
/// Everything. `count` is the number of rows behind it.
pub fn more(
    ui: &mut egui::Ui,
    id: Id,
    memory: &mut CardMemory,
    everything: bool,
    count: usize,
    rows: impl FnOnce(&mut egui::Ui),
) {
    if everything {
        rows(ui);
        return;
    }
    if memory.more {
        rows(ui);
    }
    let p = theme::look(ui.ctx()).palette;
    let text = if memory.more {
        "Less".to_owned()
    } else {
        format!("More ({count})")
    };
    let link = ui.push_id(id.with("more"), |ui| {
        ui.add(
            egui::Button::new(
                egui::RichText::new(text)
                    .text_style(TextStyle::Small)
                    .color(p.accent_text),
            )
            .frame(false)
            .min_size(egui::vec2(0.0, 24.0)),
        )
    });
    super::control_outline(
        ui,
        link.inner.rect.expand2(egui::vec2(4.0, 0.0)),
        theme::R_SM,
    );
    if link.inner.clicked() {
        memory.more = !memory.more;
    }
}

//! The empty state (ui-concept §13.14): centred in its region, a 28 pt
//! line icon in `text.tertiary`, a `type.label` sentence saying what goes
//! here, a smaller line on why it is empty, one primary action and an
//! optional secondary link.

use eframe::egui::{self, RichText, TextStyle};

use crate::theme::{self, Type, fonts};

/// Which of the empty state's actions was clicked.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EmptyOutput {
    pub primary: bool,
    pub secondary: bool,
}

pub fn empty_state(ui: &mut egui::Ui, icon: &str, title: &str, detail: &str) {
    empty_state_with(ui, icon, title, detail, None, None);
}

/// The empty state with a primary action button and a secondary link.
pub fn empty_state_with(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    detail: &str,
    primary: Option<&str>,
    secondary: Option<&str>,
) -> EmptyOutput {
    let p = theme::look(ui.ctx()).palette;
    let mut out = EmptyOutput::default();
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.label(
            RichText::new(icon)
                .font(fonts::icon_font(ui.ctx(), 28.0, false))
                .color(p.text_tertiary),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(title)
                .text_style(Type::Label.style(ui.style()))
                .color(p.text_primary),
        );
        if !detail.is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(detail)
                    .text_style(TextStyle::Small)
                    .color(p.text_secondary),
            );
        }
        if let Some(label) = primary {
            ui.add_space(12.0);
            out.primary = ui.button(label).clicked();
        }
        if let Some(label) = secondary {
            ui.add_space(4.0);
            out.secondary = ui
                .add(egui::Button::new(RichText::new(label).color(p.accent_text)).frame(false))
                .clicked();
        }
        ui.add_space(24.0);
    });
    out
}

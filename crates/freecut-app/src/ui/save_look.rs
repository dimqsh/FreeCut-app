//! **Save look…** (CL-026 on UI-064's save-as-preset sheet): a name field
//! with a suggested name, the sections to save as chips (pre-chosen: the
//! ones with changes), what is never saved, and *Save*. `Enter` saves,
//! `Esc` cancels. The app writes the file on the job pool and says where
//! it went.
//!
//! The values saved are the clip's at the playhead; keyframes are not
//! saved (CL-026), and the sheet says so when the clip has colour keys.
//! Filter has no values in this build, so it isn't offered (UI-004's rule
//! for features that aren't built).

use eframe::egui::{self, Id};
use freecut_project::look::MAX_LOOK_NAME_CHARS;
use freecut_timeline::model::{ColorStack, Section, SectionMask};

use crate::theme;
use crate::ui::look::section_label;
use crate::widgets::sheet::{self, ButtonKind, Sheet};

/// The sections the sheet offers, in display order.
pub const OFFERED: [Section; 6] = [
    Section::Light,
    Section::Color,
    Section::Lut,
    Section::Hsl,
    Section::Curves,
    Section::Wheels,
];

/// CL-026's refusal when nothing has changed.
pub const NOTHING_TO_SAVE: &str = "Nothing to save — all color settings are at defaults";

/// The open sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveLookDialog {
    pub name: String,
    /// The grade being saved: the clip's values at the playhead.
    pub stack: ColorStack,
    /// The chosen sections.
    pub sections: SectionMask,
    /// The clip has colour keyframes (they aren't saved).
    pub keyed: bool,
    focused: bool,
}

/// What the sheet decided.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveLookAnswer {
    Save {
        name: String,
        stack: Box<ColorStack>,
        sections: SectionMask,
    },
    Cancel,
}

impl SaveLookDialog {
    /// The sheet for `stack`, the changed sections chosen, named `suggested`.
    pub fn new(stack: ColorStack, keyed: bool, suggested: String) -> SaveLookDialog {
        SaveLookDialog {
            name: suggested,
            sections: stack.changed_sections(&OFFERED),
            stack,
            keyed,
            focused: false,
        }
    }

    /// Why *Save* is disabled, if it is.
    pub fn refusal(&self) -> Option<&'static str> {
        let changed = self.stack.changed_sections(&OFFERED);
        if changed == SectionMask::NONE {
            Some(NOTHING_TO_SAVE)
        } else if self.sections == SectionMask::NONE {
            Some("Choose at least one section to save")
        } else if self.name.trim().is_empty() {
            Some("Give the look a name")
        } else {
            None
        }
    }

    fn answer(&self) -> SaveLookAnswer {
        SaveLookAnswer::Save {
            name: self.name.trim().to_owned(),
            stack: Box::new(self.stack.clone()),
            sections: self.sections,
        }
    }

    /// Shows the sheet; `Some` once it is answered.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<SaveLookAnswer> {
        let mut answer = None;
        let refusal = self.refusal();
        let name_id = Id::new("save-look-name");
        let response = Sheet::new(Id::new("save-look"), "Save look")
            .width(480.0)
            .show(ctx, |s| {
                s.body(|ui| {
                    let p = theme::look(ui.ctx()).palette;
                    sheet::form_row(ui, "Name", false, |ui| {
                        let edit = egui::TextEdit::singleline(&mut self.name)
                            .id(name_id)
                            .char_limit(MAX_LOOK_NAME_CHARS)
                            .desired_width(ui.available_width());
                        let r = ui.add(edit);
                        if !self.focused {
                            r.request_focus();
                            self.focused = true;
                        }
                    });
                    sheet::form_row(ui, "Sections", false, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                            for section in OFFERED {
                                let on = self.sections.is_on(section);
                                let changed = !self.stack.section_is_default(section);
                                let chip = sheet::choice_chip(ui, section_label(section), None, on)
                                    .on_hover_text(if changed {
                                        "Has changes"
                                    } else {
                                        "At its defaults: saving it resets this section where \
                                         the look is applied"
                                    });
                                if chip.clicked() {
                                    self.sections = self.sections.with(section, !on);
                                }
                            }
                        });
                    });
                    sheet::form_row(ui, "Not saved", true, |ui| {
                        let note = |ui: &mut egui::Ui, text: &str| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text)
                                        .text_style(egui::TextStyle::Small)
                                        .color(p.text_secondary),
                                )
                                .wrap(),
                            );
                        };
                        note(
                            ui,
                            "Amount, Auto, Match and colour masks: they belong to one clip.",
                        );
                        if self.keyed {
                            note(
                                ui,
                                "Keyframes: the values at the playhead are saved instead.",
                            );
                        }
                    });
                });
                s.footer(|ui| {
                    let save = sheet::button(
                        ui,
                        ButtonKind::Primary,
                        None,
                        "Save",
                        96.0,
                        None,
                        refusal.is_none(),
                    );
                    let save = match refusal {
                        Some(why) => save.on_hover_text(why),
                        None => save,
                    };
                    if save.clicked() {
                        answer = Some(self.answer());
                    }
                    if sheet::button(ui, ButtonKind::Secondary, None, "Cancel", 80.0, None, true)
                        .clicked()
                    {
                        answer = Some(SaveLookAnswer::Cancel);
                    }
                    if let Some(why) = refusal {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let p = theme::look(ui.ctx()).palette;
                            ui.label(
                                egui::RichText::new(why)
                                    .text_style(egui::TextStyle::Small)
                                    .color(p.text_tertiary),
                            );
                        });
                    }
                });
            });
        if answer.is_none()
            && refusal.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            answer = Some(self.answer());
        }
        if response.close && answer.is_none() {
            answer = Some(SaveLookAnswer::Cancel);
        }
        answer
    }
}

/// The name the sheet suggests: "Look N" after the `saved` looks there
/// are.
pub fn suggested_name(saved: usize) -> String {
    format!("Look {}", saved + 1)
}

#[cfg(test)]
mod tests {
    use freecut_timeline::model::ColorParamId;

    use super::*;

    #[test]
    fn the_changed_sections_are_chosen_and_nothing_changed_is_refused() {
        let stack = ColorStack::default()
            .with(ColorParamId::Exposure, 10)
            .with(ColorParamId::Amount, 50);
        let d = SaveLookDialog::new(stack, false, suggested_name(2));
        assert_eq!(d.name, "Look 3");
        assert_eq!(d.sections, SectionMask::of(&[Section::Light]));
        assert_eq!(d.refusal(), None);
        let only_amount = ColorStack::default().with(ColorParamId::Amount, 50);
        let d = SaveLookDialog::new(only_amount, false, "A".into());
        assert_eq!(d.refusal(), Some(NOTHING_TO_SAVE));
        let mut d = SaveLookDialog::new(
            ColorStack::default().with(ColorParamId::Hue, 5),
            true,
            " ".into(),
        );
        assert_eq!(d.refusal(), Some("Give the look a name"));
        d.name = "Mine".into();
        d.sections = SectionMask::NONE;
        assert_eq!(d.refusal(), Some("Choose at least one section to save"));
    }
}

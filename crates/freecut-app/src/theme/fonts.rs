//! The UI fonts (ui-concept §12.6, §14): Inter for text, JetBrains Mono
//! for timecode, the Phosphor icon font, and Noto Sans (bundled for text
//! clips) as the fallback for scripts Inter lacks. All are compiled in and
//! listed in `assets/ASSETS.md`.
//!
//! egui has no font weights, so each weight is a family of its own:
//! [`MEDIUM`] and [`SEMIBOLD`] next to the stock proportional family.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// The family of Inter Medium (`type.caption`).
pub const MEDIUM: &str = "freecut-medium";
/// The family of Inter Semibold (`type.label`, `type.title`, buttons).
pub const SEMIBOLD: &str = "freecut-semibold";
/// Phosphor Regular only (icons drawn on their own).
pub const ICONS: &str = "freecut-icons";
/// Phosphor Fill: the icon of a selected or "on" state (§14).
pub const ICONS_FILL: &str = "freecut-icons-fill";

const INTER_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../../assets/fonts/Inter-SemiBold.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf");
const PHOSPHOR: &[u8] = include_bytes!("../../../../assets/fonts/Phosphor.ttf");
const PHOSPHOR_FILL: &[u8] = include_bytes!("../../../../assets/fonts/Phosphor-Fill.ttf");

/// A bundled asset's licence notice, shown in the About window (ui-concept
/// §14, UI-051): every distributed binary carries the font and icon
/// licences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice {
    /// What the notice covers.
    pub name: &'static str,
    pub license: &'static str,
    /// The licence file as shipped (`assets/fonts`).
    pub text: &'static str,
}

/// The notices of the fonts and icons compiled into FreeCut.
pub const NOTICES: [Notice; 4] = [
    Notice {
        name: "Inter",
        license: "SIL Open Font License 1.1",
        text: include_str!("../../../../assets/fonts/Inter-OFL.txt"),
    },
    Notice {
        name: "JetBrains Mono",
        license: "SIL Open Font License 1.1",
        text: include_str!("../../../../assets/fonts/JetBrainsMono-OFL.txt"),
    },
    Notice {
        name: "Noto Sans",
        license: "SIL Open Font License 1.1",
        text: include_str!("../../../../assets/fonts/OFL.txt"),
    },
    Notice {
        name: "Phosphor Icons",
        license: "MIT",
        text: include_str!("../../../../assets/fonts/Phosphor-LICENSE.txt"),
    },
];

/// The Noto Sans face `freecut-render` already compiles in (no second
/// copy in the binary).
fn noto_sans() -> Option<&'static [u8]> {
    freecut_render::text::BUNDLED_FONTS
        .iter()
        .find(|f| f.file == "NotoSans-Regular.ttf")
        .map(|f| f.data)
}

/// FreeCut's font definitions: Inter first, then the icon font (its
/// private-use codepoints never clash with text), Noto Sans, and egui's own
/// fonts (emoji) last.
pub fn definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut add = |name: &str, data: &'static [u8]| {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(data)));
    };
    add("inter", INTER_REGULAR);
    add("inter-medium", INTER_MEDIUM);
    add("inter-semibold", INTER_SEMIBOLD);
    add("jetbrains-mono", JETBRAINS_MONO);
    add("phosphor", PHOSPHOR);
    add("phosphor-fill", PHOSPHOR_FILL);
    let noto = noto_sans();
    if let Some(data) = noto {
        add("noto-sans", data);
    }
    let stock_proportional = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let stock_mono = defs
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let chain = |first: &str, rest: &[String]| -> Vec<String> {
        let mut names = vec![first.to_owned(), "phosphor".to_owned()];
        if noto.is_some() {
            names.push("noto-sans".to_owned());
        }
        names.extend(rest.iter().cloned());
        names
    };
    let proportional = chain("inter", &stock_proportional);
    let medium = chain("inter-medium", &stock_proportional);
    let semibold = chain("inter-semibold", &stock_proportional);
    let mono = chain("jetbrains-mono", &stock_mono);
    defs.families.insert(FontFamily::Proportional, proportional);
    defs.families.insert(FontFamily::Monospace, mono);
    defs.families
        .insert(FontFamily::Name(MEDIUM.into()), medium);
    defs.families
        .insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    defs.families
        .insert(FontFamily::Name(ICONS.into()), vec!["phosphor".to_owned()]);
    defs.families.insert(
        FontFamily::Name(ICONS_FILL.into()),
        vec!["phosphor-fill".to_owned()],
    );
    defs
}

fn installed_id() -> egui::Id {
    egui::Id::new("freecut-fonts-installed")
}

/// Installs [`definitions`]. They take effect at the start of the next
/// pass, so call this before the first frame (the app does it while the
/// window is created), before [`super::apply`] refers to the families.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
    ctx.data_mut(|d| d.insert_temp(installed_id(), true));
}

/// Whether [`install`] ran: the named families exist.
pub fn installed(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(installed_id()))
        .unwrap_or(false)
}

/// The Phosphor Regular font at `size` pt.
pub fn icon(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// The Phosphor Fill font at `size` pt.
pub fn icon_fill(size: f32) -> egui::FontId {
    egui::FontId::new(size, FontFamily::Name(ICONS_FILL.into()))
}

/// The icon font for painting, falling back to the proportional family
/// where FreeCut's fonts aren't installed (headless tests).
pub fn icon_font(ctx: &egui::Context, size: f32, filled: bool) -> egui::FontId {
    match (installed(ctx), filled) {
        (true, false) => icon(size),
        (true, true) => icon_fill(size),
        (false, _) => egui::FontId::proportional(size),
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{FontId, RawInput};

    use super::*;
    use crate::theme::icons;

    #[test]
    fn every_icon_is_in_both_phosphor_weights_and_text_has_its_fonts() {
        let ctx = egui::Context::default();
        install(&ctx);
        for _ in 0..2 {
            let mut output = ctx.run_ui(RawInput::default(), |_| {});
            // No renderer here to take the font atlas upload.
            output.textures_delta.clear();
        }
        let all: String = icons::ALL.concat();
        ctx.fonts_mut(|f| {
            assert!(f.has_glyphs(&icon(16.0), &all), "Phosphor Regular");
            assert!(f.has_glyphs(&icon_fill(16.0), &all), "Phosphor Fill");
            for family in [
                FontFamily::Proportional,
                FontFamily::Monospace,
                FontFamily::Name(MEDIUM.into()),
                FontFamily::Name(SEMIBOLD.into()),
            ] {
                let font = FontId::new(13.0, family.clone());
                assert!(f.has_glyphs(&font, "Volume 0:02.0"), "{family:?}");
                // Icons render inside ordinary labels too.
                assert!(f.has_glyphs(&font, icons::TEXT), "{family:?}");
            }
        });
        assert!(installed(&ctx));
        assert!(noto_sans().is_some(), "the render crate's Noto Sans");
    }
}

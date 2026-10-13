//! Design tokens (UI-013, UI-014; ui-concept §12) and their mapping to egui
//! (ui-concept §16.3).
//!
//! The colour tokens of the three themes are `const` tables ([`TOKENS`]);
//! the spacing metrics of the two densities are [`Density::metrics`].
//! [`apply`] maps one theme and one density to egui's `Style` and stores the
//! [`Look`] the custom widgets paint with. It runs when the choice changes
//! ([`ThemeState::sync`]), never per frame. Widgets never use raw colours:
//! they read [`look`].

pub mod fonts;
pub mod icons;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme, Vec2,
};

/// One theme's colour tokens (ui-concept §12.1, §12.2). Field names follow
/// the token names: `bg_app` is `bg.app`, `text_on_accent` is
/// `text.on-accent`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// The ground: top bar, gutters, status line (rev. 2).
    pub bg_app: Color32,
    pub bg_panel: Color32,
    pub bg_card: Color32,
    pub bg_well: Color32,
    /// Controls at rest: fields, selects, buttons, hover chips (rev. 2,
    /// one step above the panel).
    pub bg_field: Color32,
    pub bg_hover: Color32,
    pub bg_active: Color32,
    /// Hairlines; translucent in Dark and Light so one value works on
    /// every surface.
    pub border_subtle: Color32,
    /// Popover and menu outlines, hovered tile borders, the canvas ring,
    /// scrollbar thumbs (rev. 2).
    pub border_strong: Color32,
    pub border_control: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    /// Disabled labels: exempt from the contrast rules.
    pub text_disabled: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_text: Color32,
    pub text_on_accent: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub success: Color32,
    /// The focus ring's width: 2 pt, 3 pt in High contrast (§15).
    pub focus_width: f32,
    /// Opacity of the `e2` / `e3` shadows (§12.7), 0–255.
    pub shadow_e2: u8,
    pub shadow_e3: u8,
    /// `overlay.hover`: the wash on custom-painted hovered areas.
    pub overlay_hover: Color32,
    /// `playhead` (§12.3).
    pub playhead: Color32,
    /// The outline of a selected clip on the timeline (`clip.selected`):
    /// the accent, white in High contrast (away from its yellow accent).
    pub clip_selected: Color32,
    /// `bg.lane` / `bg.lane-alt`: alternating timeline lanes (§12.3).
    pub bg_lane: Color32,
    pub bg_lane_alt: Color32,
    /// `bg.ruler`: the timeline ruler and the header column's corner.
    pub bg_ruler: Color32,
    /// `timeline.grid`: lines through the lanes at labelled ruler ticks.
    pub timeline_grid: Color32,
    /// `overlay.badge`: badges and pills over pictures, with white text.
    pub overlay_badge: Color32,
    /// `overlay.scrim`: behind modal sheets only.
    pub scrim: Color32,
}

impl Palette {
    /// `overlay.selection`: `accent` at 22 %.
    pub fn overlay_selection(&self) -> Color32 {
        self.accent.gamma_multiply(0.22)
    }
}

/// Motion tokens (ui-concept §12.8) and Reduce motion (UI-043): with it
/// on, every custom fade and slide is instant.
pub mod motion {
    use eframe::egui;

    /// `dur.slow`: sheets and toasts arriving.
    pub const SLOW: f32 = 0.24;
    /// `dur.exit`: toasts leaving.
    pub const EXIT: f32 = 0.36;

    fn id() -> egui::Id {
        egui::Id::new("freecut-reduce-motion")
    }

    /// Sets Reduce motion for this frame on (the app calls it per frame
    /// from the preference).
    pub fn set_reduced(ctx: &egui::Context, reduced: bool) {
        ctx.data_mut(|d| d.insert_temp(id(), reduced));
    }

    /// Whether Reduce motion is on.
    pub fn reduced(ctx: &egui::Context) -> bool {
        ctx.data(|d| d.get_temp::<bool>(id())).unwrap_or(false)
    }

    /// `secs`, or 0 with Reduce motion on.
    pub fn duration(ctx: &egui::Context, secs: f32) -> f32 {
        if reduced(ctx) { 0.0 } else { secs }
    }

    /// Ease-out cubic on 0..=1.
    pub fn ease_out(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }

    /// Ease-in cubic on 0..=1.
    pub fn ease_in(t: f32) -> f32 {
        t.clamp(0.0, 1.0).powi(3)
    }
}

/// The editing-surface colours every theme shares (ui-concept §12.3).
pub mod surface {
    use super::{Color32, hex};

    /// Video and image clips.
    pub const CLIP_VIDEO: Color32 = hex(0x2F6F8F);
    /// Audio clips imported as files.
    pub const CLIP_AUDIO: Color32 = hex(0x2F7D5B);
    /// Sound extracted from a video: AU-001 colours audio by origin, so
    /// it is a bluer shade of `clip.audio`.
    pub const CLIP_AUDIO_EXTRACTED: Color32 = hex(0x2A7474);
    pub const CLIP_TEXT: Color32 = hex(0x9C6B1E);
    /// Stickers and shapes.
    pub const CLIP_ELEMENT: Color32 = hex(0x7A4FA3);
    pub const CLIP_EFFECT: Color32 = hex(0xA3476B);
    /// Filter and adjustment layers.
    pub const CLIP_ADJUST: Color32 = hex(0x4F5DB3);
    /// Around the canvas, in every theme (UI-014).
    pub const PLAYER_SURROUND: Color32 = hex(0x1E1E1E);
    /// FreeCut's 8-slot palette for markers, track colours and clip labels
    /// (TL-037, TL-049, TL-045), in `PaletteColor` order: Blue, Teal,
    /// Green, Yellow, Orange, Red, Pink, Purple. Bright enough to read on
    /// the ruler and on clip strips in every theme.
    pub const PALETTE: [Color32; 8] = [
        hex(0x4C8DF6),
        hex(0x2EC4B6),
        hex(0x3BB273),
        hex(0xF2C230),
        hex(0xFF8A3D),
        hex(0xF2545B),
        hex(0xE64AA0),
        hex(0x9B5DE5),
    ];

    /// The colour of palette slot `c`.
    pub fn palette(c: freecut_timeline::model::PaletteColor) -> Color32 {
        let i = freecut_timeline::model::PaletteColor::ALL
            .iter()
            .position(|x| *x == c)
            .unwrap_or(0);
        PALETTE[i]
    }
    /// The dark one of the two clip label colours.
    pub const LABEL_DARK: Color32 = hex(0x111214);

    /// The colour popover's palette row (ui-concept §13.5): FreeCut's own
    /// neutrals, then a hue wheel.
    pub const SWATCHES: [Color32; 16] = [
        hex(0xFFFFFF),
        hex(0xD9D9D9),
        hex(0x8C8C8C),
        hex(0x404040),
        hex(0x000000),
        hex(0xFFD23F),
        hex(0xFF8A3D),
        hex(0xF2545B),
        hex(0xE64AA0),
        hex(0x9B5DE5),
        hex(0x5B7CFA),
        hex(0x3FA7F5),
        hex(0x2EC4B6),
        hex(0x3BB273),
        hex(0xA3D65C),
        hex(0x7A5C3E),
    ];
    /// The two cells of the checkerboard under a translucent swatch.
    pub const CHECKER_LIGHT: Color32 = hex(0xC8C8C8);
    pub const CHECKER_DARK: Color32 = hex(0x8C8C8C);

    /// The label colour on a clip of `fill` (UI-013): white or `#111214`,
    /// whichever contrasts more.
    pub fn label_on(fill: Color32) -> Color32 {
        let l = super::luminance(fill);
        let on_white = 1.05 / (l + 0.05);
        let on_dark = (l + 0.05) / (super::luminance(LABEL_DARK) + 0.05);
        if on_white >= on_dark {
            Color32::WHITE
        } else {
            LABEL_DARK
        }
    }

    /// `a` mixed `t` (0..=1) of the way towards the opaque `b`.
    fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
        // Each channel stays between the two inputs, within 0..=255.
        let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
        Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
    }

    /// `clip.well` (rev. 2): a clip body under its name strip, and a
    /// media tile's background: the kind colour `mark` at 30 % over
    /// `bg.well` (Dark), 18 % over `bg.panel` (Light), black in High
    /// contrast.
    pub fn clip_well(mark: Color32, kind: super::ThemeKind) -> Color32 {
        use super::{TOKENS, ThemeKind};
        match kind {
            ThemeKind::Dark => mix(TOKENS.dark.bg_well, mark, 0.30),
            ThemeKind::Light => mix(TOKENS.light.bg_panel, mark, 0.18),
            ThemeKind::HighContrast => Color32::BLACK,
        }
    }

    /// `clip.wave` (rev. 2): waveform bars on a clip body: the kind colour
    /// mixed 50 % towards white (Dark, High contrast) or 30 % towards black
    /// (Light), at 85 %.
    pub fn clip_wave(mark: Color32, kind: super::ThemeKind) -> Color32 {
        use super::ThemeKind;
        let solid = match kind {
            ThemeKind::Light => mix(mark, Color32::BLACK, 0.30),
            ThemeKind::Dark | ThemeKind::HighContrast => mix(mark, Color32::WHITE, 0.50),
        };
        solid.gamma_multiply(0.85)
    }
}

/// WCAG 2.x relative luminance of an opaque sRGB colour.
pub fn luminance(c: Color32) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// The three themes' tokens (UI-013, UI-014).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub dark: Palette,
    pub light: Palette,
    pub high_contrast: Palette,
}

impl Tokens {
    pub fn palette(&self, kind: ThemeKind) -> &Palette {
        match kind {
            ThemeKind::Dark => &self.dark,
            ThemeKind::Light => &self.light,
            ThemeKind::HighContrast => &self.high_contrast,
        }
    }
}

const fn hex(rgb: u32) -> Color32 {
    // Each byte is masked to 0..=255 first, so the casts are exact.
    Color32::from_rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    )
}

/// The token tables of ui-concept §12.1 and §12.2.
pub const TOKENS: Tokens = Tokens {
    dark: Palette {
        bg_app: hex(0x101114),
        bg_panel: hex(0x1C1D22),
        bg_card: hex(0x24262B),
        bg_well: hex(0x15161A),
        bg_field: hex(0x2B2D33),
        bg_hover: hex(0x33353C),
        bg_active: hex(0x3A3C43),
        // White at 8 % and 16 %.
        border_subtle: Color32::from_rgba_premultiplied(20, 20, 20, 20),
        border_strong: Color32::from_rgba_premultiplied(41, 41, 41, 41),
        border_control: hex(0x71757F),
        text_primary: hex(0xECEDEF),
        text_secondary: hex(0xB3B7BF),
        text_tertiary: hex(0xA6AAB3),
        text_disabled: hex(0x5C5F67),
        accent: hex(0x8E86FF),
        accent_hover: hex(0xA39DFF),
        accent_text: hex(0xA9A3FF),
        text_on_accent: hex(0x0D0B26),
        danger: hex(0xFF8A85),
        warning: hex(0xE9AE45),
        success: hex(0x4CC38A),
        focus_width: 2.0,
        // Black at 30 % (e2) and 45 % (e3).
        shadow_e2: 77,
        shadow_e3: 115,
        // White at 6 %.
        overlay_hover: Color32::from_rgba_premultiplied(15, 15, 15, 15),
        playhead: hex(0xF4F4F6),
        // Rev. 2: the accent, inside the clip; the fill stays.
        clip_selected: hex(0x8E86FF),
        bg_lane: hex(0x15161A),
        bg_lane_alt: hex(0x18191D),
        bg_ruler: hex(0x121317),
        // White at 6 %.
        timeline_grid: Color32::from_rgba_premultiplied(15, 15, 15, 15),
        // Black at 55 %.
        overlay_badge: Color32::from_black_alpha(140),
        scrim: Color32::from_black_alpha(140),
    },
    light: Palette {
        bg_app: hex(0xD8D9DE),
        bg_panel: hex(0xF6F6F7),
        bg_card: hex(0xFFFFFF),
        bg_well: hex(0xE3E4E8),
        bg_field: hex(0xEEEFF2),
        bg_hover: hex(0xE7E8EB),
        bg_active: hex(0xDADBDF),
        // Black at 10 % and 20 %.
        border_subtle: Color32::from_black_alpha(26),
        border_strong: Color32::from_black_alpha(51),
        border_control: hex(0x80848D),
        text_primary: hex(0x16171A),
        text_secondary: hex(0x4A4E57),
        text_tertiary: hex(0x585C65),
        text_disabled: hex(0xA0A3AA),
        accent: hex(0x5A4FD8),
        accent_hover: hex(0x4B40C8),
        accent_text: hex(0x4B40C8),
        text_on_accent: hex(0xFFFFFF),
        danger: hex(0xB1241C),
        warning: hex(0x835100),
        success: hex(0x146942),
        focus_width: 2.0,
        // Black at 10 % (e2) and 18 % (e3).
        shadow_e2: 26,
        shadow_e3: 46,
        // Black at 5 %.
        overlay_hover: Color32::from_rgba_premultiplied(0, 0, 0, 13),
        playhead: hex(0x16171A),
        clip_selected: hex(0x5A4FD8),
        bg_lane: hex(0xEAEBEE),
        bg_lane_alt: hex(0xE4E5E9),
        bg_ruler: hex(0xEFF0F2),
        // Black at 7 %.
        timeline_grid: Color32::from_black_alpha(18),
        // Black at 55 %; the scrim at 35 %.
        overlay_badge: Color32::from_black_alpha(140),
        scrim: Color32::from_black_alpha(89),
    },
    high_contrast: Palette {
        bg_app: hex(0x000000),
        bg_panel: hex(0x000000),
        bg_card: hex(0x0A0A0A),
        bg_well: hex(0x000000),
        bg_field: hex(0x141414),
        bg_hover: hex(0x1F1F1F),
        bg_active: hex(0x333333),
        border_subtle: hex(0xFFFFFF),
        border_strong: hex(0xFFFFFF),
        border_control: hex(0xFFFFFF),
        text_primary: hex(0xFFFFFF),
        text_secondary: hex(0xE6E6E6),
        text_tertiary: hex(0xE6E6E6),
        text_disabled: hex(0x8C8C8C),
        accent: hex(0xFFD60A),
        accent_hover: hex(0xFFE45C),
        accent_text: hex(0xFFD60A),
        text_on_accent: hex(0x000000),
        danger: hex(0xFF7474),
        warning: hex(0xFFB000),
        success: hex(0x5CFF9D),
        focus_width: 3.0,
        shadow_e2: 77,
        shadow_e3: 115,
        overlay_hover: Color32::from_rgba_premultiplied(15, 15, 15, 15),
        playhead: hex(0xFFFFFF),
        clip_selected: hex(0xFFFFFF),
        bg_lane: hex(0x000000),
        bg_lane_alt: hex(0x0A0A0A),
        bg_ruler: hex(0x000000),
        // White at 30 %.
        timeline_grid: Color32::from_rgba_premultiplied(77, 77, 77, 77),
        overlay_badge: Color32::from_black_alpha(140),
        scrim: Color32::from_black_alpha(140),
    },
};

/// Appearance ▸ Theme (UI-014).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    /// Follows the operating system where it reports a theme; else Dark.
    System,
    HighContrast,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [
        ThemeChoice::Dark,
        ThemeChoice::Light,
        ThemeChoice::System,
        ThemeChoice::HighContrast,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
            ThemeChoice::System => "Follow system",
            ThemeChoice::HighContrast => "High contrast",
        }
    }

    /// The value stored as `appearance.theme` in `settings.json`.
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
            ThemeChoice::System => "system",
            ThemeChoice::HighContrast => "high_contrast",
        }
    }

    pub fn from_key(key: &str) -> Option<ThemeChoice> {
        ThemeChoice::ALL.into_iter().find(|c| c.key() == key)
    }

    /// The theme shown: *Follow system* takes the system's light or dark
    /// mode when the platform reports one, else Dark (UI-014).
    pub fn resolve(self, system: Option<Theme>) -> ThemeKind {
        match self {
            ThemeChoice::Dark => ThemeKind::Dark,
            ThemeChoice::Light => ThemeKind::Light,
            ThemeChoice::HighContrast => ThemeKind::HighContrast,
            ThemeChoice::System => match system {
                Some(Theme::Light) => ThemeKind::Light,
                Some(Theme::Dark) | None => ThemeKind::Dark,
            },
        }
    }
}

/// A theme that is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    Dark,
    Light,
    HighContrast,
}

/// Appearance ▸ Density (ui-concept §12.4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DensityChoice {
    /// Compact below the Compact breakpoint (§5.1), else Comfortable.
    #[default]
    Automatic,
    Comfortable,
    Compact,
}

impl DensityChoice {
    pub const ALL: [DensityChoice; 3] = [
        DensityChoice::Automatic,
        DensityChoice::Comfortable,
        DensityChoice::Compact,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DensityChoice::Automatic => "Automatic",
            DensityChoice::Comfortable => "Comfortable",
            DensityChoice::Compact => "Compact",
        }
    }

    /// The value stored as `appearance.density` in `settings.json`.
    pub fn key(self) -> &'static str {
        match self {
            DensityChoice::Automatic => "automatic",
            DensityChoice::Comfortable => "comfortable",
            DensityChoice::Compact => "compact",
        }
    }

    pub fn from_key(key: &str) -> Option<DensityChoice> {
        DensityChoice::ALL.into_iter().find(|c| c.key() == key)
    }

    /// The density used in a window `width` pt wide (after UI scaling).
    pub fn resolve(self, width: f32) -> Density {
        match self {
            DensityChoice::Comfortable => Density::Comfortable,
            DensityChoice::Compact => Density::Compact,
            DensityChoice::Automatic if width < COMPACT_BELOW => Density::Compact,
            DensityChoice::Automatic => Density::Comfortable,
        }
    }
}

/// Windows narrower than this (pt, after UI scaling) are in the Compact
/// breakpoint (ui-concept §5.1).
pub const COMPACT_BELOW: f32 = 1280.0;

/// A density's metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Density {
    Comfortable,
    Compact,
}

/// Sizes in pt (ui-concept §12.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub control_height: f32,
    pub row_height: f32,
    /// Between rows: `control_height + row_gap` is the row pitch.
    pub row_gap: f32,
    pub card_header: f32,
    /// The inspector's lens strip (was the lens bar).
    pub lens_strip: f32,
    pub label_column: f32,
    pub card_padding: f32,
    pub panel_padding: f32,
    /// The property row's gutter (reset arrow, keyframe navigator).
    pub row_gutter: f32,
    /// Ground between the floating panels and around them (rev. 2).
    pub gutter: f32,
    /// The slim top bar on the ground (§3.5).
    pub top_bar: f32,
    /// A panel header: the Stage's and the inspector's (§3.4).
    pub panel_header: f32,
    /// Rows, headers and notes start this far inside a panel.
    pub side_inset: f32,
    /// Icon button chip side (§13.18).
    pub icon_chip: f32,
    /// The Stage's transport bar (§9.1).
    pub transport: f32,
    /// Free space around the canvas on its surround (§9.1).
    pub surround_margin: f32,
    /// The timeline's sequence tab strip, toolbar and ruler (§13.26).
    pub tl_tabs: f32,
    pub tl_toolbar: f32,
    pub tl_ruler: f32,
    /// The track header column: its default width and its narrowest.
    pub tl_header: f32,
    pub tl_header_min: f32,
    /// Timeline lane heights: Small, Medium, Large.
    pub lanes: [f32; 3],
    /// The name strip on top of a two-tone clip.
    pub clip_strip: f32,
}

impl Density {
    pub const fn metrics(self) -> Metrics {
        match self {
            Density::Comfortable => Metrics {
                control_height: 28.0,
                row_height: 28.0,
                row_gap: 6.0,
                card_header: 36.0,
                lens_strip: 40.0,
                label_column: 76.0,
                card_padding: 14.0,
                panel_padding: 14.0,
                row_gutter: 56.0,
                gutter: 8.0,
                top_bar: 36.0,
                panel_header: 48.0,
                side_inset: 14.0,
                icon_chip: 26.0,
                transport: 44.0,
                surround_margin: 16.0,
                tl_tabs: 32.0,
                tl_toolbar: 40.0,
                tl_ruler: 28.0,
                tl_header: 176.0,
                tl_header_min: 140.0,
                lanes: [40.0, 72.0, 96.0],
                clip_strip: 18.0,
            },
            Density::Compact => Metrics {
                control_height: 24.0,
                row_height: 24.0,
                row_gap: 4.0,
                card_header: 30.0,
                lens_strip: 34.0,
                label_column: 64.0,
                card_padding: 10.0,
                panel_padding: 10.0,
                row_gutter: 50.0,
                gutter: 4.0,
                top_bar: 32.0,
                panel_header: 40.0,
                side_inset: 10.0,
                icon_chip: 24.0,
                transport: 36.0,
                surround_margin: 8.0,
                tl_tabs: 28.0,
                tl_toolbar: 34.0,
                tl_ruler: 24.0,
                tl_header: 148.0,
                tl_header_min: 120.0,
                lanes: [34.0, 60.0, 80.0],
                clip_strip: 16.0,
            },
        }
    }
}

/// Radii (ui-concept §12.5): badges and chips inside controls.
pub const R_XS: u8 = 3;
/// Icon buttons, tabs, list rows, clips, tooltips, Export.
pub const R_SM: u8 = 5;
/// Fields, selects, text buttons, tiles, inner cards.
pub const R_MD: u8 = 7;
/// Panels, menus, toasts, the context strip.
pub const R_LG: u8 = 12;
/// Modal sheets and Preferences.
pub const R_XL: u8 = 16;

/// The typography tokens beyond egui's built-in text styles (§12.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// 10 pt Medium: counts, badges, ruler labels, eyebrows.
    Micro,
    /// 11 pt Medium: units, chip labels, summaries.
    Caption,
    /// 13 pt Medium: section and panel titles.
    Label,
    /// 14 pt Medium: the inspector's item name, dialog titles.
    Title,
    /// 32 pt SemiBold: the Home heading only.
    Display,
}

impl Type {
    fn name(self) -> &'static str {
        match self {
            Type::Micro => "micro",
            Type::Caption => "caption",
            Type::Label => "label",
            Type::Title => "title",
            Type::Display => "display",
        }
    }

    /// The text style, falling back to a stock one where the tokens weren't
    /// applied (headless tests).
    pub fn style(self, style: &egui::Style) -> TextStyle {
        let named = TextStyle::Name(self.name().into());
        if style.text_styles.contains_key(&named) {
            named
        } else {
            match self {
                Type::Micro | Type::Caption => TextStyle::Small,
                Type::Label => TextStyle::Body,
                Type::Title | Type::Display => TextStyle::Heading,
            }
        }
    }
}

/// What the custom widgets paint with: the theme's colours and the
/// density's metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub palette: Palette,
    pub metrics: Metrics,
    pub kind: ThemeKind,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            palette: TOKENS.dark,
            metrics: Density::Comfortable.metrics(),
            kind: ThemeKind::Dark,
        }
    }
}

impl Look {
    /// The outline High contrast gives every control (UI-014); none in
    /// the other themes.
    pub fn control_border(&self) -> Stroke {
        if self.kind == ThemeKind::HighContrast {
            Stroke::new(1.0, self.palette.border_control)
        } else {
            Stroke::NONE
        }
    }
}

fn look_id() -> egui::Id {
    egui::Id::new("freecut-theme-look")
}

/// The look [`apply`] set last (Dark, Comfortable before that).
pub fn look(ctx: &egui::Context) -> Look {
    ctx.data(|d| d.get_temp::<Look>(look_id()))
        .unwrap_or_default()
}

/// Maps one theme and density to egui's style (ui-concept §16.1) and
/// stores the [`Look`] for the custom widgets. Call when the choice
/// changes, not per frame.
pub fn apply(ctx: &egui::Context, tokens: &Tokens, kind: ThemeKind, density: Density) {
    let palette = *tokens.palette(kind);
    let metrics = density.metrics();
    let base = match kind {
        ThemeKind::Light => Theme::Light,
        ThemeKind::Dark | ThemeKind::HighContrast => Theme::Dark,
    };
    let named = fonts::installed(ctx);
    let mut style = (*ctx.style_of(base)).clone();
    style.visuals = visuals(&palette, kind);
    style.text_styles = text_styles(named);
    spacing(&mut style.spacing, &metrics);
    // dur.fast (§12.8).
    style.animation_time = 0.09;
    // delay.tooltip: the first tooltip after 0.5 s, the next one at once
    // for 0.6 s after it closed (§12.8).
    style.interaction.tooltip_delay = 0.5;
    style.interaction.tooltip_grace_time = 0.6;
    // A gutter is the resize handle of the split it separates: its grab
    // area is the gutter plus 2 pt on each side (§3.4).
    style.interaction.resize_grab_radius_side = metrics.gutter / 2.0 + 2.0;
    ctx.set_theme(base);
    ctx.set_style_of(base, style);
    ctx.data_mut(|d| {
        d.insert_temp(
            look_id(),
            Look {
                palette,
                metrics,
                kind,
            },
        );
    });
}

fn shadow(offset: i8, blur: u8, alpha: u8) -> Shadow {
    Shadow {
        offset: [0, offset],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    }
}

/// The `Visuals` of one theme (ui-concept §16.1's mapping).
pub fn visuals(p: &Palette, kind: ThemeKind) -> egui::Visuals {
    let mut v = match kind {
        ThemeKind::Light => egui::Visuals::light(),
        ThemeKind::Dark | ThemeKind::HighContrast => egui::Visuals::dark(),
    };
    let r_md = CornerRadius::same(R_MD);
    // High contrast: visible borders on every control (UI-014).
    let control_border = Look {
        palette: *p,
        metrics: Density::Comfortable.metrics(),
        kind,
    }
    .control_border();
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg_card;
    w.noninteractive.weak_bg_fill = p.bg_card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border_subtle);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.noninteractive.corner_radius = r_md;
    // Rev. 2: controls are lifted one step above the panel (`bg.field`).
    w.inactive.bg_fill = p.bg_field;
    w.inactive.weak_bg_fill = p.bg_field;
    w.inactive.bg_stroke = control_border;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.inactive.corner_radius = r_md;
    w.hovered.bg_fill = p.bg_hover;
    w.hovered.weak_bg_fill = p.bg_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text_primary);
    w.hovered.corner_radius = r_md;
    w.active.bg_fill = p.bg_active;
    w.active.weak_bg_fill = p.bg_active;
    // egui draws a focused stock widget (field, button, combo box) with
    // `active`: its border is the focus ring, 2 pt (3 pt in High contrast).
    w.active.bg_stroke = Stroke::new(p.focus_width, p.accent);
    w.active.fg_stroke = Stroke::new(2.0, p.text_primary);
    w.active.corner_radius = r_md;
    w.open.bg_fill = p.bg_active;
    w.open.weak_bg_fill = p.bg_active;
    w.open.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.open.fg_stroke = Stroke::new(1.0, p.text_primary);
    w.open.corner_radius = r_md;
    v.selection.bg_fill = p.overlay_selection();
    // A focused text field's frame: the focus ring's width too.
    v.selection.stroke = Stroke::new(p.focus_width, p.accent_text);
    v.hyperlink_color = p.accent_text;
    v.faint_bg_color = p.bg_card;
    v.extreme_bg_color = p.bg_well;
    v.text_edit_bg_color = Some(p.bg_field);
    v.code_bg_color = p.bg_well;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.weak_text_color = Some(p.text_tertiary);
    v.window_fill = p.bg_card;
    v.window_stroke = Stroke::new(1.0, p.border_strong);
    v.window_corner_radius = CornerRadius::same(R_XL);
    v.menu_corner_radius = CornerRadius::same(R_LG);
    v.window_shadow = shadow(12, 32, p.shadow_e3);
    v.popup_shadow = shadow(4, 12, p.shadow_e2);
    v.panel_fill = p.bg_panel;
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v
}

fn text_styles(named: bool) -> std::collections::BTreeMap<TextStyle, FontId> {
    let family = |name: &str| {
        if named {
            FontFamily::Name(name.into())
        } else {
            FontFamily::Proportional
        }
    };
    let semibold = family(fonts::SEMIBOLD);
    let medium = family(fonts::MEDIUM);
    let named = |t: Type| TextStyle::Name(t.name().into());
    [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(12.0)),
        (TextStyle::Button, FontId::new(12.0, medium.clone())),
        (TextStyle::Heading, FontId::new(16.0, medium.clone())),
        (TextStyle::Monospace, FontId::monospace(12.0)),
        (named(Type::Micro), FontId::new(10.0, medium.clone())),
        (named(Type::Caption), FontId::new(11.0, medium.clone())),
        (named(Type::Label), FontId::new(13.0, medium.clone())),
        (named(Type::Title), FontId::new(14.0, medium)),
        (named(Type::Display), FontId::new(32.0, semibold)),
    ]
    .into_iter()
    .collect()
}

fn spacing(s: &mut egui::Spacing, m: &Metrics) {
    let compact = m.control_height < 28.0;
    s.item_spacing = Vec2::splat(m.row_gap);
    s.button_padding = if compact {
        Vec2::new(8.0, 2.0)
    } else {
        Vec2::new(10.0, 4.0)
    };
    s.interact_size = Vec2::new(40.0, m.control_height);
    s.slider_rail_height = 4.0;
    s.window_margin = egui::Margin::same(m.panel_padding as i8);
    s.menu_margin = egui::Margin::same(6);
    // Thin scrollbars that float over the content and show only when
    // something overflows (§13.24).
    s.scroll = egui::style::ScrollStyle {
        bar_width: 4.0,
        bar_inner_margin: 2.0,
        handle_min_length: 24.0,
        floating: true,
        ..egui::style::ScrollStyle::thin()
    };
}

/// Which theme and density are applied, so [`apply`] runs only on change.
#[derive(Debug, Default)]
pub struct ThemeState {
    applied: Option<(ThemeKind, Density)>,
}

impl ThemeState {
    /// Applies the chosen theme and density when they resolve differently
    /// from what is applied (a preference changed, the system switched to
    /// dark mode, the window crossed the Compact breakpoint). Returns
    /// whether it applied.
    pub fn sync(
        &mut self,
        ctx: &egui::Context,
        theme: ThemeChoice,
        density: DensityChoice,
    ) -> bool {
        let kind = theme.resolve(ctx.system_theme());
        let width = ctx.content_rect().width();
        let density = density.resolve(width);
        if self.applied == Some((kind, density)) {
            return false;
        }
        apply(ctx, &TOKENS, kind, density);
        self.applied = Some((kind, density));
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use super::*;

    /// `top` (premultiplied, as egui paints it) composited over the opaque
    /// `bottom`, the way egui blends: in gamma space.
    fn over(top: Color32, bottom: Color32) -> Color32 {
        let a = f32::from(top.a()) / 255.0;
        let mix = |t: u8, b: u8| (f32::from(t) + f32::from(b) * (1.0 - a)).round() as u8;
        Color32::from_rgb(
            mix(top.r(), bottom.r()),
            mix(top.g(), bottom.g()),
            mix(top.b(), bottom.b()),
        )
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// UI-013 / ui-concept §15: body and tertiary text 4.5 : 1 (7 : 1 in
    /// High contrast) on every background text sits on; accent text and
    /// status colours 4.5 : 1; labels on accent fills 4.5 : 1; the accent
    /// and control boundaries 3 : 1 against cards and panels.
    ///
    /// Text sits on hovered rows and buttons (`bg.hover`), on pressed
    /// buttons and hovered menu items (`bg.active`, egui's
    /// `hovered.weak_bg_fill`), and on selected rows and tiles
    /// (`overlay.selection` over a card or a panel), as well as on the
    /// four surfaces.
    #[test]
    fn every_token_pair_meets_its_wcag_ratio() {
        for (name, p, text_min) in [
            ("dark", TOKENS.dark, 4.5),
            ("light", TOKENS.light, 4.5),
            ("high contrast", TOKENS.high_contrast, 7.0),
        ] {
            let backgrounds = [
                ("bg.app", p.bg_app),
                ("bg.panel", p.bg_panel),
                ("bg.card", p.bg_card),
                ("bg.well", p.bg_well),
                ("bg.field", p.bg_field),
                ("bg.hover", p.bg_hover),
                ("bg.active", p.bg_active),
                (
                    "overlay.selection on bg.card",
                    over(p.overlay_selection(), p.bg_card),
                ),
                (
                    "overlay.selection on bg.panel",
                    over(p.overlay_selection(), p.bg_panel),
                ),
            ];
            // The menu-item hover fill is one of them.
            let hovered = visuals(&p, ThemeKind::Dark).widgets.hovered.weak_bg_fill;
            assert!(backgrounds.iter().any(|(_, bg)| *bg == hovered), "{name}");
            for (bg_name, bg) in backgrounds {
                for (fg_name, fg, min) in [
                    ("text.primary", p.text_primary, text_min),
                    ("text.secondary", p.text_secondary, text_min),
                    ("text.tertiary", p.text_tertiary, text_min),
                    ("accent.text", p.accent_text, 4.5),
                    ("danger", p.danger, 4.5),
                    ("warning", p.warning, 4.5),
                    ("success", p.success, 4.5),
                ] {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= min,
                        "{name}: {fg_name} on {bg_name} is {ratio:.2}, needs {min}"
                    );
                }
            }
            for (bg_name, bg) in [("bg.card", p.bg_card), ("bg.panel", p.bg_panel)] {
                for (fg_name, fg) in [("accent", p.accent), ("border.control", p.border_control)] {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= 3.0,
                        "{name}: {fg_name} against {bg_name} is {ratio:.2}, needs 3"
                    );
                }
            }
            // Rev. 2 timeline surfaces: track names and the dB readout sit on
            // the lanes (the header column takes the lane's fill), ruler
            // labels on the ruler; the playhead and the selected-clip
            // outline must stand out from both (non-text, 3 : 1).
            for (bg_name, bg) in [
                ("bg.lane", p.bg_lane),
                ("bg.lane-alt", p.bg_lane_alt),
                ("bg.ruler", p.bg_ruler),
            ] {
                for (fg_name, fg) in [
                    ("text.primary", p.text_primary),
                    ("text.secondary", p.text_secondary),
                    ("text.tertiary", p.text_tertiary),
                ] {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= text_min,
                        "{name}: {fg_name} on {bg_name} is {ratio:.2}, needs {text_min}"
                    );
                }
                for (fg_name, fg) in [("playhead", p.playhead), ("clip.selected", p.clip_selected)]
                {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= 3.0,
                        "{name}: {fg_name} against {bg_name} is {ratio:.2}, needs 3"
                    );
                }
            }
            // Badge labels (durations, "Offline", "Not found") are white on
            // `overlay.badge` over any picture: check the worst case, a
            // white thumbnail, at AA (the picture underneath is the user's,
            // so High contrast can't promise more than the badge gives).
            let badge = over(p.overlay_badge, Color32::WHITE);
            let ratio = contrast(Color32::WHITE, badge);
            assert!(ratio >= 4.5, "{name}: badge text over white is {ratio:.2}");
            for (fill_name, fill) in [("accent", p.accent), ("accent.hover", p.accent_hover)] {
                let ratio = contrast(p.text_on_accent, fill);
                assert!(
                    ratio >= 4.5,
                    "{name}: text.on-accent on {fill_name} is {ratio:.2}"
                );
            }
        }
        // The concept's own measurements (§12.1), as a check on the formula.
        let measured = contrast(TOKENS.dark.text_primary, TOKENS.dark.bg_card);
        assert!((measured - 12.9).abs() < 0.05, "{measured}");
        let measured = contrast(TOKENS.light.text_secondary, TOKENS.light.bg_card);
        assert!((measured - 8.3).abs() < 0.05, "{measured}");
    }

    /// UI-014 / §15: a focused stock widget (a field, a button, a combo
    /// box) and a focused text field show the focus ring's width, 3 pt in
    /// High contrast, and every control there has a visible border.
    #[test]
    fn focus_rings_and_control_borders_follow_the_theme() {
        for kind in [ThemeKind::Dark, ThemeKind::Light, ThemeKind::HighContrast] {
            let p = *TOKENS.palette(kind);
            let v = visuals(&p, kind);
            assert_eq!(
                v.widgets.active.bg_stroke,
                Stroke::new(p.focus_width, p.accent)
            );
            assert_eq!(v.selection.stroke.width, p.focus_width);
            let look = Look {
                palette: p,
                metrics: Density::Comfortable.metrics(),
                kind,
            };
            let border = look.control_border();
            assert_eq!(v.widgets.inactive.bg_stroke, border);
            if kind == ThemeKind::HighContrast {
                assert_eq!(p.focus_width, 3.0);
                assert_eq!(border, Stroke::new(1.0, p.border_control));
            } else {
                assert_eq!(border, Stroke::NONE, "{kind:?}");
            }
        }
    }

    /// ui-concept §12.3: the playhead and the selected-clip outline follow
    /// the theme; clip labels take white or `#111214` by luminance.
    #[test]
    fn editing_surface_tokens_follow_the_concept() {
        assert_eq!(TOKENS.dark.playhead, hex(0xF4F4F6));
        assert_eq!(TOKENS.light.playhead, hex(0x16171A));
        // Rev. 2: a selected clip gets a 2 pt accent outline, its fill
        // unchanged; High contrast keeps white, away from its yellow accent
        // and focus ring.
        assert_eq!(TOKENS.dark.clip_selected, TOKENS.dark.accent);
        assert_eq!(TOKENS.light.clip_selected, TOKENS.light.accent);
        assert_eq!(TOKENS.high_contrast.clip_selected, Color32::WHITE);
        assert_ne!(
            TOKENS.high_contrast.clip_selected,
            TOKENS.high_contrast.accent
        );
        for p in [TOKENS.dark, TOKENS.light, TOKENS.high_contrast] {
            assert!(contrast(p.playhead, p.bg_app) >= 3.0);
        }
        use surface::*;
        assert_eq!(label_on(CLIP_VIDEO), Color32::WHITE);
        assert_eq!(label_on(hex(0xE0E0E0)), LABEL_DARK);
        for fill in [
            CLIP_VIDEO,
            CLIP_AUDIO,
            CLIP_AUDIO_EXTRACTED,
            CLIP_TEXT,
            CLIP_ELEMENT,
            CLIP_EFFECT,
            CLIP_ADJUST,
        ] {
            assert!(contrast(label_on(fill), fill) >= 4.5, "{fill:?}");
        }
    }

    #[test]
    fn choices_round_trip_through_their_keys_and_resolve() {
        for c in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::from_key(c.key()), Some(c));
        }
        for c in DensityChoice::ALL {
            assert_eq!(DensityChoice::from_key(c.key()), Some(c));
        }
        assert_eq!(ThemeChoice::from_key("sepia"), None);
        assert_eq!(ThemeChoice::System.resolve(None), ThemeKind::Dark);
        assert_eq!(
            ThemeChoice::System.resolve(Some(Theme::Light)),
            ThemeKind::Light
        );
        assert_eq!(
            ThemeChoice::HighContrast.resolve(Some(Theme::Light)),
            ThemeKind::HighContrast
        );
        // 1366 × 768 at 125 % is about 1093 pt wide: Compact (§5.1).
        assert_eq!(DensityChoice::Automatic.resolve(1093.0), Density::Compact);
        assert_eq!(
            DensityChoice::Automatic.resolve(1440.0),
            Density::Comfortable
        );
        assert_eq!(
            DensityChoice::Comfortable.resolve(1000.0),
            Density::Comfortable
        );
        assert_eq!(Density::Compact.metrics().row_height, 24.0);
    }

    /// §12.1: the neutral ladder. In Dark every step is lighter than the
    /// one before; in Light the panel is lighter than the well and the
    /// ground, the card is the brightest, and the interactive steps run
    /// darker (pressing reads as pushing in).
    #[test]
    fn neutral_ladder_is_ordered() {
        let d = TOKENS.dark;
        let ladder = [
            d.bg_app,
            d.bg_well,
            d.bg_panel,
            d.bg_card,
            d.bg_field,
            d.bg_hover,
            d.bg_active,
        ];
        for pair in ladder.windows(2) {
            assert!(luminance(pair[0]) < luminance(pair[1]), "{pair:?}");
        }
        let l = TOKENS.light;
        assert!(luminance(l.bg_app) < luminance(l.bg_well));
        assert!(luminance(l.bg_well) < luminance(l.bg_panel));
        for other in [l.bg_app, l.bg_well, l.bg_panel, l.bg_field, l.bg_hover] {
            assert!(luminance(l.bg_card) > luminance(other));
        }
        assert!(luminance(l.bg_field) > luminance(l.bg_hover));
        assert!(luminance(l.bg_hover) > luminance(l.bg_active));
    }

    /// §12.1: hairlines are translucent in Dark and Light (one value for
    /// every surface) and stay decorative: over a panel they contrast less
    /// than `border.control`. High contrast draws them solid white.
    #[test]
    fn hairlines_are_translucent() {
        for p in [TOKENS.dark, TOKENS.light] {
            assert!(p.border_subtle.a() < 255 && p.border_strong.a() < 255);
            assert!(p.border_subtle.a() < p.border_strong.a());
            let control = contrast(p.border_control, p.bg_panel);
            for line in [p.border_subtle, p.border_strong] {
                assert!(contrast(over(line, p.bg_panel), p.bg_panel) < control);
            }
        }
        let hc = TOKENS.high_contrast;
        assert_eq!(hc.border_subtle.a(), 255);
        assert_eq!(hc.border_strong, Color32::WHITE);
    }

    /// §12.3: a clip body is darker than its mark in Dark and lighter in
    /// Light, matches the concept's listed values, and the mark still
    /// carries a readable label.
    #[test]
    fn clip_wells_are_darker_than_marks_and_readable() {
        use surface::*;
        let marks = [
            CLIP_VIDEO,
            CLIP_AUDIO,
            CLIP_AUDIO_EXTRACTED,
            CLIP_TEXT,
            CLIP_ELEMENT,
            CLIP_EFFECT,
            CLIP_ADJUST,
        ];
        for mark in marks {
            let dark = clip_well(mark, ThemeKind::Dark);
            let light = clip_well(mark, ThemeKind::Light);
            assert!(luminance(dark) < luminance(mark), "{mark:?}");
            assert!(luminance(light) > luminance(mark), "{mark:?}");
            assert_eq!(clip_well(mark, ThemeKind::HighContrast), Color32::BLACK);
            assert!(contrast(label_on(mark), mark) >= 4.5, "{mark:?}");
            assert!(clip_wave(mark, ThemeKind::Dark).a() < 255);
        }
        assert_eq!(clip_well(CLIP_VIDEO, ThemeKind::Dark), hex(0x1D313D));
        assert_eq!(clip_well(CLIP_VIDEO, ThemeKind::Light), hex(0xD2DEE4));
        assert_eq!(clip_well(CLIP_TEXT, ThemeKind::Dark), hex(0x3E301B));
    }

    /// §12.4: one rhythm of sizes per density.
    #[test]
    fn metrics_follow_the_rhythm() {
        let c = Density::Comfortable.metrics();
        let k = Density::Compact.metrics();
        assert_eq!(c.control_height + c.row_gap, 34.0);
        assert_eq!(k.control_height + k.row_gap, 28.0);
        assert_eq!((c.gutter, k.gutter), (8.0, 4.0));
        assert_eq!((c.panel_header, k.panel_header), (48.0, 40.0));
        assert_eq!((c.top_bar, k.top_bar), (36.0, 32.0));
        assert_eq!((c.transport, k.transport), (44.0, 36.0));
        assert_eq!((c.side_inset, k.side_inset), (14.0, 10.0));
        // The timeline (§12.4): tabs / toolbar / ruler, header column, lanes.
        assert_eq!((c.tl_tabs, c.tl_toolbar, c.tl_ruler), (32.0, 40.0, 28.0));
        assert_eq!((k.tl_tabs, k.tl_toolbar, k.tl_ruler), (28.0, 34.0, 24.0));
        assert_eq!((c.tl_header, c.tl_header_min), (176.0, 140.0));
        assert_eq!((k.tl_header, k.tl_header_min), (148.0, 120.0));
        assert_eq!((c.lanes, k.lanes), ([40.0, 72.0, 96.0], [34.0, 60.0, 80.0]));
        let ctx = egui::Context::default();
        apply(&ctx, &TOKENS, ThemeKind::Dark, Density::Comfortable);
        let style = ctx.global_style();
        assert_eq!(style.interaction.tooltip_delay, 0.5);
        assert_eq!(style.interaction.resize_grab_radius_side, 6.0);
        assert!(style.spacing.scroll.floating);
        assert_eq!(style.spacing.item_spacing, Vec2::splat(6.0));
    }

    #[test]
    fn apply_runs_only_when_the_resolved_theme_changes() {
        let ctx = egui::Context::default();
        let mut state = ThemeState::default();
        assert_eq!(look(&ctx).kind, ThemeKind::Dark, "before any apply");
        assert!(state.sync(&ctx, ThemeChoice::Light, DensityChoice::Comfortable));
        assert!(!state.sync(&ctx, ThemeChoice::Light, DensityChoice::Comfortable));
        assert_eq!(look(&ctx).kind, ThemeKind::Light);
        assert_eq!(ctx.theme(), Theme::Light);
        assert_eq!(ctx.global_style().visuals.panel_fill, TOKENS.light.bg_panel);
        assert!(state.sync(&ctx, ThemeChoice::HighContrast, DensityChoice::Compact));
        let look = look(&ctx);
        assert_eq!(look.palette, TOKENS.high_contrast);
        assert_eq!(look.metrics, Density::Compact.metrics());
        assert_eq!(ctx.global_style().visuals.panel_fill, Color32::BLACK);
        // Without FreeCut's fonts the named styles fall back to stock ones.
        assert_eq!(
            Type::Label.style(&ctx.global_style()),
            TextStyle::Name("label".into())
        );
    }
}

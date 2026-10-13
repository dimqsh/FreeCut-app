//! The Preferences window (UI-018, UI-029): non-modal, opened with ⌘, /
//! Ctrl+, or the banner's "Choose device…". Sections: Appearance (UI-014:
//! theme and density, switching live) and Playback & audio (the output
//! device and the buffer size, AU-010). With search text, the matching rows
//! of every section show.
//!
//! Changes apply at once; the app hands them to the audio engine and to the
//! settings store, which writes them shortly after.

use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use eframe::egui::{self, RichText};
use freecut_audio::{EngineConfig, OutputDeviceInfo};

use crate::settings::{AppearanceSettings, PlaybackSettings, Settings};
use crate::sound::{OutputState, output_line};
use crate::theme::{self, DensityChoice, ThemeChoice, Type, icons};
use crate::widgets::help;
use crate::widgets::icon_button::IconButton;
use crate::widgets::segmented::{Segment, Segmented};
use crate::widgets::select::{Select, SelectOption};
use crate::widgets::sheet;
use crate::widgets::toggle::toggle;

/// How long the device list may take before the dropdown gives up
/// (UI-029).
pub const LIST_TIMEOUT: Duration = Duration::from_secs(2);
/// The buffer sizes offered (AU-010: 256 to 4096 frames).
pub const BUFFER_CHOICES: [u32; 5] = [256, 512, 1024, 2048, 4096];

/// The listed output devices, or why there are none yet.
#[derive(Debug)]
enum Listing {
    NotAsked,
    Searching {
        rx: Receiver<Result<Vec<OutputDeviceInfo>, String>>,
        since: Instant,
    },
    Ready(Vec<OutputDeviceInfo>),
    Failed(String),
}

/// A section of the window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Playback,
}

impl Section {
    const ALL: [Section; 2] = [Section::Appearance, Section::Playback];

    fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Playback => "Playback & audio",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Section::Appearance => icons::PALETTE,
            Section::Playback => icons::SPEAKER_HIGH,
        }
    }
}

/// The window's state, kept by the app.
#[derive(Debug)]
pub struct Preferences {
    pub open: bool,
    pub section: Section,
    search: String,
    listing: Listing,
}

impl Default for Preferences {
    fn default() -> Preferences {
        Preferences {
            open: false,
            section: Section::default(),
            search: String::new(),
            listing: Listing::NotAsked,
        }
    }
}

impl Preferences {
    /// Shows the window and lists the devices again (they may have changed
    /// since it last showed).
    pub fn show_window(&mut self, wake: impl Fn() + Send + 'static) {
        self.open = true;
        self.list_devices(freecut_audio::list_output_devices, wake);
    }

    /// Lists devices with `list` on a helper thread (UI-029: listing can
    /// block on some systems). `wake` repaints when it answers.
    fn list_devices<L, E>(&mut self, list: L, wake: impl Fn() + Send + 'static)
    where
        L: FnOnce() -> Result<Vec<OutputDeviceInfo>, E> + Send + 'static,
        E: std::fmt::Display,
    {
        if matches!(self.listing, Listing::Searching { .. }) {
            return;
        }
        let (tx, rx) = bounded(1);
        let spawned = std::thread::Builder::new()
            .name("freecut-devices".into())
            .spawn(move || {
                let _ = tx.send(list().map_err(|e| e.to_string()));
                wake();
            });
        self.listing = match spawned {
            Ok(_) => Listing::Searching {
                rx,
                since: Instant::now(),
            },
            Err(e) => Listing::Failed(e.to_string()),
        };
    }

    /// Takes the device list once it arrived, or gives up after
    /// [`LIST_TIMEOUT`]. Returns when to look again while still waiting.
    fn poll_listing(&mut self, now: Instant) -> Option<Instant> {
        let Listing::Searching { rx, since } = &self.listing else {
            return None;
        };
        let deadline = *since + LIST_TIMEOUT;
        self.listing = match rx.try_recv() {
            Ok(Ok(devices)) => Listing::Ready(devices),
            Ok(Err(e)) => Listing::Failed(e),
            Err(TryRecvError::Disconnected) => {
                Listing::Failed("the device list stopped".to_owned())
            }
            Err(TryRecvError::Empty) if now >= deadline => {
                Listing::Failed("the system took too long to list the devices".to_owned())
            }
            Err(TryRecvError::Empty) => return Some(deadline),
        };
        None
    }

    /// Sets the search text (snapshots).
    #[cfg(test)]
    pub fn set_search(&mut self, search: &str) {
        search.clone_into(&mut self.search);
    }

    fn devices(&self) -> Option<&[OutputDeviceInfo]> {
        match &self.listing {
            Listing::Ready(d) => Some(d),
            _ => None,
        }
    }
}

/// One entry of the Output device dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceChoice {
    /// What goes into the settings; `None` is the system default.
    pub device: Option<String>,
    pub label: String,
}

/// The Output device entries (UI-029): "System default" first, then the
/// listed devices, then the saved device when the list doesn't have it
/// ("(not connected)"). While the list isn't known, the saved device is
/// shown by name.
pub fn device_choices(
    listed: Option<&[OutputDeviceInfo]>,
    saved: Option<&str>,
) -> Vec<DeviceChoice> {
    let default_name = listed
        .and_then(|l| l.iter().find(|d| d.is_default))
        .map(|d| d.name.as_str());
    let mut choices = vec![DeviceChoice {
        device: None,
        label: match default_name {
            Some(name) => format!("System default ({name})"),
            None => "System default".to_owned(),
        },
    }];
    for d in listed.unwrap_or_default() {
        choices.push(DeviceChoice {
            device: Some(d.name.clone()),
            label: d.name.clone(),
        });
    }
    if let Some(saved) = saved
        && !choices.iter().any(|c| c.device.as_deref() == Some(saved))
    {
        choices.push(DeviceChoice {
            device: Some(saved.to_owned()),
            label: if listed.is_some() {
                format!("{saved} (not connected)")
            } else {
                saved.to_owned()
            },
        });
    }
    choices
}

/// "1024 frames (21 ms at 48 kHz)".
pub fn buffer_label(frames: u32) -> String {
    let ms = (f64::from(frames) * 1_000.0 / 48_000.0).round();
    format!("{frames} frames ({ms} ms at 48 kHz)")
}

/// Whether a row matches the search box: every word of `query` appears in
/// its label, help or keywords, ignoring case.
pub fn row_matches(query: &str, texts: &[&str]) -> bool {
    let haystack = texts.join(" ").to_lowercase();
    query
        .split_whitespace()
        .all(|w| haystack.contains(&w.to_lowercase()))
}

const DEVICE_LABEL: &str = "Output device";
const DEVICE_HELP: &str = "Where FreeCut plays sound. If it isn't available, the system \
                           default plays instead.";
const DEVICE_KEYWORDS: &str = "audio speakers headphones sound output";
const BUFFER_LABEL: &str = "Buffer size";
const BUFFER_HELP: &str = "Larger buffers prevent crackles on a busy computer but delay the \
                           sound a little.";
const BUFFER_KEYWORDS: &str = "audio latency crackle dropout";
const THEME_LABEL: &str = "Theme";
const THEME_HELP: &str = "Dark is the default. Follow system uses the computer's light or dark \
                          mode. High contrast has black backgrounds, white text and visible \
                          borders.";
const THEME_KEYWORDS: &str = "appearance dark light mode colour color contrast accessibility";
const DENSITY_LABEL: &str = "Density";
const DENSITY_HELP: &str = "Compact fits more settings on small screens. Automatic is compact \
                            in narrow windows.";
const DENSITY_KEYWORDS: &str = "appearance compact comfortable spacing size small screen";
const MOTION_LABEL: &str = "Reduce motion";
const MOTION_HELP: &str = "Animation tiles show a still picture instead of playing when you point \
                           at them. Your video still plays as it is.";
const MOTION_KEYWORDS: &str = "appearance animation accessibility thumbnails preview hover";

/// The window's fixed size (§13.28): it never changes between pages.
pub const SIZE: egui::Vec2 = egui::vec2(680.0, 480.0);
/// The page side list's width and its rows' height.
pub const SIDE_WIDTH: f32 = 168.0;
pub const SIDE_ROW: f32 = 30.0;

/// Draws the window when open. Returns the settings after this frame's
/// changes, when anything changed.
pub fn show(
    ctx: &egui::Context,
    prefs: &mut Preferences,
    settings: &Settings,
    output: &OutputState,
    memory_only: bool,
) -> Option<Settings> {
    if !prefs.open {
        return None;
    }
    if let Some(at) = prefs.poll_listing(Instant::now()) {
        ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
    }
    let look = theme::look(ctx);
    let mut next = settings.clone();
    let mut close = false;
    let mut refresh = false;
    egui::Window::new("Preferences")
        .id(egui::Id::new("preferences"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_size(SIZE)
        // First shown centred on the window; it can be moved from there.
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .frame(sheet::sheet_frame(&look))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.set_min_size(SIZE);
            close = sheet::header(ui, "Preferences", Some(icons::GEAR), sheet::HEADER, true);
            let body = ui.available_rect_before_wrap();
            let side = egui::Rect::from_min_size(body.min, egui::vec2(SIDE_WIDTH, body.height()));
            let page =
                egui::Rect::from_min_max(egui::pos2(side.right() + 1.0, body.top()), body.max);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(side.right(), body.top()),
                    egui::vec2(1.0, body.height()),
                ),
                0.0,
                look.palette.border_subtle,
            );
            let mut side_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(side.shrink(8.0))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            side_list(&mut side_ui, prefs, memory_only);
            let mut page_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(page)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            refresh = page_view(&mut page_ui, prefs, &mut next, output);
            ui.allocate_rect(body, egui::Sense::hover());
        });
    if close {
        prefs.open = false;
    }
    if refresh {
        let ctx = ctx.clone();
        prefs.list_devices(freecut_audio::list_output_devices, move || {
            ctx.request_repaint();
        });
    }
    (next != *settings).then_some(next)
}

/// The page list: the search field, then a 30 pt row per page.
fn side_list(ui: &mut egui::Ui, prefs: &mut Preferences, memory_only: bool) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    ui.spacing_mut().item_spacing.y = 2.0;
    let field = egui::TextEdit::singleline(&mut prefs.search)
        .hint_text("Search settings")
        .desired_width(ui.available_width())
        .margin(egui::Margin {
            left: 26,
            right: 8,
            top: 6,
            bottom: 6,
        });
    let response = ui.add(field);
    // The magnifier is painted (icons never go through the text font).
    ui.painter().text(
        egui::pos2(response.rect.left() + 13.0, response.rect.center().y),
        egui::Align2::CENTER_CENTER,
        icons::SEARCH,
        theme::fonts::icon_font(ui.ctx(), 13.0, false),
        p.text_tertiary,
    );
    ui.add_space(8.0);
    let searching = !prefs.search.trim().is_empty();
    for section in Section::ALL {
        let current = prefs.section == section && !searching;
        if page_row(ui, section, current).clicked() {
            prefs.section = section;
            prefs.search.clear();
        }
    }
    if memory_only {
        let rect = ui.available_rect_before_wrap();
        let mut note = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::bottom_up(egui::Align::Min)),
        );
        note.add(
            egui::Label::new(
                RichText::new("Changes last until you quit: they can't be saved.")
                    .text_style(egui::TextStyle::Small)
                    .color(p.warning),
            )
            .wrap(),
        );
    }
}

/// A page row (§13.22): 30 pt, `r.sm`, a 16 pt icon and the label; idle
/// transparent, hover `bg.field`, current `bg.active`.
fn page_row(ui: &mut egui::Ui, section: Section, current: bool) -> egui::Response {
    let p = theme::look(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SIDE_ROW),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered(),
            ui.style().animation_time,
        );
        let fill = if current {
            p.bg_active
        } else {
            egui::Color32::TRANSPARENT.lerp_to_gamma(p.bg_field, t)
        };
        ui.painter().rect_filled(rect, theme::R_SM, fill);
        let ink = if current {
            p.text_primary
        } else {
            p.text_secondary.lerp_to_gamma(p.text_primary, t)
        };
        ui.painter().text(
            egui::pos2(rect.left() + 18.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            section.icon(),
            theme::fonts::icon_font(ui.ctx(), 16.0, current),
            ink,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 36.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            section.label(),
            egui::TextStyle::Body.resolve(ui.style()),
            ink,
        );
        if crate::widgets::icon_button::keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(ui, rect, theme::R_SM);
        }
    }
    let name = section.label();
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, name));
    response
}

/// The page (or, while searching, every section with a match). Returns
/// whether the device list should be read again.
fn page_view(
    ui: &mut egui::Ui,
    prefs: &mut Preferences,
    next: &mut Settings,
    output: &OutputState,
) -> bool {
    let look = theme::look(ui.ctx());
    let searching = !prefs.search.trim().is_empty();
    let mut refresh = false;
    egui::ScrollArea::vertical()
        .id_salt("preferences-page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    left: 20,
                    right: 20,
                    top: 8,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    let mut shown = 0;
                    if searching || prefs.section == Section::Appearance {
                        shown += appearance_section(ui, prefs, &mut next.appearance, searching);
                    }
                    if searching || prefs.section == Section::Playback {
                        let (rows, again) =
                            playback_section(ui, prefs, &mut next.playback, output, searching);
                        shown += rows;
                        refresh = again;
                    }
                    if shown == 0 {
                        ui.add_space(24.0);
                        ui.label(
                            RichText::new("No setting matches the search.")
                                .text_style(egui::TextStyle::Body)
                                .color(look.palette.text_tertiary),
                        );
                    }
                });
        });
    refresh
}

/// A page's title in `type.title`.
fn page_title(ui: &mut egui::Ui, title: &str, note: Option<&str>) {
    let p = theme::look(ui.ctx()).palette;
    ui.add_space(6.0);
    ui.label(
        RichText::new(title)
            .text_style(Type::Title.style(ui.style()))
            .color(p.text_primary),
    );
    if let Some(note) = note {
        ui.add(
            egui::Label::new(
                RichText::new(note)
                    .text_style(egui::TextStyle::Small)
                    .color(p.text_tertiary),
            )
            .wrap(),
        );
    }
    ui.add_space(2.0);
}

/// A setting as a section (§13.28): a hairline, the title with its help
/// badge, UI-018's reset marker while it differs from the default, and
/// `trailing` (a switch) at the far end; then `body`, if any. Returns
/// whether the reset marker was clicked.
fn setting(
    ui: &mut egui::Ui,
    title: &str,
    help: &str,
    changed: bool,
    trailing: impl FnOnce(&mut egui::Ui),
    body: Option<&mut dyn FnMut(&mut egui::Ui)>,
) -> bool {
    let p = theme::look(ui.ctx()).palette;
    let id = egui::Id::new(("pref-setting", title));
    ui.add_space(4.0);
    sheet::hairline(ui);
    let mut reset = false;
    sheet::section_title(ui, id, title, Some(help), |ui| {
        trailing(ui);
        if changed {
            reset = help::reset_badge(ui, id.with("reset"), true, "Restore the default");
            ui.label(
                RichText::new("Changed")
                    .text_style(egui::TextStyle::Small)
                    .color(p.text_tertiary),
            );
        }
    });
    if let Some(body) = body {
        body(ui);
    }
    reset
}

/// "Restore section defaults" at the end of a page.
fn restore_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    ui.add_space(10.0);
    sheet::hairline(ui);
    ui.add_space(10.0);
    sheet::button(
        ui,
        sheet::ButtonKind::Secondary,
        Some(icons::RESET),
        "Restore section defaults",
        0.0,
        None,
        enabled,
    )
    .clicked()
}

/// The Playback & audio rows. Returns how many rows matched the search,
/// and whether the device list should be read again.
fn playback_section(
    ui: &mut egui::Ui,
    prefs: &Preferences,
    playback: &mut PlaybackSettings,
    output: &OutputState,
    searching: bool,
) -> (usize, bool) {
    let defaults = PlaybackSettings::default();
    let query = prefs.search.trim();
    let mut refresh = false;
    let matches = [
        row_matches(query, &[DEVICE_LABEL, DEVICE_HELP, DEVICE_KEYWORDS]),
        row_matches(query, &[BUFFER_LABEL, BUFFER_HELP, BUFFER_KEYWORDS]),
    ];
    if searching && !matches.contains(&true) {
        return (0, false);
    }
    page_title(ui, "Playback & audio", Some(&output_line(output)));
    let mut shown = 0;

    if matches[0] {
        shown += 1;
        let choices = device_choices(prefs.devices(), playback.output_device.as_deref());
        let current = choices
            .iter()
            .position(|c| c.device == playback.output_device);
        let mut picked = None;
        let mut body = |ui: &mut egui::Ui| {
            let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
            let options: Vec<SelectOption<'_>> =
                labels.iter().map(|l| SelectOption::new(l)).collect();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let w = ui.available_width() - 34.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(w, theme::look(ui.ctx()).metrics.control_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let mut select =
                            Select::new(egui::Id::new("pref-output-device"), &options, current)
                                .fill_width();
                        if current.is_none() {
                            select = select.text("System default");
                        }
                        if let Some(i) = select.show(ui).picked {
                            picked = Some(i);
                        }
                    },
                );
                match &prefs.listing {
                    Listing::Searching { .. } => {
                        ui.spinner().on_hover_text("Searching…");
                    }
                    Listing::Failed(why) => {
                        if IconButton::new(
                            icons::REFRESH,
                            &format!("Couldn't list the devices: {why}. Try again"),
                        )
                        .show(ui)
                        .clicked()
                        {
                            refresh = true;
                        }
                    }
                    Listing::Ready(_) | Listing::NotAsked => {
                        if IconButton::new(icons::REFRESH, "List the devices again")
                            .show(ui)
                            .clicked()
                        {
                            refresh = true;
                        }
                    }
                }
            });
        };
        if setting(
            ui,
            DEVICE_LABEL,
            DEVICE_HELP,
            playback.output_device != defaults.output_device,
            |_| {},
            Some(&mut body),
        ) {
            playback.output_device = defaults.output_device.clone();
        }
        if let Some(i) = picked {
            playback.output_device = choices[i].device.clone();
        }
    }

    if matches[1] {
        shown += 1;
        let mut choices = BUFFER_CHOICES.to_vec();
        let current = playback.buffer_frames.clamp(
            EngineConfig::MIN_BUFFER_FRAMES,
            EngineConfig::MAX_BUFFER_FRAMES,
        );
        if !choices.contains(&current) {
            choices.push(current);
            choices.sort_unstable();
        }
        let labels: Vec<String> = choices.iter().map(|f| buffer_label(*f)).collect();
        let mut picked = None;
        let mut body = |ui: &mut egui::Ui| {
            let options: Vec<SelectOption<'_>> =
                labels.iter().map(|l| SelectOption::new(l)).collect();
            let at = choices.iter().position(|f| *f == current);
            if let Some(i) = Select::new(egui::Id::new("pref-buffer"), &options, at)
                .fill_width()
                .show(ui)
                .picked
            {
                picked = Some(i);
            }
        };
        if setting(
            ui,
            BUFFER_LABEL,
            BUFFER_HELP,
            playback.buffer_frames != defaults.buffer_frames,
            |_| {},
            Some(&mut body),
        ) {
            playback.buffer_frames = defaults.buffer_frames;
        }
        if let Some(i) = picked {
            playback.buffer_frames = choices[i];
        }
    }

    if restore_button(ui, *playback != defaults) {
        *playback = defaults;
    }
    ui.add_space(12.0);
    (shown, refresh)
}

/// The Appearance rows (UI-014, UI-043): theme, density and Reduce
/// motion, applied on the next frame. Returns how many rows matched the
/// search.
fn appearance_section(
    ui: &mut egui::Ui,
    prefs: &Preferences,
    appearance: &mut AppearanceSettings,
    searching: bool,
) -> usize {
    let defaults = AppearanceSettings::default();
    let query = prefs.search.trim();
    let theme_row = row_matches(query, &[THEME_LABEL, THEME_HELP, THEME_KEYWORDS]);
    let density_row = row_matches(query, &[DENSITY_LABEL, DENSITY_HELP, DENSITY_KEYWORDS]);
    let motion_row = row_matches(query, &[MOTION_LABEL, MOTION_HELP, MOTION_KEYWORDS]);
    if searching && !theme_row && !density_row && !motion_row {
        return 0;
    }
    let mut shown = 0;
    page_title(ui, "Appearance", None);
    if theme_row {
        shown += 1;
        let items: Vec<Segment<'_>> = ThemeChoice::ALL
            .iter()
            .map(|c| Segment::new(None, c.label()))
            .collect();
        let at = ThemeChoice::ALL
            .iter()
            .position(|c| *c == appearance.theme)
            .unwrap_or(0);
        let mut picked = None;
        let mut body = |ui: &mut egui::Ui| {
            if let Some(i) = Segmented::new(egui::Id::new("pref-theme"), &items, at)
                .fill_width()
                .show(ui)
                .picked
            {
                picked = Some(i);
            }
        };
        if setting(
            ui,
            THEME_LABEL,
            THEME_HELP,
            appearance.theme != defaults.theme,
            |_| {},
            Some(&mut body),
        ) {
            appearance.theme = defaults.theme;
        }
        if let Some(i) = picked {
            appearance.theme = ThemeChoice::ALL[i];
        }
    }
    if density_row {
        shown += 1;
        let items: Vec<Segment<'_>> = DensityChoice::ALL
            .iter()
            .map(|c| Segment::new(None, c.label()))
            .collect();
        let at = DensityChoice::ALL
            .iter()
            .position(|c| *c == appearance.density)
            .unwrap_or(0);
        let mut picked = None;
        let mut body = |ui: &mut egui::Ui| {
            if let Some(i) = Segmented::new(egui::Id::new("pref-density"), &items, at)
                .fill_width()
                .show(ui)
                .picked
            {
                picked = Some(i);
            }
        };
        if setting(
            ui,
            DENSITY_LABEL,
            DENSITY_HELP,
            appearance.density != defaults.density,
            |_| {},
            Some(&mut body),
        ) {
            appearance.density = defaults.density;
        }
        if let Some(i) = picked {
            appearance.density = DensityChoice::ALL[i];
        }
    }
    if motion_row {
        shown += 1;
        // A boolean setting: a section with a switch and no body.
        let mut on = appearance.reduce_motion;
        if setting(
            ui,
            MOTION_LABEL,
            MOTION_HELP,
            appearance.reduce_motion != defaults.reduce_motion,
            |ui| {
                toggle(
                    ui,
                    egui::Id::new("pref-reduce-motion"),
                    &mut on,
                    "Show still pictures of animations",
                )
                .on_hover_text("Show still pictures of animations");
            },
            None,
        ) {
            on = defaults.reduce_motion;
        }
        appearance.reduce_motion = on;
    }
    if restore_button(ui, *appearance != defaults) {
        *appearance = defaults;
    }
    ui.add_space(12.0);
    shown
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use super::*;

    fn device(name: &str, is_default: bool) -> OutputDeviceInfo {
        OutputDeviceInfo {
            name: name.to_owned(),
            is_default,
        }
    }

    #[test]
    fn the_saved_device_shows_as_not_connected_when_missing() {
        let listed = [device("Speakers", true), device("USB Headset", false)];
        let labels = |choices: Vec<DeviceChoice>| -> Vec<String> {
            choices.into_iter().map(|c| c.label).collect()
        };
        assert_eq!(
            labels(device_choices(Some(&listed), None)),
            ["System default (Speakers)", "Speakers", "USB Headset"]
        );
        assert_eq!(
            labels(device_choices(Some(&listed), Some("USB Headset"))),
            ["System default (Speakers)", "Speakers", "USB Headset"]
        );
        let missing = device_choices(Some(&listed), Some("Studio"));
        assert_eq!(missing[3].label, "Studio (not connected)");
        assert_eq!(missing[3].device.as_deref(), Some("Studio"));
        // While the list is unknown: the saved device by name only.
        assert_eq!(
            labels(device_choices(None, Some("Studio"))),
            ["System default", "Studio"]
        );
        assert_eq!(device_choices(None, None)[0].device, None);
    }

    #[test]
    fn listing_answers_or_times_out_after_two_seconds() {
        let mut prefs = Preferences::default();
        prefs.list_devices(|| Ok::<_, String>(vec![device("Speakers", true)]), || {});
        let deadline = Instant::now() + Duration::from_secs(10);
        while matches!(prefs.listing, Listing::Searching { .. }) {
            assert!(Instant::now() < deadline);
            prefs.poll_listing(Instant::now());
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(prefs.devices().unwrap(), [device("Speakers", true)]);

        // A list that never comes gives up after the timeout.
        let (_hold, never) = bounded::<()>(0);
        prefs.list_devices(
            move || {
                let _ = never.recv_timeout(Duration::from_secs(5));
                Ok::<_, String>(Vec::new())
            },
            || {},
        );
        let Listing::Searching { since, .. } = prefs.listing else {
            panic!("{:?}", prefs.listing);
        };
        assert_eq!(prefs.poll_listing(since), Some(since + LIST_TIMEOUT));
        assert_eq!(prefs.poll_listing(since + LIST_TIMEOUT), None);
        assert!(matches!(prefs.listing, Listing::Failed(_)));
        assert!(prefs.devices().is_none());

        // An error from the system is shown, not hidden.
        prefs.list_devices(|| Err::<Vec<OutputDeviceInfo>, _>("no host"), || {});
        while matches!(prefs.listing, Listing::Searching { .. }) {
            prefs.poll_listing(Instant::now());
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(&prefs.listing, Listing::Failed(e) if e == "no host"));
    }

    #[test]
    fn search_matches_every_word_in_label_help_or_keywords() {
        let row = [DEVICE_LABEL, DEVICE_HELP, DEVICE_KEYWORDS];
        assert!(row_matches("", &row));
        assert!(row_matches("headphones", &row));
        assert!(row_matches("OUTPUT device", &row));
        assert!(!row_matches("latency", &row));
        assert!(row_matches(
            "latency",
            &[BUFFER_LABEL, BUFFER_HELP, BUFFER_KEYWORDS]
        ));
        assert_eq!(buffer_label(1024), "1024 frames (21 ms at 48 kHz)");
        assert!(row_matches(
            "high contrast",
            &[THEME_LABEL, THEME_HELP, THEME_KEYWORDS]
        ));
        assert!(row_matches(
            "compact",
            &[DENSITY_LABEL, DENSITY_HELP, DENSITY_KEYWORDS]
        ));
        assert_eq!(buffer_label(256), "256 frames (5 ms at 48 kHz)");
    }

    /// §13.28: the window is 680 × 480 on every page and while searching.
    #[test]
    fn the_window_never_changes_size_between_pages() {
        let ctx = egui::Context::default();
        crate::theme::apply(
            &ctx,
            &crate::theme::TOKENS,
            crate::theme::ThemeKind::Dark,
            crate::theme::Density::Comfortable,
        );
        let mut prefs = Preferences {
            open: true,
            ..Preferences::default()
        };
        let settings = Settings::default();
        let output = OutputState::Opening;
        let mut sizes = Vec::new();
        for (section, search) in [
            (Section::Appearance, ""),
            (Section::Playback, ""),
            (Section::Appearance, "zzz"),
            (Section::Appearance, "audio"),
        ] {
            prefs.section = section;
            prefs.set_search(search);
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 900.0),
                    )),
                    ..egui::RawInput::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    show(ui.ctx(), &mut prefs, &settings, &output, false);
                });
                out.textures_delta.clear();
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("preferences")))
                .unwrap();
            sizes.push(rect.size());
        }
        // The content is SIZE; the outline adds 1 pt on each side.
        for size in &sizes {
            assert_eq!(*size, SIZE + egui::vec2(2.0, 2.0), "{sizes:?}");
        }
    }
}

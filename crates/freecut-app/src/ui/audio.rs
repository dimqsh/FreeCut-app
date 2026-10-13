//! The Audio tab of the asset panel (AU-030's Project audio, AU-001,
//! AU-004; ui-concept §8.2): the project's sound items as rows with their
//! waveforms, under the same content toolbar as Media (Import, search,
//! sort). The side list (or, in a narrow panel, its chips) picks All,
//! Imported or Extracted. "+" or a double-click adds an item at the
//! playhead on the first free audio track; rows drag onto the timeline
//! like media tiles.
//!
//! This build has no bundled music and no online catalogue, so the only
//! section is the project's own audio: imported audio files and sound
//! extracted from videos.

use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::{Document, Library, MediaSource};
use freecut_timeline::time::Tick;

use super::assets::{self, AssetDrag, Group, SortOrder, copy_file_path};
use super::timeline::{add_at_playhead, media_name};
use crate::session::Project;
use crate::theme::{self, Type, fonts, icons, surface};
use crate::waveform::{PeakState, Waveforms, peak_key};
use crate::widgets::empty::empty_state_with;
use crate::widgets::icon_button::{IconButton, keyboard_focus};
use crate::widgets::side_list::one_line;
use crate::widgets::tile::{short_duration, waveform};

const ROW_H: f32 = 40.0;
const ROW_GAP: f32 = 2.0;
const WAVE_W: f32 = 72.0;

/// The side-list rows of the Audio Drawer: *Audio* (All, Imported) and
/// *Made here* (Extracted).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AudioFilter {
    #[default]
    All,
    /// Audio files the user imported ("Your file").
    Imported,
    /// Sound extracted from a video (AU-004).
    Extracted,
}

impl AudioFilter {
    pub const ALL: [AudioFilter; 3] = [
        AudioFilter::All,
        AudioFilter::Imported,
        AudioFilter::Extracted,
    ];

    pub fn index(self) -> usize {
        AudioFilter::ALL
            .iter()
            .position(|f| *f == self)
            .unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioFilter::All => "All",
            AudioFilter::Imported => "Imported",
            AudioFilter::Extracted => "Extracted",
        }
    }

    fn admits(self, entry: &AudioEntry) -> bool {
        match self {
            AudioFilter::All => true,
            AudioFilter::Imported => !entry.extracted,
            AudioFilter::Extracted => entry.extracted,
        }
    }
}

/// One row of the Audio tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioEntry {
    pub id: MediaId,
    pub name: String,
    pub extracted: bool,
    pub duration: Tick,
}

/// The items the Audio tab lists, in library order: everything with sound
/// and no picture. Videos are left out; their sound gets here through
/// Extract audio.
pub fn audio_entries(library: &Library) -> Vec<AudioEntry> {
    library
        .items()
        .filter(|m| m.info.has_audio() && !m.info.has_video() && !m.info.is_still())
        .map(|m| AudioEntry {
            id: m.id,
            name: media_name(m),
            extracted: matches!(m.source, MediaSource::AudioOf { .. }),
            duration: m.info.duration,
        })
        .collect()
}

/// Whether `name` matches the search text: every word of `query` appears
/// in it, ignoring case. An empty query matches everything.
pub fn matches_search(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    query
        .split_whitespace()
        .all(|word| name.contains(&word.to_lowercase()))
}

/// The entries shown under `filter` and `query`.
pub fn visible<'a>(
    entries: &'a [AudioEntry],
    filter: AudioFilter,
    query: &str,
) -> Vec<&'a AudioEntry> {
    entries
        .iter()
        .filter(|e| filter.admits(e) && matches_search(&e.name, query))
        .collect()
}

/// Why the list is empty, when it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyState {
    /// The project has no audio at all.
    NoAudio,
    /// Nothing of the chip's kind, whatever the search.
    NoneOfKind(AudioFilter),
    /// The search hides every item of the chip's kind.
    NoMatches,
}

pub fn empty_state(entries: &[AudioEntry], filter: AudioFilter, query: &str) -> Option<EmptyState> {
    if entries.is_empty() {
        return Some(EmptyState::NoAudio);
    }
    if !entries.iter().any(|e| filter.admits(e)) {
        return Some(EmptyState::NoneOfKind(filter));
    }
    visible(entries, filter, query)
        .is_empty()
        .then_some(EmptyState::NoMatches)
}

/// A duration badge: `m:ss`, or `h:mm:ss` from an hour, to the nearest
/// second.
pub fn format_duration(t: Tick) -> String {
    short_duration(t)
}

/// The Audio Drawer's side list, with counts.
pub fn groups(library: &Library) -> Vec<Group> {
    let entries = audio_entries(library);
    let count = |f: AudioFilter| entries.iter().filter(|e| f.admits(e)).count();
    vec![
        Group {
            title: "Audio",
            rows: vec![
                (AudioFilter::All.label(), count(AudioFilter::All)),
                (AudioFilter::Imported.label(), count(AudioFilter::Imported)),
            ],
        },
        Group {
            title: "Made here",
            rows: vec![(
                AudioFilter::Extracted.label(),
                count(AudioFilter::Extracted),
            )],
        },
    ]
}

/// What the Audio tab shows; kept across tab switches, reset when another
/// project opens.
#[derive(Debug, Default)]
pub struct AudioState {
    pub filter: AudioFilter,
    pub search: String,
    /// Display order only; the library keeps its order.
    pub sort: SortOrder,
    /// The highlighted row.
    pub selected: Option<MediaId>,
    /// Scroll the highlighted row into view on the next frame.
    scroll_to_selected: bool,
}

impl AudioState {
    /// Highlights `media` and scrolls to it. With `filter`, that chip is
    /// chosen; otherwise the chip stays unless it hides the item. A search
    /// that hides the item is cleared.
    pub fn reveal(&mut self, doc: &Document, media: MediaId, filter: Option<AudioFilter>) {
        let entries = audio_entries(doc.library());
        let Some(entry) = entries.iter().find(|e| e.id == media) else {
            return;
        };
        match filter {
            Some(f) => self.filter = f,
            None if !self.filter.admits(entry) => self.filter = AudioFilter::All,
            None => {}
        }
        if !matches_search(&entry.name, &self.search) {
            self.search.clear();
        }
        self.selected = Some(media);
        self.scroll_to_selected = true;
    }
}

#[derive(Debug, Default)]
pub struct AudioOutput {
    pub import_clicked: bool,
    pub status: Option<String>,
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut AudioState,
    waves: &Waveforms,
    importing: usize,
    chips: Option<&[Group]>,
) -> AudioOutput {
    let mut out = AudioOutput::default();
    let gap = ui.spacing().item_spacing.y.max(6.0);
    let bar = assets::toolbar(
        ui,
        Id::new("audio-toolbar"),
        "Add audio files, or the sound of videos",
        importing,
        &mut state.search,
        "Search project audio",
        &mut state.sort,
    );
    out.import_clicked = bar.import_clicked;
    if let Some(groups) = chips
        && let Some(i) = {
            ui.add_space(gap);
            assets::chip_row(ui, Id::new("audio-chips"), groups, state.filter.index())
        }
        && let Some(f) = AudioFilter::ALL.get(i)
    {
        state.filter = *f;
    }
    ui.add_space(gap);
    let entries = audio_entries(project.doc().library());
    list(ui, project, state, waves, &entries, &mut out);
    out
}

fn list(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut AudioState,
    waves: &Waveforms,
    entries: &[AudioEntry],
    out: &mut AudioOutput,
) {
    if let Some(empty) = empty_state(entries, state.filter, &state.search) {
        let note =
            |ui: &mut egui::Ui, title: &str, detail: &str, p: Option<&str>, s: Option<&str>| {
                empty_state_with(ui, icons::AUDIO, title, detail, p, s)
            };
        match empty {
            EmptyState::NoAudio | EmptyState::NoneOfKind(AudioFilter::All) => {
                let e = note(
                    ui,
                    "No audio in this project yet",
                    "Import sound files, or the sound of videos",
                    Some("Import audio…"),
                    None,
                );
                out.import_clicked |= e.primary;
            }
            EmptyState::NoneOfKind(AudioFilter::Imported) => {
                let e = note(
                    ui,
                    "No imported audio yet",
                    "Sound files you import appear here",
                    Some("Import audio…"),
                    Some("Show all"),
                );
                out.import_clicked |= e.primary;
                if e.secondary {
                    state.filter = AudioFilter::All;
                }
            }
            EmptyState::NoneOfKind(AudioFilter::Extracted) => {
                let e = note(
                    ui,
                    "No extracted audio yet",
                    "Right-click a video in Media and choose Extract audio",
                    None,
                    Some("Show all"),
                );
                if e.secondary {
                    state.filter = AudioFilter::All;
                }
            }
            EmptyState::NoMatches => {
                let title = format!("No audio matches “{}”", state.search.trim());
                let e = empty_state_with(
                    ui,
                    icons::SEARCH,
                    &title,
                    "Check the spelling or try fewer words",
                    None,
                    Some("Clear search"),
                );
                if e.secondary {
                    state.search.clear();
                }
            }
        }
        return;
    }

    let mut shown: Vec<AudioEntry> = visible(entries, state.filter, &state.search)
        .into_iter()
        .cloned()
        .collect();
    state
        .sort
        .sort(&mut shown, |e| e.name.clone(), |e| u8::from(e.extracted));
    // Scroll a revealed row into view: its offset is known without laying
    // out the rows above it.
    let reveal = state
        .scroll_to_selected
        .then(|| shown.iter().position(|e| Some(e.id) == state.selected))
        .flatten();
    let mut area = egui::ScrollArea::vertical()
        .id_salt("audio-rows")
        .auto_shrink([false, false]);
    if let Some(i) = reveal {
        let view = ui.available_height();
        let y = (i as f32 * (ROW_H + ROW_GAP) - (view - ROW_H) / 2.0).max(0.0);
        area = area.vertical_scroll_offset(y);
    }
    state.scroll_to_selected = false;
    // `show_rows` steps by the row height plus the item spacing.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        area.show_rows(ui, ROW_H, shown.len(), |ui, range| {
            for entry in &shown[range] {
                row(ui, project, state, waves, entry, out);
            }
        });
    });
}

/// One item: drag it to the timeline, double-click or "+" to add it at the
/// playhead, right-click for its menu.
fn row(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut AudioState,
    waves: &Waveforms,
    entry: &AudioEntry,
    out: &mut AudioOutput,
) {
    let id = entry.id;
    let offline = project.offline.contains(&id);
    let peaks = if offline {
        None
    } else {
        peak_key(project.doc(), id, None).and_then(|key| match waves.get(key) {
            Some(PeakState::Ready(p)) => Some(Arc::clone(p)),
            _ => None,
        })
    };
    let selected = state.selected == Some(id);
    let chip = theme::look(ui.ctx()).metrics.icon_chip;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let width = (ui.available_width() - chip - 4.0).max(60.0);
        let drag_id = Id::new(("audio-row", id));
        let response = ui
            .dnd_drag_source(drag_id, AssetDrag::Media(id), |ui| {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
                let hovered = ui.rect_contains_pointer(rect);
                paint_row(
                    ui,
                    rect,
                    hovered,
                    entry,
                    selected,
                    offline,
                    peaks.as_deref(),
                );
                if keyboard_focus(ui, &resp) {
                    crate::widgets::focus_ring(ui, rect, theme::R_SM);
                }
                resp
            })
            .inner;
        let plus = ui
            .allocate_ui_with_layout(
                Vec2::new(chip, ROW_H),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    IconButton::new(
                        icons::PLUS,
                        "Add at the playhead, on the first free audio track",
                    )
                    .show(ui)
                },
            )
            .inner;
        if response.clicked() {
            state.selected = Some(id);
        }
        if response.double_clicked() || plus.clicked() {
            state.selected = Some(id);
            if let Err(e) = add_at_playhead(project, id) {
                out.status = Some(e);
            }
        }
        response.context_menu(|ui| {
            if ui.button("Add at playhead").clicked() {
                ui.close();
                if let Err(e) = add_at_playhead(project, id) {
                    out.status = Some(e);
                }
            }
            ui.separator();
            if ui.button("Copy file path").clicked() {
                ui.close();
                out.status = copy_file_path(ui.ctx(), project, id);
            }
        });
        let path = project
            .doc()
            .library()
            .get(id)
            .map_or_else(String::new, |m| m.path.absolute.clone());
        response.on_hover_text(format!("{}\n{path}", entry.name));
    });
}

/// A row (§13.22's list row grammar at 40 pt): the kind chip, the name
/// and details, and the item's waveform at the right.
fn paint_row(
    ui: &egui::Ui,
    rect: Rect,
    hovered: bool,
    entry: &AudioEntry,
    selected: bool,
    offline: bool,
    peaks: Option<&crate::waveform::Peaks>,
) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let painter = ui.painter_at(rect);
    let t = ui.ctx().animate_bool_with_time(
        Id::new(("audio-row-hover", entry.id)),
        hovered,
        ui.style().animation_time,
    );
    if selected {
        painter.rect_filled(rect, theme::R_SM, p.bg_active);
    } else if t > 0.0 {
        painter.rect_filled(rect, theme::R_SM, p.bg_field.gamma_multiply(t));
    }
    // The kind chip, in the colour the clip will have on the timeline.
    let mark = if entry.extracted {
        surface::CLIP_AUDIO_EXTRACTED
    } else {
        surface::CLIP_AUDIO
    };
    let icon = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::splat(ROW_H - 12.0));
    painter.rect_filled(icon, theme::R_SM, surface::clip_well(mark, look.kind));
    painter.rect_stroke(
        icon,
        theme::R_SM,
        Stroke::new(1.0, p.border_subtle),
        StrokeKind::Inside,
    );
    painter.text(
        icon.center(),
        Align2::CENTER_CENTER,
        icons::MUSIC_NOTE,
        fonts::icon_font(ui.ctx(), 15.0, false),
        surface::clip_wave(mark, look.kind),
    );

    let wave_w = if rect.width() > 220.0 { WAVE_W } else { 0.0 };
    let text_left = icon.right() + 10.0;
    let text_w = (rect.right() - wave_w - 12.0 - text_left).max(10.0);
    let name_color = if selected || hovered {
        p.text_primary
    } else {
        p.text_secondary
    };
    let name = one_line(
        ui,
        &entry.name,
        egui::TextStyle::Body.resolve(ui.style()),
        name_color,
        text_w,
    );
    painter.galley(Pos2::new(text_left, rect.top() + 5.0), name, name_color);

    let y = rect.top() + 22.0;
    let detail = if entry.extracted {
        format_duration(entry.duration)
    } else {
        format!("{} · Your file", format_duration(entry.duration))
    };
    let small = egui::TextStyle::Small.resolve(ui.style());
    let galley = one_line(ui, &detail, small, p.text_tertiary, text_w);
    let mut x = text_left + galley.size().x + 6.0;
    painter.galley(Pos2::new(text_left, y), galley, p.text_tertiary);
    if entry.extracted {
        x = tag(&painter, ui, Pos2::new(x, y), "Extracted", p.text_secondary) + 4.0;
    }
    if offline {
        tag(&painter, ui, Pos2::new(x, y), "Offline", p.danger);
    }

    if let Some(peaks) = peaks.filter(|_| wave_w > 0.0) {
        let band = Rect::from_min_size(
            Pos2::new(rect.right() - wave_w - 8.0, rect.center().y - 10.0),
            Vec2::new(wave_w, 20.0),
        );
        waveform(
            &painter,
            band,
            peaks,
            entry.duration,
            surface::clip_wave(mark, look.kind),
        );
    }
}

/// A small outlined label in `color`; returns its right edge.
fn tag(painter: &egui::Painter, ui: &egui::Ui, at: Pos2, text: &str, color: Color32) -> f32 {
    let font = Type::Micro.style(ui.style()).resolve(ui.style());
    let galley = one_line(ui, text, font, color, 200.0);
    let size = galley.size() + Vec2::new(8.0, 2.0);
    let r = Rect::from_min_size(at, size);
    painter.rect_stroke(
        r,
        theme::R_XS,
        Stroke::new(1.0, color.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    painter.galley(at + Vec2::new(4.0, 1.0), galley, color);
    r.right()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::num::NonZeroU64;

    use freecut_timeline::time::TICKS_PER_SECOND;

    use super::*;

    fn entry(id: u64, name: &str, extracted: bool) -> AudioEntry {
        AudioEntry {
            id: MediaId::from_raw(NonZeroU64::new(id).unwrap()),
            name: name.to_owned(),
            extracted,
            duration: Tick::from_secs(3).unwrap(),
        }
    }

    fn names(list: Vec<&AudioEntry>) -> Vec<&str> {
        list.into_iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn search_matches_every_word_ignoring_case() {
        assert!(matches_search("Beach Walk.mp3", ""));
        assert!(matches_search("Beach Walk.mp3", "   "));
        assert!(matches_search("Beach Walk.mp3", "beach"));
        assert!(matches_search("Beach Walk.mp3", "WALK beach"));
        assert!(matches_search("Beach Walk.mp3", "  walk  "));
        assert!(!matches_search("Beach Walk.mp3", "beach run"));
        assert!(matches_search("Ünïcode Söng", "söng"));
    }

    #[test]
    fn chips_and_search_filter_the_rows() {
        let entries = [
            entry(1, "song.mp3", false),
            entry(2, "clip.mp4 (audio)", true),
            entry(3, "Interview.wav", false),
        ];
        assert_eq!(
            names(visible(&entries, AudioFilter::All, "")),
            ["song.mp3", "clip.mp4 (audio)", "Interview.wav"]
        );
        assert_eq!(
            names(visible(&entries, AudioFilter::Imported, "")),
            ["song.mp3", "Interview.wav"]
        );
        assert_eq!(
            names(visible(&entries, AudioFilter::Extracted, "")),
            ["clip.mp4 (audio)"]
        );
        assert_eq!(
            names(visible(&entries, AudioFilter::All, "INTER")),
            ["Interview.wav"]
        );
        assert!(visible(&entries, AudioFilter::Extracted, "song").is_empty());
    }

    #[test]
    fn empty_states_say_why() {
        assert_eq!(
            empty_state(&[], AudioFilter::All, ""),
            Some(EmptyState::NoAudio)
        );
        assert_eq!(
            empty_state(&[], AudioFilter::Extracted, "x"),
            Some(EmptyState::NoAudio)
        );
        let imported = [entry(1, "song.mp3", false)];
        assert_eq!(empty_state(&imported, AudioFilter::All, ""), None);
        assert_eq!(
            empty_state(&imported, AudioFilter::Extracted, ""),
            Some(EmptyState::NoneOfKind(AudioFilter::Extracted))
        );
        // The chip's kind wins over the search: clearing the search
        // wouldn't help.
        assert_eq!(
            empty_state(&imported, AudioFilter::Extracted, "zzz"),
            Some(EmptyState::NoneOfKind(AudioFilter::Extracted))
        );
        assert_eq!(
            empty_state(&imported, AudioFilter::All, "zzz"),
            Some(EmptyState::NoMatches)
        );
        assert_eq!(empty_state(&imported, AudioFilter::Imported, "SONG"), None);
    }

    #[test]
    fn audio_groups_count_each_row() {
        assert_eq!(
            groups(&Library::default())[0].rows,
            [("All", 0), ("Imported", 0)]
        );
        assert_eq!(AudioFilter::Extracted.index(), 2);
    }

    #[test]
    fn durations_round_to_the_second() {
        let ms = |n: i64| Tick::from_raw(TICKS_PER_SECOND / 1000 * n);
        assert_eq!(format_duration(Tick::ZERO), "0:00");
        assert_eq!(format_duration(ms(499)), "0:00");
        assert_eq!(format_duration(ms(500)), "0:01");
        assert_eq!(format_duration(ms(59_400)), "0:59");
        assert_eq!(format_duration(ms(59_600)), "1:00");
        assert_eq!(format_duration(Tick::from_secs(754).unwrap()), "12:34");
        assert_eq!(
            format_duration(Tick::from_secs(3_600 + 62).unwrap()),
            "1:01:02"
        );
        assert_eq!(format_duration(Tick::from_raw(-5)), "0:00");
    }
}

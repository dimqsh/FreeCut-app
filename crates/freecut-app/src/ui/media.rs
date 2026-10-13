//! The Media tab (PM-050; ui-concept §8.2, §13.23): imported items as an
//! auto-filled grid of 16:9 tiles with posters, waveforms or kind glyphs
//! and duration badges. Drag a tile onto the timeline, or double-click to
//! append it to the main track (sound goes on the first free audio track at
//! the playhead instead). Each tile's menu adds it at the playhead,
//! extracts a video's sound as an audio item (AU-004), copies the file path
//! (UI-035) and removes it from the project (PM-058).
//!
//! Click selects, `Shift` / `Ctrl` / `Cmd` click adds, a drag on the empty
//! floor draws a rubber band. The selection is view state; removing goes
//! through the library's remove operation and its one undo entry.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use eframe::egui::{
    self, Align2, CursorIcon, Id, Key, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle,
    UiBuilder, Vec2,
};
use freecut_timeline::edit::{EditRequest, InsertTarget};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::{Document, Library, MediaItem, MediaSource};

use super::assets::{self, AssetDrag, Group, SortOrder, copy_file_path};
use super::timeline::{add_at_playhead, insert_media, media_name};
use crate::session::Project;
use crate::theme::{self, Type, icons, surface};
use crate::waveform::{PeakState, Waveforms, peak_key};
use crate::widgets::empty::empty_state_with;
use crate::widgets::sheet::{self, ButtonKind, Sheet};
use crate::widgets::tile::{
    Corner, Picture, TileSpec, cell_rect, corner_button_rect, grid_layout, paint_tile, row_pitch,
    rubber_band_hits, short_duration, still_format, thumb_rect,
};

/// The Media Drawer's side-list rows, in order: the *Media* group (All,
/// Video, Audio, Images) and the *Made here* group (Extracted).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MediaFilter {
    #[default]
    All,
    Video,
    Audio,
    Images,
    /// Sound extracted from a video (AU-004).
    Extracted,
}

impl MediaFilter {
    pub const ALL: [MediaFilter; 5] = [
        MediaFilter::All,
        MediaFilter::Video,
        MediaFilter::Audio,
        MediaFilter::Images,
        MediaFilter::Extracted,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MediaFilter::All => "All",
            MediaFilter::Video => "Video",
            MediaFilter::Audio => "Audio",
            MediaFilter::Images => "Images",
            MediaFilter::Extracted => "Extracted",
        }
    }

    pub fn index(self) -> usize {
        MediaFilter::ALL
            .iter()
            .position(|f| *f == self)
            .unwrap_or(0)
    }

    pub fn admits(self, item: &MediaItem) -> bool {
        let info = &item.info;
        match self {
            MediaFilter::All => true,
            MediaFilter::Video => info.has_video() && !info.is_still(),
            MediaFilter::Images => info.is_still(),
            MediaFilter::Audio => !info.has_video() && !info.is_still() && info.has_audio(),
            MediaFilter::Extracted => matches!(item.source, MediaSource::AudioOf { .. }),
        }
    }

    /// The sentence of the empty state when nothing of this kind exists.
    fn none_yet(self) -> (&'static str, &'static str) {
        match self {
            MediaFilter::All => (
                "Nothing imported yet",
                "Video, audio and pictures appear here",
            ),
            MediaFilter::Video => ("No video in this project", "Imported videos appear here"),
            MediaFilter::Audio => (
                "No audio in this project",
                "Imported sound files appear here",
            ),
            MediaFilter::Images => (
                "No pictures in this project",
                "Imported pictures appear here",
            ),
            MediaFilter::Extracted => (
                "No extracted audio yet",
                "Right-click a video and choose Extract audio",
            ),
        }
    }
}

/// The item counts of the side list's rows, in [`MediaFilter::ALL`]
/// order.
pub fn side_list_counts(library: &Library) -> [usize; 5] {
    MediaFilter::ALL.map(|f| library.items().filter(|m| f.admits(m)).count())
}

/// The Media Drawer's side list.
pub fn groups(library: &Library) -> Vec<Group> {
    let c = side_list_counts(library);
    vec![
        Group {
            title: "Media",
            rows: vec![
                (MediaFilter::All.label(), c[0]),
                (MediaFilter::Video.label(), c[1]),
                (MediaFilter::Audio.label(), c[2]),
                (MediaFilter::Images.label(), c[3]),
            ],
        },
        Group {
            title: "Made here",
            rows: vec![(MediaFilter::Extracted.label(), c[4])],
        },
    ]
}

/// A kind rank for sorting by kind: video, pictures, audio, extracted.
fn kind_rank(item: &MediaItem) -> u8 {
    if item.info.is_still() {
        1
    } else if item.info.has_video() {
        0
    } else if matches!(item.source, MediaSource::AudioOf { .. }) {
        3
    } else {
        2
    }
}

/// The items shown under `filter`, `query` and `sort`.
pub fn visible_items(
    library: &Library,
    filter: MediaFilter,
    query: &str,
    sort: SortOrder,
) -> Vec<Arc<MediaItem>> {
    let mut items: Vec<Arc<MediaItem>> = library
        .items()
        .filter(|m| filter.admits(m) && super::audio::matches_search(&media_name(m), query))
        .cloned()
        .collect();
    sort.sort(&mut items, |m| media_name(m), |m| kind_rank(m));
    items
}

/// Removing media from the project (PM-058): what goes and how many
/// timeline clips go with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// The chosen items still in the library, in id order. Sound extracted
    /// from a removed video stays: it reads the file itself (AU-004).
    pub media: Vec<MediaId>,
    /// Timeline clips that use any of them.
    pub clips: usize,
}

/// Plans removing `chosen` (PM-058), counting the clips that use it.
pub fn plan_removal(doc: &Document, chosen: &BTreeSet<MediaId>) -> Removal {
    let library = doc.library();
    let media: Vec<MediaId> = chosen
        .iter()
        .copied()
        .filter(|id| library.get(*id).is_some())
        .collect();
    let clips = doc
        .sequences()
        .flat_map(|s| s.tracks().flat_map(|(_, t)| t.clips().iter()))
        .filter(|c| c.content.asset().is_some_and(|a| media.contains(&a)))
        .count();
    Removal { media, clips }
}

/// Removes the planned media (and, with `with_clips`, the clips that use
/// it) as one undo entry. Returns the status line.
pub fn remove(
    project: &mut Project,
    removal: &Removal,
    with_clips: bool,
) -> Result<String, String> {
    let n = removal.media.len();
    project.apply(
        &EditRequest::RemoveMedia {
            media: removal.media.clone(),
            with_clips,
        },
        Coalesce::Never,
    )?;
    Ok(if n == 1 {
        "Removed 1 item from the project".to_owned()
    } else {
        format!("Removed {n} items from the project")
    })
}

/// A rubber band in progress, in grid coordinates.
#[derive(Debug, Clone)]
struct Band {
    start: Vec2,
    end: Vec2,
    /// The selection before the band, kept when `Shift` (or `Ctrl` /
    /// `Cmd`) was held, restored on `Esc`.
    before: BTreeSet<MediaId>,
    additive: bool,
}

/// The Media tab's view state; reset when another project opens.
#[derive(Debug, Default)]
pub struct MediaState {
    pub filter: MediaFilter,
    pub search: String,
    pub sort: SortOrder,
    /// The library selection: view state, never on the undo stack.
    pub selection: BTreeSet<MediaId>,
    band: Option<Band>,
    /// A removal of used media waiting for the user's answer (PM-058).
    confirm: Option<Removal>,
}

impl MediaState {
    /// Chooses side-list row `i`.
    pub fn set_filter(&mut self, i: usize) {
        if let Some(f) = MediaFilter::ALL.get(i) {
            self.filter = *f;
        }
    }
}

#[derive(Debug, Default)]
pub struct MediaOutput {
    pub import_clicked: bool,
    pub status: Option<String>,
    /// Extract audio made (or found) this audio item; the panel shows it in
    /// the Audio tab.
    pub extracted: Option<MediaId>,
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut MediaState,
    posters: &HashMap<MediaId, TextureHandle>,
    waves: &Waveforms,
    importing: usize,
    chips: Option<&[Group]>,
) -> MediaOutput {
    let mut out = MediaOutput::default();
    let gap = ui.spacing().item_spacing.y.max(6.0);
    let bar = assets::toolbar(
        ui,
        Id::new("media-toolbar"),
        "Import video, audio and pictures (⌘I)",
        importing,
        &mut state.search,
        "Search media",
        &mut state.sort,
    );
    out.import_clicked = bar.import_clicked;
    if let Some(groups) = chips {
        ui.add_space(gap);
        if let Some(i) = assets::chip_row(ui, Id::new("media-chips"), groups, state.filter.index())
        {
            state.set_filter(i);
        }
    }
    ui.add_space(gap);

    let library = project.doc().library();
    state.selection.retain(|id| library.get(*id).is_some());
    let items = visible_items(library, state.filter, &state.search, state.sort);
    if items.is_empty() {
        empty(ui, project, state, &mut out);
        confirm_removal(ui, project, state, &mut out);
        return out;
    }
    if state.selection.len() > 1 {
        selection_strip(ui, project, state, &mut out);
        ui.add_space(gap);
    }
    grid(ui, project, state, &items, posters, waves, &mut out);
    confirm_removal(ui, project, state, &mut out);
    out
}

/// The reason-specific empty state (§13.14).
fn empty(ui: &mut egui::Ui, project: &Project, state: &mut MediaState, out: &mut MediaOutput) {
    let library = project.doc().library();
    if library.is_empty() {
        let e = empty_state_with(
            ui,
            icons::MEDIA,
            "Nothing imported yet",
            "Drop video, audio or pictures here, or import them",
            Some("Import…"),
            None,
        );
        out.import_clicked |= e.primary;
    } else if !library.items().any(|m| state.filter.admits(m)) {
        let (title, detail) = state.filter.none_yet();
        let e = empty_state_with(ui, icons::MEDIA, title, detail, None, Some("Show all"));
        if e.secondary {
            state.filter = MediaFilter::All;
        }
    } else {
        let title = format!("No media matches “{}”", state.search.trim());
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

/// "N selected" with a Delete button, above the grid (§8.2).
fn selection_strip(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut MediaState,
    out: &mut MediaOutput,
) {
    let p = theme::look(ui.ctx()).palette;
    let rect = assets::strip(ui, 28.0);
    ui.painter().rect_filled(rect, theme::R_MD, p.bg_well);
    let font = Type::Caption.style(ui.style()).resolve(ui.style());
    ui.painter().text(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{} selected", state.selection.len()),
        font.clone(),
        p.text_secondary,
    );
    let label = ui
        .painter()
        .layout_no_wrap("Delete".to_owned(), font, p.danger);
    let button = Rect::from_center_size(
        Pos2::new(
            rect.right() - 6.0 - (label.size().x + 16.0) / 2.0,
            rect.center().y,
        ),
        Vec2::new(label.size().x + 16.0, 20.0),
    );
    let response = ui
        .interact(button, Id::new("media-delete-selected"), Sense::click())
        .on_hover_text("Remove the selected items from the project (files stay on disk)");
    if response.hovered() {
        ui.painter()
            .rect_filled(button, theme::R_XS, p.danger.gamma_multiply(0.18));
    }
    ui.painter()
        .galley(button.center() - label.size() / 2.0, label, p.danger);
    let name = format!("Delete {} selected items", state.selection.len());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    if response.clicked() {
        let chosen = state.selection.clone();
        begin_removal(project, state, &chosen, out);
    }
}

/// Removes `chosen` at once when no clip uses it; otherwise asks first
/// (PM-058).
fn begin_removal(
    project: &mut Project,
    state: &mut MediaState,
    chosen: &BTreeSet<MediaId>,
    out: &mut MediaOutput,
) {
    let removal = plan_removal(project.doc(), chosen);
    if removal.media.is_empty() {
        return;
    }
    if removal.clips == 0 {
        out.status = Some(remove(project, &removal, false).unwrap_or_else(|e| e));
        state.selection.retain(|id| !removal.media.contains(id));
    } else {
        state.confirm = Some(removal);
    }
}

/// PM-058's question for media that timeline clips use.
fn confirm_removal(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut MediaState,
    out: &mut MediaOutput,
) {
    let Some(removal) = state.confirm.clone() else {
        return;
    };
    let mut answer = None;
    let n = removal.media.len();
    let title = if n == 1 {
        "Remove 1 item from the project?".to_owned()
    } else {
        format!("Remove {n} items from the project?")
    };
    let sheet = Sheet::new(Id::new("media-remove-confirm"), &title)
        .width(460.0)
        .show(ui.ctx(), |s| {
            s.body(|ui| {
                let p = theme::look(ui.ctx()).palette;
                let line = |ui: &mut egui::Ui, text: String| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .text_style(egui::TextStyle::Body)
                                .color(p.text_primary),
                        )
                        .wrap(),
                    );
                };
                line(
                    ui,
                    if removal.clips == 1 {
                        "1 clip on the timeline uses this media. It is removed too.".to_owned()
                    } else {
                        format!(
                            "{} clips on the timeline use this media. They are removed too.",
                            removal.clips
                        )
                    },
                );
                line(
                    ui,
                    "Files on disk are not touched, and Undo brings everything back.".to_owned(),
                );
            });
            s.footer(|ui| {
                if sheet::button(
                    ui,
                    ButtonKind::Danger,
                    None,
                    "Remove media and clips",
                    0.0,
                    None,
                    true,
                )
                .clicked()
                {
                    answer = Some(true);
                }
                if sheet::button(ui, ButtonKind::Secondary, None, "Cancel", 80.0, None, true)
                    .clicked()
                {
                    answer = Some(false);
                }
            });
        });
    if sheet.close && answer.is_none() {
        answer = Some(false);
    }
    match answer {
        Some(true) => {
            state.confirm = None;
            out.status = Some(remove(project, &removal, true).unwrap_or_else(|e| e));
            state.selection.retain(|id| !removal.media.contains(id));
        }
        Some(false) => state.confirm = None,
        None => {}
    }
}

/// The grid (§13.23): only the visible rows are laid out and painted;
/// the empty floor below the tiles takes the rubber band.
fn grid(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut MediaState,
    items: &[Arc<MediaItem>],
    posters: &HashMap<MediaId, TextureHandle>,
    waves: &Waveforms,
    out: &mut MediaOutput,
) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let compact = look.metrics.control_height < 28.0;
    let (min, gap) = if compact { (80.0, 6.0) } else { (96.0, 8.0) };
    egui::ScrollArea::vertical()
        .id_salt("media-grid")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let width = ui.available_width();
            let (cols, tile_w) = grid_layout(width, min, gap);
            let pitch = row_pitch(tile_w, gap);
            let rows = items.len().div_ceil(cols);
            let content_h = (rows as f32 * pitch - gap).max(0.0);
            // The floor reaches the bottom so the band can start below the
            // last row.
            let height = content_h.max(viewport.height());
            let (rect, floor) =
                ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
            let first = (viewport.min.y / pitch).floor().max(0.0) as usize;
            let last = ((viewport.max.y / pitch).ceil().max(0.0) as usize).min(rows);
            for row in first..last {
                for col in 0..cols {
                    let index = row * cols + col;
                    let Some(item) = items.get(index) else { break };
                    let cell = cell_rect(rect.min, index, cols, tile_w, gap);
                    media_tile(ui, project, state, item, cell, posters, waves, out);
                }
            }

            // The rubber band.
            let modifiers = ui.input(|i| i.modifiers);
            if floor.drag_started_by(egui::PointerButton::Primary)
                && let Some(pos) = floor.interact_pointer_pos()
            {
                let at = pos - rect.min;
                state.band = Some(Band {
                    start: at,
                    end: at,
                    before: state.selection.clone(),
                    additive: modifiers.shift || modifiers.command,
                });
            }
            if let Some(band) = &mut state.band {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    band.end = pos - rect.min;
                }
                let area = Rect::from_two_pos(band.start.to_pos2(), band.end.to_pos2());
                let cells: Vec<Rect> = (0..items.len())
                    .map(|i| cell_rect(Pos2::ZERO, i, cols, tile_w, gap))
                    .collect();
                let mut chosen: BTreeSet<MediaId> = rubber_band_hits(area, &cells)
                    .into_iter()
                    .filter_map(|i| items.get(i).map(|m| m.id))
                    .collect();
                if band.additive {
                    chosen.extend(band.before.iter().copied());
                }
                let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
                state.selection = if cancelled {
                    band.before.clone()
                } else {
                    chosen
                };
                let screen = area.translate(rect.min.to_vec2());
                ui.painter().rect(
                    screen,
                    theme::R_XS,
                    p.overlay_selection(),
                    Stroke::new(1.0, p.accent),
                    StrokeKind::Inside,
                );
                if cancelled || !ui.input(|i| i.pointer.primary_down()) {
                    state.band = None;
                }
            } else if floor.clicked() {
                state.selection.clear();
            }
        });
}

/// One tile: click selects, double-click adds, drag to the timeline,
/// right-click for its menu, the hover button removes it.
#[allow(clippy::too_many_arguments)] // one tile's whole context
fn media_tile(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut MediaState,
    item: &Arc<MediaItem>,
    cell: Rect,
    posters: &HashMap<MediaId, TextureHandle>,
    waves: &Waveforms,
    out: &mut MediaOutput,
) {
    let id = item.id;
    let drag_id = Id::new(("media-tile", id));
    let offline = project.offline.contains(&id);
    let selected = state.selection.contains(&id);
    let name = media_name(item);
    let extracted = matches!(item.source, MediaSource::AudioOf { .. });
    let visual = item.info.has_video() || item.info.is_still();
    let mark = if visual {
        surface::CLIP_VIDEO
    } else if extracted {
        surface::CLIP_AUDIO_EXTRACTED
    } else {
        surface::CLIP_AUDIO
    };
    let peaks = if visual || offline {
        None
    } else {
        peak_key(project.doc(), id, None).and_then(|key| match waves.get(key) {
            Some(PeakState::Ready(p)) => Some(Arc::clone(p)),
            _ => None,
        })
    };
    let picture = match (posters.get(&id), &peaks) {
        (Some(tex), _) if visual => Picture::Texture(tex.id(), tex.size_vec2()),
        (_, Some(p)) => Picture::Wave(p, item.info.duration),
        _ if item.info.is_still() => Picture::Glyph(icons::IMAGE),
        _ if visual => Picture::Glyph(icons::VIDEO),
        _ => Picture::Glyph(icons::MUSIC_NOTE),
    };
    let badge = if item.info.is_still() {
        still_format(&item.path.absolute)
    } else {
        short_duration(item.info.duration)
    };
    let mut spec = TileSpec {
        name: &name,
        mark,
        picture,
        badge: Some(badge),
        tag: extracted.then_some("Extracted"),
        offline,
        selected,
        hovered: false,
        corner: None,
    };
    let dragged = ui.ctx().is_being_dragged(drag_id);
    let banding = state.band.is_some();
    if dragged {
        // The tile stays in place at 45 % while its copy follows the
        // pointer.
        let mut faded = ui.painter().clone();
        faded.multiply_opacity(0.45);
        paint_tile(ui, &faded, cell, &spec);
    }
    let mut remove_clicked = false;
    let response = ui
        .scope_builder(UiBuilder::new().max_rect(cell), |ui| {
            ui.dnd_drag_source(drag_id, AssetDrag::Media(id), |ui| {
                let (rect, resp) = ui.allocate_exact_size(cell.size(), Sense::click());
                let hovered = !dragged && !banding && ui.rect_contains_pointer(rect);
                if hovered {
                    let r = corner_button_rect(thumb_rect(rect));
                    let button = ui
                        .interact(r, drag_id.with("remove"), Sense::click())
                        .on_hover_text("Remove from project");
                    remove_clicked = button.clicked();
                    spec.corner = Some(Corner::Remove {
                        hot: button.hovered(),
                    });
                }
                spec.hovered = hovered;
                paint_tile(ui, ui.painter(), rect, &spec);
                resp
            })
            .inner
        })
        .inner;
    if remove_clicked {
        begin_removal(project, state, &BTreeSet::from([id]), out);
        return;
    }
    if response.clicked() {
        let m = ui.input(|i| i.modifiers);
        if m.shift || m.command {
            if !state.selection.remove(&id) {
                state.selection.insert(id);
            }
        } else {
            state.selection = BTreeSet::from([id]);
        }
    }
    if response.double_clicked() {
        let main = project
            .doc()
            .sequence(project.doc().root())
            .and_then(|s| s.main_track().map(|t| (t.id, t.end())));
        let result = match main {
            Some((track, end)) if visual => {
                insert_media(project, id, InsertTarget::Track(track), end)
            }
            _ => add_at_playhead(project, id),
        };
        if let Err(e) = result {
            out.status = Some(e);
        }
    }
    let response = response.on_hover_cursor(CursorIcon::Grab);
    response.context_menu(|ui| {
        let hint = if visual {
            "On the main track at the playhead"
        } else {
            "On the first free audio track at the playhead"
        };
        if ui.button("Add at playhead").on_hover_text(hint).clicked() {
            ui.close();
            if let Err(e) = add_at_playhead(project, id) {
                out.status = Some(e);
            }
        }
        if item.info.has_video() {
            let button = egui::Button::new("Extract audio");
            if !item.info.has_audio() {
                ui.add_enabled(false, button)
                    .on_disabled_hover_text("This video has no sound");
            } else if ui
                .add(button)
                .on_hover_text("Add the sound as an audio item, without copying the file")
                .clicked()
            {
                ui.close();
                match project.extract_audio(id) {
                    Ok((audio, created)) => {
                        out.extracted = Some(audio);
                        out.status = Some(if created {
                            format!("Extracted the sound of {}", media_name(item))
                        } else {
                            format!("The sound of {} is already in Audio", media_name(item))
                        });
                    }
                    Err(e) => out.status = Some(e),
                }
            }
        }
        ui.separator();
        if ui.button("Copy file path").clicked() {
            ui.close();
            out.status = copy_file_path(ui.ctx(), project, id);
        }
        ui.separator();
        let many = selected && state.selection.len() > 1;
        let label = if many {
            format!("Remove {} items from project", state.selection.len())
        } else {
            "Remove from project".to_owned()
        };
        if ui
            .button(label)
            .on_hover_text("Files on disk are not touched")
            .clicked()
        {
            ui.close();
            let chosen = if many {
                state.selection.clone()
            } else {
                BTreeSet::from([id])
            };
            begin_removal(project, state, &chosen, out);
        }
    });
    response.on_hover_text(format!("{}\n{}", name, item.path.absolute));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::num::NonZeroU64;
    use std::path::PathBuf;
    use std::sync::Arc;

    use freecut_timeline::edit::NewMedia;
    use freecut_timeline::model::{
        AudioStreamInfo, MediaInfo, ProjectSettings, Rotation, StoredPath, VideoStreamInfo,
    };
    use freecut_timeline::time::Tick;

    use super::*;
    use crate::session::ProbedFile;

    fn info(video: bool, audio: bool, still: bool) -> MediaInfo {
        let d = Tick::from_secs(4).unwrap();
        MediaInfo {
            container: "mov".into(),
            default_video: video.then_some(0),
            duration: d,
            video: if video || still {
                vec![VideoStreamInfo {
                    index: 0,
                    codec: "h264".into(),
                    width: 640,
                    height: 360,
                    sar: (1, 1),
                    rotation: Rotation::None,
                    mirrored: false,
                    rate: None,
                    variable_rate: false,
                    has_alpha: false,
                    bit_depth: 8,
                    start: Tick::ZERO,
                    duration: d,
                }]
            } else {
                Vec::new()
            },
            audio: if audio {
                vec![AudioStreamInfo {
                    index: 1,
                    codec: "aac".into(),
                    sample_rate: 48_000,
                    channels: 2,
                    start: Tick::ZERO,
                    duration: d,
                    duration_exact: true,
                    default: true,
                    language: None,
                    title: None,
                    layout: None,
                }]
            } else {
                Vec::new()
            },
            still: still.then_some((640, 360)),
        }
    }

    fn id(n: u64) -> MediaId {
        MediaId::from_raw(NonZeroU64::new(n).unwrap())
    }

    fn item(n: u64, name: &str, info: MediaInfo, source: MediaSource) -> Arc<MediaItem> {
        Arc::new(MediaItem {
            id: id(n),
            source,
            path: StoredPath {
                absolute: format!("/m/{name}"),
                relative: None,
            },
            info,
            fingerprint: None,
            display_name: None,
            origin: freecut_timeline::model::MediaOrigin::Imported,
        })
    }

    fn library() -> Library {
        Library::from_parts(vec![
            item(1, "beach.mp4", info(true, true, false), MediaSource::File),
            item(2, "song.wav", info(false, true, false), MediaSource::File),
            item(3, "photo.png", info(false, false, true), MediaSource::File),
            item(
                4,
                "beach.mp4 (audio)",
                info(false, true, false),
                MediaSource::AudioOf {
                    source: id(1),
                    stream: 1,
                },
            ),
            item(5, "Clip B.mov", info(true, false, false), MediaSource::File),
        ])
        .unwrap()
    }

    #[test]
    fn side_list_counts_each_kind_and_made_here() {
        let lib = library();
        let still = &lib.get(id(3)).unwrap().info;
        assert!(still.is_still(), "{still:?}");
        assert_eq!(side_list_counts(&lib), [5, 2, 2, 1, 1]);
        assert_eq!(side_list_counts(&Library::default()), [0; 5]);
        let g = groups(&lib);
        assert_eq!(
            g.len(),
            2,
            "no Mine group: Favorites and Recent don't exist yet"
        );
        assert_eq!(g[0].rows[1], ("Video", 2));
        assert_eq!(
            g[1],
            Group {
                title: "Made here",
                rows: vec![("Extracted", 1)],
            }
        );
    }

    #[test]
    fn filters_search_and_sort_change_only_what_is_shown() {
        let lib = library();
        let names = |v: Vec<Arc<MediaItem>>| v.iter().map(|m| media_name(m)).collect::<Vec<_>>();
        assert_eq!(
            names(visible_items(
                &lib,
                MediaFilter::Video,
                "",
                SortOrder::Added
            )),
            ["beach.mp4", "Clip B.mov"]
        );
        assert_eq!(
            names(visible_items(
                &lib,
                MediaFilter::All,
                "BEACH",
                SortOrder::Added
            )),
            ["beach.mp4", "beach.mp4 (audio)"]
        );
        assert_eq!(
            names(visible_items(&lib, MediaFilter::All, "", SortOrder::Name)),
            [
                "beach.mp4",
                "beach.mp4 (audio)",
                "Clip B.mov",
                "photo.png",
                "song.wav"
            ]
        );
        assert_eq!(
            names(visible_items(&lib, MediaFilter::All, "", SortOrder::Kind)),
            [
                "beach.mp4",
                "Clip B.mov",
                "photo.png",
                "song.wav",
                "beach.mp4 (audio)"
            ]
        );
        // The library itself keeps its order.
        let ids: Vec<u64> = lib.items().map(|m| m.id.get()).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
    }

    fn probed(name: &str, info: MediaInfo) -> ProbedFile {
        ProbedFile {
            path: PathBuf::from(format!("/m/{name}")),
            media: NewMedia {
                path: StoredPath {
                    absolute: format!("/m/{name}"),
                    relative: None,
                },
                info,
                fingerprint: None,
                display_name: None,
            },
        }
    }

    /// PM-058 through the panel's path: several selected items go in one
    /// undo entry, and undo brings them back; used media takes its clips
    /// along only when asked.
    #[test]
    fn removing_several_items_is_one_undo_entry() {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let ids = project
            .import(vec![
                probed("a.mp4", info(true, true, false)),
                probed("b.wav", info(false, true, false)),
                probed("c.wav", info(false, true, false)),
            ])
            .unwrap();
        let (extracted, _) = project.extract_audio(ids[0]).unwrap();
        let entries = project.editor.history().len();
        let chosen = BTreeSet::from([ids[1], ids[2]]);
        let plan = plan_removal(project.doc(), &chosen);
        assert_eq!(plan.media, [ids[1], ids[2]]);
        assert_eq!(plan.clips, 0);
        assert_eq!(
            remove(&mut project, &plan, false).unwrap(),
            "Removed 2 items from the project"
        );
        assert_eq!(
            project.editor.history().len(),
            entries + 1,
            "one undo entry"
        );
        assert_eq!(project.doc().library().len(), 2);
        project.undo().unwrap();
        assert_eq!(project.doc().library().len(), 4);

        // A used video asks first; its extracted sound stays.
        add_at_playhead(&mut project, ids[0]).unwrap();
        let plan = plan_removal(project.doc(), &BTreeSet::from([ids[0]]));
        assert_eq!(plan.clips, 1);
        assert!(remove(&mut project, &plan, false).is_err(), "in use");
        remove(&mut project, &plan, true).unwrap();
        assert!(project.doc().library().get(ids[0]).is_none());
        assert!(project.doc().library().get(extracted).is_some());
        project.undo().unwrap();
        assert!(project.doc().library().get(ids[0]).is_some());
        assert!(
            plan_removal(project.doc(), &BTreeSet::new())
                .media
                .is_empty()
        );
    }

    /// Imports `n` sound files into a new project.
    fn project_with(n: usize) -> Project {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        if n > 0 {
            let files = (0..n)
                .map(|i| probed(&format!("take {i}.wav"), info(false, true, false)))
                .collect();
            project.import(files).unwrap();
        }
        project
    }

    /// One frame of the Media and Audio tabs at 400 × 600 pt; returns how
    /// many shapes each painted.
    fn frame_shapes(project: &mut Project) -> (usize, usize) {
        use eframe::egui::{Context, RawInput};

        use crate::theme::{Density, TOKENS, ThemeKind};
        let ctx = Context::default();
        theme::apply(&ctx, &TOKENS, ThemeKind::Dark, Density::Comfortable);
        let posters = HashMap::new();
        let waves = Waveforms::new(|| {});
        let mut media = MediaState::default();
        let mut audio = crate::ui::audio::AudioState::default();
        let count = |draw: &mut dyn FnMut(&mut egui::Ui)| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 600.0))),
                ..RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| draw(ui));
            output.textures_delta.clear();
            output.shapes.len()
        };
        let m = count(&mut |ui| {
            show(ui, project, &mut media, &posters, &waves, 0, None);
        });
        let groups = crate::ui::audio::groups(project.doc().library());
        let a = count(&mut |ui| {
            crate::ui::audio::show(ui, project, &mut audio, &waves, 2, Some(&groups));
        });
        (m, a)
    }

    /// The Drawer runs headless with 0, 1 and 500 items, and only the
    /// visible rows are laid out: 500 items paint no more than 40 do.
    #[test]
    fn the_drawer_lays_out_only_visible_rows() {
        let (empty_m, empty_a) = frame_shapes(&mut project_with(0));
        assert!(empty_m > 0 && empty_a > 0, "the empty states paint");
        frame_shapes(&mut project_with(1));
        let (m40, a40) = frame_shapes(&mut project_with(40));
        let (m500, a500) = frame_shapes(&mut project_with(500));
        assert!(m500 <= m40 + 8, "media: {m500} shapes vs {m40}");
        assert!(a500 <= a40 + 8, "audio: {a500} shapes vs {a40}");
    }
}

//! The asset panel (UI-004; ui-concept §8): the **Shelf**, a tab strip
//! that is the panel's header, over the **Drawer**: a resizable side list
//! of groups with counts beside the tab's content (a content toolbar and a
//! tile grid or list). Below 360 pt of panel width the side list becomes a
//! chip row under the toolbar.
//!
//! Only tabs with content in this build are shown, Media (PM-050), Audio
//! (AU-030's project audio), Text (TX-001's Add text tile) and Adjust
//! (CL-044: adjustment clips, saved looks and LUTs); the others stay
//! hidden until they exist. Favorites and Recent (UI-006) don't exist
//! yet, so no Drawer has a *Mine* group.

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, DragAndDrop, Id, Margin, Rect, Sense, TextureHandle, UiBuilder, Vec2};
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::Document;

use super::adjust::{self, AdjustFilter, AdjustState};
use super::audio::{self, AudioFilter, AudioState};
use super::media::{self, MediaState};
use super::text;
use crate::session::Project;
use crate::theme::{self, fonts, icons};
use crate::waveform::Waveforms;
use crate::widgets::icon_button::IconButton;
use crate::widgets::import_button::{import_button, import_label_width, import_width};
use crate::widgets::shelf_tab::{ShelfTab, shelf};
use crate::widgets::side_list::{self, clamp_side_width, side_list_visible};

/// A tab of the asset panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AssetTab {
    #[default]
    Media,
    Audio,
    Text,
    Adjust,
}

impl AssetTab {
    /// The tabs this build has content for, in order.
    pub const SHOWN: [AssetTab; 4] = [
        AssetTab::Media,
        AssetTab::Audio,
        AssetTab::Text,
        AssetTab::Adjust,
    ];

    /// The tab's icon (UI-004: icon + label; ui-concept §14's icon font).
    pub fn glyph(self) -> &'static str {
        match self {
            AssetTab::Media => icons::MEDIA,
            AssetTab::Audio => icons::AUDIO,
            AssetTab::Text => icons::TEXT,
            AssetTab::Adjust => icons::ADJUST,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AssetTab::Media => "Media",
            AssetTab::Audio => "Audio",
            AssetTab::Text => "Text",
            AssetTab::Adjust => "Adjust",
        }
    }

    /// The Shelf group (§8.1): *Bring in* (Media, Audio), *Write* (Text),
    /// *Grade* (Adjust).
    fn group(self) -> u8 {
        match self {
            AssetTab::Media | AssetTab::Audio => 0,
            AssetTab::Text => 1,
            AssetTab::Adjust => 2,
        }
    }
}

/// The panel's view state. It lives for the app session: the last active
/// tab, the side list's width and its folded groups survive switching
/// projects; searches, filters, sorting and the library selection don't.
#[derive(Debug)]
pub struct AssetsState {
    pub tab: AssetTab,
    pub audio: AudioState,
    pub media: MediaState,
    pub adjust: AdjustState,
    /// The side list's wanted width (§8.2); shown clamped to the panel.
    pub side_width: f32,
    /// Folded side-list groups: (tab, group index).
    folded: HashSet<(AssetTab, usize)>,
}

impl Default for AssetsState {
    fn default() -> AssetsState {
        AssetsState {
            tab: AssetTab::default(),
            audio: AudioState::default(),
            media: MediaState::default(),
            adjust: AdjustState::default(),
            side_width: side_list::DEFAULT_WIDTH,
            folded: HashSet::new(),
        }
    }
}

impl AssetsState {
    /// Another project was opened.
    pub fn reset(&mut self) {
        self.audio = AudioState::default();
        self.media = MediaState::default();
    }

    /// Switches to the Audio tab with `media` highlighted (see
    /// [`AudioState::reveal`]).
    pub fn reveal_audio(&mut self, doc: &Document, media: MediaId, filter: Option<AudioFilter>) {
        self.tab = AssetTab::Audio;
        self.audio.reveal(doc, media, filter);
    }
}

/// What an item dragged out of the asset panel carries (UI-005): the
/// timeline accepts it as a drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetDrag {
    /// A library item (Media and Audio tabs).
    Media(MediaId),
    /// The Text tab's Add text tile (TX-001).
    Text,
}

/// Which import the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRequest {
    /// Import in the Media tab: any media.
    Media,
    /// Import in the Audio tab: only files with sound (AU-001).
    Audio,
}

#[derive(Debug, Default)]
pub struct AssetsOutput {
    pub import: Option<ImportRequest>,
    pub status: Option<String>,
    /// The Add text tile was clicked (TX-001).
    pub add_text: bool,
    /// The Adjust tab's Import (CL-016).
    pub import_luts: bool,
    /// The Adjust tab's Delete… on a saved look: `(id, name)`.
    pub delete_look: Option<(String, String)>,
}

/// One side-list group: a title and its rows (label, item count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub title: &'static str,
    pub rows: Vec<(&'static str, usize)>,
}

/// The display order of a Drawer's items (§8.2's sort select). Sorting is
/// display-only: the library keeps its order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SortOrder {
    /// Library order: oldest first.
    #[default]
    Added,
    Name,
    Kind,
}

impl SortOrder {
    pub const ALL: [SortOrder; 3] = [SortOrder::Added, SortOrder::Name, SortOrder::Kind];

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::Added => "Date added",
            SortOrder::Name => "Name",
            SortOrder::Kind => "Kind",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            SortOrder::Added => icons::SORT_ADDED,
            SortOrder::Name => icons::SORT_NAME,
            SortOrder::Kind => icons::SORT_KIND,
        }
    }

    /// Sorts `items` (given in library order) for display: by name ignoring
    /// case, or by `kind` rank then library order. Stable, so ties keep
    /// library order.
    pub fn sort<T>(self, items: &mut [T], name: impl Fn(&T) -> String, kind: impl Fn(&T) -> u8) {
        match self {
            SortOrder::Added => {}
            SortOrder::Name => items.sort_by_cached_key(|t| name(t).to_lowercase()),
            SortOrder::Kind => items.sort_by_key(|t| kind(t)),
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut AssetsState,
    posters: &HashMap<MediaId, TextureHandle>,
    waves: &Waveforms,
    importing: usize,
) -> AssetsOutput {
    let mut out = AssetsOutput::default();
    let look = theme::look(ui.ctx());
    let compact = look.metrics.control_height < 28.0;

    // The Shelf. UI-004: no switching while something dragged from the
    // panel is in flight.
    let locked = DragAndDrop::has_any_payload(ui.ctx());
    let tabs = AssetTab::SHOWN.map(|t| ShelfTab {
        icon: t.glyph(),
        label: t.label(),
        group: t.group(),
    });
    let current = AssetTab::SHOWN
        .iter()
        .position(|t| *t == state.tab)
        .unwrap_or(0);
    let shelf_out = shelf(ui, Id::new("asset-shelf"), &tabs, current, compact, locked);
    if let Some(i) = shelf_out.picked {
        state.tab = AssetTab::SHOWN[i];
    }

    // The Drawer.
    let body = ui.available_rect_before_wrap();
    let tab = state.tab;
    let (groups, row) = match tab {
        AssetTab::Media => (
            media::groups(project.doc().library()),
            state.media.filter.index(),
        ),
        AssetTab::Audio => (
            audio::groups(project.doc().library()),
            state.audio.filter.index(),
        ),
        AssetTab::Text => (text::groups(), 0),
        AssetTab::Adjust => (adjust::groups(project), state.adjust.filter.index()),
    };
    let narrow = !side_list_visible(body.width());
    let mut picked = None;
    let content = if narrow {
        body
    } else {
        let width = clamp_side_width(state.side_width, body.width());
        let side = Rect::from_min_size(body.min, Vec2::new(width, body.height()));
        picked = side_panel(ui, state, tab, &groups, row, side, shelf_out.enter);
        let mut wanted = width;
        side_list::splitter(
            ui,
            Id::new("asset-side-splitter"),
            side.right(),
            body.y_range(),
            &mut wanted,
            body.width(),
        );
        if wanted != width {
            state.side_width = wanted;
        }
        Rect::from_min_max(egui::pos2(side.right() + 1.0, body.top()), body.max)
    };
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    egui::Frame::NONE
        .inner_margin(Margin::same(8))
        .show(&mut child, |ui| {
            ui.set_min_size((content.size() - Vec2::splat(16.0)).max(Vec2::ZERO));
            let chips = narrow.then_some(groups.as_slice());
            match tab {
                AssetTab::Media => {
                    let media = media::show(
                        ui,
                        project,
                        &mut state.media,
                        posters,
                        waves,
                        importing,
                        chips,
                    );
                    if media.import_clicked {
                        out.import = Some(ImportRequest::Media);
                    }
                    if let Some(id) = media.extracted {
                        state.reveal_audio(project.doc(), id, Some(AudioFilter::Extracted));
                    }
                    out.status = media.status;
                }
                AssetTab::Audio => {
                    let audio = audio::show(ui, project, &mut state.audio, waves, importing, chips);
                    if audio.import_clicked {
                        out.import = Some(ImportRequest::Audio);
                    }
                    out.status = audio.status;
                }
                AssetTab::Text => {
                    out.add_text = text::show(ui, chips).add_clicked;
                }
                AssetTab::Adjust => {
                    let adjust = adjust::show(ui, project, &mut state.adjust, importing, chips);
                    out.import_luts = adjust.import_luts;
                    out.delete_look = adjust.delete_look;
                    out.status = adjust.status;
                }
            }
        });
    ui.advance_cursor_after_rect(body);
    if let Some(i) = picked {
        pick_row(state, tab, i);
    }
    out
}

/// Chooses side-list row `i` (flat, across groups) of `tab`.
fn pick_row(state: &mut AssetsState, tab: AssetTab, i: usize) {
    match tab {
        AssetTab::Media => state.media.set_filter(i),
        AssetTab::Audio => {
            if let Some(f) = AudioFilter::ALL.get(i) {
                state.audio.filter = *f;
            }
        }
        AssetTab::Text => {}
        AssetTab::Adjust => {
            if let Some(f) = AdjustFilter::ALL.get(i) {
                state.adjust.filter = *f;
            }
        }
    }
}

/// The side list in `rect`: headings fold their groups; returns the row
/// picked this frame (flat index across groups).
fn side_panel(
    ui: &mut egui::Ui,
    state: &mut AssetsState,
    tab: AssetTab,
    groups: &[Group],
    current: usize,
    rect: Rect,
    enter: bool,
) -> Option<usize> {
    let mut picked = None;
    let len: usize = groups.iter().map(|g| g.rows.len()).sum();
    let row_id = |i: usize| Id::new(("asset-side-row", tab, i));
    if enter {
        ui.memory_mut(|m| m.request_focus(row_id(current)));
    }
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink(side_list::PADDING))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = side_list::ROW_GAP;
    egui::ScrollArea::vertical()
        .id_salt(("asset-side", tab))
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let mut flat = 0;
            let mut focused = false;
            for (g, group) in groups.iter().enumerate() {
                let open = !state.folded.contains(&(tab, g));
                let heading = side_list::heading(
                    ui,
                    Id::new(("asset-side-heading", tab, g)),
                    group.title,
                    open,
                );
                let fold = heading.clicked()
                    || (open
                        && heading.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)));
                if fold {
                    if open {
                        state.folded.insert((tab, g));
                    } else {
                        state.folded.remove(&(tab, g));
                    }
                }
                for (label, count) in &group.rows {
                    if open {
                        let r = side_list::row(ui, row_id(flat), label, *count, flat == current);
                        if r.clicked() {
                            picked = Some(flat);
                        }
                        focused |= r.has_focus();
                    }
                    flat += 1;
                }
            }
            if let Some(to) = side_list::key_step(ui, focused, current, len) {
                picked = Some(to);
                ui.memory_mut(|m| m.request_focus(row_id(to)));
            }
        });
    picked
}

/// The narrow panel's chip row (§8.2): every side-list row as a chip,
/// on one line that scrolls sideways. Returns the row picked.
pub(super) fn chip_row(
    ui: &mut egui::Ui,
    id: Id,
    groups: &[Group],
    current: usize,
) -> Option<usize> {
    let mut picked = None;
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut flat = 0;
                for group in groups {
                    for (label, _) in &group.rows {
                        if side_list::chip(ui, label, flat == current).clicked() {
                            picked = Some(flat);
                        }
                        flat += 1;
                    }
                }
            });
        });
    picked
}

/// What the content toolbar did.
#[derive(Debug, Default)]
pub(super) struct ToolbarOutput {
    pub import_clicked: bool,
}

/// The content toolbar (§8.2): the Import button, the search field and the
/// icon-only sort select, one control-height row.
pub(super) fn toolbar(
    ui: &mut egui::Ui,
    id: Id,
    import_tip: &str,
    importing: usize,
    search: &mut String,
    hint: &str,
    sort: &mut SortOrder,
) -> ToolbarOutput {
    let mut out = ToolbarOutput::default();
    let h = theme::look(ui.ctx()).metrics.control_height;
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let label_w = import_label_width(ui, "Import", importing);
            let icon_only = import_icon_only(ui.available_width(), label_w, 2.0 * h, 6.0);
            let import = import_button(ui, "Import", importing, icon_only);
            if importing == 0 {
                out.import_clicked = import.on_hover_text(import_tip).clicked();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Narrow too: the sort select drops its caret.
                sort_select(ui, sort, !icon_only);
                search_field(ui, id.with("search"), search, hint);
            });
        },
    );
    out
}

/// The narrowest search field the toolbar keeps: room for a typed term of
/// a dozen characters beside the magnifier and the clear button.
pub(crate) const SEARCH_MIN: f32 = 150.0;

/// Whether the toolbar's Import button shows its glyph alone: when its
/// label would leave the search field (beside a sort select `sort_w` wide,
/// `gap` between controls) narrower than [`SEARCH_MIN`] in `room`.
pub(super) fn import_icon_only(room: f32, label_w: f32, sort_w: f32, gap: f32) -> bool {
    room - import_width(label_w, false) - sort_w - 2.0 * gap < SEARCH_MIN
}

/// A search field with a leading magnifier and a clear button.
pub(super) fn search_field(ui: &mut egui::Ui, id: Id, text: &mut String, hint: &str) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let h = look.metrics.control_height;
    let width = ui.available_width().max(48.0);
    let right = if text.is_empty() { 8 } else { 24 };
    // A long hint ("Search project audio") in a narrow field would be cut
    // to "Searc…"; drop to the bare verb when the whole hint can't fit.
    // Narrower still, the magnifier alone says what the field is for.
    let font = egui::TextStyle::Body.resolve(ui.style());
    let fits = |t: &str| {
        let w = ui.fonts_mut(|f| {
            f.layout_no_wrap(t.to_owned(), font.clone(), p.text_tertiary)
                .size()
                .x
        });
        w + 26.0 + f32::from(right) <= width
    };
    let hint = [hint, "Search"]
        .into_iter()
        .find(|t| fits(t))
        .unwrap_or_default();
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .hint_text(hint)
        .margin(Margin {
            left: 26,
            // Room for the clear button only when there is text to clear.
            right,
            top: 0,
            bottom: 0,
        })
        .vertical_align(egui::Align::Center)
        .min_size(Vec2::new(width, h))
        .desired_width(width);
    let response = ui.add(edit);
    let rect = response.rect;
    ui.painter().text(
        egui::pos2(rect.left() + 9.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icons::SEARCH,
        fonts::icon_font(ui.ctx(), 13.0, false),
        p.text_tertiary,
    );
    if !text.is_empty() {
        let clear = Rect::from_center_size(
            egui::pos2(rect.right() - 13.0, rect.center().y),
            Vec2::splat(20.0),
        );
        // An unfocused field shows its text from the start, so a word
        // longer than the field runs on under the clear button: cover the
        // field's tail with its own fill (inside its outline).
        let cover = Rect::from_x_y_ranges(
            (clear.left() - 2.0)..=(rect.right() - 1.0),
            (rect.top() + 1.0)..=(rect.bottom() - 1.0),
        );
        let r = theme::R_MD.saturating_sub(1);
        ui.painter().rect_filled(
            cover,
            egui::CornerRadius {
                nw: 0,
                sw: 0,
                ne: r,
                se: r,
            },
            ui.visuals().text_edit_bg_color(),
        );
        let mut inner = ui.new_child(UiBuilder::new().max_rect(clear));
        if IconButton::new(icons::CLOSE, "Clear search")
            .glyph(12.0)
            .show(&mut inner)
            .clicked()
        {
            text.clear();
        }
    }
}

/// The icon-only sort select (§13.20): the current order's glyph and a
/// caret; the list names the orders with a check on the current one.
fn sort_select(ui: &mut egui::Ui, sort: &mut SortOrder, caret: bool) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        sort.icon(),
        0.0,
        egui::TextFormat::simple(fonts::icon_font(ui.ctx(), 14.0, false), p.text_secondary),
    );
    if caret {
        job.append(
            icons::CARET_DOWN,
            4.0,
            egui::TextFormat::simple(fonts::icon_font(ui.ctx(), 12.0, false), p.text_secondary),
        );
    }
    let button = egui::Button::new(job).min_size(Vec2::new(0.0, look.metrics.control_height));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(150.0);
        for order in SortOrder::ALL {
            let on = *sort == order;
            let mut row = egui::text::LayoutJob::default();
            row.append(
                order.icon(),
                0.0,
                egui::TextFormat::simple(fonts::icon_font(ui.ctx(), 13.0, false), p.text_secondary),
            );
            row.append(
                order.label(),
                8.0,
                egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), p.text_primary),
            );
            if on {
                row.append(
                    icons::CHECK,
                    12.0,
                    egui::TextFormat::simple(
                        fonts::icon_font(ui.ctx(), 12.0, false),
                        p.accent_text,
                    ),
                );
            }
            if ui.selectable_label(on, row).clicked() {
                *sort = order;
            }
        }
    });
    response.on_hover_text(format!("Sort by {}", sort.label().to_lowercase()));
}

/// Allocates a full-width strip `h` tall and returns its rect.
pub(super) fn strip(ui: &mut egui::Ui, h: f32) -> Rect {
    ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover())
        .0
}

/// The media panels' "Copy file path": the file where it was found on this
/// machine, else the path the project stores. Returns the status line.
pub fn copy_file_path(ctx: &egui::Context, project: &Project, media: MediaId) -> Option<String> {
    let path = project
        .media_paths
        .get(&media)
        .map(|p| p.display().to_string())
        .or_else(|| {
            project
                .doc()
                .library()
                .get(media)
                .map(|m| m.path.absolute.clone())
        })?;
    ctx.copy_text(path);
    Some("Copied the file path".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_toolbar_shows_import_as_a_glyph() {
        // 230 pt: the label would leave the search field under 150 pt.
        assert!(import_icon_only(230.0, 44.0, 56.0, 6.0));
        assert!(!import_icon_only(400.0, 44.0, 56.0, 6.0));
        assert!(import_width(44.0, true) < import_width(44.0, false));
    }

    #[test]
    fn sorting_is_display_only_and_stable() {
        let library = vec![("b.mp4", 0u8), ("A.wav", 2), ("c.png", 1), ("a.mov", 0)];
        let mut shown = library.clone();
        SortOrder::Added.sort(&mut shown, |t| t.0.to_owned(), |t| t.1);
        assert_eq!(shown, library);
        SortOrder::Name.sort(&mut shown, |t| t.0.to_owned(), |t| t.1);
        let names: Vec<_> = shown.iter().map(|t| t.0).collect();
        assert_eq!(names, ["a.mov", "A.wav", "b.mp4", "c.png"]);
        let mut shown = library.clone();
        SortOrder::Kind.sort(&mut shown, |t| t.0.to_owned(), |t| t.1);
        let names: Vec<_> = shown.iter().map(|t| t.0).collect();
        // Ties keep library order.
        assert_eq!(names, ["b.mp4", "a.mov", "c.png", "A.wav"]);
        assert_eq!(library[0].0, "b.mp4", "the source order is untouched");
    }
}

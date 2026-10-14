//! The inspector (UI-008, ui-concept §6, TX-054): details of the
//! selection, else the project.
//!
//! Anatomy: a header (kind icon, name, breadcrumb, the Detail switch, Split
//! and Delete), the lens bar, a banner when the track is locked, then the
//! lens's property cards in one scroll area, and a footer where the lens
//! has one. Lenses and cards per selection kind are in [`lens`]; the text
//! and placement cards in `super::props`; the sound cards (AU-002,
//! AU-005 … AU-008, AU-038) in `sound`.
//!
//! **Typing and selection changes (UI-008).** A field typed in keeps its
//! text until `Enter` or until it lets go. When the selection changes
//! while an inspector field holds focus, the inspector keeps showing the
//! clips the field belongs to: once no other gesture is open (a canvas
//! drag that changed the selection), it lets the field go, the typed value
//! is committed to those clips, and the next frame shows the new
//! selection. The cards' typing state lives under the selection's own ids,
//! so it never reaches another clip.

pub mod lens;
mod look;
pub mod motion;
mod sound;

use std::sync::OnceLock;
use std::time::Instant;

use eframe::egui::{self, RichText, TextStyle as UiTextStyle};
use freecut_timeline::edit::{KeyEdit, ResetScope, TextStyleEdit};
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    Clip, ClipContent, Document, PropGroup, PropPath, Tail, TailTip, TextCard, TextClipData,
    TextLayerKind, TextLens, TextPropPath, TextStyle, card_props,
};
use freecut_timeline::plan::ref_px_scale;
use freecut_timeline::time::{Tick, Timecode};

use self::lens::{
    CardId, Detail, Holds, Lens, SelectionKind, Workspace, lenses, selection_kind, widget_id,
};
use super::canvas::CanvasView;
use super::props::{self, CardEnv, Edit, FontChoices, Merge, PropsState, Steps};
use super::rows::{At, KeyView};
use super::timeline::{clip_label, text_label, track_name};
use super::{box_cards, stage, style_cards};
use crate::clip_sound::ClipCommand;
use crate::prop_keys;
use crate::session::Project;
use crate::theme::{self, Type, fonts, icons};
use crate::ui::drag::{DragGesture, Held};
use crate::widgets::card::{CardMemory, PropertyCard};
use crate::widgets::empty::empty_state;
use crate::widgets::icon_button::IconButton;
use crate::widgets::segmented::{Segment, Segmented};

/// Inspector state that outlives a frame.
#[derive(Debug, Default)]
pub struct InspectorState {
    /// The slider or field drag in progress: its edits merge into one undo
    /// entry (TL-043).
    drag: DragGesture,
    /// The text and placement cards, with the content edit session.
    pub props: PropsState,
    /// Lens memory, card collapse state and the Detail switch (UI-008).
    pub workspace: Workspace,
    /// The Motion lens: its slot, tiles and presets.
    pub motion: motion::MotionState,
    /// The selection shown last frame.
    shown: Vec<ClipId>,
    /// Where the inspector was drawn last frame.
    area: Option<(egui::LayerId, egui::Rect)>,
}

impl InspectorState {
    /// Ends a drag whose widget went away without reporting its end (the
    /// selection changed mid-drag, the panel closed), and commits a text
    /// edit whose field went away. Call once per frame, after the panels.
    /// Returns why a commit failed.
    pub fn settle(
        &mut self,
        mut project: Option<&mut Project>,
        pointer_down: bool,
    ) -> Option<String> {
        let status = self.props.settle(project.as_deref_mut());
        self.drag.settle(project, pointer_down);
        status
    }

    /// Forgets a gesture of a project that is no longer open.
    pub fn reset(&mut self) {
        self.drag.reset();
        self.props.reset();
        self.shown.clear();
    }

    /// The selection to show this frame: the project's, unless it changed
    /// while an inspector field holds focus (typed text not committed yet,
    /// UI-008). Then the clips the field belongs to stay until the field
    /// has let go and committed to them; while another gesture is open (the
    /// canvas drag that changed the selection) the field keeps its focus.
    fn showing(&mut self, ui: &egui::Ui, project: &Project) -> Vec<ClipId> {
        if project.selection == self.shown {
            return self.shown.clone();
        }
        let focused = ui.memory(|m| m.focused());
        let typing_here = focused
            .and_then(|id| ui.ctx().read_response(id))
            .zip(self.area)
            .is_some_and(|(r, (layer, rect))| {
                r.layer_id == layer && rect.contains(r.rect.center())
            });
        let doc = project.doc();
        let still_there =
            !self.shown.is_empty() && self.shown.iter().all(|c| doc.clip(*c).is_some());
        if typing_here && still_there {
            if !project.gesture_open()
                && let Some(id) = focused
            {
                // Drawn once more for its clips, the field commits there.
                ui.memory_mut(|m| m.surrender_focus(id));
            }
            ui.ctx().request_repaint();
            return self.shown.clone();
        }
        self.shown.clone_from(&project.selection);
        self.shown.clone()
    }
}

#[derive(Debug, Default)]
pub struct InspectorOutput {
    pub split: bool,
    pub delete: bool,
    pub command: Option<ClipCommand>,
    pub status: Option<String>,
    /// A keyframe arrow moved the playhead here (AU-008).
    pub seek: Option<Tick>,
    /// The LUT section's Import LUT… was picked (CL-016): the app opens the
    /// file dialog and runs the import.
    pub import_luts: bool,
}

/// The debug timer of ui-concept §16.3's budget (an inspector build for a
/// text clip with every card open under 2 ms): set
/// `FREECUT_INSPECTOR_TIMING` to print each build's time.
fn timing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FREECUT_INSPECTOR_TIMING").is_some())
}

/// What the lens bodies work on this frame.
struct Env<'a, 'v> {
    project: &'a mut Project,
    drag: &'a mut DragGesture,
    props: &'a mut PropsState,
    workspace: &'a mut Workspace,
    doc: Document,
    kind: SelectionKind,
    selection: Vec<ClipId>,
    fonts: &'a FontChoices,
    view: &'a CanvasView<'v>,
    out: &'a mut InspectorOutput,
    /// One clip shown: its keys at the playhead, for the navigators.
    keys: Option<&'a KeyView>,
    motion: &'a mut motion::MotionState,
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut InspectorState,
    fonts: &FontChoices,
    view: &CanvasView<'_>,
) -> InspectorOutput {
    let started = Instant::now();
    let mut out = InspectorOutput::default();
    // The header, the strip, the banner, the stack and the footer touch:
    // hairlines separate them (§6.1).
    ui.spacing_mut().item_spacing.y = 0.0;
    // Cheap: the document is Arc-structured.
    let doc = project.doc().clone();
    let selection = state.showing(ui, project);
    state.area = Some((ui.layer_id(), ui.max_rect()));
    let kind = selection_kind(&doc, &selection);
    let clips: Vec<&Clip> = selection
        .iter()
        .filter_map(|id| doc.clip(*id))
        .map(|c| c.as_ref())
        .collect();
    let holds = Holds::of(&clips);
    let any_text = holds.text;
    let available = lenses(kind, holds);
    let InspectorState {
        drag,
        props,
        workspace,
        motion,
        ..
    } = state;
    // Keyframe navigators for one visual clip (ui-concept §6.4: none for
    // several).
    let key_view = match clips.as_slice() {
        [c] if c.content.is_visual() => doc
            .clip(c.id)
            .map(|c| KeyView::new(std::sync::Arc::clone(c), project.playhead)),
        _ => None,
    };

    let Some(mut lens) = workspace.lens(kind, &available) else {
        header(ui, &doc, kind, &selection, workspace, None, &mut out);
        return out;
    };
    let single = (available.len() == 1).then_some(lens);
    header(ui, &doc, kind, &selection, workspace, single, &mut out);
    if available.len() > 1 {
        let items: Vec<Segment<'_>> = available
            .iter()
            .map(|l| Segment::new(Some(l.icon()), l.label()))
            .collect();
        let current = available.iter().position(|l| *l == lens).unwrap_or(0);
        if let Some(i) = lens_strip(ui, kind, &items, current)
            && let Some(l) = available.get(i)
        {
            lens = *l;
        }
        workspace.set_lens(kind, lens);
    }
    if let [id] = selection.as_slice()
        && props::on_locked_track(&doc, *id)
    {
        banner(ui, "Track is locked");
    }

    let sound_footer = matches!(
        (lens, kind),
        (Lens::Sound, SelectionKind::Video | SelectionKind::Audio)
    ) && clips.first().is_some_and(|c| c.audio.active().is_some());
    let text_footer = lens.text_lens().is_some() && any_text;
    let look_footer = lens == Lens::Look;
    let footer_height = if sound_footer || text_footer || look_footer {
        FOOTER
    } else {
        0.0
    };
    let several = (kind == SelectionKind::Several).then(|| selection_sentence(&clips));
    let mut env = Env {
        project,
        drag,
        props,
        workspace,
        doc: doc.clone(),
        kind,
        selection,
        fonts,
        view,
        out: &mut out,
        keys: key_view.as_ref(),
        motion,
    };
    // Typing state (a field's text) belongs to the clips shown: ids under
    // the selection, so a selection change never carries it to others.
    let salt = ("freecut-inspector-selection", env.selection.clone());
    // The wheel and the scrollbar scroll the stack; a mouse drag never
    // does, so dragging a field, a label or a rail can't move it.
    // The thin scrollbar floats over the sections' edge.
    ui.spacing_mut().scroll.floating_allocated_width = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("freecut-inspector-cards")
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .scroll_source(
            egui::scroll_area::ScrollSource::SCROLL_BAR
                | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
        )
        .max_height((ui.available_height() - footer_height).max(0.0))
        .show(ui, |ui| {
            // Sections touch: each keeps its own padding (§6.2).
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.push_id(salt, |ui| {
                if let Some(sentence) = &several {
                    selection_line(ui, sentence);
                }
                lens_body(ui, &mut env, lens);
            })
        });
    if sound_footer {
        footer(ui, |ui| sound::footer(ui, &mut env));
    } else if text_footer {
        footer(ui, |ui| self::text_footer(ui, &mut env, lens));
    } else if look_footer {
        footer(ui, |ui| look::footer(ui, &mut env));
    }
    if let Some(t) = key_view.as_ref().and_then(|v| v.seek.get()) {
        out.seek = Some(t);
    }
    if timing() {
        eprintln!(
            "FreeCut: inspector built in {} µs ({kind:?}, {lens:?})",
            started.elapsed().as_micros()
        );
    }
    out
}

/// The footer's height: a hairline and one row of buttons.
const FOOTER: f32 = 44.0;

/// The panel header (ui-concept §6.1, §13.16): 48 pt (40 Compact) with
/// the kind icon, the item's name in `type.title` and the breadcrumb in
/// `type.small` `text.tertiary` below it (ending with the lens name in
/// `type.micro` capitals when the kind has one lens); trailing, Delete,
/// Split and the Detail switch; a hairline along the bottom.
fn header(
    ui: &mut egui::Ui,
    doc: &Document,
    kind: SelectionKind,
    selection: &[ClipId],
    workspace: &mut Workspace,
    single: Option<Lens>,
    out: &mut InspectorOutput,
) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let m = look.metrics;
    let clip = match selection {
        [id] => doc.clip(*id),
        _ => None,
    };
    let (name, crumb) = match (kind, clip) {
        (SelectionKind::Nothing, _) => ("Project".to_owned(), "Nothing selected".to_owned()),
        (SelectionKind::Several, _) => (
            format!("{} clips selected", selection.len()),
            "Changes apply to every selected clip that has the setting".to_owned(),
        ),
        (_, Some(c)) => (
            clip_label(doc, &c.content, c.name.as_deref()),
            breadcrumb(doc, c),
        ),
        (_, None) => (String::new(), String::new()),
    };
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), m.panel_header),
        egui::Sense::hover(),
    );
    let inner = rect.shrink2(egui::vec2(m.side_inset, 0.0));
    // Trailing controls first: the name takes what they leave.
    let mut trailing = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("freecut-inspector-header")
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    trailing.spacing_mut().item_spacing.x = 2.0;
    if !matches!(kind, SelectionKind::Nothing) {
        out.delete = IconButton::new(icons::DELETE, "Delete the selected clips (Delete)")
            .glyph(14.0)
            .show(&mut trailing)
            .clicked();
        out.split = IconButton::new(icons::SPLIT, "Split at the playhead (S)")
            .glyph(14.0)
            .show(&mut trailing)
            .clicked();
        trailing.add_space(6.0);
    }
    let items = [
        Segment {
            icon: Some(icons::DETAIL_ESSENTIALS),
            label: "Essentials",
            tooltip: Some("the settings most people need"),
        },
        Segment {
            icon: Some(icons::DETAIL_EVERYTHING),
            label: "Everything",
            tooltip: Some("every setting, without More"),
        },
    ];
    let current = match workspace.detail {
        Detail::Essentials => 0,
        Detail::Everything => 1,
    };
    let detail = Segmented::new(egui::Id::new("freecut-inspector-detail"), &items, current)
        .icons_only(true)
        .width(56.0)
        .height(24.0)
        .show(&mut trailing);
    match detail.picked {
        Some(0) => workspace.detail = Detail::Essentials,
        Some(_) => workspace.detail = Detail::Everything,
        None => {}
    }
    let text_right = trailing.min_rect().left() - 8.0;

    let painter = ui.painter();
    let icon_x = inner.left() + 9.0;
    painter.text(
        egui::pos2(icon_x, rect.center().y),
        egui::Align2::CENTER_CENTER,
        kind.icon(),
        fonts::icon_font(ui.ctx(), 18.0, false),
        p.text_secondary,
    );
    let text_left = inner.left() + 26.0;
    let room = (text_right - text_left).max(0.0);
    let title_font = Type::Title.style(ui.style()).resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple_singleline(name, title_font, p.text_primary);
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let title = painter.layout_job(job);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &crumb,
        0.0,
        egui::TextFormat::simple(UiTextStyle::Small.resolve(ui.style()), p.text_tertiary),
    );
    if let Some(lens) = single {
        job.append(
            &lens.label().to_uppercase(),
            if crumb.is_empty() { 0.0 } else { 8.0 },
            egui::TextFormat {
                extra_letter_spacing: 0.8,
                valign: egui::Align::Center,
                ..egui::TextFormat::simple(
                    Type::Micro.style(ui.style()).resolve(ui.style()),
                    p.text_tertiary,
                )
            },
        );
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let crumb = painter.layout_job(job);
    let total = title.size().y + 1.0 + crumb.size().y;
    let top = rect.center().y - total / 2.0;
    painter.galley(egui::pos2(text_left, top), title, p.text_primary);
    let crumb_top = top + total - crumb.size().y;
    painter.galley(egui::pos2(text_left, crumb_top), crumb, p.text_tertiary);
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, p.border_subtle),
    );
}

/// The lens strip (ui-concept §6.1, §13.10): 40 pt (34 Compact) holding a
/// segmented control across the panel inside the side insets, with a
/// hairline below. Labels drop to icons when they don't fit. Returns the
/// lens picked.
fn lens_strip(
    ui: &mut egui::Ui,
    kind: SelectionKind,
    items: &[Segment<'_>],
    current: usize,
) -> Option<usize> {
    let look = theme::look(ui.ctx());
    let m = look.metrics;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), m.lens_strip),
        egui::Sense::hover(),
    );
    let mut strip = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("freecut-lens-strip")
            .max_rect(rect.shrink2(egui::vec2(m.side_inset, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let picked = Segmented::new(egui::Id::new(("freecut-lens-bar", kind)), items, current)
        .fill_width()
        .words_or_icons()
        .show(&mut strip)
        .picked;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, look.palette.border_subtle),
    );
    picked
}

/// "Editing 3 titles", "Editing 2 videos and a title" (ui-concept §6.4).
fn selection_sentence(clips: &[&Clip]) -> String {
    let mut counts: Vec<(&str, &str, usize)> = Vec::new();
    for clip in clips {
        let (one, many) = match &clip.content {
            ClipContent::Text(_) => ("title", "titles"),
            ClipContent::Media {
                kind: freecut_timeline::model::MediaKind::Video,
                ..
            } => ("video", "videos"),
            ClipContent::Media { .. } => ("sound", "sounds"),
            ClipContent::Image { .. } => ("picture", "pictures"),
            ClipContent::Generated { .. } => ("element", "elements"),
        };
        match counts.iter_mut().find(|(o, _, _)| *o == one) {
            Some(entry) => entry.2 += 1,
            None => counts.push((one, many, 1)),
        }
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(one, many, n)| match n {
            1 => format!("a {one}"),
            n => format!("{n} {many}"),
        })
        .collect();
    let list = match parts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("Editing {list}")
}

/// The one-line sentence at the top of a multi-selection's stack.
fn selection_line(ui: &mut egui::Ui, sentence: &str) {
    let look = theme::look(ui.ctx());
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(look.metrics.side_inset as i8, 8))
        .show(ui, |ui| {
            ui.label(
                RichText::new(sentence)
                    .text_style(UiTextStyle::Small)
                    .color(look.palette.text_secondary),
            );
        });
}

/// A lens footer: a hairline, then `add` laid out in one row inside the
/// side insets.
fn footer(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let look = theme::look(ui.ctx());
    let top = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        top + 0.5,
        egui::Stroke::new(1.0, look.palette.border_subtle),
    );
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(look.metrics.side_inset as i8, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// "V2 · 0:02.00 – 0:07.50".
fn breadcrumb(doc: &Document, clip: &Clip) -> String {
    let rate = doc.settings().frame_rate;
    let tc = |t: Tick| {
        Timecode::from_tick(t, rate, false).map_or_else(|_| "--".into(), |t| t.to_string())
    };
    let track = doc
        .find_clip(clip.id)
        .map(|loc| track_name(doc, loc.seq, loc.track));
    let end = clip.start().checked_add(clip.duration()).ok();
    let range = format!(
        "{} – {}",
        tc(clip.start()),
        end.map_or_else(|| "--".to_owned(), tc)
    );
    match track {
        Some(t) => format!("{t} · {range}"),
        None => range,
    }
}

/// The banner slot (ui-concept §13.13): one line with a warning bar,
/// full width under the lens strip.
fn banner(ui: &mut egui::Ui, text: &str) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let frame = egui::Frame::new()
        .fill(p.bg_hover)
        .inner_margin(egui::Margin::symmetric(look.metrics.side_inset as i8, 8));
    let response = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::WARNING)
                        .font(fonts::icon_font(ui.ctx(), 14.0, false))
                        .color(p.warning),
                );
                ui.label(RichText::new(text).color(p.text_primary));
            });
        })
        .response;
    let bar = egui::Rect::from_min_size(response.rect.min, egui::vec2(3.0, response.rect.height()));
    ui.painter().rect_filled(bar, 0.0, p.warning);
}

/// One card of `kind` with its remembered state.
fn card<R>(
    ui: &mut egui::Ui,
    workspace: &mut Workspace,
    kind: SelectionKind,
    id: CardId,
    title: &str,
    summary: String,
    body: impl FnOnce(&mut egui::Ui, &mut CardMemory) -> R,
) -> Option<R> {
    let memory = workspace.card(kind, id);
    PropertyCard::new(widget_id(kind, id, "card"), title)
        .summary(summary)
        .show(ui, memory, body)
        .inner
}

fn lens_body(ui: &mut egui::Ui, env: &mut Env<'_, '_>, lens: Lens) {
    // TX-054: a clip on a locked track is read-only, under UI-008's banner.
    let locked = matches!(env.selection.as_slice(), [id] if props::on_locked_track(&env.doc, *id));
    if locked && (lens.text_lens().is_some() || matches!(lens, Lens::Motion | Lens::Look)) {
        ui.disable();
    }
    match lens {
        Lens::Project => project_cards(ui, env),
        Lens::Content => content_cards(ui, env),
        Lens::Style => style_cards(ui, env),
        Lens::BoxBubble => box_cards(ui, env),
        Lens::Picture => picture_cards(ui, env),
        Lens::Look => look::cards(ui, env),
        Lens::Sound => sound::cards(ui, env),
        Lens::Motion => motion::cards(ui, env),
        Lens::Presets => {
            let inset = theme::look(ui.ctx()).metrics.side_inset as i8;
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(inset, 12))
                .show(ui, |ui| {
                    empty_state(
                        ui,
                        icons::LENS_PRESETS,
                        "Looks and templates",
                        "FreeCut's text looks and the looks you save will appear here.",
                    );
                });
        }
    }
    if matches!(lens, Lens::Content | Lens::Picture | Lens::Sound) {
        clip_info(ui, env);
    }
}

fn status(env: &mut Env<'_, '_>, status: Option<String>) {
    if status.is_some() {
        env.out.status = status;
    }
}

/// The selected text clips with their data, in selection order.
fn selected_texts(env: &Env<'_, '_>) -> Vec<(ClipId, std::sync::Arc<TextClipData>)> {
    env.selection
        .iter()
        .filter_map(|id| env.doc.clip(*id))
        .filter_map(|c| match &c.content {
            ClipContent::Text(data) => Some((c.id, data.clone())),
            _ => None,
        })
        .collect()
}

/// The first selected text clip's data (values shown for several), with
/// keyed values as they are at the playhead (TX-021).
fn first_text(env: &Env<'_, '_>) -> Option<std::sync::Arc<freecut_timeline::model::TextClipData>> {
    let playhead = env.project.playhead;
    env.selection
        .iter()
        .filter_map(|id| env.doc.clip(*id))
        .find_map(|c| props::shown_text(c, playhead))
}

fn project_cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let doc = env.doc.clone();
    let s = doc.settings();
    let kind = env.kind;
    let summary = format!("{} × {} · {}", s.width, s.height, s.frame_rate);
    card(
        ui,
        env.workspace,
        kind,
        CardId::Project,
        "Project",
        summary,
        |ui, _| {
            for (row, label, value) in [
                ("canvas", "Canvas", format!("{} × {}", s.width, s.height)),
                ("rate", "Frame rate", s.frame_rate.to_string()),
                ("audio", "Audio", format!("{} Hz", s.sample_rate.hz())),
                ("media", "Media items", doc.library().len().to_string()),
            ] {
                super::rows::readout(
                    ui,
                    widget_id(kind, CardId::Project, row),
                    label,
                    &value,
                    false,
                );
            }
        },
    );
    let look = theme::look(ui.ctx());
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(look.metrics.side_inset as i8, 12))
        .show(ui, |ui| {
            ui.label(
                RichText::new("Select a clip on the timeline to see its details.")
                    .text_style(UiTextStyle::Small)
                    .color(look.palette.text_tertiary),
            );
        });
}

/// The read-only Clip info card (UI-008), collapsed at first.
fn clip_info(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let [id] = env.selection.as_slice() else {
        return;
    };
    let Some(clip) = env.doc.clip(*id) else {
        return;
    };
    let rate = env.doc.settings().frame_rate;
    let tc = |t: Tick| {
        Timecode::from_tick(t, rate, false).map_or_else(|_| "--".into(), |t| t.to_string())
    };
    let kind = env.kind;
    let mut rows = vec![
        ("start", "Start", tc(clip.start())),
        ("duration", "Duration", tc(clip.duration())),
    ];
    if let ClipContent::Media { source, kind, .. } = &clip.content {
        rows.push(("kind", "Kind", format!("{kind:?}")));
        rows.push(("in", "Source in", tc(source.start())));
        rows.push(("out", "Source out", tc(source.end())));
    }
    let summary = format!("{} long", tc(clip.duration()));
    card(
        ui,
        env.workspace,
        kind,
        CardId::ClipInfo,
        "Clip info",
        summary,
        |ui, _| {
            for (row, label, value) in rows {
                super::rows::readout(
                    ui,
                    widget_id(kind, CardId::ClipInfo, row),
                    label,
                    &value,
                    true,
                );
            }
        },
    );
}

fn content_cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let Some(data) = first_text(env) else {
        return;
    };
    let kind = env.kind;
    // TX-002: the content field only for one clip.
    if let [id] = env.selection.as_slice() {
        let id = *id;
        let summary = text_label(data.content.as_str());
        let field = card(
            ui,
            env.workspace,
            kind,
            CardId::Text,
            "Text",
            summary,
            |ui, _| {
                let typed =
                    props::content_field(ui, env.project, env.props, id, data.content.as_str());
                let cased = props::change_case_row(ui, env.project, env.props, id, kind);
                typed.or(cased)
            },
        );
        status(env, field.flatten());
    }
    let view = env.view;
    text_card(
        ui,
        env,
        &data,
        CardId::Paragraph,
        "Paragraph",
        props::paragraph_summary(&data.layout),
        |ui, cenv, _, memory| props::paragraph_card(ui, cenv, &data, view, memory),
    );
    // TX-035 converts text clips to captions; it comes with caption tracks.
    let memory = env.workspace.card(kind, CardId::Convert);
    PropertyCard::new(widget_id(kind, CardId::Convert, "card"), "Convert")
        .summary("to captions: not yet")
        .show(ui, memory, |ui, _| {
            ui.add_enabled(false, egui::Button::new("Convert to captions"))
                .on_disabled_hover_text("Arrives with caption tracks (TX-035)");
        });
}

/// What a text card's header asked for this frame.
#[derive(Debug, Default)]
struct HeaderActions {
    switched: Option<bool>,
    reset: bool,
    remove: bool,
    keys: KeyActions,
}

/// A card's keyframes at the playhead (TX-021, TR-028): for its `...`
/// key commands and its chip's diamond. Read from the first selected clip
/// that has the card's properties.
#[derive(Debug, Clone, Copy)]
struct CardKeys {
    group: PropGroup,
    nav: prop_keys::KeyNav,
    /// The card has properties that take keys.
    keyable: bool,
    /// A key of the card sits at the playhead.
    here: bool,
}

impl CardKeys {
    fn animated(&self) -> bool {
        matches!(
            self.nav.state,
            crate::widgets::keyframe::KeyState::Animated { .. }
        )
    }
}

fn card_keys(env: &Env<'_, '_>, group: PropGroup) -> Option<CardKeys> {
    let playhead = env.project.playhead;
    let clip = env
        .selection
        .iter()
        .filter_map(|id| env.doc.clip(*id))
        .find(|c| {
            !group.props(c).is_empty() || c.keys.tracks().iter().any(|t| group.contains(t.prop()))
        })?;
    Some(CardKeys {
        group,
        nav: prop_keys::group_nav(clip, group, playhead),
        keyable: !group.props(clip).is_empty(),
        here: prop_keys::group_key_at(clip, group, playhead),
    })
}

/// The key commands picked from a card's `...` menu.
#[derive(Debug, Default, Clone, Copy)]
struct KeyActions {
    key_card: bool,
    clear_here: bool,
    prev: bool,
    next: bool,
}

/// The `...` menu's key commands (TX-021): **Key this card**, **Remove
/// this card's keys here**, **Previous key in card**, **Next key in card**.
fn key_menu(ui: &mut egui::Ui, keys: &CardKeys, a: &mut KeyActions) {
    ui.separator();
    let item = |ui: &mut egui::Ui, on: bool, (icon, text): (&str, &str), tip: &str, off: &str| {
        let label = crate::widgets::icon_then_text(ui, icon, text);
        let r = ui
            .add_enabled(on, egui::Button::new(label))
            .on_hover_text(tip)
            .on_disabled_hover_text(off);
        let clicked = r.clicked();
        if clicked {
            ui.close();
        }
        clicked
    };
    a.key_card |= item(
        ui,
        keys.keyable && keys.nav.under_playhead,
        (icons::DIAMOND, "Key this card"),
        "A keyframe at the playhead on every setting of this card",
        "Move the playhead onto this clip to add keyframes",
    );
    a.clear_here |= item(
        ui,
        keys.here,
        (icons::MINUS, "Remove this card's keys here"),
        "Remove every keyframe of this card at the playhead",
        "This card has no keyframe at the playhead",
    );
    a.prev |= item(
        ui,
        keys.nav.prev.is_some(),
        (icons::CARET_LEFT, "Previous key in card"),
        "Move the playhead to this card's previous keyframe",
        "No keyframe of this card before the playhead",
    );
    a.next |= item(
        ui,
        keys.nav.next.is_some(),
        (icons::CARET_RIGHT, "Next key in card"),
        "Move the playhead to this card's next keyframe",
        "No keyframe of this card after the playhead",
    );
}

/// Commits a card's key commands: each edit one undo entry; Previous and
/// Next move the playhead.
fn key_steps(env: &mut Env<'_, '_>, keys: &CardKeys, a: &KeyActions) -> Option<String> {
    let playhead = env.project.playhead;
    // Ignored by the card edits.
    let prop = PropPath::Reveal;
    let mut steps = Steps::default();
    if a.key_card {
        let edit = KeyEdit::KeyCard {
            card: keys.group,
            at: playhead,
        };
        steps.edit(Edit::Keys { prop, edit }, Merge::Own);
    }
    if a.clear_here {
        let edit = KeyEdit::ClearCard {
            card: keys.group,
            at: Some(playhead),
        };
        steps.edit(Edit::Keys { prop, edit }, Merge::Own);
    }
    if a.prev && keys.nav.prev.is_some() {
        env.out.seek = keys.nav.prev;
    }
    if a.next && keys.nav.next.is_some() {
        env.out.seek = keys.nav.next;
    }
    props::commit(env.project, env.drag, &env.selection, steps, Held::Up)
}

/// A text card (TX-054) with its header: the summary chip, the on/off
/// switch for an optional layer, and the `...` menu with **Reset card**
/// (and **Remove layer** for the Style lens's layers). `body` draws the
/// rows and commits them.
fn text_card(
    ui: &mut egui::Ui,
    env: &mut Env<'_, '_>,
    data: &TextClipData,
    id: CardId,
    title: &str,
    summary: String,
    body: impl FnOnce(
        &mut egui::Ui,
        &mut CardEnv<'_>,
        &mut PropsState,
        &mut CardMemory,
    ) -> Option<String>,
) {
    let kind = env.kind;
    let layer = id.layer();
    let removable = matches!(id, CardId::Outline | CardId::Shadow | CardId::Glow);
    let mut actions = HeaderActions::default();
    let everything = env.workspace.detail == Detail::Everything;
    let texts = selected_texts(env);
    let keys = id
        .text_card()
        .and_then(|card| card_keys(env, PropGroup::Text(card)))
        .filter(|k| k.keyable || k.animated());
    let dirty = id
        .text_card()
        .is_some_and(|card| texts.iter().any(|(_, d)| card_dirty(d, card)));
    let memory = env.workspace.card(kind, id);
    let mut header = PropertyCard::new(widget_id(kind, id, "card"), title)
        .summary(summary)
        .keyed(keys.is_some_and(|k| k.animated()))
        .reset(dirty, "Reset this section to the default text style");
    if let Some(help) = section_help(id) {
        header = header.help(help);
    }
    if let Some(layer) = layer {
        header = header.switch(data.style.layer_on(layer), switch_tip(layer));
    }
    let (reset, remove, key_actions) = (&mut actions.reset, &mut actions.remove, &mut actions.keys);
    header = header.menu(move |ui| {
        if ui
            .button(crate::widgets::icon_then_text(
                ui,
                icons::RESET,
                "Reset section",
            ))
            .on_hover_text("This section's settings back to the default text style")
            .clicked()
        {
            *reset = true;
            ui.close();
        }
        if removable
            && ui
                .button(crate::widgets::icon_then_text(
                    ui,
                    icons::DELETE,
                    "Remove layer",
                ))
                .on_hover_text("Switch the layer off and forget its settings")
                .clicked()
        {
            *remove = true;
            ui.close();
        }
        if let Some(keys) = &keys {
            key_menu(ui, keys, key_actions);
        }
    });
    let mut cenv = CardEnv {
        project: &mut *env.project,
        drag: &mut *env.drag,
        clips: &env.selection,
        texts: &texts,
        kind,
        everything,
        keys: env.keys,
    };
    let props = &mut *env.props;
    let shown = header.show(ui, memory, |ui, memory| body(ui, &mut cenv, props, memory));
    actions.switched = shown.switched;
    actions.reset |= shown.reset;
    status(env, shown.inner.flatten());
    let result = header_steps(env, data, id, &actions);
    status(env, result);
    if let Some(keys) = &keys {
        let result = key_steps(env, keys, &actions.keys);
        status(env, result);
    }
}

/// Whether `card` of `data` differs from the factory text style: what
/// its reset badge would change (the text and the layer switches stay).
fn card_dirty(data: &TextClipData, card: TextCard) -> bool {
    // Without the text: only the card's settings are compared.
    let base = TextClipData {
        content: freecut_timeline::model::RichText::new(String::new()),
        style: data.style.clone(),
        layout: data.layout,
        origin: data.origin,
    };
    let reset = after_reset(&base, &[card]);
    reset.style != base.style || reset.layout != base.layout
}

/// The help badge's text of a section that needs explaining (ui-concept
/// §7: the Shadow and Background switches carry one).
fn section_help(id: CardId) -> Option<&'static str> {
    Some(match id {
        CardId::Shadow => {
            "A soft copy of the text behind it. Distance and angle move it; blur softens \
             its edge. Switching it off keeps its settings."
        }
        CardId::Glow => "A haze of colour around the letters, outward, inward or both.",
        CardId::Background => {
            "A box behind the text. A text has a background or a bubble, not both: \
             switching this on switches the bubble off."
        }
        CardId::Bubble => {
            "A speech or thought bubble around the text, with a tail you can point on the \
             Stage. Switching it on switches the background off."
        }
        CardId::Curve => {
            "Bends the lines into an arch (above 0) or a bowl (below 0). While the text is \
             curved, a fixed box width pauses."
        }
        _ => return None,
    })
}

/// Commits a card header's switch, reset or remove.
fn header_steps(
    env: &mut Env<'_, '_>,
    data: &TextClipData,
    id: CardId,
    actions: &HeaderActions,
) -> Option<String> {
    let mut steps = Steps::default();
    if let (Some(on), Some(layer)) = (actions.switched, id.layer()) {
        let mut edits = vec![Edit::Style(TextStyleEdit::SetLayer { layer, on })];
        // TX-055: a bubble switched on for the first time points its tail
        // by the rule, measured on the real text box.
        if on
            && layer == TextLayerKind::Bubble
            && data.style.bubble.value.spec.tail.value.tip == Tail::default().tip
            && let Some(tip) = measured_tail_tip(env, &data.style)
        {
            edits.push(Edit::Style(TextStyleEdit::BubbleTailTip(tip)));
        }
        steps.group(edits);
    }
    if let Some(card) = id.text_card() {
        if actions.reset {
            steps.group(reset_edits(env, data, ResetScope::Card(card), &[card]));
        }
        if actions.remove
            && let Some(layer) = id.layer()
        {
            steps.group(vec![
                Edit::Style(TextStyleEdit::SetLayer { layer, on: false }),
                Edit::Reset(ResetScope::Card(card)),
            ]);
        }
    }
    props::commit(env.project, env.drag, &env.selection, steps, Held::Up)
}

/// TX-055's default tail tip for the selected clip's measured text box,
/// with `style`'s bubble.
fn measured_tail_tip(env: &Env<'_, '_>, style: &TextStyle) -> Option<TailTip> {
    let [clip] = env.selection.as_slice() else {
        return None;
    };
    let layer = env.view.layer(*clip)?;
    let scale = ref_px_scale(env.view.canvas);
    (scale > 0.0)
        .then(|| stage::default_tail_tip((layer.size.0 / scale, layer.size.1 / scale), style))
}

/// **Reset card** or **Reset lens** (TX-054) as one entry. The model puts
/// a bubble's tail tip back to its unmeasured default; a bubble that stays
/// on gets TX-055's tip for the measured box in the same entry, as on
/// switch-on (the round-2 model's *Reset* note).
fn reset_edits(
    env: &Env<'_, '_>,
    data: &TextClipData,
    scope: ResetScope,
    cards: &[TextCard],
) -> Vec<Edit> {
    let mut edits = vec![Edit::Reset(scope)];
    if data.style.bubble.on
        && cards.contains(&TextCard::Bubble)
        && let Some(tip) = measured_tail_tip(env, &after_reset(data, cards).style)
    {
        edits.push(Edit::Style(TextStyleEdit::BubbleTailTip(tip)));
    }
    edits
}

/// `data` with `cards` back at the factory default, as `ResetText` makes
/// it (the content and the layer switches stay).
fn after_reset(data: &TextClipData, cards: &[TextCard]) -> TextClipData {
    let defaults = TextClipData::default();
    let mut out = data.clone();
    for card in cards {
        for p in card_props(*card) {
            if *p != TextPropPath::Content && !p.is_layer_switch() {
                p.copy(&defaults, &mut out);
            }
        }
    }
    out
}

fn switch_tip(layer: TextLayerKind) -> &'static str {
    match layer {
        TextLayerKind::Outline => "Outline on or off; its settings are kept",
        TextLayerKind::Shadow => "Shadow on or off; its settings are kept",
        TextLayerKind::Glow => "Glow on or off; its settings are kept",
        TextLayerKind::Background => {
            "Background box on or off. Switching it on switches the bubble off"
        }
        TextLayerKind::Bubble => "Bubble on or off. Switching it on switches the background off",
        TextLayerKind::Curve => "Curve on or off; the amount is kept",
    }
}

fn style_cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let Some(data) = first_text(env) else {
        return;
    };
    let style = &data.style;
    let fonts = env.fonts;
    text_card(
        ui,
        env,
        &data,
        CardId::Type,
        "Type",
        style_cards::type_summary(style),
        |ui, cenv, props, memory| style_cards::type_card(ui, cenv, props, style, fonts, memory),
    );
    text_card(
        ui,
        env,
        &data,
        CardId::Fill,
        "Fill",
        style_cards::fill_summary(&style.fill),
        |ui, cenv, props, memory| style_cards::fill_card(ui, cenv, props, style, memory),
    );
    text_card(
        ui,
        env,
        &data,
        CardId::Spacing,
        "Spacing",
        style_cards::spacing_summary(style, &data.layout),
        |ui, cenv, _, _| style_cards::spacing_card(ui, cenv, style, &data.layout),
    );
    let defaults = TextStyle::default();
    let mut hidden = Vec::new();
    // An optional layer's card shows once it was added: switched on, or
    // switched off with its settings kept (ui-concept §6.2).
    if style.stroke != defaults.stroke {
        text_card(
            ui,
            env,
            &data,
            CardId::Outline,
            "Outline",
            layer_chip(
                style.stroke.on,
                style_cards::outline_summary(&style.stroke.value),
            ),
            |ui, cenv, _, memory| style_cards::outline_card(ui, cenv, &style.stroke.value, memory),
        );
    } else {
        hidden.push((TextLayerKind::Outline, "Outline"));
    }
    if style.shadow != defaults.shadow {
        text_card(
            ui,
            env,
            &data,
            CardId::Shadow,
            "Shadow",
            layer_chip(
                style.shadow.on,
                style_cards::shadow_summary(&style.shadow.value),
            ),
            |ui, cenv, _, memory| style_cards::shadow_card(ui, cenv, &style.shadow.value, memory),
        );
    } else {
        hidden.push((TextLayerKind::Shadow, "Shadow"));
    }
    if style.glow != defaults.glow {
        text_card(
            ui,
            env,
            &data,
            CardId::Glow,
            "Glow",
            layer_chip(style.glow.on, style_cards::glow_summary(&style.glow.value)),
            |ui, cenv, _, memory| style_cards::glow_card(ui, cenv, &style.glow.value, memory),
        );
    } else {
        hidden.push((TextLayerKind::Glow, "Glow"));
    }
    if !hidden.is_empty() {
        add_layer_row(ui, env, &hidden);
    }
}

/// "+ Add layer: Outline, Shadow, Glow" (ui-concept §7.1): one click
/// switches the layer on (one entry), and its card appears.
fn add_layer_row(ui: &mut egui::Ui, env: &mut Env<'_, '_>, hidden: &[(TextLayerKind, &str)]) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let mut picked = None;
    let top = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        top + 0.5,
        egui::Stroke::new(1.0, p.border_subtle),
    );
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(look.metrics.side_inset as i8, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let mut job = crate::widgets::icon_then_text(ui, icons::PLUS, "Add layer");
                for section in &mut job.sections {
                    section.format.color = p.text_secondary;
                }
                ui.label(job);
                for (layer, name) in hidden {
                    if ui
                        .small_button(*name)
                        .on_hover_text(format!("Add an {} layer", name.to_lowercase()))
                        .clicked()
                    {
                        picked = Some(*layer);
                    }
                }
            });
        });
    if let Some(layer) = picked {
        let mut steps = Steps::default();
        steps.edit(
            Edit::Style(TextStyleEdit::SetLayer { layer, on: true }),
            Merge::Own,
        );
        let result = props::commit(env.project, env.drag, &env.selection, steps, Held::Up);
        status(env, result);
    }
}

/// A layer card's chip: its look, or "off" (its settings kept).
fn layer_chip(on: bool, look: String) -> String {
    if on { look } else { format!("off · {look}") }
}

/// The Box & bubble lens of a text clip (TX-054): Placement, then the
/// Background, Bubble and Curve layers. Other visual clips get Placement
/// only (their Picture lens).
fn box_cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    picture_cards(ui, env);
    if env.kind != SelectionKind::Text {
        return;
    }
    let Some(data) = first_text(env) else {
        return;
    };
    let style = &data.style;
    text_card(
        ui,
        env,
        &data,
        CardId::Background,
        "Background",
        box_cards::background_summary(style),
        |ui, cenv, _, memory| box_cards::background_card(ui, cenv, style, memory),
    );
    text_card(
        ui,
        env,
        &data,
        CardId::Bubble,
        "Bubble",
        box_cards::bubble_summary(style),
        |ui, cenv, _, memory| box_cards::bubble_card(ui, cenv, style, memory),
    );
    text_card(
        ui,
        env,
        &data,
        CardId::Curve,
        "Curve",
        box_cards::curve_summary(style),
        |ui, cenv, _, _| box_cards::curve_card(ui, cenv, style),
    );
}

/// The text lenses' footer (TX-054): **Reset lens**, one entry. The
/// Content lens resets the paragraph only, never the text.
fn text_footer(ui: &mut egui::Ui, env: &mut Env<'_, '_>, lens: Lens) {
    let Some(scope) = lens.text_lens() else {
        return;
    };
    let tip = match scope {
        TextLens::Content => "Paragraph settings back to the default; the text stays",
        TextLens::Style => "Type, fill, spacing and the style layers back to the default",
        TextLens::BoxAndBubble => "Placement, background, bubble and curve back to the default",
    };
    let mut reset = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        reset = ui
            .button(crate::widgets::icon_then_text(
                ui,
                icons::RESET,
                "Reset lens",
            ))
            .on_hover_text(tip)
            .clicked();
    });
    if reset {
        let mut steps = Steps::default();
        let edits = match first_text(env) {
            Some(data) => reset_edits(env, &data, ResetScope::Lens(scope), scope.cards()),
            None => vec![Edit::Reset(ResetScope::Lens(scope))],
        };
        steps.group(edits);
        let result = props::commit(env.project, env.drag, &env.selection, steps, Held::Up);
        status(env, result);
    }
}

/// The Placement card of the first visual clip; edits go to all of them.
fn picture_cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let playhead = env.project.playhead;
    let Some(t) = env
        .selection
        .iter()
        .filter_map(|id| env.doc.clip(*id))
        .find(|c| c.content.is_visual())
        .map(|c| props::shown_transform(c, playhead))
    else {
        return;
    };
    let kind = env.kind;
    let selection = env.selection.clone();
    let everything = env.workspace.detail == Detail::Everything;
    let mut reset = false;
    let mut key_actions = KeyActions::default();
    let keys = card_keys(env, PropGroup::Transform);
    let memory = env.workspace.card(kind, CardId::Placement);
    let at = At::new(kind, CardId::Placement).with_keys(env.keys);
    let defaults = freecut_timeline::model::Transform::default();
    let placed = t.position != defaults.position
        || t.scale != defaults.scale
        || t.rotation != defaults.rotation
        || t.flip_h != defaults.flip_h
        || t.flip_v != defaults.flip_v;
    let shown = PropertyCard::new(widget_id(kind, CardId::Placement, "card"), "Placement")
        .summary(props::placement_summary(&t))
        .keyed(keys.is_some_and(|k| k.animated()))
        .reset(
            placed,
            "Position, scale, rotation and flips back to their defaults",
        )
        .menu(|ui| {
            if ui
                .button(crate::widgets::icon_then_text(
                    ui,
                    icons::RESET,
                    "Reset placement",
                ))
                .on_hover_text("Position, scale, rotation and flips back to their defaults")
                .clicked()
            {
                reset = true;
                ui.close();
            }
            if let Some(keys) = &keys {
                key_menu(ui, keys, &mut key_actions);
            }
        })
        .show(ui, memory, |ui, memory| {
            props::placement_card(
                ui,
                env.project,
                env.drag,
                &selection,
                &t,
                at,
                memory,
                everything,
            )
        });
    status(env, shown.inner.flatten());
    if reset || shown.reset {
        let result = props::reset_placement(env.project, env.drag, &selection);
        status(env, result);
    }
    if let Some(keys) = &keys {
        let result = key_steps(env, keys, &key_actions);
        status(env, result);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::model::ProjectSettings;

    use super::*;
    use crate::ui::drag::tests::{Frames, project_with_a_sound};

    fn fonts() -> FontChoices {
        FontChoices {
            families: vec!["Noto Sans".to_owned()].into(),
            scanning: false,
        }
    }

    /// Draws the inspector once per lens of the selection, every card open
    /// and Everything shown, and checks that nothing was edited.
    fn every_lens(project: &mut Project, state: &mut InspectorState) -> Vec<Lens> {
        let mut frames = Frames::new();
        let doc = project.doc().clone();
        let kind = selection_kind(&doc, &project.selection);
        let clips: Vec<&Clip> = project
            .selection
            .iter()
            .filter_map(|id| doc.clip(*id))
            .map(|c| c.as_ref())
            .collect();
        let shown = lenses(kind, Holds::of(&clips));
        state.workspace.detail = Detail::Everything;
        let entries = project.editor.history().len();
        for lens in &shown {
            state.workspace.set_lens(kind, *lens);
            for _ in 0..2 {
                frames.run(Vec::new(), |ui| {
                    let view = CanvasView {
                        canvas: (1920, 1080),
                        layers: &[],
                    };
                    let out = show(ui, project, state, &fonts(), &view);
                    assert_eq!(out.status, None, "{lens:?}");
                    assert!(!out.split && !out.delete && out.command.is_none());
                });
                assert_eq!(state.settle(Some(project), false), None);
            }
            assert_eq!(state.workspace.lens(kind, &shown), Some(*lens));
        }
        assert_eq!(
            project.editor.history().len(),
            entries,
            "drawing edits nothing"
        );
        shown
    }

    #[test]
    fn a_text_clip_shows_the_five_text_lenses_and_their_cards() {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let a = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        let mut state = InspectorState::default();
        let shown = every_lens(&mut project, &mut state);
        assert_eq!(shown, lens::TEXT_LENSES);
        // Collapse state belongs to (kind, card): a second text clip finds
        // the Type card as the first one left it.
        state.workspace.card(SelectionKind::Text, CardId::Type).open = false;
        project.playhead = Tick::from_secs(10).unwrap();
        let b = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        assert_ne!(a, b);
        assert_eq!(project.selection, [b]);
        assert!(!state.workspace.card(SelectionKind::Text, CardId::Type).open);
        every_lens(&mut project, &mut state);

        // Both selected: the shared lenses.
        project.selection = vec![a, b];
        assert_eq!(
            every_lens(&mut project, &mut state),
            [Lens::Content, Lens::Style, Lens::Picture, Lens::Sound]
        );
        // Nothing selected: the project.
        project.selection.clear();
        assert_eq!(every_lens(&mut project, &mut state), [Lens::Project]);
    }

    /// A project holding one 4 s video clip (a probe result, no file),
    /// selected, the playhead on it.
    fn project_with_a_video() -> (Project, ClipId) {
        use freecut_timeline::edit::{InsertTarget, NewMedia};
        use freecut_timeline::model::{MediaInfo, Rotation, StoredPath, VideoStreamInfo};
        use freecut_timeline::time::Rational;
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let four = Tick::from_secs(4).unwrap();
        let info = MediaInfo {
            container: "mp4".to_owned(),
            default_video: Some(0),
            duration: four,
            video: vec![VideoStreamInfo {
                index: 0,
                codec: "h264".to_owned(),
                width: 640,
                height: 360,
                sar: (1, 1),
                rotation: Rotation::None,
                mirrored: false,
                rate: Rational::new(30, 1).ok(),
                variable_rate: false,
                has_alpha: false,
                bit_depth: 8,
                start: Tick::ZERO,
                duration: four,
            }],
            audio: Vec::new(),
            still: None,
        };
        let file = crate::session::ProbedFile {
            path: "beach.mp4".into(),
            media: NewMedia {
                path: StoredPath {
                    absolute: "/beach.mp4".to_owned(),
                    relative: None,
                },
                info,
                fingerprint: None,
                display_name: None,
            },
        };
        let ids = project.import(vec![file]).unwrap();
        let main = project
            .doc()
            .sequence(project.doc().root())
            .unwrap()
            .main_track()
            .unwrap()
            .id;
        crate::ui::timeline::insert_media(
            &mut project,
            ids[0],
            InsertTarget::Track(main),
            Tick::ZERO,
        )
        .unwrap();
        let clip = project
            .doc()
            .sequence(project.doc().root())
            .unwrap()
            .main_track()
            .unwrap()
            .clips()[0]
            .id;
        project.selection = vec![clip];
        project.playhead = Tick::from_secs(1).unwrap();
        (project, clip)
    }

    fn stack_of(project: &Project, clip: ClipId) -> freecut_timeline::model::ColorStack {
        freecut_timeline::model::ColorStack::of(project.doc().clip(clip).unwrap().color.as_ref())
    }

    #[test]
    fn a_video_shows_the_look_lens_and_every_page_draws_without_editing() {
        use freecut_timeline::model::ColorPage;
        let (mut project, clip) = project_with_a_video();
        let mut state = InspectorState::default();
        // Something on every row: a graded, keyed clip.
        let seq = project.doc().root();
        let req = freecut_timeline::edit::EditRequest::EditColor {
            seq,
            clips: vec![clip],
            edit: freecut_timeline::edit::ColorEdit::SetParams(vec![
                (freecut_timeline::model::ColorParamId::Exposure, 20),
                (freecut_timeline::model::ColorParamId::Hue, -30),
                (freecut_timeline::model::ColorParamId::Amount, 70),
            ]),
        };
        project
            .apply(&req, freecut_timeline::history::Coalesce::Never)
            .unwrap();
        for page in ColorPage::ALL {
            state.workspace.set_look_page(SelectionKind::Video, page);
            let shown = every_lens(&mut project, &mut state);
            assert_eq!(
                shown,
                [Lens::Picture, Lens::Look, Lens::Motion, Lens::Sound]
            );
            assert_eq!(state.workspace.look_page(SelectionKind::Video), page);
        }
        // A video and a title together share the Look lens; the title is
        // left alone by its edits.
        let title = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        project.selection = vec![clip, title];
        assert_eq!(
            every_lens(&mut project, &mut state),
            [
                Lens::Content,
                Lens::Style,
                Lens::Picture,
                Lens::Look,
                Lens::Sound
            ]
        );
    }

    #[test]
    fn an_amount_drag_is_one_entry_and_switches_and_resets_are_one_each() {
        use eframe::egui::{Event, Key, Modifiers, Vec2};
        use freecut_timeline::model::{ColorParamId, PropPath, Section};

        use crate::ui::drag::tests::{button, key};

        let (mut project, clip) = project_with_a_video();
        let mut state = InspectorState::default();
        state.workspace.set_lens(SelectionKind::Video, Lens::Look);
        let mut frames = Frames::new();
        let mut frame = |frames: &mut Frames, project: &mut Project, events: Vec<Event>| {
            let mut status = None;
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                status = show(ui, project, &mut state, &fonts(), &view).status;
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            assert_eq!(state.settle(Some(project), down), None);
            status
        };
        frame(&mut frames, &mut project, Vec::new());
        frame(&mut frames, &mut project, Vec::new());

        // Drag the Amount rail from its right end leftward: one entry.
        let rail = widget_id(SelectionKind::Video, CardId::Grade, "amount").with("rail");
        let rect = frames
            .ctx
            .read_response(rail)
            .expect("the Amount rail")
            .rect;
        let start = egui::pos2(rect.right() - 4.0, rect.center().y);
        let entries = project.editor.history().len();
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(start), button(start, true)],
        );
        for step in 1..=5 {
            let to = start - Vec2::new(rect.width() * 0.08 * step as f32, 0.0);
            frame(&mut frames, &mut project, vec![Event::PointerMoved(to)]);
        }
        let end = start - Vec2::new(rect.width() * 0.4, 0.0);
        frame(&mut frames, &mut project, vec![button(end, false)]);
        frame(&mut frames, &mut project, Vec::new());
        let amount = stack_of(&project, clip).amount;
        assert!(amount < 80, "dragged to {amount}");
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert_eq!(stack_of(&project, clip).amount, 100, "one undo");
        project.redo().unwrap();

        // Esc during a drag: back to where it was, no entry.
        let entries = project.editor.history().len();
        let rect = frames.ctx.read_response(rail).unwrap().rect;
        let at = egui::pos2(rect.left() + rect.width() * 0.5, rect.center().y);
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at + Vec2::new(-30.0, 0.0))],
        );
        frame(
            &mut frames,
            &mut project,
            vec![key(Key::Escape, Modifiers::NONE)],
        );
        frame(&mut frames, &mut project, vec![button(at, false)]);
        frame(&mut frames, &mut project, Vec::new());
        assert_eq!(stack_of(&project, clip).amount, amount);
        assert_eq!(project.editor.history().len(), entries, "no entry");

        // The Light switch: one entry; the values stay.
        let seq = project.doc().root();
        project
            .apply(
                &freecut_timeline::edit::EditRequest::EditColor {
                    seq,
                    clips: vec![clip],
                    edit: freecut_timeline::edit::ColorEdit::SetParam(ColorParamId::Exposure, 30),
                },
                freecut_timeline::history::Coalesce::Never,
            )
            .unwrap();
        frame(&mut frames, &mut project, Vec::new());
        let switch = widget_id(SelectionKind::Video, CardId::Light, "card").with("switch");
        let at = frames
            .ctx
            .read_response(switch)
            .expect("the Light switch")
            .rect
            .center();
        let entries = project.editor.history().len();
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at), button(at, true), button(at, false)],
        );
        frame(&mut frames, &mut project, Vec::new());
        let s = stack_of(&project, clip);
        assert!(!s.sections_enabled.is_on(Section::Light));
        assert_eq!(s.basic.exposure, 30, "kept");
        assert_eq!(project.editor.history().len(), entries + 1);

        // A key on Exposure, then the Light reset badge: one entry that
        // removes the key and says so; undo brings it back.
        project
            .apply(
                &freecut_timeline::edit::EditRequest::EditKeys {
                    seq,
                    clips: vec![clip],
                    prop: PropPath::Color(ColorParamId::Exposure),
                    edit: KeyEdit::Add {
                        at: project.playhead,
                        value: None,
                    },
                },
                freecut_timeline::history::Coalesce::Never,
            )
            .unwrap();
        frame(&mut frames, &mut project, Vec::new());
        let badge = widget_id(SelectionKind::Video, CardId::Light, "card").with("reset");
        let at = frames
            .ctx
            .read_response(badge)
            .expect("the reset badge")
            .rect
            .center();
        let entries = project.editor.history().len();
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        let said = frame(&mut frames, &mut project, vec![button(at, false)]);
        frame(&mut frames, &mut project, Vec::new());
        assert_eq!(
            said.as_deref(),
            Some("Reset Light removed 1 keyframe. Undo brings them back")
        );
        let after = project.doc().clip(clip).unwrap().clone();
        let s = stack_of(&project, clip);
        assert!(s.section_is_default(Section::Light) && s.sections_enabled.is_on(Section::Light));
        assert!(after.keys.is_empty());
        assert_eq!(project.editor.history().len(), entries + 1);
        project.undo().unwrap();
        assert!(
            project
                .doc()
                .clip(clip)
                .unwrap()
                .keys
                .is_animated(PropPath::Color(ColorParamId::Exposure))
        );
    }

    #[test]
    fn wheel_drags_and_curve_clicks_are_one_entry_each() {
        use eframe::egui::{Event, Vec2};
        use freecut_timeline::model::{ColorPage, CurveChannel, PropPath, PropValue, WheelId};

        use crate::ui::drag::tests::button;

        let (mut project, clip) = project_with_a_video();
        let mut state = InspectorState::default();
        state.workspace.set_lens(SelectionKind::Video, Lens::Look);
        state
            .workspace
            .set_look_page(SelectionKind::Video, ColorPage::Wheels);
        let mut frames = Frames::new();
        let frame = |frames: &mut Frames,
                     project: &mut Project,
                     state: &mut InspectorState,
                     events: Vec<Event>| {
            let mut status = None;
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                status = show(ui, project, state, &fonts(), &view).status;
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            assert_eq!(state.settle(Some(project), down), None);
            status
        };
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(&mut frames, &mut project, &mut state, Vec::new());

        // Drag the Shadows puck from the centre to the right: one entry,
        // luma untouched.
        let wheel = widget_id(SelectionKind::Video, CardId::Wheels, "wheel").with("shadows");
        let rect = frames
            .ctx
            .read_response(wheel)
            .expect("the Shadows wheel")
            .rect;
        let start = rect.center();
        let entries = project.editor.history().len();
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![Event::PointerMoved(start), button(start, true)],
        );
        for step in 1..=4 {
            let to = start + Vec2::new(rect.width() * 0.08 * step as f32, 0.0);
            frame(
                &mut frames,
                &mut project,
                &mut state,
                vec![Event::PointerMoved(to)],
            );
        }
        let end = start + Vec2::new(rect.width() * 0.3, 0.0);
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![button(end, false)],
        );
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let (x, y) = stack_of(&project, clip).puck(WheelId::Shadows);
        assert!(x > 300 && y.abs() < 20, "puck at ({x}, {y})");
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert_eq!(stack_of(&project, clip).puck(WheelId::Shadows), (0, 0));
        project.redo().unwrap();

        // With the puck keyed, a drag sets the key at the playhead.
        let seq = project.doc().root();
        project
            .apply(
                &freecut_timeline::edit::EditRequest::EditKeys {
                    seq,
                    clips: vec![clip],
                    prop: PropPath::ColorPuck(WheelId::Shadows),
                    edit: KeyEdit::Add {
                        at: Tick::ZERO,
                        value: Some(PropValue::Puck { x: 0, y: 0 }),
                    },
                },
                freecut_timeline::history::Coalesce::Never,
            )
            .unwrap();
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let at = frames.ctx.read_response(wheel).unwrap().rect.center();
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        let to = at + Vec2::new(0.0, -rect.height() * 0.25);
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![Event::PointerMoved(to)],
        );
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![button(to, false)],
        );
        let keys = project
            .doc()
            .clip(clip)
            .unwrap()
            .keys
            .keys(PropPath::ColorPuck(WheelId::Shadows))
            .to_vec();
        assert_eq!(keys.len(), 2, "a key added at the playhead");
        let PropValue::Puck { y, .. } = keys[1].value else {
            panic!("a puck key");
        };
        assert!(y > 200, "dragged up: {y}");

        // Curves: a click above the diagonal adds a point (one entry); a
        // double-click on it removes it.
        state
            .workspace
            .set_look_page(SelectionKind::Video, ColorPage::Curves);
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let editor = widget_id(SelectionKind::Video, CardId::Curves, "curve_editor").with(0usize);
        let rect = frames
            .ctx
            .read_response(editor)
            .expect("the curve editor")
            .rect;
        let at = rect.center() + Vec2::new(0.0, -rect.height() * 0.15);
        let entries = project.editor.history().len();
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![Event::PointerMoved(at), button(at, true), button(at, false)],
        );
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let luma = stack_of(&project, clip)
            .curves
            .get(CurveChannel::Luma)
            .clone();
        assert_eq!(luma.points().len(), 3);
        assert!(luma.points()[1].y > 6_000, "{:?}", luma.points());
        assert_eq!(project.editor.history().len(), entries + 1);
        // Far enough in time from the first click to count as a new
        // double-click.
        for _ in 0..40 {
            frame(&mut frames, &mut project, &mut state, Vec::new());
        }
        let point = crate::widgets::curve_editor::to_screen(rect.shrink(6.5), luma.points()[1]);
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![
                Event::PointerMoved(point),
                button(point, true),
                button(point, false),
                button(point, true),
                button(point, false),
            ],
        );
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let luma = stack_of(&project, clip)
            .curves
            .get(CurveChannel::Luma)
            .clone();
        assert!(luma.is_default(), "{:?}", luma.points());
    }

    #[test]
    fn lut_picks_and_skin_switches_are_one_entry_each() {
        use eframe::egui::{Event, Key, Modifiers};
        use freecut_timeline::model::{ColorPage, LutInterp, SkinScope};

        use crate::ui::drag::tests::{button, key};

        let (mut project, clip) = project_with_a_video();
        let luts = crate::luts::AppLuts::new();
        project.luts = std::sync::Arc::new(luts.choices(&Default::default()));
        let first = project.luts.bundled[0].lut.clone();
        let mut state = InspectorState::default();
        state.workspace.set_lens(SelectionKind::Video, Lens::Look);
        state
            .workspace
            .set_look_page(SelectionKind::Video, ColorPage::Basic);
        // Light and Color folded, so the LUT section fits in the test
        // window.
        for card in [CardId::Light, CardId::ColorBalance] {
            state.workspace.card(SelectionKind::Video, card).open = false;
        }
        let mut frames = Frames::new();
        let frame = |frames: &mut Frames,
                     project: &mut Project,
                     state: &mut InspectorState,
                     events: Vec<Event>| {
            let mut out = InspectorOutput::default();
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                out = show(ui, project, state, &fonts(), &view);
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            assert_eq!(state.settle(Some(project), down), None);
            out
        };
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let click = |frames: &mut Frames,
                     project: &mut Project,
                     state: &mut InspectorState,
                     id: egui::Id| {
            let at = frames
                .ctx
                .read_response(id)
                .unwrap_or_else(|| panic!("{id:?} is shown"))
                .rect
                .center();
            frame(
                frames,
                project,
                state,
                vec![Event::PointerMoved(at), button(at, true)],
            );
            frame(frames, project, state, vec![button(at, false)]);
            frame(frames, project, state, Vec::new())
        };

        // The select opens on None; Down skips the Bundled heading to the
        // first bundled LUT; Enter picks it: one entry.
        let select = widget_id(SelectionKind::Video, CardId::Lut, "lut").with("select");
        let entries = project.editor.history().len();
        click(&mut frames, &mut project, &mut state, select);
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![key(Key::ArrowDown, Modifiers::NONE)],
        );
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![key(Key::Enter, Modifiers::NONE)],
        );
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let slot = stack_of(&project, clip).lut.expect("a LUT was picked");
        assert_eq!(slot.lut, first);
        assert_eq!(project.editor.history().len(), entries + 1);

        // Each Skin protection switch is one entry; the LUT's shows its
        // Amount once on.
        for (scope, row) in [(SkinScope::Lut, "lut_skin"), (SkinScope::Color, "skin")] {
            let card = match scope {
                SkinScope::Color => CardId::ColorBalance,
                SkinScope::Lut => CardId::Lut,
            };
            if scope == SkinScope::Color {
                // Color open, LUT folded.
                state.workspace.card(SelectionKind::Video, CardId::Lut).open = false;
                state.workspace.card(SelectionKind::Video, card).open = true;
                frame(&mut frames, &mut project, &mut state, Vec::new());
                frame(&mut frames, &mut project, &mut state, Vec::new());
            }
            let switch = widget_id(SelectionKind::Video, card, row).with("switch");
            let before = project.editor.history().len();
            click(&mut frames, &mut project, &mut state, switch);
            assert!(stack_of(&project, clip).skin(scope).on, "{scope:?}");
            assert_eq!(project.editor.history().len(), before + 1, "{scope:?}");
        }
        project.undo().unwrap();
        assert!(!stack_of(&project, clip).skin(SkinScope::Color).on);
        assert!(stack_of(&project, clip).skin(SkinScope::Lut).on);
        assert_eq!(
            stack_of(&project, clip).lut.unwrap().interp,
            LutInterp::Tetrahedral
        );

        // Import LUT… is the select's last entry: it asks the app.
        for (card, open) in [(CardId::ColorBalance, false), (CardId::Lut, true)] {
            state.workspace.card(SelectionKind::Video, card).open = open;
        }
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(&mut frames, &mut project, &mut state, Vec::new());
        click(&mut frames, &mut project, &mut state, select);
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![key(Key::End, Modifiers::NONE)],
        );
        let out = frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![key(Key::Enter, Modifiers::NONE)],
        );
        assert!(out.import_luts, "Import LUT… asks the app to import");
        assert_eq!(stack_of(&project, clip).lut.map(|l| l.lut), Some(first));
    }

    /// Applies one text style edit to `clip`.
    fn styled(project: &mut Project, clip: ClipId, edit: TextStyleEdit) {
        let req = freecut_timeline::edit::EditRequest::SetTextStyle {
            seq: project.doc().root(),
            clips: vec![clip],
            edit,
        };
        project
            .apply(&req, freecut_timeline::history::Coalesce::Never)
            .unwrap();
    }

    fn text_of(project: &Project, clip: ClipId) -> TextClipData {
        project
            .doc()
            .clip(clip)
            .unwrap()
            .content
            .text()
            .unwrap()
            .as_ref()
            .clone()
    }

    #[test]
    fn every_round_two_card_draws_without_editing() {
        use freecut_timeline::model::{FillKind, GlowDirection, SignedPerMille};

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        for edit in [
            TextStyleEdit::FillKind(FillKind::RadialGradient),
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Outline,
                on: true,
            },
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Shadow,
                on: true,
            },
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Glow,
                on: true,
            },
            TextStyleEdit::GlowDirection(GlowDirection::Both),
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Bubble,
                on: true,
            },
            TextStyleEdit::BubbleKind(freecut_timeline::model::BubbleKind::Shout),
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Curve,
                on: true,
            },
            TextStyleEdit::Curve(SignedPerMille::new(300)),
        ] {
            styled(&mut project, clip, edit);
        }
        let mut state = InspectorState::default();
        assert_eq!(every_lens(&mut project, &mut state), lens::TEXT_LENSES);
        // A Thought bubble and a linear gradient have their own rows.
        styled(
            &mut project,
            clip,
            TextStyleEdit::BubbleKind(freecut_timeline::model::BubbleKind::Thought),
        );
        styled(
            &mut project,
            clip,
            TextStyleEdit::FillKind(FillKind::LinearGradient),
        );
        every_lens(&mut project, &mut state);
    }

    /// The parts of the inspector a card header's actions need, for one
    /// selected text clip shown in a 300 × 80 px box.
    fn with_env<R>(
        project: &mut Project,
        clip: ClipId,
        f: impl FnOnce(&mut Env<'_, '_>) -> R,
    ) -> R {
        let mut state = InspectorState::default();
        let fonts = fonts();
        let track = project.doc().find_clip(clip).unwrap().track;
        let layers = [crate::preview::LayerBox {
            clip,
            track,
            to_canvas: freecut_timeline::glam::DMat3::IDENTITY,
            size: (300.0, 80.0),
            opacity: 1.0,
        }];
        let view = CanvasView {
            canvas: (1920, 1080),
            layers: &layers,
        };
        let mut out = InspectorOutput::default();
        let doc = project.doc().clone();
        let mut env = Env {
            project,
            drag: &mut state.drag,
            props: &mut state.props,
            workspace: &mut state.workspace,
            doc,
            kind: SelectionKind::Text,
            selection: vec![clip],
            fonts: &fonts,
            view: &view,
            out: &mut out,
            keys: None,
            motion: &mut state.motion,
        };
        f(&mut env)
    }

    #[test]
    fn card_reset_remove_and_switches_are_one_entry_each() {
        use freecut_timeline::model::{PerMille, Stroke};

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        styled(
            &mut project,
            clip,
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Outline,
                on: true,
            },
        );
        styled(
            &mut project,
            clip,
            TextStyleEdit::OutlineWidth(PerMille::new(200)),
        );
        let entries = project.editor.history().len();
        let data = text_of(&project, clip);
        let act = |project: &mut Project, card: CardId, actions: HeaderActions| {
            let data = text_of(project, clip);
            with_env(project, clip, |env| {
                header_steps(env, &data, card, &actions)
            })
        };

        // Reset card: the default outline, still on, one entry.
        let reset = HeaderActions {
            reset: true,
            ..HeaderActions::default()
        };
        assert_eq!(act(&mut project, CardId::Outline, reset), None);
        let after = text_of(&project, clip);
        assert!(after.style.stroke.on);
        assert_eq!(after.style.stroke.value, Stroke::default());
        assert_eq!(project.editor.history().len(), entries + 1);
        project.undo().unwrap();
        assert_eq!(text_of(&project, clip), data, "one undo");

        // Remove layer: off and forgotten, one entry.
        let remove = HeaderActions {
            remove: true,
            ..HeaderActions::default()
        };
        assert_eq!(act(&mut project, CardId::Outline, remove), None);
        assert_eq!(
            text_of(&project, clip).style.stroke,
            TextStyle::default().stroke
        );
        assert_eq!(project.editor.history().len(), entries + 1);
        project.undo().unwrap();
        assert_eq!(text_of(&project, clip), data);

        // Switching the bubble on points its tail by TX-055's rule on the
        // measured box (1 layout px is 1 ref px on 1080 p), in one entry.
        let on = HeaderActions {
            switched: Some(true),
            ..HeaderActions::default()
        };
        assert_eq!(act(&mut project, CardId::Bubble, on), None);
        let bubbled = text_of(&project, clip).style;
        assert!(bubbled.bubble.on);
        assert_eq!(
            bubbled.bubble.value.spec.tail.value.tip,
            stage::default_tail_tip((300.0, 80.0), &data.style)
        );
        assert_eq!(project.editor.history().len(), entries + 1);
        // The background takes over in one entry; the bubble keeps its tip.
        let on = HeaderActions {
            switched: Some(true),
            ..HeaderActions::default()
        };
        assert_eq!(act(&mut project, CardId::Background, on), None);
        let boxed = text_of(&project, clip).style;
        assert!(boxed.background.on && !boxed.bubble.on);
        assert_eq!(boxed.bubble.value, bubbled.bubble.value);
        project.undo().unwrap();
        project.undo().unwrap();
        assert_eq!(text_of(&project, clip), data);

        // Reset lens (the footer) on Style: one entry, the content stays.
        let result = with_env(&mut project, clip, |env| {
            let mut steps = Steps::default();
            steps.edit(Edit::Reset(ResetScope::Lens(TextLens::Style)), Merge::Own);
            props::commit(env.project, env.drag, &env.selection, steps, Held::Up)
        });
        assert_eq!(result, None);
        let reset = text_of(&project, clip);
        assert_eq!(reset.style.stroke.value, Stroke::default());
        assert_eq!(reset.content, data.content);
        assert_eq!(project.editor.history().len(), entries + 1);
    }

    /// TX-055 / the round-2 *Reset* note: resetting a bubble that stays on
    /// points its tail by the rule for the measured box (not the model's
    /// unmeasured default), in the same entry; so does Reset lens.
    #[test]
    fn a_bubble_reset_points_its_tail_at_the_measured_box() {
        use freecut_timeline::model::{PerMille, RefPx};

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        styled(
            &mut project,
            clip,
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Bubble,
                on: true,
            },
        );
        styled(
            &mut project,
            clip,
            TextStyleEdit::BubblePadX(PerMille::new(900)),
        );
        let moved = TailTip {
            x: RefPx::from_px(300),
            y: RefPx::from_px(-200),
        };
        styled(&mut project, clip, TextStyleEdit::BubbleTailTip(moved));
        let measured = stage::default_tail_tip((300.0, 80.0), &TextStyle::default());
        assert_ne!(
            measured,
            Tail::default().tip,
            "the rule differs from the model's default"
        );
        let tip = |p: &Project| text_of(p, clip).style.bubble.value.spec.tail.value.tip;

        for scope in [
            ResetScope::Card(TextCard::Bubble),
            ResetScope::Lens(TextLens::BoxAndBubble),
        ] {
            let entries = project.editor.history().len();
            let data = text_of(&project, clip);
            let result = with_env(&mut project, clip, |env| {
                let cards: &[TextCard] = match scope {
                    ResetScope::Card(card) => &[card],
                    ResetScope::Lens(lens) => lens.cards(),
                };
                let mut steps = Steps::default();
                steps.group(reset_edits(env, &data, scope, cards));
                props::commit(env.project, env.drag, &env.selection, steps, Held::Up)
            });
            assert_eq!(result, None);
            let after = text_of(&project, clip).style;
            assert!(after.bubble.on, "{scope:?}");
            assert_eq!(
                after.bubble.value.pad_x,
                TextStyle::default().bubble.value.pad_x
            );
            assert_eq!(tip(&project), measured, "{scope:?}");
            assert_eq!(project.editor.history().len(), entries + 1, "one entry");
            project.undo().unwrap();
            assert_eq!(tip(&project), moved, "one undo");
        }
        // A bubble that is off keeps the model's reset.
        styled(
            &mut project,
            clip,
            TextStyleEdit::SetLayer {
                layer: TextLayerKind::Bubble,
                on: false,
            },
        );
        let data = text_of(&project, clip);
        let edits = with_env(&mut project, clip, |env| {
            reset_edits(
                env,
                &data,
                ResetScope::Card(TextCard::Bubble),
                &[TextCard::Bubble],
            )
        });
        assert_eq!(edits.len(), 1);
    }

    /// UI-008: a number typed into a field and not committed yet when the
    /// selection changes (a canvas drag over another clip keeps the field's
    /// focus) goes to the clip it was typed for, never to the new one; while
    /// that drag runs, the field waits.
    #[test]
    fn typing_left_in_a_field_commits_to_its_own_clip_when_the_selection_changes() {
        use eframe::egui::{Event, Key, Modifiers, Pos2};
        use freecut_timeline::model::{ProjectSettings, RefPx};

        use crate::ui::drag::tests::{button, key};
        use crate::widgets::field::NumberField;

        for during_a_drag in [false, true] {
            let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
            let a = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
            project.playhead = Tick::from_secs(10).unwrap();
            let b = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
            project.selection = vec![a];
            let mut state = InspectorState::default();
            state.workspace.set_lens(SelectionKind::Text, Lens::Style);
            let mut frames = Frames::new();
            let mut frame = |frames: &mut Frames, project: &mut Project, events: Vec<Event>| {
                frames.run(events, |ui| {
                    let view = CanvasView {
                        canvas: (1920, 1080),
                        layers: &[],
                    };
                    let out = show(ui, project, &mut state, &fonts(), &view);
                    assert_eq!(out.status, None);
                });
                let down = frames.ctx.input(|i| i.pointer.any_down());
                assert_eq!(state.settle(Some(project), down), None);
            };
            let size = |p: &Project, c: ClipId| text_of(p, c).style.size;
            frame(&mut frames, &mut project, Vec::new());
            frame(&mut frames, &mut project, Vec::new());
            let key_id = widget_id(SelectionKind::Text, CardId::Type, "size").with("field");
            let field = NumberField::field_id(&frames.ctx, key_id).unwrap();
            let at: Pos2 = frames.ctx.read_response(field).unwrap().rect.center();
            frame(
                &mut frames,
                &mut project,
                vec![Event::PointerMoved(at), button(at, true)],
            );
            frame(&mut frames, &mut project, vec![button(at, false)]);
            frame(&mut frames, &mut project, Vec::new());
            frame(
                &mut frames,
                &mut project,
                vec![Event::Text("50".to_owned())],
            );
            let before = size(&project, a);
            assert_eq!(size(&project, b), before, "both at the default");

            // The selection moves to `b` while "50" waits in `a`'s field.
            project.selection = vec![b];
            let gesture = during_a_drag.then(|| project.begin_gesture());
            for _ in 0..3 {
                frame(&mut frames, &mut project, Vec::new());
            }
            if let Some(g) = gesture {
                assert_eq!(size(&project, a), before, "waits for the drag");
                project.end_gesture(g);
                for _ in 0..3 {
                    frame(&mut frames, &mut project, Vec::new());
                }
            }
            // A later Enter reaches nothing.
            frame(
                &mut frames,
                &mut project,
                vec![key(Key::Enter, Modifiers::NONE)],
            );
            frame(&mut frames, &mut project, Vec::new());
            assert_eq!(
                size(&project, a),
                RefPx::from_px(50),
                "drag {during_a_drag}"
            );
            assert_eq!(size(&project, b), before, "drag {during_a_drag}");
            assert_eq!(state.shown, [b], "the inspector moved on");
        }
    }

    /// A text clip with In, Loop and Out and some keys: every lens, the
    /// Motion lens included, draws without editing anything.
    fn animated_text(project: &mut Project) -> ClipId {
        use freecut_timeline::edit::{EditRequest, KeyEdit};
        use freecut_timeline::history::Coalesce;
        use freecut_timeline::model::{PresetId, Slot};

        let clip = crate::ui::text::add_text_at_playhead(project).unwrap();
        let seq = project.doc().root();
        let table = freecut_timeline::motion::PresetTable::samples();
        for (slot, id) in [
            (Slot::Entrance, "fc.in.typewriter"),
            (Slot::Continuous, "fc.loop.wave"),
            (Slot::Exit, "fc.out.fade"),
        ] {
            let choice = crate::anim::slot_presets(&table, slot, true)
                .into_iter()
                .find(|c| c.id == PresetId::new(id))
                .unwrap();
            let req = EditRequest::SetClipAnimation {
                seq,
                clips: vec![clip],
                slot,
                spec: Some(crate::anim::chosen_spec(&choice)),
            };
            project.apply(&req, Coalesce::Never).unwrap();
        }
        let req = EditRequest::EditKeys {
            seq,
            clips: vec![clip],
            prop: PropPath::Text(TextPropPath::Size),
            edit: KeyEdit::KeyCard {
                card: PropGroup::Text(TextCard::Type),
                at: project.playhead,
            },
        };
        project.apply(&req, Coalesce::Never).unwrap();
        clip
    }

    #[test]
    fn the_motion_lens_draws_an_animated_clip_without_editing() {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        animated_text(&mut project);
        let mut state = InspectorState::default();
        state.motion.presets =
            std::sync::Arc::new(freecut_timeline::motion::PresetTable::samples());
        for slot in freecut_timeline::model::Slot::ALL {
            state.motion.set_slot(slot);
            assert_eq!(every_lens(&mut project, &mut state), lens::TEXT_LENSES);
        }
        // Collapsed cards with keys show the diamond; the summary says so.
        let doc = project.doc().clone();
        let clip = doc.clip(project.selection[0]).unwrap();
        assert!(!clip.keys.is_empty());
    }

    /// FX-019 / FX-018: a video's Motion lens draws the Loop grid under its
    /// kind headings and the presets this build can't play as disabled
    /// tiles, in every slot, without editing anything.
    #[test]
    fn the_motion_lens_draws_grouped_and_unavailable_tiles() {
        use freecut_timeline::model::{PresetId, Slot};

        let (mut project, _) = crate::clip_streams::tests::project_with_streams(Vec::new());
        let why: std::sync::Arc<str> = std::sync::Arc::from("unknown channel \"glitch\"");
        let table = freecut_project::presets::bundled().table.with_unavailable([
            (
                PresetId::new("fc.clip.drift.spiral"),
                std::sync::Arc::clone(&why),
            ),
            (PresetId::new("fc.clip.in.glitch"), why),
        ]);
        let mut state = InspectorState::default();
        state.motion.presets = std::sync::Arc::new(table);
        for slot in Slot::ALL {
            state.motion.set_slot(slot);
            assert!(every_lens(&mut project, &mut state).contains(&Lens::Motion));
        }
        let grids = state.motion.grids();
        assert_eq!(grids.groups(Slot::Continuous, false).len(), 3);
        assert_eq!(grids.groups(Slot::Exit, false)[0].unavailable.len(), 1);
    }

    /// ui-concept §13.11 / TX-022: dragging the In join is one undo entry;
    /// `Esc` during a drag puts the length back with none.
    #[test]
    fn a_timing_strip_drag_is_one_entry_and_esc_cancels_it() {
        use eframe::egui::{Event, Key, Modifiers, Vec2};
        use freecut_timeline::model::Slot;

        use crate::ui::drag::tests::{button, key};

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = animated_text(&mut project);
        let mut state = InspectorState::default();
        state.motion.presets =
            std::sync::Arc::new(freecut_timeline::motion::PresetTable::samples());
        state.workspace.set_lens(SelectionKind::Text, Lens::Motion);
        let mut frames = Frames::new();
        let mut frame = |frames: &mut Frames, project: &mut Project, events: Vec<Event>| {
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                let out = show(ui, project, &mut state, &fonts(), &view);
                assert_eq!(out.status, None);
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            assert_eq!(state.settle(Some(project), down), None);
        };
        let in_len = |p: &Project| {
            p.doc()
                .clip(clip)
                .unwrap()
                .animations
                .get(Slot::Entrance)
                .unwrap()
                .duration
        };
        frame(&mut frames, &mut project, Vec::new());
        frame(&mut frames, &mut project, Vec::new());
        let join =
            widget_id(SelectionKind::Text, CardId::Animation, "strip").with(("join", 0usize));
        let at = frames
            .ctx
            .read_response(join)
            .expect("the In join")
            .rect
            .center();
        let entries = project.editor.history().len();
        let before = in_len(&project);

        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        for step in 1..=5 {
            let to = at + Vec2::new(12.0 * step as f32, 0.0);
            frame(&mut frames, &mut project, vec![Event::PointerMoved(to)]);
        }
        let end = at + Vec2::new(60.0, 0.0);
        frame(&mut frames, &mut project, vec![button(end, false)]);
        frame(&mut frames, &mut project, Vec::new());
        let dragged = in_len(&project);
        assert!(dragged > before, "{dragged:?} after {before:?}");
        let rate = project.doc().settings().frame_rate;
        assert!(rate.is_on_grid(dragged), "snapped to frames");
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert_eq!(in_len(&project), before, "one undo");
        project.redo().unwrap();

        // Esc during a drag: back to where it was, no entry.
        frame(&mut frames, &mut project, Vec::new());
        let at = frames.ctx.read_response(join).unwrap().rect.center();
        let entries = project.editor.history().len();
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        for step in 1..=3 {
            let to = at + Vec2::new(-10.0 * step as f32, 0.0);
            frame(&mut frames, &mut project, vec![Event::PointerMoved(to)]);
        }
        assert_ne!(in_len(&project), dragged, "dragging live");
        frame(
            &mut frames,
            &mut project,
            vec![key(Key::Escape, Modifiers::NONE)],
        );
        frame(&mut frames, &mut project, vec![button(at, false)]);
        frame(&mut frames, &mut project, Vec::new());
        assert_eq!(in_len(&project), dragged);
        assert_eq!(project.editor.history().len(), entries, "no entry");
        assert!(!project.gesture_open());
    }

    /// TX-022: when In and Out are shortened to fit the clip, their joins
    /// meet. Dragging the join there follows the pointer and the other
    /// segment stays where it is drawn: neither jumps to where the stored
    /// lengths would split the clip. One undo entry puts both back.
    #[test]
    fn a_join_drag_on_a_shortened_strip_follows_the_pointer() {
        use eframe::egui::{Event, Vec2};
        use freecut_timeline::edit::EditRequest;
        use freecut_timeline::history::Coalesce;
        use freecut_timeline::model::Slot;

        use crate::ui::drag::tests::button;

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = animated_text(&mut project);
        let dur = project.doc().clip(clip).unwrap().duration();
        // Without an Out, In as long as the clip; then a 1 s Out: In + Out
        // is longer than the clip.
        let seq = project.doc().root();
        let spec_of = |p: &Project, slot| {
            p.doc()
                .clip(clip)
                .unwrap()
                .animations
                .get(slot)
                .unwrap()
                .clone()
        };
        let (mut in_spec, mut out_spec) = (
            spec_of(&project, Slot::Entrance),
            spec_of(&project, Slot::Exit),
        );
        in_spec.duration = dur;
        in_spec.params.pace = None;
        out_spec.duration = Tick::from_secs(1).unwrap();
        out_spec.params.pace = None;
        for (slot, spec) in [
            (Slot::Exit, None),
            (Slot::Entrance, Some(in_spec)),
            (Slot::Exit, Some(out_spec)),
        ] {
            let req = EditRequest::SetClipAnimation {
                seq,
                clips: vec![clip],
                slot,
                spec,
            };
            project.apply(&req, Coalesce::Never).unwrap();
        }
        let layout = |p: &Project| crate::anim::strip_layout(p.doc().clip(clip).unwrap()).unwrap();
        let before = layout(&project);
        assert!(before.shortened, "{before:?}");

        let mut state = InspectorState::default();
        state.motion.presets =
            std::sync::Arc::new(freecut_timeline::motion::PresetTable::samples());
        state.workspace.set_lens(SelectionKind::Text, Lens::Motion);
        let mut frames = Frames::new();
        let mut frame = |frames: &mut Frames, project: &mut Project, events: Vec<Event>| {
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                let out = show(ui, project, &mut state, &fonts(), &view);
                assert_eq!(out.status, None);
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            assert_eq!(state.settle(Some(project), down), None);
        };
        let entries = project.editor.history().len();
        frame(&mut frames, &mut project, Vec::new());
        frame(&mut frames, &mut project, Vec::new());
        // Showing it edits nothing: the Length rows show the stored
        // lengths, longer than the clip leaves room for.
        assert_eq!(layout(&project), before, "drawing kept the lengths");
        assert_eq!(project.editor.history().len(), entries);
        let join =
            widget_id(SelectionKind::Text, CardId::Animation, "strip").with(("join", 2usize));
        let at = frames
            .ctx
            .read_response(join)
            .expect("the joins")
            .rect
            .center();
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        let frame_ticks = project.doc().settings().frame_rate.frame_ticks().raw();
        let mut last_out = before.out_len;
        for step in 1..=4 {
            let to = at + Vec2::new(10.0 * step as f32, 0.0);
            frame(&mut frames, &mut project, vec![Event::PointerMoved(to)]);
            let now = layout(&project);
            assert!(
                (now.in_len.raw() - before.in_len.raw()).abs() <= frame_ticks,
                "In stays where it was drawn: {:?} after {:?}",
                now.in_len,
                before.in_len
            );
            assert!(now.out_len < last_out, "Out follows the pointer: {now:?}");
            last_out = now.out_len;
        }
        let end = at + Vec2::new(40.0, 0.0);
        frame(&mut frames, &mut project, vec![button(end, false)]);
        frame(&mut frames, &mut project, Vec::new());
        assert!(!layout(&project).shortened, "the drag resolved it");
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert_eq!(layout(&project), before, "one undo puts both back");
    }

    /// TX-022: a tile chosen while Pieces is set moves those pieces, as
    /// the Pieces control keeps showing.
    #[test]
    fn a_tile_chosen_under_pieces_takes_that_unit() {
        use eframe::egui::Event;
        use freecut_timeline::model::{AnimUnit, PresetId, Slot};

        use crate::ui::drag::tests::button;

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        let mut state = InspectorState::default();
        state.motion.presets =
            std::sync::Arc::new(freecut_timeline::motion::PresetTable::samples());
        state.workspace.set_lens(SelectionKind::Text, Lens::Motion);
        state.motion.set_slot(Slot::Continuous);
        // Wave moves letters (its first unit) or words.
        state.motion.set_pieces(Slot::Continuous, AnimUnit::Word);
        let mut frames = Frames::new();
        let mut frame = |frames: &mut Frames, project: &mut Project, events: Vec<Event>| {
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                let out = show(ui, project, &mut state, &fonts(), &view);
                assert_eq!(out.status, None);
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            assert_eq!(state.settle(Some(project), down), None);
        };
        frame(&mut frames, &mut project, Vec::new());
        frame(&mut frames, &mut project, Vec::new());
        let wave = PresetId::new("fc.loop.wave");
        let tile = motion::tile_id(SelectionKind::Text, Slot::Continuous, &wave);
        let at = frames
            .ctx
            .read_response(tile)
            .expect("the Wave tile")
            .rect
            .center();
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(at), button(at, true)],
        );
        frame(&mut frames, &mut project, vec![button(at, false)]);
        frame(&mut frames, &mut project, Vec::new());
        let doc = project.doc();
        let spec = doc
            .clip(clip)
            .unwrap()
            .animations
            .get(Slot::Continuous)
            .unwrap();
        assert_eq!(spec.preset, wave);
        assert_eq!(spec.params.unit, Some(AnimUnit::Word));
    }

    /// UI-043: under Reduce motion a hovered tile shows its still, so
    /// resting the pointer on it doesn't keep the window repainting.
    #[test]
    fn a_hovered_tile_under_reduce_motion_asks_for_no_frames() {
        use eframe::egui::Event;
        use freecut_timeline::model::{PresetId, Slot};

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        let presets = std::sync::Arc::new(freecut_timeline::motion::PresetTable::samples());
        let mut state = InspectorState::default();
        state.motion.presets = std::sync::Arc::clone(&presets);
        state.motion.posters = Some(crate::posters::PosterWorker::new(
            freecut_render::FontRegistry::bundled(),
            presets,
            || {},
        ));
        state.motion.reduce_motion = true;
        state.workspace.set_lens(SelectionKind::Text, Lens::Motion);
        state.motion.set_slot(Slot::Entrance);
        let mut frames = Frames::new();
        let mut frame = |frames: &mut Frames, project: &mut Project, events: Vec<Event>| {
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                show(ui, project, &mut state, &fonts(), &view);
            })
        };
        frame(&mut frames, &mut project, Vec::new());
        frame(&mut frames, &mut project, Vec::new());
        let tile = motion::tile_id(
            SelectionKind::Text,
            Slot::Entrance,
            &PresetId::new("fc.in.fade"),
        );
        let at = frames
            .ctx
            .read_response(tile)
            .expect("the Fade tile")
            .rect
            .center();
        frame(&mut frames, &mut project, vec![Event::PointerMoved(at)]);
        // The hover's own fades and the tooltip's countdown settle first.
        let mut delays = Vec::new();
        for _ in 0..90 {
            delays.push(frame(&mut frames, &mut project, Vec::new()));
        }
        // No frame asks for the loop's next frame (12 fps). A poster still
        // arriving from the worker may ask for one repaint at once.
        let last = &delays[40..];
        let loop_rate = std::time::Duration::from_millis(100);
        assert!(
            last.iter().all(|d| d.is_zero() || *d > loop_rate),
            "{last:?}"
        );
        assert!(
            last.iter().filter(|d| **d > loop_rate).count() > 20,
            "{last:?}"
        );
    }

    /// TX-056: Typewriter's **Caret** is a switch of its In, stored with
    /// the slot in one undo entry; a Loop has none.
    #[test]
    fn the_caret_switch_stores_typewriters_caret_in_one_entry() {
        use freecut_timeline::model::Slot;

        use crate::ui::drag::tests::button;

        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = animated_text(&mut project);
        let mut state = InspectorState::default();
        state.motion.presets =
            std::sync::Arc::new(freecut_timeline::motion::PresetTable::samples());
        state.workspace.set_lens(SelectionKind::Text, Lens::Motion);
        state.motion.set_slot(Slot::Entrance);
        let mut frames = Frames::new();
        let frame = |frames: &mut Frames,
                     project: &mut Project,
                     state: &mut InspectorState,
                     events: Vec<eframe::egui::Event>| {
            frames.run(events, |ui| {
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                let out = show(ui, project, state, &fonts(), &view);
                assert_eq!(out.status, None);
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            assert_eq!(state.settle(Some(project), down), None);
        };
        let caret = |p: &Project| {
            p.doc()
                .clip(clip)
                .unwrap()
                .animations
                .get(Slot::Entrance)
                .unwrap()
                .params
                .caret
        };
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(&mut frames, &mut project, &mut state, Vec::new());
        let switch = widget_id(SelectionKind::Text, CardId::Animation, "caret").with("toggle");
        let at = frames
            .ctx
            .read_response(switch)
            .expect("the Caret switch")
            .rect
            .center();
        let entries = project.editor.history().len();
        assert!(!caret(&project), "off by default");
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![eframe::egui::Event::PointerMoved(at), button(at, true)],
        );
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![button(at, false)],
        );
        frame(&mut frames, &mut project, &mut state, Vec::new());
        assert!(caret(&project), "switched on");
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert!(!caret(&project), "one undo");
        project.redo().unwrap();
        assert!(caret(&project));

        // The Loop (Wave) has no caret.
        state.motion.set_slot(Slot::Continuous);
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(&mut frames, &mut project, &mut state, Vec::new());
        assert!(frames.ctx.read_response(switch).is_none());
    }

    #[test]
    fn an_audio_clip_shows_the_sound_lens() {
        let (mut project, _) = project_with_a_sound();
        let mut state = InspectorState::default();
        assert_eq!(every_lens(&mut project, &mut state), [Lens::Sound]);
    }
}

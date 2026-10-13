//! The inspector's cards for visual clips (TX-054, ui-concept §7):
//!
//! - **Content** lens: the Text card's content field (TX-002) with
//!   **Change case** for its selection (TX-008), and the Paragraph card
//!   (TX-005: alignment and justify, box width, vertical anchor).
//! - **Style** lens and **Box & bubble** lens's text layers: in
//!   `super::style_cards` and `super::box_cards`.
//! - **Box & bubble** / **Picture** lens: Placement (TR-001 … TR-003,
//!   TR-015: position, scale, rotation, opacity, flips behind *More*, and
//!   the card's reset).
//!
//! Each card collects what its widgets asked for as [`Steps`] and commits
//! them after drawing; a drag (slider, scrubbed label, field) is one undo
//! entry and `Esc` during it reverts with none (UI-009). With several clips
//! selected, edits apply to every selected clip that has the property (the
//! planner skips the rest), and the content field is hidden (TX-002). The
//! cards show the first clip's values; a row whose value differs across
//! the clips is *mixed* (UI-010): a dash, no lit segment, its reset arrow,
//! and any value picked or typed goes to all of them. An edit of one part
//! of a compound value (a radial centre's X, one gradient stop) is made
//! per clip from that clip's own value ([`Edit::Each`]), one entry in all.
//!
//! **Content edit sessions.** Typing never commits per keystroke. The field
//! edits a draft buffer ([`ContentSession`]) that the preview shows in
//! place of the stored content; the session commits as one
//! `SetTextContent` (one undo entry, none when nothing changed) when the
//! field loses focus, on Ctrl/Cmd+Enter or Esc, or when the clip stops
//! being shown (the selection changed). Undo inside the field walks the
//! field's own history first (TX-002).

use std::sync::Arc;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{self, Id, Key, Modifiers, Response};
use freecut_timeline::edit::{
    ColorEdit, EditRequest, KeyEdit, ResetScope, TextLayoutEdit, TextStyleEdit, TransformEdit,
    change_case,
};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::{ClipId, SeqId};
use freecut_timeline::model::{
    Angle, AnimationSpec, CanvasPos, CaseTransform, Clip, Document, FontRef, HAlign, PropPath,
    RefPx, RichText as Content, Scale, Slot, TextClipData, TextLayout, Transform, TransformProp,
    VAnchor,
};
use freecut_timeline::plan::ref_px_scale;
use freecut_timeline::time::Tick;

use super::canvas::CanvasView;
use super::inspector::lens::{CardId, SelectionKind, widget_id};
use super::rows::{self, At, KeyView, edit_prop, per_mille_in, ref_px, round_i32};
use crate::preview::TextDraft;
use crate::session::Project;
use crate::theme::icons;
use crate::ui::drag::{DragGesture, Held};
use crate::widgets::card::{self, CardMemory};
use crate::widgets::field::NumberField;
use crate::widgets::row::PropRow;
use crate::widgets::segmented::Segment;
use crate::widgets::toggle::toggle;

/// TX-002's soft limit on the length of a text clip.
pub const MAX_TEXT_CHARS: usize = 10_000;

/// The fixed box width a clip gets when its size isn't measured yet.
const DEFAULT_BOX_WIDTH: RefPx = RefPx::from_px(600);

/// The fonts the picker offers (TX-003).
#[derive(Debug, Clone)]
pub struct FontChoices {
    /// Every family the registry knows, bundled ones included.
    pub families: Arc<[String]>,
    /// The system font scan is still running: only bundled fonts so far.
    pub scanning: bool,
}

impl FontChoices {
    /// The bundled default family first, then the rest in registry order.
    pub fn ordered(&self) -> Vec<&str> {
        let default = FontRef::DEFAULT_FAMILY;
        std::iter::once(default)
            .chain(
                self.families
                    .iter()
                    .map(String::as_str)
                    .filter(|f| !f.eq_ignore_ascii_case(default)),
            )
            .collect()
    }

    pub(crate) fn has(&self, family: &str) -> bool {
        family.eq_ignore_ascii_case(FontRef::DEFAULT_FAMILY)
            || self.families.iter().any(|f| f.eq_ignore_ascii_case(family))
    }
}

/// Text as it is stored (TX-002): CRLF and lone CR become LF, tabs become
/// 4 spaces, and at most [`MAX_TEXT_CHARS`] characters are kept.
pub fn normalize_content(text: &str) -> String {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    unified
        .replace('\t', "    ")
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect()
}

/// One edit session of a text clip's content (TX-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSession {
    pub clip: ClipId,
    /// The stored content when the session began.
    pub original: String,
    /// What the field holds now.
    pub draft: String,
}

impl ContentSession {
    pub fn new(clip: ClipId, original: impl Into<String>) -> ContentSession {
        let original = original.into();
        ContentSession {
            clip,
            draft: original.clone(),
            original,
        }
    }

    /// The edit that commits this session: `None` when the session made no
    /// net change (no undo entry, TX-002).
    pub fn request(&self, seq: SeqId) -> Option<EditRequest> {
        let content = normalize_content(&self.draft);
        (content != self.original).then(|| EditRequest::SetTextContent {
            seq,
            clip: self.clip,
            content: Content::new(content),
        })
    }

    /// What the preview should show while the session is open: `None` while
    /// the draft equals the stored text.
    pub fn draft(&self) -> Option<TextDraft> {
        (self.draft != self.original).then(|| TextDraft {
            clip: self.clip,
            content: normalize_content(&self.draft),
        })
    }
}

/// The inspector's state for these sections.
#[derive(Debug, Default)]
pub struct PropsState {
    /// The content edit in progress.
    pub content: Option<ContentSession>,
    /// The content field was drawn this frame.
    content_shown: bool,
    /// Focus this clip's content field, with everything selected, when it
    /// is next drawn (TX-001 after an add, TX-002 double-click).
    focus_content: Option<ClipId>,
    /// The font picker's search text.
    pub(crate) font_search: String,
    /// The gradient stop the Fill card edits (TX-014).
    pub(crate) fill_stop: usize,
    /// The session's commit waits for a gesture to end.
    deferred: bool,
}

impl PropsState {
    /// Asks for `clip`'s content field to take focus with all its text
    /// selected, as soon as the inspector shows it.
    pub fn edit_content(&mut self, clip: ClipId) {
        self.focus_content = Some(clip);
    }

    pub fn text_draft(&self) -> Option<TextDraft> {
        self.content.as_ref().and_then(ContentSession::draft)
    }

    /// Typing in the content field that isn't committed yet: unsaved work,
    /// like an edit (closing or replacing the project asks first).
    pub fn has_uncommitted_text(&self) -> bool {
        self.text_draft().is_some()
    }

    /// A commit waits for a drag to end: the app keeps repainting so it
    /// lands on the frame after the drag, not on the next input event.
    pub fn commit_waiting(&self) -> bool {
        self.deferred && self.content.is_some()
    }

    /// Commits a session whose field went away this frame (selection
    /// changed, panel closed). Call once per frame after the panels; returns
    /// why the commit failed.
    pub fn settle(&mut self, project: Option<&mut Project>) -> Option<String> {
        // A focus request for a clip that is no longer selected is dropped;
        // one made this frame waits for the next one's inspector.
        if let Some(clip) = self.focus_content
            && !project
                .as_ref()
                .is_some_and(|p| p.selection.contains(&clip))
        {
            self.focus_content = None;
        }
        let shown = std::mem::take(&mut self.content_shown);
        if shown {
            return None;
        }
        match project {
            Some(p) => finish_content(p, self),
            None => {
                self.content = None;
                None
            }
        }
    }

    /// Forgets everything about a project that is no longer open.
    pub fn reset(&mut self) {
        self.content = None;
        self.content_shown = false;
        self.focus_content = None;
        self.deferred = false;
    }
}

/// Commits the open content session, if any. Returns why it failed.
///
/// While another gesture is open (a canvas or slider drag that began while
/// the field kept focus), the commit would be refused, so the session is
/// kept, still shown in the preview, and commits on a later call once the
/// gesture has ended. Any other refusal drops it with the reason.
pub fn finish_content(project: &mut Project, state: &mut PropsState) -> Option<String> {
    state.deferred = project.gesture_open();
    if state.deferred {
        return None;
    }
    let session = state.content.take()?;
    let req = session.request(project.doc().root())?;
    project.apply(&req, Coalesce::Never).err()
}

/// Whether `clip` sits on a locked track.
pub fn on_locked_track(doc: &Document, clip: ClipId) -> bool {
    doc.find_clip(clip)
        .and_then(|loc| doc.sequence(loc.seq)?.find_track(loc.track))
        .is_some_and(|(_, track)| track.is_locked())
}

/// The multi-line content field of one text clip (TX-002).
pub fn content_field(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut PropsState,
    clip: ClipId,
    stored: &str,
) -> Option<String> {
    let mut status = None;
    if state.content.as_ref().is_some_and(|s| s.clip != clip) {
        status = finish_content(project, state);
    }
    let locked = on_locked_track(project.doc(), clip);
    let id = content_id(clip);
    // A session of another clip is still here only while it waits for a
    // gesture to end: this field shows its own stored text meanwhile.
    let mut buffer = state
        .content
        .as_ref()
        .filter(|s| s.clip == clip)
        .map_or_else(|| stored.to_owned(), |s| s.draft.clone());
    let focused = ui.memory(|m| m.has_focus(id));
    let commit_key = focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
    // ui-concept §7.3: at least 64 pt tall, `bg.field`, `r.md`, 8 / 6 pt
    // padding, a 1 pt `accent` border while focused.
    let p = crate::theme::look(ui.ctx()).palette;
    let mut output = egui::TextEdit::multiline(&mut buffer)
        .id(id)
        .desired_rows(3)
        .desired_width(f32::INFINITY)
        .min_size(egui::vec2(0.0, 64.0))
        .char_limit(MAX_TEXT_CHARS)
        .hint_text("Type your text")
        .interactive(!locked)
        .frame(
            egui::Frame::new()
                .fill(p.bg_field)
                .corner_radius(crate::theme::R_MD)
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ui);
    let response = output.response.response.clone();
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            crate::theme::R_MD,
            egui::Stroke::new(1.0, p.accent),
            egui::StrokeKind::Inside,
        );
    } else {
        crate::widgets::control_outline(ui, response.rect, crate::theme::R_MD);
    }
    let focus = state.focus_content == Some(clip);
    if focus {
        state.focus_content = None;
    }
    if locked {
        response.clone().on_hover_text("The track is locked");
    } else if focus {
        response.request_focus();
        let all = CCursorRange::two(CCursor::new(0), CCursor::new(buffer.chars().count()));
        output.state.cursor.set_char_range(Some(all));
        output.state.store(ui.ctx(), id);
    }
    if (response.has_focus() || response.changed()) && state.content.is_none() {
        state.content = Some(ContentSession::new(clip, stored));
    }
    if response.changed()
        && let Some(session) = state.content.as_mut().filter(|s| s.clip == clip)
    {
        if buffer.chars().count() >= MAX_TEXT_CHARS {
            status = Some(format!(
                "A text clip holds up to {MAX_TEXT_CHARS} characters"
            ));
        }
        session.draft = buffer;
    }
    if commit_key {
        response.surrender_focus();
    }
    // Losing focus commits; so does a later frame without focus when the
    // commit had to wait for a gesture.
    if commit_key || !response.has_focus() {
        status = finish_content(project, state).or(status);
    } else {
        // Focused again: the session commits when the field lets go.
        state.deferred = false;
    }
    state.content_shown = true;
    status
}

/// The content field's id for `clip` (its edit state holds the selection).
fn content_id(clip: ClipId) -> Id {
    Id::new(("text-content", clip))
}

/// The byte range of `text` covering the characters `chars` (clamped to
/// the text).
pub(crate) fn byte_range(text: &str, chars: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let byte = |i: usize| text.char_indices().nth(i).map_or(text.len(), |(b, _)| b);
    let (start, end) = (byte(chars.start), byte(chars.end.max(chars.start)));
    start..end
}

/// **Change case** (TX-008): rewrites the content field's selected
/// characters in capitals, small letters, title or sentence case, as one
/// `SetTextContent` entry. Until on-canvas editing exists the selection is
/// the content field's (the round-2 scope). Disabled without a selection.
pub fn change_case_row(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut PropsState,
    clip: ClipId,
    kind: SelectionKind,
) -> Option<String> {
    let selection = egui::TextEdit::load_state(ui.ctx(), content_id(clip))
        .and_then(|s| s.cursor.char_range())
        .filter(|r| !r.is_empty())
        .map(|r| {
            let r = r.as_sorted_char_range();
            r.start.0..r.end.0
        });
    let locked = on_locked_track(project.doc(), clip);
    let cases = [
        (
            CaseTransform::Upper,
            "AA",
            "Rewrite the selection in capitals",
        ),
        (
            CaseTransform::Lower,
            "aa",
            "Rewrite the selection in small letters",
        ),
        (
            CaseTransform::Title,
            "Aa",
            "Capitalize each word of the selection",
        ),
        (
            CaseTransform::Sentence,
            "Sentence",
            "Capitalize each sentence of the selection",
        ),
    ];
    // Label and the four chips on one line when they fit beside the
    // gutter; otherwise the chips go under the label (Compact).
    let needed = {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let spacing = ui.spacing();
        let (pad, gap) = (spacing.button_padding.x, spacing.item_spacing.x);
        let look = crate::theme::look(ui.ctx());
        ui.fonts_mut(|f| {
            let mut w = |t: &str, font: &egui::FontId| {
                f.layout_no_wrap(t.to_owned(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
            };
            let chips: f32 = cases
                .iter()
                .map(|(_, label, _)| w(label, &font) + 2.0 * pad + gap)
                .sum();
            w("Change case", &font) + 8.0 + chips + look.metrics.row_gutter + 4.0
        })
    };
    let mut row = PropRow::new(widget_id(kind, CardId::Text, "change_case"), "Change case")
        .hint("Rewrites the characters selected in the text above");
    if needed > ui.available_width() {
        row = row.stacked();
    }
    let row = row.show(ui, |ui| {
        ui.add_enabled_ui(selection.is_some() && !locked, |ui| {
            let mut picked = None;
            for (case, label, tip) in cases {
                let response = ui
                    .small_button(label)
                    .on_hover_text(tip)
                    .on_disabled_hover_text("Select some text above first");
                if response.clicked() {
                    picked = Some(case);
                }
            }
            picked
        })
        .inner
    });
    let (Some(case), Some(chars)) = (row.inner, selection) else {
        return None;
    };
    // The field lost focus when the button was pressed, so its session has
    // committed; make sure before reading the stored text.
    if let Some(e) = finish_content(project, state) {
        return Some(e);
    }
    let doc = project.doc();
    let text = doc.clip(clip)?.content.text()?.content.as_str().to_owned();
    let req = change_case(doc, doc.root(), clip, byte_range(&text, chars), case, None)
        .map_err(|e| crate::session::describe_edit_error(&e));
    match req {
        Ok(req) => project.apply(&req, Coalesce::Never).err(),
        Err(e) => Some(e),
    }
}

/// An edit made by a widget of these cards this frame.
#[derive(Debug, Clone)]
pub(crate) enum Edit {
    Style(TextStyleEdit),
    Layout(TextLayoutEdit),
    Transform(TransformEdit),
    /// **Reset card** / **Reset lens** (TX-054).
    Reset(ResetScope),
    /// An edit of its own for each of these clips, one entry in all.
    Each(Vec<(ClipId, Edit)>),
    /// Keyframes of a property (TR-028 … TR-031, TX-021): the navigator's
    /// diamond, the Keyframes card, the cards' key commands.
    Keys {
        prop: PropPath,
        edit: KeyEdit,
    },
    /// The In, Out or Loop animation of the clips (FX-018, FX-019,
    /// TX-022): the Motion lens.
    Animation {
        slot: Slot,
        spec: Option<AnimationSpec>,
    },
    /// The colour stacks of the clips (CL-007, CL-008, CL-023, CL-024,
    /// CL-035): the Look lens.
    Color(ColorEdit),
}

/// The selected text clips and their data, for mixed values and per-clip
/// edits.
pub(crate) type Texts = [(ClipId, Arc<TextClipData>)];

/// Whether `f` differs across `texts` (a mixed value, UI-010).
pub(crate) fn differ<T: PartialEq>(texts: &Texts, f: impl Fn(&TextClipData) -> T) -> bool {
    let mut values = texts.iter().map(|(_, d)| f(d));
    values
        .next()
        .is_some_and(|first| values.any(|v| v != first))
}

/// A style edit made per clip from that clip's own style: `None` when no
/// clip has the property.
pub(crate) fn each_style(
    texts: &Texts,
    f: impl Fn(&freecut_timeline::model::TextStyle) -> Option<TextStyleEdit>,
) -> Option<Edit> {
    let mut edits: Vec<(ClipId, Edit)> = texts
        .iter()
        .filter_map(|(id, d)| f(&d.style).map(|e| (*id, Edit::Style(e))))
        .collect();
    match edits.len() {
        0 => None,
        1 => edits.pop().map(|(_, e)| e),
        _ => Some(Edit::Each(edits)),
    }
}

/// How the widget that made an edit wants it recorded.
pub(crate) enum Merge {
    /// Its own undo entry, unless it is a drag (sliders, scrubbed labels).
    Widget(Response),
    /// Merged while the pointer is held (the colour picker's square).
    WhileHeld,
    /// Its own undo entry (a reset arrow).
    Own,
}

/// What the cards' widgets asked for this frame, committed in order after
/// drawing.
pub(crate) enum Step {
    Edit(Edit, Merge),
    /// A widget without a change this frame: ends its drag on release.
    Finish(Response),
    /// `Esc` during a drag: revert it, with no undo entry (UI-009).
    Cancel,
    /// Edits that make one undo entry together (a layer switched off and
    /// reset, a bubble switched on with its tail placed, a face choice).
    Group(Vec<Edit>),
}

/// The steps of one card.
#[derive(Default)]
pub(crate) struct Steps(pub(crate) Vec<Step>);

impl Steps {
    pub(crate) fn edit(&mut self, edit: Edit, merge: Merge) {
        self.0.push(Step::Edit(edit, merge));
    }

    pub(crate) fn finish(&mut self, response: Response) {
        self.0.push(Step::Finish(response));
    }

    pub(crate) fn cancel(&mut self) {
        self.0.push(Step::Cancel);
    }

    /// `edits` as one undo entry; a single edit is an entry of its own.
    pub(crate) fn group(&mut self, mut edits: Vec<Edit>) {
        match edits.len() {
            0 => {}
            1 => {
                if let Some(edit) = edits.pop() {
                    self.edit(edit, Merge::Own);
                }
            }
            _ => self.0.push(Step::Group(edits)),
        }
    }

    /// A button-like widget: an edit of its own when `edit` is given.
    pub(crate) fn button(&mut self, response: Response, edit: Option<Edit>) {
        match edit {
            Some(edit) => self.edit(edit, Merge::Widget(response)),
            None => self.finish(response),
        }
    }
}

/// Commits a card's steps on `clips`. Drags merge into one undo entry
/// (TL-043); returns the last refusal.
pub(crate) fn commit(
    project: &mut Project,
    drag: &mut DragGesture,
    clips: &[ClipId],
    steps: Steps,
    held: Held,
) -> Option<String> {
    let mut status = None;
    for step in steps.0 {
        match step {
            Step::Cancel => {
                if let Err(e) = drag.cancel(project) {
                    status = Some(e);
                }
            }
            Step::Finish(response) => drag.finish(project, &response),
            Step::Group(edits) => {
                drag.end(project);
                let gesture = project.begin_gesture();
                for edit in edits {
                    if let Some(e) = apply(project, clips, edit, Coalesce::Gesture(gesture)) {
                        status = Some(e);
                    }
                }
                project.end_gesture(gesture);
            }
            Step::Edit(edit, merge) => {
                let coalesce = match &merge {
                    Merge::Widget(response) => drag.coalesce(project, response),
                    Merge::WhileHeld => drag.coalesce_held(project, held),
                    Merge::Own => {
                        drag.end(project);
                        Coalesce::Never
                    }
                };
                if let Some(e) = apply(project, clips, edit, coalesce) {
                    status = Some(e);
                }
                if let Merge::Widget(response) = &merge {
                    drag.finish(project, response);
                }
            }
        }
    }
    status
}

/// Applies `edit` to `clips`; returns why it was refused. [`Edit::Each`]
/// is one entry: its edits merge into `coalesce`'s gesture, or into one of
/// their own. Every edit of a keyable property goes through here, the
/// Stage's drags included, so TR-027's key-at-playhead rule holds.
pub(crate) fn apply(
    project: &mut Project,
    clips: &[ClipId],
    edit: Edit,
    coalesce: Coalesce,
) -> Option<String> {
    let seq = project.doc().root();
    // TR-027: editing a property that has keys sets the key at the
    // playhead (adding one there if needed); clips without keys on it get
    // the static value. One entry either way.
    if let Some((prop, Some(value))) = edit_prop(&edit)
        && matches!(
            edit,
            Edit::Style(_) | Edit::Layout(_) | Edit::Transform(_) | Edit::Color(_)
        )
    {
        let doc = project.doc();
        let (keyed, plain): (Vec<ClipId>, Vec<ClipId>) = clips
            .iter()
            .partition(|c| doc.clip(**c).is_some_and(|c| c.keys.is_animated(prop)));
        if !keyed.is_empty() {
            let key = Edit::Keys {
                prop,
                edit: KeyEdit::Add {
                    at: project.playhead,
                    value: Some(value),
                },
            };
            if plain.is_empty() {
                return apply(project, &keyed, key, coalesce);
            }
            let own = matches!(coalesce, Coalesce::Never).then(|| project.begin_gesture());
            let merge = own.map_or(coalesce, Coalesce::Gesture);
            let status = apply(project, &keyed, key, merge);
            let status = apply(project, &plain, edit, merge).or(status);
            if let Some(g) = own {
                project.end_gesture(g);
            }
            return status;
        }
    }
    let req = match edit {
        Edit::Style(edit) => EditRequest::SetTextStyle {
            seq,
            clips: clips.to_vec(),
            edit,
        },
        Edit::Layout(edit) => EditRequest::SetTextLayout {
            seq,
            clips: clips.to_vec(),
            edit,
        },
        Edit::Transform(edit) => EditRequest::SetTransform {
            seq,
            clips: clips.to_vec(),
            edit,
        },
        Edit::Reset(scope) => EditRequest::ResetText {
            seq,
            clips: clips.to_vec(),
            scope,
        },
        Edit::Keys { prop, edit } => EditRequest::EditKeys {
            seq,
            clips: clips.to_vec(),
            prop,
            edit,
        },
        Edit::Animation { slot, spec } => EditRequest::SetClipAnimation {
            seq,
            clips: clips.to_vec(),
            slot,
            spec,
        },
        Edit::Color(edit) => EditRequest::EditColor {
            seq,
            clips: clips.to_vec(),
            edit,
        },
        Edit::Each(edits) => {
            let own = matches!(coalesce, Coalesce::Never).then(|| project.begin_gesture());
            let merge = own.map_or(coalesce, Coalesce::Gesture);
            let mut status = None;
            for (clip, edit) in edits {
                if let Some(e) = apply(project, &[clip], edit, merge) {
                    status = Some(e);
                }
            }
            if let Some(g) = own {
                project.end_gesture(g);
            }
            return status;
        }
    };
    project.apply(&req, coalesce).err()
}

/// What every card body works on.
pub(crate) struct CardEnv<'a> {
    pub project: &'a mut Project,
    pub drag: &'a mut DragGesture,
    /// The selected clips; edits skip those without the property.
    pub clips: &'a [ClipId],
    /// The selected text clips, the first one's values shown.
    pub texts: &'a Texts,
    pub kind: SelectionKind,
    /// The Detail switch is on Everything: no *More* links.
    pub everything: bool,
    /// The one clip shown, for the rows' keyframe navigators; `None` with
    /// several clips.
    pub keys: Option<&'a KeyView>,
}

impl<'a> CardEnv<'a> {
    /// Where `card`'s rows sit, with keyframe navigators.
    pub fn at(&self, card: CardId) -> At<'a> {
        At::new(self.kind, card).with_keys(self.keys)
    }

    /// Whether `f` differs across the selected text clips (UI-010).
    pub fn differ<T: PartialEq>(&self, f: impl Fn(&TextClipData) -> T) -> bool {
        differ(self.texts, f)
    }

    /// Commits a card's steps; returns the last refusal.
    pub fn commit(&mut self, ui: &egui::Ui, steps: Steps) -> Option<String> {
        commit(self.project, self.drag, self.clips, steps, held(ui))
    }
}

pub(crate) fn held(ui: &egui::Ui) -> Held {
    Held::of(ui)
}

/// The Paragraph card (TX-005, TX-054): alignment (left, centre, right or
/// justified, one choice), box width; behind More, the vertical anchor.
/// While the text is curved the fixed width is paused (TX-016) and says so.
#[allow(clippy::too_many_arguments)]
pub fn paragraph_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    data: &TextClipData,
    view: &CanvasView<'_>,
    memory: &mut CardMemory,
) -> Option<String> {
    let layout = &data.layout;
    let mut steps = Steps::default();
    let defaults = TextLayout::default();
    let at = env.at(CardId::Paragraph);
    let items = [
        Segment::new(Some(icons::ALIGN_LEFT), "Left"),
        Segment::new(Some(icons::ALIGN_CENTER), "Center"),
        Segment::new(Some(icons::ALIGN_RIGHT), "Right"),
        Segment::new(Some(icons::ALIGN_JUSTIFY), "Justify"),
    ];
    let values = [
        Alignment::Align(HAlign::Left),
        Alignment::Align(HAlign::Center),
        Alignment::Align(HAlign::Right),
        Alignment::Justify,
    ];
    rows::choice(
        ui,
        at.mixed(env.differ(|d| Alignment::of(&d.layout))),
        &mut steps,
        "align",
        "Alignment",
        &items,
        &values,
        Alignment::of(layout),
        Alignment::of(&defaults),
        true,
        |a| match a {
            Alignment::Align(h) => Edit::Layout(TextLayoutEdit::Align(h)),
            Alignment::Justify => Edit::Layout(TextLayoutEdit::Justify(true)),
        },
    );

    let id = at.id("box");
    let curved = data.style.is_curved();
    let hint = if curved {
        "Paused while the text is curved: lines end only at line breaks"
    } else {
        "Wrap lines at a fixed width; otherwise lines end only at line breaks"
    };
    let width_mixed = env.differ(|d| d.layout.box_width);
    let width_row = PropRow::new(id, "Box width")
        .changed(width_mixed || layout.box_width != defaults.box_width)
        .hint(hint)
        .show(ui, |ui| {
            let mut fixed = layout.box_width.is_some();
            let toggle = toggle(ui, id.with("fixed"), &mut fixed, "Fixed width")
                .on_hover_text("Wrap lines at a fixed width; the Stage shows side handles");
            let p = crate::theme::look(ui.ctx()).palette;
            let note = |ui: &mut egui::Ui, text: &str| {
                ui.label(
                    egui::RichText::new(text)
                        .text_style(egui::TextStyle::Small)
                        .color(p.text_tertiary),
                );
            };
            let field = layout.box_width.map(|width| {
                let room = ui.available_width();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let key = id.with("field");
                    let field = ui
                        .push_id(key, |ui| {
                            NumberField::new(
                                key,
                                width.as_f64(),
                                TextLayout::MIN_BOX_WIDTH.as_f64()
                                    ..=TextLayout::MAX_BOX_WIDTH.as_f64(),
                            )
                            .decimals(1)
                            .unit("px")
                            .mixed(width_mixed)
                            .show(ui)
                        })
                        .inner;
                    if curved && room > 150.0 {
                        note(ui, "paused");
                    }
                    field
                })
                .inner
            });
            if field.is_none() {
                note(ui, "Auto");
            }
            (toggle, fixed, field)
        });
    if width_row.reset {
        steps.edit(Edit::Layout(TextLayoutEdit::BoxWidth(None)), Merge::Own);
    }
    let (toggle, fixed, field) = width_row.inner;
    let clips = env.clips;
    let switched = toggle.changed().then(|| {
        let width = fixed
            .then(|| measured_width(view, clips.first().copied()).unwrap_or(DEFAULT_BOX_WIDTH));
        Edit::Layout(TextLayoutEdit::BoxWidth(TextLayout::clamp_box_width(width)))
    });
    steps.button(toggle, switched);
    if let Some(field) = field {
        // UI-009: Esc during a drag of the field puts the width back, with
        // no undo entry (egui drops the drag on Esc: that frame reports it
        // stopped).
        let response = field.response;
        if (response.dragged() || response.drag_stopped())
            && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            ui.ctx().stop_dragging();
            steps.cancel();
        } else {
            let wanted = TextLayout::clamp_box_width(Some(ref_px(field.value)));
            let edit = (field.changed && (width_mixed || wanted != layout.box_width))
                .then_some(Edit::Layout(TextLayoutEdit::BoxWidth(wanted)));
            steps.button(response, edit);
        }
    }

    let everything = env.everything;
    let anchor_mixed = env.differ(|d| d.layout.v_anchor);
    card::more(ui, at.id("more"), memory, everything, 1, |ui| {
        let items = [
            Segment::new(Some(icons::ANCHOR_TOP), "Top"),
            Segment::new(Some(icons::ANCHOR_MIDDLE), "Middle"),
            Segment::new(Some(icons::ANCHOR_BOTTOM), "Bottom"),
        ];
        rows::choice(
            ui,
            at.mixed(anchor_mixed),
            &mut steps,
            "v_anchor",
            "Anchor",
            &items,
            &[VAnchor::Top, VAnchor::Middle, VAnchor::Bottom],
            layout.v_anchor,
            defaults.v_anchor,
            true,
            |v| Edit::Layout(TextLayoutEdit::VAnchor(v)),
        );
    });
    env.commit(ui, steps)
}

/// The Paragraph card's alignment control: the three alignments and
/// justify are one choice (TX-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alignment {
    Align(HAlign),
    Justify,
}

impl Alignment {
    fn of(layout: &TextLayout) -> Alignment {
        if layout.justify {
            Alignment::Justify
        } else {
            Alignment::Align(layout.align)
        }
    }
}

/// The Placement card (TR-001, TR-002's 100 % baseline, TR-003, TR-015's
/// opacity), applied to every selected visual clip. `t` is the first one's.
#[allow(clippy::too_many_arguments)]
pub fn placement_card(
    ui: &mut egui::Ui,
    project: &mut Project,
    drag: &mut DragGesture,
    clips: &[ClipId],
    t: &Transform,
    at: At<'_>,
    memory: &mut CardMemory,
    everything: bool,
) -> Option<String> {
    let kind = at.kind;
    let (w, h) = {
        let s = project.doc().settings();
        (s.width, s.height)
    };
    let defaults = Transform::default();
    let mut steps = Steps::default();

    let id = widget_id(kind, CardId::Placement, "position");
    let positions: Vec<(ClipId, CanvasPos)> = clips
        .iter()
        .filter_map(|c| {
            let clip = project.doc().clip(*c)?;
            Some((*c, shown_transform(clip, project.playhead).position))
        })
        .collect();
    let mixed =
        |axis: fn(&CanvasPos) -> i32| positions.iter().any(|(_, p)| axis(p) != axis(&t.position));
    let (x_mixed, y_mixed) = (mixed(|p| p.x), mixed(|p| p.y));
    let nav = at.nav(
        "Position",
        Some(PropPath::Transform(TransformProp::Position)),
    );
    let mut row =
        PropRow::new(id, "Position").changed(x_mixed || y_mixed || t.position != defaults.position);
    if let Some((_, n)) = nav {
        row = row.keys(n);
    }
    let position = row.show(ui, |ui| {
        let (px, py) = t.position.to_px(w, h);
        let limit = |side: u32| 4.0 * f64::from(side);
        // X and Y split the control's width equally (ui-concept §6.2).
        let half = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(48.0);
        let axis = |ui: &mut egui::Ui, key: Id, value: f64, side: u32, prefix, mixed| {
            ui.push_id(key, |ui| {
                NumberField::new(key, value, -limit(side)..=limit(side))
                    .decimals(1)
                    .prefix(prefix)
                    .unit("px")
                    .width(half)
                    .mixed(mixed)
                    .show(ui)
            })
            .inner
        };
        let fx = axis(ui, id.with("x"), px, w, "X", x_mixed);
        let fy = axis(ui, id.with("y"), py, h, "Y", y_mixed);
        let x = CanvasPos::from_px(fx.value, 0.0, w, h).x;
        let y = CanvasPos::from_px(0.0, fy.value, w, h).y;
        [(fx, Some(x), None, x_mixed), (fy, None, Some(y), y_mixed)]
    });
    if let Some((prop, _)) = nav {
        at.keys_clicked(prop, position.keys, &mut steps);
    }
    if position.reset {
        steps.edit(
            Edit::Transform(TransformEdit::Position(defaults.position)),
            Merge::Own,
        );
    }
    // One axis of each clip's own position: the other axis stays.
    for (field, x, y, mixed) in position.inner {
        let moved = |p: CanvasPos| CanvasPos::new(x.unwrap_or(p.x), y.unwrap_or(p.y));
        let edit = (field.changed && (mixed || moved(t.position) != t.position)).then(|| {
            Edit::Each(
                positions
                    .iter()
                    .map(|(c, p)| (*c, Edit::Transform(TransformEdit::Position(moved(*p)))))
                    .collect(),
            )
        });
        steps.button(field.response, edit);
    }

    let stored = t.scale;
    let n = rows::Numeric::new(
        "scale",
        "Scale",
        t.scale.factor() * 100.0,
        defaults.scale.factor() * 100.0,
        1.0..=1000.0,
    )
    .soft(10.0..=400.0)
    .logarithmic()
    .decimals(2)
    .speed(0.5)
    .suffix("%");
    rows::numeric(ui, at, n, &mut steps, |v| {
        let wanted = Scale::from_basis_points(round_i32(v * 100.0)).clamped();
        (wanted != stored).then_some(Edit::Transform(TransformEdit::Scale(wanted)))
    });

    let stored = t.rotation;
    let n = rows::Numeric::new(
        "rotation",
        "Rotation",
        t.rotation.degrees(),
        defaults.rotation.degrees(),
        -3600.0..=3600.0,
    )
    .decimals(2)
    .speed(0.5)
    .suffix("°");
    rows::numeric(ui, at, n, &mut steps, |v| {
        let wanted = Angle::from_hundredths(round_i32(v * 100.0)).clamped();
        (wanted != stored).then_some(Edit::Transform(TransformEdit::Rotation(wanted)))
    });

    let _ = opacity_row(ui, at, t, &mut steps);

    card::more(
        ui,
        widget_id(kind, CardId::Placement, "more"),
        memory,
        everything,
        1,
        |ui| {
            let flip =
                PropRow::new(widget_id(kind, CardId::Placement, "flip"), "Flip").show(ui, |ui| {
                    let label = crate::widgets::icon_then_text(ui, icons::FLIP_H, "Horizontal");
                    let fh = ui
                        .add(egui::Button::new(label).selected(t.flip_h))
                        .on_hover_text("Mirror left to right");
                    let label = crate::widgets::icon_then_text(ui, icons::FLIP_V, "Vertical");
                    let fv = ui
                        .add(egui::Button::new(label).selected(t.flip_v))
                        .on_hover_text("Mirror top to bottom");
                    (fh, fv)
                });
            let (fh, fv) = flip.inner;
            let edit = fh
                .clicked()
                .then_some(Edit::Transform(TransformEdit::FlipH(!t.flip_h)));
            steps.button(fh, edit);
            let edit = fv
                .clicked()
                .then_some(Edit::Transform(TransformEdit::FlipV(!t.flip_v)));
            steps.button(fv, edit);
        },
    );
    commit(project, drag, clips, steps, held(ui))
}

/// `clip`'s transform as shown at the playhead: keyed values evaluated
/// (TR-027), the static values elsewhere.
pub(crate) fn shown_transform(clip: &Clip, playhead: Tick) -> Transform {
    match clip.content_time_at(playhead) {
        Some(ct) if !clip.keys.is_empty() => clip.transform_at(ct),
        _ => clip.transform,
    }
}

/// `clip`'s text as shown at the playhead: keyed style values evaluated
/// (TX-021). `None` for other clips.
pub(crate) fn shown_text(clip: &Clip, playhead: Tick) -> Option<Arc<TextClipData>> {
    match clip.content_time_at(playhead) {
        Some(ct) if !clip.keys.is_empty() => clip.text_at(ct),
        _ => clip.content.text().cloned(),
    }
}

/// The Opacity row (TR-015): 0–100 %, the headline row of its section
/// (label above, rail and field). Returns the rail's track.
pub(crate) fn opacity_row(
    ui: &mut egui::Ui,
    at: At<'_>,
    t: &Transform,
    steps: &mut Steps,
) -> egui::Rect {
    let stored = t.opacity;
    let default = Transform::default().opacity;
    let n = rows::Numeric::new(
        "opacity",
        "Opacity",
        stored.as_f64() * 100.0,
        default.as_f64() * 100.0,
        0.0..=100.0,
    )
    .suffix("%")
    .headline();
    let at = At::new(at.kind, CardId::Placement).with_keys(at.keys);
    rows::numeric(ui, at, n, steps, |v| {
        let wanted = per_mille_in(v * 10.0, 0, 1000);
        (wanted != stored).then_some(Edit::Transform(TransformEdit::Opacity(wanted)))
    })
    .unwrap_or(egui::Rect::NOTHING)
}

/// The Placement card's reset (TR-001): position, scale, rotation and
/// flips back to their defaults, and the keys of those properties
/// removed, as one entry; opacity and its keys stay. Returns the refusal,
/// or the toast naming the keys removed (TR-027).
pub fn reset_placement(
    project: &mut Project,
    drag: &mut DragGesture,
    clips: &[ClipId],
) -> Option<String> {
    const RESET: [TransformProp; 3] = [
        TransformProp::Position,
        TransformProp::Scale,
        TransformProp::Rotation,
    ];
    let doc = project.doc();
    let mut removals = Vec::new();
    let mut removed = Vec::new();
    for p in RESET {
        let prop = PropPath::Transform(p);
        let before = removals.len();
        for c in clips.iter().filter_map(|id| doc.clip(*id)) {
            removals.extend(
                c.keys
                    .keys(prop)
                    .iter()
                    .filter_map(|k| c.sequence_time_of(k.at))
                    .map(|at| {
                        let edit = KeyEdit::Remove { at };
                        (c.id, Edit::Keys { prop, edit })
                    }),
            );
        }
        let n = removals.len() - before;
        if n > 0 {
            removed.push(format!("{} {n}", crate::prop_keys::prop_label(prop)));
        }
    }
    let total = removals.len();
    let mut steps = Steps::default();
    let reset = Edit::Transform(TransformEdit::Reset);
    if removals.is_empty() {
        steps.edit(reset, Merge::Own);
    } else {
        // Keys go first: removing a property's last key writes its value
        // back as the static value, which the reset then replaces.
        steps.group(vec![Edit::Each(removals), reset]);
    }
    let refused = commit(project, drag, clips, steps, Held::Up);
    if refused.is_some() || total == 0 {
        return refused;
    }
    let s = if total == 1 { "" } else { "s" };
    Some(format!(
        "Reset placement removed {total} keyframe{s} ({}). Undo brings them back",
        removed.join(", ")
    ))
}

/// "Center · auto width", "Justified · 600 px · top".
pub fn paragraph_summary(layout: &TextLayout) -> String {
    let align = if layout.justify {
        "Justified"
    } else {
        match layout.align {
            HAlign::Left => "Left",
            HAlign::Center => "Center",
            HAlign::Right => "Right",
        }
    };
    let mut s = match layout.box_width {
        Some(w) => format!("{align} · {:.0} px", w.as_f64()),
        None => format!("{align} · auto width"),
    };
    match layout.v_anchor {
        VAnchor::Top => s.push_str(" · top"),
        VAnchor::Middle => {}
        VAnchor::Bottom => s.push_str(" · bottom"),
    }
    s
}

/// "100 % · 0° · opacity 100 %".
pub fn placement_summary(t: &Transform) -> String {
    format!(
        "{:.0} % · {:.0}° · opacity {:.0} %",
        t.scale.factor() * 100.0,
        t.rotation.degrees(),
        t.opacity.as_f64() * 100.0
    )
}

/// The measured width of `clip`'s text box in the latest preview frame, in
/// ref px (for switching to a fixed width without a jump).
fn measured_width(view: &CanvasView<'_>, clip: Option<ClipId>) -> Option<RefPx> {
    let layer = view.layer(clip?)?;
    let scale = ref_px_scale(view.canvas);
    (scale > 0.0 && layer.size.0 > 0.0).then(|| ref_px(layer.size.0 / scale))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::num::NonZeroU64;

    use freecut_timeline::model::{PerMille, TextClipData};

    use super::*;

    fn clip() -> ClipId {
        ClipId::from_raw(NonZeroU64::new(7).unwrap())
    }

    fn seq() -> SeqId {
        SeqId::from_raw(NonZeroU64::new(1).unwrap())
    }

    #[test]
    fn a_session_without_a_net_change_commits_nothing() {
        let mut session = ContentSession::new(clip(), "Your text");
        assert_eq!(session.request(seq()), None);
        assert_eq!(session.draft(), None);
        session.draft = "Your tex".to_owned();
        session.draft = "Your text".to_owned();
        assert_eq!(session.request(seq()), None, "typed and deleted again");
        assert_eq!(session.draft(), None);
    }

    #[test]
    fn a_session_commits_its_whole_draft_once() {
        let mut session = ContentSession::new(clip(), "Your text");
        session.draft = "Hello\r\nworld".to_owned();
        assert_eq!(
            session.request(seq()),
            Some(EditRequest::SetTextContent {
                seq: seq(),
                clip: clip(),
                content: Content::new("Hello\nworld"),
            })
        );
        assert_eq!(
            session.draft(),
            Some(TextDraft {
                clip: clip(),
                content: "Hello\nworld".to_owned(),
            })
        );
        // Emptying the text is a real edit (TX-002 allows empty clips).
        session.draft.clear();
        assert!(session.request(seq()).is_some());
    }

    #[test]
    fn content_is_normalized_and_limited() {
        assert_eq!(normalize_content("a\r\nb\rc\td"), "a\nb\nc    d");
        let long = "x".repeat(MAX_TEXT_CHARS + 5);
        assert_eq!(normalize_content(&long).chars().count(), MAX_TEXT_CHARS);
        // Kept exactly as typed otherwise (no Unicode normalization).
        assert_eq!(normalize_content("e\u{301} 👋🏽"), "e\u{301} 👋🏽");
    }

    /// A project with one default text clip, selected.
    fn project_with_text() -> (Project, ClipId) {
        let mut project =
            Project::new(freecut_timeline::model::ProjectSettings::new_default().unwrap()).unwrap();
        let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        (project, clip)
    }

    fn stored(project: &Project, clip: ClipId) -> String {
        project
            .doc()
            .clip(clip)
            .unwrap()
            .content
            .text()
            .unwrap()
            .content
            .as_str()
            .to_owned()
    }

    #[test]
    fn typing_in_the_content_field_is_one_undo_entry_on_commit() {
        use eframe::egui::Event;

        use crate::ui::drag::tests::{Frames, key};

        let (mut project, clip) = project_with_text();
        let mut state = PropsState::default();
        let mut frames = Frames::new();
        let frame = |frames: &mut Frames,
                     project: &mut Project,
                     state: &mut PropsState,
                     events: Vec<Event>| {
            frames.run(events, |ui| {
                let text = stored(project, clip);
                assert_eq!(content_field(ui, project, state, clip, &text), None);
            });
            assert_eq!(state.settle(Some(project)), None);
        };
        let steps = project.editor.history().len();

        // Focused with everything selected (TX-001), so typing replaces it.
        state.edit_content(clip);
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(&mut frames, &mut project, &mut state, Vec::new());
        for typed in ["Hel", "lo"] {
            frame(
                &mut frames,
                &mut project,
                &mut state,
                vec![Event::Text(typed.to_owned())],
            );
        }
        assert_eq!(stored(&project, clip), TextClipData::PLACEHOLDER, "not yet");
        assert_eq!(project.editor.history().len(), steps);
        assert_eq!(
            state.text_draft().map(|d| d.content),
            Some("Hello".to_owned()),
            "the preview shows the draft"
        );

        // Esc commits the session (TX-002).
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![key(Key::Escape, Modifiers::NONE)],
        );
        assert_eq!(stored(&project, clip), "Hello");
        assert_eq!(project.editor.history().len(), steps + 1);
        assert_eq!(state.text_draft(), None);
        project.undo().unwrap();
        assert_eq!(stored(&project, clip), TextClipData::PLACEHOLDER);
        project.redo().unwrap();

        // A session whose field goes away (the selection changed) commits.
        state.edit_content(clip);
        frame(&mut frames, &mut project, &mut state, Vec::new());
        frame(
            &mut frames,
            &mut project,
            &mut state,
            vec![Event::Text("Bye".to_owned())],
        );
        frames.run(Vec::new(), |_| {});
        assert_eq!(state.settle(Some(&mut project)), None);
        assert_eq!(stored(&project, clip), "Bye");
        assert_eq!(project.editor.history().len(), steps + 2);
    }

    #[test]
    fn typing_survives_a_drag_that_began_while_the_field_had_focus() {
        use eframe::egui::Event;

        use crate::ui::drag::tests::Frames;

        let (mut project, a) = project_with_text();
        project.playhead = freecut_timeline::time::Tick::from_secs(10).unwrap();
        let b = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        let mut state = PropsState::default();
        let mut frames = Frames::new();
        let field = |frames: &mut Frames,
                     project: &mut Project,
                     state: &mut PropsState,
                     clip: ClipId,
                     events: Vec<Event>| {
            frames.run(events, |ui| {
                let text = stored(project, clip);
                assert_eq!(content_field(ui, project, state, clip, &text), None);
            });
            assert_eq!(state.settle(Some(project)), None);
        };

        state.edit_content(a);
        project.selection = vec![a];
        field(&mut frames, &mut project, &mut state, a, Vec::new());
        field(&mut frames, &mut project, &mut state, a, Vec::new());
        field(
            &mut frames,
            &mut project,
            &mut state,
            a,
            vec![Event::Text("Hello".to_owned())],
        );

        // A press-drag on the canvas keeps the field's focus, selects `b`
        // and opens a gesture: the inspector now shows `b`'s field.
        project.selection = vec![b];
        let gesture = project.begin_gesture();
        field(&mut frames, &mut project, &mut state, b, Vec::new());
        frames.run(Vec::new(), |_| {});
        assert_eq!(state.settle(Some(&mut project)), None, "nothing refused");
        assert_eq!(
            state.text_draft().map(|d| (d.clip, d.content)),
            Some((a, "Hello".to_owned())),
            "the draft waits for the drag"
        );
        assert_eq!(stored(&project, b), TextClipData::PLACEHOLDER);
        assert!(state.commit_waiting());

        // The drag ends: the typing commits.
        project.end_gesture(gesture);
        field(&mut frames, &mut project, &mut state, b, Vec::new());
        assert_eq!(stored(&project, a), "Hello");
        assert_eq!(stored(&project, b), TextClipData::PLACEHOLDER);
        assert_eq!(state.text_draft(), None);
        assert!(!state.commit_waiting());
    }

    /// UI-009: Esc during a drag of the Box width field puts the width back
    /// with no undo entry, like every other numeric field.
    #[test]
    fn esc_during_a_box_width_drag_reverts_it_without_an_entry() {
        use eframe::egui::{Event, Pos2, Vec2};

        use crate::ui::drag::tests::{Frames, button, key};

        let (mut project, clip) = project_with_text();
        let req = EditRequest::SetTextLayout {
            seq: project.doc().root(),
            clips: vec![clip],
            edit: TextLayoutEdit::BoxWidth(Some(RefPx::from_px(600))),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        project.selection = vec![clip];
        let width = |p: &Project| {
            p.doc()
                .clip(clip)
                .unwrap()
                .content
                .text()
                .unwrap()
                .layout
                .box_width
        };
        let mut drag = DragGesture::default();
        let mut memory = CardMemory::new(true);
        let mut frames = Frames::new();
        let mut frame = |frames: &mut Frames, project: &mut Project, events: Vec<Event>| {
            let data = project
                .doc()
                .clip(clip)
                .unwrap()
                .content
                .text()
                .unwrap()
                .clone();
            let texts = [(clip, data.clone())];
            frames.run(events, |ui| {
                let mut env = CardEnv {
                    project: &mut *project,
                    drag: &mut drag,
                    clips: &[clip],
                    texts: &texts,
                    kind: SelectionKind::Text,
                    everything: true,
                    keys: None,
                };
                let view = CanvasView {
                    canvas: (1920, 1080),
                    layers: &[],
                };
                assert_eq!(
                    paragraph_card(ui, &mut env, &data, &view, &mut memory),
                    None
                );
            });
            let down = frames.ctx.input(|i| i.pointer.any_down());
            drag.settle(Some(project), down);
        };
        frame(&mut frames, &mut project, Vec::new());
        let entries = project.editor.history().len();
        let key_id = At::new(SelectionKind::Text, CardId::Paragraph)
            .id("box")
            .with("field");
        let field = NumberField::field_id(&frames.ctx, key_id).unwrap();
        let from: Pos2 = frames.ctx.read_response(field).unwrap().rect.center();
        frame(
            &mut frames,
            &mut project,
            vec![Event::PointerMoved(from), button(from, true)],
        );
        for step in 1..=4 {
            let at = from + Vec2::new(10.0 * step as f32, 0.0);
            frame(&mut frames, &mut project, vec![Event::PointerMoved(at)]);
        }
        assert_ne!(width(&project), Some(RefPx::from_px(600)), "dragging live");
        frame(
            &mut frames,
            &mut project,
            vec![key(Key::Escape, Modifiers::NONE)],
        );
        frame(&mut frames, &mut project, vec![button(from, false)]);
        assert_eq!(width(&project), Some(RefPx::from_px(600)));
        assert_eq!(project.editor.history().len(), entries, "no entry");
        assert!(!project.editor.gesture_open());
    }

    fn opacity(project: &Project, clip: ClipId) -> PerMille {
        project.doc().clip(clip).unwrap().transform.opacity
    }

    /// One frame of the Placement card's Opacity row, committed like the
    /// card does. Returns the rail's track.
    fn opacity_frame(
        frames: &mut crate::ui::drag::tests::Frames,
        project: &mut Project,
        drag: &mut DragGesture,
        clip: ClipId,
        events: Vec<egui::Event>,
    ) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        frames.run(events, |ui| {
            let t = project.doc().clip(clip).unwrap().transform;
            let mut steps = Steps::default();
            rect = opacity_row(
                ui,
                At::new(SelectionKind::Text, CardId::Placement),
                &t,
                &mut steps,
            );
            assert_eq!(commit(project, drag, &[clip], steps, Held::Up), None);
        });
        let down = frames.ctx.input(|i| i.pointer.any_down());
        drag.settle(Some(project), down);
        rect
    }

    /// Where `fraction` of the rail is: `rect` is the rail's track.
    fn on_rail(rect: egui::Rect, fraction: f32) -> egui::Pos2 {
        egui::pos2(rect.left() + rect.width() * fraction, rect.center().y)
    }

    #[test]
    fn a_slider_drag_is_one_undo_entry_and_esc_cancels_one() {
        use eframe::egui::Event;

        use crate::ui::drag::tests::{Frames, button, key};

        let (mut project, clip) = project_with_text();
        let mut drag = DragGesture::default();
        let mut frames = Frames::new();
        let rect = opacity_frame(&mut frames, &mut project, &mut drag, clip, Vec::new());
        let entries = project.editor.history().len();
        assert_eq!(opacity(&project, clip), PerMille::ONE);

        // Press at 80 %, drag down to 30 %, release while moving.
        let start = on_rail(rect, 0.8);
        let mut events = vec![Event::PointerMoved(start), button(start, true)];
        for f in [0.7, 0.6, 0.5, 0.4] {
            events.push(Event::PointerMoved(on_rail(rect, f)));
            opacity_frame(&mut frames, &mut project, &mut drag, clip, events);
            events = Vec::new();
        }
        let end = on_rail(rect, 0.3);
        opacity_frame(
            &mut frames,
            &mut project,
            &mut drag,
            clip,
            vec![Event::PointerMoved(end), button(end, false)],
        );
        opacity_frame(&mut frames, &mut project, &mut drag, clip, Vec::new());
        let dragged = opacity(&project, clip);
        assert!(
            (PerMille::new(280)..=PerMille::new(320)).contains(&dragged),
            "{dragged:?}"
        );
        assert!(!project.gesture_open(), "the drag ended");
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert_eq!(opacity(&project, clip), PerMille::ONE, "one undo: back");
        project.redo().unwrap();

        // Esc during a drag puts the value back and records nothing.
        let entries = project.editor.history().len();
        let start = on_rail(rect, 0.3);
        let mut events = vec![Event::PointerMoved(start), button(start, true)];
        for f in [0.4, 0.6, 0.9] {
            events.push(Event::PointerMoved(on_rail(rect, f)));
            opacity_frame(&mut frames, &mut project, &mut drag, clip, events);
            events = Vec::new();
        }
        assert_ne!(opacity(&project, clip), dragged, "the drag changed it");
        opacity_frame(
            &mut frames,
            &mut project,
            &mut drag,
            clip,
            vec![key(egui::Key::Escape, Modifiers::NONE)],
        );
        assert_eq!(opacity(&project, clip), dragged, "restored");
        assert!(!project.gesture_open());
        // Moving on and releasing after Esc changes nothing.
        let far = on_rail(rect, 0.1);
        opacity_frame(
            &mut frames,
            &mut project,
            &mut drag,
            clip,
            vec![Event::PointerMoved(far)],
        );
        opacity_frame(
            &mut frames,
            &mut project,
            &mut drag,
            clip,
            vec![button(far, false)],
        );
        opacity_frame(&mut frames, &mut project, &mut drag, clip, Vec::new());
        assert_eq!(opacity(&project, clip), dragged);
        assert_eq!(project.editor.history().len(), entries, "no entry");
        project.undo().unwrap();
        assert_eq!(
            opacity(&project, clip),
            PerMille::ONE,
            "the first drag's entry"
        );
    }

    #[test]
    fn the_bundled_family_is_offered_first() {
        let fonts = FontChoices {
            families: vec![
                "Arial".to_owned(),
                "Noto Sans".to_owned(),
                "Zapfino".to_owned(),
            ]
            .into(),
            scanning: false,
        };
        assert_eq!(fonts.ordered(), ["Noto Sans", "Arial", "Zapfino"]);
        assert!(fonts.has("arial"));
        assert!(!fonts.has("Missing Sans"));
    }
}

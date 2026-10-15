//! One open project: the editor, where its file lives, its lock, and the
//! runtime state around it (selection, playhead, where media files are).
//!
//! Blocking work (probing, opening, saving) is done by free functions meant
//! to run on worker threads; the UI thread only plans and commits.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use freecut_media::Probed;
use freecut_project::export::ExportSettings;
use freecut_project::fs::RealFs;
use freecut_project::lock::ProjectLock;
use freecut_project::paths::{ResolvedHow, resolve, stored_path};
use freecut_project::{Extras, OpenWarning, Opened, SaveRequest, UiState};
use freecut_timeline::edit::{
    ColorOp, Cut, EditCtx, EditError, EditRequest, NewMedia, Notice, OpKind, RecordTarget,
    Unavailable,
};
use freecut_timeline::history::{Coalesce, GestureId, HistoryLimits, StateToken};
use freecut_timeline::ids::{ClipId, MediaId, TrackId};
use freecut_timeline::model::{Document, MarkerRef, MediaSource, ProjectSettings, StoredPath};
use freecut_timeline::time::Tick;
use freecut_timeline::{CommitMeta, Committed, Editor};

/// The open project.
#[derive(Debug)]
pub struct Project {
    pub editor: Editor,
    /// The project file; `None` until first saved.
    pub file: Option<PathBuf>,
    /// The folder the document's relative media paths are relative to.
    pub relative_base: Option<PathBuf>,
    /// Unknown keys from the opened file, written back on save.
    pub extras: Extras,
    /// The settings of the last export (EX-021), kept with the project
    /// file; `None` until the first export.
    pub export: Option<ExportSettings>,
    /// Keep a backup of the original file on the next save (after a repair).
    pub backup_on_save: bool,
    /// What the editor remembers about the project outside the document
    /// (AU-012's solo set): saved with it, never undoable. Change the solo
    /// set through [`Project::set_solo`].
    ui: UiState,
    /// Held for its lifetime: dropping it releases the project lock.
    #[allow(dead_code)]
    pub lock: Option<ProjectLock>,
    /// Where each media item's file was found on this machine.
    pub media_paths: BTreeMap<MediaId, PathBuf>,
    /// Media whose file can't be found (shown as offline slates).
    pub offline: BTreeSet<MediaId>,
    /// The LUTs the Look lens offers and which of them are missing (CL-016,
    /// CL-017); the app's, shared by every project and refreshed by it.
    pub luts: Arc<crate::luts::LutChoices>,
    /// The saved looks the Adjust tab and the Presets select list
    /// (CL-026); the app's, shared by every project and refreshed by it.
    pub looks: Arc<crate::looks::SavedLooks>,
    pub selection: Vec<ClipId>,
    /// The selected markers (TL-037): kept beside the clip selection, never
    /// undoable; markers that no longer exist drop out after every change.
    pub markers: BTreeSet<MarkerRef>,
    pub playhead: Tick,
    /// A save is running; don't start another.
    pub saving: bool,
    /// Bumped whenever `media_paths` or `offline` change, so the sound
    /// engine knows when to rebuild its file table.
    media_revision: u64,
    /// The format version the project was upgraded from when opened.
    pub upgraded_from: Option<u32>,
    /// The style **Copy style** took from a text clip (TX-020), for
    /// **Paste style**. Not saved with the project.
    pub style_clipboard: Option<Box<freecut_timeline::model::TextStyle>>,
    /// Sentences about what the last edits did that the user should know
    /// (an import with several audio streams, AU-038); the app shows and
    /// clears them.
    notes: Vec<String>,
    /// Why edits are refused right now, when they are: a voiceover take is
    /// counting down or recording, and its start must not move under it
    /// (AU-027). Undo and redo are refused too.
    pub edit_lock: Option<&'static str>,
    /// The Auto or Match analysis running on the pool (CL-019, CL-020),
    /// for the Look lens's spinner and Cancel; never saved.
    pub color_job: Option<ColorJob>,
    /// Match's pick mode (CL-020): the clip waiting for a reference frame
    /// to be clicked on the timeline; never saved.
    pub match_pick: Option<MatchPick>,
    /// The colour mask the Look lens's Masks page is editing (CL-031), so
    /// the Stage draws its outline and handles (TR-019). Set by the
    /// inspector every frame it shows the page; never saved.
    pub mask_focus: Option<MaskFocus>,
}

/// The colour mask being edited on the Stage (CL-031, TR-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskFocus {
    pub clip: ClipId,
    /// The mask's id in the clip's stack.
    pub id: u16,
}

/// An analysis running for a clip (CL-019, CL-020).
#[derive(Debug, Clone)]
pub struct ColorJob {
    pub id: u64,
    pub clip: ClipId,
    /// Auto (`true`) or Match.
    pub auto: bool,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

/// Match's pick mode (CL-020).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchPick {
    /// The clip to match.
    pub target: ClipId,
    /// Measure the reference as graded (CL-020's default) or ungraded.
    pub graded: bool,
}

/// Why an edit didn't happen, for the status line.
pub type EditOutcome = Result<Committed, String>;

impl Project {
    /// An empty, unsaved project.
    pub fn new(settings: ProjectSettings) -> anyhow::Result<Project> {
        let editor = Editor::with_settings(settings, HistoryLimits::default())
            .map_err(|e| anyhow!("can't create a project: {e}"))?;
        Ok(Project::with_editor(editor))
    }

    fn with_editor(editor: Editor) -> Project {
        Project {
            editor,
            file: None,
            relative_base: None,
            extras: Extras::new(),
            export: None,
            backup_on_save: false,
            ui: UiState::new(),
            lock: None,
            media_paths: BTreeMap::new(),
            offline: BTreeSet::new(),
            luts: Arc::default(),
            looks: Arc::default(),
            selection: Vec::new(),
            markers: BTreeSet::new(),
            playhead: Tick::ZERO,
            saving: false,
            media_revision: 0,
            upgraded_from: None,
            style_clipboard: None,
            notes: Vec::new(),
            edit_lock: None,
            color_job: None,
            match_pick: None,
            mask_focus: None,
        }
    }

    /// A project loaded by [`open_project`].
    pub fn from_loaded(loaded: Loaded) -> Project {
        let Loaded {
            path,
            opened,
            lock,
            media_paths,
            offline,
        } = loaded;
        // PM-021: an upgraded file keeps its original as a backup on the
        // first save, like a repaired one.
        let backup_on_save = opened.backup_on_save();
        let upgraded_from = opened.warnings.iter().find_map(|w| match w {
            OpenWarning::Migrated { from, .. } => Some(*from),
            _ => None,
        });
        let mut editor =
            Editor::from_document(opened.document, opened.ids, HistoryLimits::default());
        if opened.needs_save {
            editor.mark_unsaved();
        }
        let notes = open_notes(&opened.warnings);
        Project {
            relative_base: path.parent().map(Path::to_path_buf),
            file: Some(path),
            extras: opened.extras,
            export: opened.export,
            ui: opened.ui,
            backup_on_save,
            upgraded_from,
            notes,
            lock: Some(lock),
            media_paths,
            offline,
            ..Project::with_editor(editor)
        }
    }

    pub fn doc(&self) -> &Document {
        self.editor.document()
    }

    /// The project's name: its file name without the extension, or
    /// "Untitled" before the first save.
    pub fn name(&self) -> String {
        self.file.as_deref().and_then(Path::file_stem).map_or_else(
            || "Untitled".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
    }

    /// The window title: file name and an unsaved-changes marker.
    pub fn title(&self) -> String {
        let name = self.name();
        if self.editor.is_dirty() {
            format!("{name} •")
        } else {
            name
        }
    }

    /// The soloed tracks (AU-012); empty when nothing is soloed.
    pub fn solo(&self) -> &BTreeSet<TrackId> {
        &self.ui.solo
    }

    /// Replaces the solo set (AU-012). Not an edit: nothing goes on the undo
    /// stack, but the project counts as modified so the set is saved with it
    /// (like the export settings, EX-021). Tracks that don't carry sound are
    /// dropped.
    pub fn set_solo(&mut self, solo: BTreeSet<TrackId>) {
        let ui = UiState { solo }.retained_for(self.editor.document());
        if ui != self.ui {
            self.ui = ui;
            self.editor.mark_unsaved();
        }
    }

    /// Takes the notes collected since the last call.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    pub fn ctx(&self) -> EditCtx {
        EditCtx::default()
    }

    /// Changes whenever `media_paths` or `offline` change.
    pub fn media_revision(&self) -> u64 {
        self.media_revision
    }

    /// Starts a continuous edit (a slider drag): commits made with
    /// `Coalesce::Gesture` of the returned id merge into one undo entry.
    pub fn begin_gesture(&mut self) -> GestureId {
        self.editor.begin_gesture()
    }

    pub fn end_gesture(&mut self, id: GestureId) {
        self.editor.end_gesture(id);
    }

    /// Reverts everything the gesture `id` committed and forgets it, as if
    /// it never happened (TR-007: Esc cancels a drag).
    pub fn cancel_gesture(&mut self, id: GestureId) -> Result<(), String> {
        let result = self.editor.cancel_gesture(id).map_err(|e| e.to_string());
        self.after_change();
        result
    }

    /// A slider or field drag is in progress: until it ends, other edits
    /// would either be refused or end it early.
    pub fn gesture_open(&self) -> bool {
        self.editor.gesture_open()
    }

    /// Refuses with the lock's reason while edits are locked.
    fn unlocked(&self) -> Result<(), String> {
        self.edit_lock.map_or(Ok(()), |why| Err(why.to_owned()))
    }

    /// Plans and commits `req`, keeping the selection in step.
    pub fn apply(&mut self, req: &EditRequest, coalesce: Coalesce) -> EditOutcome {
        self.unlocked()?;
        let plan = self
            .editor
            .plan(&self.ctx(), req)
            .map_err(|e| describe_edit_error(&e))?;
        let meta = CommitMeta {
            selection_before: self.selection.clone(),
            coalesce,
        };
        let committed = self.editor.commit(plan, meta).map_err(|e| e.to_string())?;
        if let Committed::Applied { selection, notices } = &committed {
            if let Some(sel) = selection {
                self.selection.clone_from(sel);
            }
            self.take_notices(notices);
        }
        self.after_change();
        Ok(committed)
    }

    /// Acts on what an edit reports: the playhead a head cut asks for
    /// (TL-026), the marker an edit added (TL-037), and sentences for the
    /// status line.
    fn take_notices(&mut self, notices: &[Notice]) {
        for notice in notices {
            match notice {
                Notice::PlayheadTo(t) => self.playhead = *t,
                Notice::AddedMarker(r) => self.markers = BTreeSet::from([*r]),
                _ => {}
            }
            if let Some(text) = describe_notice(self.editor.document(), notice) {
                self.notes.push(text);
            }
        }
    }

    pub fn undo(&mut self) -> Result<(), String> {
        self.unlocked()?;
        let restored = self.editor.undo().map_err(|e| e.to_string())?;
        if let Some(sel) = restored.selection {
            self.selection = sel;
        }
        self.after_change();
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), String> {
        self.unlocked()?;
        let restored = self.editor.redo().map_err(|e| e.to_string())?;
        if let Some(sel) = restored.selection {
            self.selection = sel;
        }
        self.after_change();
        Ok(())
    }

    /// Keeps runtime state consistent with the document after any change.
    fn after_change(&mut self) {
        let doc = self.editor.document();
        self.selection.retain(|id| doc.clip(*id).is_some());
        self.markers.retain(|r| marker_exists(doc, *r));
        // AU-012: removing a soloed track clears its solo (and an undo that
        // brings the track back doesn't bring the solo back).
        if !self.ui.solo.is_empty() {
            let kept = self.ui.retained_for(doc);
            if kept != self.ui {
                self.ui = kept;
            }
        }
        let library = doc.library();
        // Entries of items an undo removed are kept: a redo brings the item
        // back under the same id (ids are never reused), and it must find
        // its file again.
        let mut changed = false;
        // Extracted audio (AU-004) reads its parent's file: an item that
        // isn't resolved yet takes the parent's result when it points at the
        // same file. Opened projects resolve every item themselves.
        for item in library.items() {
            let MediaSource::AudioOf { source, .. } = item.source else {
                continue;
            };
            if self.media_paths.contains_key(&item.id) {
                continue;
            }
            let Some(parent) = library.get(source).filter(|p| p.path == item.path) else {
                continue;
            };
            if let Some(path) = self.media_paths.get(&parent.id).cloned() {
                if self.offline.contains(&parent.id) {
                    self.offline.insert(item.id);
                }
                self.media_paths.insert(item.id, path);
                changed = true;
            }
        }
        if changed {
            self.media_revision += 1;
        }
        let end = doc
            .sequence(doc.root())
            .map_or(Tick::ZERO, freecut_timeline::model::Sequence::duration);
        if self.playhead > end {
            self.playhead = end;
        }
    }

    /// Everything a worker needs to save to `target`.
    pub fn save_job(&self, target: PathBuf) -> SaveJob {
        SaveJob {
            target,
            document: self.editor.document().clone(),
            ids_next: self.editor.ids().next(),
            extras: self.extras.clone(),
            export: self.export.clone(),
            ui: self.ui.retained_for(self.editor.document()),
            backup: self.backup_on_save,
            relative_base: self.relative_base.clone(),
            token: self.editor.generation(),
        }
    }

    /// Called on the UI thread when a save finished.
    pub fn saved(&mut self, job: &SavedJob) {
        self.saving = false;
        if job.result.is_ok() {
            self.editor.mark_saved_at(job.token);
            self.backup_on_save = false;
            if self.file.as_deref() != Some(job.target.as_path()) {
                self.file = Some(job.target.clone());
            }
            if self.relative_base.is_none() {
                self.relative_base = job.target.parent().map(Path::to_path_buf);
            }
        }
    }

    /// Adds probed files to the library in one undoable step. Returns the
    /// new media ids.
    pub fn import(&mut self, probed: Vec<ProbedFile>) -> Result<Vec<MediaId>, String> {
        self.import_with(probed, Coalesce::Never)
    }

    /// Adds files picked with the Audio panel's "Import audio…" (AU-001,
    /// AU-030) in one undoable step: each file is imported, and a video's
    /// sound is extracted as well (AU-004), so every file shows up in the
    /// Audio panel. Callers drop files without sound first ([`sound_only`]).
    /// Returns the items the Audio panel lists, in file order.
    pub fn import_audio(&mut self, probed: Vec<ProbedFile>) -> Result<Vec<MediaId>, String> {
        if probed.is_empty() {
            return Ok(Vec::new());
        }
        let gesture = self.begin_gesture();
        let result = self.import_audio_in(probed, gesture);
        self.end_gesture(gesture);
        result
    }

    fn import_audio_in(
        &mut self,
        probed: Vec<ProbedFile>,
        gesture: GestureId,
    ) -> Result<Vec<MediaId>, String> {
        let ids = self.import_with(probed, Coalesce::Gesture(gesture))?;
        let mut listed = Vec::with_capacity(ids.len());
        for id in ids {
            let has_video = self
                .doc()
                .library()
                .get(id)
                .is_some_and(|m| m.info.has_video());
            if has_video {
                let (audio, _) = self.extract_audio_with(id, Coalesce::Gesture(gesture))?;
                listed.push(audio);
            } else {
                listed.push(id);
            }
        }
        Ok(listed)
    }

    fn import_with(
        &mut self,
        probed: Vec<ProbedFile>,
        coalesce: Coalesce,
    ) -> Result<Vec<MediaId>, String> {
        if probed.is_empty() {
            return Ok(Vec::new());
        }
        self.unlocked()?;
        let paths: Vec<PathBuf> = probed.iter().map(|p| p.path.clone()).collect();
        let items = probed.into_iter().map(|p| p.media).collect();
        let req = EditRequest::ImportMedia(items);
        let plan = self
            .editor
            .plan(&self.ctx(), &req)
            .map_err(|e| describe_edit_error(&e))?;
        let ids = plan.imported().to_vec();
        let meta = CommitMeta {
            selection_before: self.selection.clone(),
            coalesce,
        };
        let committed = self.editor.commit(plan, meta).map_err(|e| e.to_string())?;
        if let Committed::Applied { notices, .. } = &committed {
            let doc = self.editor.document();
            self.notes
                .extend(notices.iter().filter_map(|n| describe_notice(doc, n)));
        }
        for (id, path) in ids.iter().zip(paths) {
            self.media_paths.insert(*id, path);
        }
        self.media_revision += 1;
        Ok(ids)
    }
}

impl Project {
    /// Places a recorded voiceover take (AU-027) as one undoable step: the
    /// library item (origin Voiceover), the clip starting at `at` with
    /// `handle_in` of pre-roll before its in-point, any new track, and for
    /// *Redo last take* the removal of `replace`. Returns the new clip,
    /// which is selected.
    pub fn insert_recorded(
        &mut self,
        probed: ProbedFile,
        at: Tick,
        handle_in: Tick,
        target: RecordTarget,
        replace: Option<ClipId>,
    ) -> Result<ClipId, String> {
        self.unlocked()?;
        let req = EditRequest::InsertRecordedClip {
            seq: self.doc().root(),
            media: probed.media,
            at,
            handle_in,
            target,
            replace,
        };
        let plan = self
            .editor
            .plan(&self.ctx(), &req)
            .map_err(|e| describe_edit_error(&e))?;
        let media = plan
            .imported()
            .first()
            .copied()
            .ok_or("FreeCut couldn't place the take")?;
        let meta = CommitMeta {
            selection_before: self.selection.clone(),
            coalesce: Coalesce::Never,
        };
        let committed = self.editor.commit(plan, meta).map_err(|e| e.to_string())?;
        if let Committed::Applied { selection, notices } = &committed {
            if let Some(sel) = selection {
                self.selection.clone_from(sel);
            }
            self.take_notices(notices);
        }
        self.media_paths.insert(media, probed.path);
        self.media_revision += 1;
        self.after_change();
        self.selection
            .first()
            .copied()
            .ok_or_else(|| "FreeCut couldn't place the take".to_owned())
    }

    /// Adds an audio-only library item for `media`'s default audio stream
    /// (AU-004). Returns the item and whether it is new; extracting the same
    /// stream again reuses the existing item.
    pub fn extract_audio(&mut self, media: MediaId) -> Result<(MediaId, bool), String> {
        self.extract_audio_with(media, Coalesce::Never)
    }

    fn extract_audio_with(
        &mut self,
        media: MediaId,
        coalesce: Coalesce,
    ) -> Result<(MediaId, bool), String> {
        self.unlocked()?;
        let req = EditRequest::ExtractAudio {
            media,
            stream: None,
        };
        let plan = self
            .editor
            .plan(&self.ctx(), &req)
            .map_err(|e| describe_edit_error(&e))?;
        let id = plan
            .imported()
            .first()
            .copied()
            .ok_or("FreeCut couldn't extract the sound")?;
        let created = !plan.is_noop();
        let meta = CommitMeta {
            selection_before: self.selection.clone(),
            coalesce,
        };
        self.editor.commit(plan, meta).map_err(|e| e.to_string())?;
        self.after_change();
        Ok((id, created))
    }
}

/// Whether `r` names a marker of the root sequence or of one of its clips.
pub fn marker_exists(doc: &Document, r: MarkerRef) -> bool {
    match r {
        MarkerRef::Timeline(id) => doc
            .sequence(doc.root())
            .is_some_and(|s| s.markers.get(id).is_some()),
        MarkerRef::Clip(clip, id) => doc.clip(clip).is_some_and(|c| c.markers.get(id).is_some()),
    }
}

/// A user-facing sentence for an edit that was refused.
pub fn describe_edit_error(err: &EditError) -> String {
    match err {
        EditError::Invariant(_) | EditError::Internal(_) => {
            // A bug, not a user error; keep the details for the log.
            format!("FreeCut couldn't make that edit (internal error: {err})")
        }
        EditError::NoSound(_) => "This video has no sound".to_owned(),
        EditError::UnknownStream { .. } => "That file doesn't have this audio stream".to_owned(),
        EditError::NothingToDetach => "None of the selected clips has sound to detach".to_owned(),
        EditError::NothingToRestore => {
            "None of the selected clips has detached sound to restore".to_owned()
        }
        EditError::NoClipAudio => "None of the selected clips has sound".to_owned(),
        EditError::NegativeFade => "A fade can't be shorter than 0 seconds".to_owned(),
        EditError::NoTextClip => "None of the selected clips is a text clip".to_owned(),
        EditError::NoVisualClip => "None of the selected clips shows a picture".to_owned(),
        EditError::NoColorClip => {
            "Only video, image and adjustment clips take colour adjustments".to_owned()
        }
        EditError::VolumeKeyframed => {
            "The volume has keyframes: change the keyframes instead".to_owned()
        }
        EditError::NoKeyframe { .. } => "There is no volume keyframe there".to_owned(),
        EditError::NoKey { .. } => "There is no keyframe there".to_owned(),
        EditError::NotAnimatable => "That setting can't take keyframes".to_owned(),
        EditError::NotUnderPlayhead => {
            "None of the selected clips is under the playhead".to_owned()
        }
        EditError::TrackHasNoSound { .. } => "This track has no sound to change".to_owned(),
        EditError::SoundDetached => {
            "The sound of the selected clips is detached; change its audio clips instead".to_owned()
        }
        EditError::NotVideo { .. } => "Only a video clip can add another audio stream".to_owned(),
        EditError::SingleStream(_) => "This file has only one audio stream".to_owned(),
        EditError::StreamAlreadyPlaying { .. } => {
            "This clip already plays that stream; detach its audio instead".to_owned()
        }
        EditError::NothingToResync => "None of the selected clips is out of sync".to_owned(),
        EditError::NothingToTrim => "Move the playhead inside the clip".to_owned(),
        EditError::StaleMarker(_) => "That marker no longer exists".to_owned(),
        EditError::StreamPlaysElsewhere { .. } => {
            "That stream already plays on a linked clip; choosing it here would play it twice"
                .to_owned()
        }
        EditError::SectionSwitchedOff { section } => auto_apply_reason(*section),
        other => {
            let text = other.to_string();
            let mut chars = text.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        }
    }
}

/// Why **Apply to sliders** can't apply (CL-019): `section` is switched
/// off, so moving Auto's offsets would change the picture.
pub fn auto_apply_reason(section: freecut_timeline::model::Section) -> String {
    format!(
        "Switch {} on first: applying Auto now would change the picture",
        crate::ui::look::section_label(section)
    )
}

/// The name of an edit, for the Edit menu ("Undo Detach Audio").
pub fn op_name(op: OpKind) -> &'static str {
    match op {
        OpKind::AddTrack => "Add Track",
        OpKind::RemoveTrack => "Remove Track",
        OpKind::SetTrackHeader => "Track Setting",
        OpKind::SetMagnet => "Magnetic Main Track",
        OpKind::InsertClips => "Insert",
        OpKind::MoveClips => "Move",
        OpKind::TrimClip => "Trim",
        OpKind::SplitClips => "Split",
        OpKind::DeleteClips => "Delete",
        OpKind::RippleDeleteClips => "Ripple Delete",
        OpKind::Conform => "Conform to Frames",
        OpKind::ImportMedia => "Import",
        OpKind::RemoveMedia => "Remove Media",
        OpKind::RelinkMedia => "Relink",
        OpKind::UpdateMediaInfo => "Update Media",
        OpKind::SetProjectSettings => "Project Settings",
        OpKind::DetachAudio => "Detach Audio",
        OpKind::RestoreAudio => "Restore Audio",
        OpKind::SetClipAudio => "Clip Sound",
        OpKind::ExtractAudio => "Extract Audio",
        OpKind::SetMasterGain => "Master Volume",
        OpKind::SetTextContent => "Edit Text",
        OpKind::SetTextStyle => "Text Style",
        OpKind::SetTextLayout => "Text Layout",
        OpKind::ResetText => "Reset Text",
        OpKind::SetTransform => "Transform",
        OpKind::EditGainKeys => "Volume Keyframes",
        OpKind::SetTrackGain => "Track Volume",
        OpKind::SetTrackPan => "Track Pan",
        OpKind::SetAudioStream => "Audio Stream",
        OpKind::AddStreamAsClip => "Add Stream as Clip",
        OpKind::Resync => "Resync",
        OpKind::EditKeys => "Keyframes",
        OpKind::SetClipAnimation => "Animation",
        OpKind::TrimToPlayhead(Cut::Head) => "Cut Away Before Playhead",
        OpKind::TrimToPlayhead(Cut::Tail) => "Cut Away After Playhead",
        OpKind::AddMarker => "Add Marker",
        OpKind::EditMarker => "Edit Marker",
        OpKind::MoveMarker => "Move Marker",
        OpKind::DeleteMarkers => "Delete Markers",
        OpKind::SetFollowMain => "Follow Main Track",
        OpKind::SetFollowKinds => "Item Kinds",
        OpKind::InsertRecordedClip => "Record Voiceover",
        OpKind::EditColor(ColorOp::Set) => "Color Adjustment",
        OpKind::EditColor(ColorOp::Switch) => "Color Section",
        OpKind::EditColor(ColorOp::Bypass) => "Color On/Off",
        OpKind::EditColor(ColorOp::Reset) => "Reset Color",
        OpKind::EditColor(ColorOp::Curve) => "Curve",
        OpKind::EditColor(ColorOp::Puck) => "Color Wheel",
        OpKind::EditColor(ColorOp::Lut) => "LUT",
        OpKind::EditColor(ColorOp::Look) => "Apply Look",
        OpKind::EditColor(ColorOp::Auto) => "Auto Color",
        OpKind::EditColor(ColorOp::Match) => "Match Color",
        OpKind::EditColor(ColorOp::Mask) => "Color Mask",
        OpKind::AddAdjustmentClip => "Add Adjustment Clip",
    }
}

/// A sentence about an edit's side effect worth telling the user, if any.
pub fn describe_notice(doc: &Document, notice: &Notice) -> Option<String> {
    match notice {
        Notice::MultipleAudioStreams {
            media,
            count,
            default_stream,
        } => {
            let item = doc.library().get(*media)?;
            let name = crate::ui::timeline::media_name(item);
            let position = item
                .info
                .audio
                .iter()
                .position(|s| s.index == *default_stream)
                .map_or_else(String::new, |i| format!(" (stream {})", i + 1));
            Some(format!(
                "{name} has {count} audio streams; the default one{position} plays. \
                 Choose another in the inspector"
            ))
        }
        Notice::AlsoRemovedAttached(n) => Some(if *n == 1 {
            "Also removed 1 attached item".to_owned()
        } else {
            format!("Also removed {n} attached items")
        }),
        Notice::Skipped(skipped) => skipped_summary(skipped),
        Notice::ExtractedAudioNotRelinked(_)
        | Notice::ClosedGaps(_)
        | Notice::CreatedTrack(_)
        | Notice::RemovedEmptyTracks(_)
        | Notice::PlayheadTo(_)
        | Notice::AddedMarker(_) => None,
    }
}

/// Why a command can't act on a clip (TL-054), as a tooltip or summary
/// sentence.
pub fn unavailable_reason(why: Unavailable) -> &'static str {
    match why {
        Unavailable::NoTarget => "Select a clip or put the playhead over the main track",
        Unavailable::LockedTrack => "The selected clip is on a locked track",
        Unavailable::PlayheadOutside => "Move the playhead inside the clip",
        Unavailable::NoRoom => "A linked clip has no room for the change",
        _ => "This clip can't be changed now",
    }
}

/// TL-053's summary of the clips an edit left alone, or `None` when it
/// skipped none.
fn skipped_summary(skipped: &[(ClipId, Unavailable)]) -> Option<String> {
    let first = skipped.first()?.1;
    let n = skipped.len();
    let what = if n == 1 {
        "1 clip was skipped".to_owned()
    } else {
        format!("{n} clips were skipped")
    };
    let same = skipped.iter().all(|(_, why)| *why == first);
    Some(if same {
        let reason = match first {
            Unavailable::LockedTrack => "on a locked track",
            Unavailable::PlayheadOutside => "the playhead isn't inside",
            Unavailable::NoRoom => "no room to stay in sync",
            _ => "nothing to cut",
        };
        format!("{what} ({reason})")
    } else {
        what
    })
}

/// What opening found that the user should hear about, beyond repairs and
/// upgrades (which have their own messages).
fn open_notes(warnings: &[OpenWarning]) -> Vec<String> {
    warnings
        .iter()
        .filter_map(|w| match w {
            OpenWarning::MissingStreams { clips } => Some(format!(
                "{} clip(s) chose an audio stream their file doesn't have and play its \
                 default stream",
                clips.len()
            )),
            OpenWarning::KeyframesSorted { clips } => Some(format!(
                "Volume keyframes of {clips} clip(s) were out of order and were sorted"
            )),
            OpenWarning::StrayColorKeysDropped { clips } => Some(format!(
                "Colour keyframes of {clips} clip(s) named a mask, LUT or result the clip \
                 doesn't have and were removed"
            )),
            _ => None,
        })
        .collect()
}

/// A save to run on a worker.
#[derive(Debug, Clone)]
pub struct SaveJob {
    pub target: PathBuf,
    document: Document,
    ids_next: NonZeroU64,
    extras: Extras,
    export: Option<ExportSettings>,
    ui: UiState,
    backup: bool,
    relative_base: Option<PathBuf>,
    token: StateToken,
}

/// A finished save.
#[derive(Debug)]
pub struct SavedJob {
    pub target: PathBuf,
    pub token: StateToken,
    pub result: Result<(), String>,
}

impl SaveJob {
    /// Blocking: writes the project atomically.
    pub fn run(self) -> SavedJob {
        let request = SaveRequest {
            document: &self.document,
            ids_next: self.ids_next,
            extras: &self.extras,
            backup: self.backup,
            relative_base: self.relative_base.as_deref(),
            export: self.export.as_ref(),
            ui: Some(&self.ui),
        };
        let result = freecut_project::save(&self.target, &request).map_err(|e| e.to_string());
        SavedJob {
            target: self.target,
            token: self.token,
            result,
        }
    }
}

/// A project read from disk, ready to become the open [`Project`].
#[derive(Debug)]
pub struct Loaded {
    pub path: PathBuf,
    pub opened: Opened,
    pub lock: ProjectLock,
    pub media_paths: BTreeMap<MediaId, PathBuf>,
    pub offline: BTreeSet<MediaId>,
}

/// Blocking: locks, opens and validates the project at `path`, and finds
/// its media files.
pub fn open_project(path: &Path) -> anyhow::Result<Loaded> {
    let lock =
        ProjectLock::acquire(path).with_context(|| format!("can't open {}", path.display()))?;
    let opened =
        freecut_project::open(path).with_context(|| format!("can't open {}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut media_paths = BTreeMap::new();
    let mut offline = BTreeSet::new();
    for item in opened.document.library().items() {
        let found = resolve(&item.path, dir, &RealFs);
        if found.how == ResolvedHow::NotFound {
            offline.insert(item.id);
        }
        media_paths.insert(item.id, found.path);
    }
    Ok(Loaded {
        path: path.to_path_buf(),
        opened,
        lock,
        media_paths,
        offline,
    })
}

/// A probed file, ready to import.
#[derive(Debug, Clone)]
pub struct ProbedFile {
    pub path: PathBuf,
    pub media: NewMedia,
}

/// Blocking: probes `path` for import. `base` is the project folder the
/// stored relative path should be relative to, when known.
pub fn probe_for_import(path: &Path, base: Option<&Path>) -> Result<ProbedFile, String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let Probed { info, fingerprint } =
        freecut_media::probe(path).map_err(|e| format!("Couldn't import {name}: {e}"))?;
    let stored = match base {
        Some(dir) => stored_path(path, dir).map_err(|e| format!("Couldn't import {name}: {e}"))?,
        None => StoredPath {
            absolute: path
                .to_str()
                .ok_or_else(|| format!("Couldn't import {name}: the path isn't valid Unicode"))?
                .to_owned(),
            relative: None,
        },
    };
    Ok(ProbedFile {
        path: path.to_path_buf(),
        media: NewMedia {
            path: stored,
            info,
            fingerprint: Some(fingerprint),
            display_name: None,
        },
    })
}

/// Splits an "Import audio…" batch (AU-001): files with an audio stream are
/// kept; the rest are counted as skipped.
pub fn sound_only(files: Vec<ProbedFile>) -> (Vec<ProbedFile>, usize) {
    let total = files.len();
    let kept: Vec<ProbedFile> = files
        .into_iter()
        .filter(|f| f.media.info.has_audio())
        .collect();
    let skipped = total - kept.len();
    (kept, skipped)
}

/// The status line after an import batch (PM-037): how many files were
/// imported and how many were skipped for having no sound, worded for one
/// file or several.
pub fn import_report(imported: usize, skipped_no_sound: usize) -> String {
    let files = |n: usize| if n == 1 { "file" } else { "files" };
    let done = format!("Imported {imported} {}", files(imported));
    match (imported, skipped_no_sound) {
        (_, 0) => done,
        (0, s) => format!("{s} {} skipped: no sound", files(s)),
        (_, s) => format!("{done}. {s} {} skipped: no sound", files(s)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::model::{AudioStreamInfo, MediaInfo};

    use super::*;

    fn probed(name: &str, audio: bool) -> ProbedFile {
        let info = MediaInfo {
            container: "wav".to_owned(),
            default_video: None,
            duration: Tick::from_secs(1).unwrap(),
            video: Vec::new(),
            audio: if audio {
                vec![AudioStreamInfo {
                    index: 0,
                    codec: "pcm_s16le".to_owned(),
                    sample_rate: 48_000,
                    channels: 2,
                    start: Tick::ZERO,
                    duration: Tick::from_secs(1).unwrap(),
                    duration_exact: true,
                    default: true,
                    language: None,
                    title: None,
                    layout: None,
                }]
            } else {
                Vec::new()
            },
            still: None,
        };
        ProbedFile {
            path: PathBuf::from(name),
            media: NewMedia {
                path: StoredPath {
                    absolute: name.to_owned(),
                    relative: None,
                },
                info,
                fingerprint: None,
                display_name: None,
            },
        }
    }

    #[test]
    fn audio_import_skips_files_without_sound() {
        let files = vec![
            probed("a.wav", true),
            probed("b.mp4", false),
            probed("c.mp3", true),
        ];
        let (kept, skipped) = sound_only(files);
        let names: Vec<_> = kept.iter().map(|f| f.path.clone()).collect();
        assert_eq!(names, [PathBuf::from("a.wav"), PathBuf::from("c.mp3")]);
        assert_eq!(skipped, 1);
        let (kept, skipped) = sound_only(Vec::new());
        assert!(kept.is_empty());
        assert_eq!(skipped, 0);
    }

    #[test]
    fn import_report_counts_skipped_files() {
        assert_eq!(import_report(1, 0), "Imported 1 file");
        assert_eq!(import_report(3, 0), "Imported 3 files");
        assert_eq!(import_report(0, 1), "1 file skipped: no sound");
        assert_eq!(import_report(0, 2), "2 files skipped: no sound");
        assert_eq!(
            import_report(2, 1),
            "Imported 2 files. 1 file skipped: no sound"
        );
        assert_eq!(
            import_report(1, 4),
            "Imported 1 file. 4 files skipped: no sound"
        );
    }

    #[test]
    fn the_solo_set_is_saved_with_the_project_but_never_undone() {
        use crate::ui::drag::tests::project_with_a_sound;

        let (mut project, _) = project_with_a_sound();
        let doc = project.doc().clone();
        let seq = doc.root();
        let s = doc.sequence(seq).unwrap();
        let main = s.main_track().unwrap().id;
        let audio = s.audio_tracks()[0].id;
        project.editor.mark_saved();
        let entries = project.editor.history().len();

        // Soloing marks the project modified but adds no undo entry.
        project.set_solo(BTreeSet::from([audio, main]));
        assert_eq!(project.solo(), &BTreeSet::from([audio, main]));
        assert!(project.editor.is_dirty());
        assert_eq!(project.editor.history().len(), entries);
        // The same set again changes nothing.
        project.editor.mark_saved();
        project.set_solo(BTreeSet::from([main, audio]));
        assert!(!project.editor.is_dirty());

        // Save and reopen: the set comes back.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Solo.freecut");
        let saved = project.save_job(path.clone()).run();
        assert_eq!(saved.result, Ok(()));
        let reopened = Project::from_loaded(open_project(&path).unwrap());
        assert_eq!(reopened.solo(), &BTreeSet::from([audio, main]));
        assert!(!reopened.editor.is_dirty());
        drop(reopened);

        // Removing a soloed track clears its solo; undo doesn't bring the
        // solo back (AU-012), and the next save writes what is left.
        project
            .apply(
                &EditRequest::RemoveTrack { seq, track: audio },
                Coalesce::Never,
            )
            .unwrap();
        assert_eq!(project.solo(), &BTreeSet::from([main]));
        project.undo().unwrap();
        assert_eq!(project.solo(), &BTreeSet::from([main]));
        project.set_solo(BTreeSet::new());
        let saved = project.save_job(path.clone()).run();
        assert_eq!(saved.result, Ok(()));
        let reopened = Project::from_loaded(open_project(&path).unwrap());
        assert!(reopened.solo().is_empty());
    }

    #[test]
    fn imports_with_several_audio_streams_leave_a_note() {
        use freecut_timeline::model::AudioStreamInfo;

        let mut file = probed("talk.mkv", true);
        let stream = |index: u32, default: bool| AudioStreamInfo {
            index,
            codec: "aac".to_owned(),
            sample_rate: 48_000,
            channels: 2,
            start: Tick::ZERO,
            duration: Tick::from_secs(1).unwrap(),
            duration_exact: true,
            default,
            language: None,
            title: None,
            layout: None,
        };
        file.media.info.audio = vec![stream(1, false), stream(2, true)];
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        project.import(vec![file]).unwrap();
        assert_eq!(
            project.take_notes(),
            [
                "talk.mkv has 2 audio streams; the default one (stream 2) plays. Choose another \
              in the inspector"
            ]
        );
        assert!(project.take_notes().is_empty());
        project.import(vec![probed("one.wav", true)]).unwrap();
        assert!(project.take_notes().is_empty());
    }

    #[test]
    fn new_refusals_read_as_sentences() {
        assert_eq!(
            describe_edit_error(&EditError::VolumeKeyframed),
            "The volume has keyframes: change the keyframes instead"
        );
        assert_eq!(
            describe_edit_error(&EditError::NothingToResync),
            "None of the selected clips is out of sync"
        );
        let id = freecut_timeline::ids::ClipId::from_raw(std::num::NonZeroU64::MIN);
        assert_eq!(
            describe_edit_error(&EditError::StreamPlaysElsewhere {
                clip: id,
                stream: 2,
                by: id,
            }),
            "That stream already plays on a linked clip; choosing it here would play it twice"
        );
        assert_eq!(op_name(OpKind::SetTrackGain), "Track Volume");
    }
}

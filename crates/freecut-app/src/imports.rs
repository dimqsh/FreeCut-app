//! Files being probed for import, and the finished batches waiting to be
//! added to the open project (PM-030, AU-001).
//!
//! Probes run on workers; each carries a [`Ticket`] naming the project it
//! was started for, so a probe that finishes after another project opened
//! is dropped instead of landing in the wrong project.

use crate::session::ProbedFile;
use crate::ui::assets::ImportRequest;

/// What a probe job carries back with its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    serial: u64,
    kind: ImportRequest,
}

/// Every probed file of a batch, ready to import as one undo step each.
#[derive(Debug, Default)]
pub struct Batch {
    /// From Import… and dropped files.
    pub media: Vec<ProbedFile>,
    /// From the Audio panel's "Import audio…".
    pub audio: Vec<ProbedFile>,
}

#[derive(Debug, Default)]
pub struct Imports {
    /// Bumped whenever another project opens.
    serial: u64,
    probing: usize,
    pending: Batch,
}

impl Imports {
    /// A probe is starting for the open project.
    pub fn start(&mut self, kind: ImportRequest) -> Ticket {
        self.probing += 1;
        Ticket {
            serial: self.serial,
            kind,
        }
    }

    /// A probe finished. Returns the error to show, if any. Results for a
    /// project that is no longer open are dropped silently.
    pub fn finished(
        &mut self,
        ticket: Ticket,
        result: Result<ProbedFile, String>,
    ) -> Option<String> {
        if ticket.serial != self.serial {
            return None;
        }
        self.probing = self.probing.saturating_sub(1);
        match (result, ticket.kind) {
            (Ok(file), ImportRequest::Media) => self.pending.media.push(file),
            (Ok(file), ImportRequest::Audio) => self.pending.audio.push(file),
            (Err(e), _) => return Some(e),
        }
        None
    }

    /// Files still being probed for the open project.
    pub fn probing(&self) -> usize {
        self.probing
    }

    /// The finished batch, once every probe is done. While `busy` (a slider
    /// or field drag holds the undo history), the batch waits: importing
    /// then would fail and lose the files, or end the drag early.
    pub fn take_ready(&mut self, busy: bool) -> Option<Batch> {
        if busy || !self.waiting() {
            return None;
        }
        Some(std::mem::take(&mut self.pending))
    }

    /// A finished batch is waiting to be imported.
    pub fn waiting(&self) -> bool {
        self.probing == 0 && !(self.pending.media.is_empty() && self.pending.audio.is_empty())
    }

    /// Another project opened: forget the old one's files and probes.
    pub fn reset(&mut self) {
        self.serial += 1;
        self.probing = 0;
        self.pending = Batch::default();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::edit::{AudioEdit, EditRequest};
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::model::GainDb;

    use super::*;
    use crate::ui::drag::tests::{gain, project_with_a_sound};

    fn file(project: &crate::session::Project) -> ProbedFile {
        // Re-import the item the fixture already holds: same probe data.
        let item = project.doc().library().items().next().unwrap();
        ProbedFile {
            path: "again.wav".into(),
            media: freecut_timeline::edit::NewMedia {
                path: item.path.clone(),
                info: item.info.clone(),
                fingerprint: None,
                display_name: None,
            },
        }
    }

    #[test]
    fn a_batch_finished_during_a_drag_waits_for_it_and_keeps_its_files() {
        let (mut project, clip) = project_with_a_sound();
        let mut imports = Imports::default();
        let media = imports.start(ImportRequest::Media);
        let audio = imports.start(ImportRequest::Audio);

        // The user drags the Volume slider while the probes run.
        let g = project.begin_gesture();
        let louder = EditRequest::SetClipAudio {
            seq: project.doc().root(),
            clips: vec![clip],
            edit: AudioEdit::Gain(GainDb::from_hundredths(-300)),
        };
        project.apply(&louder, Coalesce::Gesture(g)).unwrap();
        assert_eq!(imports.finished(media, Ok(file(&project))), None);
        assert_eq!(imports.finished(audio, Ok(file(&project))), None);
        assert!(imports.waiting());
        assert!(imports.take_ready(project.gesture_open()).is_none());

        // The drag goes on, undisturbed, and ends as one undo entry.
        let entries = project.editor.history().len();
        let quieter = EditRequest::SetClipAudio {
            seq: project.doc().root(),
            clips: vec![clip],
            edit: AudioEdit::Gain(GainDb::from_hundredths(-900)),
        };
        project.apply(&quieter, Coalesce::Gesture(g)).unwrap();
        project.end_gesture(g);
        assert_eq!(project.editor.history().len(), entries);

        // Then the whole batch is imported.
        let batch = imports.take_ready(project.gesture_open()).unwrap();
        assert_eq!((batch.media.len(), batch.audio.len()), (1, 1));
        let library = project.doc().library().len();
        project.import(batch.media).unwrap();
        project.import_audio(batch.audio).unwrap();
        assert_eq!(project.doc().library().len(), library + 2);
        assert!(!imports.waiting());
        project.undo().unwrap();
        project.undo().unwrap();
        project.undo().unwrap();
        assert_eq!(gain(&project, clip), GainDb::ZERO, "the drag was one entry");
    }

    #[test]
    fn probes_of_a_project_that_was_closed_are_dropped() {
        let (project, _) = project_with_a_sound();
        let mut imports = Imports::default();
        let old = imports.start(ImportRequest::Audio);
        let failing = imports.start(ImportRequest::Media);
        assert_eq!(imports.probing(), 2);

        // File > New Project before the probes finish.
        imports.reset();
        assert_eq!(imports.probing(), 0);
        assert_eq!(imports.finished(old, Ok(file(&project))), None);
        assert_eq!(
            imports.finished(failing, Err("Couldn't import".into())),
            None
        );
        assert!(!imports.waiting());
        assert!(imports.take_ready(false).is_none());

        // The new project's own probes still import.
        let new = imports.start(ImportRequest::Media);
        assert_eq!(imports.finished(new, Ok(file(&project))), None);
        let batch = imports.take_ready(false).unwrap();
        assert_eq!((batch.media.len(), batch.audio.len()), (1, 0));
    }

    #[test]
    fn a_failed_probe_is_reported_and_the_rest_of_the_batch_imports() {
        let (project, _) = project_with_a_sound();
        let mut imports = Imports::default();
        let bad = imports.start(ImportRequest::Media);
        let good = imports.start(ImportRequest::Media);
        assert_eq!(
            imports.finished(bad, Err("Couldn't import x.mp4".into())),
            Some("Couldn't import x.mp4".to_owned())
        );
        assert!(!imports.waiting(), "one probe is still running");
        imports.finished(good, Ok(file(&project)));
        assert_eq!(imports.take_ready(false).unwrap().media.len(), 1);
    }
}

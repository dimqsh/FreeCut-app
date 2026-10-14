//! Saved looks and adjustment clips in the app (CL-006, CL-026, CL-044).
//!
//! [`SavedLooks`] is what the Adjust tab's **Saved** group and the Look
//! lens's **Presets** select list: the usable looks of the library in app
//! data (`freecut_project::look`), oldest first, and the files that can't
//! be used, with their reason. The library is read, written and deleted
//! from on the job pool ([`load`], [`save`], [`delete`]); the UI thread
//! only swaps in the list a job returns.
//!
//! The edit helpers ([`apply_look`], [`apply_lut`], [`add_adjustment`])
//! turn a click in the Adjust tab into one undoable edit and the sentence
//! the status line shows.

use std::path::Path;
use std::sync::Arc;

use freecut_project::look::{LookError, LookFile, LookLibrary};
use freecut_timeline::edit::{AdjustSpan, ColorEdit, EditRequest};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{ColorStack, LutRef, LutSlot, Section, SectionSnapshot};

use crate::session::Project;
use crate::ui::look::section_label;

/// One usable saved look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedLook {
    /// The file's id in the library.
    pub id: String,
    pub name: String,
    /// What applying it copies.
    pub snapshot: Arc<SectionSnapshot>,
    /// The LUT it names, if any (for the missing-LUT badge).
    pub lut: Option<LutRef>,
}

impl SavedLook {
    /// "Light · Color · LUT": the sections it holds.
    pub fn summary(&self) -> String {
        sections_summary(&self.snapshot)
    }
}

/// The section names a snapshot holds, joined by " · ".
pub fn sections_summary(snapshot: &SectionSnapshot) -> String {
    snapshot
        .sections()
        .sections()
        .map(section_label)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The saved looks the UI lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedLooks {
    pub looks: Vec<SavedLook>,
    /// Files that can't be used: (id, why), CL-026's "unavailable".
    pub unavailable: Vec<(String, String)>,
    /// The library is still being read at startup.
    pub loading: bool,
}

impl Default for SavedLooks {
    fn default() -> Self {
        SavedLooks {
            looks: Vec::new(),
            unavailable: Vec::new(),
            loading: true,
        }
    }
}

impl SavedLooks {
    /// The list of `library`.
    pub fn of(library: &LookLibrary) -> SavedLooks {
        let mut out = SavedLooks {
            loading: false,
            ..SavedLooks::default()
        };
        for entry in library.entries() {
            match &entry.look {
                Ok(look) => out.looks.push(SavedLook {
                    id: entry.id.clone(),
                    name: look.name.clone(),
                    snapshot: Arc::new(look.snapshot.clone()),
                    lut: look.luts().into_iter().next(),
                }),
                Err(e) => out
                    .unavailable
                    .push((entry.id.clone(), unavailable_reason(e))),
            }
        }
        out
    }

    /// No library (no app-data folder): nothing to list, nothing loading.
    pub fn none() -> SavedLooks {
        SavedLooks {
            loading: false,
            ..SavedLooks::default()
        }
    }
}

/// CL-026's wording for a look file FreeCut can't use.
pub fn unavailable_reason(e: &LookError) -> String {
    match e {
        LookError::Newer(v) => format!("Made with a newer FreeCut (look format {v})"),
        other => format!("Can't be read: {other}"),
    }
}

/// Reads the library in `app_data` (a job).
pub fn load(app_data: &Path) -> Result<LookLibrary, String> {
    LookLibrary::in_app_data(app_data).map_err(|e| e.to_string())
}

/// Saves `look` into the library in `app_data` and returns the library
/// with it and the name it was saved under (a job).
pub fn save(app_data: &Path, look: LookFile, seed: u64) -> Result<(LookLibrary, String), String> {
    let mut library = LookLibrary::in_app_data(app_data).map_err(|e| e.to_string())?;
    let id = library.save(look, seed).map_err(|e| e.to_string())?;
    let name = library
        .entry(&id)
        .map(|e| e.display_name().to_owned())
        .unwrap_or_default();
    Ok((library, name))
}

/// Deletes look `id` from the library in `app_data` (a job).
pub fn delete(app_data: &Path, id: &str) -> Result<LookLibrary, String> {
    let mut library = LookLibrary::in_app_data(app_data).map_err(|e| e.to_string())?;
    library.delete(id).map_err(|e| e.to_string())?;
    Ok(library)
}

/// CL-044's sentence when nothing is selected.
pub const SELECT_FIRST: &str = "Select a clip first, or use + to add an adjustment clip";

/// The selected clips a look or a LUT can go on (video, images and
/// adjustment clips), in selection order.
pub fn graded_selection(project: &Project) -> Vec<ClipId> {
    let doc = project.doc();
    project
        .selection
        .iter()
        .copied()
        .filter(|id| doc.clip(*id).is_some_and(|c| c.content.takes_color()))
        .collect()
}

/// Why the selection can't take a look: nothing selected, or only titles
/// and sound.
fn require_graded(project: &Project) -> Result<Vec<ClipId>, String> {
    if project.selection.is_empty() {
        return Err(SELECT_FIRST.to_owned());
    }
    let clips = graded_selection(project);
    if clips.is_empty() {
        return Err("Looks and LUTs go on video, image and adjustment clips".to_owned());
    }
    Ok(clips)
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 clip".to_owned()
    } else {
        format!("{n} clips")
    }
}

/// Applies `look` to the selected clips (CL-026's apply, one undo entry);
/// returns the status sentence ("Look applied to 5 clips").
pub fn apply_look(project: &mut Project, look: &SavedLook) -> Result<String, String> {
    let clips = require_graded(project)?;
    let n = clips.len();
    let seq = project.doc().root();
    project.apply(
        &EditRequest::EditColor {
            seq,
            clips,
            edit: ColorEdit::ApplySections(Arc::clone(&look.snapshot)),
        },
        Coalesce::Never,
    )?;
    Ok(format!("Look applied to {}", count(n)))
}

/// Puts `lut` in the LUT slot of the selected clips (CL-017, one undo
/// entry); returns CL-044's sentence ("LUT added", "LUT replaced (was
/// Teal 01)").
pub fn apply_lut(
    project: &mut Project,
    lut: &LutRef,
    name_of: impl Fn(&LutRef) -> String,
) -> Result<String, String> {
    let clips = require_graded(project)?;
    let replaced: Option<LutRef> = clips
        .iter()
        .filter_map(|id| project.doc().clip(*id))
        .filter_map(|c| c.color.as_ref().and_then(|s| s.lut.as_ref()))
        .map(|slot| slot.lut.clone())
        .find(|old| old.hash != lut.hash);
    let seq = project.doc().root();
    project.apply(
        &EditRequest::EditColor {
            seq,
            clips,
            edit: ColorEdit::SetLut(Some(LutSlot::new(lut.clone()))),
        },
        Coalesce::Never,
    )?;
    Ok(match replaced {
        Some(old) => format!("LUT replaced (was {})", name_of(&old)),
        None => "LUT added".to_owned(),
    })
}

/// CL-006's default span: the selected clips' union, else 3 s from the
/// playhead.
pub fn adjust_span(project: &Project) -> AdjustSpan {
    if project.selection.is_empty() {
        AdjustSpan::AtPlayhead(project.playhead)
    } else {
        AdjustSpan::Selection(project.selection.clone())
    }
}

/// Adds an adjustment clip with CL-006's span and placement carrying
/// `color` (`None`: blank), selected afterward (one undo entry). Returns
/// its id.
pub fn add_adjustment(project: &mut Project, color: Option<ColorStack>) -> Result<ClipId, String> {
    let seq = project.doc().root();
    let span = adjust_span(project);
    project.apply(
        &EditRequest::AddAdjustmentClip {
            seq,
            span,
            color: color.map(Arc::new),
        },
        Coalesce::Never,
    )?;
    project
        .selection
        .first()
        .copied()
        .ok_or_else(|| "FreeCut couldn't add the adjustment clip".to_owned())
}

/// The stack an adjustment clip made from `look` carries.
pub fn stack_of_look(look: &SavedLook) -> ColorStack {
    look.snapshot.applied_to(&ColorStack::default())
}

/// The stack an adjustment clip made from a LUT carries.
pub fn stack_of_lut(lut: &LutRef) -> ColorStack {
    ColorStack::default().with_lut(Some(LutSlot::new(lut.clone())))
}

/// Whether `stack` has anything a look can hold (CL-026: saving a grade
/// with nothing changed is refused).
pub fn has_look_values(stack: &ColorStack) -> bool {
    Section::LOOK.iter().any(|s| !stack.section_is_default(*s))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_project::look::write_look;
    use freecut_timeline::edit::{InsertReq, InsertTarget, NewClip};
    use freecut_timeline::model::{
        ColorParamId, GeneratedKind, ProjectSettings, SectionMask, TextClipData, TrackKind,
    };
    use freecut_timeline::time::Tick;

    use super::*;

    fn project() -> Project {
        Project::new(ProjectSettings::new_default().unwrap()).unwrap()
    }

    fn warm_look() -> SavedLook {
        let stack = ColorStack::default()
            .with(ColorParamId::Temperature, 40)
            .with(ColorParamId::Exposure, 10);
        SavedLook {
            id: "a".into(),
            name: "Warm".into(),
            snapshot: Arc::new(SectionSnapshot::new(
                &stack,
                SectionMask::of(&[Section::Light, Section::Color]),
            )),
            lut: None,
        }
    }

    fn add_text(p: &mut Project) -> ClipId {
        let seq = p.doc().root();
        p.apply(
            &EditRequest::Insert(InsertReq {
                seq,
                target: InsertTarget::NewTrack,
                at: Tick::ZERO,
                clips: vec![NewClip::text(
                    TextClipData::with_content("Hi"),
                    Tick::from_secs(2).unwrap(),
                )],
            }),
            Coalesce::Never,
        )
        .unwrap();
        p.selection[0]
    }

    #[test]
    fn an_adjustment_clip_lands_at_the_playhead_and_is_selected() {
        let mut p = project();
        p.playhead = Tick::from_secs(1).unwrap();
        let clip = add_adjustment(&mut p, Some(stack_of_look(&warm_look()))).unwrap();
        assert_eq!(p.selection, [clip]);
        let doc = p.doc().clone();
        let c = doc.clip(clip).unwrap();
        assert_eq!(c.start(), Tick::from_secs(1).unwrap());
        assert_eq!(c.duration(), Tick::from_secs(3).unwrap());
        assert_eq!(
            c.content,
            freecut_timeline::model::ClipContent::Generated {
                kind: GeneratedKind::Adjustment
            }
        );
        assert_eq!(c.color.as_ref().unwrap().basic.temperature, 40);
        let (_, track) = doc.sequence(doc.root()).unwrap().find_clip(clip).unwrap();
        assert_eq!(track.kind, TrackKind::Adjustment);
        assert_eq!(p.editor.history().len(), 1, "one undo step");
    }

    #[test]
    fn looks_and_luts_need_a_clip_that_takes_colour() {
        let mut p = project();
        assert_eq!(
            apply_look(&mut p, &warm_look()),
            Err(SELECT_FIRST.to_owned())
        );
        add_text(&mut p);
        assert!(apply_look(&mut p, &warm_look()).is_err(), "a title alone");
        let adj = add_adjustment(&mut p, None).unwrap();
        assert_eq!(
            apply_look(&mut p, &warm_look()).unwrap(),
            "Look applied to 1 clip"
        );
        let s = p.doc().clip(adj).unwrap().color.clone().unwrap();
        assert_eq!((s.basic.temperature, s.basic.exposure), (40, 10));
        let teal = LutRef::new([1; 16], "teal.cube");
        let name = |l: &LutRef| l.file_name.clone();
        assert_eq!(apply_lut(&mut p, &teal, name).unwrap(), "LUT added");
        let night = LutRef::new([2; 16], "night.cube");
        assert_eq!(
            apply_lut(&mut p, &night, name).unwrap(),
            "LUT replaced (was teal.cube)"
        );
        assert_eq!(p.editor.history().len(), 5);
    }

    #[test]
    fn the_list_names_unusable_files_with_their_reason() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path();
        let (library, name) = save(
            tmp,
            LookFile::new(
                "Warm",
                &stack_of_look(&warm_look()),
                SectionMask::of(&[Section::Light, Section::Color]),
                5,
            ),
            7,
        )
        .unwrap();
        assert_eq!(name, "Warm");
        let folder = library.dir().to_path_buf();
        let mut newer = write_look(&LookFile::new(
            "Later",
            &ColorStack::default(),
            SectionMask::NONE,
            9,
        ));
        let text = String::from_utf8(newer.clone())
            .unwrap()
            .replace("\"look_version\": 1", "\"look_version\": 4");
        newer = text.into_bytes();
        std::fs::write(folder.join("later.fclook"), newer).unwrap();
        let list = SavedLooks::of(&load(tmp).unwrap());
        assert_eq!(list.looks.len(), 1);
        assert_eq!(list.looks[0].summary(), "Light · Color");
        assert_eq!(
            list.unavailable,
            [(
                "later".to_owned(),
                "Made with a newer FreeCut (look format 4)".to_owned()
            )]
        );
        let id = list.looks[0].id.clone();
        let after = SavedLooks::of(&delete(tmp, &id).unwrap());
        assert!(after.looks.is_empty());
    }
}

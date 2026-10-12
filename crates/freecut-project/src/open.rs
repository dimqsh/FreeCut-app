//! The open pipeline (PM-013): read → parse → envelope check → migrate →
//! typed decode → map → repair → validate.
//!
//! Everything that must not be undoable happens here, before the document
//! reaches `Editor` (README §5.11). The result is either a valid document or
//! an error that says what is wrong and where (a JSON path).

use core::num::NonZeroU64;
use std::io::Read;
use std::path::{Path, PathBuf};

use freecut_timeline::edit::Limits;
use freecut_timeline::ids::{ClipId, IdAlloc, MAX_ID, SeqId, TrackId};
use freecut_timeline::model::{
    ClipContent, Document, InvariantViolation, Library, MediaItem, MediaSource, StreamSel,
    ValidateCtx,
};
use serde_json::Value;

use crate::export::ExportSettings;
use crate::extras::Extras;
use crate::format::map::{Parts, SeqParts};
use crate::format::{self, FORMAT_MAGIC};
use crate::migrate::{self, CURRENT_FORMAT_VERSION, MigrationError};
use crate::repair::{
    MainRepair, clear_broken_attachments, clear_stray_links, dissolve_small_relations,
    repair_main_track,
};
use crate::ui::UiState;

/// Why a project couldn't be opened.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OpenError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The bytes are not a FreeCut project at all (another file type, or a
    /// JSON file without the project envelope).
    #[error("not a FreeCut project")]
    NotAProject,
    /// The file is damaged: it holds nothing but whitespace or zero bytes,
    /// which is what a lost write looks like on some file systems. Like
    /// [`OpenError::Json`] this is the user's project, not a foreign file,
    /// so the app offers the damaged-file recovery (PM-013, PM-016).
    #[error("the project file is damaged: it is empty")]
    Empty,
    /// The file is damaged (a JSON syntax error, for example after a
    /// truncated write) or doesn't match the schema. `path` locates the
    /// problem (`document.sequences[0].visual[1].clips[3].range`, or `.` for
    /// the whole file).
    #[error("the project file is damaged at {path}: {message}")]
    Json { path: String, message: String },
    /// A value parses but is refused, such as a frame rate whose frame
    /// isn't a whole number of ticks (TL-001). Never rounded or guessed.
    #[error("unsupported value at {path}: {message}")]
    BadValue { path: String, message: String },
    /// Written by a newer FreeCut. The app may offer to open it read-only
    /// (PM-021).
    #[error("made with a newer FreeCut (format {found}; this build reads up to {supported})")]
    NewerVersion {
        found: u32,
        supported: u32,
        /// The writer named in the file (`"freecut 2.3.0"`), if readable.
        written_by: Option<String>,
    },
    #[error(transparent)]
    Migration(#[from] MigrationError),
    /// The document breaks model invariants that load-time repair can't fix.
    #[error("the project breaks {} rule(s), first: {}", .0.len(), first_violation(.0))]
    Invalid(Vec<InvariantViolation>),
    /// Larger than any real project; refused before reading it whole.
    #[error("the file is {len} bytes; projects are at most {limit} bytes")]
    TooLarge { len: u64, limit: u64 },
}

/// The largest project file [`open`] reads (PM-021: a project holds no
/// binary data, so even huge timelines stay far below this).
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

fn first_violation(violations: &[InvariantViolation]) -> String {
    violations
        .first()
        .map_or_else(String::new, ToString::to_string)
}

/// Something the open pipeline noticed or fixed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenWarning {
    /// The file was in an older format and was upgraded in memory. The file
    /// on disk is unchanged until the next save.
    Migrated { from: u32, to: u32 },
    /// The stored id counter was not above every id in the file; it was
    /// raised so no id is ever reused.
    IdsNextRaised { stored: u64, raised_to: u64 },
    /// `visual[0]` was not a video track; the lowest video track (`track`,
    /// found at `from_index`) was moved there, and any other video tracks
    /// below the upper-band tracks with it, keeping the band order (TL-004).
    MainTrackPromoted {
        seq: SeqId,
        track: TrackId,
        from_index: usize,
    },
    /// The sequence had no video track; an empty main track was added.
    MainTrackCreated { seq: SeqId, track: TrackId },
    /// Groups and links that fewer than two clips of the sequence belonged
    /// to were dissolved (TL-011). A video whose detached sound lost its
    /// audio clips stays detached.
    RelationsDissolved {
        seq: SeqId,
        groups: usize,
        links: usize,
    },
    /// The file has keys this build doesn't know. They are kept and written
    /// back on save (PM-021).
    UnknownFields { objects: usize },
    /// Clips that can't belong to a link (anything but the detached videos
    /// and audio clips of a detached sound, TL-030) carried one; it was
    /// cleared, so restoring that sound never deletes them.
    StrayLinksCleared { seq: SeqId, clips: usize },
    /// Transform, text, keyframe or animation values outside their ranges
    /// (a hand-edited file) were clamped into range on `clips` clips, as an
    /// edit would have clamped them (TR-001, TX-004, TX-005, TX-008 to
    /// TX-016, TX-055, FX-018): angles wrapped into one turn, gradient
    /// stops sorted, a background box switched off where a bubble was on
    /// beside it (TX-054), overlap and intensity capped, Pace held in
    /// range, a `hold` easing override dropped.
    ValuesClamped { clips: usize },
    /// Volume or property keyframes out of time order (a hand-edited file)
    /// were put in order on `clips` clips (AU-008, TR-027).
    KeyframesSorted { clips: usize },
    /// Media clips chose an audio stream their media doesn't have (after a
    /// relink to a file with fewer streams, or a hand edit). The choice is
    /// kept and the media's default stream plays (AU-038).
    MissingStreams { clips: Vec<ClipId> },
    /// Marker lists (a timeline's or a clip's) were out of order, or held a
    /// name or note longer than TL-037's limits (a hand-edited file): `lists`
    /// lists were put in order and their long texts shortened, as an edit
    /// would have.
    MarkersFixed { lists: usize },
    /// Items were attached to clips they can't follow (TL-029): to a clip
    /// that is not on the main track (after a hand edit, or after the main
    /// track was repaired), from the main track itself, or in a sequence
    /// with *Follow main track* off. Their attachments were cleared; the
    /// items stay where they are.
    AttachmentsCleared { seq: SeqId, clips: usize },
}

impl OpenWarning {
    /// Whether this warning means the document differs from the file, so
    /// the project should open with unsaved changes.
    pub fn is_repair(&self) -> bool {
        match self {
            OpenWarning::IdsNextRaised { .. }
            | OpenWarning::MainTrackPromoted { .. }
            | OpenWarning::MainTrackCreated { .. }
            | OpenWarning::RelationsDissolved { .. }
            | OpenWarning::StrayLinksCleared { .. }
            | OpenWarning::ValuesClamped { .. }
            | OpenWarning::KeyframesSorted { .. }
            | OpenWarning::MarkersFixed { .. }
            | OpenWarning::AttachmentsCleared { .. } => true,
            OpenWarning::Migrated { .. }
            | OpenWarning::UnknownFields { .. }
            | OpenWarning::MissingStreams { .. } => false,
        }
    }
}

/// A successfully opened project.
#[derive(Debug, Clone)]
pub struct Opened {
    /// A valid document (it passed `Document::validate`).
    pub document: Document,
    /// The id allocator to resume: above every id in the document.
    pub ids: IdAlloc,
    /// Unknown keys to write back on save.
    pub extras: Extras,
    /// The settings of the last export started from this project (EX-021);
    /// `None` before the first one. Give them back in
    /// [`crate::SaveRequest::export`].
    pub export: Option<ExportSettings>,
    /// The project's UI state (AU-012's solo set), holding only tracks of
    /// the document that carry sound. Not part of the document and never
    /// undoable; give it back in [`crate::SaveRequest::ui`].
    pub ui: UiState,
    pub warnings: Vec<OpenWarning>,
    /// The document was repaired: open it with unsaved changes (PM-013), and
    /// keep a backup of the original on the first save.
    pub needs_save: bool,
    /// The writer named in the file (`"freecut 0.1.0"`).
    pub written_by: String,
}

impl Opened {
    /// Whether the first save should keep a copy of the file as it was
    /// (`SaveRequest::backup`): after a repair (PM-013), and after an
    /// upgrade from an older format, which the builds that wrote it can't
    /// open once it is saved again (PM-021).
    pub fn backup_on_save(&self) -> bool {
        self.needs_save
            || self
                .warnings
                .iter()
                .any(|w| matches!(w, OpenWarning::Migrated { .. }))
    }
}

/// Opens the project file at `path`. Blocking; call it from a worker thread.
pub fn open(path: &Path) -> Result<Opened, OpenError> {
    let io_err = |source| OpenError::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = std::fs::File::open(path).map_err(io_err)?;
    let len = file.metadata().map_err(io_err)?.len();
    if len > MAX_FILE_BYTES {
        return Err(OpenError::TooLarge {
            len,
            limit: MAX_FILE_BYTES,
        });
    }
    // `take` also guards against a file that grows while it is read.
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_err)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(OpenError::TooLarge {
            len: bytes.len() as u64,
            limit: MAX_FILE_BYTES,
        });
    }
    decode(&bytes)
}

/// Opens a project from its file bytes.
pub fn decode(bytes: &[u8]) -> Result<Opened, OpenError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    // Nothing but blanks and zero bytes: a lost write, so a damaged project
    // rather than a foreign file (project-format open pipeline, step 1).
    let blank = |b: &&u8| b.is_ascii_whitespace() || **b == 0;
    let Some(first) = bytes.iter().find(|b| !blank(b)) else {
        return Err(OpenError::Empty);
    };
    // Every project file is a JSON object. Anything else (an image, an MLT
    // or OTIO file, a text note) is not a damaged project but a wrong file.
    // Zeros before the `{` are damage too; parsing reports them.
    if *first != b'{' {
        return Err(OpenError::NotAProject);
    }
    let value = format::parse(bytes)?;
    let version = envelope_version(&value)?;
    let migrated = migrate::migrate(value, version)?;
    let mut warnings = Vec::new();
    if migrated.changed() {
        warnings.push(OpenWarning::Migrated {
            from: migrated.from,
            to: CURRENT_FORMAT_VERSION,
        });
    }
    let parts = format::decode(migrated.document)?;
    finish(parts, warnings)
}

/// Checks the envelope and returns the file's format version.
fn envelope_version(value: &Value) -> Result<u32, OpenError> {
    let object = value.as_object().ok_or(OpenError::NotAProject)?;
    if object.get("format").and_then(Value::as_str) != Some(FORMAT_MAGIC) {
        return Err(OpenError::NotAProject);
    }
    let version = object
        .get("format_version")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| OpenError::Json {
            path: "format_version".into(),
            message: "expected a format version (a positive integer)".into(),
        })?;
    if version > CURRENT_FORMAT_VERSION {
        return Err(OpenError::NewerVersion {
            found: version,
            supported: CURRENT_FORMAT_VERSION,
            written_by: object.get("app").and_then(Value::as_str).map(String::from),
        });
    }
    Ok(version)
}

/// Raises the id counter, repairs, builds and validates the document.
fn finish(parts: Parts, mut warnings: Vec<OpenWarning>) -> Result<Opened, OpenError> {
    let Parts {
        app,
        ids_next: stored,
        root,
        settings,
        library,
        sequences,
        export,
        ui,
        extras,
        clamped_clips,
        sorted_clips,
        fixed_marker_lists,
    } = parts;
    if clamped_clips > 0 {
        warnings.push(OpenWarning::ValuesClamped {
            clips: clamped_clips,
        });
    }
    if sorted_clips > 0 {
        warnings.push(OpenWarning::KeyframesSorted {
            clips: sorted_clips,
        });
    }
    if fixed_marker_lists > 0 {
        warnings.push(OpenWarning::MarkersFixed {
            lists: fixed_marker_lists,
        });
    }

    // Ids are never reused: the counter must be above every id in the file.
    let max = max_id(&library, &sequences);
    let mut next = stored;
    if max >= stored.get() {
        // max ≤ MAX_ID (checked while mapping), so max + 1 can't overflow.
        next = NonZeroU64::new(max.saturating_add(1)).unwrap_or(NonZeroU64::MAX);
        warnings.push(OpenWarning::IdsNextRaised {
            stored: stored.get(),
            raised_to: next.get(),
        });
    }

    let mut built = Vec::with_capacity(sequences.len());
    for mut seq in sequences {
        let strays = clear_stray_links(&mut seq.visual, &mut seq.audio);
        if strays > 0 {
            warnings.push(OpenWarning::StrayLinksCleared {
                seq: seq.id,
                clips: strays,
            });
        }
        let dissolved = dissolve_small_relations(&mut seq.visual, &mut seq.audio);
        if dissolved.any() {
            warnings.push(OpenWarning::RelationsDissolved {
                seq: seq.id,
                groups: dissolved.groups,
                links: dissolved.links,
            });
        }
        let fresh = || {
            if next.get() > MAX_ID {
                return Err(OpenError::Invalid(vec![InvariantViolation::MainTrack {
                    seq: seq.id,
                }]));
            }
            let id = TrackId::from_raw(next);
            next = next.saturating_add(1);
            Ok(id)
        };
        match repair_main_track(&mut seq.visual, fresh)? {
            None => {}
            Some(MainRepair::Promoted { track, from }) => {
                warnings.push(OpenWarning::MainTrackPromoted {
                    seq: seq.id,
                    track,
                    from_index: from,
                });
            }
            Some(MainRepair::Created { track }) => {
                warnings.push(OpenWarning::MainTrackCreated { seq: seq.id, track });
            }
        }
        let unattached = clear_broken_attachments(seq.flags, &mut seq.visual, &mut seq.audio);
        if unattached > 0 {
            warnings.push(OpenWarning::AttachmentsCleared {
                seq: seq.id,
                clips: unattached,
            });
        }
        built.push(seq.into_sequence());
    }

    let library = Library::from_parts(library)
        .map_err(|id| OpenError::Invalid(vec![InvariantViolation::DuplicateId(id.get())]))?;
    let document = Document::from_parts(root, settings, library, built)
        .map_err(|v| OpenError::Invalid(vec![v]))?;
    let ctx = ValidateCtx {
        limits: Limits::default(),
        id_high_water: Some(next),
    };
    document.validate(&ctx).map_err(OpenError::Invalid)?;
    let missing = missing_streams(&document);
    if !missing.is_empty() {
        warnings.push(OpenWarning::MissingStreams { clips: missing });
    }
    // Solo of a removed track is cleared (AU-012); UI state is no repair.
    let ui = ui.retained_for(&document);

    if !extras.is_empty() {
        tracing::warn!(
            objects = extras.len(),
            "project has keys this build doesn't know; they will be kept on save"
        );
        warnings.push(OpenWarning::UnknownFields {
            objects: extras.len(),
        });
    }
    let needs_save = warnings.iter().any(OpenWarning::is_repair);
    if needs_save {
        tracing::warn!(?warnings, "project was repaired while opening");
    }
    Ok(Opened {
        document,
        ids: IdAlloc::starting_at(next),
        extras,
        export,
        ui,
        warnings,
        needs_save,
        written_by: app,
    })
}

/// Media clips whose chosen audio stream is not in their media (AU-038), in
/// sequence, track and clip order.
fn missing_streams(document: &Document) -> Vec<ClipId> {
    let mut out = Vec::new();
    for seq in document.sequences() {
        for track in seq.visual_tracks().iter().chain(seq.audio_tracks()) {
            for clip in track.clips() {
                if let ClipContent::Media {
                    asset,
                    stream: Some(StreamSel::Index(index)),
                    ..
                } = clip.content
                    && document
                        .library()
                        .get(asset)
                        .is_some_and(|item| item.info.audio_stream(index).is_none())
                {
                    out.push(clip.id);
                }
            }
        }
    }
    out
}

/// The largest id in the parts: media, sequences, tracks, clips, markers,
/// groups and links share one id space.
///
/// Ids that only remain as references count too, so they are never handed
/// out again: the link of a detached video whose audio clips are gone (it
/// still names that sound, TL-030), and the parent of an extracted audio
/// item that was removed (a new item with that id would look like its
/// parent, AU-004).
fn max_id(library: &[std::sync::Arc<MediaItem>], sequences: &[SeqParts]) -> u64 {
    let mut max = 0;
    for item in library {
        max = max.max(item.id.get());
        if let MediaSource::AudioOf { source, .. } = item.source {
            max = max.max(source.get());
        }
    }
    let markers = |list: &freecut_timeline::model::Markers| {
        list.iter().map(|m| m.id.get()).max().unwrap_or(0)
    };
    for seq in sequences {
        max = max.max(seq.id.get()).max(markers(&seq.markers));
        for track in seq.visual.iter().chain(&seq.audio) {
            max = max.max(track.id.get());
            for clip in track.clips() {
                max = max
                    .max(clip.id.get())
                    .max(markers(&clip.markers))
                    .max(clip.group.map_or(0, |g| g.get()))
                    .max(clip.link.map_or(0, |l| l.get()))
                    .max(clip.audio.detached().map_or(0, |l| l.get()));
            }
        }
    }
    max
}

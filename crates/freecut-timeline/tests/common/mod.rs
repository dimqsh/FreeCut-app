//! Shared test fixture: an editor at 25 fps whose library holds a few known
//! media items, helpers in frames, and a compact text rendering of a
//! sequence.
#![allow(dead_code)]

use core::num::NonZeroU64;
use std::sync::Arc;

use freecut_timeline::edit::{
    EditCtx, EditError, EditRequest, InsertReq, InsertTarget, Limits, NewClip, NewMedia,
};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, IdAlloc, MediaId, SeqId, TrackId};
use freecut_timeline::model::{
    AudioStreamInfo, ClipContent, Document, Library, MediaInfo, MediaItem, MediaKind, MediaOrigin,
    MediaSource, ProjectSettings, Sequence, StoredPath, TrackSlot, ValidateCtx, VideoStreamInfo,
};
use freecut_timeline::time::{FrameRate, Tick, TimeRange};
use freecut_timeline::{CommitMeta, Committed, Editor};
use pretty_assertions::assert_eq;

const fn media(id: u64) -> MediaId {
    match NonZeroU64::new(id) {
        Some(n) => MediaId::from_raw(n),
        None => panic!("media ids are non-zero"),
    }
}

/// 100 frames of video with audio.
pub const VIDEO_100: MediaId = media(3);
/// 50 frames of video with audio.
pub const VIDEO_50: MediaId = media(4);
/// 200 frames of audio only.
pub const AUDIO_200: MediaId = media(5);
/// Not in the library.
pub const PROBING: MediaId = media(99);

pub fn rate() -> FrameRate {
    FrameRate::new(25, 1).unwrap()
}

/// `n` frames at 25 fps.
pub fn f(n: i64) -> Tick {
    rate().frames(n).unwrap()
}

pub fn settings() -> ProjectSettings {
    ProjectSettings {
        frame_rate: rate(),
        ..ProjectSettings::new_default().unwrap()
    }
}

/// Probe-like info for `frames` of media: video (with audio) or audio only.
pub fn media_info(frames: i64, video: bool) -> MediaInfo {
    media_info_with(frames, video, true)
}

/// Probe-like info for `frames` of media, with or without a picture and
/// with or without sound.
pub fn media_info_with(frames: i64, video: bool, sound: bool) -> MediaInfo {
    let duration = f(frames);
    let audio = AudioStreamInfo {
        index: u32::from(video),
        codec: "aac".into(),
        sample_rate: 48_000,
        channels: 2,
        start: Tick::ZERO,
        duration,
        duration_exact: true,
        default: false,
        language: None,
        title: None,
        layout: Some("stereo".into()),
    };
    let video = video.then(|| VideoStreamInfo {
        index: 0,
        codec: "h264".into(),
        width: 1920,
        height: 1080,
        sar: (1, 1),
        rotation: Default::default(),
        mirrored: false,
        rate: None,
        variable_rate: false,
        has_alpha: false,
        bit_depth: 8,
        start: Tick::ZERO,
        duration,
    });
    let default_video = video.is_some().then_some(0);
    MediaInfo {
        container: "mp4".into(),
        default_video,
        duration,
        video: video.into_iter().collect(),
        audio: if sound { vec![audio] } else { Vec::new() },
        still: None,
    }
}

/// Probe-like info for `frames` of media with `streams` audio streams
/// (container indices after the video stream, if any); stream `default`
/// (a place among the audio streams) is flagged default, when given.
pub fn media_info_streams(
    frames: i64,
    video: bool,
    streams: u32,
    default: Option<usize>,
) -> MediaInfo {
    let mut info = media_info_with(frames, video, streams > 0);
    if let Some(first) = info.audio.first().cloned() {
        info.audio = (0..streams)
            .map(|i| AudioStreamInfo {
                index: first.index + i,
                title: Some(format!("Mic {}", i + 1)),
                language: (i == 0).then(|| "eng".into()),
                default: default == Some(i as usize),
                ..first.clone()
            })
            .collect();
    }
    info
}

fn item(id: MediaId, frames: i64, video: bool) -> Arc<MediaItem> {
    Arc::new(MediaItem {
        id,
        source: MediaSource::File,
        origin: MediaOrigin::Imported,
        path: StoredPath {
            absolute: format!("/media/{}.mp4", id.get()),
            relative: None,
        },
        info: media_info(frames, video),
        fingerprint: None,
        display_name: None,
    })
}

/// The starting document: sequence 1 with main track 2, and media 3–5.
pub fn starting_document() -> Document {
    let library = Library::from_parts(vec![
        item(VIDEO_100, 100, true),
        item(VIDEO_50, 50, true),
        item(AUDIO_200, 200, false),
    ])
    .unwrap();
    let seq = SeqId::from_raw(NonZeroU64::new(1).unwrap());
    let main = TrackId::from_raw(NonZeroU64::new(2).unwrap());
    Document::from_parts(
        seq,
        settings(),
        library,
        vec![Arc::new(Sequence::new(seq, main))],
    )
    .unwrap()
}

pub struct Fixture {
    pub editor: Editor,
}

impl Fixture {
    pub fn new() -> Fixture {
        Fixture::with_limits(HistoryLimits::default())
    }

    pub fn with_limits(limits: HistoryLimits) -> Fixture {
        Fixture {
            editor: Editor::open(starting_document(), IdAlloc::default(), limits).unwrap(),
        }
    }

    pub fn ctx(&self) -> EditCtx {
        EditCtx {
            limits: Limits::default(),
        }
    }

    pub fn doc(&self) -> &Document {
        self.editor.document()
    }

    pub fn seq_id(&self) -> SeqId {
        self.doc().root()
    }

    pub fn seq(&self) -> &Sequence {
        self.doc().sequence(self.seq_id()).unwrap()
    }

    pub fn main(&self) -> TrackId {
        self.seq().main_track().unwrap().id
    }

    pub fn track_at(&self, slot: TrackSlot) -> TrackId {
        self.seq().track_at(slot).unwrap().id
    }

    /// Plans and commits `req`, checking the undo contract on the way:
    /// the plan doesn't touch the input, the result validates, undo restores
    /// the original exactly and redo reproduces the result.
    pub fn apply(&mut self, req: EditRequest) -> Result<Committed, EditError> {
        let before = self.doc().clone();
        let plan = self.editor.plan(&self.ctx(), &req)?;
        assert_eq!(self.doc(), &before, "planning must not mutate the document");
        let committed = self.editor.commit(plan, CommitMeta::default()).unwrap();
        self.assert_valid();
        if matches!(committed, Committed::Applied { .. }) {
            let after = self.doc().clone();
            self.editor.undo().unwrap();
            assert_eq!(self.doc(), &before, "undo restores the original exactly");
            self.editor.redo().unwrap();
            assert_eq!(self.doc(), &after, "redo reproduces the result");
        } else {
            assert_eq!(self.doc(), &before, "a no-op changes nothing");
        }
        Ok(committed)
    }

    pub fn assert_valid(&self) {
        let ctx = ValidateCtx {
            limits: Limits::default(),
            id_high_water: Some(self.editor.ids().next()),
        };
        if let Err(errors) = self.doc().validate(&ctx) {
            panic!("invalid document: {errors:#?}\n{}", render(self.seq()));
        }
    }

    /// Inserts media clips (`(media, src_in, len)` in frames) on `target` at
    /// frame `at` and returns the new ids.
    pub fn insert(
        &mut self,
        target: InsertTarget,
        at: i64,
        clips: &[(MediaId, i64, i64)],
    ) -> Vec<ClipId> {
        let clips = clips
            .iter()
            .map(|&(asset, src, len)| {
                let kind = if asset == AUDIO_200 {
                    MediaKind::Audio
                } else {
                    MediaKind::Video
                };
                NewClip::media(asset, kind, TimeRange::new(f(src), f(len)).unwrap())
            })
            .collect();
        let req = EditRequest::Insert(InsertReq {
            seq: self.seq_id(),
            target,
            at: f(at),
            clips,
        });
        match self.apply(req).unwrap() {
            Committed::Applied { selection, .. } => selection.unwrap(),
            Committed::NoOp => panic!("insert was a no-op"),
        }
    }

    pub fn render(&self) -> String {
        render(self.seq())
    }

    /// Imports a probed file at `path` and returns its id.
    pub fn import(&mut self, path: &str, info: MediaInfo) -> MediaId {
        let req = EditRequest::ImportMedia(vec![NewMedia {
            path: StoredPath {
                absolute: path.into(),
                relative: None,
            },
            info,
            fingerprint: None,
            display_name: None,
        }]);
        let plan = self.editor.plan(&self.ctx(), &req).unwrap();
        let id = plan.imported()[0];
        self.editor.commit(plan, CommitMeta::default()).unwrap();
        self.assert_valid();
        id
    }

    /// Turns *Follow main track* off (TL-029), for tests about items that
    /// keep their place when main-track clips move.
    pub fn follow_off(&mut self) {
        let seq = self.seq_id();
        self.apply(EditRequest::SetFollowMain { seq, on: false })
            .unwrap();
    }

    /// Changes the sequence frame rate without conforming (clips keep their
    /// ticks and may sit off the new grid).
    pub fn set_rate(&mut self, rate: FrameRate) {
        let settings = ProjectSettings {
            frame_rate: rate,
            ..*self.doc().settings()
        };
        self.apply(EditRequest::SetProjectSettings {
            settings,
            conform: false,
        })
        .unwrap();
    }
}

/// One line per track, top to bottom: `V1: 6@0+10 7@20+5`, positions and
/// lengths in frames, `*` marks the main track, `L` locked tracks.
pub fn render(seq: &Sequence) -> String {
    let frame = rate().frame_ticks().raw();
    let mut lines = Vec::new();
    let mut visual: Vec<_> = seq.visual_tracks().iter().enumerate().collect();
    visual.reverse();
    let fmt_track = |name: String, t: &freecut_timeline::model::Track| {
        let clips: Vec<String> = t
            .clips()
            .iter()
            .map(|c| {
                let src = match &c.content {
                    ClipContent::Media { source, .. } => {
                        format!("[{}]", source.start().raw() / frame)
                    }
                    _ => String::new(),
                };
                format!(
                    "{}@{}+{}{}",
                    c.id.get(),
                    c.start().raw() / frame,
                    c.duration().raw() / frame,
                    src
                )
            })
            .collect();
        let lock = if t.is_locked() { "L" } else { "" };
        format!("{name}{lock}: {}", clips.join(" "))
            .trim_end()
            .to_string()
    };
    for (i, t) in visual {
        let name = if i == 0 {
            "V0*".to_string()
        } else {
            format!("V{i}")
        };
        lines.push(fmt_track(name, t));
    }
    for (i, t) in seq.audio_tracks().iter().enumerate() {
        lines.push(fmt_track(format!("A{i}"), t));
    }
    lines.join("\n")
}

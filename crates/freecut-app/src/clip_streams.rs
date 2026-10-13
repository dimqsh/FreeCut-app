//! Files with several audio streams (AU-038) and linked sound that slipped
//! out of sync (TL-030), seen from the app: the Sound source picker, the
//! "Add stream as clip" menu, the out-of-sync badge and Resync. Pure
//! functions of the document (plus a dry-run plan for the menu); the
//! panels draw their results and the app runs the commands.

use freecut_timeline::edit::{EditError, EditRequest, SyncOffset, stream_doubled_by, sync_offset};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::{ClipId, SeqId};
use freecut_timeline::model::{
    AudioStreamInfo, Clip, ClipContent, Document, EmbeddedAudio, MediaKind, MediaSource, StreamSel,
};
use freecut_timeline::time::Tick;
use freecut_timeline::{Committed, Editor};

use crate::clip_sound::{SoundCommand, sound_command};
use crate::session::{Project, describe_edit_error};

// -------------------------------------------------------------- labels

/// A stream's name in pickers and menus (AU-038): its number in the file's
/// list of audio streams, then the language, title and channel layout the
/// file gives ("Stream 2 · eng · Commentary · 5.1"); without a named layout,
/// "mono", "stereo" or the channel count.
pub fn stream_label(ordinal: usize, stream: &AudioStreamInfo) -> String {
    let mut parts = vec![format!("Stream {}", ordinal + 1)];
    parts.extend(stream.language.iter().cloned());
    parts.extend(stream.title.iter().cloned());
    parts.push(
        stream
            .layout
            .clone()
            .unwrap_or_else(|| match stream.channels {
                1 => "mono".to_owned(),
                2 => "stereo".to_owned(),
                n => format!("{n} ch"),
            }),
    );
    parts.join(" · ")
}

pub const ALL_MIXED: &str = "All streams mixed";

/// One entry of the Sound source picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChoice {
    pub label: String,
    /// What picking it asks for: `None` follows the media's default stream.
    pub sel: Option<StreamSel>,
    /// Why it can't be picked: the stream already plays on a linked clip,
    /// so picking it would double it (AU-038). The entry shows disabled
    /// with this on hover.
    pub blocked: Option<&'static str>,
}

/// The Sound source picker of a clip (AU-038).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPicker {
    pub choices: Vec<StreamChoice>,
    /// The choice that plays now.
    pub current: usize,
    /// Why the picker can't be used, if it can't.
    pub disabled: Option<&'static str>,
}

/// The picker for a media clip whose file has audio streams; `None` for
/// other clips. The file's default stream is offered as "follow the
/// default" (`None`), every other stream by index, and "All streams mixed"
/// when there are several.
pub fn stream_picker(doc: &Document, clip: ClipId) -> Option<StreamPicker> {
    let c = doc.clip(clip)?;
    let ClipContent::Media { asset, stream, .. } = &c.content else {
        return None;
    };
    let info = &doc.library().get(*asset)?.info;
    if info.audio.is_empty() {
        return None;
    }
    let default = info.default_audio().map(|a| a.index);
    let mut choices: Vec<StreamChoice> = info
        .audio
        .iter()
        .enumerate()
        .map(|(i, a)| StreamChoice {
            label: stream_label(i, a),
            sel: (Some(a.index) != default).then_some(StreamSel::Index(a.index)),
            blocked: None,
        })
        .collect();
    if info.audio.len() > 1 {
        choices.push(StreamChoice {
            label: ALL_MIXED.to_owned(),
            sel: Some(StreamSel::AllMixed),
            blocked: None,
        });
    }
    let current = match stream {
        Some(StreamSel::AllMixed) if info.audio.len() > 1 => Some(choices.len() - 1),
        Some(StreamSel::Index(index)) => info.audio.iter().position(|a| a.index == *index),
        _ => None,
    }
    .or_else(|| info.audio.iter().position(|a| Some(a.index) == default))
    .unwrap_or(0);
    let locked = doc
        .sequence(doc.root())
        .and_then(|s| s.find_clip(clip))
        .is_some_and(|(_, t)| t.header.flags.locked);
    let disabled = match c.audio {
        EmbeddedAudio::Detached(_) => Some(
            "The sound of this clip is detached: its audio clips play the stream; restore the \
             audio to choose here",
        ),
        EmbeddedAudio::None => Some("This clip has no sound"),
        EmbeddedAudio::Active(_) if locked => Some("The clip's track is locked"),
        EmbeddedAudio::Active(_) => None,
    };
    let mut picker = StreamPicker {
        choices,
        current,
        disabled,
    };
    block_doubling(doc, &[clip], &mut picker);
    Some(picker)
}

/// The picker the clip context menu shows for a selection: the first
/// selected clip that has one (the choice goes to every selected clip), its
/// entries blocked only when no selected clip could take them.
pub fn selection_picker(doc: &Document, clips: &[ClipId]) -> Option<StreamPicker> {
    let pickers: Vec<StreamPicker> = clips
        .iter()
        .filter_map(|c| stream_picker(doc, *c))
        .collect();
    let mut picker = pickers
        .iter()
        .find(|p| p.disabled.is_none())
        .or_else(|| pickers.first())
        .cloned()?;
    block_doubling(doc, clips, &mut picker);
    Some(picker)
}

const PLAYS_ON_ITS_OWN_CLIP: &str = "Already playing on its own clip";
const PLAYS_ON_A_LINKED_CLIP: &str = "Already playing on a linked audio clip";

/// Marks the entries of an enabled picker for `clips` that would only
/// double a stream (see [`choice_blocked`]). The current entry stays
/// available: choosing it changes nothing.
fn block_doubling(doc: &Document, clips: &[ClipId], picker: &mut StreamPicker) {
    if picker.disabled.is_some() {
        return;
    }
    for (i, choice) in picker.choices.iter_mut().enumerate() {
        choice.blocked = if i == picker.current {
            None
        } else {
            choice_blocked(doc, clips, choice.sel)
        };
    }
}

/// Why choosing stream selection `sel` for `clips` would be refused
/// because it doubles a stream (AU-038): every clip that plays its own sound
/// and has the chosen stream would play a stream that a linked clip
/// overlapping it already plays, so the model skips them all. `None` when
/// some clip can take it (the model skips only the others), or when nothing
/// doubles (any other refusal is the model's to word). Read-only, through
/// [`stream_doubled_by`].
pub fn choice_blocked(
    doc: &Document,
    clips: &[ClipId],
    sel: Option<StreamSel>,
) -> Option<&'static str> {
    let seq = doc.root();
    let mut reason = None;
    for &id in clips {
        let Some(c) = doc.clip(id) else {
            continue;
        };
        let ClipContent::Media { asset, .. } = &c.content else {
            continue;
        };
        let Some(item) = doc.library().get(*asset) else {
            continue;
        };
        if c.audio.active().is_none() {
            continue;
        }
        if let Some(StreamSel::Index(stream)) = sel
            && item.info.audio_stream(stream).is_none()
        {
            continue;
        }
        // This clip can take it: the entry stays available.
        let by = stream_doubled_by(doc, seq, id, sel)?;
        reason.get_or_insert_with(|| doubling_reason(doc, by));
    }
    reason
}

/// The wording for a stream clip `by` already plays: a stream added as a
/// clip ("its own clip"), or a piece of the detached sound.
fn doubling_reason(doc: &Document, by: ClipId) -> &'static str {
    let own_clip = doc
        .clip(by)
        .and_then(|c| c.content.asset())
        .and_then(|m| doc.library().get(m))
        .is_some_and(|m| matches!(m.source, MediaSource::AudioOf { .. }));
    if own_clip {
        PLAYS_ON_ITS_OWN_CLIP
    } else {
        PLAYS_ON_A_LINKED_CLIP
    }
}

/// Chooses the stream of every listed clip (AU-038), one undo entry.
pub fn set_stream(
    project: &mut Project,
    clips: &[ClipId],
    sel: Option<StreamSel>,
) -> Result<(), String> {
    let req = EditRequest::SetAudioStream {
        seq: project.doc().root(),
        clips: clips.to_vec(),
        sel,
    };
    project.apply(&req, Coalesce::Never).map(|_| ())
}

// ------------------------------------------------- add stream as clip

/// What one entry of "Add stream as clip ▸" does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddStreamState {
    /// Adds the stream as an audio clip.
    Add,
    /// The clip already plays this stream: Detach audio instead, or why
    /// that is unavailable (AU-038).
    AlreadyPlaying(Result<(), &'static str>),
    /// Unavailable, with the reason.
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddStreamEntry {
    /// The stream's position in the file's list (0-based).
    pub ordinal: usize,
    pub label: String,
    pub state: AddStreamState,
}

/// The selected video clips whose file has several audio streams, with
/// the container index of stream `ordinal` of each, where it exists.
fn stream_targets(doc: &Document, clips: &[ClipId], ordinal: usize) -> Vec<(ClipId, u32)> {
    clips
        .iter()
        .filter_map(|id| {
            let c = doc.clip(*id)?;
            let ClipContent::Media {
                asset,
                kind: MediaKind::Video,
                ..
            } = &c.content
            else {
                return None;
            };
            let info = &doc.library().get(*asset)?.info;
            if info.audio.len() < 2 {
                return None;
            }
            Some((*id, info.audio.get(ordinal)?.index))
        })
        .collect()
}

/// What already plays a stream the model refused to add as a clip
/// (`StreamAlreadyPlaying`, AU-038).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Player {
    /// The video's own sound, or its detached audio clips: Detach (or
    /// Restore) is the way to hear it on its own.
    VideoSound,
    /// An audio clip added from that stream before.
    StreamClip,
}

/// Who plays container stream `stream` of video clip `video`'s file. The
/// model decides *whether* it plays (timeline overlap, current selections,
/// muted clips included); this only tells the cases apart for the wording.
fn player(doc: &Document, video: ClipId, stream: u32) -> Player {
    let Some(c) = doc.clip(video) else {
        return Player::StreamClip;
    };
    let Some(file) = c.content.asset() else {
        return Player::StreamClip;
    };
    let plays = |clip: &Clip| match &clip.content {
        ClipContent::Media {
            asset, stream: sel, ..
        } if *asset == file => doc.library().get(file).is_some_and(|m| {
            m.info
                .played_streams(*sel)
                .iter()
                .any(|s| s.index == stream)
        }),
        _ => false,
    };
    if c.audio.active().is_some() && plays(c) {
        return Player::VideoSound;
    }
    let detached_plays = c.audio.detached().is_some_and(|link| {
        doc.sequence(doc.root()).is_some_and(|seq| {
            seq.tracks().any(|(_, t)| {
                t.clips()
                    .iter()
                    .any(|o| o.id != video && o.link == Some(link) && plays(o))
            })
        })
    });
    if detached_plays {
        Player::VideoSound
    } else {
        Player::StreamClip
    }
}

/// The sentence for a refused "Add stream as clip".
fn refusal_text(doc: &Document, e: &EditError) -> String {
    match e {
        EditError::StreamAlreadyPlaying { clip, stream }
            if player(doc, *clip, *stream) == Player::StreamClip =>
        {
            ON_ITS_OWN_CLIP.to_owned()
        }
        other => describe_edit_error(other),
    }
}

const ON_ITS_OWN_CLIP: &str = "This stream already plays on its own audio clip";

/// Whether the clip menu shows "Add stream as clip ▸": a selected video
/// clip's file has several audio streams (AU-038; one stream is covered by
/// Detach audio).
pub fn offers_add_stream(doc: &Document, clips: &[ClipId]) -> bool {
    clips
        .iter()
        .any(|c| !stream_targets(doc, std::slice::from_ref(c), 0).is_empty())
}

/// The "Add stream as clip ▸" entries for a selection; empty (the item is
/// hidden) unless a selected video clip's file has several audio streams
/// (AU-038). Each entry is planned against the editor without committing,
/// so it says exactly what the command would do.
pub fn add_stream_menu(editor: &Editor, clips: &[ClipId]) -> Vec<AddStreamEntry> {
    let doc = editor.document();
    let Some(info) = clips.iter().find_map(|id| {
        let c = doc.clip(*id)?;
        let ClipContent::Media {
            asset,
            kind: MediaKind::Video,
            ..
        } = &c.content
        else {
            return None;
        };
        let info = &doc.library().get(*asset)?.info;
        (info.audio.len() > 1).then_some(info)
    }) else {
        return Vec::new();
    };
    let seq = doc.root();
    let ctx = freecut_timeline::edit::EditCtx::default();
    info.audio
        .iter()
        .enumerate()
        .map(|(ordinal, stream)| {
            let mut errors = Vec::new();
            let mut addable = false;
            let mut video_sound = false;
            let mut on_own_clip = false;
            for (clip, stream) in stream_targets(doc, clips, ordinal) {
                let req = EditRequest::AddStreamAsClip { seq, clip, stream };
                match editor.plan(&ctx, &req) {
                    Ok(_) => {
                        addable = true;
                        break;
                    }
                    Err(EditError::StreamAlreadyPlaying { clip, stream }) => {
                        match player(doc, clip, stream) {
                            Player::VideoSound => video_sound = true,
                            Player::StreamClip => on_own_clip = true,
                        }
                    }
                    Err(e) => errors.push(e),
                }
            }
            let state = if addable {
                AddStreamState::Add
            } else if let Some(e) = errors.first() {
                AddStreamState::Unavailable(describe_edit_error(e))
            } else if video_sound {
                AddStreamState::AlreadyPlaying(match sound_command(doc, clips) {
                    SoundCommand::Detach(ready) => ready,
                    SoundCommand::Restore => {
                        Err("Its detached audio clip already plays this stream")
                    }
                })
            } else if on_own_clip {
                AddStreamState::Unavailable(ON_ITS_OWN_CLIP.to_owned())
            } else {
                AddStreamState::Unavailable("No selected clip has that stream".to_owned())
            };
            AddStreamEntry {
                ordinal,
                label: stream_label(ordinal, stream),
                state,
            }
        })
        .collect()
}

/// Puts stream `ordinal` of every selected video clip that has it on its
/// own linked audio clip (AU-038), as one undo entry; clips that already
/// play it are passed over. The new clips are selected. Returns how many
/// were added.
pub fn add_stream_as_clips(
    project: &mut Project,
    clips: &[ClipId],
    ordinal: usize,
) -> Result<usize, String> {
    let targets = stream_targets(project.doc(), clips, ordinal);
    let seq = project.doc().root();
    let gesture = project.begin_gesture();
    let mut added: Vec<ClipId> = Vec::new();
    let mut first_error = None;
    for (clip, stream) in targets {
        let req = EditRequest::AddStreamAsClip { seq, clip, stream };
        // The model refuses every stream that already plays (AU-038); a dry
        // run first tells who plays it, for the message.
        if let Err(e) = project.editor.plan(&project.ctx(), &req) {
            first_error.get_or_insert_with(|| refusal_text(project.doc(), &e));
            continue;
        }
        match project.apply(&req, Coalesce::Gesture(gesture)) {
            Ok(Committed::Applied { .. }) => added.extend(project.selection.iter().copied()),
            Ok(Committed::NoOp) => {}
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    project.end_gesture(gesture);
    if added.is_empty() {
        return Err(first_error.unwrap_or_else(|| "No selected clip has that stream".to_owned()));
    }
    added.dedup();
    project.selection = added.clone();
    Ok(added.len())
}

// ----------------------------------------------------------- sync

/// The out-of-sync badge (TL-030): the offset in frames, "+12f" or "-3f";
/// a slip that isn't a whole number of frames gets one decimal ("+0.4f"),
/// and one under a tenth of a frame reads "+<0.1f".
pub fn sync_badge(offset: &SyncOffset, frame: Tick) -> String {
    let ticks = offset.ticks.raw();
    let per = frame.raw().max(1);
    if ticks % per == 0 {
        return format!("{:+}f", ticks / per);
    }
    let frames = ticks as f64 / per as f64;
    let tenths = (frames * 10.0).round() / 10.0;
    if tenths == 0.0 {
        if ticks > 0 { "+<0.1f" } else { "-<0.1f" }.to_owned()
    } else {
        format!("{tenths:+.1}f")
    }
}

/// Whether Resync applies to the selection: some selected clip is out of
/// sync with its partner (TL-030).
pub fn can_resync(doc: &Document, seq: SeqId, clips: &[ClipId]) -> bool {
    clips.iter().any(|c| sync_offset(doc, seq, *c).is_some())
}

/// Moves the selected clips' sound back in sync (TL-030), one undo entry.
pub fn resync(project: &mut Project, clips: &[ClipId]) -> Result<(), String> {
    let req = EditRequest::Resync {
        seq: project.doc().root(),
        clips: clips.to_vec(),
    };
    project.apply(&req, Coalesce::Never).map(|_| ())
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::edit::{InsertTarget, MoveReq, NewMedia};
    use freecut_timeline::model::{MediaInfo, ProjectSettings, StoredPath, VideoStreamInfo};
    use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND};

    use super::*;
    use crate::session::ProbedFile;

    fn stream(
        index: u32,
        default: bool,
        lang: Option<&str>,
        layout: Option<&str>,
    ) -> AudioStreamInfo {
        AudioStreamInfo {
            index,
            codec: "aac".into(),
            sample_rate: 48_000,
            channels: 2,
            start: Tick::ZERO,
            duration: Tick::from_secs(4).unwrap(),
            duration_exact: true,
            default,
            language: lang.map(str::to_owned),
            title: None,
            layout: layout.map(str::to_owned),
        }
    }

    /// A project with one 4 s video clip on the main track whose file has
    /// the given audio streams; the clip is selected.
    pub(crate) fn project_with_streams(audio: Vec<AudioStreamInfo>) -> (Project, ClipId) {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let four = Tick::from_secs(4).unwrap();
        let info = MediaInfo {
            container: "mkv".into(),
            default_video: Some(0),
            duration: four,
            video: vec![VideoStreamInfo {
                index: 0,
                codec: "h264".into(),
                width: 640,
                height: 360,
                sar: (1, 1),
                rotation: freecut_timeline::model::Rotation::None,
                mirrored: false,
                rate: None,
                variable_rate: false,
                has_alpha: false,
                bit_depth: 8,
                start: Tick::ZERO,
                duration: four,
            }],
            audio,
            still: None,
        };
        let ids = project
            .import(vec![ProbedFile {
                path: "/talk.mkv".into(),
                media: NewMedia {
                    path: StoredPath {
                        absolute: "/talk.mkv".into(),
                        relative: None,
                    },
                    info,
                    fingerprint: None,
                    display_name: None,
                },
            }])
            .unwrap();
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
        let clip = project.selection[0];
        (project, clip)
    }

    #[test]
    fn stream_labels_name_number_language_title_and_layout() {
        let mut s = stream(3, false, Some("eng"), Some("5.1(side)"));
        s.title = Some("Director".into());
        assert_eq!(stream_label(1, &s), "Stream 2 · eng · Director · 5.1(side)");
        assert_eq!(
            stream_label(0, &stream(1, true, None, None)),
            "Stream 1 · stereo"
        );
    }

    #[test]
    fn the_picker_offers_each_stream_and_all_mixed_and_tracks_the_choice() {
        let (mut project, clip) = project_with_streams(vec![
            stream(1, false, Some("eng"), Some("stereo")),
            stream(2, true, Some("fra"), Some("mono")),
        ]);
        let picker = stream_picker(project.doc(), clip).unwrap();
        let labels: Vec<_> = picker.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Stream 1 · eng · stereo",
                "Stream 2 · fra · mono",
                ALL_MIXED
            ]
        );
        // Stream 2 is the file's default: it plays, and picking it follows
        // the default.
        assert_eq!(picker.current, 1);
        assert_eq!(picker.choices[1].sel, None);
        assert_eq!(picker.choices[0].sel, Some(StreamSel::Index(1)));
        assert_eq!(picker.disabled, None);

        set_stream(&mut project, &[clip], Some(StreamSel::AllMixed)).unwrap();
        assert_eq!(stream_picker(project.doc(), clip).unwrap().current, 2);
        set_stream(&mut project, &[clip], Some(StreamSel::Index(1))).unwrap();
        assert_eq!(stream_picker(project.doc(), clip).unwrap().current, 0);
        project.undo().unwrap();
        assert_eq!(stream_picker(project.doc(), clip).unwrap().current, 2);

        // Detached sound: the picker is disabled with the reason.
        let seq = project.doc().root();
        project
            .apply(
                &EditRequest::DetachAudio {
                    seq,
                    clips: vec![clip],
                },
                Coalesce::Never,
            )
            .unwrap();
        let picker = selection_picker(project.doc(), &[clip]).unwrap();
        assert!(picker.disabled.unwrap().contains("detached"));
    }

    #[test]
    fn a_single_stream_shows_just_that_stream_and_hides_add_stream() {
        let (project, clip) = project_with_streams(vec![stream(1, true, None, Some("stereo"))]);
        let picker = stream_picker(project.doc(), clip).unwrap();
        assert_eq!(picker.choices.len(), 1);
        assert!(add_stream_menu(&project.editor, &[clip]).is_empty());
    }

    #[test]
    fn add_stream_offers_detach_for_the_playing_stream_and_links_a_new_clip() {
        let (mut project, video) = project_with_streams(vec![
            stream(1, true, None, Some("mono")),
            stream(2, false, None, Some("stereo")),
        ]);
        let menu = add_stream_menu(&project.editor, &[video]);
        assert_eq!(menu.len(), 2);
        assert_eq!(menu[0].state, AddStreamState::AlreadyPlaying(Ok(())));
        assert_eq!(menu[1].state, AddStreamState::Add);

        let entries = project.editor.history().len();
        assert_eq!(add_stream_as_clips(&mut project, &[video], 1), Ok(1));
        assert_eq!(project.editor.history().len(), entries + 1);
        let added = project.selection[0];
        let doc = project.doc().clone();
        assert_eq!(doc.clip(added).unwrap().link, doc.clip(video).unwrap().link);
        assert!(doc.clip(added).unwrap().link.is_some());
        // In sync: no badge, no Resync.
        let seq = doc.root();
        assert!(freecut_timeline::edit::sync_offsets(&doc, seq).is_empty());
        assert!(!can_resync(&doc, seq, &[video, added]));
        // Stream 2 is playing now on its own clip.
        let menu = add_stream_menu(&project.editor, &[video]);
        assert_eq!(
            menu[1].state,
            AddStreamState::Unavailable(ON_ITS_OWN_CLIP.to_owned())
        );
        assert_eq!(
            add_stream_as_clips(&mut project, &[video], 1),
            Err(ON_ITS_OWN_CLIP.to_owned())
        );

        // Moving the new clip 3 frames late puts both out of sync.
        let frame = doc.settings().frame_rate.frame_ticks();
        project
            .apply(
                &EditRequest::Move(MoveReq {
                    seq,
                    clips: vec![added],
                    delta: Tick::from_raw(3 * frame.raw()),
                    track_shift: 0,
                }),
                Coalesce::Never,
            )
            .unwrap();
        let doc = project.doc().clone();
        let offsets = freecut_timeline::edit::sync_offsets(&doc, seq);
        assert_eq!(sync_badge(&offsets[&added], frame), "+3f");
        assert_eq!(sync_badge(&offsets[&video], frame), "+3f");
        assert!(can_resync(&doc, seq, &[video]));
        resync(&mut project, &[added]).unwrap();
        assert!(freecut_timeline::edit::sync_offsets(project.doc(), seq).is_empty());
        assert!(resync(&mut project, &[added]).is_err());
    }

    #[test]
    fn a_stream_clip_moved_past_the_video_no_longer_blocks_the_stream() {
        let (mut project, video) = project_with_streams(vec![
            stream(1, true, None, Some("mono")),
            stream(2, false, None, Some("stereo")),
        ]);
        add_stream_as_clips(&mut project, &[video], 1).unwrap();
        let added = project.selection[0];
        let seq = project.doc().root();
        // Moved past the video's end: they don't overlap, so adding stream
        // 2 again doesn't double it (AU-038).
        project
            .apply(
                &EditRequest::Move(MoveReq {
                    seq,
                    clips: vec![added],
                    delta: Tick::from_secs(5).unwrap(),
                    track_shift: 0,
                }),
                Coalesce::Never,
            )
            .unwrap();
        let menu = add_stream_menu(&project.editor, &[video]);
        assert_eq!(menu[1].state, AddStreamState::Add);
        assert_eq!(add_stream_as_clips(&mut project, &[video], 1), Ok(1));
    }

    #[test]
    fn after_detach_the_stream_clip_still_blocks_its_stream_and_goes_on_restore() {
        let (mut project, video) = project_with_streams(vec![
            stream(1, true, None, Some("mono")),
            stream(2, false, None, Some("stereo")),
        ]);
        add_stream_as_clips(&mut project, &[video], 1).unwrap();
        let added = project.selection[0];
        let seq = project.doc().root();
        project
            .apply(
                &EditRequest::DetachAudio {
                    seq,
                    clips: vec![video],
                },
                Coalesce::Never,
            )
            .unwrap();
        // Stream 1 plays on the detached clip: Restore is the way back.
        // Stream 2 plays on the clip added before, with its own wording.
        let menu = add_stream_menu(&project.editor, &[video]);
        assert_eq!(
            menu[0].state,
            AddStreamState::AlreadyPlaying(Err(
                "Its detached audio clip already plays this stream"
            ))
        );
        assert_eq!(
            menu[1].state,
            AddStreamState::Unavailable(ON_ITS_OWN_CLIP.to_owned())
        );
        assert_eq!(
            add_stream_as_clips(&mut project, &[video], 1),
            Err(ON_ITS_OWN_CLIP.to_owned())
        );
        // The stream clip stays linked, so Restore removes it too and asks.
        let (removed, ask) = crate::clip_sound::restore_removals(project.doc(), seq, &[video]);
        assert!(removed.contains(&added), "{removed:?}");
        assert_eq!(removed.len(), 2);
        assert!(ask);
    }

    #[test]
    fn streams_a_linked_clip_plays_show_disabled_with_the_reason() {
        let (mut project, video) = project_with_streams(vec![
            stream(1, true, None, Some("mono")),
            stream(2, false, None, Some("stereo")),
        ]);
        add_stream_as_clips(&mut project, &[video], 1).unwrap();
        let blocked = |project: &Project, clips: &[ClipId]| -> Vec<Option<&'static str>> {
            selection_picker(project.doc(), clips)
                .unwrap()
                .choices
                .iter()
                .map(|c| c.blocked)
                .collect()
        };
        // Stream 2 plays on its own clip: picking it or all streams would
        // double it. Stream 1 is the current choice.
        let doubled = Some(PLAYS_ON_ITS_OWN_CLIP);
        assert_eq!(blocked(&project, &[video]), [None, doubled, doubled]);
        let picker = stream_picker(project.doc(), video).unwrap();
        assert_eq!(picker.choices[1].blocked, doubled, "inspector agrees");
        assert_eq!(
            choice_blocked(project.doc(), &[video], Some(StreamSel::Index(2))),
            doubled
        );
        // The model refuses the same choice.
        assert_eq!(
            set_stream(&mut project, &[video], Some(StreamSel::Index(2))),
            Err(describe_edit_error(&EditError::StreamPlaysElsewhere {
                clip: video,
                stream: 2,
                by: project.selection[0],
            }))
        );

        // With a second clip of the file selected, that one can take it:
        // the menu entry is enabled and only the other clip is skipped.
        let media = project.doc().clip(video).unwrap().content.asset().unwrap();
        let main = project
            .doc()
            .sequence(project.doc().root())
            .unwrap()
            .main_track()
            .unwrap()
            .id;
        crate::ui::timeline::insert_media(
            &mut project,
            media,
            InsertTarget::Track(main),
            Tick::from_secs(4).unwrap(),
        )
        .unwrap();
        let other = project.selection[0];
        assert_eq!(blocked(&project, &[video, other]), [None, None, None]);
        set_stream(&mut project, &[video, other], Some(StreamSel::Index(2))).unwrap();
        let sel = |c: ClipId| match &project.doc().clip(c).unwrap().content {
            ClipContent::Media { stream, .. } => *stream,
            _ => unreachable!(),
        };
        assert_eq!((sel(video), sel(other)), (None, Some(StreamSel::Index(2))));
    }

    #[test]
    fn a_detached_clip_cannot_pick_the_stream_of_a_stream_clip() {
        let (mut project, video) = project_with_streams(vec![
            stream(1, true, None, Some("mono")),
            stream(2, false, None, Some("stereo")),
        ]);
        add_stream_as_clips(&mut project, &[video], 1).unwrap();
        let seq = project.doc().root();
        project
            .apply(
                &EditRequest::DetachAudio {
                    seq,
                    clips: vec![video],
                },
                Coalesce::Never,
            )
            .unwrap();
        let file = project.doc().clip(video).unwrap().content.asset();
        let detached = project
            .doc()
            .sequence(seq)
            .unwrap()
            .tracks()
            .flat_map(|(_, t)| t.clips().iter())
            .find(|c| c.id != video && c.content.asset() == file)
            .unwrap()
            .id;
        let picker = stream_picker(project.doc(), detached).unwrap();
        assert_eq!(picker.disabled, None);
        let blocked: Vec<_> = picker.choices.iter().map(|c| c.blocked).collect();
        assert_eq!(
            blocked,
            [
                None,
                Some(PLAYS_ON_ITS_OWN_CLIP),
                Some(PLAYS_ON_ITS_OWN_CLIP)
            ]
        );
        // The video's own picker is disabled for detached sound, not blocked.
        let picker = stream_picker(project.doc(), video).unwrap();
        assert!(picker.disabled.is_some());
        assert!(picker.choices.iter().all(|c| c.blocked.is_none()));
    }

    #[test]
    fn badges_read_whole_frames_tenths_and_tiny_slips() {
        let rate = FrameRate::new(30, 1).unwrap();
        let frame = rate.frame_ticks();
        let off = |ticks: i64| SyncOffset {
            frames: 0,
            ticks: Tick::from_raw(ticks),
            partner: ClipId::from_raw(std::num::NonZeroU64::MIN),
        };
        assert_eq!(sync_badge(&off(12 * frame.raw()), frame), "+12f");
        assert_eq!(sync_badge(&off(-3 * frame.raw()), frame), "-3f");
        assert_eq!(sync_badge(&off(frame.raw() * 2 / 5), frame), "+0.4f");
        assert_eq!(sync_badge(&off(-frame.raw() * 5 / 2), frame), "-2.5f");
        assert_eq!(sync_badge(&off(1), frame), "+<0.1f");
        assert_eq!(sync_badge(&off(-1), frame), "-<0.1f");
        assert_eq!(TICKS_PER_SECOND % frame.raw(), 0);
    }
}

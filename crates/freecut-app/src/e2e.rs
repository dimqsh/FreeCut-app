//! End-to-end check of the app's own flows without a window: new project,
//! probe and import media, place clips, edit, render preview frames, mix
//! the sound offline, detach and restore it, import audio and extract a
//! video's sound from the asset panel, save, reopen, export to MP4
//! (EX-022) with the sound's levels, fades, keyframes and solo measured in
//! the file, stop on an unreadable source (`decode-error`) and cancel an
//! export. Fixtures are generated
//! at test time with the ffmpeg CLI (the
//! test is skipped when it is missing). Set FREECUT_E2E_OUT to a folder to
//! also write the rendered frames there as PPM images.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use freecut_audio::{DocSchedule, FileOpener, Mixer, Schedule};
use freecut_render::FontRegistry;
use freecut_timeline::edit::{EditRequest, InsertTarget, MoveReq, SplitReq, SplitTargets};
use freecut_timeline::history::Coalesce;
use freecut_timeline::mix::mix_sources;
use freecut_timeline::model::{
    ClipContent, EmbeddedAudio, MediaKind, MediaSource, ProjectSettings, TrackHeader, TrackKind,
    TrackSlot,
};
use freecut_timeline::time::{FrameRate, SampleRate, Tick, TimeRange};

use crate::clip_sound::{SoundCommand, restore_removals, sound_command};
use crate::preview::{Preview, PreviewFrame, PreviewRequest};
use crate::session::{Project, import_report, open_project, probe_for_import, sound_only};
use crate::ui::assets::{AssetTab, AssetsState};
use crate::ui::audio::{AudioFilter, audio_entries, visible};
use crate::ui::timeline::{add_at_playhead, insert_media};

/// The presets the app loads (FX-030, TX-056).
fn presets() -> Arc<freecut_timeline::motion::PresetTable> {
    Arc::new(freecut_project::presets::bundled().table)
}

fn ffmpeg(args: &[&str]) -> bool {
    Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

fn fixtures(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let landscape = dir.join("landscape.mp4");
    let portrait = dir.join("portrait.mp4");
    let ok = ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=30:duration=4",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=4",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        landscape.to_str()?,
    ]) && ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=red:size=180x320:rate=30:duration=2",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        portrait.to_str()?,
    ]);
    ok.then_some((landscape, portrait))
}

fn render(project: &Project, preview: &Preview, generation: u64) -> PreviewFrame {
    preview.request(PreviewRequest {
        doc: Arc::new(project.doc().clone()),
        media_paths: Arc::new(project.media_paths.clone()),
        offline: Arc::new(project.offline.clone()),
        time: project.playhead,
        max_size: (640, 360),
        text_draft: None,
        generation,
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(frame) = preview.poll()
            && frame.generation == generation
        {
            return frame;
        }
        assert!(Instant::now() < deadline, "preview timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn pixel(frame: &PreviewFrame, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * frame.width + x) * 4) as usize;
    [frame.rgba[i], frame.rgba[i + 1], frame.rgba[i + 2]]
}

fn write_ppm(frame: &PreviewFrame, name: &str) {
    let Some(dir) = std::env::var_os("FREECUT_E2E_OUT") else {
        return;
    };
    let mut bytes = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    for px in frame.rgba.as_chunks::<4>().0 {
        bytes.extend_from_slice(&px[..3]);
    }
    std::fs::write(Path::new(&dir).join(name), bytes).unwrap();
}

#[test]
fn new_project_import_edit_preview_save_and_reopen() {
    let dir = tempfile_dir("edit");
    let Some((landscape, portrait)) = fixtures(&dir) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    freecut_media::init().unwrap();

    // A 16:9 project at 30 fps.
    let settings = ProjectSettings {
        width: 640,
        height: 360,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        ..ProjectSettings::new_default().unwrap()
    };
    let mut project = Project::new(settings).unwrap();

    // Import: probe on "workers", then one undoable import.
    let files = vec![
        probe_for_import(&landscape, None).unwrap(),
        probe_for_import(&portrait, None).unwrap(),
    ];
    let ids = project.import(files).unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(
        project.editor.history().len(),
        1,
        "one import, one undo step"
    );

    // Place the landscape clip on the main track and the portrait one above.
    let main = project
        .doc()
        .sequence(project.doc().root())
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    insert_media(&mut project, ids[0], InsertTarget::Track(main), Tick::ZERO).unwrap();
    insert_media(
        &mut project,
        ids[1],
        InsertTarget::NewTrack,
        Tick::from_secs(1).unwrap(),
    )
    .unwrap();
    let seq = project.doc().sequence(project.doc().root()).unwrap();
    assert_eq!(seq.visual_tracks().len(), 2);
    assert_eq!(seq.duration(), Tick::from_secs(4).unwrap());

    let preview = Preview::new(FontRegistry::bundled(), presets(), || {});

    // Frame at 0.5 s: only the test pattern.
    project.playhead = Tick::from_raw(Tick::from_secs(1).unwrap().raw() / 2);
    let a = render(&project, &preview, 1);
    write_ppm(&a, "frame_0_5s.ppm");
    assert_eq!((a.width, a.height), (640, 360));
    let corner = pixel(&a, 5, 5);
    assert!(
        corner.iter().any(|c| *c > 20),
        "the test pattern is visible: {corner:?}"
    );

    // Frame at 1.5 s: the red portrait overlay is fitted into the centre.
    project.playhead = Tick::from_raw(Tick::from_secs(3).unwrap().raw() / 2);
    let b = render(&project, &preview, 2);
    write_ppm(&b, "frame_1_5s.ppm");
    let center = pixel(&b, 320, 180);
    assert!(
        center[0] > 200 && center[1] < 60 && center[2] < 60,
        "red overlay at the centre: {center:?}"
    );
    // testsrc2's second bar is green; the pillarboxed overlay leaves it alone.
    let left = pixel(&b, 160, 100);
    assert!(
        left[1] > 200 && left[0] < 60,
        "the overlay is pillarboxed: {left:?}"
    );

    // Split the main clip at 2 s and move the right half up a track.
    project.playhead = Tick::from_secs(2).unwrap();
    project
        .apply(
            &EditRequest::Split(SplitReq {
                seq: project.doc().root(),
                at: project.playhead,
                targets: SplitTargets::Clips(vec![
                    project
                        .doc()
                        .sequence(project.doc().root())
                        .unwrap()
                        .main_track()
                        .unwrap()
                        .clips()[0]
                        .id,
                ]),
            }),
            Coalesce::Never,
        )
        .unwrap();
    let right = project.selection.clone();
    assert_eq!(right.len(), 1);
    project
        .apply(
            &EditRequest::Move(MoveReq {
                seq: project.doc().root(),
                clips: right.clone(),
                delta: Tick::ZERO,
                track_shift: 2,
            }),
            Coalesce::Never,
        )
        .unwrap();
    let seq = project.doc().sequence(project.doc().root()).unwrap();
    assert_eq!(seq.track_at(TrackSlot::MAIN).unwrap().clips().len(), 1);
    assert!(seq.visual_tracks().len() >= 3);

    // Undo both, redo both.
    project.undo().unwrap();
    project.undo().unwrap();
    assert_eq!(
        project
            .doc()
            .sequence(project.doc().root())
            .unwrap()
            .main_track()
            .unwrap()
            .clips()
            .len(),
        1
    );
    project.redo().unwrap();
    project.redo().unwrap();

    // Save, reopen, and get the same document back.
    let file = dir.join("e2e.freecut");
    let saved = project.save_job(file.clone()).run();
    assert!(saved.result.is_ok(), "{:?}", saved.result);
    project.saved(&saved);
    assert!(!project.editor.is_dirty());
    let doc_before = project.doc().clone();
    drop(project); // releases the lock
    let loaded = open_project(&file).unwrap();
    assert!(loaded.offline.is_empty());
    let reopened = Project::from_loaded(loaded);
    assert_eq!(reopened.doc(), &doc_before);
    assert!(!reopened.editor.is_dirty());
}

fn tempfile_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("freecut-e2e-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The level of a `freq` Hz tone in the left channel of interleaved stereo
/// `samples` (a single-bin DFT), as a peak amplitude.
fn tone_level(samples: &[f32], rate: u32, freq: f64) -> f64 {
    let left: Vec<f64> = samples.iter().step_by(2).map(|v| f64::from(*v)).collect();
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in left.iter().enumerate() {
        let phase = std::f64::consts::TAU * freq * i as f64 / f64::from(rate);
        re += v * phase.cos();
        im -= v * phase.sin();
    }
    2.0 * re.hypot(im) / left.len() as f64
}

/// Mixes 1 s..2 s of the project's sound offline, as playback would.
fn mix_second(project: &Project) -> Vec<f32> {
    let rate = SampleRate::new(48_000).unwrap();
    let mut mixer = Mixer::new(rate);
    let schedule = DocSchedule::root(project.doc().clone());
    let opener = FileOpener::from_paths(&project.media_paths, &project.offline);
    let mut out = vec![0.0f32; 2 * 48_000];
    mixer.render(&schedule, &opener, 48_000, &mut out);
    out
}

fn assert_tone(project: &Project, what: &str) {
    let out = mix_second(project);
    let level = tone_level(&out, 48_000, 440.0);
    let off = tone_level(&out, 48_000, 1_000.0);
    assert!(level > 0.05, "{what}: 440 Hz at {level}");
    assert!(off < level / 20.0, "{what}: a clean tone ({off} at 1 kHz)");
}

fn assert_silent(project: &Project, what: &str) {
    let out = mix_second(project);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 1e-6, "{what}: silent, but the peak is {peak}");
}

#[test]
fn sound_plays_detaches_mutes_restores_and_reopens() {
    let dir = tempfile_dir("sound");
    let Some((landscape, _)) = fixtures(&dir) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    freecut_media::init().unwrap();
    let settings = ProjectSettings {
        width: 640,
        height: 360,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        ..ProjectSettings::new_default().unwrap()
    };
    let mut project = Project::new(settings).unwrap();
    let ids = project
        .import(vec![probe_for_import(&landscape, None).unwrap()])
        .unwrap();
    let seq = project.doc().root();
    let main = project
        .doc()
        .sequence(seq)
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    insert_media(&mut project, ids[0], InsertTarget::Track(main), Tick::ZERO).unwrap();
    let video = project.selection[0];
    let doc = project.doc().clone();
    let stream = doc.library().get(ids[0]).unwrap().info.audio[0].index;

    // The placed video plays its own sound (AU-002).
    let whole = TimeRange::new(Tick::ZERO, Tick::from_secs(4).unwrap()).unwrap();
    let sources = mix_sources(&doc, seq, whole);
    assert_eq!(sources.len(), 1);
    assert_eq!(
        (sources[0].clip, sources[0].asset, sources[0].stream),
        (video, ids[0], stream)
    );
    let inputs = DocSchedule::root(doc.clone()).inputs(whole);
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].key, video.get());
    assert_eq!(inputs[0].gain, 1.0);
    assert_tone(&project, "embedded sound");

    // Detach (TL-030): a linked audio clip on an audio track, a silent video.
    assert_eq!(sound_command(&doc, &[video]), SoundCommand::Detach(Ok(())));
    project
        .apply(
            &EditRequest::DetachAudio {
                seq,
                clips: vec![video],
            },
            Coalesce::Never,
        )
        .unwrap();
    let audio = project.selection[0];
    let doc = project.doc().clone();
    let (slot, audio_track) = doc.sequence(seq).unwrap().find_clip(audio).unwrap();
    assert_eq!(audio_track.kind, TrackKind::Audio);
    assert_eq!(slot, TrackSlot::audio(0));
    let audio_clip = doc.clip(audio).unwrap();
    assert!(matches!(
        audio_clip.content,
        ClipContent::Media {
            kind: MediaKind::Audio,
            ..
        }
    ));
    let link = audio_clip.link.unwrap();
    assert_eq!(
        doc.clip(video).unwrap().audio,
        EmbeddedAudio::Detached(link)
    );
    let sources = mix_sources(&doc, seq, whole);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].clip, audio);
    assert_tone(&project, "detached sound");

    // Muting the audio track silences it (AU-011).
    let audio_track = audio_track.id;
    let mut header: TrackHeader = doc
        .sequence(seq)
        .unwrap()
        .find_track(audio_track)
        .unwrap()
        .1
        .header
        .clone();
    header.flags.muted = true;
    project
        .apply(
            &EditRequest::SetTrackHeader {
                seq,
                track: audio_track,
                header,
            },
            Coalesce::Never,
        )
        .unwrap();
    assert!(mix_sources(project.doc(), seq, whole).is_empty());
    assert_silent(&project, "muted audio track");

    // Restore: the untouched audio clip goes without a question, and the
    // video sounds again from the main track.
    let doc = project.doc().clone();
    assert_eq!(sound_command(&doc, &[video]), SoundCommand::Restore);
    assert_eq!(restore_removals(&doc, seq, &[video]), (vec![audio], false));
    project
        .apply(
            &EditRequest::RestoreAudio {
                seq,
                clips: vec![video],
            },
            Coalesce::Never,
        )
        .unwrap();
    let restored = project.doc().clone();
    assert!(restored.clip(audio).is_none());
    assert!(restored.clip(video).unwrap().audio.active().is_some());
    assert_tone(&project, "restored sound");

    // Undo brings back the muted, detached state; redo the restore.
    project.undo().unwrap();
    assert!(project.doc().clip(audio).is_some());
    assert_silent(&project, "undone restore");
    project.redo().unwrap();
    assert_eq!(project.doc(), &restored);
    assert_tone(&project, "redone restore");

    // Save, reopen, same document and the same sound.
    let file = dir.join("sound.freecut");
    let saved = project.save_job(file.clone()).run();
    assert!(saved.result.is_ok(), "{:?}", saved.result);
    project.saved(&saved);
    drop(project);
    let reopened = Project::from_loaded(open_project(&file).unwrap());
    assert_eq!(reopened.doc(), &restored);
    assert_tone(&reopened, "reopened");
}

/// A 2 s, 880 Hz tone as a WAV file: audio only.
fn tone_wav(dir: &Path) -> Option<PathBuf> {
    let wav = dir.join("tone.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=880:duration=2",
        "-ac",
        "2",
        wav.to_str()?,
    ])
    .then_some(wav)
}

fn new_project() -> Project {
    let settings = ProjectSettings {
        width: 640,
        height: 360,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        ..ProjectSettings::new_default().unwrap()
    };
    Project::new(settings).unwrap()
}

/// Undoes the last step and redoes it, checking both states.
fn undo_redo(project: &mut Project) {
    let after = project.doc().clone();
    project.undo().unwrap();
    assert_ne!(project.doc(), &after, "undo changed nothing");
    project.redo().unwrap();
    assert_eq!(project.doc(), &after, "redo gives the same document");
}

#[test]
fn audio_panel_imports_adds_and_extracts() {
    let dir = tempfile_dir("audio-panel");
    let (Some((landscape, portrait)), Some(wav)) = (fixtures(&dir), tone_wav(&dir)) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    freecut_media::init().unwrap();
    let mut project = new_project();
    let seq = project.doc().root();

    // The video comes in through the Media tab's Import….
    let video = project
        .import(vec![probe_for_import(&landscape, None).unwrap()])
        .unwrap()[0];

    // "Import audio…" with a silent video in the batch: it is skipped.
    let batch = vec![
        probe_for_import(&wav, None).unwrap(),
        probe_for_import(&portrait, None).unwrap(),
    ];
    let (kept, skipped) = sound_only(batch);
    assert_eq!((kept.len(), skipped), (1, 1));
    assert_eq!(
        import_report(kept.len(), skipped),
        "Imported 1 file. 1 file skipped: no sound"
    );
    let steps = project.editor.history().len();
    let listed = project.import_audio(kept).unwrap();
    assert_eq!(project.editor.history().len(), steps + 1);
    let tone = listed[0];
    let entries = audio_entries(project.doc().library());
    assert_eq!(entries.len(), 1, "the video isn't listed in Audio");
    assert_eq!((entries[0].id, entries[0].extracted), (tone, false));
    assert_eq!(entries[0].name, "tone.wav");
    undo_redo(&mut project);
    assert_eq!(
        project.media_paths.get(&tone),
        Some(&wav),
        "a redone import finds its file again"
    );

    // "+" on the row: the first free audio track at the playhead; the clip
    // is selected and the playhead stays.
    let at = Tick::from_secs(1).unwrap();
    project.playhead = at;
    add_at_playhead(&mut project, tone).unwrap();
    assert_eq!(project.playhead, at);
    let tone_clip = project.selection[0];
    let doc = project.doc().clone();
    let (slot, track) = doc.sequence(seq).unwrap().find_clip(tone_clip).unwrap();
    assert_eq!((slot, track.kind), (TrackSlot::audio(0), TrackKind::Audio));
    assert_eq!(doc.clip(tone_clip).unwrap().start(), at);
    undo_redo(&mut project);
    assert_eq!(project.selection, [tone_clip]);

    // Extract audio from the video: a new item, shown in the Audio tab
    // under Extracted.
    let (extracted, created) = project.extract_audio(video).unwrap();
    assert!(created);
    let item = project.doc().library().get(extracted).unwrap().clone();
    assert_eq!(
        item.source,
        MediaSource::AudioOf {
            source: video,
            stream: project.doc().library().get(video).unwrap().info.audio[0].index,
        }
    );
    assert_eq!(
        project.media_paths.get(&extracted),
        project.media_paths.get(&video),
        "extracted sound reads the video's file"
    );
    let mut panel = AssetsState::default();
    panel.audio.search = "tone".to_owned();
    panel.reveal_audio(project.doc(), extracted, Some(AudioFilter::Extracted));
    assert_eq!(panel.tab, AssetTab::Audio);
    assert_eq!(panel.audio.selected, Some(extracted));
    assert!(panel.audio.search.is_empty(), "a hiding search is cleared");
    let entries = audio_entries(project.doc().library());
    let shown: Vec<_> = visible(&entries, panel.audio.filter, &panel.audio.search)
        .into_iter()
        .map(|e| (e.id, e.name.as_str()))
        .collect();
    assert_eq!(shown, [(extracted, "landscape.mp4 (audio)")]);
    undo_redo(&mut project);

    // Extracting again finds the same item and adds no undo step.
    let steps = project.editor.history().len();
    assert_eq!(project.extract_audio(video).unwrap(), (extracted, false));
    assert_eq!(project.editor.history().len(), steps);

    // The extracted item at the same playhead (undoing the first add moved
    // the playhead back to the end of the then empty timeline): Audio 1 is
    // taken there, so it gets a track of its own.
    project.playhead = at;
    add_at_playhead(&mut project, extracted).unwrap();
    let ext_clip = project.selection[0];
    let doc = project.doc().clone();
    let (slot, _) = doc.sequence(seq).unwrap().find_clip(ext_clip).unwrap();
    assert_ne!(slot, TrackSlot::audio(0));
    assert_eq!(doc.sequence(seq).unwrap().audio_tracks().len(), 2);
    assert_eq!(doc.clip(ext_clip).unwrap().start(), at);
    undo_redo(&mut project);

    // Both sound from 1 s: the tone file and the video's extracted sound.
    let out = mix_second(&project);
    let (a, b) = (
        tone_level(&out, 48_000, 440.0),
        tone_level(&out, 48_000, 880.0),
    );
    assert!(a > 0.05 && b > 0.05, "440 Hz at {a}, 880 Hz at {b}");

    // Save, reopen: the same document, and the same sound.
    let file = dir.join("audio-panel.freecut");
    let saved = project.save_job(file.clone()).run();
    assert!(saved.result.is_ok(), "{:?}", saved.result);
    project.saved(&saved);
    let before = project.doc().clone();
    drop(project);
    let reopened = Project::from_loaded(open_project(&file).unwrap());
    assert_eq!(reopened.doc(), &before);
    assert!(reopened.offline.is_empty());
    let out = mix_second(&reopened);
    let (a, b) = (
        tone_level(&out, 48_000, 440.0),
        tone_level(&out, 48_000, 880.0),
    );
    assert!(
        a > 0.05 && b > 0.05,
        "reopened: 440 Hz at {a}, 880 Hz at {b}"
    );
}

#[test]
fn importing_a_video_as_audio_extracts_its_sound_in_one_step() {
    let dir = tempfile_dir("audio-import-video");
    let Some((landscape, _)) = fixtures(&dir) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    freecut_media::init().unwrap();
    let mut project = new_project();
    let (kept, skipped) = sound_only(vec![probe_for_import(&landscape, None).unwrap()]);
    assert_eq!(skipped, 0);
    let listed = project.import_audio(kept).unwrap();
    assert_eq!(
        project.editor.history().len(),
        1,
        "import and extract, one step"
    );
    let library = project.doc().library();
    assert_eq!(library.len(), 2);
    let item = library.get(listed[0]).unwrap();
    assert!(matches!(item.source, MediaSource::AudioOf { .. }));
    assert_eq!(
        audio_entries(library)
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>(),
        listed
    );
    project.undo().unwrap();
    assert!(project.doc().library().is_empty());
    project.redo().unwrap();
    assert_eq!(project.doc().library().len(), 2);
}

/// Renders the project's playhead frame, with an uncommitted text draft.
fn render_draft(
    project: &Project,
    preview: &Preview,
    generation: u64,
    text_draft: Option<crate::preview::TextDraft>,
) -> PreviewFrame {
    preview.request(PreviewRequest {
        doc: Arc::new(project.doc().clone()),
        media_paths: Arc::new(project.media_paths.clone()),
        offline: Arc::new(project.offline.clone()),
        time: project.playhead,
        max_size: (640, 360),
        text_draft,
        generation,
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(frame) = preview.poll()
            && frame.generation == generation
        {
            return frame;
        }
        assert!(Instant::now() < deadline, "preview timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The bounding box (x0, y0, x1, y1) of the clearly red pixels, if any.
fn red_box(frame: &PreviewFrame) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..frame.height {
        for x in 0..frame.width {
            let [r, g, b] = pixel(frame, x, y);
            if r > 200 && g < 60 && b < 60 {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    found
}

fn box_centre(b: (u32, u32, u32, u32)) -> (f64, f64) {
    (
        f64::from(b.0 + b.2 + 1) / 2.0,
        f64::from(b.1 + b.3 + 1) / 2.0,
    )
}

fn near(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

#[test]
fn text_is_added_styled_moved_rendered_undone_and_reopened() {
    use freecut_timeline::edit::{TextStyleEdit, TransformEdit};
    use freecut_timeline::glam::DVec2;
    use freecut_timeline::model::{CanvasPos, RefPx, Rgba8, TextClipData};

    use crate::ui::canvas::{contains, hit_test};
    use crate::ui::props::ContentSession;
    use crate::ui::text::add_text_at_playhead;

    let dir = tempfile_dir("text");
    let mut project = new_project(); // 640 × 360 at 30 fps
    let seq = project.doc().root();
    let preview = Preview::new(FontRegistry::bundled(), presets(), || {});

    // The Text tab's tile at the playhead (TX-001): 3 s, selected.
    project.playhead = Tick::from_secs(1).unwrap();
    let clip = add_text_at_playhead(&mut project).unwrap();
    assert_eq!(project.selection, [clip]);
    let placed = project.doc().clip(clip).unwrap().clone();
    assert_eq!(placed.start(), Tick::from_secs(1).unwrap());
    assert_eq!(placed.duration(), Tick::from_secs(3).unwrap());
    assert_eq!(
        placed.content.text().unwrap().content.as_str(),
        TextClipData::PLACEHOLDER
    );
    let (_, track) = project
        .doc()
        .sequence(seq)
        .unwrap()
        .find_clip(clip)
        .unwrap();
    assert_eq!(track.kind, TrackKind::Text);
    undo_redo(&mut project);
    assert_eq!(project.selection, [clip]);

    // A content edit session (TX-002) commits once, as one undo entry.
    let steps = project.editor.history().len();
    let mut session = ContentSession::new(clip, TextClipData::PLACEHOLDER);
    for typed in ["H", "HI", "HIT"] {
        session.draft = typed.to_owned();
    }
    project
        .apply(&session.request(seq).unwrap(), Coalesce::Never)
        .unwrap();
    assert_eq!(project.editor.history().len(), steps + 1);
    assert_eq!(
        project
            .doc()
            .clip(clip)
            .unwrap()
            .content
            .text()
            .unwrap()
            .content
            .as_str(),
        "HIT"
    );

    // Style (TX-004): big, bold, red.
    for edit in [
        TextStyleEdit::Size(RefPx::from_px(150)),
        TextStyleEdit::Fill(Rgba8::new(255, 0, 0, 255).into()),
        TextStyleEdit::Bold(true),
    ] {
        project
            .apply(
                &EditRequest::SetTextStyle {
                    seq,
                    clips: vec![clip],
                    edit,
                },
                Coalesce::Never,
            )
            .unwrap();
    }
    undo_redo(&mut project);

    // Centred by default (TX-001). (Undoing the insert above moved the
    // playhead back to the end of the then empty timeline.)
    project.playhead = Tick::from_secs(2).unwrap();
    let centred = render_draft(&project, &preview, 1, None);
    write_ppm(&centred, "text_centred.ppm");
    let red = red_box(&centred).expect("red text is drawn");
    assert!(
        near(box_centre(red), (320.0, 180.0), 4.0),
        "centred: {red:?}"
    );
    // The frame reports the text box: the pointer there picks the clip.
    let layer = *centred.layers.iter().find(|l| l.clip == clip).unwrap();
    assert!(contains(&layer, DVec2::new(320.0, 180.0)));
    assert!(!contains(&layer, DVec2::new(20.0, 20.0)));
    assert_eq!(
        hit_test(&centred.layers, DVec2::new(320.0, 180.0), |_| true),
        Some(clip)
    );

    // Moved by (+160, −90) project px (TR-001), as a canvas drag would.
    let to = CanvasPos::from_px(160.0, -90.0, 640, 360);
    project
        .apply(
            &EditRequest::SetTransform {
                seq,
                clips: vec![clip],
                edit: TransformEdit::Position(to),
            },
            Coalesce::Never,
        )
        .unwrap();
    let moved = render_draft(&project, &preview, 2, None);
    write_ppm(&moved, "text_moved.ppm");
    let red = red_box(&moved).expect("red text is drawn");
    assert!(near(box_centre(red), (480.0, 90.0), 4.0), "moved: {red:?}");
    assert_eq!(pixel(&moved, 320, 180), [0, 0, 0], "nothing left behind");
    assert_eq!(
        hit_test(&moved.layers, DVec2::new(480.0, 90.0), |_| true),
        Some(clip)
    );

    // An uncommitted draft shows in the preview without touching the
    // document; an empty draft draws nothing (TX-002).
    let revision = project.editor.revision();
    let empty = render_draft(
        &project,
        &preview,
        3,
        Some(crate::preview::TextDraft {
            clip,
            content: String::new(),
        }),
    );
    assert_eq!(red_box(&empty), None);
    assert_eq!(project.editor.revision(), revision);

    // Undo puts it back in the centre, redo moves it again.
    project.undo().unwrap();
    let undone = render_draft(&project, &preview, 4, None);
    assert!(near(
        box_centre(red_box(&undone).unwrap()),
        (320.0, 180.0),
        4.0
    ));
    project.redo().unwrap();
    let redone = render_draft(&project, &preview, 5, None);
    assert_eq!(redone.rgba, moved.rgba, "redo renders the same frame");

    // Save, reopen: the same document and the same picture.
    let file = dir.join("text.freecut");
    let saved = project.save_job(file.clone()).run();
    assert!(saved.result.is_ok(), "{:?}", saved.result);
    project.saved(&saved);
    let before = project.doc().clone();
    drop(project);
    let reopened = Project::from_loaded(open_project(&file).unwrap());
    assert_eq!(reopened.doc(), &before);
    let mut reopened = reopened;
    reopened.playhead = Tick::from_secs(2).unwrap();
    let again = render_draft(&reopened, &preview, 6, None);
    assert_eq!(
        again.rgba, moved.rgba,
        "the reopened project renders the same"
    );
}

/// A project of exactly `seconds`: a blue 320×180 video with a 440 Hz tone
/// on the main track, and big red bold "HIT" text centred above it for the
/// whole length (the video loops its 2 s source only once, so a longer
/// project shows black after 2 s).
fn export_project(dir: &Path, seconds: i64) -> Option<Project> {
    use freecut_timeline::edit::{InsertReq, NewClip};
    use freecut_timeline::model::{RefPx, Rgba8, TextClipData, TextStyle};

    let video = dir.join("blue.mp4");
    let ok = ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=0x2040c0:size=320x180:rate=30:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        video.to_str()?,
    ]);
    if !ok {
        return None;
    }
    freecut_media::init().unwrap();
    let settings = ProjectSettings {
        width: 320,
        height: 180,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        ..ProjectSettings::new_default().unwrap()
    };
    let mut project = Project::new(settings).unwrap();
    let id = project
        .import(vec![probe_for_import(&video, None).unwrap()])
        .unwrap()[0];
    let seq = project.doc().root();
    let main = project
        .doc()
        .sequence(seq)
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    let two = Tick::from_secs(2).unwrap();
    project
        .apply(
            &EditRequest::Insert(InsertReq {
                seq,
                target: InsertTarget::Track(main),
                at: Tick::ZERO,
                clips: vec![NewClip::media(
                    id,
                    MediaKind::Video,
                    TimeRange::new(Tick::ZERO, two).unwrap(),
                )],
            }),
            Coalesce::Never,
        )
        .unwrap();
    let text = TextClipData {
        style: TextStyle {
            size: RefPx::from_px(400),
            fill: Rgba8::new(255, 0, 0, 255).into(),
            bold: true,
            ..TextStyle::default()
        },
        ..TextClipData::with_content("HIT")
    };
    project
        .apply(
            &EditRequest::Insert(InsertReq {
                seq,
                target: InsertTarget::FirstFree,
                at: Tick::ZERO,
                clips: vec![NewClip::text(text, Tick::from_secs(seconds).unwrap())],
            }),
            Coalesce::Never,
        )
        .unwrap();
    assert_eq!(
        project.doc().sequence(seq).unwrap().duration(),
        Tick::from_secs(seconds).unwrap()
    );
    Some(project)
}

fn export_request(
    project: &Project,
    folder: &Path,
    name: &str,
    conflict: freecut_project::export::ConflictChoice,
) -> crate::jobs::export::ExportRequest {
    let mut settings = freecut_project::export::ExportSettings::defaults(
        name,
        Some(folder.to_path_buf()),
        project.doc(),
    );
    assert!(settings.audio.include, "the timeline has sound");
    settings.audio.sample_rate = SampleRate::new(48_000).unwrap();
    crate::jobs::export::ExportRequest {
        doc: Arc::new(project.doc().clone()),
        seq: project.doc().root(),
        media_paths: project.media_paths.clone(),
        offline: project.offline.clone(),
        settings,
        conflict,
        fonts: FontRegistry::bundled(),
        presets: presets(),
        solo: None,
        luts: Arc::new(crate::luts::AppLuts::new()),
        slates_for_unreadable: false,
    }
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The bounding box of clearly red pixels in straight RGBA, and their count.
fn red_pixels(rgba: &[u8], width: u32) -> (usize, Option<(u32, u32, u32, u32)>) {
    let mut count = 0;
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for (i, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if px[0] > 180 && px[1] < 90 && px[2] < 110 {
            count += 1;
            let (x, y) = (i as u32 % width, i as u32 / width);
            found = Some(match found {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    (count, found)
}

#[test]
fn export_job_writes_a_verified_mp4_with_text_and_sound() {
    use freecut_media::encode::verify_output;
    use freecut_media::{AudioReader, SeekMode, VideoDecoder};
    use freecut_project::export::ConflictChoice;

    use crate::jobs::export::{ExportEnd, ExportJob};
    use crate::jobs::progress::Phase;

    let dir = tempfile_dir("export");
    let Some(project) = export_project(&dir, 2) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    if !crate::jobs::platform::encoder_available() {
        eprintln!("skipped: this FFmpeg has no libx264");
        return;
    }
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let request = export_request(&project, &out, "Export: test", ConflictChoice::KeepBoth);
    let mut job = ExportJob::start(request.clone(), || {});
    let end = job.wait(Duration::from_secs(300)).expect("the export ends");
    let ExportEnd::Done(result) = end else {
        panic!("export failed: {end:?}");
    };
    assert_eq!(job.progress().phase, Phase::Ended);
    // The name is sanitized (EX-023); nothing else is left in the folder.
    assert_eq!(result.path, out.join("Export_ test.mp4"));
    assert_eq!(files_in(&out), ["Export_ test.mp4"]);
    assert_eq!((result.size, result.frames), ((320, 180), 60));
    assert_eq!(result.duration, Tick::from_secs(2).unwrap());
    assert!(result.audio);
    assert_eq!(result.bytes, std::fs::metadata(&result.path).unwrap().len());

    // Read back: H.264 High + AAC, 60 frames, 2 s, metadata only as EX-040
    // allows.
    let info = verify_output(&result.path).unwrap();
    let v = info.video.clone().unwrap();
    assert_eq!(
        (v.codec.as_str(), v.profile.as_deref()),
        ("h264", Some("High"))
    );
    assert_eq!((v.width, v.height, v.packets), (320, 180, 60));
    assert_eq!(v.duration, Some(Tick::from_secs(2).unwrap()));
    let a = info.audio.clone().unwrap();
    assert_eq!(
        (a.codec.as_str(), a.sample_rate, a.channels),
        ("aac", 48_000, 2)
    );
    let samples = SampleRate::new(48_000)
        .unwrap()
        .sample_index(a.duration.unwrap())
        .unsigned_abs();
    assert!(samples.abs_diff(96_000) <= 1024, "{samples} audio samples");
    assert_eq!(info.index_before_media(), Some(true), "fast start");
    for key in &info.metadata_keys {
        assert!(
            [
                "creation_time",
                "title",
                "encoder",
                "major_brand",
                "minor_version",
                "compatible_brands"
            ]
            .contains(&key.as_str()),
            "unexpected metadata {key}"
        );
    }

    // The frame at 1 s: the red text centred over the blue video.
    let mut dec = VideoDecoder::open(&result.path, v.index).unwrap();
    let frame = dec
        .frame_at(Tick::from_secs(1).unwrap(), SeekMode::Exact)
        .unwrap();
    assert_eq!((frame.width, frame.height), (320, 180));
    let (count, bbox) = red_pixels(&frame.rgba, frame.width);
    let bbox = bbox.expect("the text is in the exported frame");
    assert!(count > 300, "{count} red pixels");
    let centre = (
        f64::from(bbox.0 + bbox.2 + 1) / 2.0,
        f64::from(bbox.1 + bbox.3 + 1) / 2.0,
    );
    assert!(near(centre, (160.0, 90.0), 8.0), "text box {bbox:?}");
    let corner = &frame.rgba[..4];
    assert!(
        corner[2] > 150 && corner[0] < 80,
        "blue video behind: {corner:?}"
    );

    // The sound: the video's 440 Hz tone.
    let mut reader =
        AudioReader::open(&result.path, a.index, SampleRate::new(48_000).unwrap(), 2).unwrap();
    let pcm = reader
        .read(
            Tick::from_raw(Tick::from_secs(1).unwrap().raw() / 2),
            48_000,
        )
        .unwrap();
    let level = tone_level(&pcm, 48_000, 440.0);
    assert!(level > 0.05, "440 Hz at {level}");

    // Again with Keep both: a second file, the first untouched.
    let first = std::fs::read(&result.path).unwrap();
    let mut again = ExportJob::start(request.clone(), || {});
    let ExportEnd::Done(second) = again.wait(Duration::from_secs(300)).unwrap() else {
        panic!("second export failed");
    };
    assert_eq!(second.path, out.join("Export_ test (2).mp4"));
    assert_eq!(std::fs::read(&result.path).unwrap(), first);

    // Replace: the plain name is rewritten in place, no third file.
    let replace = crate::jobs::export::ExportRequest {
        conflict: ConflictChoice::Replace,
        ..request
    };
    let mut third = ExportJob::start(replace, || {});
    let ExportEnd::Done(replaced) = third.wait(Duration::from_secs(300)).unwrap() else {
        panic!("replace export failed");
    };
    assert_eq!(replaced.path, result.path);
    assert_eq!(files_in(&out), ["Export_ test (2).mp4", "Export_ test.mp4"]);
}

#[test]
fn cancelled_exports_leave_no_files() {
    use freecut_project::export::ConflictChoice;

    use crate::jobs::export::{ExportEnd, ExportJob};

    let dir = tempfile_dir("export-cancel");
    // 30 s: long enough that the job is still rendering when cancelled.
    let Some(project) = export_project(&dir, 30) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    if !crate::jobs::platform::encoder_available() {
        eprintln!("skipped: this FFmpeg has no libx264");
        return;
    }
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("keep.mp4"), b"an older export").unwrap();

    // Cancelled while rendering.
    let request = export_request(&project, &out, "keep", ConflictChoice::Replace);
    let mut job = ExportJob::start(request.clone(), || {});
    let deadline = Instant::now() + Duration::from_secs(120);
    while job.progress().frames_rendered < 10 {
        assert!(Instant::now() < deadline, "the export never started");
        assert!(job.poll().is_none(), "ended before it could be cancelled");
        std::thread::sleep(Duration::from_millis(1));
    }
    let rendered = job.progress().frames_rendered;
    assert!(rendered < 900, "still rendering ({rendered} of 900)");
    let asked = Instant::now();
    job.cancel();
    let end = job
        .wait(Duration::from_secs(30))
        .expect("cancel ends the job");
    let latency = asked.elapsed();
    eprintln!("cancel took {latency:?}");
    assert_eq!(end, ExportEnd::Cancelled);
    assert!(latency < Duration::from_secs(3), "cancel took {latency:?}");
    // No .part, and Replace's old file is untouched.
    assert_eq!(files_in(&out), ["keep.mp4"]);
    assert_eq!(
        std::fs::read(out.join("keep.mp4")).unwrap(),
        b"an older export"
    );

    // Cancelled before it really started; dropping a running job cancels
    // it too.
    let mut early = ExportJob::start(request.clone(), || {});
    early.cancel();
    assert_eq!(
        early.wait(Duration::from_secs(30)),
        Some(ExportEnd::Cancelled)
    );
    drop(ExportJob::start(request, || {}));
    assert_eq!(files_in(&out), ["keep.mp4"]);
}

/// 2 s of grey picture with two audio streams: a mono 440 Hz tone (stream
/// 1, the default) and a stereo 1 kHz tone (stream 2), both at 0.4, as
/// float PCM in Matroska.
fn two_stream_mkv(dir: &Path) -> Option<PathBuf> {
    let path = dir.join("two-streams.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=gray:s=160x120:r=30:d=2",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.4*sin(2*PI*440*t):c=mono:s=48000:d=2",
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.4*sin(2*PI*1000*t)|0.4*sin(2*PI*1000*t):c=stereo:s=48000:d=2",
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-map",
        "2:a",
        "-c:v",
        "mpeg4",
        "-c:a",
        "pcm_f32le",
        path.to_str()?,
    ])
    .then_some(path)
}

/// The 440 Hz and 1 kHz levels of 1 s..2 s of the project's mix.
fn tone_levels(project: &Project) -> (f64, f64) {
    let out = mix_second(project);
    (
        tone_level(&out, 48_000, 440.0),
        tone_level(&out, 48_000, 1_000.0),
    )
}

fn near_level(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 0.02, "{what}: {got} vs {want}");
}

#[test]
fn a_two_stream_file_switches_streams_mixes_them_and_adds_one_as_a_clip() {
    use freecut_timeline::edit::sync_offsets;
    use freecut_timeline::model::StreamSel;

    use crate::clip_streams::{
        ALL_MIXED, AddStreamState, add_stream_as_clips, add_stream_menu, can_resync, resync,
        set_stream, stream_picker, sync_badge,
    };

    let dir = tempfile_dir("streams");
    let Some(file) = two_stream_mkv(&dir) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    freecut_media::init().unwrap();
    let mut project = new_project();
    let seq = project.doc().root();

    // The import notice names the count and the default stream (AU-038).
    let media = project
        .import(vec![probe_for_import(&file, None).unwrap()])
        .unwrap()[0];
    let notes = project.take_notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].starts_with("two-streams.mkv has 2 audio streams; the default one (stream 1)"),
        "{notes:?}"
    );

    let main = project
        .doc()
        .sequence(seq)
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    insert_media(&mut project, media, InsertTarget::Track(main), Tick::ZERO).unwrap();
    let video = project.selection[0];

    // The picker lists both streams and the mix; the default plays.
    let picker = stream_picker(project.doc(), video).unwrap();
    let labels: Vec<&str> = picker.choices.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels.len(), 3, "{labels:?}");
    assert!(labels[0].starts_with("Stream 1 · mono"), "{labels:?}");
    assert!(labels[1].starts_with("Stream 2 · stereo"), "{labels:?}");
    assert_eq!(labels[2], ALL_MIXED);
    assert_eq!(picker.current, 0);
    let (a, b) = tone_levels(&project);
    near_level(a, 0.4, "default stream: 440 Hz");
    assert!(b < 0.01, "default stream: no 1 kHz ({b})");

    // Switching to stream 2 changes what plays.
    let second = picker.choices[1].sel;
    assert!(matches!(second, Some(StreamSel::Index(_))));
    set_stream(&mut project, &[video], second).unwrap();
    let (a, b) = tone_levels(&project);
    assert!(a < 0.01, "stream 2: no 440 Hz ({a})");
    near_level(b, 0.4, "stream 2: 1 kHz");

    // All streams mixed: each at 1/√2.
    set_stream(&mut project, &[video], Some(StreamSel::AllMixed)).unwrap();
    let each = 0.4 * freecut_audio::stream_mix_factor(2);
    let (a, b) = tone_levels(&project);
    near_level(a, f64::from(each), "all mixed: 440 Hz");
    near_level(b, f64::from(each), "all mixed: 1 kHz");
    assert_eq!(stream_picker(project.doc(), video).unwrap().current, 2);

    // Back to the default, then stream 2 as its own linked clip.
    set_stream(&mut project, &[video], None).unwrap();
    let menu = add_stream_menu(&project.editor, &[video]);
    assert!(matches!(
        menu[0].state,
        AddStreamState::AlreadyPlaying(Ok(()))
    ));
    assert_eq!(menu[1].state, AddStreamState::Add);
    assert_eq!(add_stream_as_clips(&mut project, &[video], 1), Ok(1));
    let added = project.selection[0];
    let doc = project.doc().clone();
    let link = doc.clip(video).unwrap().link;
    assert!(link.is_some());
    assert_eq!(doc.clip(added).unwrap().link, link);
    assert_eq!(
        doc.sequence(seq).unwrap().find_clip(added).unwrap().1.kind,
        TrackKind::Audio
    );
    // Lined up: no badge, and both tones play.
    assert!(sync_offsets(&doc, seq).is_empty());
    assert!(!can_resync(&doc, seq, &[video, added]));
    let (a, b) = tone_levels(&project);
    near_level(a, 0.4, "video's own stream");
    near_level(b, 0.4, "the added stream");

    // Moving the added clip 3 frames late badges both clips.
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
    let offsets = sync_offsets(&doc, seq);
    assert_eq!(sync_badge(&offsets[&added], frame), "+3f");
    assert_eq!(sync_badge(&offsets[&video], frame), "+3f");
    assert!(can_resync(&doc, seq, &[video]));

    // Resync from the video puts the sound back, one undo entry.
    let moved = doc.clone();
    resync(&mut project, &[video]).unwrap();
    assert!(sync_offsets(project.doc(), seq).is_empty());
    assert_eq!(project.doc().clip(added).unwrap().start(), Tick::ZERO);
    project.undo().unwrap();
    assert_eq!(project.doc(), &moved);
    undo_redo(&mut project);
    project.redo().unwrap();
    assert!(sync_offsets(project.doc(), seq).is_empty());
}

/// A 4 s mono sine at `freq` Hz, amplitude 1/8 (ffmpeg's default).
fn mono_tone(dir: &Path, freq: u32) -> Option<PathBuf> {
    let wav = dir.join(format!("mono-{freq}.wav"));
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency={freq}:duration=4"),
        "-ac",
        "1",
        wav.to_str()?,
    ])
    .then_some(wav)
}

/// The `freq` Hz level in the left and right channels of `file`'s sound
/// over `[from, from + 0.1 s)` (whole cycles of 440 Hz and 1 kHz).
fn exported_level(file: &Path, from: f64, freq: f64) -> (f64, f64) {
    use freecut_media::AudioReader;
    use freecut_media::encode::verify_output;

    let stream = verify_output(file).unwrap().audio.unwrap().index;
    let rate = SampleRate::new(48_000).unwrap();
    let mut reader = AudioReader::open(file, stream, rate, 2).unwrap();
    let start = Tick::from_raw((from * freecut_timeline::time::TICKS_PER_SECOND as f64) as i64);
    let pcm = reader.read(start, 4_800).unwrap();
    let right: Vec<f32> = pcm.iter().skip(1).copied().chain([0.0]).collect();
    (
        tone_level(&pcm, 48_000, freq),
        tone_level(&right, 48_000, freq),
    )
}

/// The export-level check of AU-006, AU-008, AU-010 and AU-012 in the file
/// itself: a mono source keeps its level in stereo, a keyframed dip and a
/// fade curve are measurable, and solo is ignored unless "Export as heard".
#[test]
fn exported_sound_keeps_levels_keyframes_fades_and_solo() {
    use std::collections::BTreeSet;

    use freecut_project::export::ConflictChoice;
    use freecut_timeline::edit::{AudioEdit, GainKeyEdit};
    use freecut_timeline::model::{FadeCurve, GainDb};

    use crate::jobs::export::{ExportEnd, ExportJob};
    use crate::ui::export::ExportDialog;

    let dir = tempfile_dir("export-levels");
    let (Some(low), Some(high)) = (mono_tone(&dir, 440), mono_tone(&dir, 1_000)) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    if !crate::jobs::platform::encoder_available() {
        eprintln!("skipped: this FFmpeg has no libx264");
        return;
    }
    freecut_media::init().unwrap();
    let mut project = Project::new(ProjectSettings {
        width: 320,
        height: 180,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        ..ProjectSettings::new_default().unwrap()
    })
    .unwrap();
    let seq = project.doc().root();
    let ids = project
        .import_audio(vec![
            probe_for_import(&low, None).unwrap(),
            probe_for_import(&high, None).unwrap(),
        ])
        .unwrap();
    let mut placed = Vec::new();
    for id in ids {
        project.playhead = Tick::ZERO;
        add_at_playhead(&mut project, id).unwrap();
        let clip = project.selection[0];
        let track = project
            .doc()
            .sequence(seq)
            .unwrap()
            .find_clip(clip)
            .unwrap()
            .1
            .id;
        placed.push((clip, track));
    }
    let [(tone, tone_track), (other, other_track)] = placed[..] else {
        panic!("two clips: {placed:?}");
    };
    assert_ne!(tone_track, other_track);
    let s =
        |secs: f64| Tick::from_raw((secs * freecut_timeline::time::TICKS_PER_SECOND as f64) as i64);

    // The 440 Hz clip: a 1 s "slow" fade-in (1 − cos(π·p/2): 29 % at
    // half-way) and a dip to −20 dB between 2.2 s and 2.8 s.
    for edit in [
        AudioEdit::FadeIn(s(1.0)),
        AudioEdit::FadeInCurve(FadeCurve::Slow),
    ] {
        project
            .apply(
                &EditRequest::SetClipAudio {
                    seq,
                    clips: vec![tone],
                    edit,
                },
                Coalesce::Never,
            )
            .unwrap();
    }
    for (at, hundredths) in [(2.0, 0), (2.2, -2_000), (2.8, -2_000), (3.0, 0)] {
        project
            .apply(
                &EditRequest::EditGainKeys {
                    seq,
                    clips: vec![tone],
                    edit: GainKeyEdit::Set {
                        at: s(at),
                        gain: Some(GainDb::from_hundredths(hundredths)),
                    },
                },
                Coalesce::Never,
            )
            .unwrap();
    }
    // Only the 440 Hz track is soloed.
    project.set_solo(BTreeSet::from([tone_track]));
    assert!(project.doc().clip(other).is_some());

    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let export = |project: &Project, as_heard: bool, name: &str| {
        let mut dialog = ExportDialog::new(project, Some(out.clone()));
        dialog.as_heard = as_heard;
        let mut request = export_request(project, &out, name, ConflictChoice::Replace);
        request.solo = dialog.export_solo(project);
        let mut job = ExportJob::start(request, || {});
        match job.wait(Duration::from_secs(300)).expect("the export ends") {
            ExportEnd::Done(result) => result.path,
            end => panic!("export failed: {end:?}"),
        }
    };

    // Default export: solo is ignored, both tones are in the file.
    let all = export(&project, false, "all");
    let (left, right) = exported_level(&all, 1.4, 440.0);
    // A mono source plays at its own level on both sides (AU-010).
    assert!((left - 0.125).abs() < 0.0125, "mono 440 Hz left at {left}");
    assert!(
        (right - 0.125).abs() < 0.0125,
        "mono 440 Hz right at {right}"
    );
    let (other_level, _) = exported_level(&all, 1.4, 1_000.0);
    assert!(
        (other_level - 0.125).abs() < 0.0125,
        "the unsoloed track plays: 1 kHz at {other_level}"
    );
    // The fade curve, half-way through the fade-in.
    let (mid_fade, _) = exported_level(&all, 0.45, 440.0);
    let ratio = mid_fade / left;
    assert!(
        (ratio - 0.293).abs() < 0.05,
        "slow fade-in at half-way: {ratio:.3} of the level"
    );
    // The keyframed dip.
    let (dip, _) = exported_level(&all, 2.45, 440.0);
    let ratio = dip / left;
    assert!(
        (ratio - 0.1).abs() < 0.02,
        "−20 dB dip: {ratio:.3} of the level"
    );
    let (after, _) = exported_level(&all, 3.4, 440.0);
    assert!((after / left - 1.0).abs() < 0.1, "back up after the dip");

    // "Export as heard": only the soloed track.
    let heard = export(&project, true, "heard");
    let (left_heard, _) = exported_level(&heard, 1.4, 440.0);
    assert!((left_heard - left).abs() < 0.0125, "the soloed tone stays");
    let (silent, _) = exported_level(&heard, 1.4, 1_000.0);
    assert!(silent < 0.003, "the unsoloed track is left out: {silent}");
}

/// EX-022 `decode-error`: a source damaged after import stops the export,
/// naming the clip and time; "Retry using slates" finishes it with the
/// error slate and lists the frames.
#[test]
fn an_unreadable_source_stops_the_export_unless_slates_are_asked_for() {
    use freecut_project::export::ConflictChoice;

    use crate::jobs::export::{ExportEnd, ExportJob, FailureCode, FixAction};

    let dir = tempfile_dir("export-decode");
    let Some(project) = export_project(&dir, 2) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    if !crate::jobs::platform::encoder_available() {
        eprintln!("skipped: this FFmpeg has no libx264");
        return;
    }
    let video = dir.join("blue.mp4");
    let bytes = std::fs::read(&video).unwrap();
    std::fs::write(&video, &bytes[..64]).unwrap();
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let mut request = export_request(&project, &out, "damaged", ConflictChoice::KeepBoth);
    request.settings.audio.include = false;

    let mut job = ExportJob::start(request.clone(), || {});
    let ExportEnd::Failed(failure) = job.wait(Duration::from_secs(300)).unwrap() else {
        panic!("the export should stop");
    };
    assert_eq!(failure.code, FailureCode::DecodeError);
    assert_eq!(failure.code.id(), "decode-error");
    assert_eq!(failure.code.fixes(), [FixAction::RetryWithSlates]);
    let message = failure.message();
    assert!(
        message.contains("“blue.mp4” at 00:00:00:00"),
        "names the clip and time: {message}"
    );
    assert!(
        files_in(&out).is_empty(),
        "no .part left: {:?}",
        files_in(&out)
    );

    let retry = crate::jobs::export::ExportRequest {
        slates_for_unreadable: true,
        ..request
    };
    let mut job = ExportJob::start(retry, || {});
    let ExportEnd::Done(result) = job.wait(Duration::from_secs(300)).unwrap() else {
        panic!("the slate retry should finish");
    };
    assert_eq!(result.frames, 60);
    assert_eq!(result.slated.len(), 60, "every frame of the clip is slated");
    assert_eq!(result.slated[0], "“blue.mp4” at 00:00:00:00");
    assert_eq!(files_in(&out), ["damaged.mp4"]);
}

/// Applies `edit` to `clip` as the Style and Box & bubble cards do (one
/// `SetTextStyle` entry), then checks its undo and redo.
fn style_title(
    project: &mut Project,
    clip: freecut_timeline::ids::ClipId,
    edit: freecut_timeline::edit::TextStyleEdit,
) {
    let seq = project.doc().root();
    let req = EditRequest::SetTextStyle {
        seq,
        clips: vec![clip],
        edit,
    };
    project.apply(&req, Coalesce::Never).unwrap();
    undo_redo(project);
}

/// Applies `edit` to `clip` as the Paragraph and Spacing cards do.
fn lay_out_title(
    project: &mut Project,
    clip: freecut_timeline::ids::ClipId,
    edit: freecut_timeline::edit::TextLayoutEdit,
) {
    let seq = project.doc().root();
    let req = EditRequest::SetTextLayout {
        seq,
        clips: vec![clip],
        edit,
    };
    project.apply(&req, Coalesce::Never).unwrap();
    undo_redo(project);
}

/// Text round 2 (S1): three titles carrying every round-2 layer between
/// them (gradient fills, case, letter spacing, decorations, outline,
/// shadow, glow both ways, a bubble with its tail, per-line boxes with a
/// border, justify, the bottom anchor and a curve), set with the requests
/// the cards send, each undone and redone. The preview frame and the
/// export frame of the same time at the same size are byte-identical, and
/// so is the reopened project's.
#[test]
fn a_title_with_every_style_layer_previews_and_exports_identically() {
    use freecut_render::Compositor;
    use freecut_render::export::ExportRenderer;
    use freecut_render::media::MediaImages;
    use freecut_timeline::edit::{ResetScope, TextLayoutEdit, TextStyleEdit as S, TransformEdit};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{
        BoxShape, CanvasPos, CaseTransform, DeciDegrees, FillCenter, FillKind, GlowDirection,
        HAlign, PerMille, RefPx, Rgba8, RichText, SignedPerMille, TailTip, TextLayerKind, TextLens,
        VAnchor,
    };

    let dir = tempfile_dir("styled");
    let mut project = new_project();
    let seq = project.doc().root();
    let title = |project: &mut Project, text: &str, y: i32| {
        let clip = crate::ui::text::add_text_at_playhead(project).unwrap();
        let req = EditRequest::SetTextContent {
            seq,
            clip,
            content: RichText::new(text),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        let req = EditRequest::SetTransform {
            seq,
            clips: vec![clip],
            edit: TransformEdit::Position(CanvasPos::from_px(0.0, f64::from(y), 640, 360)),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        clip
    };
    let speech = title(&mut project, "Hello there", -110);
    let boxed = title(&mut project, "Two short lines of boxed words", 110);
    let curved = title(&mut project, "Curved", 10);

    // Speech: capitals, spacing, underline, a linear gradient, outline,
    // shadow and glow both ways, in a bubble whose tail points down-left.
    for edit in [
        S::Size(RefPx::from_px(80)),
        S::Case(CaseTransform::Upper),
        S::LetterSpacing(SignedPerMille::new(80)),
        S::Underline(true),
        S::FillKind(FillKind::LinearGradient),
        S::FillAngle(DeciDegrees::from_degrees(30)),
        S::FillStopColor {
            index: 1,
            color: Rgba8::new(255, 60, 0, 255),
        },
        S::SetLayer {
            layer: TextLayerKind::Outline,
            on: true,
        },
        S::OutlineWidth(PerMille::new(90)),
        S::SetLayer {
            layer: TextLayerKind::Shadow,
            on: true,
        },
        S::ShadowDistance(PerMille::new(150)),
        S::SetLayer {
            layer: TextLayerKind::Glow,
            on: true,
        },
        S::GlowDirection(GlowDirection::Both),
        S::GlowColor(Rgba8::new(0, 220, 255, 255)),
        S::SetLayer {
            layer: TextLayerKind::Bubble,
            on: true,
        },
        S::BubbleTailTip(TailTip {
            x: RefPx::from_px(-180),
            y: RefPx::from_px(170),
        }),
        S::BubbleTailBend(SignedPerMille::new(-400)),
    ] {
        style_title(&mut project, speech, edit);
    }
    // Boxes: per line with a border, italic and struck through, justified
    // at a fixed width, anchored at the bottom with roomy lines.
    for edit in [
        S::Size(RefPx::from_px(48)),
        S::Italic(true),
        S::Strike(true),
        S::SetLayer {
            layer: TextLayerKind::Background,
            on: true,
        },
        S::BackgroundShape(BoxShape::PerLine),
        S::BackgroundRoundness(PerMille::new(600)),
        S::BackgroundBorder(true),
        S::BackgroundColor(Rgba8::new(30, 40, 160, 255)),
    ] {
        style_title(&mut project, boxed, edit);
    }
    for edit in [
        TextLayoutEdit::BoxWidth(Some(RefPx::from_px(420))),
        TextLayoutEdit::Align(HAlign::Left),
        TextLayoutEdit::Justify(true),
        TextLayoutEdit::VAnchor(VAnchor::Bottom),
        TextLayoutEdit::LineSpacing(PerMille::new(1300)),
    ] {
        lay_out_title(&mut project, boxed, edit);
    }
    // Curved: an arch with a radial gradient and an inward glow.
    for edit in [
        S::Size(RefPx::from_px(70)),
        S::SetLayer {
            layer: TextLayerKind::Curve,
            on: true,
        },
        S::Curve(SignedPerMille::new(350)),
        S::FillKind(FillKind::RadialGradient),
        S::FillCenter(FillCenter {
            x: PerMille::new(300),
            y: PerMille::new(400),
        }),
        S::FillSpread(PerMille::new(1400)),
        S::SetLayer {
            layer: TextLayerKind::Glow,
            on: true,
        },
        S::GlowDirection(GlowDirection::Inward),
    ] {
        style_title(&mut project, curved, edit);
    }

    // Reset lens (TX-054) is one entry that undoes in one step.
    let styled = project.doc().clone();
    let req = EditRequest::ResetText {
        seq,
        clips: vec![speech, boxed, curved],
        scope: ResetScope::Lens(TextLens::Style),
    };
    project.apply(&req, Coalesce::Never).unwrap();
    assert_ne!(project.doc(), &styled);
    project.undo().unwrap();
    assert_eq!(project.doc(), &styled, "one undo restores every title");

    // The preview frame at 1 s, and the export frame of the same time.
    project.playhead = Tick::from_secs(1).unwrap();
    let preview = Preview::new(FontRegistry::bundled(), presets(), || {});
    let shown = render_draft(&project, &preview, 1, None);
    write_ppm(&shown, "text_styled.ppm");
    assert_eq!((shown.width, shown.height), (640, 360));
    let mut previewed = shown.rgba.clone();
    for px in previewed.as_chunks_mut::<4>().0 {
        // Export composites over black (an opaque canvas is unchanged).
        let a = u32::from(px[3]);
        if a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * a + 127) / 255) as u8;
            }
            px[3] = 255;
        }
    }
    let lit = previewed
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0].max(p[1]).max(p[2]) > 40)
        .count();
    assert!(lit > 5_000, "the titles are drawn: {lit} lit pixels");
    // The bottom-anchored title's box ends at its position (TX-005): the
    // frame's layer boxes follow the anchor, so Stage handles do too.
    let layer = shown.layers.iter().find(|l| l.clip == boxed).unwrap();
    let bottom = crate::ui::canvas::corners(layer)
        .iter()
        .map(|c| c.y)
        .fold(f64::MIN, f64::max);
    assert!(
        (bottom - (180.0 + 110.0)).abs() < 0.5,
        "bottom edge at {bottom}"
    );
    let export_frame = |project: &Project| {
        let rate = project.doc().settings().frame_rate;
        let range = TimeRange::new(project.playhead, rate.frame_ticks()).unwrap();
        let clock = ExportClock::new(range, rate).unwrap();
        let mut renderer = ExportRenderer::new(
            Arc::new(project.doc().clone()),
            project.doc().root(),
            clock,
            (640, 360),
            Compositor::new(FontRegistry::bundled()),
            MediaImages::new(),
        )
        .unwrap();
        renderer.render_frame(0).unwrap().rgba.clone()
    };
    let exported = export_frame(&project);
    assert!(
        exported == previewed,
        "the export frame equals the preview frame byte for byte ({} of {} bytes differ)",
        exported
            .iter()
            .zip(&previewed)
            .filter(|(a, b)| a != b)
            .count(),
        exported.len()
    );

    // Save and reopen: the same document and the same pictures.
    let file = dir.join("styled.freecut");
    let saved = project.save_job(file.clone()).run();
    assert!(saved.result.is_ok(), "{:?}", saved.result);
    project.saved(&saved);
    let before = project.doc().clone();
    let mut reopened = Project::from_loaded(open_project(&file).unwrap());
    assert_eq!(reopened.doc(), &before);
    reopened.playhead = Tick::from_secs(1).unwrap();
    let again = render_draft(&reopened, &preview, 2, None);
    assert_eq!(
        again.rgba, shown.rgba,
        "the reopened project previews the same"
    );
    assert!(export_frame(&reopened) == exported, "and exports the same");
}

/// Commits `edits` on `clips` as the inspector's cards do (props'
/// `commit`), each its own entry, then checks undo and redo of each.
fn card_edits(
    project: &mut Project,
    clips: &[freecut_timeline::ids::ClipId],
    edits: Vec<crate::ui::props::Edit>,
) {
    use crate::ui::drag::{DragGesture, Held};
    use crate::ui::props::{Merge, Steps, commit};
    for edit in edits {
        let mut steps = Steps::default();
        steps.edit(edit, Merge::Own);
        let failed = commit(project, &mut DragGesture::default(), clips, steps, Held::Up);
        assert_eq!(failed, None);
        undo_redo(project);
    }
}

/// Counts of lit pixels in `frame` (over black): all, mostly red, mostly
/// blue, and the lit columns' extent.
fn lit_stats(rgba: &[u8], width: u32) -> (usize, usize, usize, Option<(u32, u32)>) {
    let (mut lit, mut red, mut blue) = (0, 0, 0);
    let mut cols: Option<(u32, u32)> = None;
    for (i, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
        let [r, g, b, _] = *px;
        if r.max(g).max(b) < 60 {
            continue;
        }
        lit += 1;
        if r > b.saturating_add(60) && r > g.saturating_add(60) {
            red += 1;
        }
        if b > r.saturating_add(60) && b > g.saturating_add(60) {
            blue += 1;
        }
        let x = i as u32 % width;
        cols = Some(cols.map_or((x, x), |(a, b)| (a.min(x), b.max(x))));
    }
    (lit, red, blue, cols)
}

/// S2 acceptance (demo part 2): a title with a keyframed colour, a
/// per-letter rise In, a Wave Loop and a Fade Out, set through the app's
/// paths (the navigator's diamond, keyed edits, the Motion lens's
/// `SetClipAnimation`), each undone and redone. Preview frames change as
/// the motion says; export frames equal preview frames byte for byte; the
/// saved and reopened project is the same document and pictures.
#[test]
fn an_animated_title_previews_exports_undoes_and_reopens() {
    use freecut_render::Compositor;
    use freecut_render::export::ExportRenderer;
    use freecut_render::media::MediaImages;
    use freecut_timeline::edit::{KeyEdit, TextStyleEdit, TransformEdit};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{
        CanvasPos, Fill, PresetId, PropPath, RefPx, Rgba8, RichText, Slot, TextPropPath,
    };
    use freecut_timeline::motion::PresetCatalog;

    use crate::anim::{chosen_spec, slot_presets};
    use crate::ui::props::Edit;

    let ms = |n: i64| Tick::from_raw(freecut_timeline::time::TICKS_PER_SECOND / 1000 * n);
    let dir = tempfile_dir("animated");
    // The catalogue the app loads (Letter rise, Wave and Fade are TX-056's).
    let table = presets();

    let mut project = new_project();
    let seq = project.doc().root();
    let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
    for req in [
        EditRequest::SetTextContent {
            seq,
            clip,
            content: RichText::new("WAVES"),
        },
        EditRequest::SetTextStyle {
            seq,
            clips: vec![clip],
            edit: TextStyleEdit::Size(RefPx::from_px(200)),
        },
        EditRequest::SetTransform {
            seq,
            clips: vec![clip],
            edit: TransformEdit::Position(CanvasPos::from_px(0.0, 0.0, 640, 360)),
        },
    ] {
        project.apply(&req, Coalesce::Never).unwrap();
    }
    let clips = [clip];

    // The colour: a key at 0.2 s (the diamond: the value shown), then the
    // Fill row's edits key it red there and blue at 0.9 s (TR-027).
    let fill = PropPath::Text(TextPropPath::FillColor);
    let red = Rgba8::new(255, 0, 0, 255);
    let blue = Rgba8::new(0, 0, 255, 255);
    project.playhead = ms(200);
    card_edits(
        &mut project,
        &clips,
        vec![Edit::Keys {
            prop: fill,
            edit: KeyEdit::Add {
                at: ms(200),
                value: None,
            },
        }],
    );
    card_edits(
        &mut project,
        &clips,
        vec![Edit::Style(TextStyleEdit::Fill(Fill::Solid(red)))],
    );
    project.playhead = ms(900);
    card_edits(
        &mut project,
        &clips,
        vec![Edit::Style(TextStyleEdit::Fill(Fill::Solid(blue)))],
    );
    let keys = crate::prop_keys::shown_keys(project.doc().clip(clip).unwrap(), fill);
    assert_eq!(keys.len(), 2, "two colour keys");

    // The Motion lens's tiles: Letter rise In, Wave Loop, Fade Out.
    let pick = |slot: Slot, name: &str| {
        let choice = slot_presets(&table, slot, true)
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} in {slot:?}"));
        Edit::Animation {
            slot,
            spec: Some(chosen_spec(&choice)),
        }
    };
    card_edits(
        &mut project,
        &clips,
        vec![
            pick(Slot::Entrance, "Letter rise"),
            pick(Slot::Continuous, "Wave"),
            pick(Slot::Exit, "Fade"),
        ],
    );
    let c = project.doc().clip(clip).unwrap().clone();
    assert_eq!(
        c.animations.exit.as_ref().unwrap().preset,
        PresetId::new("fc.out.fade")
    );
    assert!(
        table
            .resolve(Slot::Exit, &PresetId::new("fc.out.fade"))
            .is_some()
    );
    // A 0.6 s cycle (the Cycle row), so whole cycles fit where the Loop
    // plays at full strength: TX-022 eases it in over [0.8, 1.05) after
    // the In and out over [2.25, 2.5) before the Out.
    let mut wave = (**c.animations.continuous.as_ref().unwrap()).clone();
    wave.duration = ms(600);
    card_edits(
        &mut project,
        &clips,
        vec![Edit::Animation {
            slot: Slot::Continuous,
            spec: Some(wave),
        }],
    );
    let c = project.doc().clip(clip).unwrap().clone();
    // In [0, 0.8), Loop [0.8, 2.5), Out [2.5, 3).
    let regions = crate::anim::shaded_regions(&c, ms(33));
    assert_eq!(regions.len(), 2);

    let preview = Preview::new(FontRegistry::bundled(), Arc::clone(&table), || {});
    let mut generation = 0;
    let mut frame_at = |project: &mut Project, t: Tick| {
        project.playhead = t;
        generation += 1;
        let frame = render_draft(project, &preview, generation, None);
        let mut rgba = frame.rgba.clone();
        for px in rgba.as_chunks_mut::<4>().0 {
            // Over black, as export composites.
            let a = u32::from(px[3]);
            if a != 255 {
                for c in &mut px[..3] {
                    *c = ((u32::from(*c) * a + 127) / 255) as u8;
                }
                px[3] = 255;
            }
        }
        (frame, rgba)
    };
    let export_frame = |project: &Project, t: Tick| {
        let rate = project.doc().settings().frame_rate;
        let range = TimeRange::new(t, rate.frame_ticks()).unwrap();
        let clock = ExportClock::new(range, rate).unwrap();
        let mut renderer = ExportRenderer::new(
            Arc::new(project.doc().clone()),
            project.doc().root(),
            clock,
            (640, 360),
            Compositor::new(FontRegistry::bundled()),
            MediaImages::new(),
        )
        .unwrap()
        .with_presets(Arc::clone(&table) as Arc<dyn PresetCatalog + Send + Sync>);
        renderer.render_frame(0).unwrap().rgba.clone()
    };

    // In: nothing at the first frame; letters rise in reading order.
    let (_, start) = frame_at(&mut project, Tick::ZERO);
    let (lit0, _, _, _) = lit_stats(&start, 640);
    assert!(lit0 < 50, "every letter starts transparent: {lit0}");
    let (shown, early) = frame_at(&mut project, ms(300));
    write_ppm(&shown, "animated_in.ppm");
    let (lit_early, red_early, blue_early, cols) = lit_stats(&early, 640);
    let (_, rest) = frame_at(&mut project, ms(1_000));
    let (lit_rest, red_rest, blue_rest, rest_cols) = lit_stats(&rest, 640);
    assert!(
        lit_early > 100 && lit_early < lit_rest,
        "{lit_early} of {lit_rest}"
    );
    let (cols, rest_cols) = (cols.unwrap(), rest_cols.unwrap());
    assert!(
        cols.1 + 40 < rest_cols.1,
        "the last letters aren't there yet: {cols:?} of {rest_cols:?}"
    );
    // Keyed colour: mostly red early, blue from 0.9 s.
    assert!(
        red_early > blue_early * 10,
        "red {red_early}, blue {blue_early}"
    );
    assert!(
        blue_rest > red_rest * 10,
        "red {red_rest}, blue {blue_rest}"
    );
    // Loop: the wave repeats every cycle (0.6 s): the same picture at 1.2 s
    // and 1.8 s (after the last colour key, at full strength), another
    // half a cycle on.
    let (_, early_cycle) = frame_at(&mut project, ms(1_200));
    let (_, late) = frame_at(&mut project, ms(1_800));
    let differ = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
    let (_, half) = frame_at(&mut project, ms(1_500));
    let periodic = differ(&early_cycle, &late);
    assert_eq!(
        periodic, 0,
        "the wave is periodic ({periodic} bytes differ)"
    );
    assert!(differ(&early_cycle, &half) > 100, "the wave moves");
    // While it eases in, the wave is weaker than at full strength.
    assert!(differ(&rest, &late) > 0, "1.0 s is inside the ease-in");
    // Out: at rest when it starts, nearly gone at the end.
    let (_, out_start) = frame_at(&mut project, ms(2_500));
    let (_, out_end) = frame_at(&mut project, ms(2_967));
    let bright = |rgba: &[u8]| -> u64 {
        rgba.as_chunks::<4>()
            .0
            .iter()
            .map(|p| u64::from(p[0].max(p[1]).max(p[2])))
            .sum()
    };
    assert!(
        bright(&out_end) * 5 < bright(&out_start),
        "fading out: {} then {}",
        bright(&out_start),
        bright(&out_end)
    );

    // Export equals preview, frame for frame.
    for (t, previewed) in [(ms(300), &early), (ms(1_000), &rest), (ms(1_800), &late)] {
        let exported = export_frame(&project, t);
        assert!(
            exported == *previewed,
            "export at {t:?} equals the preview ({} bytes differ)",
            exported
                .iter()
                .zip(previewed.iter())
                .filter(|(a, b)| a != b)
                .count()
        );
    }

    // Save and reopen: the same document and the same pictures.
    let file = dir.join("animated.freecut");
    let saved = project.save_job(file.clone()).run();
    assert!(saved.result.is_ok(), "{:?}", saved.result);
    project.saved(&saved);
    let before = project.doc().clone();
    let mut reopened = Project::from_loaded(open_project(&file).unwrap());
    assert_eq!(reopened.doc(), &before);
    let (_, again) = frame_at(&mut reopened, ms(300));
    assert_eq!(again, early, "the reopened project previews the same");
    let (_, again) = frame_at(&mut reopened, ms(1_800));
    assert_eq!(again, late);
}

/// K1's acceptance (CL-007, CL-008, CL-028, README C52): a graded video
/// with a keyframed temperature ramp previews and exports the same bytes at
/// several frames; a drag-like run of Exposure edits merged into one
/// gesture is one undo entry; the grade survives save and reopen.
#[test]
fn a_graded_clip_previews_exports_undoes_and_reopens() {
    use freecut_render::Compositor;
    use freecut_render::export::ExportRenderer;
    use freecut_timeline::edit::{ColorEdit, KeyEdit};
    use freecut_timeline::export::ExportClock;
    use freecut_timeline::model::{ColorParamId, PropPath, PropValue};

    let dir = tempfile_dir("graded");
    let Some((landscape, _)) = fixtures(&dir) else {
        eprintln!("skipped: the ffmpeg CLI is not available");
        return;
    };
    freecut_media::init().unwrap();
    let mut project = new_project();
    let ids = project
        .import(vec![probe_for_import(&landscape, None).unwrap()])
        .unwrap();
    let seq = project.doc().root();
    let main = project
        .doc()
        .sequence(seq)
        .unwrap()
        .main_track()
        .unwrap()
        .id;
    insert_media(&mut project, ids[0], InsertTarget::Track(main), Tick::ZERO).unwrap();
    let clip = project
        .doc()
        .sequence(seq)
        .unwrap()
        .main_track()
        .unwrap()
        .clips()[0]
        .id;
    let preview = Preview::new(FontRegistry::bundled(), presets(), || {});
    project.playhead = Tick::from_secs(1).unwrap();
    let plain = render_draft(&project, &preview, 1, None);
    // An Exposure drag: many values, one gesture, one entry.
    let entries = project.editor.history().len();
    let gesture = project.begin_gesture();
    for v in [5, 12, 20, 31, 40, 50] {
        let req = EditRequest::EditColor {
            seq,
            clips: vec![clip],
            edit: ColorEdit::SetParam(ColorParamId::Exposure, v),
        };
        project.apply(&req, Coalesce::Gesture(gesture)).unwrap();
    }
    project.end_gesture(gesture);
    assert_eq!(project.editor.history().len(), entries + 1, "one entry");
    let bright = render_draft(&project, &preview, 2, None);
    let mean = |f: &PreviewFrame| {
        f.rgba
            .chunks(4)
            .map(|p| u64::from(p[0]) + u64::from(p[1]))
            .sum::<u64>()
    };
    assert!(mean(&bright) > mean(&plain), "Exposure +50 brightens");
    project.undo().unwrap();
    assert_eq!(render_draft(&project, &preview, 3, None).rgba, plain.rgba);
    project.redo().unwrap();

    // A temperature ramp from cool to warm over the clip.
    for (secs, v) in [(0, -80), (3, 80)] {
        let req = EditRequest::EditKeys {
            seq,
            clips: vec![clip],
            prop: PropPath::Color(ColorParamId::Temperature),
            edit: KeyEdit::Add {
                at: Tick::from_secs(secs).unwrap(),
                value: Some(PropValue::Level(v)),
            },
        };
        project.apply(&req, Coalesce::Never).unwrap();
    }
    let export_frame = |project: &Project| {
        let rate = project.doc().settings().frame_rate;
        let range = TimeRange::new(project.playhead, rate.frame_ticks()).unwrap();
        let clock = ExportClock::new(range, rate).unwrap();
        let mut renderer = ExportRenderer::with_media(
            Arc::new(project.doc().clone()),
            project.doc().root(),
            clock,
            (640, 360),
            Compositor::new(FontRegistry::bundled()),
            &project.media_paths,
        )
        .unwrap();
        renderer.render_frame(0).unwrap().rgba.clone()
    };
    let mut frames = Vec::new();
    for (i, ms) in [100i64, 1_500, 2_900].into_iter().enumerate() {
        project.playhead = Tick::from_raw(Tick::from_secs(1).unwrap().raw() * ms / 1000);
        let shown = render_draft(&project, &preview, 10 + i as u64, None);
        write_ppm(&shown, &format!("graded_{ms}.ppm"));
        assert!(
            export_frame(&project) == shown.rgba,
            "the export frame at {ms} ms equals the preview frame"
        );
        frames.push(shown);
    }
    let warmth = |f: &PreviewFrame| {
        f.rgba
            .chunks(4)
            .map(|p| i64::from(p[0]) - i64::from(p[2]))
            .sum::<i64>()
    };
    assert!(
        warmth(&frames[2]) > warmth(&frames[0]),
        "the ramp warms the picture"
    );

    // Save and reopen: the same grade and the same picture.
    let file = dir.join("graded.freecut");
    let saved = project.save_job(file.clone()).run();
    assert!(saved.result.is_ok(), "{:?}", saved.result);
    project.saved(&saved);
    let mut reopened = Project::from_loaded(open_project(&file).unwrap());
    assert_eq!(reopened.doc(), project.doc());
    reopened.playhead = project.playhead;
    let again = render_draft(&reopened, &preview, 20, None);
    assert_eq!(again.rgba, frames[2].rgba);
}

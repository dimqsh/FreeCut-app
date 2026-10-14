//! Auto color (CL-019) and colour match (CL-020): measuring a clip's
//! footage on the job pool.
//!
//! [`plan`] runs on the UI thread and only reads the document: which
//! frames to decode (five spread over the clip's used range, or the one at
//! the playhead), at about [`ANALYSIS_WIDTH`] pixels wide, in exact mode,
//! and what the clip looked like ([`Stamp`]). [`run`] runs on a worker: it
//! decodes the frames, grades a match reference that is measured as
//! graded, and hands the pictures to [`analyse`], which turns
//! `freecut_render::color::stats` into an [`AutoCorrection`] or a
//! [`MatchTransform`]. [`edit_for`] runs on the UI thread when the job is
//! done: a clip that was deleted, locked or changed meanwhile is skipped,
//! so a job never overwrites a newer edit.
//!
//! Auto measures the stack input (after the clip's match, before any
//! slider); Match measures the clip's frames before its own match, so
//! neither compounds when it runs again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use freecut_render::color::stats::{
    ANALYSIS_WIDTH, ColorStatsError, analysis_picture, auto_color, frame_stats, match_stats,
    ycc_stats,
};
use freecut_render::color::{CompiledStack, LutSource, grade, stack_input};
use freecut_render::cpu::{LayerImages, Picture};
use freecut_render::media::{MediaFile, MediaImages};
use freecut_timeline::edit::{ColorEdit, EditRequest};
use freecut_timeline::ids::{ClipId, MediaId};
use freecut_timeline::model::{
    AutoCorrection, AutoSource, AutoStyle, Clip, ClipContent, ClipKeys, ColorStack, Document,
    GeneratedKind, MatchRef, MatchTransform, MediaKind, Rotation,
};
use freecut_timeline::time::Tick;

/// How many frames a whole-clip analysis measures (CL-019, CL-020).
pub const SAMPLES: i64 = 5;

/// What to measure.
#[derive(Debug, Clone, PartialEq)]
pub enum Analysis {
    /// An auto correction in `style`, from `source`.
    Auto {
        style: AutoStyle,
        source: AutoSource,
    },
    /// A match of the clip to `reference`.
    Match { reference: MatchRef },
}

impl Analysis {
    pub fn is_auto(&self) -> bool {
        matches!(self, Analysis::Auto { .. })
    }
}

/// One frame to decode: a video frame at a source time, or a still.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub asset: MediaId,
    pub file: MediaFile,
    /// `None` for a still image.
    pub at: Option<Tick>,
    /// The share of the display size to decode at (about 512 px wide).
    pub fraction: (f64, f64),
}

/// What the clip was when the job started: the job's result is only used
/// while it still is (CL-019: a clip colour-edited, trimmed or slipped
/// meanwhile is skipped).
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    color: Option<Arc<ColorStack>>,
    content: ClipContent,
    keys: ClipKeys,
}

impl Stamp {
    pub fn of(clip: &Clip) -> Stamp {
        Stamp {
            color: clip.color.clone(),
            content: clip.content.clone(),
            keys: clip.keys.clone(),
        }
    }
}

/// A job ready for the pool.
#[derive(Debug, Clone)]
pub struct Request {
    pub id: u64,
    pub clip: ClipId,
    pub stamp: Stamp,
    pub analysis: Analysis,
    /// The clip's frames.
    pub targets: Vec<Frame>,
    /// The clip's stack (Auto measures after its match).
    pub stack: ColorStack,
    /// A match's reference frame and, when it is measured as graded, the
    /// reference clip's stack at that frame.
    pub reference: Option<(Frame, Option<ColorStack>)>,
    pub cancel: Arc<AtomicBool>,
}

/// What an analysis found.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Auto(AutoCorrection),
    /// A match; `limited` when CL-020's limits held a gain or a shift back.
    Match {
        xf: MatchTransform,
        limited: bool,
    },
}

/// Why an analysis has no result.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// The statistics found nothing to work with.
    Stats(ColorStatsError),
    /// Auto would change nothing.
    Balanced,
    /// A frame couldn't be decoded.
    Unreadable(String),
    /// The user cancelled.
    Cancelled,
}

impl Failure {
    /// The status line's sentence.
    pub fn sentence(&self) -> String {
        match self {
            Failure::Stats(e) => crate::ui::look::analysis_failure(e).to_owned(),
            Failure::Balanced => "Already balanced: Auto would change nothing".to_owned(),
            Failure::Unreadable(why) => format!("The footage couldn't be read: {why}"),
            Failure::Cancelled => "Cancelled; nothing changed".to_owned(),
        }
    }
}

/// A finished job, back on the UI thread.
#[derive(Debug, Clone)]
pub struct Finished {
    pub id: u64,
    pub clip: ClipId,
    pub stamp: Stamp,
    pub result: Result<Outcome, Failure>,
}

/// Whether `clip` is an adjustment clip (no footage of its own).
fn is_adjustment(clip: &Clip) -> bool {
    matches!(
        clip.content,
        ClipContent::Generated {
            kind: GeneratedKind::Adjustment
        }
    )
}

/// The frame of `clip` at source time `at` (`None` for a still), with its
/// file.
fn frame_of(
    doc: &Document,
    paths: &BTreeMap<MediaId, PathBuf>,
    offline: &BTreeSet<MediaId>,
    asset: MediaId,
    at: Option<Tick>,
) -> Result<Frame, String> {
    let item = doc
        .library()
        .get(asset)
        .ok_or("the clip's media is no longer in the library")?;
    let path = paths
        .get(&asset)
        .filter(|_| !offline.contains(&asset))
        .ok_or("the clip's media is offline")?;
    let file = MediaFile::of_item(item, path.clone()).ok_or("the media has no picture")?;
    let video = item
        .info
        .video
        .iter()
        .find(|v| v.index as usize == file.stream);
    let fraction = match video {
        Some(v) if v.width > 0 => {
            let display = match v.rotation {
                Rotation::Cw90 | Rotation::Cw270 => v.height,
                Rotation::None | Rotation::Cw180 => v.width,
            };
            let f = (f64::from(ANALYSIS_WIDTH) / f64::from(display.max(1))).min(1.0);
            (f, f)
        }
        _ => (1.0, 1.0),
    };
    Ok(Frame {
        asset,
        file,
        at,
        fraction,
    })
}

/// The frames of `clip` to measure: one for a still; for video, five
/// spread over the used range (the middles of five equal parts) or the one
/// at `playhead` (clamped into the clip).
fn clip_frames(
    doc: &Document,
    paths: &BTreeMap<MediaId, PathBuf>,
    offline: &BTreeSet<MediaId>,
    clip: &Clip,
    source: AutoSource,
    playhead: Tick,
) -> Result<Vec<Frame>, String> {
    match &clip.content {
        ClipContent::Image { asset } => Ok(vec![frame_of(doc, paths, offline, *asset, None)?]),
        ClipContent::Media {
            asset,
            kind: MediaKind::Video,
            source: range,
            ..
        } => {
            let times: Vec<Tick> = match source {
                AutoSource::WholeClip => {
                    let (start, dur) = (range.start().raw(), range.duration().raw());
                    (0..SAMPLES)
                        .map(|i| {
                            let off =
                                i128::from(dur) * i128::from(2 * i + 1) / i128::from(2 * SAMPLES);
                            // At most the duration, which fits an i64.
                            Tick::from_raw(start + i64::try_from(off).unwrap_or(dur))
                        })
                        .collect()
                }
                AutoSource::Playhead => {
                    let last = clip
                        .end()
                        .checked_sub(Tick::from_raw(1))
                        .unwrap_or(clip.start());
                    let at = playhead.clamp(clip.start(), last.max(clip.start()));
                    vec![clip.source_time_at(at).unwrap_or(range.start())]
                }
            };
            times
                .into_iter()
                .map(|t| frame_of(doc, paths, offline, *asset, Some(t)))
                .collect()
        }
        _ => Err("only video and image clips have footage to measure".to_owned()),
    }
}

/// The topmost visible video or image clip under `playhead` other than
/// `target`, or `target` itself when it is the only one: what **Use frame
/// at playhead** matches to (CL-020).
pub fn clip_under_playhead(doc: &Document, target: ClipId, playhead: Tick) -> Option<ClipId> {
    let seq = doc.sequence(doc.root())?;
    let mut under = seq
        .visual_tracks()
        .iter()
        .rev()
        .filter(|t| !t.header.flags.hidden)
        .flat_map(|t| t.clips())
        .filter(|c| c.range.contains(playhead))
        .filter(|c| {
            matches!(
                c.content,
                ClipContent::Image { .. }
                    | ClipContent::Media {
                        kind: MediaKind::Video,
                        ..
                    }
            )
        })
        .map(|c| c.id);
    let first = under.next()?;
    if first != target {
        return Some(first);
    }
    Some(under.next().unwrap_or(target))
}

/// The job for `analysis` of `clip`, read from the document (no I/O):
/// refused with a sentence when the clip can't be measured.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    doc: &Document,
    paths: &BTreeMap<MediaId, PathBuf>,
    offline: &BTreeSet<MediaId>,
    clip: ClipId,
    analysis: Analysis,
    playhead: Tick,
    id: u64,
) -> Result<Request, String> {
    let c = doc.clip(clip).ok_or("The clip is gone")?;
    if is_adjustment(c) {
        return Err(crate::ui::look::NO_FOOTAGE.to_owned());
    }
    if crate::ui::props::on_locked_track(doc, clip) {
        return Err("The clip's track is locked".to_owned());
    }
    let source = match &analysis {
        Analysis::Auto { source, .. } => *source,
        Analysis::Match { .. } => AutoSource::WholeClip,
    };
    let say = |e: String| {
        let mut chars = e.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().collect::<String>() + chars.as_str()
        })
    };
    let targets = clip_frames(doc, paths, offline, c, source, playhead).map_err(say)?;
    let reference = match &analysis {
        Analysis::Auto { .. } => None,
        Analysis::Match {
            reference:
                MatchRef::Clip {
                    clip: r,
                    at,
                    graded,
                },
        } => {
            let rc = doc
                .clip(*r)
                .ok_or("The reference clip is gone, so the match can't be measured again")?;
            let asset = match &rc.content {
                ClipContent::Image { asset } => (*asset, None),
                ClipContent::Media {
                    asset,
                    kind: MediaKind::Video,
                    ..
                } => (*asset, Some(*at)),
                _ => return Err("The reference has no picture to match to".to_owned()),
            };
            let frame = frame_of(doc, paths, offline, asset.0, asset.1)
                .map_err(|e| say(format!("reference: {e}")))?;
            let grade = graded
                .then(|| ColorStack::of(rc.color_at(*at).as_ref()))
                .filter(|s| !s.is_identity());
            Some((frame, grade))
        }
        Analysis::Match {
            reference: MatchRef::Media { media, at },
        } => {
            let still = doc
                .library()
                .get(*media)
                .is_some_and(|m| m.info.video.is_empty() || m.info.still.is_some());
            let frame = frame_of(doc, paths, offline, *media, (!still).then_some(*at))
                .map_err(|e| say(format!("reference: {e}")))?;
            Some((frame, None))
        }
    };
    Ok(Request {
        id,
        clip,
        stamp: Stamp::of(c),
        analysis,
        targets,
        stack: ColorStack::of(c.color.as_ref()),
        reference,
        cancel: Arc::new(AtomicBool::new(false)),
    })
}

/// Decodes `frame` at analysis size.
fn decode(images: &mut MediaImages, frame: &Frame) -> Result<Picture, Failure> {
    let picture = match frame.at {
        Some(at) => images.video_scaled(frame.asset, at, frame.fraction),
        None => images.still(frame.asset),
    };
    picture
        .map(|p| analysis_picture(&p))
        .ok_or_else(|| Failure::Unreadable(frame.file.path.display().to_string()))
}

/// Runs `request` (on a worker: it decodes): the frames, the graded
/// reference, then [`analyse`]. LUTs of a graded reference come from
/// `luts`.
pub fn run(request: Request, luts: Arc<dyn LutSource>) -> Finished {
    let result = measure(&request, luts.as_ref());
    Finished {
        id: request.id,
        clip: request.clip,
        stamp: request.stamp,
        result,
    }
}

fn measure(request: &Request, luts: &dyn LutSource) -> Result<Outcome, Failure> {
    let mut files: BTreeMap<MediaId, MediaFile> = BTreeMap::new();
    for f in request
        .targets
        .iter()
        .chain(request.reference.iter().map(|(f, _)| f))
    {
        files.insert(f.asset, f.file.clone());
    }
    let mut images = MediaImages::new();
    images.set_files(files);
    let cancelled = || request.cancel.load(Ordering::Relaxed);
    let mut targets = Vec::with_capacity(request.targets.len());
    for frame in &request.targets {
        if cancelled() {
            return Err(Failure::Cancelled);
        }
        targets.push(decode(&mut images, frame)?);
    }
    let reference = match &request.reference {
        Some((frame, grade_with)) => {
            if cancelled() {
                return Err(Failure::Cancelled);
            }
            let picture = decode(&mut images, frame)?;
            Some(match grade_with {
                Some(stack) => grade(&CompiledStack::with_luts(stack, luts), &picture, 1),
                None => picture,
            })
        }
        None => None,
    };
    if cancelled() {
        return Err(Failure::Cancelled);
    }
    analyse(
        &request.analysis,
        &request.stack,
        &targets,
        reference.as_ref(),
    )
}

/// The result of `analysis` on decoded frames (pure): Auto measures each
/// frame's stack input under `stack`; Match measures the frames as they
/// are against `reference`.
pub fn analyse(
    analysis: &Analysis,
    stack: &ColorStack,
    targets: &[Picture],
    reference: Option<&Picture>,
) -> Result<Outcome, Failure> {
    match analysis {
        Analysis::Auto { style, source } => {
            let stats: Vec<_> = targets
                .iter()
                .map(|p| frame_stats(&stack_input(stack, p)))
                .collect();
            let offsets = auto_color(&stats, *style).map_err(Failure::Stats)?;
            if offsets.is_zero() {
                return Err(Failure::Balanced);
            }
            Ok(Outcome::Auto(AutoCorrection::new(offsets, *style, *source)))
        }
        Analysis::Match { reference: r } => {
            let Some(picture) = reference else {
                return Err(Failure::Stats(ColorStatsError::NoPixels));
            };
            let stats: Vec<_> = targets.iter().map(ycc_stats).collect();
            let gains = match_stats(&stats, &ycc_stats(picture)).map_err(Failure::Stats)?;
            let (gain, offset) = gains.to_fixed();
            Ok(Outcome::Match {
                xf: MatchTransform::new(gain, offset, r.clone()),
                limited: gains.limited,
            })
        }
    }
}

/// The edit a finished job commits on the root sequence, or why it is
/// skipped: the clip was deleted, its track locked, or it changed while
/// it was measured (CL-019's staleness check).
pub fn edit_for(doc: &Document, finished: &Finished) -> Result<EditRequest, String> {
    let outcome = finished.result.as_ref().map_err(Failure::sentence)?;
    let clip = doc
        .clip(finished.clip)
        .ok_or("The clip was deleted while it was measured; nothing changed")?;
    if crate::ui::props::on_locked_track(doc, finished.clip) {
        return Err("The clip's track was locked while it was measured; nothing changed".into());
    }
    if Stamp::of(clip) != finished.stamp {
        return Err(
            "The clip changed while it was measured, so nothing was applied. Run it again"
                .to_owned(),
        );
    }
    let edit = match outcome {
        Outcome::Auto(auto) => ColorEdit::SetAuto(Some(*auto)),
        Outcome::Match { xf, .. } => ColorEdit::SetMatch(Some(xf.clone())),
    };
    Ok(EditRequest::EditColor {
        seq: doc.root(),
        clips: vec![finished.clip],
        edit,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)] // test code
    use super::*;

    fn chart(gains: [f32; 3]) -> Picture {
        let (w, h) = (64u32, 32u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = x as f32 / (w - 1) as f32;
                let c = if y < 16 { [v, v, v] } else { [v, 1.0 - v, 0.5] };
                for i in 0..3 {
                    rgba.push(((c[i] * gains[i]).min(1.0) * 255.0).round() as u8);
                }
                rgba.push(255);
            }
        }
        Picture::new(w, h, rgba)
    }

    #[test]
    fn auto_corrects_a_cast_and_refuses_a_flat_frame() {
        let auto = Analysis::Auto {
            style: AutoStyle::Natural,
            source: AutoSource::WholeClip,
        };
        let blue = vec![chart([0.88, 0.96, 1.0]); 5];
        let Ok(Outcome::Auto(a)) = analyse(&auto, &ColorStack::default(), &blue, None) else {
            panic!("a cast is corrected");
        };
        assert!(a.offsets.temperature > 0, "{a:?}");
        assert_eq!(a.strength, 100);
        // The same frames give the same correction.
        assert_eq!(
            analyse(&auto, &ColorStack::default(), &blue, None),
            Ok(Outcome::Auto(a))
        );
        let flat = vec![Picture::new(8, 8, [30, 30, 30, 255].repeat(64))];
        assert_eq!(
            analyse(&auto, &ColorStack::default(), &flat, None),
            Err(Failure::Stats(ColorStatsError::NothingToCorrect))
        );
    }

    /// A binary PPM of `picture` (FFmpeg reads it as a still).
    fn write_ppm(path: &std::path::Path, picture: &Picture) {
        let mut bytes = format!("P6\n{} {}\n255\n", picture.width, picture.height).into_bytes();
        for px in picture.rgba.chunks(4) {
            bytes.extend_from_slice(&px[..3]);
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_still_is_measured_on_a_worker_and_a_changed_clip_is_skipped() {
        use freecut_timeline::edit::InsertTarget;
        use freecut_timeline::history::Coalesce;
        use freecut_timeline::model::{ColorParamId, ProjectSettings};

        use crate::session::Project;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cast.ppm");
        write_ppm(&path, &chart([0.88, 0.96, 1.0]));
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let file = crate::session::probe_for_import(&path, None).unwrap();
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
        let auto = Analysis::Auto {
            style: AutoStyle::Natural,
            source: AutoSource::WholeClip,
        };
        let plan_now = |project: &Project, id: u64| {
            plan(
                project.doc(),
                &project.media_paths,
                &project.offline,
                clip,
                auto.clone(),
                Tick::ZERO,
                id,
            )
            .unwrap()
        };
        let request = plan_now(&project, 1);
        assert_eq!(request.targets.len(), 1, "a still is one frame");
        let luts: Arc<dyn LutSource> = Arc::new(freecut_render::color::NoLuts);
        let done = run(request, Arc::clone(&luts));
        let Ok(Outcome::Auto(a)) = &done.result else {
            panic!("{:?}", done.result);
        };
        assert!(a.offsets.temperature > 0, "{a:?}");
        let edit = edit_for(project.doc(), &done).unwrap();
        project.apply(&edit, Coalesce::Never).unwrap();
        let stack = ColorStack::of(project.doc().clip(clip).unwrap().color.as_ref());
        assert_eq!(stack.auto, Some(*a));

        // A run whose clip is colour-edited before it lands is skipped.
        let done = run(plan_now(&project, 2), Arc::clone(&luts));
        project
            .apply(
                &EditRequest::EditColor {
                    seq: project.doc().root(),
                    clips: vec![clip],
                    edit: ColorEdit::SetParam(ColorParamId::Exposure, 5),
                },
                Coalesce::Never,
            )
            .unwrap();
        let skipped = edit_for(project.doc(), &done).unwrap_err();
        assert!(skipped.contains("changed"), "{skipped}");

        // A cancelled run measures nothing.
        let request = plan_now(&project, 3);
        request.cancel.store(true, Ordering::Relaxed);
        assert_eq!(run(request, luts).result, Err(Failure::Cancelled));

        // A match to the clip's own frame, ungraded, is the identity: the
        // target is measured before its sliders too.
        let reference = MatchRef::Clip {
            clip,
            at: Tick::ZERO,
            graded: false,
        };
        let request = plan(
            project.doc(),
            &project.media_paths,
            &project.offline,
            clip,
            Analysis::Match { reference },
            Tick::ZERO,
            4,
        )
        .unwrap();
        let done = run(request, Arc::new(freecut_render::color::NoLuts));
        let Ok(Outcome::Match { xf, .. }) = &done.result else {
            panic!("{:?}", done.result);
        };
        assert!(xf.is_identity(), "ungraded self-match");
    }

    #[test]
    fn a_self_match_is_the_identity() {
        let r = MatchRef::Clip {
            clip: ClipId::from_raw(core::num::NonZeroU64::new(1).unwrap()),
            at: Tick::ZERO,
            graded: true,
        };
        let frame = chart([1.0, 0.9, 0.8]);
        let m = Analysis::Match { reference: r };
        let Ok(Outcome::Match { xf, limited }) = analyse(
            &m,
            &ColorStack::default(),
            std::slice::from_ref(&frame),
            Some(&frame),
        ) else {
            panic!("a match");
        };
        assert!(xf.is_identity() && !limited);
    }
}

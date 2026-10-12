//! Export frames (EX-002): the export clock's grid, the canvas fitted into
//! the output with black bars, and pixels identical to the preview path's.
//! The media tests use fixtures made with the `ffmpeg` CLI and skip with a
//! message when it isn't installed.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

mod common;

use common::project::{Project, ink_box};
use freecut_render::cpu::{Compositor, Frame, NoImages};
use freecut_render::export::{ExportRenderError, ExportRenderer};
use freecut_render::text::FontRegistry;
use freecut_timeline::edit::{TextStyleEdit, TransformEdit};
use freecut_timeline::export::ExportClock;
use freecut_timeline::ids::SeqId;
use freecut_timeline::model::{CanvasPos, RefPx, TextClipData};
use freecut_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const BLACK: [u8; 4] = [0, 0, 0, 255];

fn secs(s: f64) -> Tick {
    Tick::from_raw((s * TICKS_PER_SECOND as f64).round() as i64)
}

fn clock(start: f64, len: f64, fps: u32) -> ExportClock {
    let range = TimeRange::new(secs(start), secs(len)).unwrap();
    ExportClock::new(range, FrameRate::new(fps, 1).unwrap()).unwrap()
}

fn compositor() -> Compositor {
    Compositor::new(FontRegistry::bundled())
}

/// A 480 × 270, 30 fps project with a title over [0.5 s, 2 s).
fn titled() -> Project {
    let mut p = Project::new(480, 270, 30);
    let clip = p.add_text(TextClipData::default(), secs(0.5), secs(1.5));
    p.set_content(clip, "Export");
    p.style(clip, TextStyleEdit::Size(RefPx::from_px(200)));
    p.transform(
        clip,
        TransformEdit::Position(CanvasPos::new(-100_000, 150_000)),
    );
    p
}

/// What the preview shows for export frame `k`: the same plan through the
/// compositor at `size`.
fn preview(p: &Project, clock: &ExportClock, k: u64, size: (u32, u32)) -> Frame {
    let plan = p.plan_at(clock.frame_time(k).unwrap());
    compositor().render(&plan, size.0, size.1, &mut NoImages)
}

#[test]
fn text_frames_match_the_preview_render() {
    let p = titled();
    let clock = clock(0.0, 3.0, 30);
    let mut export =
        ExportRenderer::new(p.doc(), p.seq(), clock, (480, 270), compositor(), NoImages).unwrap();
    assert_eq!(export.frame_count(), 90);
    assert_eq!(export.picture_rect(), (0, 0, 480, 270));
    for k in [0, 14, 15, 16, 30, 44, 59, 60, 89] {
        let frame = export.render_frame(k).unwrap().clone();
        assert!(
            frame == preview(&p, &clock, k, (480, 270)),
            "frame {k} differs from the preview"
        );
        // The title shows over [0.5 s, 2 s): frames 15..60.
        let titled = (15..60).contains(&k);
        assert_eq!(ink_box(&frame, BLACK).is_some(), titled, "frame {k}");
        assert!(
            frame.rgba.chunks(4).all(|px| px[3] == 255),
            "frame {k} is opaque"
        );
    }
}

#[test]
fn a_smaller_output_matches_the_preview_at_that_size() {
    let p = titled();
    let clock = clock(0.0, 3.0, 30);
    let mut export =
        ExportRenderer::new(p.doc(), p.seq(), clock, (240, 136), compositor(), NoImages).unwrap();
    // 480 × 270 into 240 × 136: 240 × 135, one bar row left over.
    assert_eq!(export.picture_rect(), (0, 0, 240, 135));
    let frame = export.render_frame(30).unwrap().clone();
    let small = preview(&p, &clock, 30, (240, 135));
    for y in 0..135 {
        for x in 0..240 {
            assert_eq!(frame.pixel(x, y), small.pixel(x, y), "({x}, {y})");
        }
    }
    assert!((0..240).all(|x| frame.pixel(x, 135) == Some(BLACK)));
}

#[test]
fn other_aspects_are_letterboxed_in_black() {
    let p = titled();
    let clock = clock(0.0, 3.0, 30);
    let mut export =
        ExportRenderer::new(p.doc(), p.seq(), clock, (400, 400), compositor(), NoImages).unwrap();
    let (x, y, w, h) = export.picture_rect();
    assert_eq!((x, y, w, h), (0, 87, 400, 225));
    let frame = export.render_frame(30).unwrap().clone();
    let picture = preview(&p, &clock, 30, (400, 225));
    assert!(ink_box(&picture, BLACK).is_some());
    for row in 0..400 {
        for col in 0..400 {
            let want = if (87..312).contains(&row) {
                picture.pixel(col, row - 87)
            } else {
                Some(BLACK)
            };
            assert_eq!(frame.pixel(col, row), want, "({col}, {row})");
        }
    }
}

#[test]
fn output_frames_follow_the_export_rate() {
    // A 30 fps timeline exported at 25 fps from 0.5 s: frame k shows
    // sequence time 0.5 s + k / 25 s.
    let p = titled();
    let clock = clock(0.5, 1.0, 25);
    let mut export =
        ExportRenderer::new(p.doc(), p.seq(), clock, (480, 270), compositor(), NoImages).unwrap();
    assert_eq!(export.frame_count(), 25);
    for k in [0, 1, 7, 24] {
        let t = secs(0.5)
            .checked_add(Tick::from_raw(k as i64 * TICKS_PER_SECOND / 25))
            .unwrap();
        assert_eq!(export.plan(k).unwrap().time, t);
    }
}

#[test]
fn rendering_is_deterministic() {
    let p = titled();
    let clock = clock(0.0, 3.0, 30);
    let mut a = ExportRenderer::new(
        p.doc(),
        p.seq(),
        clock,
        (480, 270),
        compositor().with_threads(1),
        NoImages,
    )
    .unwrap();
    let mut b = ExportRenderer::new(
        p.doc(),
        p.seq(),
        clock,
        (480, 270),
        compositor().with_threads(4),
        NoImages,
    )
    .unwrap();
    // Any order, any thread count: the same bytes.
    let late = b.render_frame(40).unwrap().clone();
    let early = b.render_frame(20).unwrap().clone();
    assert!(a.render_frame(20).unwrap() == &early);
    assert!(a.render_frame(40).unwrap() == &late);
}

#[test]
fn bad_inputs_are_errors() {
    let p = titled();
    let clock = clock(0.0, 1.0, 30);
    let make = |seq: SeqId, size: (u32, u32)| {
        ExportRenderer::new(p.doc(), seq, clock, size, compositor(), NoImages)
    };
    assert!(matches!(
        make(p.seq(), (0, 270)),
        Err(ExportRenderError::InvalidSize { .. })
    ));
    assert!(matches!(
        make(p.seq(), (20_000, 270)),
        Err(ExportRenderError::InvalidSize { .. })
    ));
    let other = SeqId::from_raw(std::num::NonZeroU64::new(9999).unwrap());
    assert_eq!(
        make(other, (480, 270)).unwrap_err(),
        ExportRenderError::NoSequence(other)
    );
    let mut export = make(p.seq(), (480, 270)).unwrap();
    assert_eq!(
        export.render_frame(30).unwrap_err(),
        ExportRenderError::FrameOutOfRange {
            frame: 30,
            count: 30
        }
    );
}

#[cfg(feature = "media")]
mod media {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use freecut_render::media::{MediaFile, MediaImages};
    use freecut_timeline::edit::InsertTarget;
    use freecut_timeline::ids::MediaId;

    use super::*;

    /// Encodes `testsrc` (160 × 90, 25 fps, 2 s) with the ffmpeg CLI.
    fn testsrc(dir: &Path) -> Option<PathBuf> {
        let path = dir.join("testsrc.mp4");
        let encode = |codec: &[&str]| {
            let mut args = vec![
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=160x90:rate=25:duration=2",
            ];
            args.extend_from_slice(codec);
            args.extend_from_slice(&["-pix_fmt", "yuv420p", path.to_str()?]);
            Command::new("ffmpeg")
                .args(&args)
                .output()
                .ok()
                .map(|o| o.status.success())
        };
        let ok = encode(&["-c:v", "libx264", "-g", "12"]) == Some(true)
            || encode(&["-c:v", "mpeg4", "-q:v", "2", "-g", "12"]) == Some(true);
        ok.then_some(path)
    }

    /// A 320 × 180, 25 fps project with the 2 s clip on the main track and a
    /// title over it; returns it with its media paths.
    fn project(path: &Path) -> (Project, BTreeMap<MediaId, PathBuf>) {
        let probed = freecut_media::probe(path).unwrap();
        let mut p = Project::new(320, 180, 25);
        let id = p.import(path.to_str().unwrap(), probed.info);
        let main = p.editor.document().sequence(p.seq()).unwrap();
        let main = main.main_track().unwrap().id;
        p.place_video(
            InsertTarget::Track(main),
            Tick::ZERO,
            id,
            TimeRange::new(Tick::ZERO, secs(2.0)).unwrap(),
        );
        let title = p.add_text(TextClipData::default(), secs(1.0), secs(1.0));
        p.style(title, TextStyleEdit::Size(RefPx::from_px(300)));
        (p, BTreeMap::from([(id, path.to_path_buf())]))
    }

    #[test]
    fn video_frames_match_the_preview_render_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let Some(path) = testsrc(dir.path()) else {
            eprintln!("skipped: the ffmpeg CLI is not available");
            return;
        };
        let (p, paths) = project(&path);
        let clock = clock(0.0, 2.0, 25);
        let doc = p.doc();
        let mut export = ExportRenderer::with_media(
            Arc::clone(&doc),
            p.seq(),
            clock,
            (320, 180),
            compositor(),
            &paths,
        )
        .unwrap();
        assert_eq!(export.frame_count(), 50);
        let files = MediaFile::from_library(doc.library(), &paths);
        let mut frames = Vec::new();
        // Sequential, as the export job renders.
        for k in 0..export.frame_count() {
            frames.push(export.render_frame(k).unwrap().clone());
        }
        // The preview path seeks to each frame with a fresh decoder.
        for k in [0, 1, 2, 11, 12, 13, 24, 25, 26, 37, 49] {
            let mut images = MediaImages::new();
            images.set_files(files.clone());
            let plan = p.plan_at(clock.frame_time(k).unwrap());
            let want = compositor().render(&plan, 320, 180, &mut images);
            assert!(
                frames[k as usize] == want,
                "frame {k} differs from the preview"
            );
        }
        // Not stuck on one picture, and not the offline slate.
        assert!(frames[0] != frames[24]);
        assert!(frames[0].pixel(5, 5) != Some(freecut_render::cpu::SLATE));
        // From 1 s the title is drawn over the video (frames 26, 37, 49
        // above compare text over video too).
        assert_eq!(export.plan(24).unwrap().nodes.len(), 1);
        assert_eq!(export.plan(25).unwrap().nodes.len(), 2);
    }

    #[test]
    fn a_missing_file_draws_the_slate() {
        let dir = tempfile::tempdir().unwrap();
        let Some(path) = testsrc(dir.path()) else {
            eprintln!("skipped: the ffmpeg CLI is not available");
            return;
        };
        let (p, _) = project(&path);
        let clock = clock(0.0, 2.0, 25);
        let mut export = ExportRenderer::with_media(
            p.doc(),
            p.seq(),
            clock,
            (320, 180),
            compositor(),
            &BTreeMap::new(),
        )
        .unwrap();
        let frame = export.render_frame(3).unwrap();
        assert_eq!(frame.pixel(5, 5), Some(freecut_render::cpu::SLATE));
    }

    /// EX-022 `decode-error`: a source that exists but can't be read stops
    /// the export, naming the clip and time; the slate retry lists it.
    #[test]
    fn an_unreadable_file_stops_the_export_unless_slates_are_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let Some(path) = testsrc(dir.path()) else {
            eprintln!("skipped: the ffmpeg CLI is not available");
            return;
        };
        let (p, paths) = project(&path);
        // The file is damaged after import: only its first bytes are left.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..64]).unwrap();
        let clock = clock(0.0, 2.0, 25);
        let video_clip = p
            .plan_at(Tick::ZERO)
            .nodes
            .iter()
            .map(|n| {
                let freecut_timeline::plan::PlanNode::Layer(l) = n;
                l.clip
            })
            .next()
            .unwrap();
        let make = || {
            ExportRenderer::with_media(p.doc(), p.seq(), clock, (320, 180), compositor(), &paths)
                .unwrap()
        };
        let mut export = make();
        match export.render_frame(3) {
            Err(ExportRenderError::Decode { clip, at, .. }) => {
                assert_eq!(clip, Some(video_clip));
                assert_eq!(at, clock.frame_time(3).unwrap());
            }
            other => panic!("expected a decode error, got {:?}", other.map(|_| ())),
        }
        let mut export = make().with_slates_for_unreadable();
        let frame = export.render_frame(3).unwrap();
        assert_eq!(frame.pixel(5, 5), Some(freecut_render::cpu::SLATE));
        export.render_frame(4).unwrap();
        let listed: Vec<(u64, Option<_>)> = export
            .unreadable()
            .iter()
            .map(|u| (u.frame, u.clip))
            .collect();
        assert_eq!(listed, vec![(3, Some(video_clip)), (4, Some(video_clip))]);
    }

    /// Video drawn much smaller than its size is decoded at about the drawn
    /// size (R7: cheaper, and filtered for the downscale).
    #[test]
    fn small_layers_are_decoded_small() {
        use freecut_render::cpu::{LayerImages, Picture};
        use freecut_timeline::time::Tick;

        /// Records the sizes the compositor got.
        struct Spy {
            inner: MediaImages,
            got: Vec<(u32, u32)>,
        }
        impl LayerImages for Spy {
            fn video(&mut self, asset: MediaId, t: Tick) -> Option<Arc<Picture>> {
                let p = self.inner.video(asset, t)?;
                self.got.push((p.width, p.height));
                Some(p)
            }
            fn video_scaled(
                &mut self,
                asset: MediaId,
                t: Tick,
                fraction: (f64, f64),
            ) -> Option<Arc<Picture>> {
                let p = self.inner.video_scaled(asset, t, fraction)?;
                self.got.push((p.width, p.height));
                Some(p)
            }
            fn still(&mut self, asset: MediaId) -> Option<Arc<Picture>> {
                self.inner.still(asset)
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let Some(path) = testsrc(dir.path()) else {
            eprintln!("skipped: the ffmpeg CLI is not available");
            return;
        };
        let (p, paths) = project(&path);
        let doc = p.doc();
        let mut spy = Spy {
            inner: MediaImages::new(),
            got: Vec::new(),
        };
        spy.inner
            .set_files(MediaFile::from_library(doc.library(), &paths));
        let plan = p.plan_at(secs(0.4));
        // The 160 × 90 clip fills the 320 × 180 canvas; rendered at 64 × 36
        // it is drawn at 0.4 of its size, and decoded at that size.
        let small = compositor().render(&plan, 64, 36, &mut spy);
        assert_eq!(spy.got, vec![(64, 36)]);
        assert!(small.rgba.chunks(4).all(|px| px[3] == 255));
        // At full size the coded picture is used.
        compositor().render(&plan, 320, 180, &mut spy);
        assert_eq!(spy.got[1], (160, 90));
        assert!(spy.inner.take_failures().is_empty());
    }
}

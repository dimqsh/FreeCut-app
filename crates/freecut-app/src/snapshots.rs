//! Headless snapshots of the main window (ui-concept §17.1's manual
//! check-list, made visible without a display).
//!
//! The window is built for a seeded demo project at 1440 × 900 pt in Dark,
//! Light and High contrast with `egui_kittest`. Every run checks that the
//! whole window builds headless in each theme. When `FREECUT_SNAPSHOT_DIR`
//! names a folder, each frame is also rendered through wgpu and written
//! there as `<theme>.png` (`FREECUT_SNAPSHOT_SCALE` sets the pixels per
//! point, default 1). Pixels are never compared yet, so a renderer
//! difference never fails the build.
//!
//! The demo project uses media generated with the ffmpeg CLI (a test
//! pattern with a tone, and a tone), like the e2e tests; without the CLI it
//! falls back to text clips only.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use freecut_timeline::edit::InsertTarget;
use freecut_timeline::model::ProjectSettings;
use freecut_timeline::time::{FrameRate, Tick};

use crate::app::FreeCutApp;
use crate::session::{Project, probe_for_import};
use crate::theme::ThemeChoice;
use crate::ui::inspector::lens::{CardId, Detail, Lens, SelectionKind};

/// The window size the concept's anatomy is drawn at (§3.1).
const WINDOW: egui::Vec2 = egui::vec2(1440.0, 900.0);

fn ffmpeg(args: &[&str]) -> bool {
    Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// A 6 s test pattern with a tone, and a 6 s tone, in `dir`.
fn media(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let video = dir.join("beach.mp4");
    let music = dir.join("music.wav");
    let ok = ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=30:duration=6",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:duration=6",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        video.to_str()?,
    ]) && ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        // A tone that swells and fades, so the waveform has a shape.
        "aevalsrc=0.8*sin(2*PI*220*t)*abs(sin(0.9*t)):d=6:s=48000",
        "-ac",
        "2",
        music.to_str()?,
    ]);
    ok.then_some((video, music))
}

/// "Beach trip": a video on the main track, music under it and a title
/// above it, the title selected and the playhead on it; saved.
fn demo_project(dir: &Path) -> Project {
    let settings = ProjectSettings {
        width: 1920,
        height: 1080,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        ..ProjectSettings::new_default().unwrap()
    };
    let mut project = Project::new(settings).unwrap();
    project.file = Some(dir.join("Beach trip.freecut"));
    if let Some((video, music)) = media(dir)
        && freecut_media::init().is_ok()
    {
        let files = vec![
            probe_for_import(&video, None).unwrap(),
            probe_for_import(&music, None).unwrap(),
        ];
        let ids = project.import(files).unwrap();
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
        crate::ui::timeline::insert_media(&mut project, ids[1], InsertTarget::NewTrack, Tick::ZERO)
            .unwrap();
    } else {
        eprintln!("snapshots: no ffmpeg CLI; the demo project has text clips only");
    }
    project.playhead = Tick::from_secs(2).unwrap();
    let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
    project.selection = vec![clip];
    project.playhead = Tick::from_secs(3).unwrap();
    project.editor.mark_saved();
    project
}

/// More items for the asset browser (R2): a second landscape video, a
/// portrait video (cover crop), a picture, and the sound extracted from
/// the beach video. Without the ffmpeg CLI the library stays as it is.
fn rich_project(dir: &Path) -> Project {
    let mut project = demo_project(dir);
    let waves = dir.join("waves.mp4");
    let portrait = dir.join("portrait.mp4");
    let photo = dir.join("photo.png");
    let made = ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "smptehdbars=size=640x360:rate=30:duration=3",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        waves.to_str().unwrap(),
    ]) && ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=360x640:rate=30:duration=65",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        portrait.to_str().unwrap(),
    ]) && ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "mandelbrot=size=640x480",
        "-frames:v",
        "1",
        photo.to_str().unwrap(),
    ]);
    if !made || project.doc().library().is_empty() {
        return project;
    }
    let files = [&waves, &portrait, &photo]
        .iter()
        .map(|p| probe_for_import(p, None).unwrap())
        .collect();
    project.import(files).unwrap();
    let beach = project.doc().library().items().next().unwrap().id;
    project.extract_audio(beach).unwrap();
    project.editor.mark_saved();
    project
}

/// Steps the window until the preview frame and the posters are in (or
/// 20 s pass), then a few more frames so hover fades and layout settle.
fn settle(harness: &mut Harness<'_, FreeCutApp>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut quiet = 0;
    while quiet < 10 && Instant::now() < deadline {
        harness.step();
        if harness.state().settled() {
            quiet += 1;
        } else {
            quiet = 0;
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn harness(
    project: Option<Project>,
    theme: ThemeChoice,
    size: egui::Vec2,
    render: bool,
) -> Harness<'static, FreeCutApp> {
    let scale = std::env::var("FREECUT_SNAPSHOT_SCALE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);
    let mut builder = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(scale)
        .with_max_steps(10_000);
    if render {
        builder = builder.wgpu();
    }
    builder.build_eframe(move |cc| FreeCutApp::headless(&cc.egui_ctx, project, theme))
}

/// One picture of the window.
struct Scene {
    name: &'static str,
    theme: ThemeChoice,
    size: egui::Vec2,
    /// Something to do once the window settled.
    then: Then,
}

#[derive(Clone, Copy)]
enum Then {
    Nothing,
    /// Hover the gutter right of the asset panel (its resize line).
    HoverGutter,
    /// Zoom the Stage to 200 % of the canvas's pixels, panned a little.
    ZoomStage,
    /// No project: the Home screen.
    Home,
    /// The R2 asset browser on a fuller library, then `Browse`.
    Assets(Browse),
    /// The R4 inspector: what is selected and which lens shows.
    Inspect(Inspect),
    /// The R5 timeline on a fuller project, then `Look`.
    Timeline(TimelineLook),
    /// R6: Home, the sheets, Preferences and toasts.
    R6(R6Scene),
    /// R3 toolbar round: the toolbar, tools and markers.
    R3(R3Look),
}

/// What an R3 toolbar-round scene shows.
#[derive(Clone, Copy)]
enum R3Look {
    /// The timeline as it opens, with markers and an attached title.
    Plain,
    /// Zoomed in, a marker selected and the blade over the beach video.
    Blade,
    /// The marker editor open on the range marker.
    Editor,
    /// The menu of the split button with this accessibility label open.
    Menu(&'static str),
    /// The voiceover recorder panel (AU-027).
    Recorder(RecorderLook),
}

/// What a recorder-panel scene shows.
#[derive(Clone, Copy)]
enum RecorderLook {
    /// Open, the (synthetic) microphone ready and the meter moving.
    Ready,
    /// A take recording: the red clip grows on the timeline.
    Recording,
    /// The microphone refused (the privacy settings).
    Denied,
    /// A cancelled take waiting in the Takes list.
    Takes,
    /// Hover skim on, the pointer over the video.
    Skim,
}

/// A sheet the R6 scenes open ([`FreeCutApp::open_for_tests`]).
#[derive(Debug, Clone)]
pub enum Open {
    NewProject,
    /// Export, saving into this folder.
    Export(PathBuf),
    /// Preferences on a page, with a search.
    Preferences(crate::ui::preferences::Section, &'static str),
    About,
    /// "Save changes?"
    Unsaved,
}

/// What an R6 scene shows.
#[derive(Clone, Copy)]
enum R6Scene {
    /// Home with recent projects (`true`) or none; the pointer on a card.
    Home {
        recent: bool,
        hover: bool,
    },
    /// Home with the new-project sheet.
    NewProject,
    /// The Export sheet.
    Export(ExportLook),
    /// Preferences on a page, with a search.
    Prefs(crate::ui::preferences::Section, &'static str),
    About,
    /// Three toasts over the editor.
    Toasts,
    /// The top bar's File menu open.
    Menu,
    /// "Save changes?"
    Unsaved,
}

/// The Export sheet's state.
#[derive(Clone, Copy)]
enum ExportLook {
    Settings,
    /// A file of that name is already in the folder.
    Conflict,
    Finished,
    Failed,
}

/// What the timeline shows (R5).
#[derive(Clone, Copy)]
enum TimelineLook {
    /// As it opens.
    Plain,
    /// The pointer on the music's volume line: the level pill.
    HoverVolume,
    /// Zoomed in on the first seconds: frame ticks and filmstrip tiles.
    Zoomed,
}

/// What the inspector shows (R4).
#[derive(Clone, Copy)]
enum Inspect {
    /// The styled title, on `Lens`; `true` shows Everything.
    Text(Lens, bool),
    /// The beach video, on `Lens`.
    Video(Lens),
    /// The beach video graded (a keyed Exposure, Contrast, a warm
    /// Temperature, Saturation and Amount 85 %), on the Look lens's page.
    Look(freecut_timeline::model::ColorPage),
    /// The graded video's Basic page with Light folded, so Color (with
    /// Skin protection) shows, and with Color folded too, so the LUT
    /// section does; `missing`: its LUT's file is (K3).
    LookLut { missing: bool, fold_color: bool },
    /// Nothing selected: the Project lens.
    Project,
    /// The title and the video together.
    Several,
}

/// What to do in the asset browser (R2).
#[derive(Clone, Copy)]
enum Browse {
    /// The Media tab as it opens.
    Media,
    /// Three tiles selected (the Delete strip) and one hovered.
    Selected,
    /// A rubber band being drawn over the grid.
    Band,
    /// The Audio tab.
    Audio,
    /// The Text tab.
    Text,
    /// The Media tab with a search that matches nothing.
    NoMatches,
    /// A project with nothing imported.
    Empty,
}

/// The editor window builds headless in every theme and density; with
/// `FREECUT_SNAPSHOT_DIR` set it is also written out as PNGs.
#[test]
fn main_window_snapshots() {
    let out = std::env::var_os("FREECUT_SNAPSHOT_DIR").map(PathBuf::from);
    let media_dir = tempfile::tempdir().unwrap();
    let scenes = [
        Scene {
            name: "dark",
            theme: ThemeChoice::Dark,
            size: WINDOW,
            then: Then::Nothing,
        },
        Scene {
            name: "light",
            theme: ThemeChoice::Light,
            size: WINDOW,
            then: Then::Nothing,
        },
        Scene {
            name: "high-contrast",
            theme: ThemeChoice::HighContrast,
            size: WINDOW,
            then: Then::Nothing,
        },
        // Below 1280 pt the Automatic density is Compact (§5.1).
        Scene {
            name: "dark-compact",
            theme: ThemeChoice::Dark,
            size: egui::vec2(1100.0, 700.0),
            then: Then::Nothing,
        },
        Scene {
            name: "light-compact",
            theme: ThemeChoice::Light,
            size: egui::vec2(1100.0, 700.0),
            then: Then::Nothing,
        },
        Scene {
            name: "dark-gutter-hover",
            theme: ThemeChoice::Dark,
            size: WINDOW,
            then: Then::HoverGutter,
        },
        Scene {
            name: "dark-stage-zoom",
            theme: ThemeChoice::Dark,
            size: WINDOW,
            then: Then::ZoomStage,
        },
        Scene {
            name: "light-stage-zoom",
            theme: ThemeChoice::Light,
            size: WINDOW,
            then: Then::ZoomStage,
        },
        Scene {
            name: "dark-home",
            theme: ThemeChoice::Dark,
            size: WINDOW,
            then: Then::Home,
        },
    ];
    let assets = |name, theme, size, browse| Scene {
        name,
        theme,
        size,
        then: Then::Assets(browse),
    };
    let compact = egui::vec2(1100.0, 700.0);
    let r2 = [
        assets("r2-media-dark", ThemeChoice::Dark, WINDOW, Browse::Media),
        assets("r2-media-light", ThemeChoice::Light, WINDOW, Browse::Media),
        assets(
            "r2-media-hc",
            ThemeChoice::HighContrast,
            WINDOW,
            Browse::Media,
        ),
        assets(
            "r2-selected-dark",
            ThemeChoice::Dark,
            WINDOW,
            Browse::Selected,
        ),
        assets(
            "r2-selected-light",
            ThemeChoice::Light,
            WINDOW,
            Browse::Selected,
        ),
        assets("r2-band-dark", ThemeChoice::Dark, WINDOW, Browse::Band),
        assets("r2-audio-dark", ThemeChoice::Dark, WINDOW, Browse::Audio),
        assets("r2-audio-light", ThemeChoice::Light, WINDOW, Browse::Audio),
        assets("r2-text-dark", ThemeChoice::Dark, WINDOW, Browse::Text),
        assets(
            "r2-nomatch-dark",
            ThemeChoice::Dark,
            WINDOW,
            Browse::NoMatches,
        ),
        assets("r2-empty-dark", ThemeChoice::Dark, WINDOW, Browse::Empty),
        assets("r2-empty-light", ThemeChoice::Light, WINDOW, Browse::Empty),
        assets("r2-compact-dark", ThemeChoice::Dark, compact, Browse::Media),
        assets(
            "r2-compact-light",
            ThemeChoice::Light,
            compact,
            Browse::Media,
        ),
        assets(
            "r2-compact-audio-dark",
            ThemeChoice::Dark,
            compact,
            Browse::Audio,
        ),
    ];
    let inspect = |name, theme, size, what| Scene {
        name,
        theme,
        size,
        then: Then::Inspect(what),
    };
    use {Inspect as I, ThemeChoice as T};
    let mut r4 = vec![
        inspect(
            "r4-text-style-dark",
            T::Dark,
            WINDOW,
            I::Text(Lens::Style, false),
        ),
        inspect(
            "r4-video-picture-light",
            T::Light,
            WINDOW,
            I::Video(Lens::Picture),
        ),
        inspect("r4-several-dark", T::Dark, WINDOW, I::Several),
        inspect(
            "k1-look-basic-dark",
            T::Dark,
            WINDOW,
            I::Look(freecut_timeline::model::ColorPage::Basic),
        ),
        inspect(
            "k2-look-curves-dark",
            T::Dark,
            WINDOW,
            I::Look(freecut_timeline::model::ColorPage::Curves),
        ),
        inspect(
            "k2-look-wheels-dark",
            T::Dark,
            WINDOW,
            I::Look(freecut_timeline::model::ColorPage::Wheels),
        ),
        inspect(
            "k3-look-lut-missing-dark",
            T::Dark,
            WINDOW,
            I::LookLut {
                missing: true,
                fold_color: true,
            },
        ),
    ];
    // Every lens in both themes only when pictures are wanted: each scene
    // costs a window.
    if out.is_some() {
        r4.clear();
        for (theme, suffix) in [
            (T::Dark, "dark"),
            (T::Light, "light"),
            (T::HighContrast, "hc"),
        ] {
            for (what, stem) in [
                (I::Text(Lens::Content, false), "r4-text-content"),
                (I::Text(Lens::Style, false), "r4-text-style"),
                (I::Text(Lens::Style, true), "r4-text-style-all"),
                (I::Text(Lens::BoxBubble, false), "r4-text-box"),
                (I::Text(Lens::Motion, false), "r4-text-motion"),
                (I::Text(Lens::Presets, false), "r4-text-presets"),
                (I::Video(Lens::Picture), "r4-video-picture"),
                (I::Video(Lens::Motion), "r4-video-motion"),
                (I::Video(Lens::Sound), "r4-video-sound"),
                (I::Project, "r4-project"),
                (I::Several, "r4-several"),
            ] {
                let name: &'static str = format!("{stem}-{suffix}").leak();
                r4.push(inspect(name, theme, WINDOW, what));
            }
        }
        r4.push(inspect(
            "r4-text-style-compact-dark",
            T::Dark,
            egui::vec2(1100.0, 700.0),
            I::Text(Lens::Style, false),
        ));
        r4.push(inspect(
            "r4-video-picture-compact-light",
            T::Light,
            egui::vec2(1100.0, 700.0),
            I::Video(Lens::Picture),
        ));
        // K1 and K2: the Look lens.
        use freecut_timeline::model::ColorPage as P;
        let compact = egui::vec2(1100.0, 700.0);
        let tall = egui::vec2(1440.0, 1400.0);
        for (name, theme, size, page) in [
            ("k1-look-basic-dark", T::Dark, WINDOW, P::Basic),
            ("k1-look-basic-light", T::Light, WINDOW, P::Basic),
            ("k1-look-basic-hc", T::HighContrast, WINDOW, P::Basic),
            ("k1-look-masks-light", T::Light, WINDOW, P::Masks),
            ("k1-look-basic-compact-dark", T::Dark, compact, P::Basic),
            ("k2-look-hsl-dark", T::Dark, WINDOW, P::Hsl),
            ("k2-look-hsl-light", T::Light, WINDOW, P::Hsl),
            ("k2-look-curves-dark", T::Dark, WINDOW, P::Curves),
            ("k2-look-curves-hc", T::HighContrast, WINDOW, P::Curves),
            ("k2-look-wheels-dark", T::Dark, WINDOW, P::Wheels),
            ("k2-look-wheels-light", T::Light, WINDOW, P::Wheels),
            ("k2-look-wheels-compact-dark", T::Dark, compact, P::Wheels),
            ("k2-look-curves-compact-light", T::Light, compact, P::Curves),
            // Tall windows, so each page shows whole.
            ("k2-look-tall-hsl-dark", T::Dark, tall, P::Hsl),
            ("k2-look-tall-curves-light", T::Light, tall, P::Curves),
            ("k2-look-tall-wheels-dark", T::Dark, tall, P::Wheels),
            ("k2-look-tall-wheels-hc", T::HighContrast, tall, P::Wheels),
        ] {
            r4.push(inspect(name, theme, size, I::Look(page)));
        }
        // K3: Color's Skin protection and the LUT section.
        for (name, theme, size, missing, fold_color) in [
            ("k3-look-skin-dark", T::Dark, tall, false, false),
            ("k3-look-skin-light", T::Light, tall, false, false),
            ("k3-look-lut-tall-dark", T::Dark, tall, false, true),
            ("k3-look-lut-tall-hc", T::HighContrast, tall, false, true),
            ("k3-look-lut-dark", T::Dark, WINDOW, false, true),
            ("k3-look-lut-light", T::Light, WINDOW, false, true),
            ("k3-look-lut-hc", T::HighContrast, WINDOW, false, true),
            ("k3-look-lut-compact-dark", T::Dark, compact, false, true),
            ("k3-look-lut-missing-dark", T::Dark, WINDOW, true, true),
            ("k3-look-lut-missing-light", T::Light, WINDOW, true, true),
        ] {
            r4.push(inspect(
                name,
                theme,
                size,
                I::LookLut {
                    missing,
                    fold_color,
                },
            ));
        }
    }
    let timeline = |name, theme, size, look| Scene {
        name,
        theme,
        size,
        then: Then::Timeline(look),
    };
    let mut r5 = vec![timeline("r5-dark", T::Dark, WINDOW, TimelineLook::Plain)];
    if out.is_some() {
        r5.extend([
            timeline("r5-light", T::Light, WINDOW, TimelineLook::Plain),
            timeline("r5-hc", T::HighContrast, WINDOW, TimelineLook::Plain),
            timeline("r5-compact-dark", T::Dark, compact, TimelineLook::Plain),
            timeline("r5-compact-light", T::Light, compact, TimelineLook::Plain),
            timeline("r5-hover-dark", T::Dark, WINDOW, TimelineLook::HoverVolume),
            timeline(
                "r5-hover-light",
                T::Light,
                WINDOW,
                TimelineLook::HoverVolume,
            ),
            timeline("r5-zoom-dark", T::Dark, WINDOW, TimelineLook::Zoomed),
            timeline(
                "r5-narrow-dark",
                T::Dark,
                egui::vec2(900.0, 640.0),
                TimelineLook::Plain,
            ),
        ]);
    }
    let r3s = |name, theme, size, look| Scene {
        name,
        theme,
        size,
        then: Then::R3(look),
    };
    let narrow = egui::vec2(700.0, 680.0);
    let rec = |look| R3Look::Recorder(look);
    let mut r3 = vec![
        r3s("r3-toolbar-wide-dark", T::Dark, WINDOW, R3Look::Plain),
        r3s("r3-toolbar-narrow-dark", T::Dark, narrow, R3Look::Plain),
        r3s(
            "r3-recorder-ready-dark",
            T::Dark,
            WINDOW,
            rec(RecorderLook::Ready),
        ),
    ];
    if out.is_some() {
        r3.extend([
            r3s("r3-toolbar-wide-light", T::Light, WINDOW, R3Look::Plain),
            r3s("r3-toolbar-wide-hc", T::HighContrast, WINDOW, R3Look::Plain),
            r3s("r3-toolbar-narrow-light", T::Light, narrow, R3Look::Plain),
            r3s(
                "r3-toolbar-narrow-hc",
                T::HighContrast,
                narrow,
                R3Look::Plain,
            ),
            r3s("r3-toolbar-compact-dark", T::Dark, compact, R3Look::Plain),
            r3s("r3-markers-blade-dark", T::Dark, WINDOW, R3Look::Blade),
            r3s("r3-markers-blade-light", T::Light, WINDOW, R3Look::Blade),
            r3s(
                "r3-markers-blade-hc",
                T::HighContrast,
                WINDOW,
                R3Look::Blade,
            ),
            r3s("r3-marker-editor-dark", T::Dark, WINDOW, R3Look::Editor),
            r3s("r3-marker-editor-light", T::Light, WINDOW, R3Look::Editor),
            r3s(
                "r3-menu-markers-dark",
                T::Dark,
                WINDOW,
                R3Look::Menu("Marker options"),
            ),
            r3s(
                "r3-menu-follow-dark",
                T::Dark,
                WINDOW,
                R3Look::Menu("Follow main track options"),
            ),
            r3s(
                "r3-menu-follow-light",
                T::Light,
                WINDOW,
                R3Look::Menu("Follow main track options"),
            ),
            r3s(
                "r3-menu-tools-dark",
                T::Dark,
                WINDOW,
                R3Look::Menu("Choose a tool"),
            ),
            r3s(
                "r3-menu-more-narrow-dark",
                T::Dark,
                narrow,
                R3Look::Menu("More timeline commands"),
            ),
            r3s(
                "r3-recorder-ready-light",
                T::Light,
                WINDOW,
                rec(RecorderLook::Ready),
            ),
            r3s(
                "r3-recorder-ready-hc",
                T::HighContrast,
                WINDOW,
                rec(RecorderLook::Ready),
            ),
            r3s(
                "r3-recorder-recording-dark",
                T::Dark,
                WINDOW,
                rec(RecorderLook::Recording),
            ),
            r3s(
                "r3-recorder-recording-light",
                T::Light,
                WINDOW,
                rec(RecorderLook::Recording),
            ),
            r3s(
                "r3-recorder-denied-dark",
                T::Dark,
                WINDOW,
                rec(RecorderLook::Denied),
            ),
            r3s(
                "r3-recorder-takes-dark",
                T::Dark,
                WINDOW,
                rec(RecorderLook::Takes),
            ),
            r3s(
                "r3-recorder-takes-light",
                T::Light,
                WINDOW,
                rec(RecorderLook::Takes),
            ),
            r3s("r3-skim-dark", T::Dark, WINDOW, rec(RecorderLook::Skim)),
            r3s(
                "r3-menu-skim-dark",
                T::Dark,
                WINDOW,
                R3Look::Menu("Hover skim options"),
            ),
        ]);
    }
    let r6s = |name, theme, size, scene| Scene {
        name,
        theme,
        size,
        then: Then::R6(scene),
    };
    use crate::ui::preferences::Section as PS;
    let home = R6Scene::Home {
        recent: true,
        hover: false,
    };
    let mut r6 = vec![
        r6s("r6-home-dark", T::Dark, WINDOW, home),
        r6s(
            "r6-export-dark",
            T::Dark,
            WINDOW,
            R6Scene::Export(ExportLook::Settings),
        ),
        r6s(
            "r6-prefs-appearance-dark",
            T::Dark,
            WINDOW,
            R6Scene::Prefs(PS::Appearance, ""),
        ),
    ];
    if out.is_some() {
        r6.extend([
            r6s("r6-home-light", T::Light, WINDOW, home),
            r6s("r6-home-hc", T::HighContrast, WINDOW, home),
            r6s(
                "r6-home-hover-dark",
                T::Dark,
                WINDOW,
                R6Scene::Home {
                    recent: true,
                    hover: true,
                },
            ),
            r6s(
                "r6-home-empty-dark",
                T::Dark,
                WINDOW,
                R6Scene::Home {
                    recent: false,
                    hover: false,
                },
            ),
            r6s(
                "r6-home-rail56-dark",
                T::Dark,
                egui::vec2(900.0, 640.0),
                home,
            ),
            r6s(
                "r6-home-norail-light",
                T::Light,
                egui::vec2(680.0, 760.0),
                home,
            ),
            r6s("r6-new-project-dark", T::Dark, WINDOW, R6Scene::NewProject),
            r6s(
                "r6-new-project-light",
                T::Light,
                WINDOW,
                R6Scene::NewProject,
            ),
            r6s(
                "r6-export-light",
                T::Light,
                WINDOW,
                R6Scene::Export(ExportLook::Settings),
            ),
            r6s(
                "r6-export-hc",
                T::HighContrast,
                WINDOW,
                R6Scene::Export(ExportLook::Settings),
            ),
            r6s(
                "r6-export-2col-dark",
                T::Dark,
                egui::vec2(1000.0, 720.0),
                R6Scene::Export(ExportLook::Conflict),
            ),
            r6s(
                "r6-export-1col-light",
                T::Light,
                egui::vec2(720.0, 900.0),
                R6Scene::Export(ExportLook::Settings),
            ),
            r6s(
                "r6-export-finished-dark",
                T::Dark,
                WINDOW,
                R6Scene::Export(ExportLook::Finished),
            ),
            r6s(
                "r6-export-failed-light",
                T::Light,
                WINDOW,
                R6Scene::Export(ExportLook::Failed),
            ),
            r6s(
                "r6-prefs-appearance-light",
                T::Light,
                WINDOW,
                R6Scene::Prefs(PS::Appearance, ""),
            ),
            r6s(
                "r6-prefs-playback-dark",
                T::Dark,
                WINDOW,
                R6Scene::Prefs(PS::Playback, ""),
            ),
            r6s(
                "r6-prefs-search-dark",
                T::Dark,
                WINDOW,
                R6Scene::Prefs(PS::Appearance, "audio"),
            ),
            r6s(
                "r6-prefs-hc",
                T::HighContrast,
                WINDOW,
                R6Scene::Prefs(PS::Appearance, ""),
            ),
            r6s("r6-about-dark", T::Dark, WINDOW, R6Scene::About),
            r6s("r6-about-light", T::Light, WINDOW, R6Scene::About),
            r6s("r6-toasts-dark", T::Dark, WINDOW, R6Scene::Toasts),
            r6s("r6-toasts-light", T::Light, WINDOW, R6Scene::Toasts),
            r6s("r6-menu-dark", T::Dark, WINDOW, R6Scene::Menu),
            r6s("r6-menu-light", T::Light, WINDOW, R6Scene::Menu),
            r6s("r6-unsaved-dark", T::Dark, WINDOW, R6Scene::Unsaved),
        ]);
    }
    // FREECUT_SNAPSHOT_ONLY=<prefix> keeps the scenes whose name starts
    // with it (FREECUT_SNAPSHOT_ONLY_R2 is the R2 set).
    let only = std::env::var("FREECUT_SNAPSHOT_ONLY")
        .ok()
        .or_else(|| std::env::var_os("FREECUT_SNAPSHOT_ONLY_R2").map(|_| "r2".to_owned()));
    let scenes = scenes
        .into_iter()
        .chain(r2)
        .chain(r4)
        .chain(r5)
        .chain(r6)
        .chain(r3)
        .filter(|s| only.as_deref().is_none_or(|o| s.name.starts_with(o)));
    for scene in scenes {
        let project = match scene.then {
            Then::Home | Then::R6(R6Scene::Home { .. }) | Then::R6(R6Scene::NewProject) => None,
            Then::Assets(Browse::Empty) => {
                let mut p = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
                p.file = Some(media_dir.path().join("Empty.freecut"));
                p.editor.mark_saved();
                Some(p)
            }
            Then::Assets(_) => Some(rich_project(media_dir.path())),
            Then::Inspect(what) => Some(inspect_project(media_dir.path(), what)),
            Then::Timeline(_) => Some(timeline_project(media_dir.path())),
            Then::R3(_) => Some(toolbar_project(media_dir.path())),
            _ => Some(demo_project(media_dir.path())),
        };
        let mut harness = harness(project, scene.theme, scene.size, out.is_some());
        settle(&mut harness);
        match scene.then {
            Then::Nothing | Then::Home => {}
            Then::Inspect(what) => {
                let (kind, lens, everything) = match what {
                    Inspect::Text(lens, all) => (SelectionKind::Text, lens, all),
                    Inspect::Video(lens) => (SelectionKind::Video, lens, false),
                    Inspect::Look(_) | Inspect::LookLut { .. } => {
                        (SelectionKind::Video, Lens::Look, false)
                    }
                    Inspect::Project => (SelectionKind::Nothing, Lens::Project, false),
                    Inspect::Several => (SelectionKind::Several, Lens::Style, false),
                };
                let ws = &mut harness.state_mut().inspector_mut().workspace;
                ws.set_lens(kind, lens);
                if everything {
                    ws.detail = Detail::Everything;
                }
                // Fold the lens's first sections so the ones below show.
                let folded: &[CardId] = match (kind, lens, everything) {
                    (SelectionKind::Text, Lens::Motion, _) => &[CardId::Animation],
                    (SelectionKind::Text, Lens::BoxBubble, _) => &[CardId::Placement],
                    (SelectionKind::Text, Lens::Style, true) => &[CardId::Type, CardId::Fill],
                    (SelectionKind::Video, Lens::Motion, _) => &[CardId::Animation],
                    (SelectionKind::Video, Lens::Look, _) => match what {
                        Inspect::LookLut {
                            fold_color: true, ..
                        } => &[CardId::Light, CardId::ColorBalance],
                        Inspect::LookLut { .. } => &[CardId::Light],
                        _ => &[],
                    },
                    _ => &[],
                };
                for card in folded {
                    ws.card(kind, *card).open = false;
                }
                if let Inspect::Look(page) = what {
                    ws.set_look_page(kind, page);
                }
                if let Inspect::LookLut { .. } = what {
                    ws.set_look_page(kind, freecut_timeline::model::ColorPage::Basic);
                    // The LUT section's More is open, to show Interpolation.
                    ws.card(kind, CardId::Lut).more = true;
                }
                settle(&mut harness);
            }
            Then::Assets(browse) => browse_assets(&mut harness, browse),
            Then::R6(scene) => r6_scene(&mut harness, scene, media_dir.path()),
            Then::Timeline(look) => look_at_timeline(&mut harness, look),
            Then::R3(look) => r3_scene(&mut harness, look),
            Then::HoverGutter => {
                let panel =
                    egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("media"))
                        .expect("the asset panel was shown");
                let edge = panel.outer_rect.right_center();
                harness.hover_at(edge);
                for _ in 0..20 {
                    harness.step();
                }
            }
            Then::ZoomStage => {
                let ppp = harness.ctx.pixels_per_point();
                harness
                    .state_mut()
                    .set_stage_view(crate::ui::player::StageView {
                        fitted: false,
                        zoom: 2.0 / ppp,
                        pan: egui::vec2(-120.0, 40.0),
                    });
                settle(&mut harness);
            }
        }
        let Some(dir) = &out else { continue };
        std::fs::create_dir_all(dir).unwrap();
        let image = harness.render().expect("the window renders");
        let path = dir.join(format!("{}.png", scene.name));
        image.save(&path).unwrap();
        eprintln!("snapshot: {}", path.display());
    }
}

/// The demo project selected as `what` shows, with a title styled so the
/// inspector has something to say: a larger size (the changed dot), an
/// outline and a shadow layer, a background box, and keys on its Type
/// section at the playhead.
fn inspect_project(dir: &Path, what: Inspect) -> Project {
    use freecut_timeline::edit::{EditRequest, KeyEdit, TextStyleEdit};
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::model::{
        ClipContent, MediaKind, PropGroup, PropPath, RefPx, TextCard, TextLayerKind, TextPropPath,
    };

    let mut project = demo_project(dir);
    let title = project.selection[0];
    let seq = project.doc().root();
    for edit in [
        TextStyleEdit::Size(RefPx::from_px(88)),
        TextStyleEdit::SetLayer {
            layer: TextLayerKind::Outline,
            on: true,
        },
        TextStyleEdit::SetLayer {
            layer: TextLayerKind::Shadow,
            on: true,
        },
        TextStyleEdit::SetLayer {
            layer: TextLayerKind::Background,
            on: true,
        },
    ] {
        let req = EditRequest::SetTextStyle {
            seq,
            clips: vec![title],
            edit,
        };
        project.apply(&req, Coalesce::Never).unwrap();
    }
    let req = EditRequest::EditKeys {
        seq,
        clips: vec![title],
        prop: PropPath::Text(TextPropPath::Size),
        edit: KeyEdit::KeyCard {
            card: PropGroup::Text(TextCard::Type),
            at: project.playhead,
        },
    };
    project.apply(&req, Coalesce::Never).unwrap();
    let video = project
        .doc()
        .sequence(seq)
        .unwrap()
        .tracks()
        .flat_map(|(_, t)| t.clips())
        .find(|c| {
            matches!(
                c.content,
                ClipContent::Media {
                    kind: MediaKind::Video,
                    ..
                }
            )
        })
        .map(|c| c.id);
    project.selection = match (what, video) {
        (Inspect::Text(..), _) | (_, None) => vec![title],
        (Inspect::Video(_) | Inspect::Look(_) | Inspect::LookLut { .. }, Some(v)) => vec![v],
        (Inspect::Project, _) => Vec::new(),
        (Inspect::Several, Some(v)) => vec![title, v],
    };
    if matches!(what, Inspect::Project) {
        project.selection.clear();
    }
    if let (Inspect::Look(_) | Inspect::LookLut { .. }, Some(v)) = (what, video) {
        use freecut_timeline::edit::ColorEdit;
        use freecut_timeline::model::{ColorParamId as C, PropValue};
        let req = EditRequest::EditColor {
            seq,
            clips: vec![v],
            edit: ColorEdit::SetParams(vec![
                (C::Contrast, 18),
                (C::Temperature, 35),
                (C::Saturation, 12),
                (C::Amount, 85),
            ]),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        // K2: two HSL bands, a master S-curve and a red curve, two pucks
        // (one keyed), a luminance and Strength 80 %.
        use freecut_timeline::model::{
            Curve, CurveChannel, CurvePoint, HslChannel, HueBand, WheelId,
        };
        let curve = |pts: &[(u16, u16)]| {
            Curve::new(pts.iter().map(|(x, y)| CurvePoint::new(*x, *y)).collect()).unwrap()
        };
        for edit in [
            ColorEdit::SetParams(vec![
                (
                    C::Hsl {
                        band: HueBand::Orange,
                        channel: HslChannel::Saturation,
                    },
                    -30,
                ),
                (
                    C::Hsl {
                        band: HueBand::Blue,
                        channel: HslChannel::Hue,
                    },
                    25,
                ),
                (C::WheelLuma(WheelId::Highlights), -15),
                (C::WheelStrength, 80),
            ]),
            ColorEdit::SetCurve {
                channel: CurveChannel::Luma,
                curve: curve(&[(0, 600), (2_500, 1_900), (7_500, 8_300), (10_000, 9_600)]),
            },
            ColorEdit::SetCurve {
                channel: CurveChannel::Red,
                curve: curve(&[(0, 0), (4_500, 5_300), (10_000, 10_000)]),
            },
            ColorEdit::SetPuck {
                wheel: WheelId::Shadows,
                x: 420,
                y: -380,
            },
        ] {
            let req = EditRequest::EditColor {
                seq,
                clips: vec![v],
                edit,
            };
            project.apply(&req, Coalesce::Never).unwrap();
        }
        for (secs, x, y) in [(0, -300, 200), (4, 350, 450)] {
            let req = EditRequest::EditKeys {
                seq,
                clips: vec![v],
                prop: PropPath::ColorPuck(WheelId::Highlights),
                edit: KeyEdit::Add {
                    at: freecut_timeline::time::Tick::from_secs(secs).unwrap(),
                    value: Some(PropValue::Puck { x, y }),
                },
            };
            project.apply(&req, Coalesce::Never).unwrap();
        }
        // K3: a LUT at 60 % with Skin protection (FreeCut's bundled split
        // look, or a file this machine lacks), the Color section's Skin
        // protection, and a keyed Intensity.
        let lut = match what {
            Inspect::LookLut { missing: true, .. } => {
                freecut_timeline::model::LutRef::new([0xab; 16], "Night drive.cube")
            }
            _ => crate::luts::bundled()
                .into_iter()
                .find(|b| b.file == "split-teal-warm-01.cube")
                .expect("the bundled split look")
                .lut_ref(),
        };
        for edit in [
            ColorEdit::SetLut(Some(freecut_timeline::model::LutSlot::new(lut))),
            ColorEdit::SetSkin {
                scope: freecut_timeline::model::SkinScope::Lut,
                on: true,
            },
            ColorEdit::SetSkin {
                scope: freecut_timeline::model::SkinScope::Color,
                on: true,
            },
            ColorEdit::SetParam(C::SkinAmount(freecut_timeline::model::SkinScope::Color), 55),
        ] {
            let req = EditRequest::EditColor {
                seq,
                clips: vec![v],
                edit,
            };
            project.apply(&req, Coalesce::Never).unwrap();
        }
        for (secs, value) in [(0, 40), (4, 80)] {
            let req = EditRequest::EditKeys {
                seq,
                clips: vec![v],
                prop: PropPath::Color(C::LutIntensity),
                edit: KeyEdit::Add {
                    at: freecut_timeline::time::Tick::from_secs(secs).unwrap(),
                    value: Some(PropValue::Level(value)),
                },
            };
            project.apply(&req, Coalesce::Never).unwrap();
        }
        for (secs, value) in [(0, -10), (4, 25)] {
            let req = EditRequest::EditKeys {
                seq,
                clips: vec![v],
                prop: PropPath::Color(C::Exposure),
                edit: KeyEdit::Add {
                    at: freecut_timeline::time::Tick::from_secs(secs).unwrap(),
                    value: Some(PropValue::Level(value)),
                },
            };
            project.apply(&req, Coalesce::Never).unwrap();
        }
    }
    project.editor.mark_saved();
    project
}

/// The R5 timeline's project: the beach video, a second clip after it on
/// the main track, a picture on a track above, the music with a fade-in
/// and volume keys under it, the beach sound extracted onto a locked,
/// muted track, and the title with an In and an Out. The title and the
/// music are selected.
fn timeline_project(dir: &Path) -> Project {
    use freecut_timeline::edit::{AudioEdit, EditRequest};
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::model::{
        ClipContent, MediaKind, MediaSource, PresetId, Slot, TrackFlags, TrackHeader,
    };

    let mut project = rich_project(dir);
    let doc = project.doc().clone();
    let seq = doc.root();
    let items: Vec<_> = doc.library().items().cloned().collect();
    let title = project.selection[0];
    let s = |n: i64| Tick::from_secs(n).unwrap();
    if items.len() >= 6 {
        let main = doc.sequence(seq).unwrap().main_track().unwrap().id;
        let id_of = |name: &str| {
            items
                .iter()
                .find(|m| m.path.absolute.ends_with(name))
                .map(|m| m.id)
                .unwrap()
        };
        crate::ui::timeline::insert_media(
            &mut project,
            id_of("waves.mp4"),
            InsertTarget::Track(main),
            s(6),
        )
        .unwrap();
        crate::ui::timeline::insert_media(
            &mut project,
            id_of("photo.png"),
            InsertTarget::NewTrack,
            s(4),
        )
        .unwrap();
        let extracted = items
            .iter()
            .find(|m| matches!(m.source, MediaSource::AudioOf { .. }))
            .map(|m| m.id)
            .unwrap();
        crate::ui::timeline::insert_media(&mut project, extracted, InsertTarget::NewTrack, s(1))
            .unwrap();
    }
    let doc = project.doc().clone();
    let clips: Vec<_> = doc
        .sequence(seq)
        .unwrap()
        .tracks()
        .flat_map(|(_, t)| t.clips().iter().cloned())
        .collect();
    let music = clips.iter().find(|c| {
        matches!(
            &c.content,
            ClipContent::Media { kind: MediaKind::Audio, asset, .. }
                if doc.library().get(*asset).is_some_and(|m| m.path.absolute.ends_with("music.wav"))
        )
    });
    if let Some(music) = music.map(|c| c.id) {
        let req = EditRequest::SetClipAudio {
            seq,
            clips: vec![music],
            edit: AudioEdit::FadeIn(s(1)),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        crate::clip_keys::add_key(&mut project, music, s(3)).unwrap();
        crate::clip_keys::add_key(&mut project, music, s(5)).unwrap();
        project.selection = vec![title, music];
    }
    let extracted = clips.iter().find(|c| {
        matches!(
            &c.content,
            ClipContent::Media { kind: MediaKind::Audio, asset, .. }
                if doc.library().get(*asset).is_some_and(|m| matches!(m.source, MediaSource::AudioOf { .. }))
        )
    });
    if let Some(clip) = extracted {
        let req = EditRequest::SetClipAudio {
            seq,
            clips: vec![clip.id],
            edit: AudioEdit::Muted(true),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        let (_, track) = doc.sequence(seq).unwrap().find_clip(clip.id).unwrap();
        let req = EditRequest::SetTrackHeader {
            seq,
            track: track.id,
            header: TrackHeader {
                name: None,
                flags: TrackFlags {
                    locked: true,
                    ..track.header.flags
                },
            },
        };
        project.apply(&req, Coalesce::Never).unwrap();
    }
    let table = freecut_timeline::motion::PresetTable::samples();
    for (slot, id) in [
        (Slot::Entrance, "fc.in.typewriter"),
        (Slot::Exit, "fc.out.fade"),
    ] {
        let choice = crate::anim::slot_presets(&table, slot, true)
            .into_iter()
            .find(|c| c.id == PresetId::new(id))
            .unwrap();
        let req = EditRequest::SetClipAnimation {
            seq,
            clips: vec![title],
            slot,
            spec: Some(crate::anim::chosen_spec(&choice)),
        };
        project.apply(&req, Coalesce::Never).unwrap();
    }
    project.playhead = s(3);
    project.editor.mark_saved();
    project
}

/// The R5 timeline project with markers (TL-037): a blue point and an
/// orange range on the timeline, a green marker on the beach video, the
/// range selected; the title over the video is attached to it (TL-029).
fn toolbar_project(dir: &Path) -> Project {
    use freecut_timeline::edit::{EditRequest, MarkerOwner, MarkerPatch};
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::model::PaletteColor;

    let mut project = timeline_project(dir);
    let seq = project.doc().root();
    let s = |n: i64| Tick::from_secs(n).unwrap();
    let add = |project: &mut Project, at: Tick, owner: MarkerOwner, color, name: &str| {
        let req = EditRequest::AddMarker {
            seq,
            at,
            owner,
            color: Some(color),
        };
        project.apply(&req, Coalesce::Never).unwrap();
        let m = *project.markers.iter().next().unwrap();
        let patch = MarkerPatch {
            name: Some(name.to_owned()),
            ..MarkerPatch::default()
        };
        project.edit_marker(m, patch).unwrap();
        m
    };
    add(
        &mut project,
        s(1),
        MarkerOwner::Timeline,
        PaletteColor::Blue,
        "Intro",
    );
    let range = add(
        &mut project,
        s(5),
        MarkerOwner::Timeline,
        PaletteColor::Orange,
        "B-roll",
    );
    project
        .edit_marker(
            range,
            MarkerPatch {
                duration: Some(s(2)),
                ..MarkerPatch::default()
            },
        )
        .unwrap();
    let main = project
        .doc()
        .sequence(seq)
        .unwrap()
        .main_track()
        .and_then(|t| t.clips().first().map(|c| c.id));
    if let Some(video) = main {
        add(
            &mut project,
            s(2),
            MarkerOwner::Clip(video),
            PaletteColor::Green,
            "Wave",
        );
    }
    project.markers = std::collections::BTreeSet::from([range]);
    project.selection.clear();
    project.playhead = s(3);
    project.editor.mark_saved();
    project
}

/// Puts the timeline into the state `look` shows (R3).
fn r3_scene(harness: &mut Harness<'_, FreeCutApp>, look: R3Look) {
    use egui_kittest::kittest::Queryable;
    match look {
        R3Look::Plain => {}
        R3Look::Blade => {
            harness.state_mut().timeline_mut().px_per_sec = 110.0;
            settle(harness);
            let point = {
                let app = harness.state();
                let project = app.project_for_tests().unwrap();
                let view = app.timeline_for_tests();
                let doc = project.doc();
                let seq = doc.sequence(doc.root()).unwrap();
                let video = seq.main_track().unwrap().clips()[0].id;
                let s2 = Tick::from_secs(2).unwrap();
                let at = Tick::from_raw(s2.raw() + s2.raw() / 4);
                crate::ui::timeline::tests::clip_point(project, view, view.shown(), video, at)
            };
            harness
                .state_mut()
                .timeline_mut()
                .tool
                .set(crate::ui::timeline_tools::TimelineTool::Blade);
            harness.hover_at(point);
            for _ in 0..10 {
                harness.step();
            }
        }
        R3Look::Editor => {
            let app = harness.state_mut();
            let project = app.project_for_tests().unwrap();
            let doc = project.doc().clone();
            let r = *project.markers.iter().next().unwrap();
            let view = app.timeline_for_tests();
            let anchor = crate::ui::timeline::tests::ruler_point(
                view,
                view.shown(),
                Tick::from_secs(5).unwrap(),
            );
            app.timeline_mut().open_marker_editor(&doc, r, anchor);
            for _ in 0..6 {
                harness.step();
            }
        }
        R3Look::Recorder(look) => recorder_scene(harness, look),
        R3Look::Menu(label) => {
            let at = harness.get_by_label(label).rect().center();
            harness.hover_at(at);
            harness.step();
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                harness.step();
            }
            for _ in 0..6 {
                harness.step();
            }
        }
    }
}

/// Steps the window until `done` holds (the recorder runs in real time).
fn step_until(harness: &mut Harness<'_, FreeCutApp>, done: impl Fn(&FreeCutApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done(harness.state()) {
        assert!(Instant::now() < deadline, "the recorder never got there");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..4 {
        harness.step();
    }
}

/// Puts the recorder panel into the state `look` shows (AU-027, TL-014).
fn recorder_scene(harness: &mut Harness<'_, FreeCutApp>, look: RecorderLook) {
    use crate::ui::recorder_panel::{Phase, Waiting};
    let ready = |a: &FreeCutApp| a.recorder_for_tests().phase() == &Phase::Ready;
    if let RecorderLook::Skim = look {
        let point = {
            let app = harness.state();
            let project = app.project_for_tests().unwrap();
            let view = app.timeline_for_tests();
            let doc = project.doc();
            let video = doc
                .sequence(doc.root())
                .unwrap()
                .main_track()
                .unwrap()
                .clips()[0]
                .id;
            let at = Tick::from_raw(Tick::from_secs(1).unwrap().raw() * 3 / 2);
            crate::ui::timeline::tests::clip_point(project, view, view.shown(), video, at)
        };
        harness.state_mut().timeline_mut().skim.toggle();
        harness.hover_at(point);
        for _ in 0..10 {
            harness.step();
        }
        settle(harness);
        return;
    }
    if let RecorderLook::Denied = look {
        harness
            .state_mut()
            .recorder_mut_for_tests()
            .set_backend(crate::recorder::Backend::new(
                |_, _, _| {
                    Err(freecut_audio::record::RecordError::Denied(
                        "not allowed".to_owned(),
                    ))
                },
                || Ok(Vec::new()),
            ));
    }
    harness
        .state_mut()
        .voiceover_for_tests(|v| v.countdown_secs = 0);
    harness.state_mut().open_recorder_for_tests();
    match look {
        RecorderLook::Denied => step_until(harness, |a| {
            a.recorder_for_tests().phase() == &Phase::Waiting(Waiting::Denied)
        }),
        RecorderLook::Ready => {
            step_until(harness, ready);
            // Let the meter move.
            let until = Instant::now() + Duration::from_millis(300);
            while Instant::now() < until {
                harness.step();
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        RecorderLook::Recording | RecorderLook::Takes => {
            step_until(harness, ready);
            harness.state_mut().record_for_tests();
            step_until(harness, |a| {
                a.recorder_for_tests()
                    .elapsed()
                    .is_some_and(|e| e > Tick::from_secs(1).unwrap())
            });
            if let RecorderLook::Takes = look {
                harness.state_mut().recorder_mut_for_tests().cancel();
                step_until(harness, |a| {
                    !a.recorder_for_tests().loose_takes().is_empty()
                });
                // The Takes list is at the end of the panel: scroll to it.
                let button = harness.state().timeline_for_tests().record_button;
                harness.hover_at(egui::pos2(button.center().x - 100.0, button.top() - 150.0));
                harness.event(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -2000.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
                for _ in 0..10 {
                    harness.step();
                }
            }
        }
        RecorderLook::Skim => {}
    }
}

/// Puts the timeline into the state `look` shows.
fn look_at_timeline(harness: &mut Harness<'_, FreeCutApp>, look: TimelineLook) {
    use freecut_timeline::model::{ClipContent, MediaKind};
    match look {
        TimelineLook::Plain => {}
        TimelineLook::Zoomed => {
            let view = harness.state_mut().timeline_mut();
            view.px_per_sec = 240.0;
            view.scroll_secs = 0.5;
            settle(harness);
        }
        TimelineLook::HoverVolume => {
            // The music is below the fold: scroll the tracks to the end.
            harness.state_mut().timeline_mut().track_scroll = 10_000.0;
            settle(harness);
            let (point, _) = {
                let app = harness.state();
                let doc = app.project_doc().unwrap();
                let seq = doc.sequence(doc.root()).unwrap();
                let music = seq
                    .audio_tracks()
                    .iter()
                    .flat_map(|t| t.clips())
                    .find(|c| {
                        matches!(
                            c.content,
                            ClipContent::Media {
                                kind: MediaKind::Audio,
                                ..
                            }
                        ) && c.audio.active().is_some_and(|a| !a.muted)
                    })
                    .unwrap()
                    .id;
                let project = app.project_for_tests().unwrap();
                let view = app.timeline_for_tests();
                let at = Tick::from_secs(4).unwrap();
                (
                    crate::ui::timeline::tests::line_point(project, view, view.shown(), music, at),
                    music,
                )
            };
            harness.hover_at(point);
            for _ in 0..20 {
                harness.step();
            }
        }
    }
}

/// Puts the asset browser into the state `browse` shows.
fn browse_assets(harness: &mut Harness<'_, FreeCutApp>, browse: Browse) {
    use crate::ui::assets::AssetTab;
    let ids: Vec<_> = harness
        .state()
        .project_doc()
        .map(|d| d.library().items().map(|m| m.id).collect())
        .unwrap_or_default();
    {
        let assets = harness.state_mut().assets_mut();
        match browse {
            Browse::Media | Browse::Band | Browse::Empty => {}
            Browse::Selected => {
                assets.media.selection = ids.iter().take(3).copied().collect();
            }
            Browse::Audio => assets.tab = AssetTab::Audio,
            Browse::Text => assets.tab = AssetTab::Text,
            Browse::NoMatches => assets.media.search = "sunset".to_owned(),
        }
    }
    settle(harness);
    let panel = egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("media"))
        .expect("the asset panel was shown")
        .outer_rect;
    match browse {
        Browse::Selected => {
            // Hover the second tile of the first row: its remove button.
            harness.hover_at(egui::pos2(panel.right() - 60.0, panel.top() + 160.0));
            for _ in 0..20 {
                harness.step();
            }
        }
        Browse::Band => {
            let from = egui::pos2(panel.right() - 24.0, panel.bottom() - 24.0);
            let to = egui::pos2(panel.right() - 150.0, panel.top() + 200.0);
            harness.hover_at(from);
            harness.step();
            harness.event(egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            for i in 1..=10 {
                let t = i as f32 / 10.0;
                harness.hover_at(from + (to - from) * t);
                harness.step();
            }
            for _ in 0..5 {
                harness.step();
            }
        }
        _ => {}
    }
}

/// Home's demo projects: six recent projects, covers from the generated
/// media for four of them, one whose file is gone.
fn seed_home(harness: &mut Harness<'_, FreeCutApp>, dir: &Path) {
    use crate::recent::{RecentEntry, unix_now};
    // The media files the covers come from.
    let _ = rich_project(dir);
    let now = unix_now();
    let base = PathBuf::from("/Users/you/Movies/FreeCut");
    let entry = |name: &str, canvas, rate, secs: u64, ago: u64, found| RecentEntry {
        path: base.join(format!("{name}.freecut")),
        name: name.to_owned(),
        opened: now - ago,
        canvas,
        rate,
        duration_ms: secs * 1000,
        found,
    };
    let entries = [
        (
            entry("Beach trip", (1920, 1080), (30, 1), 42, 90, true),
            Some("beach.mp4"),
        ),
        (
            entry(
                "Portrait reel",
                (1080, 1920),
                (30_000, 1001),
                65,
                3 * 3600,
                true,
            ),
            Some("portrait.mp4"),
        ),
        (
            entry("Waves", (1920, 1080), (25, 1), 3, 30 * 3600, true),
            Some("waves.mp4"),
        ),
        (
            entry(
                "Mandelbrot zoom",
                (1080, 1080),
                (30, 1),
                12,
                5 * 86_400,
                true,
            ),
            Some("photo.png"),
        ),
        (
            entry(
                "Birthday party",
                (1080, 1350),
                (30, 1),
                184,
                15 * 86_400,
                true,
            ),
            None,
        ),
        (
            entry(
                "Old interview",
                (2560, 1080),
                (24, 1),
                1_520,
                80 * 86_400,
                false,
            ),
            None,
        ),
    ];
    let ctx = harness.ctx.clone();
    for (e, cover) in entries.into_iter().rev() {
        let path = e.path.clone();
        harness.state_mut().recent_mut().touch(e, Instant::now());
        if let Some(file) = cover
            && let Ok(image) = freecut_media::poster(&dir.join(file), 480)
        {
            harness.state_mut().insert_cover(&ctx, path, image);
        }
    }
}

/// Steps until the Export sheet's checks are back (or 20 s pass).
fn wait_export_ready(harness: &mut Harness<'_, FreeCutApp>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline
        && !harness
            .state_mut()
            .export_mut()
            .is_some_and(|d| d.is_ready())
    {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..6 {
        harness.step();
    }
}

/// Puts the window into the R6 `scene`.
fn r6_scene(harness: &mut Harness<'_, FreeCutApp>, scene: R6Scene, dir: &Path) {
    use crate::ui::export::Stage;
    use crate::ui::toast::{ToastAction, ToastKind};
    match scene {
        R6Scene::Home { recent, hover } => {
            if recent {
                seed_home(harness, dir);
            }
            for _ in 0..6 {
                harness.step();
            }
            if hover {
                harness.hover_at(egui::pos2(690.0, 330.0));
                for _ in 0..6 {
                    harness.step();
                }
            }
        }
        R6Scene::NewProject => {
            seed_home(harness, dir);
            harness.state_mut().open_for_tests(Open::NewProject);
            for _ in 0..6 {
                harness.step();
            }
        }
        R6Scene::Export(look) => {
            let out = dir.join("exports");
            std::fs::create_dir_all(&out).unwrap();
            let existing = out.join("Beach trip.mp4");
            if matches!(look, ExportLook::Conflict) {
                std::fs::write(&existing, b"").unwrap();
            } else {
                let _ = std::fs::remove_file(&existing);
            }
            harness
                .state_mut()
                .open_for_tests(Open::Export(out.clone()));
            wait_export_ready(harness);
            let dialog = harness.state_mut().export_mut().unwrap();
            match look {
                ExportLook::Settings | ExportLook::Conflict => {}
                ExportLook::Finished => {
                    dialog.stage = Stage::Finished(crate::jobs::export::ExportResult {
                        path: out.join("Beach trip.mp4"),
                        bytes: 14_820_000,
                        duration: Tick::from_secs(6).unwrap(),
                        size: (1920, 1080),
                        frame_rate: FrameRate::new(30, 1).unwrap(),
                        frames: 180,
                        audio: true,
                        took: Duration::from_secs(9),
                        slated: Vec::new(),
                    });
                }
                ExportLook::Failed => {
                    dialog.stage = Stage::Failed(crate::jobs::export::ExportFailure {
                        code: crate::jobs::export::FailureCode::DiskFull,
                        detail: "write: no space left on device".to_owned(),
                        subject: Some("Macintosh HD".to_owned()),
                    });
                }
            }
            for _ in 0..6 {
                harness.step();
            }
        }
        R6Scene::Prefs(section, search) => {
            harness
                .state_mut()
                .open_for_tests(Open::Preferences(section, search));
            for _ in 0..6 {
                harness.step();
            }
        }
        R6Scene::About => {
            harness.state_mut().open_for_tests(Open::About);
            for _ in 0..6 {
                harness.step();
            }
        }
        R6Scene::Unsaved => {
            harness.state_mut().open_for_tests(Open::Unsaved);
            for _ in 0..6 {
                harness.step();
            }
        }
        R6Scene::Toasts => {
            let app = harness.state_mut();
            app.toast_for_tests(ToastKind::Info, "Snapping off", Vec::new());
            app.toast_for_tests(
                ToastKind::Success,
                "Export finished: Beach trip.mp4",
                vec![(
                    "Show in folder".to_owned(),
                    ToastAction::Reveal(dir.join("Beach trip.mp4")),
                )],
            );
            app.toast_for_tests(
                ToastKind::Error,
                "Couldn't save: the disk is full",
                vec![("Retry".to_owned(), ToastAction::RetrySave)],
            );
            for _ in 0..4 {
                harness.step();
            }
        }
        R6Scene::Menu => {
            let at = egui::pos2(94.0, 18.0);
            harness.hover_at(at);
            harness.step();
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                harness.step();
            }
            harness.hover_at(egui::pos2(130.0, 76.0));
            for _ in 0..6 {
                harness.step();
            }
        }
    }
}

/// Drags a gutter from `from` to `to` in a few steps.
fn drag(harness: &mut Harness<'_, FreeCutApp>, from: egui::Pos2, to: egui::Pos2) {
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.hover_at(from);
    harness.step();
    harness.event(press(from, true));
    harness.step();
    for i in 1..=10 {
        harness.hover_at(from + (to - from) * (i as f32 / 10.0));
        harness.step();
    }
    harness.event(press(to, false));
    for _ in 0..4 {
        harness.step();
    }
}

/// §3.2 / §13.17: dragging the gutters as far as they go keeps every
/// region at its minimum: the inspector stays in the window, the Stage
/// keeps 320 pt, and the upper row keeps its height (no panic on a
/// collapsed row).
#[test]
fn gutters_stop_at_the_regions_minimums() {
    use egui::containers::panel::PanelState;

    use crate::widgets::panel::{INSPECTOR_MIN, STAGE_MIN, UPPER_MIN};

    let media_dir = tempfile::tempdir().unwrap();
    let project = demo_project(media_dir.path());
    let mut harness = harness(Some(project), ThemeChoice::Dark, WINDOW, false);
    settle(&mut harness);
    let rect = |h: &Harness<'_, FreeCutApp>, id: &str| {
        PanelState::load(&h.ctx, egui::Id::new(id))
            .expect("the panel was shown")
            .outer_rect
    };
    let gutter = crate::theme::look(&harness.ctx).metrics.gutter;

    let media = rect(&harness, "media");
    drag(
        &mut harness,
        media.right_center(),
        egui::pos2(WINDOW.x - 40.0, media.center().y),
    );
    let (media, inspector) = (rect(&harness, "media"), rect(&harness, "inspector"));
    assert!(inspector.right() <= WINDOW.x + 0.5, "{inspector:?}");
    assert!(
        inspector.width() >= INSPECTOR_MIN + gutter * 1.5 - 0.5,
        "{inspector:?}"
    );
    let stage = inspector.left() - media.right() - gutter;
    assert!(stage >= STAGE_MIN - 0.5, "the Stage keeps {stage}");

    let timeline = rect(&harness, "timeline");
    drag(
        &mut harness,
        timeline.center_top(),
        egui::pos2(timeline.center().x, 20.0),
    );
    let (timeline, media) = (rect(&harness, "timeline"), rect(&harness, "media"));
    assert!(
        media.height() >= UPPER_MIN + gutter / 2.0 - 0.5,
        "the upper row keeps {media:?} above {timeline:?}"
    );
}

/// UI-006: at the default layout the asset search field keeps room for a
/// typed term (the Import button turns into its glyph to make room).
#[test]
fn the_asset_search_keeps_room_for_a_term() {
    use crate::ui::assets::SEARCH_MIN;

    let media_dir = tempfile::tempdir().unwrap();
    for size in [WINDOW, egui::vec2(1100.0, 700.0)] {
        let project = demo_project(media_dir.path());
        let mut harness = harness(Some(project), ThemeChoice::Dark, size, false);
        settle(&mut harness);
        let search = egui::Id::new("media-toolbar").with("search");
        let field = harness
            .ctx
            .read_response(search)
            .expect("the search field")
            .rect;
        assert!(field.width() >= SEARCH_MIN - 0.5, "{size:?}: {field:?}");
    }
}

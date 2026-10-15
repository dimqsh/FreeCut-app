//! Top-level application state: the open project, background jobs, the
//! preview, sound playback, and the window layout.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use freecut_timeline::Committed;
use freecut_timeline::edit::Notice;
use freecut_timeline::ids::ClipId;

use eframe::egui::{
    self, ColorImage, Id, Key, KeyboardShortcut, Modifiers, Panel, TextureHandle, TextureOptions,
    ViewportCommand,
};
use freecut_media::RgbaImage;
use freecut_media::color::HdrTransfer;
use freecut_render::FontRegistry;
use freecut_render::text::ScanState;
use freecut_timeline::edit::AudioEdit;
use freecut_timeline::edit::{Cut, DeleteReq, EditRequest};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::MediaId;
use freecut_timeline::model::{ProjectSettings, Sequence};
use freecut_timeline::motion::PresetTable;
use freecut_timeline::time::{FrameRate, Tick};

use crate::clip_keys;
use crate::clip_sound::{ClipCommand, restore_removals};
use crate::clip_streams;
use crate::hdr::{HdrNotices, hdr_notice};
use crate::imports::{Imports, Ticket};
use crate::jobs::export::{ExportEnd, ExportJob, ExportRequest};
use crate::jobs::platform::{
    Destination, Os, default_export_folder, encoder_available, home_dir, open_path,
    probe_destination,
};
use crate::jobs::{Jobs, default_workers};
use crate::preview::{LayerBox, Preview, PreviewRequest, TextDraft};
use crate::recent::{CoverFrame, RecentEntry, RecentProjects};
use crate::session::{
    Loaded, ProbedFile, Project, SavedJob, import_report, op_name, open_project, probe_for_import,
    sound_only,
};
use crate::settings::{self, SettingsStore};
use crate::sound::{Playback, Sound};
use crate::ui;
use crate::ui::assets::{AssetsState, ImportRequest};
use crate::ui::canvas::CanvasView;
use crate::ui::drag::DragGesture;
use crate::ui::export::{ExportAction, ExportDialog};
use crate::ui::home::{
    CANVAS_PRESETS, HomeAction, NewProjectChoice, NewProjectDialog, RATE_PRESETS,
};
use crate::ui::mixer::MixerState;
use crate::ui::preferences::Preferences;
use crate::ui::props::FontChoices;
use crate::ui::recorder_panel::{PanelAction, PanelEvent, Placement, RecorderPanel};
use crate::ui::toast::{ToastAction, ToastKind, Toasts};
use crate::waveform::Waveforms;
use crate::widgets::sheet::{ButtonKind, Sheet};
use crate::workspace::{self, WorkspaceStore};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "mts", "m2ts", "3gp", "mp3", "wav", "aac", "m4a",
    "flac", "ogg", "opus", "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "heic",
];
/// The Audio panel's "Import audio…" (AU-001): audio files, plus video
/// containers for their sound.
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "aac", "m4a", "flac", "ogg", "opus", "aif", "aiff", "mp4", "mov", "m4v", "mkv",
    "webm", "avi", "mts", "m2ts", "3gp",
];

/// Work finished on a background thread.
enum Done {
    Probed(Result<ProbedFile, String>, Ticket),
    Opened(Box<Result<Loaded, String>>),
    /// A save of the project with this serial finished.
    Saved(u64, SavedJob),
    Poster(Poster),
    /// The export folder check (EX-023) for `(folder, name)`.
    Destination((PathBuf, String), Destination),
    /// Whether the H.264/AAC encoders exist (EX-029).
    Encoder(bool),
    /// "Open" or "Show in folder" finished (EX-039).
    OsOpened(Result<(), String>),
    /// The preferences file was read (UI-018).
    Settings(Box<settings::Loaded>, PathBuf),
    /// The workspace file was read (UI-002); `None`: none to use.
    Workspace(Option<serde_json::Map<String, serde_json::Value>>, PathBuf),
    /// The recent-projects registry was read (PM-017).
    Recent(
        Option<serde_json::Map<String, serde_json::Value>>,
        Vec<RecentEntry>,
        PathBuf,
    ),
    /// A project's cover was read (`None`: it has none).
    Cover(PathBuf, Option<(u32, u32, Vec<u8>)>),
    /// A project's cover was written.
    CoverWritten(PathBuf, Result<(), String>),
    /// A finished voiceover take was probed for the project with this
    /// serial (AU-027).
    TakeProbed(u64, Placement, Result<ProbedFile, String>),
    /// An accidental take's file was removed.
    TakeRemoved(Result<(), String>),
    /// Takes interrupted by a crash were repaired for the project with this
    /// serial (PM-043): the takes, and what couldn't be repaired.
    Recovered(u64, Vec<PathBuf>, Vec<String>),
    /// The LUT library's index was read at startup (CL-016).
    LutLibrary(Result<freecut_project::luts::LutLibrary, String>),
    /// A LUT import finished (CL-016).
    LutsImported(Box<crate::jobs::luts::ImportReport>),
    /// The saved looks were read, at startup or after a delete (CL-026).
    Looks(Result<freecut_project::look::LookLibrary, String>),
    /// A look was deleted: the library without it (CL-026).
    LookDeleted(Result<freecut_project::look::LookLibrary, String>),
    /// A look was saved: the library with it and its name (CL-026).
    LookSaved(Result<(freecut_project::look::LookLibrary, String), String>),
    /// An Auto or Match analysis finished (CL-019, CL-020).
    ColorAnalysed(Box<crate::jobs::color::Finished>),
}

/// A media tile's poster (PM-050), made for item `id` of the project whose
/// serial was `project` when the job started. Media ids restart with every
/// project, so a poster is the open project's only if `project` still is.
struct Poster {
    project: u64,
    id: MediaId,
    image: Result<RgbaImage, String>,
}

/// Something that would discard unsaved changes, waiting for confirmation.
#[derive(Debug, Clone)]
enum Pending {
    New,
    Open(PathBuf),
    Quit,
}

/// A restore waiting for the user to confirm removing audio clips
/// (TL-030).
struct RestoreConfirm {
    clips: Vec<ClipId>,
    /// "name (start)" of each audio clip that would be removed.
    removed: Vec<String>,
}

/// What the frame on screen was rendered from: a new request goes out when
/// any of it changes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewKey {
    revision: u64,
    time: Tick,
    size: (u32, u32),
    offline: usize,
    fonts: u64,
    draft: Option<TextDraft>,
}

/// The FreeCut editor window.
pub struct FreeCutApp {
    project: Option<Project>,
    jobs: Jobs<Done>,
    preview: Preview,
    preview_tex: Option<TextureHandle>,
    preview_key: Option<PreviewKey>,
    /// Where the clips of the frame on screen landed, for the player's
    /// hit-testing and outlines (TR-007) and the text box size.
    preview_layers: Vec<LayerBox>,
    preview_canvas: (u32, u32),
    /// Bundled fonts now, system fonts once the background scan is done
    /// (TX-003); shared with the preview's text renderer.
    fonts: FontRegistry,
    preview_generation: u64,
    /// Generation of the frame on screen; older frames are never shown.
    preview_shown: u64,
    preview_size: (u32, u32),
    posters: HashMap<MediaId, TextureHandle>,
    posters_requested: HashSet<MediaId>,
    /// HDR items of the open project already announced (CL-002).
    hdr: HdrNotices,
    imports: Imports,
    opening: bool,
    /// Toasts (UI-040): every message the app has for the user.
    toasts: Toasts,
    /// The recent-projects registry (PM-017), Home's cards.
    recent: RecentProjects,
    /// Where covers are kept; `None`: nowhere (no settings folder, tests).
    covers_dir: Option<PathBuf>,
    /// Home's cover textures by project file, and those asked for.
    covers: HashMap<PathBuf, TextureHandle>,
    covers_requested: HashSet<PathBuf>,
    /// The frame on the Stage, kept for the project's cover at save time.
    cover_frame: Option<CoverFrame>,
    timeline: ui::timeline::TimelineView,
    assets: AssetsState,
    inspector: ui::inspector::InspectorState,
    /// A drag of the player's master volume (AU-010).
    master_drag: DragGesture,
    /// The player's on-canvas drag (TR-007).
    player: ui::player::PlayerState,
    playback: Option<Playback>,
    sound: Sound,
    /// App preferences (UI-018); in memory until the file was read.
    settings: SettingsStore,
    /// Workspace state (UI-002); in memory until the file was read.
    workspace: WorkspaceStore,
    /// The inspector's workspace state last handed to `workspace`.
    workspace_seen: Option<ui::inspector::lens::Workspace>,
    /// The Preferences window (UI-018, UI-029).
    prefs: Preferences,
    /// The theme and density applied (UI-013, UI-014).
    theme: crate::theme::ThemeState,
    /// The Mixer panel (AU-013).
    mixer: MixerState,
    /// The sound banner the user closed: it stays hidden until its text
    /// changes (AU-010).
    banner_dismissed: Option<String>,
    waves: Waveforms,
    /// Filmstrip frames for the timeline's clips (R5).
    films: crate::thumbs::Filmstrips,
    /// Tells open projects apart (for the sound engine's caches, and for
    /// poster jobs that finish after another project opened).
    project_serial: u64,
    restore_confirm: Option<RestoreConfirm>,
    new_dialog: Option<NewProjectDialog>,
    pending: Option<Pending>,
    allow_close: bool,
    show_about: bool,
    /// The Export dialog and the export it runs (EX-021, EX-022); created
    /// on first use.
    export: Option<ExportDialog>,
    /// The folder of the last export this session (EX-021's default).
    export_folder: Option<PathBuf>,
    /// The window was asked to close while an export runs.
    quit_during_export: bool,
    /// Why the last save of the open project failed; the top bar shows
    /// "Save failed — Retry" until a save succeeds (UI-012, PM-012).
    save_failed: Option<String>,
    /// FreeCut's animation presets (FX-030, TX-056), compiled in.
    presets: Arc<PresetTable>,
    /// Hover skim's frame requests, paced to the display (TL-014).
    skim_throttle: crate::skim::Throttle,
    /// The skim time the player shows instead of the playhead.
    skim_shown: Option<Tick>,
    /// The voiceover recorder panel (AU-027).
    recorder: RecorderPanel,
    /// The preview mute before a take muted the timeline.
    muted_before_take: Option<bool>,
    /// The app-data folder (recordings of unsaved projects, PM-041);
    /// `None`: none (tests).
    app_data: Option<PathBuf>,
    /// The bundled LUTs and the user's library, the LUT source of the
    /// preview and of exports (CL-016, CL-017, CL-045).
    luts: Arc<crate::luts::AppLuts>,
    /// LUT files the preview couldn't read since the library last changed.
    missing_luts: BTreeSet<[u8; 16]>,
    /// What the Look lens's LUT select offers; handed to the project
    /// every frame.
    lut_choices: Arc<crate::luts::LutChoices>,
    /// The saved looks (CL-026); handed to the project every frame.
    looks: Arc<crate::looks::SavedLooks>,
    /// The open Save look sheet (CL-026, UI-064).
    save_look: Option<ui::save_look::SaveLookDialog>,
    /// A saved look waiting for its delete to be confirmed: `(id, name)`.
    delete_look: Option<(String, String)>,
    /// The last Auto or Match job's number (CL-019, CL-020).
    color_jobs: u64,
}

impl std::fmt::Debug for FreeCutApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreeCutApp").finish_non_exhaustive()
    }
}

impl FreeCutApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> FreeCutApp {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        // Before the first frame: the theme refers to these families.
        crate::theme::fonts::install(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let wake_sound = move || ctx.request_repaint();
        if let Err(e) = freecut_media::init() {
            eprintln!("FreeCut: media support is unavailable: {e}");
        }
        let mut app = FreeCutApp::with_sound(
            &cc.egui_ctx,
            Sound::new(wake_sound),
            FontRegistry::with_system_fonts(),
        );
        // The output device opens once the preferences say which one
        // (UI-029); the file is read on the pool, never here.
        match settings::settings_path() {
            Some(path) => {
                if !app
                    .jobs
                    .spawn(move || Done::Settings(Box::new(settings::load(&path)), path))
                {
                    app.sound
                        .start(app.settings.settings().playback.engine_config());
                }
            }
            None => {
                eprintln!("FreeCut: no settings folder; preferences last until you quit");
                app.sound
                    .start(app.settings.settings().playback.engine_config());
            }
        }
        if let Some(path) = workspace::workspace_path() {
            app.jobs
                .spawn(move || Done::Workspace(workspace::load(&path), path));
        }
        // Recordings of unsaved projects go to app data (PM-041).
        app.app_data = settings::settings_path().and_then(|p| p.parent().map(PathBuf::from));
        // The LUT library's index (CL-016) and the saved looks (CL-026),
        // read on the pool.
        if let Some(data) = app.app_data.clone() {
            let looks = data.clone();
            app.jobs.spawn(move || {
                Done::LutLibrary(
                    freecut_project::luts::LutLibrary::in_app_data(&data)
                        .map_err(|e| e.to_string()),
                )
            });
            app.looks = Arc::new(crate::looks::SavedLooks::default());
            app.jobs
                .spawn(move || Done::Looks(crate::looks::load(&looks)));
        }
        // Home's cards (PM-017): read on the pool, never here.
        app.covers_dir = crate::recent::covers_dir();
        match crate::recent::registry_path() {
            Some(path) => {
                app.jobs.spawn(move || {
                    let (raw, entries) = crate::recent::load(&path);
                    Done::Recent(raw, entries, path)
                });
            }
            None => app.recent.without_file(),
        }
        app
    }

    fn with_sound(egui_ctx: &egui::Context, sound: Sound, fonts: FontRegistry) -> FreeCutApp {
        let ctx = egui_ctx.clone();
        let wake_jobs = move || ctx.request_repaint();
        let ctx = egui_ctx.clone();
        let wake_preview = move || ctx.request_repaint();
        let ctx = egui_ctx.clone();
        let wake_waves = move || ctx.request_repaint();
        let ctx = egui_ctx.clone();
        let wake_posters = move || ctx.request_repaint();
        let ctx = egui_ctx.clone();
        let wake_films = move || ctx.request_repaint();
        let ctx = egui_ctx.clone();
        let wake_recorder: crate::recorder::Wake = Arc::new(move || ctx.request_repaint());
        let presets = Arc::new(load_presets());
        precompute_safety(&presets);
        let luts = Arc::new(crate::luts::AppLuts::new());
        let lut_choices = Arc::new(luts.choices(&BTreeSet::new()));
        let mut inspector = ui::inspector::InspectorState::default();
        inspector.motion.presets = Arc::clone(&presets);
        inspector.motion.posters = Some(crate::posters::PosterWorker::new(
            fonts.clone(),
            Arc::clone(&presets),
            wake_posters,
        ));
        FreeCutApp {
            project: None,
            jobs: Jobs::new(default_workers(), wake_jobs),
            preview: {
                let preview = Preview::new(fonts.clone(), Arc::clone(&presets), wake_preview);
                preview.set_luts(luts.clone());
                preview
            },
            preview_tex: None,
            preview_key: None,
            preview_layers: Vec::new(),
            preview_canvas: (16, 9),
            fonts,
            preview_generation: 0,
            preview_shown: 0,
            preview_size: (960, 540),
            posters: HashMap::new(),
            posters_requested: HashSet::new(),
            hdr: HdrNotices::default(),
            imports: Imports::default(),
            opening: false,
            toasts: Toasts::default(),
            recent: RecentProjects::default(),
            covers_dir: None,
            covers: HashMap::new(),
            covers_requested: HashSet::new(),
            cover_frame: None,
            timeline: ui::timeline::TimelineView::default(),
            assets: AssetsState::default(),
            inspector,
            master_drag: DragGesture::default(),
            player: ui::player::PlayerState::default(),
            playback: None,
            sound,
            settings: SettingsStore::in_memory(),
            workspace: WorkspaceStore::default(),
            workspace_seen: None,
            prefs: Preferences::default(),
            theme: crate::theme::ThemeState::default(),
            mixer: MixerState::default(),
            banner_dismissed: None,
            waves: Waveforms::new(wake_waves),
            films: crate::thumbs::Filmstrips::new(wake_films),
            project_serial: 0,
            restore_confirm: None,
            new_dialog: None,
            pending: None,
            allow_close: false,
            show_about: false,
            export: None,
            export_folder: None,
            quit_during_export: false,
            save_failed: None,
            presets,
            skim_throttle: crate::skim::Throttle::default(),
            skim_shown: None,
            recorder: RecorderPanel::new(recorder_backend(), wake_recorder),
            muted_before_take: None,
            app_data: None,
            luts,
            missing_luts: BTreeSet::new(),
            lut_choices,
            looks: Arc::new(crate::looks::SavedLooks::none()),
            save_look: None,
            delete_look: None,
            color_jobs: 0,
        }
    }

    /// Rebuilds the LUT select's choices after the library or the missing
    /// set changed.
    fn refresh_luts(&mut self) {
        self.lut_choices = Arc::new(self.luts.choices(&self.missing_luts));
    }

    /// Opens the LUT file dialog and imports what was picked (CL-016), on
    /// the pool.
    fn import_luts_dialog(&mut self) {
        let Some(app_data) = self.app_data.clone() else {
            self.say("LUTs can't be imported: FreeCut has no app-data folder here");
            return;
        };
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Import LUT")
            .add_filter("LUT files", &["cube", "3dl"])
            .pick_files()
        else {
            return;
        };
        self.import_luts(paths, &app_data);
    }

    /// Imports LUT files and folders into the library in `app_data`, on the
    /// pool.
    fn import_luts(&mut self, paths: Vec<PathBuf>, app_data: &Path) {
        let dir = app_data.join(freecut_project::luts::LIBRARY_FOLDER);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.jobs.spawn(move || {
            Done::LutsImported(Box::new(crate::jobs::luts::import(&paths, &dir, now)))
        });
    }

    /// Runs what the Look lens's Auto or Match section asked (CL-019,
    /// CL-020): the frames are chosen here from the document, decoded and
    /// measured on the pool.
    fn color_request(&mut self, request: ui::inspector::ColorRequest) {
        use crate::jobs::color::{self, Analysis};
        use ui::inspector::ColorRequest as R;
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let (clip, analysis) = match request {
            R::CancelJob => {
                if let Some(job) = p.color_job.take() {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                return;
            }
            R::Auto {
                clip,
                style,
                source,
            } => (clip, Analysis::Auto { style, source }),
            R::Match { clip, reference } => (clip, Analysis::Match { reference }),
        };
        if p.color_job.is_some() {
            self.say("Another analysis is running; wait for it or cancel it");
            return;
        }
        self.color_jobs += 1;
        let planned = color::plan(
            p.doc(),
            &p.media_paths,
            &p.offline,
            clip,
            analysis,
            p.playhead,
            self.color_jobs,
        );
        let request = match planned {
            Ok(r) => r,
            Err(e) => {
                self.say(e);
                return;
            }
        };
        p.color_job = Some(crate::session::ColorJob {
            id: request.id,
            clip,
            auto: request.analysis.is_auto(),
            cancel: Arc::clone(&request.cancel),
        });
        let luts: Arc<dyn freecut_render::color::LutSource> = self.luts.clone();
        self.jobs
            .spawn(move || Done::ColorAnalysed(Box::new(crate::jobs::color::run(request, luts))));
    }

    /// A clip picked on the timeline in Match's pick mode (CL-020): the
    /// frame under the pointer becomes the reference.
    fn match_picked(&mut self, reference: ClipId, at: Tick) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let Some(pick) = p.match_pick.take() else {
            return;
        };
        let Some(content) = p
            .doc()
            .clip(reference)
            .and_then(|c| c.content_time_at(at.clamp(c.start(), c.end())))
        else {
            return;
        };
        self.color_request(ui::inspector::ColorRequest::Match {
            clip: pick.target,
            reference: freecut_timeline::model::MatchRef::Clip {
                clip: reference,
                at: content,
                graded: pick.graded,
            },
        });
    }

    /// An analysis finished: committed as one undo entry unless it was
    /// cancelled or the clip changed meanwhile (CL-019, CL-020).
    fn color_analysed(&mut self, finished: crate::jobs::color::Finished) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        // A cancelled or superseded job (another project, an older run).
        if p.color_job.as_ref().is_none_or(|j| j.id != finished.id) {
            return;
        }
        p.color_job = None;
        let request = match crate::jobs::color::edit_for(p.doc(), &finished) {
            Ok(r) => r,
            Err(e) => {
                self.say(e);
                return;
            }
        };
        let said = match &finished.result {
            Ok(crate::jobs::color::Outcome::Match { limited: true, .. }) => {
                "Matched. The reference is very different, so the match is limited"
            }
            Ok(crate::jobs::color::Outcome::Match { .. }) => "Matched",
            _ => "Auto color applied under your sliders",
        };
        match p.apply(&request, Coalesce::Never) {
            Ok(_) => self.say(said),
            Err(e) => self.say(e),
        }
    }

    /// Tells the user something in a toast (UI-040).
    fn say(&mut self, message: impl Into<String>) {
        self.toasts.push(ToastKind::Info, message);
    }

    /// A toast of `kind`, with up to two buttons.
    fn say_as(
        &mut self,
        kind: ToastKind,
        message: impl Into<String>,
        actions: Vec<(String, ToastAction)>,
    ) {
        self.toasts.push_with(kind, message, actions);
    }

    /// A toast's button was pressed.
    fn toast_action(&mut self, action: ToastAction) {
        match action {
            ToastAction::ShowExport => {
                if let Some(dialog) = &mut self.export {
                    dialog.visible = true;
                }
            }
            ToastAction::RetrySave => self.save(false),
            ToastAction::Reveal(path) => {
                self.jobs.spawn(move || {
                    Done::OsOpened(open_path(&path, crate::jobs::platform::OpenAction::Reveal))
                });
            }
            ToastAction::PlaceTake(path) => self.place_loose_take(&path),
            ToastAction::ShowTakes => self.open_recorder(),
        }
    }

    /// The open project was opened or saved: it goes to the top of the
    /// recent projects (PM-017).
    fn remember_project(&mut self) {
        let Some(p) = &self.project else { return };
        let ms = duration(p).raw().max(0) as u128 * 1000
            / freecut_timeline::time::TICKS_PER_SECOND.unsigned_abs() as u128;
        let ms = u64::try_from(ms).unwrap_or(u64::MAX);
        if let Some(entry) = RecentEntry::of(p, ms, crate::recent::unix_now()) {
            self.recent.touch(entry, Instant::now());
        }
    }

    /// Writes the frame on the Stage as the cover of `path`, on the pool.
    fn write_cover(&mut self, path: PathBuf) {
        let (Some(dir), Some(frame)) = (self.covers_dir.clone(), self.cover_frame.clone()) else {
            return;
        };
        self.jobs.spawn(move || {
            let result = crate::recent::write_cover(&dir, &path, &frame).map_err(|e| e.to_string());
            Done::CoverWritten(path, result)
        });
    }

    /// Reads the covers of the recent projects Home shows, on the pool.
    fn request_covers(&mut self) {
        let Some(dir) = self.covers_dir.clone() else {
            return;
        };
        let wanted: Vec<PathBuf> = self
            .recent
            .entries()
            .iter()
            .filter(|e| !self.covers_requested.contains(&e.path))
            .map(|e| e.path.clone())
            .collect();
        for path in wanted {
            self.covers_requested.insert(path.clone());
            let dir = dir.clone();
            self.jobs.spawn(move || {
                let cover = crate::recent::read_cover(&dir, &path);
                Done::Cover(path, cover)
            });
        }
    }

    /// A cover arrived: Home shows it.
    fn cover_arrived(&mut self, ctx: &egui::Context, path: PathBuf, cover: (u32, u32, Vec<u8>)) {
        let (w, h, rgba) = cover;
        let color = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        let tex = ctx.load_texture(
            format!("cover-{}", path.display()),
            color,
            TextureOptions::LINEAR,
        );
        self.covers.insert(path, tex);
    }

    // ---------------------------------------------------------------- jobs

    fn handle_jobs(&mut self, ctx: &egui::Context) {
        // HDR items whose posters arrived now (CL-002), told in one line.
        let mut hdr_found = Vec::new();
        for done in self.jobs.poll() {
            match done {
                Done::LutLibrary(Ok(library)) => {
                    self.luts.set_library(library);
                    self.missing_luts.clear();
                    self.refresh_luts();
                }
                Done::LutLibrary(Err(e)) => {
                    self.say(format!("The LUT library couldn't be read: {e}"));
                }
                Done::Looks(Ok(library)) => {
                    self.looks = Arc::new(crate::looks::SavedLooks::of(&library));
                }
                Done::Looks(Err(e)) => {
                    self.looks = Arc::new(crate::looks::SavedLooks::none());
                    self.say(format!("Your saved looks couldn't be read: {e}"));
                }
                Done::LookDeleted(Ok(library)) => {
                    self.looks = Arc::new(crate::looks::SavedLooks::of(&library));
                    self.say("Look deleted");
                }
                Done::LookDeleted(Err(e)) => {
                    self.say_as(
                        ToastKind::Error,
                        format!("The look couldn't be deleted: {e}"),
                        Vec::new(),
                    );
                }
                Done::LookSaved(Ok((library, name))) => {
                    self.looks = Arc::new(crate::looks::SavedLooks::of(&library));
                    self.assets.tab = ui::assets::AssetTab::Adjust;
                    self.assets.adjust.filter = ui::adjust::AdjustFilter::Saved;
                    self.say(format!("Saved \u{201c}{name}\u{201d} to Saved looks"));
                }
                Done::LookSaved(Err(e)) => {
                    self.say_as(
                        ToastKind::Error,
                        format!("The look couldn't be saved: {e}"),
                        Vec::new(),
                    );
                }
                Done::ColorAnalysed(finished) => self.color_analysed(*finished),
                Done::LutsImported(report) => {
                    let summary = report.summary();
                    if let Some(library) = report.library {
                        self.luts.set_library(library);
                        self.missing_luts.clear();
                        self.refresh_luts();
                    }
                    self.say(summary);
                }
                Done::Probed(result, ticket) => {
                    if let Some(e) = self.imports.finished(ticket, result) {
                        self.say(e);
                    }
                }
                Done::Opened(result) => {
                    self.opening = false;
                    match *result {
                        Ok(loaded) => {
                            let mut project = Project::from_loaded(loaded);
                            let offline = project.offline.len();
                            let upgraded = project.upgraded_from.is_some();
                            let mut messages = project.take_notes();
                            self.install(project);
                            self.remember_project();
                            if upgraded {
                                messages.insert(
                                    0,
                                    "Project upgraded from an older format. The original is \
                                     kept as a backup when you save"
                                        .to_owned(),
                                );
                            }
                            if offline > 0 {
                                messages.insert(
                                    0,
                                    format!(
                                        "{offline} media file(s) can't be found and show as \
                                         offline"
                                    ),
                                );
                            }
                            if !messages.is_empty() {
                                self.say(messages.join(". "));
                            }
                        }
                        Err(e) => self.say_as(ToastKind::Error, e, Vec::new()),
                    }
                }
                Done::Saved(project, job) => self.save_finished(ctx, project, &job),
                Done::Poster(poster) => hdr_found.extend(self.poster_arrived(ctx, poster)),
                Done::Destination(key, dest) => {
                    if let Some(dialog) = &mut self.export {
                        dialog.destination_checked(key, dest);
                    }
                }
                Done::Encoder(available) => {
                    if let Some(dialog) = &mut self.export {
                        dialog.encoder = Some(available);
                    }
                }
                Done::OsOpened(Err(e)) => self.say(e),
                Done::OsOpened(Ok(())) => {}
                Done::Settings(loaded, path) => {
                    self.settings_loaded(*loaded, path);
                    self.sound
                        .start(self.settings.settings().playback.engine_config());
                }
                Done::Workspace(raw, path) => self.workspace_loaded(raw, path),
                Done::Recent(raw, entries, path) => {
                    self.recent.loaded(raw, entries, path, Instant::now());
                }
                Done::Cover(path, Some(cover)) => self.cover_arrived(ctx, path, cover),
                Done::Cover(_, None) => {}
                Done::CoverWritten(path, result) => {
                    if let Err(e) = result {
                        eprintln!("FreeCut: can't keep the project's cover: {e}");
                    }
                    // Home reads the new cover the next time it shows.
                    self.covers.remove(&path);
                    self.covers_requested.remove(&path);
                }
                Done::TakeProbed(serial, placement, result) => {
                    if serial == self.project_serial {
                        self.place_take(placement, result);
                    }
                }
                Done::TakeRemoved(Err(e)) => self.say(e),
                Done::TakeRemoved(Ok(())) => {}
                Done::Recovered(serial, takes, failed) => {
                    if serial == self.project_serial {
                        self.takes_recovered(takes, &failed);
                    }
                }
            }
        }
        if let Some(notice) = hdr_notice(&hdr_found) {
            self.say(notice);
        }
        // A whole import batch becomes one undo step once every file probed,
        // and waits for a slider or field drag to end.
        let busy = self.project.as_ref().is_some_and(Project::gesture_open);
        if let Some(batch) = self.imports.take_ready(busy) {
            let files = batch.media;
            let (audio, skipped) = sound_only(batch.audio);
            let mut messages = Vec::new();
            if let Some(p) = &mut self.project {
                if !files.is_empty() {
                    match p.import(files) {
                        Ok(ids) => messages.push(import_report(ids.len(), 0)),
                        Err(e) => messages.push(e),
                    }
                }
                if !audio.is_empty() || skipped > 0 {
                    let count = audio.len();
                    match p.import_audio(audio) {
                        Ok(listed) => {
                            messages.push(import_report(count, skipped));
                            if let Some(first) = listed.first() {
                                self.assets.reveal_audio(p.doc(), *first, None);
                            }
                        }
                        Err(e) => messages.push(e),
                    }
                }
                // AU-038: files with several audio streams.
                messages.extend(p.take_notes());
            }
            if !messages.is_empty() {
                self.say(messages.join(". "));
            }
        }
        self.request_posters();
        self.waves.poll();
        self.films.poll(ctx);
        if let Some(p) = &self.project {
            self.waves.request_missing(p);
        }
        if let Some(message) = self.sound.poll() {
            self.say(message);
        }
    }

    /// The settings file was read (UI-018). A preference changed in
    /// Preferences before that wins over the file; the file's other values
    /// stay.
    fn settings_loaded(&mut self, loaded: settings::Loaded, path: PathBuf) {
        let early = self.settings.settings().clone();
        let mut store = SettingsStore::new(loaded, Some(path));
        let merged = early.changes_onto(&settings::Settings::default(), store.settings());
        store.set(merged, Instant::now());
        self.settings = store;
        self.timeline.skim = self.settings.settings().timeline.skim;
    }

    /// A save finished. One started for a project that is no longer open
    /// (another was opened while it ran) only says how it went: its file,
    /// save point and failure belong to that project, not to this one, and
    /// a Retry would save this one over it.
    fn save_finished(&mut self, ctx: &egui::Context, project: u64, job: &SavedJob) {
        let name = job
            .target
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        if project != self.project_serial {
            match &job.result {
                Ok(()) => self.say_as(ToastKind::Success, format!("Saved {name}"), Vec::new()),
                Err(e) => self.say_as(
                    ToastKind::Error,
                    format!("Couldn't save {name}: {e}"),
                    Vec::new(),
                ),
            }
            return;
        }
        if let Some(p) = &mut self.project {
            p.saved(job);
            self.save_failed = job.result.as_ref().err().cloned();
        }
        match &job.result {
            Ok(()) => {
                self.say_as(ToastKind::Success, format!("Saved {name}"), Vec::new());
                if self.project.as_ref().and_then(|p| p.file.as_ref()) == Some(&job.target) {
                    self.remember_project();
                    self.write_cover(job.target.clone());
                }
            }
            Err(e) => self.say_as(
                ToastKind::Error,
                format!("Couldn't save: {e}"),
                vec![("Retry".to_owned(), ToastAction::RetrySave)],
            ),
        }
        if job.result.is_ok() && matches!(self.pending, Some(Pending::Quit)) {
            self.allow_close = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// Puts a finished poster on its tile, and returns its item's name and
    /// transfer when that makes the HDR notice due (CL-002). A poster made
    /// for a project that is no longer open is dropped: its id may name
    /// another item of this one.
    fn poster_arrived(
        &mut self,
        ctx: &egui::Context,
        poster: Poster,
    ) -> Option<(String, HdrTransfer)> {
        if poster.project != self.project_serial {
            return None;
        }
        let (id, image) = (poster.id, poster.image.ok()?);
        let color = ColorImage::from_rgba_unmultiplied(
            [image.width as usize, image.height as usize],
            &image.rgba,
        );
        let tex = ctx.load_texture(
            format!("poster-{}", id.get()),
            color,
            TextureOptions::LINEAR,
        );
        self.posters.insert(id, tex);
        let item = self.project.as_ref()?.doc().library().get(id)?;
        let tf = image.hdr?;
        self.hdr
            .due(id, image.hdr)
            .then(|| (ui::timeline::media_name(item), tf))
    }

    fn request_posters(&mut self) {
        let Some(p) = &self.project else { return };
        let wanted: Vec<(MediaId, PathBuf)> = p
            .doc()
            .library()
            .items()
            .filter(|m| {
                (m.info.has_video() || m.info.is_still()) && !self.posters_requested.contains(&m.id)
            })
            .filter(|m| !p.offline.contains(&m.id))
            .filter_map(|m| p.media_paths.get(&m.id).map(|path| (m.id, path.clone())))
            .collect();
        let project = self.project_serial;
        for (id, path) in wanted {
            self.posters_requested.insert(id);
            self.jobs.spawn(move || {
                Done::Poster(Poster {
                    project,
                    id,
                    image: freecut_media::poster(&path, 256).map_err(|e| e.to_string()),
                })
            });
        }
    }

    fn install(&mut self, project: Project) {
        self.stop_playback();
        // A take of the old project is cancelled and kept on disk.
        self.recorder.project_changed();
        self.restore_take_mute();
        self.sound.reset();
        self.waves.clear();
        self.films.clear();
        self.project_serial += 1;
        self.project = Some(project);
        self.posters.clear();
        self.posters_requested.clear();
        self.hdr.reset();
        self.preview_key = None;
        self.preview_tex = None;
        self.preview_layers.clear();
        self.preview_shown = self.preview_generation;
        // Hover skim is a per-user preference: it outlives the project.
        let skim = self.timeline.skim;
        self.timeline = ui::timeline::TimelineView::default();
        self.timeline.skim = skim;
        self.skim_shown = None;
        self.assets.reset();
        self.inspector.reset();
        self.master_drag.reset();
        self.mixer.reset();
        self.player.reset();
        // Probes still running were started for the old project.
        self.imports.reset();
        self.restore_confirm = None;
        self.save_failed = None;
        // A running export keeps its own snapshot and carries on, but its
        // settings are not reused for the new project; an idle dialog
        // belonged to the old project.
        match &mut self.export {
            Some(dialog) if dialog.is_running() => dialog.project_closed(),
            _ => self.export = None,
        }
        self.scan_interrupted_takes();
    }

    // -------------------------------------------------------------- export

    /// File › Export…, ⌘E and the Export button (EX-021).
    fn open_export(&mut self) {
        let Some(p) = &self.project else { return };
        let folder = self
            .export_folder
            .clone()
            .or_else(|| default_export_folder(Os::current(), home_dir()));
        match &mut self.export {
            Some(dialog) => dialog.open(p, folder),
            None => self.export = Some(ExportDialog::new(p, folder)),
        }
    }

    /// Export pressed (EX-021): the settings are stored in the project
    /// (modified, no undo entry), playback pauses, and the job starts on a
    /// snapshot of the document (EX-022). Editing continues meanwhile.
    /// `slates` is EX-022's "Retry using slates for unreadable frames".
    fn start_export(&mut self, ctx: &egui::Context, slates: bool) {
        self.stop_playback();
        let (Some(p), Some(dialog)) = (&mut self.project, &mut self.export) else {
            return;
        };
        if dialog.is_running() || !dialog.settings_current() {
            return;
        }
        let settings = dialog.settings.clone();
        if p.export.as_ref() != Some(&settings) {
            p.export = Some(settings.clone());
            p.editor.mark_unsaved();
        }
        self.export_folder.clone_from(&settings.file.folder);
        let request = ExportRequest {
            doc: Arc::new(p.doc().clone()),
            seq: p.doc().root(),
            media_paths: p.media_paths.clone(),
            offline: p.offline.clone(),
            settings,
            conflict: dialog.conflict,
            fonts: self.fonts.clone(),
            presets: Arc::clone(&self.presets),
            // AU-012: export ignores solo unless "Export as heard".
            solo: dialog.export_solo(p),
            luts: self.luts.clone(),
            slates_for_unreadable: slates,
        };
        let ctx = ctx.clone();
        dialog.started(ExportJob::start(request, move || ctx.request_repaint()));
    }

    fn export_actions(&mut self, ctx: &egui::Context, actions: Vec<ExportAction>) {
        for action in actions {
            match action {
                ExportAction::Start => self.start_export(ctx, false),
                ExportAction::StartWithSlates => self.start_export(ctx, true),
                ExportAction::ChooseFolder => {
                    let Some(dialog) = &mut self.export else {
                        continue;
                    };
                    let mut picker = rfd::FileDialog::new().set_title("Choose the export folder");
                    if let Some(folder) = &dialog.settings.file.folder {
                        picker = picker.set_directory(folder);
                    }
                    if let Some(folder) = picker.pick_folder() {
                        dialog.set_folder(folder);
                    }
                }
                ExportAction::Probe { folder, name } => {
                    let container = self
                        .export
                        .as_ref()
                        .map(|d| d.settings.video.container)
                        .unwrap_or_default();
                    self.jobs.spawn(move || {
                        let dest = probe_destination(&folder, &name, container);
                        Done::Destination((folder, name), dest)
                    });
                }
                ExportAction::ProbeEncoder => {
                    self.jobs.spawn(|| Done::Encoder(encoder_available()));
                }
                ExportAction::Os(action, path) => {
                    self.jobs
                        .spawn(move || Done::OsOpened(open_path(&path, action)));
                }
                ExportAction::Select(clips) => {
                    if let Some(p) = &mut self.project {
                        p.selection = clips;
                    }
                }
                ExportAction::Seek(at) => {
                    self.stop_playback();
                    if let Some(p) = &mut self.project {
                        p.playhead = at;
                    }
                    self.seeked();
                }
            }
        }
    }

    /// Draws the Export dialog, when shown, and carries out what it asks.
    fn export_dialog(&mut self, ctx: &egui::Context) {
        if let (Some(dialog), Some(p)) = (&mut self.export, &self.project) {
            let actions = dialog.show(ctx, p, &self.fonts, self.preview_tex.as_ref());
            self.export_actions(ctx, actions);
        }
    }

    /// Polls the running export and reports how it ended.
    fn poll_export(&mut self) {
        let Some(end) = self.export.as_mut().and_then(ExportDialog::poll) else {
            return;
        };
        match end {
            ExportEnd::Done(result) => {
                let name = result
                    .path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.say_as(
                    ToastKind::Success,
                    format!("Export finished: {name}"),
                    vec![(
                        "Show in folder".to_owned(),
                        ToastAction::Reveal(result.path.clone()),
                    )],
                );
            }
            ExportEnd::Failed(failure) => self.say_as(
                ToastKind::Error,
                format!("Export failed: {}", failure.message()),
                vec![("Details".to_owned(), ToastAction::ShowExport)],
            ),
            ExportEnd::Cancelled => self.say("Export cancelled"),
        }
    }

    /// Quitting while an export runs asks first (EX-022).
    fn quit_export_dialog(&mut self, ctx: &egui::Context) {
        if !self.quit_during_export {
            return;
        }
        if !self.export.as_ref().is_some_and(ExportDialog::is_running) {
            self.quit_during_export = false;
            return;
        }
        let mut choice = None;
        let sheet = Sheet::new(Id::new("quit-export"), "An export is running")
            .width(440.0)
            .show(ctx, |s| {
                s.body(|ui| {
                    body_text(ui, "Quitting stops it, and the unfinished file is deleted.");
                });
                s.footer(|ui| {
                    if sheet_button(ui, ButtonKind::Primary, "Keep exporting").clicked() {
                        choice = Some(false);
                    }
                    if sheet_button(ui, ButtonKind::Secondary, "Stop export and quit").clicked() {
                        choice = Some(true);
                    }
                });
            });
        if sheet.close && choice.is_none() {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                self.quit_during_export = false;
                // Dropping the dialog drops the job: it cancels and waits,
                // so no partial file stays behind.
                self.export = None;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Some(false) => self.quit_during_export = false,
            None => {}
        }
    }

    // ------------------------------------------------------------- actions

    fn import_paths(&mut self, paths: Vec<PathBuf>, kind: ImportRequest) {
        let Some(p) = &self.project else { return };
        let base = p.relative_base.clone();
        for path in paths {
            let base = base.clone();
            let ticket = self.imports.start(kind);
            self.jobs
                .spawn(move || Done::Probed(probe_for_import(&path, base.as_deref()), ticket));
        }
    }

    fn import_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Import media")
            .add_filter("Media", MEDIA_EXTENSIONS)
            .pick_files()
        {
            self.import_paths(paths, ImportRequest::Media);
        }
    }

    /// The Audio panel's "Import audio…": files without sound are skipped
    /// once probed (AU-001).
    fn import_audio_dialog(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Import audio")
            .add_filter("Audio, or video with sound", AUDIO_EXTENSIONS)
            .pick_files()
        {
            self.import_paths(paths, ImportRequest::Audio);
        }
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open project")
            .add_filter("FreeCut project", &["freecut"])
            .pick_file()
        {
            self.guarded(Pending::Open(path));
        }
    }

    fn start_open(&mut self, path: PathBuf) {
        self.opening = true;
        self.jobs.spawn(move || {
            Done::Opened(Box::new(open_project(&path).map_err(|e| format!("{e:#}"))))
        });
    }

    /// Commits what is typed in the content field (TX-002), so that closing
    /// or replacing the project sees it as an unsaved edit.
    fn commit_text(&mut self) {
        let Some(p) = &mut self.project else { return };
        if let Some(e) = ui::props::finish_content(p, &mut self.inspector.props) {
            self.say(e);
        }
    }

    /// Whether closing the project would lose work: edits not saved, or
    /// typing that couldn't be committed yet (a drag is still open).
    fn unsaved(&self) -> bool {
        self.project.as_ref().is_some_and(|p| p.editor.is_dirty())
            || self.inspector.props.has_uncommitted_text()
    }

    /// Runs `action` now, or asks first if it would discard unsaved work.
    fn guarded(&mut self, action: Pending) {
        self.commit_text();
        if self.unsaved() {
            self.pending = Some(action);
        } else {
            self.run_pending(action);
        }
    }

    fn run_pending(&mut self, action: Pending) {
        match action {
            Pending::New => self.new_dialog = Some(NewProjectDialog::default()),
            Pending::Open(path) => self.start_open(path),
            Pending::Quit => {}
        }
    }

    fn save(&mut self, save_as: bool) {
        let Some(p) = &mut self.project else { return };
        if p.saving {
            return;
        }
        let target = match (&p.file, save_as) {
            (Some(file), false) => Some(file.clone()),
            _ => rfd::FileDialog::new()
                .set_title("Save project")
                .add_filter("FreeCut project", &["freecut"])
                .set_file_name("Untitled.freecut")
                .save_file()
                .map(|mut path| {
                    if path.extension().is_none() {
                        path.set_extension("freecut");
                    }
                    path
                }),
        };
        let Some(target) = target else { return };
        let job = p.save_job(target);
        p.saving = true;
        let serial = self.project_serial;
        self.jobs.spawn(move || Done::Saved(serial, job.run()));
    }

    /// *Split at playhead* (TL-023): the selection, or with nothing
    /// selected the main-track clip under the playhead (and its linked
    /// partners, TL-057).
    fn split(&mut self) {
        self.split_with(false);
    }

    /// *Split all* (`Mod+Shift+B`, TL-023): every clip under the playhead
    /// on unlocked tracks.
    fn split_all(&mut self) {
        self.split_with(true);
    }

    fn split_with(&mut self, all: bool) {
        let linked = self.timeline.linked_selection;
        let Some(p) = &mut self.project else { return };
        if let Err(e) = p.split_at_playhead(all, linked) {
            self.say(e);
        }
    }

    /// Cut away the part before (`Head`, Q) or after (`Tail`, W) the
    /// playhead (TL-026). During playback it acts at the frame being shown
    /// and playback goes on from wherever the playhead lands.
    fn cut_away(&mut self, cut: Cut, ripple_others: bool) {
        let linked = self.timeline.linked_selection;
        let Some(p) = &mut self.project else { return };
        match p.cut_away(cut, ripple_others, linked) {
            Ok(true) => self.seeked(),
            Ok(false) => {}
            Err(e) => self.say(e),
        }
    }

    /// `M` / `Alt+M` (TL-037): a marker at the playhead, Blue or in the
    /// colour after the last one used.
    fn add_marker(&mut self, next_color: bool) {
        let Some(p) = &mut self.project else { return };
        let color = next_color
            .then(|| crate::timeline_commands::next_marker_color(self.timeline.last_marker_color));
        match p.add_marker(color) {
            Ok(_) => self.timeline.last_marker_color = Some(color.unwrap_or_default()),
            Err(e) => self.say(e),
        }
    }

    /// `Shift+M` / `Ctrl+Shift+M` (TL-037): the next or previous marker.
    fn jump_to_marker(&mut self, forward: bool) {
        let Some(p) = &mut self.project else { return };
        match p.jump_to_marker(forward) {
            Ok(()) => self.seeked(),
            Err(e) => self.say(e),
        }
    }

    /// `Alt+N` (TL-029): *Follow main track* on or off, one undo step.
    fn toggle_follow_main(&mut self) {
        let Some(p) = &mut self.project else { return };
        if let Err(e) = p.toggle_follow_main() {
            self.say(e);
        }
    }

    /// `Shift+N` (TL-005): the magnetic main track, one undo step.
    fn toggle_magnet(&mut self) {
        let Some(p) = &mut self.project else { return };
        let req = EditRequest::SetMagnet {
            seq: p.doc().root(),
            on: !p.flags().main_magnet,
        };
        if let Err(e) = p.apply(&req, Coalesce::Never) {
            self.say(e);
        }
    }

    /// Delete from the keyboard or the Edit menu: the selected volume
    /// keyframes when there are any (AU-008), else the selected markers
    /// and clips together, as one undo entry (TL-025, TL-037).
    fn delete(&mut self, ripple: bool) {
        let Some(p) = &mut self.project else { return };
        if !ripple && let Some(keys) = self.timeline.selected_keys(p) {
            self.timeline.clear_keys();
            if let Err(e) = clip_keys::remove_keys(p, keys.clip, &keys.times) {
                self.say(e);
            }
            return;
        }
        self.delete_clips(ripple);
    }

    fn delete_clips(&mut self, ripple: bool) {
        let Some(p) = &mut self.project else { return };
        if p.selection.is_empty() && p.markers.is_empty() {
            return;
        }
        // Markers and clips go in one entry.
        let gesture = p.begin_gesture();
        let mut result = p
            .delete_selected_markers(Coalesce::Gesture(gesture))
            .map(|_| ());
        if result.is_ok() && !p.selection.is_empty() {
            let req = EditRequest::Delete(DeleteReq {
                seq: p.doc().root(),
                clips: p.selection.clone(),
                ripple,
            });
            result = p.apply(&req, Coalesce::Gesture(gesture)).map(|_| ());
        }
        p.end_gesture(gesture);
        if let Err(e) = result {
            self.say(e);
        }
    }

    /// Carries out a clip command from a context menu or the inspector.
    fn run_command(&mut self, command: ClipCommand) {
        match command {
            ClipCommand::Split => self.split(),
            ClipCommand::Delete => self.delete_clips(false),
            ClipCommand::SetMuted(muted) => self.set_muted(muted),
            ClipCommand::Detach => self.detach_audio(),
            ClipCommand::Restore => self.restore_audio(),
            ClipCommand::SetStream(sel) => {
                let Some(p) = &mut self.project else { return };
                let clips = p.selection.clone();
                if let Err(e) = clip_streams::set_stream(p, &clips, sel) {
                    self.say(e);
                }
            }
            ClipCommand::AddStream(ordinal) => {
                let Some(p) = &mut self.project else { return };
                let clips = p.selection.clone();
                match clip_streams::add_stream_as_clips(p, &clips, ordinal) {
                    Ok(1) => self.say("Stream added as an audio clip"),
                    Ok(n) => self.say(format!("Stream added as {n} audio clips")),
                    Err(e) => self.say(e),
                }
            }
            ClipCommand::Resync => {
                let Some(p) = &mut self.project else { return };
                let clips = p.selection.clone();
                if let Err(e) = clip_streams::resync(p, &clips) {
                    self.say(e);
                }
            }
        }
    }

    /// Mutes or unmutes every selected clip that plays its own sound
    /// (AU-007), as one undo step.
    fn set_muted(&mut self, muted: bool) {
        let Some(p) = &mut self.project else { return };
        let req = EditRequest::SetClipAudio {
            seq: p.doc().root(),
            clips: p.selection.clone(),
            edit: AudioEdit::Muted(muted),
        };
        if let Err(e) = p.apply(&req, Coalesce::Never) {
            self.say(e);
        }
    }

    /// Moves the sound of the selected video clips to audio clips (TL-030)
    /// and says which track it went to.
    fn detach_audio(&mut self) {
        let Some(p) = &mut self.project else { return };
        let seq = p.doc().root();
        let req = EditRequest::DetachAudio {
            seq,
            clips: p.selection.clone(),
        };
        let message = match p.apply(&req, Coalesce::Never) {
            Ok(Committed::Applied { notices, .. }) => {
                let created = notices.iter().any(|n| matches!(n, Notice::CreatedTrack(_)));
                let doc = p.doc();
                let names: Vec<String> = p
                    .selection
                    .iter()
                    .filter_map(|c| doc.sequence(seq)?.find_clip(*c))
                    .map(|(_, track)| ui::timeline::track_name(doc, seq, track.id))
                    .fold(Vec::new(), |mut acc, n| {
                        if !acc.contains(&n) {
                            acc.push(n);
                        }
                        acc
                    });
                let place = if names.is_empty() {
                    "an audio track".to_owned()
                } else {
                    names.join(", ")
                };
                if created {
                    format!("Sound detached to {place} (new track)")
                } else {
                    format!("Sound detached to {place}")
                }
            }
            Ok(Committed::NoOp) => return,
            Err(e) => e,
        };
        self.say(message);
    }

    /// Restores the sound of the selected detached video clips (TL-030),
    /// asking first when that removes more than a plain, untouched audio
    /// clip.
    fn restore_audio(&mut self) {
        let Some(p) = &self.project else { return };
        let doc = p.doc();
        let seq = doc.root();
        let (removed, ask) = restore_removals(doc, seq, &p.selection);
        if ask {
            let rate = doc.settings().frame_rate;
            let removed = removed
                .iter()
                .filter_map(|id| doc.clip(*id))
                .map(|c| {
                    let at = freecut_timeline::time::Timecode::from_tick(c.start(), rate, false)
                        .map_or_else(|_| String::new(), |tc| format!(" at {tc}"));
                    format!(
                        "{}{at}",
                        ui::timeline::clip_label(doc, &c.content, c.name.as_deref())
                    )
                })
                .collect();
            self.restore_confirm = Some(RestoreConfirm {
                clips: p.selection.clone(),
                removed,
            });
        } else {
            let clips = p.selection.clone();
            self.apply_restore(clips);
        }
    }

    fn apply_restore(&mut self, clips: Vec<ClipId>) {
        let Some(p) = &mut self.project else { return };
        let req = EditRequest::RestoreAudio {
            seq: p.doc().root(),
            clips,
        };
        if let Err(e) = p.apply(&req, Coalesce::Never) {
            self.say(e);
        }
    }

    fn restore_dialog(&mut self, ctx: &egui::Context) {
        let Some(confirm) = &self.restore_confirm else {
            return;
        };
        let mut choice = None;
        let modal = Sheet::new(Id::new("restore-audio"), "Restore audio?")
            .width(460.0)
            .show(ctx, |s| {
                s.body(|ui| {
                    body_text(
                        ui,
                        &format!(
                            "Restoring gives the video its sound back and removes {}:",
                            if confirm.removed.len() == 1 {
                                "this audio clip".to_owned()
                            } else {
                                format!("these {} audio clips", confirm.removed.len())
                            }
                        ),
                    );
                    for name in &confirm.removed {
                        body_text(ui, &format!("• {name}"));
                    }
                });
                s.footer(|ui| {
                    if sheet_button(ui, ButtonKind::Primary, "Restore").clicked() {
                        choice = Some(true);
                    }
                    if sheet_button(ui, ButtonKind::Secondary, "Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
        if modal.close && choice.is_none() {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                if let Some(confirm) = self.restore_confirm.take() {
                    self.apply_restore(confirm.clips);
                }
            }
            Some(false) => self.restore_confirm = None,
            None => {}
        }
    }

    /// Opens the Save look sheet for `request` (CL-026).
    fn open_save_look(&mut self, request: ui::inspector::SaveLookRequest) {
        if self.app_data.is_none() {
            self.say("Looks can't be saved: FreeCut has no app-data folder here");
            return;
        }
        let name = ui::save_look::suggested_name(self.looks.looks.len());
        self.save_look = Some(ui::save_look::SaveLookDialog::new(
            request.stack,
            request.keyed,
            name,
        ));
    }

    /// The Save look sheet; *Save* writes the file on the pool.
    fn save_look_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.save_look else {
            return;
        };
        let Some(answer) = dialog.show(ctx) else {
            return;
        };
        self.save_look = None;
        let ui::save_look::SaveLookAnswer::Save {
            name,
            stack,
            sections,
        } = answer
        else {
            return;
        };
        let Some(data) = self.app_data.clone() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let look = freecut_project::look::LookFile::new(&name, &stack, sections, now.as_secs());
        // The clock varies the file's random id.
        let seed = now.as_nanos() as u64;
        self.jobs
            .spawn(move || Done::LookSaved(crate::looks::save(&data, look, seed)));
    }

    /// Deleting a saved look asks first: FreeCut has no trash of its own,
    /// so the file goes for good (CL-026).
    fn delete_look_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, name)) = self.delete_look.clone() else {
            return;
        };
        let mut choice = None;
        let title = format!("Delete \u{201c}{name}\u{201d}?");
        let modal = Sheet::new(Id::new("delete-look"), &title)
            .width(460.0)
            .show(ctx, |s| {
                s.body(|ui| {
                    body_text(
                        ui,
                        "The look's file is deleted for good: FreeCut has no trash of its \
                         own. Clips you applied it to keep their grade.",
                    );
                });
                s.footer(|ui| {
                    if sheet_button(ui, ButtonKind::Danger, "Delete look").clicked() {
                        choice = Some(true);
                    }
                    if sheet_button(ui, ButtonKind::Secondary, "Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
        if modal.close && choice.is_none() {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                self.delete_look = None;
                if let Some(data) = self.app_data.clone() {
                    self.jobs
                        .spawn(move || Done::LookDeleted(crate::looks::delete(&data, &id)));
                }
            }
            Some(false) => self.delete_look = None,
            None => {}
        }
    }

    /// The Text tab's tile (TX-001): a text clip at the playhead, selected,
    /// with its content field focused and all selected. Playback pauses
    /// first, so the clip lands at the paused time.
    fn add_text(&mut self) {
        self.stop_playback();
        let Some(p) = &mut self.project else { return };
        match ui::text::add_text_at_playhead(p) {
            Ok(clip) => self.inspector.props.edit_content(clip),
            Err(e) => self.say(e),
        }
    }

    fn undo(&mut self) {
        let result = self.project.as_mut().map(Project::undo);
        if let Some(Err(e)) = result {
            self.say(e);
        }
    }

    fn redo(&mut self) {
        let result = self.project.as_mut().map(Project::redo);
        if let Some(Err(e)) = result {
            self.say(e);
        }
    }

    fn toggle_play(&mut self) {
        // AU-027: Space is ignored while a take counts down or records.
        if self.recorder.running() {
            return;
        }
        if self.playback.is_some() {
            self.stop_playback();
            return;
        }
        let Some(p) = &mut self.project else { return };
        let end = duration(p);
        if p.playhead >= end {
            p.playhead = Tick::ZERO;
        }
        if end > Tick::ZERO {
            let from = p.playhead;
            // The engine must have the current document before it starts.
            self.sound.sync(self.project_serial, p);
            self.sound.play(from);
            self.playback = Some(Playback::new(from));
        }
    }

    /// Pauses on the frame showing now (EX-002).
    fn stop_playback(&mut self) {
        if self.playback.take().is_some() {
            self.sound.pause();
        }
    }

    /// The playhead was moved by the user: playback, if running, continues
    /// from there.
    fn seeked(&mut self) {
        let Some(p) = &self.project else { return };
        if self.playback.is_some() {
            self.sound.seek(p.playhead);
            self.playback = Some(Playback::new(p.playhead));
        }
    }

    fn step(&mut self, frames: i64) {
        self.stop_playback();
        let Some(p) = &mut self.project else { return };
        let rate = p.doc().settings().frame_rate;
        let k = rate.frame_index(p.playhead).saturating_add(frames).max(0);
        if let Ok(t) = rate.frame_start(k) {
            p.playhead = t.min(duration(p));
        }
    }

    // -------------------------------------------------------- frame update

    fn advance_playback(&mut self, ctx: &egui::Context) {
        let (Some(play), Some(p)) = (&self.playback, &mut self.project) else {
            return;
        };
        let t = play.time(Instant::now(), self.sound.heard());
        let rate = p.doc().settings().frame_rate;
        let end = duration(p);
        if t >= end {
            p.playhead = end;
            self.stop_playback();
        } else {
            p.playhead = rate
                .round_to_grid(t, freecut_timeline::time::Rounding::Floor)
                .unwrap_or(t);
            ctx.request_repaint_after(Duration::from_millis(5));
        }
    }

    fn update_preview(&mut self, ctx: &egui::Context) {
        if let Some(frame) = self.preview.poll()
            && frame.generation >= self.preview_shown
        {
            self.preview_shown = frame.generation;
            let before = self.missing_luts.len();
            self.missing_luts
                .extend(frame.missing_luts.iter().map(|l| l.hash));
            if self.missing_luts.len() != before {
                self.refresh_luts();
            }
            self.preview_layers = frame.layers;
            self.preview_canvas = frame.canvas;
            let color = ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.rgba,
            );
            // Kept (not copied) for the project's cover at the next save.
            self.cover_frame = Some(CoverFrame {
                width: frame.width,
                height: frame.height,
                rgba: Arc::new(frame.rgba),
            });
            match &mut self.preview_tex {
                Some(tex) => tex.set(color, TextureOptions::LINEAR),
                None => {
                    self.preview_tex =
                        Some(ctx.load_texture("preview", color, TextureOptions::LINEAR))
                }
            }
        }
        let Some(p) = &self.project else { return };
        // TL-014: preview_time = skim.unwrap_or(playhead).
        let time = self.skim_shown.unwrap_or(p.playhead);
        let key = PreviewKey {
            revision: p.editor.revision(),
            time,
            size: self.preview_size,
            offline: p.offline.len(),
            fonts: self.fonts.generation(),
            draft: self.inspector.props.text_draft(),
        };
        if self.preview_key.as_ref() == Some(&key) {
            return;
        }
        self.preview_generation += 1;
        self.preview.request(PreviewRequest {
            doc: Arc::new(p.doc().clone()),
            media_paths: Arc::new(p.media_paths.clone()),
            offline: Arc::new(p.offline.clone()),
            time,
            max_size: self.preview_size,
            text_draft: key.draft.clone(),
            generation: self.preview_generation,
        });
        self.preview_key = Some(key);
    }

    /// A dialog is open: it owns the keyboard, and edits behind it would
    /// change what it is asking about.
    fn modal_open(&self) -> bool {
        self.restore_confirm.is_some()
            || self.save_look.is_some()
            || self.delete_look.is_some()
            || self.pending.is_some()
            || self.new_dialog.is_some()
            || self.show_about
            || self.quit_during_export
            || self.export.as_ref().is_some_and(|d| d.visible)
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if keyboard_taken(ctx) || self.modal_open() {
            return;
        }
        let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let cmd_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if pressed(cmd_shift(Key::Z)) {
            self.redo();
        } else if pressed(cmd(Key::Z)) {
            self.undo();
        }
        if pressed(cmd_shift(Key::S)) {
            self.save(true);
        } else if pressed(cmd(Key::S)) {
            self.save(false);
        }
        if pressed(cmd(Key::O)) {
            self.open_dialog();
        }
        if pressed(cmd(Key::N)) {
            self.guarded(Pending::New);
        }
        if pressed(cmd(Key::Comma)) {
            self.open_preferences(ctx);
        }
        if self.project.is_none() {
            return;
        }
        if pressed(cmd(Key::I)) {
            self.import_dialog();
        }
        if pressed(cmd(Key::E)) {
            self.open_export();
        }
        // UI-031: `Mod+B` splits (plain `S` belongs to hover skim, TL-014);
        // `Mod+Shift+B` splits every track. Shift first: a shortcut match
        // ignores extra modifiers.
        if pressed(cmd_shift(Key::B)) {
            self.split_all();
        } else if pressed(cmd(Key::B)) {
            self.split();
        }
        if pressed(cmd_shift(Key::M)) {
            self.jump_to_marker(false);
        }
        let key = |k: Key, m: Modifiers| ctx.input_mut(|i| i.consume_key(m, k));
        // Tools (TL-024).
        for (k, tool) in [
            (Key::A, ui::timeline_tools::TimelineTool::Select),
            (Key::B, ui::timeline_tools::TimelineTool::Blade),
            (Key::H, ui::timeline_tools::TimelineTool::Hand),
        ] {
            if key(k, Modifiers::NONE) {
                self.timeline.tool.set(tool);
            }
        }
        // Cut away before / after the playhead (TL-026); Shift ripples
        // other tracks.
        for (k, cut) in [(Key::Q, Cut::Head), (Key::W, Cut::Tail)] {
            if key(k, Modifiers::SHIFT) {
                self.cut_away(cut, true);
            } else if key(k, Modifiers::NONE) {
                self.cut_away(cut, false);
            }
        }
        // Markers (TL-037).
        if key(Key::M, Modifiers::SHIFT) {
            self.jump_to_marker(true);
        } else if key(Key::M, Modifiers::ALT) {
            self.add_marker(true);
        } else if key(Key::M, Modifiers::NONE) {
            self.add_marker(false);
        }
        // Hover skim (TL-014): Off ↔ the last on-mode.
        if key(Key::S, Modifiers::NONE) {
            self.timeline.skim.toggle();
        }
        // The recorder (AU-027): Esc cancels a take (or closes the panel),
        // Enter starts or stops one while the panel is open.
        if (self.recorder.open || self.recorder.running()) && key(Key::Escape, Modifiers::NONE) {
            self.recorder.cancel();
        }
        if self.recorder.open && key(Key::Enter, Modifiers::NONE) {
            if self.recorder.running() {
                self.recorder.stop();
            } else {
                self.start_take(None);
            }
        }
        // Snapping, the magnet and Follow main track (TL-028, TL-005, TL-029).
        if key(Key::N, Modifiers::SHIFT) {
            self.toggle_magnet();
        } else if key(Key::N, Modifiers::ALT) {
            self.toggle_follow_main();
        } else if key(Key::N, Modifiers::NONE) {
            self.timeline.snapping = !self.timeline.snapping;
        }
        if key(Key::Space, Modifiers::NONE) {
            self.toggle_play();
        }
        if key(Key::Delete, Modifiers::SHIFT) || key(Key::Backspace, Modifiers::SHIFT) {
            self.delete(true);
        } else if key(Key::Delete, Modifiers::NONE) || key(Key::Backspace, Modifiers::NONE) {
            self.delete(false);
        }
        if key(Key::ArrowLeft, Modifiers::SHIFT) {
            self.step(-10);
        } else if key(Key::ArrowLeft, Modifiers::NONE) {
            self.step(-1);
        }
        if key(Key::ArrowRight, Modifiers::SHIFT) {
            self.step(10);
        } else if key(Key::ArrowRight, Modifiers::NONE) {
            self.step(1);
        }
        if key(Key::Home, Modifiers::NONE) {
            self.step(i64::MIN / 4);
        }
        if key(Key::End, Modifiers::NONE) {
            self.stop_playback();
            if let Some(p) = &mut self.project {
                p.playhead = duration(p);
            }
        }
        if key(Key::Plus, Modifiers::NONE) || key(Key::Equals, Modifiers::NONE) {
            self.timeline.px_per_sec = (self.timeline.px_per_sec * 1.25).min(2_000.0);
        }
        if key(Key::Minus, Modifiers::NONE) {
            self.timeline.px_per_sec = (self.timeline.px_per_sec / 1.25).max(2.0);
        }
    }

    // --------------------------------------------------------- preferences

    /// ⌘, / Ctrl+, , File › Preferences… and the banner's "Choose device…"
    /// (UI-018): shows the window, listing the output devices again.
    fn open_preferences(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.prefs.show_window(move || ctx.request_repaint());
    }

    /// Draws Preferences when open; a change applies at once and is saved
    /// shortly after (UI-018).
    fn preferences_window(&mut self, ctx: &egui::Context) {
        let output = self.sound.output_state();
        if let Some(next) = ui::preferences::show(
            ctx,
            &mut self.prefs,
            self.settings.settings(),
            &output,
            self.settings.memory_only(),
        ) {
            self.sound.configure(next.playback.engine_config());
            self.settings.set(next, Instant::now());
        }
    }

    /// The workspace file was read (UI-002): the inspector takes its state,
    /// unless it was changed meanwhile, and changes are saved from now on.
    fn workspace_loaded(
        &mut self,
        raw: Option<serde_json::Map<String, serde_json::Value>>,
        path: PathBuf,
    ) {
        let store = WorkspaceStore::new(raw, path);
        // Cards only looked at are still in their first state.
        let untouched = ui::inspector::lens::Workspace::default().to_json();
        if self.inspector.workspace.to_json() == untouched
            && let Some(saved) = store.get(workspace::INSPECTOR)
        {
            self.inspector.workspace = ui::inspector::lens::Workspace::from_json(saved);
        }
        self.workspace = store;
        self.workspace_seen = None;
    }

    /// Writes changed workspace state a second after it settles (UI-002).
    fn save_workspace(&mut self, ctx: &egui::Context) {
        if self.workspace_seen.as_ref() != Some(&self.inspector.workspace) {
            let seen = self.inspector.workspace.clone();
            self.workspace
                .set(workspace::INSPECTOR, seen.to_json(), Instant::now());
            self.workspace_seen = Some(seen);
        }
        let wake = ctx.clone();
        if let Some(at) = self
            .workspace
            .tick(Instant::now(), move || wake.request_repaint())
        {
            ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
        }
    }

    /// Writes changed preferences once they settle, and reports problems.
    fn save_settings(&mut self, ctx: &egui::Context) {
        let wake = ctx.clone();
        if let Some(at) = self
            .settings
            .tick(Instant::now(), move || wake.request_repaint())
        {
            ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
        }
        let notices = self.settings.take_notices();
        if !notices.is_empty() {
            self.say_as(ToastKind::Warning, notices.join(". "), Vec::new());
        }
    }

    /// The sound banner (AU-010): no device, a lost device, or a chosen
    /// device that isn't the one playing, with "Choose device…".
    fn sound_banner(&mut self, ui: &mut egui::Ui) {
        let Some(banner) = crate::sound::banner(&self.sound.output_state()) else {
            self.banner_dismissed = None;
            return;
        };
        if self.banner_dismissed.as_deref() == Some(banner.text.as_str()) {
            return;
        }
        let mut choose = false;
        let mut dismiss = false;
        let look = crate::theme::look(ui.ctx());
        let gutter = look.metrics.gutter;
        let ground = egui::Frame::NONE
            .fill(look.palette.bg_app)
            .inner_margin(egui::Margin {
                left: gutter as i8,
                right: gutter as i8,
                top: 0,
                bottom: (gutter / 2.0) as i8,
            });
        let card = egui::Frame::NONE
            .fill(look.palette.bg_card)
            .corner_radius(crate::theme::R_LG)
            .inner_margin(egui::Margin::symmetric(look.metrics.side_inset as i8, 6));
        Panel::top("sound-banner")
            .frame(ground)
            .show_separator_line(false)
            .show(ui, |ui| {
                card.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let text = ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!("{}  {}", crate::theme::icons::WARNING, banner.text),
                        );
                        if let Some(detail) = &banner.detail {
                            text.on_hover_text(detail.as_str());
                        }
                        choose = ui.button("Choose device…").clicked();
                        dismiss = ui
                            .small_button(crate::theme::icons::CLOSE)
                            .on_hover_text("Hide")
                            .clicked();
                    })
                });
            });
        if choose {
            self.prefs.section = ui::preferences::Section::Playback;
            let ctx = ui.ctx().clone();
            self.open_preferences(&ctx);
        }
        if dismiss {
            self.banner_dismissed = Some(banner.text);
        }
    }

    // -------------------------------------------------------------- layout

    /// The slim top bar on the ground (ui-concept §3.5): menus, the
    /// project name and its save state, the Command bar's slot, then
    /// Preferences and the accent Export on the right. Empty space drags
    /// the window where the content runs under the title bar (macOS).
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let look = crate::theme::look(ui.ctx());
        let macos = cfg!(target_os = "macos");
        let frame = egui::Frame::NONE.fill(look.palette.bg_app);
        Panel::top("top_bar")
            .exact_size(look.metrics.top_bar)
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                let bar = ui.max_rect();
                // The background first, so every control added after it
                // wins the hit test.
                let background = ui.interact(
                    bar,
                    ui.id().with("top-bar-handle"),
                    egui::Sense::click_and_drag(),
                );
                if macos {
                    // The window drag starts once the press has moved 4 pt;
                    // the OS takes the pointer from there.
                    let started = ui.id().with("top-bar-drag-started");
                    if background.is_pointer_button_down_on() {
                        let moved = ui.input(|i| {
                            i.pointer
                                .press_origin()
                                .zip(i.pointer.latest_pos())
                                .map_or(egui::Vec2::ZERO, |(from, to)| to - from)
                        });
                        let already = ui.data(|d| d.get_temp::<bool>(started)).unwrap_or(false);
                        if !already && ui::shell::drag_should_start(moved) {
                            ui.data_mut(|d| d.insert_temp(started, true));
                            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
                        }
                    } else {
                        ui.data_mut(|d| d.remove::<bool>(started));
                    }
                    if background.double_clicked() {
                        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                    }
                }
                let inset = ui::shell::top_bar_left_inset(macos);
                let row = egui::Rect::from_min_max(
                    egui::pos2(bar.left() + inset, bar.top()),
                    egui::pos2(bar.right() - look.metrics.gutter, bar.bottom()),
                );
                let mut row_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(row)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                self.top_bar_row(&mut row_ui);
            });
    }

    fn top_bar_row(&mut self, ui: &mut egui::Ui) {
        let look = crate::theme::look(ui.ctx());
        let p = look.palette;
        if self.project.is_none() {
            // Home (UI-016): the FreeCut name and the gear only.
            ui.label(
                egui::RichText::new("FreeCut")
                    .text_style(egui::TextStyle::Body)
                    .color(p.text_secondary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.preferences_button(ui);
            });
            return;
        }
        let bar_style = move |style: &mut egui::Style| {
            egui::containers::menu::menu_style(style);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.spacing.item_spacing.x = 2.0;
            style.spacing.interact_size.y = look.metrics.icon_chip;
            let w = &mut style.visuals.widgets;
            w.inactive.fg_stroke.color = p.text_secondary;
            for state in [&mut w.hovered, &mut w.active, &mut w.open] {
                state.weak_bg_fill = p.bg_field;
                state.fg_stroke.color = p.text_primary;
                state.corner_radius = egui::CornerRadius::same(crate::theme::R_SM);
            }
        };
        let menu_style = move |style: &mut egui::Style| {
            // Rows rest without a fill (egui's own menu look), 26 pt tall.
            egui::containers::menu::menu_style(style);
            style.spacing.interact_size.y = 26.0;
            style.spacing.button_padding = egui::vec2(10.0, 4.0);
            style.spacing.item_spacing.y = 1.0;
            let w = &mut style.visuals.widgets;
            for state in [&mut w.hovered, &mut w.active, &mut w.open] {
                state.weak_bg_fill = p.bg_active;
                state.corner_radius = egui::CornerRadius::same(crate::theme::R_XS);
            }
        };
        // The right group first, right to left: Export, then the gear.
        // The Command bar (UI-034, ui-concept §10) takes the slot centred on
        // the bar once it exists; the Assistant toggle (S8) and the
        // shortcuts button (UI-033) join the right group then.
        let right = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                self.export_button(ui);
                ui.add_space(6.0 - 2.0);
                self.preferences_button(ui);
            })
            .response
            .rect;
        let row = ui.max_rect();
        let left = egui::Rect::from_min_max(
            row.min,
            egui::pos2((right.left() - 12.0).max(row.left()), row.max.y),
        );
        let mut left_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(left)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        egui::MenuBar::new()
            .style(bar_style)
            .config(egui::containers::menu::MenuConfig::new().style(menu_style))
            .ui(&mut left_ui, |ui| {
                self.menus(ui);
                ui.add_space(12.0);
                self.project_status(ui);
            });
    }

    /// The Preferences gear of the top bar (UI-018).
    fn preferences_button(&mut self, ui: &mut egui::Ui) {
        let tip = format!("Preferences ({})", shortcut_label(","));
        let gear =
            crate::widgets::icon_button::IconButton::new(crate::theme::icons::GEAR, &tip).show(ui);
        if gear.clicked() {
            let ctx = ui.ctx().clone();
            self.open_preferences(&ctx);
        }
    }

    /// The project name and its save state (UI-012); a failed save's
    /// state retries the save when clicked.
    fn project_status(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.project else { return };
        let state = ui::shell::SaveState::of(
            p.saving,
            p.editor.is_dirty(),
            self.save_failed.is_some(),
            p.file.is_some(),
        );
        let path = p.file.as_ref().map(|f| f.display().to_string());
        let retry = ui::shell::project_name(
            ui,
            &p.name(),
            path.as_deref(),
            state,
            self.save_failed.as_deref(),
        );
        if retry {
            self.save(false);
        }
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        // Every File command shows its UI-031 shortcut.
        let item = |ui: &mut egui::Ui, enabled: bool, label: &str| {
            let mut button = egui::Button::new(label);
            if let Some(keys) = file_shortcut(label) {
                button = button.shortcut_text(keys);
            }
            ui.add_enabled(enabled, button).clicked()
        };
        shell_menu(ui, "File", |ui| {
            if item(ui, true, "New Project…") {
                self.guarded(Pending::New);
            }
            if item(ui, true, "Open…") {
                self.open_dialog();
            }
            self.open_recent_menu(ui);
            ui.separator();
            let has_project = self.project.is_some();
            if item(ui, has_project, "Import Media…") {
                self.import_dialog();
            }
            ui.separator();
            if item(ui, has_project, "Save") {
                self.save(false);
            }
            if item(ui, has_project, "Save As…") {
                self.save(true);
            }
            ui.separator();
            if item(ui, has_project, "Export…") {
                self.open_export();
            }
            ui.separator();
            if item(ui, true, "Preferences…") {
                let ctx = ui.ctx().clone();
                self.open_preferences(&ctx);
            }
        });
        shell_menu(ui, "Edit", |ui| {
            let (undo, redo) = self.project.as_ref().map_or((None, None), |p| {
                (
                    p.editor.history().undo_label(),
                    p.editor.history().redo_label(),
                )
            });
            let undo_text = undo.map_or("Undo".to_owned(), |l| format!("Undo {}", op_name(l.op)));
            let redo_text = redo.map_or("Redo".to_owned(), |l| format!("Redo {}", op_name(l.op)));
            if ui
                .add_enabled(undo.is_some(), egui::Button::new(undo_text))
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(redo.is_some(), egui::Button::new(redo_text))
                .clicked()
            {
                self.redo();
            }
            ui.separator();
            if ui
                .add(egui::Button::new("Split at Playhead").shortcut_text(shortcut_label("B")))
                .clicked()
            {
                self.split();
            }
            if ui
                .add(egui::Button::new("Split All Tracks").shortcut_text(shift_shortcut_label("B")))
                .clicked()
            {
                self.split_all();
            }
            if ui.button("Delete").clicked() {
                self.delete(false);
            }
            if ui.button("Ripple Delete").clicked() {
                self.delete(true);
            }
        });
        shell_menu(ui, "Timeline", |ui| self.timeline_menu(ui));
        shell_menu(ui, "View", |ui| {
            ui.checkbox(&mut self.mixer.open, "Mixer");
        });
        shell_menu(ui, "Playback", |ui| {
            let has_project = self.project.is_some();
            let label = if self.playback.is_some() {
                "Pause"
            } else {
                "Play"
            };
            if ui
                .add_enabled(has_project, egui::Button::new(label).shortcut_text("Space"))
                .clicked()
            {
                self.toggle_play();
            }
            ui.separator();
            if ui
                .add_enabled(
                    has_project,
                    egui::Button::new("Go to Start").shortcut_text("Home"),
                )
                .clicked()
            {
                self.step(i64::MIN / 4);
            }
            if ui
                .add_enabled(
                    has_project,
                    egui::Button::new("Go to End").shortcut_text("End"),
                )
                .clicked()
            {
                self.go_to_end();
            }
            if ui
                .add_enabled(
                    has_project,
                    egui::Button::new("Previous Frame").shortcut_text("←"),
                )
                .clicked()
            {
                self.step(-1);
            }
            if ui
                .add_enabled(
                    has_project,
                    egui::Button::new("Next Frame").shortcut_text("→"),
                )
                .clicked()
            {
                self.step(1);
            }
            ui.separator();
            let mut muted = self.sound.muted();
            if ui.checkbox(&mut muted, "Mute Preview Sound").clicked() {
                self.sound.set_muted(muted);
            }
        });
        shell_menu(ui, "Help", |ui| {
            if ui.button("About FreeCut").clicked() {
                self.show_about = true;
            }
        });
    }

    /// The Timeline menu (UI-012): tools, cut away, markers and the
    /// sequence's modes, each with its UI-031 key.
    fn timeline_menu(&mut self, ui: &mut egui::Ui) {
        use ui::timeline_tools::TimelineTool;
        let has_project = self.project.is_some();
        let item = |ui: &mut egui::Ui, enabled: bool, label: &str, keys: &str| {
            ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys))
                .clicked()
        };
        for tool in TimelineTool::ALL {
            let label = format!("{} tool", tool.label());
            if ui
                .add(
                    egui::Button::new(label)
                        .shortcut_text(tool.key())
                        .selected(self.timeline.tool.active == tool),
                )
                .clicked()
            {
                self.timeline.tool.set(tool);
            }
        }
        ui.separator();
        let linked = self.timeline.linked_selection;
        let cut = self.project.as_ref().map(|p| p.cut_available(linked));
        let can_cut = matches!(cut, Some(Ok(())));
        if item(ui, can_cut, "Cut Away Before Playhead", "Q") {
            self.cut_away(Cut::Head, false);
        }
        if item(ui, can_cut, "Cut Away After Playhead", "W") {
            self.cut_away(Cut::Tail, false);
        }
        ui.separator();
        if item(ui, has_project, "Add Marker", "M") {
            self.add_marker(false);
        }
        if item(
            ui,
            has_project,
            "Add Marker with Next Colour",
            &alt_label("M"),
        ) {
            self.add_marker(true);
        }
        if item(
            ui,
            has_project,
            "Previous Marker",
            &shift_shortcut_label("M"),
        ) {
            self.jump_to_marker(false);
        }
        if item(ui, has_project, "Next Marker", &shift_label("M")) {
            self.jump_to_marker(true);
        }
        ui.separator();
        if let Some(p) = &self.project {
            let flags = p.flags();
            let mut magnet = flags.main_magnet;
            if ui
                .add(egui::Checkbox::new(&mut magnet, "Magnetic main track"))
                .clicked()
            {
                self.toggle_magnet();
            }
            let mut follow = flags.follow_main;
            if ui
                .add(egui::Checkbox::new(&mut follow, "Follow main track"))
                .clicked()
            {
                self.toggle_follow_main();
            }
        }
        ui.checkbox(&mut self.timeline.snapping, "Snapping");
        let mut skim = self.timeline.skim.is_on();
        if ui
            .add(egui::Checkbox::new(&mut skim, "Hover skim"))
            .on_hover_text("S")
            .clicked()
        {
            self.timeline.skim.toggle();
        }
        ui.checkbox(
            &mut self.timeline.linked_selection,
            "Select linked clips together",
        );
        ui.separator();
        if item(ui, has_project, "Record Voiceover…", "") {
            self.open_recorder();
        }
    }

    /// File › Open Recent (PM-017): the last ten projects, missing ones
    /// greyed with their path, and Clear list.
    fn open_recent_menu(&mut self, ui: &mut egui::Ui) {
        let entries: Vec<RecentEntry> = self
            .recent
            .entries()
            .iter()
            .take(crate::recent::MENU_ENTRIES)
            .cloned()
            .collect();
        let mut open = None;
        let mut clear = false;
        ui.add_enabled_ui(!entries.is_empty(), |ui| {
            ui.menu_button("Open Recent", |ui| {
                for e in &entries {
                    let path = e.path.display().to_string();
                    let item = ui
                        .add_enabled(e.found, egui::Button::new(&e.name))
                        .on_hover_text(&path)
                        .on_disabled_hover_text(format!("Not found: {path}"));
                    if item.clicked() {
                        open = Some(e.path.clone());
                    }
                }
                ui.separator();
                clear = ui.button("Clear List").clicked();
            });
        });
        if let Some(path) = open {
            self.guarded(Pending::Open(path));
        }
        if clear {
            self.recent.clear(Instant::now());
        }
    }

    /// The status line under the panels (22 pt on the ground): the export's
    /// task indicator (UI-040; messages are toasts).
    fn status_line(&mut self, ui: &mut egui::Ui) {
        let look = crate::theme::look(ui.ctx());
        let inset = (look.metrics.gutter + 6.0) as i8;
        let frame = egui::Frame::NONE
            .fill(look.palette.bg_app)
            .inner_margin(egui::Margin::symmetric(inset, 0));
        Panel::bottom("status")
            .exact_size(22.0)
            .frame(frame)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(dialog) = &mut self.export {
                        dialog.status_item(ui);
                    }
                });
            });
    }

    /// The accent Export button at the top right (UI-012, EX-021).
    fn export_button(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.project else { return };
        let empty = duration(p) <= Tick::ZERO;
        let running = self.export.as_ref().is_some_and(ExportDialog::is_running);
        let label = if running { "Exporting…" } else { "Export" };
        let response = ui::shell::export_button(ui, label, running || !empty)
            .on_disabled_hover_text("Add something to the timeline to export it")
            .on_hover_text(format!("Export the video ({})", shortcut_label("E")));
        if response.clicked() {
            self.open_export();
        }
    }

    /// Playback › Go to End and the End key.
    fn go_to_end(&mut self) {
        self.stop_playback();
        if let Some(p) = &mut self.project {
            p.playhead = duration(p);
        }
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        self.top_bar(ui);
        self.sound_banner(ui);
        self.request_covers();
        let view = ui::home::HomeView {
            recent: self.recent.entries(),
            covers: &self.covers,
            opening: self.opening,
            now: crate::recent::unix_now(),
            version: VERSION,
        };
        for action in ui::home::show(ui, &view) {
            match action {
                HomeAction::NewProject => self.guarded(Pending::New),
                HomeAction::Quick(preset) => {
                    self.create_project(preset, ui::home::DEFAULT_RATE);
                }
                HomeAction::Open => self.open_dialog(),
                HomeAction::OpenRecent(path) => self.guarded(Pending::Open(path)),
                HomeAction::Forget(path) => self.recent.remove(&path, Instant::now()),
                HomeAction::Reveal(path) => {
                    self.toast_action(ToastAction::Reveal(path));
                }
                HomeAction::About => self.show_about = true,
            }
        }
    }

    /// A new project with canvas `preset` and rate `rate` (PM-001);
    /// whether it was made.
    fn create_project(&mut self, preset: usize, rate: usize) -> bool {
        let (_, w, h) = CANVAS_PRESETS[preset.min(CANVAS_PRESETS.len() - 1)];
        let (_, num, den) = RATE_PRESETS[rate.min(RATE_PRESETS.len() - 1)];
        let settings = ProjectSettings::new_default().ok().and_then(|base| {
            Some(ProjectSettings {
                width: w,
                height: h,
                frame_rate: FrameRate::new(num, den).ok()?,
                ..base
            })
        });
        match settings.map(Project::new) {
            Some(Ok(project)) => {
                self.install(project);
                true
            }
            Some(Err(e)) => {
                self.say_as(ToastKind::Error, format!("{e:#}"), Vec::new());
                false
            }
            None => {
                self.say_as(
                    ToastKind::Error,
                    "Those settings aren't supported",
                    Vec::new(),
                );
                false
            }
        }
    }

    fn new_project_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.new_dialog else {
            return;
        };
        match ui::home::new_project_sheet(ctx, dialog) {
            Some(NewProjectChoice::Create) => {
                let (preset, rate) = (dialog.preset, dialog.rate);
                if self.create_project(preset, rate) {
                    self.new_dialog = None;
                }
            }
            Some(NewProjectChoice::Cancel) => self.new_dialog = None,
            None => {}
        }
    }

    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending.clone() else {
            return;
        };
        let mut choice = None;
        let name = self
            .project
            .as_ref()
            .map_or_else(String::new, Project::name);
        let sheet = Sheet::new(Id::new("unsaved"), "Save changes?")
            .width(440.0)
            .show(ctx, |s| {
                s.body(|ui| {
                    body_text(
                        ui,
                        &format!("“{name}” has changes that aren't saved. Save them first?"),
                    );
                });
                s.footer(|ui| {
                    if sheet_button(ui, ButtonKind::Primary, "Save").clicked() {
                        choice = Some(0);
                    }
                    if sheet_button(ui, ButtonKind::Secondary, "Cancel").clicked() {
                        choice = Some(2);
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if sheet_button(ui, ButtonKind::Ghost, "Don't Save").clicked() {
                            choice = Some(1);
                        }
                    });
                });
            });
        if sheet.close && choice.is_none() {
            choice = Some(2);
        }
        match choice {
            Some(0) => {
                self.save(false);
                if !matches!(action, Pending::Quit) {
                    self.pending = None;
                    self.run_pending(action);
                }
            }
            Some(1) => {
                self.pending = None;
                if matches!(action, Pending::Quit) {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                } else {
                    self.run_pending(action);
                }
            }
            Some(_) => self.pending = None,
            None => {}
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        use crate::widgets::panel::{Beyond, Sides, ground_frame, show_panel};
        use ui::shell::{Edge, gutter_feedback};

        self.top_bar(ui);
        self.sound_banner(ui);
        self.status_line(ui);

        let Some(project) = &mut self.project else {
            return;
        };
        let ctx = ui.ctx().clone();
        let look = crate::theme::look(&ctx);
        let m = look.metrics;
        let room = ui.available_rect_before_wrap();
        // Rev. 2 shell: every region is a rounded panel on the ground;
        // the gutters between them are the resize handles (§3.4).
        let restore = ui::shell::quiet_stock_resize_lines(ui);
        let mut gutters: Vec<(egui::Id, Edge, egui::Rect, egui::Rect)> = Vec::new();
        let mut focused: Option<egui::Rect> = None;
        let mut note_focus = |card: egui::Rect| {
            if ui::shell::holds_focus(&ctx, card) {
                focused = Some(card);
            }
        };
        let body_margin = egui::Margin::symmetric(m.side_inset as i8, m.side_inset as i8 - 4);
        let half = m.gutter / 2.0;
        // §3.2 / §13.17: every split stops where the regions beyond it reach
        // their minimums (outer sizes: the cards plus their gutters).
        use crate::widgets::panel::{
            ASSETS_MIN, INSPECTOR_MIN, STAGE_MIN, TIMELINE_MIN, UPPER_MIN, split_range,
        };
        const MIXER_MIN: f32 = 200.0;
        let upper_min = UPPER_MIN + half;
        let mixer_min = if self.mixer.open {
            MIXER_MIN + m.gutter
        } else {
            0.0
        };
        let stage_min = STAGE_MIN + m.gutter;
        let side_ground = m.gutter + half;

        let mut status = None;
        let mut seeked = false;
        let mut command = None;
        let mut tool = None;
        let mut edit_text = false;
        let mut match_reference = None;
        let mut skim_input = crate::skim::SkimInput {
            playing: self.playback.is_some(),
            ..crate::skim::SkimInput::default()
        };
        self.timeline.playing = self.playback.is_some();
        let bottom = Sides {
            left: Beyond::Window,
            right: Beyond::Window,
            top: Beyond::Panel,
            bottom: Beyond::Ground,
        };
        let timeline_range = split_range(TIMELINE_MIN, half, room.height(), upper_min + mixer_min);
        let timeline = Panel::bottom("timeline")
            .resizable(true)
            .default_size((room.height() * 0.42).round())
            .size_range(timeline_range)
            .frame(ground_frame(&look, bottom))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.set_style(Arc::clone(&restore));
                show_panel(ui, egui::Margin::ZERO, |ui| {
                    let out = ui::timeline::show(
                        ui,
                        project,
                        &mut self.timeline,
                        &self.waves,
                        &mut self.films,
                        self.mixer.open,
                    );
                    seeked = out.seeked;
                    command = out.command;
                    edit_text = out.edit_text;
                    tool = out.action;
                    skim_input.pointer = out.pointer_time;
                    skim_input.blade = out.blade_time;
                    skim_input.dragging = out.dragging;
                    match_reference = out.match_reference;
                    status = status.take().or(out.status);
                })
                .response
                .rect
            });
        note_focus(timeline.inner);
        gutters.push((
            egui::Id::new("timeline"),
            Edge::Top,
            timeline.response.rect,
            timeline.inner,
        ));
        if self.mixer.open {
            let mixer_sides = Sides {
                bottom: Beyond::Panel,
                ..bottom
            };
            let left_over = room.height() - timeline.response.rect.height();
            let mixer = Panel::bottom("mixer")
                .resizable(true)
                .default_size(250.0)
                .size_range(split_range(MIXER_MIN, m.gutter, left_over, upper_min))
                .frame(ground_frame(&look, mixer_sides))
                .show_separator_line(false)
                .show(ui, |ui| {
                    ui.set_style(Arc::clone(&restore));
                    show_panel(ui, body_margin, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong("Mixer");
                            ui.weak("Solo affects preview only");
                        });
                        let failed = ui::mixer::show(ui, project, &mut self.mixer);
                        status = status.take().or(failed);
                    })
                    .response
                    .rect
                });
            note_focus(mixer.inner);
            gutters.push((
                egui::Id::new("mixer"),
                Edge::Top,
                mixer.response.rect,
                mixer.inner,
            ));
        }

        let mut import = None;
        let mut add_text = false;
        let mut import_luts = false;
        let mut delete_look = None;
        let left = Sides {
            left: Beyond::Window,
            right: Beyond::Panel,
            top: Beyond::Ground,
            bottom: Beyond::Panel,
        };
        let media = Panel::left("media")
            .resizable(true)
            .default_size((room.width() * 0.26).round())
            .size_range(split_range(
                ASSETS_MIN,
                side_ground,
                room.width(),
                stage_min + INSPECTOR_MIN + side_ground,
            ))
            .frame(ground_frame(&look, left))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.set_style(Arc::clone(&restore));
                // The Shelf is the panel's header and runs edge to edge.
                show_panel(ui, egui::Margin::ZERO, |ui| {
                    let out = ui::assets::show(
                        ui,
                        project,
                        &mut self.assets,
                        &self.posters,
                        &self.waves,
                        self.imports.probing(),
                    );
                    import = out.import;
                    add_text = out.add_text;
                    import_luts = out.import_luts;
                    delete_look = out.delete_look;
                    status = status.take().or(out.status);
                })
                .response
                .rect
            });
        note_focus(media.inner);
        gutters.push((
            egui::Id::new("media"),
            Edge::Right,
            media.response.rect,
            media.inner,
        ));

        let view = CanvasView {
            canvas: self.preview_canvas,
            layers: &self.preview_layers,
        };
        let fonts = FontChoices {
            families: self.fonts.family_names(),
            scanning: self.fonts.scan_state() == ScanState::Scanning,
        };
        let mut inspector = ui::inspector::InspectorOutput::default();
        let right = Sides {
            left: Beyond::Panel,
            right: Beyond::Window,
            ..left
        };
        let inspector_room = room.width() - media.response.rect.width();
        // The inspector names the colour mask its Masks page edits each
        // frame it shows it (CL-031); the Stage reads it after.
        project.mask_focus = None;
        let inspector_panel = Panel::right("inspector")
            .resizable(true)
            .default_size((room.width() * 0.24).round())
            .size_range(split_range(
                INSPECTOR_MIN,
                side_ground,
                inspector_room,
                stage_min,
            ))
            .frame(ground_frame(&look, right))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.set_style(Arc::clone(&restore));
                // Full-bleed: the header, the lens strip and the sections run
                // edge to edge and keep their own side insets (§6.1).
                show_panel(ui, egui::Margin::ZERO, |ui| {
                    // The inspector scrolls its sections itself, under the lens strip.
                    inspector =
                        ui::inspector::show(ui, project, &mut self.inspector, &fonts, &view);
                })
                .response
                .rect
            });
        note_focus(inspector_panel.inner);
        gutters.push((
            egui::Id::new("inspector"),
            Edge::Left,
            inspector_panel.response.rect,
            inspector_panel.inner,
        ));
        ui.set_style(Arc::clone(&restore));

        let playing = self.playback.is_some();
        let sound = ui::player::SoundStatus {
            muted: self.sound.muted(),
            problem: self.sound.problem(),
        };
        let mut player = ui::player::PlayerOutput::default();
        let centre = Sides {
            left: Beyond::Panel,
            right: Beyond::Panel,
            top: Beyond::Ground,
            bottom: Beyond::Panel,
        };
        let stage = egui::CentralPanel::default()
            .frame(ground_frame(&look, centre))
            .show(ui, |ui| {
                show_panel(ui, egui::Margin::ZERO, |ui| {
                    player = ui::player::show(
                        ui,
                        project,
                        &mut self.master_drag,
                        &mut self.player,
                        &view,
                        self.preview_tex.as_ref(),
                        playing,
                        &sound,
                    );
                })
                .response
                .rect
            });
        note_focus(stage.inner);
        for (id, edge, outer, card) in gutters {
            gutter_feedback(ui, id, edge, outer, card);
        }
        if let Some(card) = focused {
            crate::widgets::panel::focus_line(ui.painter(), card, &look);
        }
        self.preview_size = player.wanted;
        // Hover skim (TL-014): the frame under the pointer, paced to the
        // display; the playhead stays.
        let want = crate::skim::skim_time(self.timeline.skim, skim_input);
        let refresh = ctx.input(|i| i.predicted_dt).clamp(1.0 / 240.0, 1.0 / 24.0);
        self.skim_throttle
            .set_interval(Duration::from_secs_f32(refresh));
        self.skim_shown = self.skim_throttle.update(Instant::now(), want);
        self.timeline.skim_at = self.skim_shown;
        if let Some(due) = self.skim_throttle.due() {
            ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
        }

        if let Some(s) = status.or(inspector.status.take()).or(player.status.take()) {
            self.say(s);
        }
        if seeked {
            self.seeked();
        }
        if let Some(c) = command.or(inspector.command) {
            self.run_command(c);
        }
        // The timeline toolbar (UI-011): the same commands as the menus
        // and keys.
        match tool {
            Some(ui::timeline_toolbar::ToolAction::Undo) => self.undo(),
            Some(ui::timeline_toolbar::ToolAction::Redo) => self.redo(),
            Some(ui::timeline_toolbar::ToolAction::Split) => self.split(),
            Some(ui::timeline_toolbar::ToolAction::Delete) => self.delete(false),
            Some(ui::timeline_toolbar::ToolAction::AddText) => self.add_text(),
            Some(ui::timeline_toolbar::ToolAction::ToggleMixer) => {
                self.mixer.open = !self.mixer.open;
            }
            Some(ui::timeline_toolbar::ToolAction::Import) => self.import_dialog(),
            Some(ui::timeline_toolbar::ToolAction::CutBefore { ripple_others }) => {
                self.cut_away(Cut::Head, ripple_others);
            }
            Some(ui::timeline_toolbar::ToolAction::CutAfter { ripple_others }) => {
                self.cut_away(Cut::Tail, ripple_others);
            }
            Some(ui::timeline_toolbar::ToolAction::Record) => {
                if self.recorder.running() {
                    self.recorder.stop();
                } else if self.recorder.open {
                    self.recorder.request_close();
                } else {
                    self.open_recorder();
                }
            }
            None => {}
        }
        if let Some(t) = inspector.seek.or(player.seek) {
            // Seeking is not an edit (UI-009, ui-concept §13.25).
            if let Some(p) = &mut self.project {
                p.playhead = t;
            }
            self.seeked();
        }
        if player.toggle_mute {
            let muted = !self.sound.muted();
            self.sound.set_muted(muted);
        }
        match import {
            Some(ImportRequest::Media) => self.import_dialog(),
            Some(ImportRequest::Audio) => self.import_audio_dialog(),
            None => {}
        }
        if (edit_text || player.edit_text)
            && let Some(clip) = self
                .project
                .as_ref()
                .and_then(|p| p.selection.first().copied())
        {
            self.stop_playback();
            self.inspector.props.edit_content(clip);
        }
        if add_text {
            self.add_text();
        }
        if inspector.split {
            self.split();
        }
        if inspector.import_luts || import_luts {
            self.import_luts_dialog();
        }
        if let Some(request) = inspector.save_look.take() {
            self.open_save_look(request);
        }
        if let Some(request) = inspector.color.take() {
            self.color_request(request);
        }
        if let Some((clip, at)) = match_reference {
            self.match_picked(clip, at);
        }
        // Pick mode ends when its clip is no longer the one selected.
        if let Some(p) = self.project.as_mut()
            && p.match_pick
                .is_some_and(|m| p.selection.first() != Some(&m.target))
        {
            p.match_pick = None;
        }
        if delete_look.is_some() {
            self.delete_look = delete_look;
        }
        if inspector.delete {
            self.delete_clips(false);
        }
        if player.toggle_play {
            self.toggle_play();
        }
        if player.step != 0 {
            self.step(player.step);
        }
        if player.to_start {
            self.step(i64::MIN / 4);
        }
        if player.to_end {
            self.stop_playback();
            if let Some(p) = &mut self.project {
                p.playhead = duration(p);
            }
        }
        // What edits reported (TL-025's "Also removed N attached items",
        // TL-053's skipped clips).
        if let Some(p) = &mut self.project {
            let notes = p.take_notes();
            if !notes.is_empty() {
                self.say(notes.join(". "));
            }
        }
    }
}

/// Hover skim's preference and the voiceover recorder (TL-014, AU-027).
impl FreeCutApp {
    /// Keeps the per-user preferences the timeline holds (hover skim) in
    /// the settings file.
    fn sync_preferences(&mut self) {
        let skim = self.timeline.skim;
        if self.settings.settings().timeline.skim != skim {
            let mut next = self.settings.settings().clone();
            next.timeline.skim = skim;
            self.settings.set(next, Instant::now());
        }
    }

    /// Opens the recorder panel and its microphone (AU-027).
    fn open_recorder(&mut self) {
        let Some(p) = &self.project else { return };
        let rate = p.doc().settings().sample_rate.hz();
        let prefs = self.settings.settings().voiceover.clone();
        self.recorder.show_panel(&prefs, rate);
    }

    /// Where a new take may go, in order: the project's Recordings folder,
    /// then the app-data fallback (PM-041).
    fn recording_folders(&self) -> Vec<PathBuf> {
        use freecut_project::recordings::{fallback_recordings_folder, recordings_folder};
        let file = self.project.as_ref().and_then(|p| p.file.as_deref());
        let mut folders: Vec<PathBuf> = file.and_then(recordings_folder).into_iter().collect();
        if let Some(data) = &self.app_data {
            folders.push(fallback_recordings_folder(data, file));
        }
        folders
    }

    /// Record (AU-027): a take at the playhead, or for *Redo last take* at
    /// the replaced take's start.
    fn start_take(&mut self, redo: Option<(freecut_timeline::ids::ClipId, Tick)>) {
        let Some(p) = &self.project else { return };
        if !self.recorder.open {
            self.open_recorder();
            return;
        }
        let start = redo.map_or(p.playhead, |(_, at)| at);
        // The take plays the timeline itself once its countdown is over.
        self.stop_playback();
        let folders = self.recording_folders();
        let prefs = self.settings.settings().voiceover.clone();
        if let Err(e) = self.recorder.begin_take(
            start,
            &prefs,
            folders,
            redo.map(|(clip, _)| clip),
            SystemTime::now(),
        ) {
            self.say(e);
        }
    }

    /// Reads the recorder's news, once per frame, before the shortcuts and
    /// the panels: edits lock while a take counts down or records.
    fn recorder_frame(&mut self, ctx: &egui::Context) {
        let offset = self.settings.settings().voiceover.latency_offset_ms;
        for event in self.recorder.update(offset) {
            self.recorder_event(event);
        }
        let locked = self.recorder.locks_edits();
        if let Some(p) = &mut self.project {
            p.edit_lock = locked.then_some(ui::recorder_panel::EDIT_LOCK);
        }
        self.timeline.record = ui::timeline_toolbar::RecordButton {
            running: self.recorder.button_text(),
            open: self.recorder.open,
        };
        self.timeline.live_take = self.project.as_ref().and_then(|p| {
            let doc = p.doc();
            doc.sequence(doc.root())
                .and_then(|seq| self.recorder.live_take(seq))
        });
        if self.recorder.running() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    fn recorder_event(&mut self, event: PanelEvent) {
        match event {
            PanelEvent::StartPlayback(from) => self.play_for_take(from),
            PanelEvent::StopPlayback => {
                self.stop_playback();
                self.restore_take_mute();
            }
            PanelEvent::Monitor(feed) => {
                let offered = feed.is_some();
                let plays = self.sound.set_monitor(feed);
                if offered && !plays && self.settings.settings().voiceover.monitor {
                    self.say(
                        "Hear yourself is off: the output device runs at another sample rate                          than the microphone",
                    );
                }
            }
            PanelEvent::Place(placement) => self.probe_take(placement),
            PanelEvent::Discard(path) => {
                self.jobs.spawn(move || {
                    Done::TakeRemoved(
                        std::fs::remove_file(&path)
                            .map_err(|e| format!("Couldn't remove the short take: {e}")),
                    )
                });
            }
            PanelEvent::Kept(path) => {
                self.say_as(
                    ToastKind::Info,
                    "The take was kept but not placed. It's in the recorder's Takes list",
                    vec![("Place".to_owned(), ToastAction::PlaceTake(path))],
                );
            }
            PanelEvent::Notice(text) => self.say(text),
        }
    }

    /// The countdown is over: the timeline plays from `from` (the start
    /// position less the pre-roll), muted if the user asked.
    fn play_for_take(&mut self, from: Tick) {
        self.stop_playback();
        let Some(p) = &mut self.project else { return };
        p.playhead = from;
        let end = duration(p);
        if from >= end {
            // Past the end there is nothing to play; the take still records.
            return;
        }
        if self.settings.settings().voiceover.mute_timeline {
            self.muted_before_take = Some(self.sound.muted());
            self.sound.set_muted(true);
        }
        self.sound.sync(self.project_serial, p);
        self.sound.play(from);
        self.playback = Some(Playback::new(from));
        self.recorder.playback_started(Instant::now(), from);
    }

    fn restore_take_mute(&mut self) {
        if let Some(muted) = self.muted_before_take.take() {
            self.sound.set_muted(muted);
        }
    }

    /// Probes a finished take on a worker; [`Done::TakeProbed`] places it.
    fn probe_take(&mut self, placement: Placement) {
        let serial = self.project_serial;
        let base = self.project.as_ref().and_then(|p| p.relative_base.clone());
        self.jobs.spawn(move || {
            let probed = probe_for_import(&placement.path, base.as_deref());
            Done::TakeProbed(serial, placement, probed)
        });
    }

    /// Places a probed take as one undo entry (`InsertRecordedClip`). One
    /// that can't be placed stays in the Takes list.
    fn place_take(&mut self, placement: Placement, probed: Result<ProbedFile, String>) {
        let Some(p) = &mut self.project else { return };
        let result = probed.and_then(|probed| {
            // A take that finished while the next one started is placed
            // anyway: placing it never moves the new take's start.
            let lock = p.edit_lock.take();
            let placed = p.insert_recorded(
                probed,
                placement.at,
                placement.handle_in,
                placement.target,
                placement.replace,
            );
            p.edit_lock = lock;
            placed
        });
        match result {
            Ok(clip) => self.recorder.placed(&placement.path, clip, placement.start),
            Err(e) => {
                self.recorder.not_placed(&placement);
                self.say(format!("The take couldn't be placed: {e}"));
            }
        }
    }

    /// *Place* on a take in the Takes list: where it was meant to go, or
    /// at the playhead for a recovered one.
    fn place_loose_take(&mut self, path: &std::path::Path) {
        let Some(p) = &self.project else { return };
        let Some(take) = self.recorder.loose_take(path).cloned() else {
            return;
        };
        let at = take.at.unwrap_or(p.playhead);
        let target = if self.settings.settings().voiceover.new_track {
            freecut_timeline::edit::RecordTarget::NewTrack
        } else {
            freecut_timeline::edit::RecordTarget::FirstFree
        };
        self.probe_take(Placement {
            path: take.path,
            at,
            handle_in: take.handle_in,
            target,
            replace: None,
            start: at,
        });
    }

    /// The startup scan for takes a crash interrupted (AU-027, PM-043),
    /// on a worker: their headers are repaired and they are offered in the
    /// Takes list.
    fn scan_interrupted_takes(&mut self) {
        let folders = self.recording_folders();
        if folders.is_empty() {
            return;
        }
        let serial = self.project_serial;
        self.jobs.spawn(move || {
            let mut found = Vec::new();
            let mut failed = Vec::new();
            for folder in folders {
                match freecut_project::recordings::recover_takes(&folder, |p| {
                    freecut_audio::record::wav::repair(p).map(|_| ())
                }) {
                    Ok(r) => {
                        found.extend(r.recovered);
                        failed.extend(
                            r.failed
                                .into_iter()
                                .map(|f| format!("{}: {}", f.take.display(), f.error)),
                        );
                    }
                    Err(e) => failed.push(format!("{}: {e}", folder.display())),
                }
            }
            Done::Recovered(serial, found, failed)
        });
    }

    fn takes_recovered(&mut self, takes: Vec<PathBuf>, failed: &[String]) {
        let Some(p) = &self.project else { return };
        // A take already in the library needs no offer.
        let known: Vec<&PathBuf> = p.media_paths.values().collect();
        let fresh: Vec<PathBuf> = takes.into_iter().filter(|t| !known.contains(&t)).collect();
        if !fresh.is_empty() {
            let n = fresh.len();
            self.recorder.recovered(fresh);
            let text = if n == 1 {
                "A recording interrupted last time was recovered".to_owned()
            } else {
                format!("{n} recordings interrupted last time were recovered")
            };
            self.say_as(
                ToastKind::Info,
                text,
                vec![("Show takes".to_owned(), ToastAction::ShowTakes)],
            );
        }
        for f in failed {
            eprintln!("FreeCut: couldn't recover a recording: {f}");
        }
    }

    /// Draws the recorder panel and runs what was clicked.
    fn recorder_panel(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.project else { return };
        if !self.recorder.open {
            return;
        }
        let mut prefs = self.settings.settings().voiceover.clone();
        let actions = self.recorder.show(
            ctx,
            self.timeline.record_button,
            &mut prefs,
            p.doc(),
            p.playhead,
        );
        if prefs != self.settings.settings().voiceover {
            let mut next = self.settings.settings().clone();
            next.voiceover = prefs;
            self.settings.set(next, Instant::now());
        }
        for action in actions {
            self.panel_action(action);
        }
    }

    fn panel_action(&mut self, action: PanelAction) {
        match action {
            PanelAction::Record => self.start_take(None),
            PanelAction::Stop => self.recorder.stop(),
            PanelAction::Cancel => self.recorder.cancel(),
            PanelAction::Close => self.recorder.request_close(),
            PanelAction::RedoLastTake => {
                let redo = self
                    .project
                    .as_ref()
                    .and_then(|p| self.recorder.redo_target(p.doc()));
                if redo.is_some() {
                    self.start_take(redo);
                }
            }
            PanelAction::Place(path) => self.place_loose_take(&path),
            PanelAction::ShowOnTimeline(media) => {
                let Some(p) = &mut self.project else { return };
                let doc = p.doc();
                let clips: Vec<(freecut_timeline::ids::ClipId, Tick)> = doc
                    .sequence(doc.root())
                    .map(|seq| {
                        seq.tracks()
                            .flat_map(|(_, t)| t.clips())
                            .filter(|c| {
                                matches!(
                                    c.content,
                                    freecut_timeline::model::ClipContent::Media { asset, .. }
                                        if asset == media
                                )
                            })
                            .map(|c| (c.id, c.start()))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(first) = clips.iter().map(|(_, t)| *t).min() {
                    p.selection = clips.iter().map(|(c, _)| *c).collect();
                    p.playhead = first;
                    self.seeked();
                }
            }
            PanelAction::Remove(media) => {
                let Some(p) = &mut self.project else { return };
                let req = EditRequest::RemoveMedia {
                    media: vec![media],
                    with_clips: true,
                };
                if let Err(e) = p.apply(&req, Coalesce::Never) {
                    self.say(e);
                }
            }
            PanelAction::OpenPrivacySettings => {
                self.jobs
                    .spawn(|| Done::OsOpened(crate::jobs::platform::open_microphone_settings()));
            }
        }
    }
}

/// The recorder's microphones: the system's, and in tests a synthetic tone
/// (no test ever opens a real microphone).
fn recorder_backend() -> crate::recorder::Backend {
    #[cfg(test)]
    {
        crate::recorder::Backend::synthetic(crate::recorder::tests::tone(), 48_000)
    }
    #[cfg(not(test))]
    {
        crate::recorder::Backend::system()
    }
}

/// A sheet's text: `type.body` in `text.primary`, wrapping.
fn body_text(ui: &mut egui::Ui, text: &str) {
    let p = crate::theme::look(ui.ctx()).palette;
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .text_style(egui::TextStyle::Body)
                .color(p.text_primary),
        )
        .wrap(),
    );
}

/// A footer button of a sheet.
fn sheet_button(ui: &mut egui::Ui, kind: ButtonKind, label: &str) -> egui::Response {
    crate::widgets::sheet::button(ui, kind, None, label, 80.0, None, true)
}

/// A menu of the top bar: a text label that shows a chip on hover or
/// while open (ui-concept §3.5).
fn shell_menu(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::containers::menu::MenuButton::from_button(
        egui::Button::new(label).frame_when_inactive(false),
    )
    .ui(ui, |ui| {
        // Menus are at least 200 pt wide, so labels and shortcuts breathe.
        ui.set_min_width(200.0);
        add(ui);
    });
}

/// "⌘E" on macOS, "Ctrl+E" elsewhere (UI-031).
/// Whether a widget owns the keyboard: a focused control, or an open list
/// or menu (a select's list uses `Space`, `Home`, `End` and the arrows),
/// so the app's shortcuts stand back.
fn keyboard_taken(ctx: &egui::Context) -> bool {
    ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx)
}

pub(crate) fn shortcut_label(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// `key` with `Shift` and the command key, as menus show it.
/// `Shift+key` with no command modifier ("⇧M" / "Shift+M").
fn shift_label(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⇧{key}")
    } else {
        format!("Shift+{key}")
    }
}

/// `Alt+key` ("⌥M" / "Alt+M").
fn alt_label(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌥{key}")
    } else {
        format!("Alt+{key}")
    }
}

pub(crate) fn shift_shortcut_label(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⇧⌘{key}")
    } else {
        format!("Ctrl+Shift+{key}")
    }
}

/// The UI-031 shortcut a File menu command shows (the keys
/// [`FreeCutApp::shortcuts`] handles).
fn file_shortcut(label: &str) -> Option<String> {
    Some(match label {
        "New Project…" => shortcut_label("N"),
        "Open…" => shortcut_label("O"),
        "Import Media…" => shortcut_label("I"),
        "Save" => shortcut_label("S"),
        "Save As…" => shift_shortcut_label("S"),
        "Export…" => shortcut_label("E"),
        "Preferences…" => shortcut_label(","),
        _ => return None,
    })
}

fn duration(project: &Project) -> Tick {
    project
        .doc()
        .sequence(project.doc().root())
        .map_or(Tick::ZERO, Sequence::duration)
}

/// FreeCut's bundled animation presets (FX-030, TX-056): compiled-in
/// manifests, so no file is read. Unusable ones are logged; the catalogue
/// still lists the ones this build can't play, which the grids show as
/// "Unavailable animation" (FX-018).
fn load_presets() -> PresetTable {
    let catalog = freecut_project::presets::bundled();
    for skipped in &catalog.skipped {
        eprintln!(
            "FreeCut: animation preset {} left out: {}",
            skipped.file, skipped.error
        );
    }
    catalog.table
}

/// Runs FX-030's photosensitivity check for every preset on a worker
/// thread, once per catalogue: the catalogue caches each result (shared by
/// every clone), so the Motion lens's grids never wait on it on the UI
/// thread. Without a thread the check runs on first use instead.
fn precompute_safety(presets: &Arc<PresetTable>) {
    let table = Arc::clone(presets);
    let spawned = std::thread::Builder::new()
        .name("freecut-preset-safety".into())
        .spawn(move || table.precompute());
    if let Err(e) = spawned {
        eprintln!("FreeCut: checking presets on a worker failed ({e}); checking on first use");
    }
}

impl eframe::App for FreeCutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // UI-014: a theme or density change applies on the next frame.
        let appearance = &self.settings.settings().appearance;
        if self.theme.sync(&ctx, appearance.theme, appearance.density) {
            ctx.request_repaint();
        }
        // UI-043: Reduce motion stops preset tiles playing on hover only.
        self.inspector.motion.reduce_motion = appearance.reduce_motion;
        crate::theme::motion::set_reduced(&ctx, appearance.reduce_motion);
        crate::widgets::icon_button::track_keyboard_navigation(&ctx);
        self.handle_jobs(&ctx);
        // The Look lens and the timeline read the LUT choices from the
        // project (cheap: one shared pointer).
        if let Some(p) = &mut self.project
            && !Arc::ptr_eq(&p.luts, &self.lut_choices)
        {
            p.luts = Arc::clone(&self.lut_choices);
        }
        if let Some(p) = &mut self.project
            && !Arc::ptr_eq(&p.looks, &self.looks)
        {
            p.looks = Arc::clone(&self.looks);
        }

        // Files dropped from the OS: projects open, LUTs and media import.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        // LUT files go to the LUT library (CL-016).
        let (luts, dropped): (Vec<_>, Vec<_>) = dropped
            .into_iter()
            .partition(|p| freecut_project::luts::LutFileKind::of_path(p).is_some());
        if !luts.is_empty() {
            match self.app_data.clone() {
                Some(data) => self.import_luts(luts, &data),
                None => self.say("LUTs can't be imported: FreeCut has no app-data folder here"),
            }
        }
        if !dropped.is_empty() {
            let (projects, media): (Vec<_>, Vec<_>) = dropped
                .into_iter()
                .partition(|p| p.extension().is_some_and(|e| e == "freecut"));
            if let Some(project) = projects.into_iter().next() {
                self.guarded(Pending::Open(project));
            } else if self.project.is_some() {
                self.import_paths(media, ImportRequest::Media);
            } else {
                self.say("Create or open a project first, then drop media into it");
            }
        }

        // Closing the window while exporting, or with unsaved work, asks
        // first.
        let close = ctx.input(|i| i.viewport().close_requested());
        let closing = close && !self.allow_close;
        if closing {
            // The OS close button and ⌘Q leave the content field focused.
            self.commit_text();
        }
        if closing && self.export.as_ref().is_some_and(ExportDialog::is_running) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.quit_during_export = true;
        } else if closing && self.unsaved() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        } else if close {
            // The window closes: a preference changed just now is written
            // first (UI-018), and so is the workspace (UI-002).
            self.recorder.shutdown_and_wait(Duration::from_secs(2));
            self.settings.flush_and_wait(Duration::from_secs(1));
            self.workspace.flush_and_wait(Duration::from_secs(1));
            self.recent.flush_and_wait(Duration::from_secs(1));
        }
        self.poll_export();
        if self.export.as_ref().is_some_and(ExportDialog::is_running) {
            // Progress in the dialog or the status bar, and the ETA window.
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        self.recorder_frame(&ctx);
        self.shortcuts(&ctx);
        self.advance_playback(&ctx);

        if self.project.is_some() {
            self.editor(ui);
        } else {
            self.home(ui);
        }
        self.recorder_panel(&ctx);
        self.sync_preferences();
        self.new_project_dialog(&ctx);
        self.unsaved_dialog(&ctx);
        self.restore_dialog(&ctx);
        self.save_look_dialog(&ctx);
        self.delete_look_dialog(&ctx);
        self.export_dialog(&ctx);
        self.quit_export_dialog(&ctx);
        self.preferences_window(&ctx);
        self.save_settings(&ctx);
        self.save_workspace(&ctx);
        let wake = ctx.clone();
        if let Some(at) = self
            .recent
            .tick(Instant::now(), move || wake.request_repaint())
        {
            ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
        }
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if let Some(e) = self.inspector.settle(self.project.as_mut(), pointer_down) {
            self.say(e);
        }
        self.master_drag.settle(self.project.as_mut(), pointer_down);
        self.mixer.settle(self.project.as_mut(), pointer_down);
        self.player.settle(self.project.as_mut(), pointer_down);
        self.timeline.settle(self.project.as_mut(), pointer_down);
        if self.imports.waiting() || self.inspector.props.commit_waiting() {
            // An import batch or typed text waited for a drag: it goes in
            // next frame.
            ctx.request_repaint();
        }
        if let Some(p) = &self.project {
            // Edits, undo, track mute and solo, master volume: heard after
            // the engine's read-ahead (AU-010, AU-011, AU-012).
            self.sound.sync(self.project_serial, p);
        }
        self.update_preview(&ctx);

        let title = self.project.as_ref().map_or_else(
            || "FreeCut".to_owned(),
            |p| format!("{} — FreeCut", p.title()),
        );
        ctx.send_viewport_cmd(ViewportCommand::Title(title));

        if self.show_about {
            let sheet = Sheet::new(Id::new("about"), "About FreeCut")
                .width(520.0)
                .show(&ctx, |s| {
                    let max = (ctx.content_rect().height() - 160.0).max(200.0);
                    s.scroll_body(max, about);
                    s.footer(|ui| sheet_button(ui, ButtonKind::Primary, "Done").clicked())
                });
            if sheet.close || sheet.inner {
                self.show_about = false;
            }
        }
        // Toasts above everything, over the status line in the editor.
        let bottom = if self.project.is_some() {
            22.0 + 8.0
        } else {
            16.0
        };
        for action in self.toasts.show(&ctx, bottom) {
            self.toast_action(action);
        }
    }
}

/// Hooks for the headless window snapshots (`crate::snapshots`).
#[cfg(test)]
impl FreeCutApp {
    /// The editor with `project` open, no sound device, the bundled fonts
    /// and `theme`, as the window would show it.
    pub(crate) fn headless(
        ctx: &egui::Context,
        project: Option<Project>,
        theme: crate::theme::ThemeChoice,
    ) -> FreeCutApp {
        crate::theme::fonts::install(ctx);
        let mut app = FreeCutApp::with_sound(ctx, Sound::without_device(), FontRegistry::bundled());
        let mut settings = app.settings.settings().clone();
        settings.appearance.theme = theme;
        app.settings.set(settings, Instant::now());
        app.project = project;
        app.project_serial = 1;
        // No output device here: the window shows without the banner
        // about it, as on a machine with sound.
        app.banner_dismissed = crate::sound::banner(&app.sound.output_state()).map(|b| b.text);
        app
    }

    /// The Stage's zoom and pan.
    pub(crate) fn set_stage_view(&mut self, view: ui::player::StageView) {
        self.player.view = view;
    }

    /// The open project's document.
    pub(crate) fn project_doc(&self) -> Option<&freecut_timeline::model::Document> {
        self.project.as_ref().map(|p| p.doc())
    }

    /// The asset panel's view state (tab, filters, library selection).
    pub(crate) fn assets_mut(&mut self) -> &mut AssetsState {
        &mut self.assets
    }

    /// The open project.
    pub(crate) fn project_for_tests(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    /// The timeline's view state.
    pub(crate) fn timeline_for_tests(&self) -> &ui::timeline::TimelineView {
        &self.timeline
    }

    /// The timeline's view state (zoom, scroll, lane sizes).
    pub(crate) fn timeline_mut(&mut self) -> &mut ui::timeline::TimelineView {
        &mut self.timeline
    }

    /// The inspector's state (the lens per kind, the Detail switch).
    pub(crate) fn inspector_mut(&mut self) -> &mut ui::inspector::InspectorState {
        &mut self.inspector
    }

    /// The recent projects Home lists.
    pub(crate) fn recent_mut(&mut self) -> &mut RecentProjects {
        &mut self.recent
    }

    /// Gives Home a cover for `path`.
    pub(crate) fn insert_cover(&mut self, ctx: &egui::Context, path: PathBuf, rgba: RgbaImage) {
        self.covers_requested.insert(path.clone());
        self.cover_arrived(ctx, path, (rgba.width, rgba.height, rgba.rgba));
    }

    /// Opens one of the sheets, as its menu item or button would.
    pub(crate) fn open_for_tests(&mut self, what: crate::snapshots::Open) {
        use crate::snapshots::Open;
        match what {
            Open::NewProject => self.new_dialog = Some(NewProjectDialog::default()),
            Open::Export(folder) => {
                self.export_folder = Some(folder);
                self.open_export();
            }
            Open::Preferences(section, search) => {
                self.prefs.open = true;
                self.prefs.section = section;
                self.prefs.set_search(search);
            }
            Open::About => self.show_about = true,
            Open::Unsaved => self.pending = Some(Pending::New),
        }
    }

    /// Sets the saved looks the Adjust tab and the Presets select list.
    pub(crate) fn set_looks_for_tests(&mut self, looks: crate::looks::SavedLooks) {
        self.looks = Arc::new(looks);
    }

    /// Selects `clips` in the open project.
    pub(crate) fn select_for_tests(&mut self, clips: Vec<ClipId>) {
        if let Some(p) = &mut self.project {
            p.selection = clips;
        }
    }

    /// Opens the Save look sheet for `stack`, as the lens's footer does.
    pub(crate) fn open_save_look_for_tests(
        &mut self,
        stack: freecut_timeline::model::ColorStack,
        keyed: bool,
    ) {
        // The sheet only asks for an app-data folder to write into.
        self.save_look = Some(ui::save_look::SaveLookDialog::new(
            stack,
            keyed,
            ui::save_look::suggested_name(self.looks.looks.len()),
        ));
    }

    /// The recorder panel (AU-027).
    pub(crate) fn recorder_for_tests(&self) -> &RecorderPanel {
        &self.recorder
    }

    pub(crate) fn recorder_mut_for_tests(&mut self) -> &mut RecorderPanel {
        &mut self.recorder
    }

    /// Opens the recorder, as the toolbar's microphone button does.
    pub(crate) fn open_recorder_for_tests(&mut self) {
        self.open_recorder();
    }

    /// Record, as the panel's button does.
    pub(crate) fn record_for_tests(&mut self) {
        self.start_take(None);
    }

    /// Changes the recorder's preferences.
    pub(crate) fn voiceover_for_tests(&mut self, f: impl FnOnce(&mut settings::VoiceoverSettings)) {
        let mut next = self.settings.settings().clone();
        f(&mut next.voiceover);
        self.settings.set(next, Instant::now());
    }

    /// The Export sheet.
    pub(crate) fn export_mut(&mut self) -> Option<&mut ExportDialog> {
        self.export.as_mut()
    }

    /// Raises a toast.
    pub(crate) fn toast_for_tests(
        &mut self,
        kind: ToastKind,
        message: &str,
        actions: Vec<(String, ToastAction)>,
    ) {
        self.say_as(kind, message, actions);
    }

    /// Whether the frame for the current playhead is on screen and every
    /// poster and waveform asked for has arrived.
    pub(crate) fn settled(&self) -> bool {
        if self.project.is_none() {
            return true;
        }
        self.preview_tex.is_some()
            && self.preview_key.is_some()
            && self.preview_shown == self.preview_generation
            && self
                .posters_requested
                .iter()
                .all(|id| self.posters.contains_key(id))
            && !self.waves.pending()
            && !self.films.pending()
    }
}

/// The About window's body (UI-051): the licence, then the notices of the
/// bundled fonts and icons.
fn about(ui: &mut egui::Ui) {
    let p = crate::theme::look(ui.ctx()).palette;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            egui::RichText::new("FreeCut")
                .text_style(crate::theme::Type::Title.style(ui.style()))
                .color(p.text_primary),
        );
        ui.label(
            egui::RichText::new(VERSION)
                .text_style(egui::TextStyle::Small)
                .color(p.text_tertiary),
        );
    });
    body_text(ui, "A free, open-source video editor.");
    ui.add_space(4.0);
    crate::widgets::sheet::fact_list(
        ui,
        &[
            ("Version", VERSION.to_owned()),
            ("License", "GPL-3.0-or-later".to_owned()),
            ("Privacy", "FreeCut collects nothing".to_owned()),
            ("Account", "None needed. No watermark, no cloud".to_owned()),
        ],
    );
    ui.add_space(2.0);
    if crate::widgets::sheet::button(
        ui,
        ButtonKind::Secondary,
        Some(crate::theme::icons::COPY),
        "Copy version info",
        0.0,
        None,
        true,
    )
    .clicked()
    {
        ui.ctx().copy_text(format!(
            "FreeCut {VERSION} ({} {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    }
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Bundled fonts and icons")
            .text_style(crate::theme::Type::Label.style(ui.style()))
            .color(p.text_primary),
    );
    for notice in crate::theme::fonts::NOTICES {
        egui::CollapsingHeader::new(format!("{} — {}", notice.name, notice.license))
            .id_salt(("about-notice", notice.name))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("about-notice-text", notice.name))
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(notice.text)
                                    .text_style(egui::TextStyle::Small)
                                    .color(p.text_secondary),
                            )
                            .wrap(),
                        );
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Key, Modifiers};

    use super::*;
    use crate::ui::drag::tests::{Frames, key, project_with_a_sound};

    /// UI-031: every File menu command with a shortcut shows it, not only
    /// Export and Preferences.
    #[test]
    fn the_file_menu_shows_every_shortcut() {
        for (label, keys) in [
            ("New Project…", shortcut_label("N")),
            ("Open…", shortcut_label("O")),
            ("Import Media…", shortcut_label("I")),
            ("Save", shortcut_label("S")),
            ("Save As…", shift_shortcut_label("S")),
            ("Export…", shortcut_label("E")),
            ("Preferences…", shortcut_label(",")),
        ] {
            assert_eq!(file_shortcut(label), Some(keys), "{label}");
        }
        assert_eq!(file_shortcut("Open Recent"), None);
    }

    /// ui-concept §13.20: while a select's list is open it owns the
    /// keyboard (`Space` picks, `Home` / `End` move), so the app's
    /// shortcuts stand back; once a pointer pick closed it, they work again.
    #[test]
    fn an_open_list_takes_the_keyboard_from_the_shortcuts() {
        use crate::ui::drag::tests::button;
        use crate::widgets::select::{Select, SelectOption};

        let options = [SelectOption::new("Linear"), SelectOption::new("Fast")];
        let id = egui::Id::new("select");
        let mut frames = Frames::new();
        let frame = |frames: &mut Frames, events: Vec<egui::Event>| {
            frames.run(events, |ui| {
                Select::new(id, &options, Some(0)).show(ui);
            });
        };
        let click = |frames: &mut Frames, at: egui::Pos2| {
            frame(
                frames,
                vec![egui::Event::PointerMoved(at), button(at, true)],
            );
            frame(frames, vec![button(at, false)]);
            frame(frames, Vec::new());
        };
        frame(&mut frames, Vec::new());
        assert!(!keyboard_taken(&frames.ctx));
        let at = frames.ctx.read_response(id).unwrap().rect.center();
        click(&mut frames, at);
        assert!(keyboard_taken(&frames.ctx), "the open list has it");
        let row = frames
            .ctx
            .read_response(id.with("list").with(1usize))
            .unwrap()
            .rect;
        click(&mut frames, row.center());
        assert!(!keyboard_taken(&frames.ctx), "back to the shortcuts");
    }

    /// UI-051 / ui-concept §14: the About window carries the notices of
    /// every bundled font and icon set, each with its licence text.
    #[test]
    fn about_shows_the_bundled_licence_notices() {
        use crate::theme::fonts::NOTICES;

        let names: Vec<&str> = NOTICES.iter().map(|n| n.name).collect();
        assert_eq!(
            names,
            ["Inter", "JetBrains Mono", "Noto Sans", "Phosphor Icons"]
        );
        for n in NOTICES {
            match n.license {
                "MIT" => assert!(
                    n.text.contains("Permission is hereby granted"),
                    "{}",
                    n.name
                ),
                _ => assert!(
                    n.text.contains("SIL Open Font License, Version 1.1"),
                    "{}",
                    n.name
                ),
            }
            assert!(n.text.contains("Copyright"), "{}", n.name);
        }
        // The window lists each notice.
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), about);
        output.textures_delta.clear();
        let texts: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        for n in NOTICES {
            let title = format!("{} — {}", n.name, n.license);
            assert!(texts.contains(&title), "{title} in {texts:?}");
        }
    }

    fn press(app: &mut FreeCutApp, frames: &mut Frames, k: Key, modifiers: Modifiers) {
        let ctx = frames.ctx.clone();
        frames.run(vec![key(k, modifiers)], |_| app.shortcuts(&ctx));
    }

    #[test]
    fn exporting_from_the_dialog_saves_the_settings_and_keeps_editing_open() {
        use freecut_timeline::edit::{EditRequest, InsertTarget};
        use freecut_timeline::model::RichText;

        use crate::ui::export::Stage;

        if !crate::jobs::platform::encoder_available() {
            eprintln!("skipped: this FFmpeg has no libx264");
            return;
        }
        let mut frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let mut project = Project::new(ProjectSettings {
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            ..ProjectSettings::new_default().unwrap()
        })
        .unwrap();
        let clip = crate::ui::text::insert_text(&mut project, InsertTarget::FirstFree, Tick::ZERO)
            .unwrap();
        project.editor.mark_saved();
        app.project = Some(project);
        let out = tempfile::tempdir().unwrap();
        let frame = |app: &mut FreeCutApp, frames: &mut Frames| {
            frames.run(Vec::new(), |ui| {
                let ctx = ui.ctx().clone();
                app.handle_jobs(&ctx);
                app.poll_export();
                app.export_dialog(&ctx);
            });
        };

        // ⌘E opens the dialog; it owns the keyboard.
        press(&mut app, &mut frames, Key::E, Modifiers::COMMAND);
        let dialog = app.export.as_mut().unwrap();
        assert!(dialog.visible);
        assert_eq!(dialog.settings.file.name, "Untitled");
        dialog.set_folder(out.path().to_path_buf());
        dialog.settings.file.name = "Title card".to_owned();
        assert!(app.modal_open());

        // The folder and encoder checks run on the pool.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !app.export.as_ref().unwrap().is_ready() {
            assert!(Instant::now() < deadline, "the checks never finished");
            frame(&mut app, &mut frames);
            std::thread::sleep(Duration::from_millis(5));
        }

        // Export: the job starts on a snapshot; the settings are stored
        // without an undo entry, and the project counts as modified.
        let entries = app.project.as_ref().unwrap().editor.history().len();
        let ctx = frames.ctx.clone();
        app.export_actions(&ctx, vec![ExportAction::Start]);
        assert!(app.export.as_ref().unwrap().is_running());
        let p = app.project.as_ref().unwrap();
        assert_eq!(p.export.as_ref().unwrap().file.name, "Title card");
        assert!(p.editor.is_dirty());
        assert_eq!(p.editor.history().len(), entries);

        // "Keep editing": the dialog hides and edits go through while the
        // job runs on its snapshot.
        app.export.as_mut().unwrap().visible = false;
        assert!(!app.modal_open());
        let p = app.project.as_mut().unwrap();
        let seq = p.doc().root();
        p.apply(
            &EditRequest::SetTextContent {
                seq,
                clip,
                content: RichText::new("Changed"),
            },
            Coalesce::Never,
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(120);
        while app.export.as_ref().unwrap().is_running() {
            assert!(Instant::now() < deadline, "the export never ended");
            frame(&mut app, &mut frames);
            std::thread::sleep(Duration::from_millis(5));
        }
        let Stage::Finished(result) = &app.export.as_ref().unwrap().stage else {
            panic!("{:?}", app.export.as_ref().unwrap().stage);
        };
        assert_eq!(result.path, out.path().join("Title card.mp4"));
        assert_eq!(result.frames, 90);
        assert!(!result.audio, "a silent timeline exports without audio");
        assert!(result.path.is_file());
        assert_eq!(
            std::fs::read_dir(out.path()).unwrap().count(),
            1,
            "no .part left"
        );
        // A second export can start now; reopening shows the last settings.
        app.open_export();
        let dialog = app.export.as_ref().unwrap();
        assert!(matches!(dialog.stage, Stage::Settings));
        assert_eq!(dialog.settings.file.name, "Title card");
    }

    /// PM-001 / PM-017: a quick-start canvas makes a project at once; once
    /// it has a file it tops the recent projects, which Home can forget;
    /// messages are toasts (UI-040).
    #[test]
    fn quick_start_and_recent_projects() {
        let frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        assert!(app.create_project(1, 2));
        let p = app.project.as_mut().unwrap();
        let s = p.doc().settings();
        assert_eq!((s.width, s.height), (1080, 1920));
        assert_eq!(s.frame_rate, FrameRate::new(25, 1).unwrap());
        // Not saved yet: nothing to remember.
        app.remember_project();
        assert!(app.recent.entries().is_empty());
        let file = PathBuf::from("/projects/Reel.freecut");
        app.project.as_mut().unwrap().file = Some(file.clone());
        app.remember_project();
        let entry = &app.recent.entries()[0];
        assert_eq!(entry.name, "Reel");
        assert_eq!(entry.canvas, (1080, 1920));
        assert_eq!(entry.rate, (25, 1));
        assert!(entry.found);
        app.recent.remove(&file, Instant::now());
        assert!(app.recent.entries().is_empty());

        app.say("Snapping off");
        assert_eq!(app.toasts.messages(), ["Snapping off"]);
    }

    #[test]
    fn typed_text_counts_as_unsaved_work_before_another_project_opens() {
        use freecut_timeline::model::TextClipData;

        use crate::ui::props::ContentSession;

        let frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = ui::text::add_text_at_playhead(&mut project).unwrap();
        project.editor.mark_saved();
        app.project = Some(project);
        let content = |app: &FreeCutApp| {
            let c = app.project.as_ref().unwrap().doc().clip(clip).unwrap();
            c.content.text().unwrap().content.as_str().to_owned()
        };
        // The field still has focus: nothing is committed yet.
        let mut session = ContentSession::new(clip, TextClipData::PLACEHOLDER);
        session.draft = "New caption".to_owned();
        app.inspector.props.content = Some(session);

        // Dropping a project file asks first, with the typing committed.
        app.guarded(Pending::Open(PathBuf::from("/nowhere/other.freecut")));
        assert!(matches!(app.pending, Some(Pending::Open(_))), "asks first");
        assert!(!app.opening);
        assert_eq!(content(&app), "New caption");
        assert!(app.project.as_ref().unwrap().editor.is_dirty());

        // While a drag holds the commit back, the draft alone counts.
        app.pending = None;
        let p = app.project.as_mut().unwrap();
        p.editor.mark_saved();
        let gesture = p.begin_gesture();
        let mut session = ContentSession::new(clip, "New caption");
        session.draft = "Newer".to_owned();
        app.inspector.props.content = Some(session);
        app.guarded(Pending::New);
        assert!(matches!(app.pending, Some(Pending::New)));
        app.project.as_mut().unwrap().end_gesture(gesture);
    }

    /// UI-002 / UI-008: the inspector's remembered state comes back from
    /// `workspace.json`, unless it was changed before the file was read;
    /// changes are written back.
    #[test]
    fn the_inspector_workspace_is_restored_and_saved() {
        use crate::ui::inspector::lens::{CardId, Detail, SelectionKind, Workspace};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(workspace::FILE_NAME);
        let mut saved = Workspace::default();
        saved.card(SelectionKind::Text, CardId::Shadow).more = true;
        saved.detail = Detail::Everything;
        let raw = serde_json::json!({ "version": 1, "inspector": saved.to_json() });
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();

        let frames = Frames::new();
        let ctx = frames.ctx.clone();
        let mut app =
            FreeCutApp::with_sound(&ctx, Sound::without_device(), FontRegistry::bundled());
        // Drawn before the file arrived, unchanged.
        app.inspector
            .workspace
            .card(SelectionKind::Nothing, CardId::Project);
        app.workspace_loaded(workspace::load(&path), path.clone());
        assert_eq!(app.inspector.workspace, saved);
        app.inspector.workspace.detail = Detail::Essentials;
        app.save_workspace(&ctx);
        app.workspace.flush_and_wait(Duration::from_secs(10));
        let back = workspace::load(&path).unwrap();
        assert_eq!(
            Workspace::from_json(&back[workspace::INSPECTOR]).detail,
            Detail::Essentials
        );

        // Changed before the file arrived: the change wins.
        let mut app =
            FreeCutApp::with_sound(&ctx, Sound::without_device(), FontRegistry::bundled());
        app.inspector
            .workspace
            .card(SelectionKind::Video, CardId::Volume)
            .open = false;
        let mine = app.inspector.workspace.clone();
        app.workspace_loaded(workspace::load(&path), path);
        assert_eq!(app.inspector.workspace, mine);
    }

    /// A save that finishes after another project was opened is not
    /// applied to the new one: its file, save point and failure stay the
    /// new project's own.
    #[test]
    fn a_save_finished_for_the_previous_project_leaves_the_new_one_alone() {
        let frames = Frames::new();
        let ctx = frames.ctx.clone();
        let mut app =
            FreeCutApp::with_sound(&ctx, Sound::without_device(), FontRegistry::bundled());
        let (mut first, _) = project_with_a_sound();
        first.file = Some(PathBuf::from("/projects/A.freecut"));
        let job = |p: &Project, result| SavedJob {
            target: PathBuf::from("/projects/A.freecut"),
            token: p.editor.generation(),
            result,
        };
        let (ok, failed) = (
            job(&first, Ok(())),
            job(&first, Err("disk full".to_owned())),
        );
        app.install(first);
        let started_for = app.project_serial;
        // Another project opens while the save runs; it has unsaved edits.
        let (second, _) = project_with_a_sound();
        app.install(second);
        let dirty = app.project.as_ref().unwrap().editor.is_dirty();

        app.save_finished(&ctx, started_for, &ok);
        app.save_finished(&ctx, started_for, &failed);
        let p = app.project.as_ref().unwrap();
        assert_eq!(p.file, None, "not bound to A's file");
        assert_eq!(p.editor.is_dirty(), dirty, "its save point is its own");
        assert_eq!(app.save_failed, None, "no Retry for the other project");

        // Its own save still lands.
        let own = SavedJob {
            target: PathBuf::from("/projects/B.freecut"),
            token: app.project.as_ref().unwrap().editor.generation(),
            result: Ok(()),
        };
        app.save_finished(&ctx, app.project_serial, &own);
        let p = app.project.as_ref().unwrap();
        assert_eq!(p.file, Some(PathBuf::from("/projects/B.freecut")));
        assert!(!p.editor.is_dirty());
    }

    #[test]
    fn a_poster_made_for_the_previous_project_is_dropped() {
        let frames = Frames::new();
        let ctx = frames.ctx.clone();
        let mut app =
            FreeCutApp::with_sound(&ctx, Sound::without_device(), FontRegistry::bundled());
        let hdr_poster = |project, id| Poster {
            project,
            id,
            image: Ok(RgbaImage {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
                hdr: Some(HdrTransfer::Pq),
            }),
        };
        let first_item = |p: &Project| p.doc().library().items().next().unwrap().id;

        // A poster job starts for the open project's item...
        let (first, _) = project_with_a_sound();
        let old = first_item(&first);
        app.install(first);
        let started_for = app.project_serial;
        // ...and another project opens before it finishes. Its item has
        // the same id: ids restart with every project.
        let (second, _) = project_with_a_sound();
        let id = first_item(&second);
        assert_eq!(id, old);
        app.install(second);

        // The late result is dropped: no picture, no HDR notice.
        assert_eq!(app.poster_arrived(&ctx, hdr_poster(started_for, id)), None);
        assert!(app.posters.is_empty(), "not on the new project's tile");

        // The new project's own poster lands, and its notice is still due.
        let notice = app.poster_arrived(&ctx, hdr_poster(app.project_serial, id));
        assert_eq!(notice.map(|(_, tf)| tf), Some(HdrTransfer::Pq));
        assert!(app.posters.contains_key(&id));
    }

    #[test]
    fn a_preference_changed_before_the_file_loads_keeps_the_files_others() {
        let frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let mut early = app.settings.settings().clone();
        early.playback.buffer_frames = 2048;
        app.settings.set(early, Instant::now());

        let mut file = settings::Settings::default();
        file.playback.output_device = Some("USB Headphones".to_owned());
        let dir = tempfile::tempdir().unwrap();
        app.settings_loaded(
            settings::Loaded {
                raw: file.to_raw(&serde_json::Map::new()),
                settings: file,
                notice: None,
            },
            dir.path().join(settings::FILE_NAME),
        );
        let now = app.settings.settings();
        assert_eq!(
            now.playback.output_device.as_deref(),
            Some("USB Headphones"),
            "the saved device stays"
        );
        assert_eq!(now.playback.buffer_frames, 2048, "the early change wins");
    }

    #[test]
    fn delete_removes_selected_volume_keyframes_before_the_clip() {
        use crate::ui::drag::tests::button;
        use crate::ui::timeline::tests::{line_point, timeline_frame};

        let mut frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let (mut project, clip) = project_with_a_sound();
        let one = Tick::from_secs(1).unwrap();
        let two = Tick::from_secs(2).unwrap();
        clip_keys::add_key(&mut project, clip, one).unwrap();
        clip_keys::add_key(&mut project, clip, two).unwrap();
        app.project = Some(project);
        let keys = |app: &FreeCutApp| {
            app.project.as_ref().unwrap().doc().clip(clip).map(|c| {
                c.audio
                    .active()
                    .unwrap()
                    .gain
                    .keys()
                    .iter()
                    .map(|k| k.at)
                    .collect::<Vec<_>>()
            })
        };

        // Click the first dot on the timeline.
        let FreeCutApp {
            project,
            timeline,
            waves,
            ..
        } = &mut app;
        let project = project.as_mut().unwrap();
        let (rect, _) = timeline_frame(&mut frames, project, timeline, waves, Vec::new());
        let dot = line_point(project, timeline, rect, clip, one);
        let events = vec![egui::Event::PointerMoved(dot), button(dot, true)];
        timeline_frame(&mut frames, project, timeline, waves, events);
        timeline_frame(
            &mut frames,
            project,
            timeline,
            waves,
            vec![button(dot, false)],
        );

        // Delete removes the dot, not the clip; then the clip goes.
        press(&mut app, &mut frames, Key::Delete, Modifiers::NONE);
        assert_eq!(keys(&app), Some(vec![two]));
        press(&mut app, &mut frames, Key::Delete, Modifiers::NONE);
        assert_eq!(keys(&app), None, "the clip is deleted");
        app.undo();
        app.undo();
        assert_eq!(keys(&app), Some(vec![one, two]));
    }

    #[test]
    fn editing_shortcuts_wait_while_the_restore_dialog_is_open() {
        let mut frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let (mut project, clip) = project_with_a_sound();
        // Split needs the playhead inside the clip.
        project.playhead = freecut_timeline::time::Tick::from_secs(1).unwrap();
        app.project = Some(project);
        app.restore_confirm = Some(RestoreConfirm {
            clips: vec![clip],
            removed: Vec::new(),
        });
        let entries = |app: &FreeCutApp| app.project.as_ref().unwrap().editor.history().len();
        let before = entries(&app);

        for (k, modifiers) in [
            (Key::Delete, Modifiers::NONE),
            (Key::B, Modifiers::COMMAND),
            (Key::Z, Modifiers::COMMAND),
        ] {
            press(&mut app, &mut frames, k, modifiers);
            let p = app.project.as_ref().unwrap();
            assert!(p.doc().clip(clip).is_some(), "{k:?} waits for the dialog");
            assert_eq!(entries(&app), before, "no edit or undo behind it ({k:?})");
        }

        // With the dialog closed, the same key deletes.
        app.restore_confirm = None;
        press(&mut app, &mut frames, Key::Delete, Modifiers::NONE);
        assert!(app.project.as_ref().unwrap().doc().clip(clip).is_none());
    }

    /// UI-031's toolbar-round keys: `S` no longer splits; `A`/`B`/`H`
    /// pick tools; `Q`/`W` cut away; `M`, `Alt+M`, `Shift+M` and
    /// `Ctrl+Shift+M` add and visit markers; `Alt+N`, `Shift+N` and `N`
    /// toggle following, the magnet and snapping. Each edit is one entry.
    #[test]
    fn the_toolbar_round_keys() {
        use crate::timeline_commands::tests::{s, talking_head};
        use crate::ui::timeline_tools::TimelineTool;
        use freecut_timeline::model::PaletteColor;

        let mut frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let (mut project, [a, b, _]) = talking_head();
        project.playhead = s(2);
        app.project = Some(project);
        let entries = |app: &FreeCutApp| app.project.as_ref().unwrap().editor.history().len();
        let before = entries(&app);

        press(&mut app, &mut frames, Key::S, Modifiers::NONE);
        assert_eq!(entries(&app), before, "plain S doesn't split any more");
        for (k, tool) in [
            (Key::B, TimelineTool::Blade),
            (Key::H, TimelineTool::Hand),
            (Key::A, TimelineTool::Select),
        ] {
            press(&mut app, &mut frames, k, Modifiers::NONE);
            assert_eq!(app.timeline.tool.active, tool);
        }

        // Q: the head of the main clip under the playhead goes.
        press(&mut app, &mut frames, Key::Q, Modifiers::NONE);
        let p = app.project.as_ref().unwrap();
        assert_eq!(entries(&app), before + 1);
        assert_eq!(p.doc().clip(a).unwrap().end(), s(2));
        assert_eq!(p.playhead, Tick::ZERO, "to the clip's start");
        // W at 1 s: the tail of the same clip.
        app.project.as_mut().unwrap().playhead = s(1);
        press(&mut app, &mut frames, Key::W, Modifiers::NONE);
        let p = app.project.as_ref().unwrap();
        assert_eq!(p.doc().clip(a).unwrap().end(), s(1));
        assert_eq!(p.doc().clip(b).unwrap().start(), s(1), "rippled");

        // Markers.
        press(&mut app, &mut frames, Key::M, Modifiers::NONE);
        app.project.as_mut().unwrap().playhead = s(3);
        press(&mut app, &mut frames, Key::M, Modifiers::ALT);
        let p = app.project.as_ref().unwrap();
        let colors: Vec<PaletteColor> = p
            .doc()
            .sequence(p.doc().root())
            .unwrap()
            .markers
            .iter()
            .map(|m| m.color)
            .collect();
        assert_eq!(colors, [PaletteColor::Blue, PaletteColor::Teal]);
        press(
            &mut app,
            &mut frames,
            Key::M,
            Modifiers::COMMAND | Modifiers::SHIFT,
        );
        assert_eq!(app.project.as_ref().unwrap().playhead, s(1));
        press(&mut app, &mut frames, Key::M, Modifiers::SHIFT);
        assert_eq!(app.project.as_ref().unwrap().playhead, s(3));

        // Modes.
        let n = entries(&app);
        press(&mut app, &mut frames, Key::N, Modifiers::ALT);
        assert!(!app.project.as_ref().unwrap().flags().follow_main);
        press(&mut app, &mut frames, Key::N, Modifiers::SHIFT);
        assert!(!app.project.as_ref().unwrap().flags().main_magnet);
        assert_eq!(entries(&app), n + 2, "one entry each");
        press(&mut app, &mut frames, Key::N, Modifiers::NONE);
        assert!(!app.timeline.snapping);
        assert_eq!(entries(&app), n + 2, "snapping is a preference");

        // Delete takes a selected marker with the selected clip, as one
        // entry.
        let p = app.project.as_mut().unwrap();
        let marker = p.markers.iter().next().copied().unwrap();
        p.selection = vec![b];
        p.markers = std::collections::BTreeSet::from([marker]);
        let n = entries(&app);
        press(&mut app, &mut frames, Key::Delete, Modifiers::NONE);
        let p = app.project.as_ref().unwrap();
        assert!(p.doc().clip(b).is_none());
        assert!(!crate::session::marker_exists(p.doc(), marker));
        assert_eq!(entries(&app), n + 1);
    }

    /// Runs the app's per-frame recorder work and its jobs until `done`.
    fn pump_recorder(
        app: &mut FreeCutApp,
        frames: &Frames,
        mut done: impl FnMut(&FreeCutApp) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(15);
        let ctx = frames.ctx.clone();
        while !done(app) {
            assert!(Instant::now() < deadline, "timed out: {:?}", app.recorder);
            app.handle_jobs(&ctx);
            app.recorder_frame(&ctx);
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    /// The talking-head project saved in `dir`, with the recorder open and
    /// its (synthetic) microphone ready.
    fn recording_app(frames: &Frames, dir: &std::path::Path) -> (FreeCutApp, [ClipId; 3]) {
        use crate::timeline_commands::tests::{s, talking_head};
        freecut_media::init().unwrap();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let (mut project, clips) = talking_head();
        project.file = Some(dir.join("Talk.freecut"));
        project.playhead = s(1);
        app.project = Some(project);
        let mut prefs = app.settings.settings().clone();
        prefs.voiceover.countdown_secs = 0;
        app.settings.set(prefs, Instant::now());
        app.open_recorder();
        pump_recorder(&mut app, frames, |a| {
            a.recorder.phase() == &crate::ui::recorder_panel::Phase::Ready
        });
        (app, clips)
    }

    /// AU-027: while a take records, edits, undo and redo are refused with
    /// the reason; the take is placed on a free audio track at the start
    /// position as one undo entry (library item, clip and track), and one
    /// undo takes it all back.
    #[test]
    fn edits_wait_for_the_take_which_lands_as_one_entry() {
        use crate::timeline_commands::tests::s;
        use freecut_timeline::edit::OpKind;
        use freecut_timeline::model::{ClipContent, GeneratedMediaKind, MediaOrigin};

        let dir = tempfile::tempdir().unwrap();
        let mut frames = Frames::new();
        let (mut app, [a, _, _]) = recording_app(&frames, dir.path());
        let entries = |app: &FreeCutApp| app.project.as_ref().unwrap().editor.history().len();
        let before = entries(&app);
        press(&mut app, &mut frames, Key::Enter, Modifiers::NONE);
        assert!(app.recorder.running(), "Enter starts a take");
        pump_recorder(&mut app, &frames, |a| {
            a.recorder
                .elapsed()
                .is_some_and(|e| e > Tick::from_raw(s(1).raw() / 2))
        });
        assert!(app.playback.is_some(), "the timeline plays during the take");
        let p = app.project.as_mut().unwrap();
        p.selection = vec![a];
        assert_eq!(p.undo().unwrap_err(), "Stop recording to edit");
        app.delete(false);
        assert!(app.project.as_ref().unwrap().doc().clip(a).is_some());
        assert_eq!(entries(&app), before, "nothing was edited");
        press(&mut app, &mut frames, Key::Space, Modifiers::NONE);
        assert!(app.playback.is_some(), "Space is ignored while recording");

        press(&mut app, &mut frames, Key::Enter, Modifiers::NONE);
        pump_recorder(&mut app, &frames, |a| entries(a) == before + 1);
        assert!(app.playback.is_none(), "the timeline stops with the take");
        let p = app.project.as_ref().unwrap();
        assert_eq!(p.edit_lock, None);
        let label = p.editor.history().undo_label().unwrap();
        assert_eq!(label.op, OpKind::InsertRecordedClip);
        let clip = p.doc().clip(p.selection[0]).unwrap();
        // The first sample came in a moment before the timeline started;
        // the clip lands on the frame grid at the start position.
        let frame = p.doc().settings().frame_rate.frame_ticks();
        assert!(
            clip.start().abs_diff(s(1)) <= frame.raw().unsigned_abs(),
            "{:?}",
            clip.start()
        );
        let ClipContent::Media { asset, .. } = clip.content else {
            panic!("a media clip");
        };
        let item = p.doc().library().get(asset).unwrap();
        assert_eq!(
            item.origin,
            MediaOrigin::Generated {
                kind: GeneratedMediaKind::Voiceover
            }
        );
        let file = p.media_paths.get(&asset).unwrap().clone();
        assert!(file.starts_with(dir.path().join("Talk Media").join("Recordings")));
        let seq = p.doc().sequence(p.doc().root()).unwrap();
        assert!(
            seq.audio_tracks()
                .iter()
                .any(|t| t.clips().iter().any(|c| c.id == clip.id))
        );
        let take = clip.id;
        app.undo();
        let p = app.project.as_ref().unwrap();
        assert!(p.doc().clip(take).is_none());
        assert!(
            p.doc().library().get(asset).is_none(),
            "one undo: clip and item"
        );
        assert!(file.exists(), "undo never deletes the recording");
    }

    /// AU-027: Esc during a take keeps the file in the Takes list, not
    /// placed and with no history entry; *Place* puts it where it was meant
    /// to go.
    #[test]
    fn a_cancelled_take_waits_in_the_takes_list() {
        use crate::timeline_commands::tests::s;
        let dir = tempfile::tempdir().unwrap();
        let mut frames = Frames::new();
        let (mut app, _) = recording_app(&frames, dir.path());
        let entries = |app: &FreeCutApp| app.project.as_ref().unwrap().editor.history().len();
        let before = entries(&app);
        app.start_take(None);
        pump_recorder(&mut app, &frames, |a| {
            a.recorder
                .elapsed()
                .is_some_and(|e| e > Tick::from_raw(s(1).raw() / 2))
        });
        press(&mut app, &mut frames, Key::Escape, Modifiers::NONE);
        pump_recorder(&mut app, &frames, |a| !a.recorder.loose_takes().is_empty());
        assert!(!app.recorder.running());
        assert_eq!(entries(&app), before, "not placed");
        let kept = app.recorder.loose_takes()[0].path.clone();
        assert!(kept.exists());
        assert!(app.recorder.open, "Esc ended the take, not the panel");

        app.place_loose_take(&kept);
        pump_recorder(&mut app, &frames, |a| entries(a) == before + 1);
        assert!(
            app.recorder.loose_takes().is_empty(),
            "placed, so no longer loose"
        );
        // Esc with no take closes the panel.
        press(&mut app, &mut frames, Key::Escape, Modifiers::NONE);
        assert!(!app.recorder.open);
    }

    /// TL-014: `S` toggles hover skim, the preference reaches the settings,
    /// and the player renders the skimmed time instead of the playhead.
    #[test]
    fn s_toggles_hover_skim_and_the_player_shows_the_skimmed_frame() {
        use crate::timeline_commands::tests::{s, talking_head};
        let mut frames = Frames::new();
        let mut app = FreeCutApp::with_sound(
            &frames.ctx,
            Sound::without_device(),
            FontRegistry::bundled(),
        );
        let (project, _) = talking_head();
        app.project = Some(project);
        press(&mut app, &mut frames, Key::S, Modifiers::NONE);
        assert!(app.timeline.skim.is_on());
        app.sync_preferences();
        assert!(
            app.settings.settings().timeline.skim.is_on(),
            "saved per user"
        );
        app.skim_shown = Some(s(3));
        let ctx = frames.ctx.clone();
        app.update_preview(&ctx);
        assert_eq!(app.preview_key.as_ref().unwrap().time, s(3));
        assert_eq!(
            app.project.as_ref().unwrap().playhead,
            Tick::ZERO,
            "the playhead stays"
        );
        app.skim_shown = None;
        app.update_preview(&ctx);
        assert_eq!(app.preview_key.as_ref().unwrap().time, Tick::ZERO);
        press(&mut app, &mut frames, Key::S, Modifiers::NONE);
        assert!(!app.timeline.skim.is_on());
    }
}

//! The Export dialog (EX-021) and what it turns into: the progress view
//! (EX-022), the result view (EX-039) and the failure view.
//!
//! - **Settings:** name and folder (EX-023), resolution (EX-024), frame
//!   rate (EX-026, MVP: the project rate only), quality (EX-027), audio
//!   (EX-031), the read-only H.264/MP4 and Rec.709 rows, the size estimate
//!   and the preflight line (EX-036). Problems disable Export; warnings ask
//!   once. An existing file asks Replace or Keep both (EX-023).
//! - **Running:** phase, bar, frames, elapsed, ETA, speed; Cancel, and
//!   "Keep editing", which hides the dialog while the job runs on its
//!   snapshot (the status bar keeps showing it).
//! - **Finished:** file name, size, duration, resolution, rate, codec, how
//!   long it took; Show in folder, Open, Copy path, Done. No share or upload
//!   buttons, ever (EX-039, EX-040).
//!
//! Everything blocking (folder checks, free space, the encoder probe, the
//! OS opener) is asked of the app as an [`ExportAction`] and runs on the
//! worker pool; the job itself runs on its own threads.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Align, Id, Layout, RichText, TextureHandle};
use freecut_project::export::{
    AAC_KBPS_CHOICES, AUDIO_SAMPLE_RATES, ConflictChoice, ExportSettings, FolderStatus, Issue,
    IssueCode, IssueTarget, OutputSize, PreflightEnv, QualityTier, Severity, ShortSide,
    estimate_size, has_problems, output_dimensions, sanitize_file_stem, tier_kbps, validate,
};
use freecut_render::FontRegistry;
use freecut_render::text::ScanState;
use freecut_timeline::ids::{ClipId, TrackId};
use freecut_timeline::model::{Document, Sequence};
use freecut_timeline::time::{FrameRate, SampleRate, TICKS_PER_SECOND, Tick};

use crate::jobs::export::{ExportEnd, ExportFailure, ExportJob, ExportResult, FixAction};
use crate::jobs::platform::{Destination, OpenAction};
use crate::jobs::progress::{EtaWindow, Phase, format_bytes, format_clock, format_speed};
use crate::session::Project;
use crate::theme::{self, icons, motion};
use crate::widgets::select::{Select, SelectOption};
use crate::widgets::sheet::{self, ButtonKind, Sheet, SheetUi};

/// A cancel after this long asks first (EX-022).
const CONFIRM_CANCEL_AFTER: Duration = Duration::from_secs(30);

/// What the dialog asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportAction {
    /// Start the export with the dialog's settings (a new snapshot).
    Start,
    /// Start it again, drawing the error slate for pictures that can't be
    /// read (EX-022 "Retry using slates for unreadable frames").
    StartWithSlates,
    /// Open the OS folder picker for the destination.
    ChooseFolder,
    /// Check the folder and the file the name would be saved as.
    Probe { folder: PathBuf, name: String },
    /// Find out whether the H.264 and AAC encoders exist.
    ProbeEncoder,
    /// "Open" or "Show in folder" on the finished file.
    Os(OpenAction, PathBuf),
    /// Select clips a preflight issue is about.
    Select(Vec<ClipId>),
    /// Move the playhead (the Preview section's seek track).
    Seek(Tick),
}

/// Where the dialog is.
#[derive(Debug)]
pub enum Stage {
    Settings,
    Running(Box<ExportJob>),
    Finished(ExportResult),
    Failed(ExportFailure),
}

/// The dialog's state, kept by the app across frames.
#[derive(Debug)]
pub struct ExportDialog {
    /// Shown. A running export continues while hidden.
    pub visible: bool,
    pub stage: Stage,
    pub settings: ExportSettings,
    pub conflict: ConflictChoice,
    /// The latest destination check, and what it was asked for.
    dest: Option<Destination>,
    dest_for: Option<(PathBuf, String)>,
    /// `None` until the background probe answers.
    pub encoder: Option<bool>,
    encoder_asked: bool,
    issues: Vec<Issue>,
    issues_key: Option<IssuesKey>,
    confirm_warnings: bool,
    confirm_cancel: bool,
    show_issues: bool,
    eta: EtaWindow,
    /// "Export as heard" (AU-012): export only the soloed tracks. Off each
    /// time the dialog opens; not saved with the export settings.
    pub as_heard: bool,
    /// The settings (and the running or ended job) belong to a project that
    /// is no longer open: they can't be used for the open one without being
    /// confirmed again, so Retry isn't offered and the dialog doesn't go
    /// back to its settings.
    other_project: bool,
}

/// What the cached preflight result was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IssuesKey {
    revision: u64,
    settings: ExportSettings,
    dest: Option<Destination>,
    encoder: Option<bool>,
    offline: usize,
    fonts: u64,
}

impl ExportDialog {
    /// The dialog for `project`, open (see [`ExportDialog::open`]).
    pub fn new(project: &Project, default_folder: Option<PathBuf>) -> ExportDialog {
        let mut dialog = ExportDialog {
            visible: false,
            stage: Stage::Settings,
            settings: ExportSettings::defaults(&project_name(project), None, project.doc()),
            conflict: ConflictChoice::KeepBoth,
            dest: None,
            dest_for: None,
            encoder: None,
            encoder_asked: false,
            issues: Vec::new(),
            issues_key: None,
            confirm_warnings: false,
            confirm_cancel: false,
            show_issues: false,
            eta: EtaWindow::default(),
            as_heard: false,
            other_project: false,
        };
        dialog.open(project, default_folder);
        dialog
    }

    pub fn is_running(&self) -> bool {
        matches!(self.stage, Stage::Running(_))
    }

    /// The tracks the export plays (AU-012): `None`, every track whatever
    /// is soloed, unless "Export as heard" is ticked while tracks are
    /// soloed; then only those.
    pub fn export_solo(&self, project: &Project) -> Option<BTreeSet<TrackId>> {
        (self.as_heard && !project.solo().is_empty()).then(|| project.solo().clone())
    }

    /// Another project was opened while this dialog's export runs: its
    /// settings stay with the job, and the next [`ExportDialog::open`]
    /// takes the open project's.
    pub fn project_closed(&mut self) {
        self.other_project = true;
    }

    /// The settings can be exported for the open project (not those of a
    /// project closed since, see [`ExportDialog::project_closed`]).
    pub fn settings_current(&self) -> bool {
        !self.other_project
    }

    /// Opens the dialog (File › Export…, ⌘E, the Export button). A running
    /// export shows its progress; otherwise the settings are the project's
    /// last ones (EX-021), or the first-export defaults.
    pub fn open(&mut self, project: &Project, default_folder: Option<PathBuf>) {
        self.visible = true;
        match self.stage {
            Stage::Running(_) => return,
            Stage::Finished(_) | Stage::Failed(_) => self.stage = Stage::Settings,
            Stage::Settings => {}
        }
        // AU-012: "Export as heard" is off each time the dialog opens.
        self.as_heard = false;
        self.other_project = false;
        let name = project_name(project);
        self.settings = match &project.export {
            Some(last) => {
                let mut s = last.clone();
                if s.file.folder.is_none() {
                    s.file.folder = default_folder;
                }
                s
            }
            None => ExportSettings::defaults(&name, default_folder, project.doc()),
        };
        self.conflict = ConflictChoice::KeepBoth;
        self.dest = None;
        self.dest_for = None;
        self.issues_key = None;
        self.confirm_warnings = false;
        self.confirm_cancel = false;
    }

    /// The destination check for `key` finished. Stale answers (the folder
    /// or name changed since) are ignored.
    pub fn destination_checked(&mut self, key: (PathBuf, String), dest: Destination) {
        if self.dest_for.as_ref() == Some(&key) {
            self.dest = Some(dest);
        }
    }

    /// The folder picked with the OS dialog.
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.settings.file.folder = Some(folder);
        self.confirm_warnings = false;
    }

    /// The job was started.
    pub fn started(&mut self, job: ExportJob) {
        self.eta = EtaWindow::default();
        self.confirm_cancel = false;
        self.confirm_warnings = false;
        self.stage = Stage::Running(Box::new(job));
    }

    /// Polls the running job. Returns how it ended, once.
    pub fn poll(&mut self) -> Option<ExportEnd> {
        let Stage::Running(job) = &mut self.stage else {
            return None;
        };
        let p = job.progress();
        self.eta.push(job.started().elapsed(), p.frames_muxed);
        let end = job.poll()?;
        self.stage = match &end {
            ExportEnd::Done(result) => Stage::Finished(result.clone()),
            ExportEnd::Failed(failure) => Stage::Failed(failure.clone()),
            ExportEnd::Cancelled => Stage::Settings,
        };
        // A probe from before the export is stale now (a file was written).
        self.dest_for = None;
        Some(end)
    }

    /// The status bar item while the dialog is hidden: "Exporting 42 %",
    /// "Export finished". Clicking it shows the dialog.
    pub fn status_item(&mut self, ui: &mut egui::Ui) {
        if self.visible {
            return;
        }
        let text = match &self.stage {
            Stage::Settings => return,
            Stage::Running(job) => {
                let p = job.progress();
                let mut text = format!("Exporting {:.0} %", p.fraction() * 100.0);
                if let Some(eta) =
                    self.eta
                        .eta(job.started().elapsed(), p.frames_muxed, p.frames_total)
                {
                    text.push_str(&format!(" · {} left", format_clock(eta)));
                }
                text
            }
            Stage::Finished(_) => "Export finished".to_owned(),
            Stage::Failed(_) => "Export failed".to_owned(),
        };
        if ui
            .add(egui::Button::new(format!("{text} — Show")).small())
            .clicked()
        {
            self.visible = true;
        }
    }

    /// Draws the sheet when visible and returns what the user asked for.
    /// `preview` is the Stage's frame, shown in the Preview section.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        project: &Project,
        fonts: &FontRegistry,
        preview: Option<&TextureHandle>,
    ) -> Vec<ExportAction> {
        let mut actions = Vec::new();
        if self.other_project && matches!(self.stage, Stage::Settings) {
            // The old project's export was cancelled: its settings are not
            // shown for the open project. Export opens again with its own.
            self.visible = false;
        }
        let slide_id = Id::new("export-sheet-slide");
        let t =
            ctx.animate_bool_with_time(slide_id, self.visible, motion::duration(ctx, motion::SLOW));
        if !self.visible {
            return actions;
        }
        if self.encoder.is_none() && !self.encoder_asked {
            self.encoder_asked = true;
            actions.push(ExportAction::ProbeEncoder);
        }
        if matches!(self.stage, Stage::Settings) {
            self.prepare_settings(project, fonts, &mut actions);
        }
        let window = ctx.content_rect();
        let width = export_sheet_width(window.width());
        let columns = export_columns(window.width());
        let eased = motion::ease_out(t);
        let running = self.is_running();
        let max_body = window.height() - 48.0 - sheet::LARGE_HEADER - 96.0;
        let facts = Facts::of(self, project);
        let response = Sheet::new(Id::new("export-dialog"), "Export")
            .icon(icons::EXPORT)
            .width(width)
            .header_height(sheet::LARGE_HEADER)
            // The close button is absent while an export runs.
            .closable(!running)
            .slide((1.0 - eased) * 48.0, 0.4 + 0.6 * eased)
            .show(ctx, |s| {
                s.scroll_body(max_body, |ui| {
                    let room = max_body - sheet::body_margin(&crate::theme::look(ui.ctx())).sum().y;
                    self.body(ui, columns, room, project, preview, &facts, &mut actions);
                });
                s.ui.spacing_mut().item_spacing.y = 0.0;
                self.footer(s, project, &facts, &mut actions);
            });
        if response.close || (running && ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
            // Esc or a click outside: hide; a running export continues.
            self.visible = false;
            self.confirm_cancel = false;
            if matches!(self.stage, Stage::Finished(_) | Stage::Failed(_)) {
                self.stage = Stage::Settings;
            }
        }
        actions
    }

    /// Asks for a destination check whenever the folder or name changed,
    /// and refreshes the preflight.
    fn prepare_settings(
        &mut self,
        project: &Project,
        fonts: &FontRegistry,
        actions: &mut Vec<ExportAction>,
    ) {
        if let Some(folder) = &self.settings.file.folder {
            let key = (folder.clone(), self.settings.file.name.clone());
            if self.dest_for.as_ref() != Some(&key) {
                // Only the name changed: the folder facts still hold, but
                // whether the file exists is asked again. Until the answer
                // comes, Keep both (below) can never overwrite anything.
                let same_folder = self.dest_for.as_ref().is_some_and(|(f, _)| f == folder);
                self.dest = self
                    .dest
                    .take()
                    .filter(|_| same_folder)
                    .map(|d| Destination {
                        target_exists: false,
                        keep_both: None,
                        ..d
                    });
                self.dest_for = Some(key.clone());
                // Replace was chosen for another file: ask again.
                self.conflict = ConflictChoice::KeepBoth;
                actions.push(ExportAction::Probe {
                    folder: key.0,
                    name: key.1,
                });
            }
        } else {
            self.dest = None;
            self.dest_for = None;
        }
        self.refresh_issues(project, fonts);
    }

    /// The sheet's columns: what goes where depends on the stage and on
    /// how many columns fit. The preview gives up height so the Summary
    /// under it fits in the body's `room` without scrolling.
    #[allow(clippy::too_many_arguments)]
    fn body(
        &mut self,
        ui: &mut egui::Ui,
        columns: usize,
        room: f32,
        project: &Project,
        preview: Option<&TextureHandle>,
        facts: &Facts,
        actions: &mut Vec<ExportAction>,
    ) {
        use Part as P;
        let layout: Vec<Vec<Part>> = match (&self.stage, columns) {
            (Stage::Settings, 3) => vec![
                vec![P::Preview, P::Summary],
                vec![P::Output, P::Audio, P::File],
                vec![P::Quality, P::Checks],
            ],
            (Stage::Settings, 2) => vec![
                vec![P::Preview, P::Summary, P::Quality],
                vec![P::Output, P::Audio, P::File, P::Checks],
            ],
            (Stage::Settings, _) => vec![vec![
                P::Preview,
                P::Summary,
                P::File,
                P::Output,
                P::Audio,
                P::Quality,
                P::Checks,
            ]],
            (_, 1) => vec![vec![P::Status, P::Preview, P::Summary]],
            _ => vec![vec![P::Preview, P::Summary], vec![P::Status]],
        };
        let n = layout.len() as f32;
        let gap = 24.0;
        let col_w = ((ui.available_width() - (n - 1.0) * gap) / n).max(160.0);
        let cap_id = Id::new("export-preview-cap");
        let cap = ui
            .data(|d| d.get_temp::<f32>(cap_id))
            .unwrap_or(PREVIEW_MAX);
        let mut summary_end = None;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for parts in &layout {
                ui.allocate_ui_with_layout(
                    egui::vec2(col_w, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(col_w);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let top = ui.cursor().top();
                        for (i, part) in parts.iter().enumerate() {
                            if i > 0 {
                                ui.add_space(10.0);
                                sheet::hairline(ui);
                                ui.add_space(4.0);
                            }
                            self.part(ui, *part, project, preview, cap, facts, actions);
                            if *part == Part::Summary && parts.contains(&Part::Preview) {
                                summary_end = Some(ui.min_rect().bottom() - top);
                            }
                        }
                    },
                );
            }
        });
        if let Some(used) = summary_end {
            let next = next_preview_cap(cap, used, room);
            if (next - cap).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(cap_id, next));
                ui.ctx().request_repaint();
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn part(
        &mut self,
        ui: &mut egui::Ui,
        part: Part,
        project: &Project,
        preview: Option<&TextureHandle>,
        preview_cap: f32,
        facts: &Facts,
        actions: &mut Vec<ExportAction>,
    ) {
        let id = Id::new("export-part").with(part as u8);
        match part {
            Part::Preview => {
                sheet::section_title(ui, id, "Preview", None, |_| {});
                preview_section(ui, project, preview, preview_cap, facts, actions);
            }
            Part::Summary => {
                sheet::section_title(ui, id, "Summary", None, |_| {});
                sheet::fact_list(ui, &facts.summary(self));
            }
            Part::Output => {
                sheet::section_title(
                    ui,
                    id,
                    "Video",
                    Some(
                        "The picture size and rate of the file. Smaller sizes make smaller \
                         files; the canvas size keeps every pixel.",
                    ),
                    |_| {},
                );
                self.output_section(ui, facts);
            }
            Part::Audio => {
                let mut include = self.settings.audio.include;
                sheet::section_title(
                    ui,
                    id,
                    "Audio",
                    Some("AAC sound in the MP4. Turn it off for a silent file."),
                    |ui| {
                        crate::widgets::toggle::toggle(
                            ui,
                            id.with("include"),
                            &mut include,
                            "Include audio",
                        )
                        .on_hover_text("Include audio");
                    },
                );
                self.settings.audio.include = include;
                self.audio_section(ui, project);
            }
            Part::File => {
                sheet::section_title(ui, id, "File", None, |_| {});
                self.file_section(ui, actions);
            }
            Part::Quality => {
                sheet::section_title(
                    ui,
                    id,
                    "Quality",
                    Some(
                        "A higher bitrate keeps more detail and makes a bigger file; a \
                         lower one makes a smaller file with more visible compression.",
                    ),
                    |_| {},
                );
                self.quality_section(ui, project, facts);
            }
            Part::Checks => {
                sheet::section_title(ui, id, "Checks", None, |_| {});
                self.checks_section(ui, actions);
            }
            Part::Status => self.status_section(ui, project),
        }
    }

    fn output_section(&mut self, ui: &mut egui::Ui, facts: &Facts) {
        let mut sizes = vec![OutputSize::Canvas];
        sizes.extend(ShortSide::ALL.map(OutputSize::Preset));
        let labels: Vec<String> = sizes.iter().map(|s| size_label(facts.canvas, *s)).collect();
        let options: Vec<SelectOption<'_>> = labels.iter().map(|l| SelectOption::new(l)).collect();
        let current = sizes.iter().position(|s| *s == self.settings.video.size);
        form_line(ui, "Resolution", |ui| {
            let out = Select::new(Id::new("export-size"), &options, current)
                .fill_width()
                .show(ui);
            if let Some(i) = out.picked {
                self.settings.video.size = sizes[i];
            }
        });
        form_line(ui, "Frame rate", |ui| {
            // The MVP exports at the project rate only (EX-026).
            value_text(
                ui,
                &format!("Project ({} fps)", rate_label(facts.project_rate)),
            );
        });
        form_line(ui, "Format", |ui| value_text(ui, "H.264 video in MP4"));
        form_line(ui, "Color", |ui| value_text(ui, "Rec.709 (SDR)"));
    }

    fn audio_section(&mut self, ui: &mut egui::Ui, project: &Project) {
        let doc = project.doc();
        // The same rule as the first-export default (EX-031).
        let silent = !ExportSettings::defaults("", None, doc).audio.include;
        if silent {
            note_text(ui, "The timeline is silent.");
        }
        ui.add_enabled_ui(self.settings.audio.include, |ui| {
            let kbps_labels: Vec<String> = AAC_KBPS_CHOICES
                .iter()
                .map(|k| format!("{k} kbps"))
                .collect();
            let kbps_options: Vec<SelectOption<'_>> =
                kbps_labels.iter().map(|l| SelectOption::new(l)).collect();
            let kbps_now = AAC_KBPS_CHOICES
                .iter()
                .position(|k| *k == self.settings.audio.kbps);
            form_line(ui, "Bitrate", |ui| {
                let out = Select::new(Id::new("export-aac"), &kbps_options, kbps_now)
                    .fill_width()
                    .show(ui);
                if let Some(i) = out.picked {
                    self.settings.audio.kbps = AAC_KBPS_CHOICES[i];
                }
            });
            let rates: Vec<SampleRate> = AUDIO_SAMPLE_RATES
                .iter()
                .filter_map(|hz| SampleRate::new(*hz).ok())
                .collect();
            let rate_labels: Vec<String> = rates.iter().map(|r| hz_label(r.hz())).collect();
            let rate_options: Vec<SelectOption<'_>> =
                rate_labels.iter().map(|l| SelectOption::new(l)).collect();
            let rate_now = rates
                .iter()
                .position(|r| *r == self.settings.audio.sample_rate);
            form_line(ui, "Sample rate", |ui| {
                let out = Select::new(Id::new("export-rate"), &rate_options, rate_now)
                    .fill_width()
                    .show(ui);
                if let Some(i) = out.picked {
                    self.settings.audio.sample_rate = rates[i];
                }
            });
            form_line(ui, "Channels", |ui| value_text(ui, "Stereo"));
            // AU-012: solo is a monitoring aid; export ignores it unless
            // asked to export what preview plays.
            let soloed = solo_names(project);
            if !soloed.is_empty() {
                let warning = theme::look(ui.ctx()).palette.warning;
                sheet::note_box(ui, icons::INFO, warning, |ui| {
                    body_line(
                        ui,
                        &format!(
                            "Solo is on ({}). The export includes every track.",
                            soloed.join(", ")
                        ),
                    );
                    ui.checkbox(
                        &mut self.as_heard,
                        "Export as heard (only the soloed tracks)",
                    );
                });
            }
        });
    }

    fn file_section(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ExportAction>) {
        let p = theme::look(ui.ctx()).palette;
        let ext = format!(".{}", self.settings.video.container.extension());
        form_line(ui, "Name", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let w = (ui.available_width() - 40.0).max(60.0);
                let edit = egui::TextEdit::singleline(&mut self.settings.file.name)
                    .hint_text("Enter a file name")
                    .desired_width(w)
                    .margin(egui::vec2(8.0, 5.0));
                if ui.add(edit).changed() {
                    self.confirm_warnings = false;
                }
                ui.label(
                    RichText::new(&ext)
                        .text_style(egui::TextStyle::Body)
                        .color(p.text_tertiary),
                );
            });
        });
        let folder = self
            .settings
            .file
            .folder
            .as_deref()
            .map_or_else(|| "No folder chosen".to_owned(), folder_label);
        let red = matches!(
            self.dest.as_ref().map(|d| &d.folder),
            Some(
                FolderStatus::Missing
                    | FolderStatus::NotWritable
                    | FolderStatus::Disconnected { .. }
            )
        );
        form_line(ui, "Folder", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let browse_w = 96.0;
                let room = (ui.available_width() - browse_w - 8.0).max(40.0);
                let colour = if red { p.danger } else { p.text_primary };
                let font = egui::TextStyle::Body.resolve(ui.style());
                let galley = crate::widgets::side_list::one_line(ui, &folder, font, colour, room);
                let (rect, label) = ui.allocate_exact_size(
                    egui::vec2(room, theme::look(ui.ctx()).metrics.control_height),
                    egui::Sense::hover(),
                );
                ui.painter().galley(
                    egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                    galley,
                    colour,
                );
                if let Some(f) = &self.settings.file.folder {
                    label.on_hover_text(f.display().to_string());
                }
                if sheet::button(
                    ui,
                    ButtonKind::Secondary,
                    Some(icons::FOLDER_OPEN),
                    "Browse…",
                    browse_w,
                    None,
                    true,
                )
                .clicked()
                {
                    actions.push(ExportAction::ChooseFolder);
                }
            });
        });
        // An existing file: Replace or Keep both (EX-023).
        if let Some(dest) = &self.dest
            && dest.target_exists
        {
            let keep = dest
                .keep_both
                .as_deref()
                .and_then(Path::file_name)
                .map_or_else(
                    || "Keep both".to_owned(),
                    |n| format!("Keep both (save as “{}”)", n.to_string_lossy()),
                );
            let message = format!(
                "“{}.{}” already exists in this folder.",
                sanitize_file_stem(&self.settings.file.name),
                self.settings.video.container.extension()
            );
            sheet::note_box(ui, icons::WARNING, p.warning, |ui| {
                body_line(ui, &message);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                    if sheet::choice_chip(
                        ui,
                        &keep,
                        None,
                        self.conflict == ConflictChoice::KeepBoth,
                    )
                    .clicked()
                    {
                        self.conflict = ConflictChoice::KeepBoth;
                    }
                    if sheet::choice_chip(
                        ui,
                        "Replace",
                        None,
                        self.conflict == ConflictChoice::Replace,
                    )
                    .clicked()
                    {
                        self.conflict = ConflictChoice::Replace;
                    }
                });
            });
        }
    }

    fn quality_section(&mut self, ui: &mut egui::Ui, project: &Project, facts: &Facts) {
        let cards = quality_cards(&self.settings, project.doc(), facts.duration);
        let gap = 6.0;
        let n = cards.len() as f32;
        let w = ((ui.available_width() - (n - 1.0) * gap) / n).max(60.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for card in &cards {
                let tip = quality_label(card.tier, card.kbps);
                if quality_card(ui, card, w).on_hover_text(tip).clicked() {
                    self.settings.video.quality = card.tier;
                }
            }
        });
    }

    fn checks_section(&mut self, ui: &mut egui::Ui, actions: &mut Vec<ExportAction>) {
        let p = theme::look(ui.ctx()).palette;
        let checking = self.encoder.is_none() || self.dest_pending();
        if self.issues.is_empty() {
            let (icon, colour, text) = if checking {
                (
                    icons::INFO,
                    p.text_tertiary,
                    "Checking the timeline and the folder…",
                )
            } else {
                (icons::CHECK_CIRCLE, p.success, "Ready to export.")
            };
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon)
                        .font(theme::fonts::icon_font(ui.ctx(), 14.0, false))
                        .color(colour),
                );
                body_line(ui, text);
            });
            return;
        }
        for issue in &self.issues {
            let (mark, colour) = match issue.severity {
                Severity::Problem => (icons::ERROR, p.danger),
                Severity::Warning => (icons::WARNING, p.warning),
                Severity::Info => (icons::INFO, p.text_tertiary),
            };
            let clips: Vec<ClipId> = issue
                .targets
                .iter()
                .filter_map(|t| match t {
                    IssueTarget::Clip(c) => Some(*c),
                    _ => None,
                })
                .collect();
            let row = ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    RichText::new(mark)
                        .font(theme::fonts::icon_font(ui.ctx(), 14.0, false))
                        .color(colour),
                );
                let text = RichText::new(issue_text(&issue.code))
                    .text_style(egui::TextStyle::Body)
                    .color(p.text_primary);
                if clips.is_empty() {
                    ui.add(egui::Label::new(text).wrap());
                    false
                } else {
                    ui.add(egui::Label::new(text).wrap().sense(egui::Sense::click()))
                        .on_hover_text("Select these clips")
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                }
            });
            if row.inner {
                actions.push(ExportAction::Select(clips));
            }
        }
    }

    /// The running, finished or failed export's details.
    fn status_section(&mut self, ui: &mut egui::Ui, project: &Project) {
        let p = theme::look(ui.ctx()).palette;
        match &self.stage {
            Stage::Settings => {}
            Stage::Running(job) => {
                sheet::section_title(ui, Id::new("export-status"), "Exporting", None, |_| {});
                let prog = job.progress();
                let elapsed = job.started().elapsed();
                let phase = if job.is_cancelling() {
                    "Stopping…"
                } else {
                    prog.phase.label()
                };
                let mut facts = vec![
                    ("Step", phase.to_owned()),
                    (
                        "Frame",
                        format!(
                            "{} of {}",
                            prog.frames_muxed.min(prog.frames_total),
                            prog.frames_total
                        ),
                    ),
                    ("Elapsed", format_clock(elapsed)),
                ];
                if prog.phase == Phase::Rendering {
                    if let Some(eta) = self.eta.eta(elapsed, prog.frames_muxed, prog.frames_total) {
                        facts.push(("Left", format!("about {}", format_clock(eta))));
                    }
                    let fps = f64::from(job.frame_rate().num()) / f64::from(job.frame_rate().den());
                    if elapsed >= crate::jobs::progress::ETA_DELAY
                        && let Some(speed) = self.eta.speed(fps)
                    {
                        facts.push(("Speed", format_speed(speed)));
                    }
                }
                sheet::fact_list(ui, &facts);
                note_text(
                    ui,
                    "You can keep editing: the export works on the timeline as it was \
                     when it started.",
                );
            }
            Stage::Finished(result) => {
                sheet::section_title(
                    ui,
                    Id::new("export-status"),
                    "Export finished",
                    None,
                    |_| {},
                );
                let name = result
                    .path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(icons::CHECK_CIRCLE)
                            .font(theme::fonts::icon_font(ui.ctx(), 16.0, false))
                            .color(p.success),
                    );
                    ui.label(
                        RichText::new(name)
                            .text_style(theme::Type::Label.style(ui.style()))
                            .color(p.text_primary),
                    );
                });
                for line in result_lines(result) {
                    body_line(ui, &line);
                }
                note_text(
                    ui,
                    "Saved on this computer. FreeCut adds no watermark and uploads nothing.",
                );
            }
            Stage::Failed(failure) => {
                sheet::section_title(ui, Id::new("export-status"), "Export failed", None, |_| {});
                let message = failure.message();
                let code = failure.code.id();
                let details = failure.details(&self.settings, project.doc());
                sheet::note_box(ui, icons::ERROR, p.danger, |ui| {
                    body_line(ui, &message);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(code)
                                .text_style(egui::TextStyle::Monospace)
                                .color(p.text_tertiary),
                        );
                        if sheet::choice_chip(ui, "Copy details", None, false).clicked() {
                            ui.ctx().copy_text(details.clone());
                        }
                    });
                });
                if self.other_project {
                    note_text(
                        ui,
                        "The project of this export is closed. Open Export again to export \
                         this one.",
                    );
                }
            }
        }
    }

    /// The footer: the primary button and the stage's state.
    fn footer(
        &mut self,
        s: &mut SheetUi<'_>,
        project: &Project,
        facts: &Facts,
        actions: &mut Vec<ExportAction>,
    ) {
        let p = theme::look(s.ui.ctx()).palette;
        // Running: the 4 pt bar runs across the sheet above the buttons.
        if let Stage::Running(job) = &self.stage {
            let fraction = job.progress().fraction();
            let pad = sheet::body_margin(&theme::look(s.ui.ctx()));
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    top: 0,
                    bottom: 0,
                    ..pad
                })
                .show(s.ui, |ui| {
                    sheet::progress_bar(ui, fraction, false);
                });
        }
        if matches!(self.stage, Stage::Finished(_)) {
            let pad = sheet::body_margin(&theme::look(s.ui.ctx()));
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    top: 0,
                    bottom: 0,
                    ..pad
                })
                .show(s.ui, |ui| sheet::progress_bar(ui, 1.0, true));
        }
        let warnings: Vec<&Issue> = self
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .collect();
        let warning_count = warnings.len();
        let mut hide = false;
        let mut fix = None;
        let mut done = false;
        s.footer(|ui| {
            let large = Some(sheet::PRIMARY_LARGE);
            match &self.stage {
                Stage::Settings if self.confirm_warnings => {
                    if sheet::button(
                        ui,
                        ButtonKind::Primary,
                        None,
                        "Export anyway",
                        160.0,
                        large,
                        true,
                    )
                    .clicked()
                    {
                        actions.push(ExportAction::Start);
                    }
                    if sheet::button(ui, ButtonKind::Secondary, None, "Back", 80.0, large, true)
                        .clicked()
                    {
                        self.confirm_warnings = false;
                    }
                    footer_note(
                        ui,
                        p.warning,
                        &if warning_count == 1 {
                            "Export anyway? 1 warning is listed under Checks.".to_owned()
                        } else {
                            format!(
                                "Export anyway? {warning_count} warnings are listed under Checks."
                            )
                        },
                    );
                }
                Stage::Settings => {
                    let ready = self.is_ready();
                    let label = export_button_label(Some(facts.estimate));
                    let export = sheet::button(
                        ui,
                        ButtonKind::Primary,
                        Some(icons::EXPORT),
                        &label,
                        280.0,
                        large,
                        ready,
                    )
                    .on_disabled_hover_text(disabled_reason(&self.issues));
                    if export.clicked() {
                        if warning_count == 0 {
                            actions.push(ExportAction::Start);
                        } else {
                            self.confirm_warnings = true;
                        }
                    }
                    if sheet::button(ui, ButtonKind::Ghost, None, "Cancel", 80.0, large, true)
                        .clicked()
                    {
                        hide = true;
                    }
                    let line =
                        preflight_line(&self.issues, self.encoder.is_none() || self.dest_pending());
                    let colour = if has_problems(&self.issues) {
                        p.danger
                    } else if warning_count > 0 {
                        p.warning
                    } else {
                        p.text_tertiary
                    };
                    if footer_note(ui, colour, &line).clicked() && !self.issues.is_empty() {
                        self.show_issues = !self.show_issues;
                    }
                }
                Stage::Running(job) => {
                    let elapsed = job.started().elapsed();
                    if self.confirm_cancel {
                        if sheet::button(
                            ui,
                            ButtonKind::Secondary,
                            None,
                            "Keep exporting",
                            0.0,
                            large,
                            true,
                        )
                        .clicked()
                        {
                            self.confirm_cancel = false;
                        }
                        if sheet::button(
                            ui,
                            ButtonKind::Danger,
                            None,
                            "Stop export",
                            0.0,
                            large,
                            true,
                        )
                        .clicked()
                        {
                            job.cancel();
                            self.confirm_cancel = false;
                        }
                        footer_note(
                            ui,
                            p.text_secondary,
                            "Stop the export? The partial file is deleted.",
                        );
                    } else {
                        if sheet::button(
                            ui,
                            ButtonKind::Primary,
                            None,
                            "Keep editing",
                            140.0,
                            large,
                            true,
                        )
                        .on_hover_text("Hide this window; the export continues")
                        .clicked()
                        {
                            hide = true;
                        }
                        if sheet::button(
                            ui,
                            ButtonKind::Ghost,
                            None,
                            "Cancel export",
                            0.0,
                            large,
                            !job.is_cancelling(),
                        )
                        .clicked()
                        {
                            if elapsed > CONFIRM_CANCEL_AFTER {
                                self.confirm_cancel = true;
                            } else {
                                job.cancel();
                            }
                        }
                        let prog = job.progress();
                        let mut text = format!("Exporting {:.0} %", prog.fraction() * 100.0);
                        if let Some(eta) =
                            self.eta.eta(elapsed, prog.frames_muxed, prog.frames_total)
                        {
                            text.push_str(&format!(" · about {} left", format_clock(eta)));
                        }
                        footer_note(ui, p.text_secondary, &text);
                    }
                }
                Stage::Finished(result) => {
                    let path = result.path.clone();
                    if sheet::button(ui, ButtonKind::Primary, None, "Done", 120.0, large, true)
                        .clicked()
                    {
                        done = true;
                    }
                    if sheet::button(
                        ui,
                        ButtonKind::Ghost,
                        Some(icons::COPY),
                        "Copy path",
                        0.0,
                        large,
                        true,
                    )
                    .clicked()
                    {
                        ui.ctx().copy_text(path.display().to_string());
                    }
                    if sheet::button(
                        ui,
                        ButtonKind::Secondary,
                        Some(icons::OPEN_OUT),
                        "Open",
                        0.0,
                        large,
                        true,
                    )
                    .clicked()
                    {
                        actions.push(ExportAction::Os(OpenAction::Open, path.clone()));
                    }
                    if sheet::button(
                        ui,
                        ButtonKind::Secondary,
                        Some(icons::FOLDER_OPEN),
                        "Show in folder",
                        0.0,
                        large,
                        true,
                    )
                    .clicked()
                    {
                        actions.push(ExportAction::Os(OpenAction::Reveal, path));
                    }
                    footer_note(
                        ui,
                        p.success,
                        &format!("Exported · {}", format_bytes(result.bytes)),
                    );
                }
                Stage::Failed(failure) => {
                    let fixes = self.offered_fixes(failure);
                    for (i, f) in fixes.iter().enumerate() {
                        let label = match f {
                            FixAction::ChooseFolder => "Choose folder…",
                            FixAction::Retry => "Retry",
                            FixAction::RetryWithSlates => {
                                "Retry using slates for unreadable frames"
                            }
                        };
                        let kind = if i == 0 {
                            ButtonKind::Primary
                        } else {
                            ButtonKind::Secondary
                        };
                        if sheet::button(ui, kind, None, label, 0.0, large, true).clicked() {
                            fix = Some(*f);
                        }
                    }
                    if sheet::button(ui, ButtonKind::Secondary, None, "Close", 80.0, large, true)
                        .clicked()
                    {
                        done = true;
                    }
                    footer_note(ui, p.danger, "The export stopped.");
                }
            }
        });
        let _ = project;
        if hide {
            self.visible = false;
        }
        if done {
            self.visible = false;
            self.stage = Stage::Settings;
        }
        if let Some(f) = fix {
            actions.extend(self.fix_chosen(f));
        }
    }

    /// Export may be pressed: the checks are back and nothing blocks.
    pub fn is_ready(&self) -> bool {
        matches!(self.stage, Stage::Settings)
            && self.issues_key.is_some()
            && !has_problems(&self.issues)
            && self.encoder == Some(true)
            && !self.dest_pending()
    }

    /// A destination check is on its way.
    fn dest_pending(&self) -> bool {
        self.settings.file.folder.is_some() && self.dest.is_none()
    }

    fn refresh_issues(&mut self, project: &Project, fonts: &FontRegistry) {
        let key = IssuesKey {
            revision: project.editor.revision(),
            settings: self.settings.clone(),
            dest: self.dest.clone(),
            encoder: self.encoder,
            offline: project.offline.len(),
            fonts: fonts.generation(),
        };
        if self.issues_key.as_ref() == Some(&key) {
            return;
        }
        let doc = project.doc();
        let env = PreflightEnv {
            folder: self
                .dest
                .as_ref()
                .map_or(FolderStatus::Unknown, |d| d.folder.clone()),
            free_bytes: self.dest.as_ref().and_then(|d| d.free_bytes),
            volume: self.dest.as_ref().and_then(|d| d.volume.clone()),
            // The CPU renderer has no texture limit; 8192 is the encoder's.
            max_texture_side: None,
            // Unknown until the probe answers; Export waits for it.
            encoder_available: self.encoder.unwrap_or(true),
            offline_media: project.offline.clone(),
            substituted_fonts: substituted_fonts(doc, fonts),
        };
        self.issues = validate(&self.settings, doc, doc.root(), &env);
        self.issues_key = Some(key);
    }

    /// The fixes the failure view offers: none for the export of a project
    /// that is closed now (only Copy details and Close).
    fn offered_fixes(&self, failure: &ExportFailure) -> &'static [FixAction] {
        if self.other_project {
            &[]
        } else {
            failure.code.fixes()
        }
    }

    /// A fix button of the failure view was pressed: back to the settings,
    /// and what the app should do.
    fn fix_chosen(&mut self, fix: FixAction) -> Option<ExportAction> {
        if self.other_project {
            return None;
        }
        self.stage = Stage::Settings;
        Some(match fix {
            FixAction::ChooseFolder => ExportAction::ChooseFolder,
            FixAction::Retry => ExportAction::Start,
            FixAction::RetryWithSlates => ExportAction::StartWithSlates,
        })
    }
}

/// The export sheet's width in a window `window_w` wide (§13.28).
pub fn export_sheet_width(window_w: f32) -> f32 {
    (window_w - 48.0).min(1160.0).max(200.0_f32.min(window_w))
}

/// How many columns the export sheet has: 3 from a 1100 pt window, 2 from
/// 760 pt, else 1.
pub fn export_columns(window_w: f32) -> usize {
    if window_w >= 1100.0 {
        3
    } else if window_w >= 760.0 {
        2
    } else {
        1
    }
}

/// The primary button's label: "Export · ≈ 120 MB".
pub fn export_button_label(estimate: Option<u64>) -> String {
    match estimate {
        Some(bytes) if bytes > 0 => format!("Export · ≈ {}", format_bytes(bytes)),
        _ => "Export".to_owned(),
    }
}

/// One of the three quality cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityCard {
    pub tier: QualityTier,
    pub name: &'static str,
    /// "20 Mbps".
    pub note: String,
    pub kbps: u32,
    /// The size estimate at this tier.
    pub bytes: u64,
    pub current: bool,
}

/// The quality cards for `settings` (EX-027): Higher quality,
/// Recommended, Smaller file, each with its bitrate and size estimate.
pub fn quality_cards(
    settings: &ExportSettings,
    doc: &Document,
    duration: Tick,
) -> Vec<QualityCard> {
    let canvas = (doc.settings().width, doc.settings().height);
    let (w, h) = output_dimensions(canvas, settings.video.size);
    let rate = settings.output_rate(doc.settings().frame_rate);
    [
        QualityTier::Higher,
        QualityTier::Recommended,
        QualityTier::Smaller,
    ]
    .into_iter()
    .map(|tier| {
        let kbps = tier_kbps(tier, w, h, rate, settings.video.codec);
        let mut at = settings.clone();
        at.video.quality = tier;
        let mbps = format!("{:.1}", f64::from(kbps) / 1000.0);
        let mbps = mbps.trim_end_matches('0').trim_end_matches('.').to_owned();
        QualityCard {
            tier,
            name: match tier {
                QualityTier::Higher => "Higher quality",
                QualityTier::Recommended => "Recommended",
                QualityTier::Smaller => "Smaller file",
            },
            note: format!("{mbps} Mbps"),
            kbps,
            bytes: estimate_size(&at, doc.settings(), duration).bytes,
            current: settings.video.quality == tier,
        }
    })
    .collect()
}

/// What goes into the sheet's columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Preview,
    Summary,
    Output,
    Audio,
    File,
    Quality,
    Checks,
    Status,
}

/// What the sheet says about the export, worked out once per frame.
#[derive(Debug, Clone)]
struct Facts {
    canvas: (u32, u32),
    project_rate: FrameRate,
    duration: Tick,
    estimate: u64,
}

impl Facts {
    fn of(dialog: &ExportDialog, project: &Project) -> Facts {
        let doc = project.doc();
        let duration = doc
            .sequence(doc.root())
            .map_or(Tick::ZERO, Sequence::duration);
        Facts {
            canvas: (doc.settings().width, doc.settings().height),
            project_rate: doc.settings().frame_rate,
            duration,
            estimate: estimate_size(&dialog.settings, doc.settings(), duration).bytes,
        }
    }

    /// The Summary rows.
    fn summary(&self, dialog: &ExportDialog) -> Vec<(&'static str, String)> {
        let (w, h) = dialog.settings.output_dimensions(self.canvas);
        let format = if dialog.settings.audio.include {
            "MP4 · H.264 · AAC"
        } else {
            "MP4 · H.264 · no audio"
        };
        let mut size = format!("≈ {}", format_bytes(self.estimate));
        if let Some(free) = dialog.dest.as_ref().and_then(|d| d.free_bytes) {
            size.push_str(&format!(" · {} free", format_bytes(free)));
        }
        vec![
            ("Format", format.to_owned()),
            (
                "Picture",
                format!(
                    "{w} × {h} · {} fps",
                    rate_label(dialog.settings.output_rate(self.project_rate))
                ),
            ),
            ("Length", format_clock(tick_duration(self.duration))),
            ("Size", size),
        ]
    }
}

/// A row of a section: `label` in `text.secondary` in an 88 pt column,
/// `add` across the rest.
fn form_line<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = theme::look(ui.ctx()).palette;
    let h = theme::look(ui.ctx()).metrics.control_height;
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (cell, _) =
            ui.allocate_exact_size(egui::vec2(sheet::FORM_LABEL, h), egui::Sense::hover());
        ui.painter().text(
            egui::pos2(cell.left(), cell.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            p.text_secondary,
        );
        ui.allocate_ui_with_layout(
            egui::vec2(width - sheet::FORM_LABEL, h),
            Layout::left_to_right(Align::Center),
            add,
        )
        .inner
    })
    .inner
}

/// A read-only value in a row.
fn value_text(ui: &mut egui::Ui, text: &str) {
    let p = theme::look(ui.ctx()).palette;
    ui.label(
        RichText::new(text)
            .text_style(egui::TextStyle::Body)
            .color(p.text_primary),
    );
}

/// A line of body text, wrapping.
fn body_line(ui: &mut egui::Ui, text: &str) {
    let p = theme::look(ui.ctx()).palette;
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .text_style(egui::TextStyle::Body)
                .color(p.text_primary),
        )
        .wrap(),
    );
}

/// A note in `type.small` `text.tertiary`, wrapping.
fn note_text(ui: &mut egui::Ui, text: &str) {
    let p = theme::look(ui.ctx()).palette;
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .text_style(egui::TextStyle::Small)
                .color(p.text_tertiary),
        )
        .wrap(),
    );
}

/// The footer's state line at its left, in `colour`; clickable.
fn footer_note(ui: &mut egui::Ui, colour: egui::Color32, text: &str) -> egui::Response {
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .text_style(egui::TextStyle::Body)
                    .color(colour),
            )
            .truncate()
            .sense(egui::Sense::click()),
        )
    })
    .inner
}

/// A quality card (§13.28): 72 pt, `r.md`, name, bitrate note and size
/// estimate; the current one `overlay.selection` with an `accent` outline.
fn quality_card(ui: &mut egui::Ui, card: &QualityCard, width: f32) -> egui::Response {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 72.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered(),
            ui.style().animation_time,
        );
        let (fill, stroke) = if card.current {
            (p.overlay_selection(), p.accent)
        } else {
            (p.bg_card, p.border_subtle.lerp_to_gamma(p.border_strong, t))
        };
        ui.painter().rect_filled(rect, theme::R_MD, fill);
        ui.painter().rect_stroke(
            rect,
            theme::R_MD,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        let x = rect.left() + 10.0;
        let room = rect.width() - 20.0;
        let label = theme::Type::Caption.style(ui.style()).resolve(ui.style());
        let small = egui::TextStyle::Small.resolve(ui.style());
        let name_colour = if card.current {
            p.accent_text
        } else {
            p.text_primary
        };
        let lines = [
            (
                card.name.to_owned(),
                label.clone(),
                name_colour,
                rect.top() + 10.0,
            ),
            (card.note.clone(), small, p.text_tertiary, rect.top() + 28.0),
            (
                format!("≈ {}", format_bytes(card.bytes)),
                label,
                name_colour,
                rect.bottom() - 24.0,
            ),
        ];
        for (text, font, colour, y) in lines {
            let g = crate::widgets::side_list::one_line(ui, &text, font, colour, room);
            ui.painter().galley(egui::pos2(x, y), g, colour);
        }
        if crate::widgets::icon_button::keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(ui, rect, theme::R_MD);
        }
    }
    let name = card.name;
    let current = card.current;
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, current, name)
    });
    response
}

/// The Preview section: the Stage's frame and a seek track over the
/// timeline with "now / length".
/// The preview's tallest height: a single column keeps its settings in
/// view.
const PREVIEW_MAX: f32 = 260.0;
/// The preview's shortest height when it gives way to the Summary.
const PREVIEW_MIN: f32 = 120.0;

/// The preview's height limit for the next frame: the current `cap`
/// shrunk (or grown back) by how far the Summary under it ends below the
/// body's `room` (`used` is how far down it ends), within
/// [`PREVIEW_MIN`]..=[`PREVIEW_MAX`]. The Summary's last row is then in view
/// above the footer without scrolling.
pub fn next_preview_cap(cap: f32, used: f32, room: f32) -> f32 {
    (cap + room - used).clamp(PREVIEW_MIN, PREVIEW_MAX)
}

fn preview_section(
    ui: &mut egui::Ui,
    project: &Project,
    preview: Option<&TextureHandle>,
    cap: f32,
    facts: &Facts,
    actions: &mut Vec<ExportAction>,
) {
    let p = theme::look(ui.ctx()).palette;
    let width = ui.available_width();
    // 16:9 of the column, at most `cap` tall (see `next_preview_cap`).
    let height = (width * 9.0 / 16.0).min(cap);
    let (frame, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    ui.painter()
        .rect_filled(frame, theme::R_MD, theme::surface::PLAYER_SURROUND);
    if let Some(tex) = preview {
        let size = tex.size_vec2();
        let aspect = size.x / size.y.max(1.0);
        let fitted = if aspect > frame.aspect_ratio() {
            egui::vec2(frame.width(), frame.width() / aspect)
        } else {
            egui::vec2(frame.height() * aspect, frame.height())
        };
        let rect = egui::Rect::from_center_size(frame.center(), fitted);
        egui::Image::from_texture((tex.id(), size)).paint_at(ui, rect);
    }
    // The seek track: 4 pt in a 24 pt row; click or drag seeks.
    let (row, response) =
        ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::click_and_drag());
    let track = egui::Rect::from_center_size(row.center(), egui::vec2(width - 12.0, 4.0));
    let total = facts.duration.raw().max(1) as f64;
    let at = (project.playhead.raw().max(0) as f64 / total).clamp(0.0, 1.0) as f32;
    ui.painter().rect_filled(track, 2.0, p.border_strong);
    let x = egui::lerp(track.x_range(), at);
    ui.painter().rect_filled(
        egui::Rect::from_min_max(track.min, egui::pos2(x, track.max.y)),
        2.0,
        p.accent,
    );
    ui.painter()
        .circle_filled(egui::pos2(x, track.center().y), 6.0, p.accent);
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let f = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
        let raw = (f64::from(f) * total).round() as i64;
        actions.push(ExportAction::Seek(Tick::from_raw(raw)));
    }
    let now = format_clock(tick_duration(project.playhead.min(facts.duration)));
    let len = format_clock(tick_duration(facts.duration));
    note_text(ui, &format!("{now} / {len}"));
}

/// The name a first export suggests: the project file's name.
pub fn project_name(project: &Project) -> String {
    project
        .file
        .as_deref()
        .and_then(Path::file_stem)
        .map_or_else(
            || "Untitled".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
}

/// The last two parts of a folder path, for a narrow field.
fn folder_label(folder: &Path) -> String {
    let parts: Vec<String> = folder
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    match parts.as_slice() {
        [last, parent] if folder.components().count() > 2 => format!("…/{parent}/{last}"),
        _ => folder.display().to_string(),
    }
}

/// Font families used by text clips that this machine doesn't have
/// (TX-007). Checked only once the system scan is done, so a family
/// doesn't flicker as "missing" while fonts load.
fn substituted_fonts(doc: &Document, fonts: &FontRegistry) -> Vec<String> {
    if fonts.scan_state() == ScanState::Scanning {
        return Vec::new();
    }
    let mut missing: Vec<String> = Vec::new();
    for (_, track) in doc
        .sequence(doc.root())
        .into_iter()
        .flat_map(Sequence::tracks)
    {
        for clip in track.clips() {
            if let Some(text) = clip.content.text() {
                let family = &text.style.font.family;
                if !fonts.has_family(family) && !missing.contains(family) {
                    missing.push(family.clone());
                }
            }
        }
    }
    missing
}

/// `d` ticks as a `Duration` (display only).
fn tick_duration(d: Tick) -> Duration {
    let ticks = u64::try_from(d.raw()).unwrap_or(0);
    let per_sec = TICKS_PER_SECOND.unsigned_abs();
    Duration::from_secs(ticks / per_sec)
        + Duration::from_nanos(((ticks % per_sec) * 1_000_000_000) / per_sec)
}

/// "30", "29.97", "23.976" (EX-026 labels).
pub fn rate_label(rate: FrameRate) -> String {
    if rate.den() == 1 {
        return rate.num().to_string();
    }
    let fps = f64::from(rate.num()) / f64::from(rate.den());
    let text = format!("{fps:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// "48 kHz", "44.1 kHz".
pub fn hz_label(hz: u32) -> String {
    if hz.is_multiple_of(1000) {
        format!("{} kHz", hz / 1000)
    } else {
        format!("{:.1} kHz", f64::from(hz) / 1000.0)
    }
}

/// A resolution entry (EX-024): "Canvas size (1080×1920)", "1080p —
/// 1080×1920", "2160p (4K) — 3840×2160".
pub fn size_label(canvas: (u32, u32), size: OutputSize) -> String {
    let (w, h) = output_dimensions(canvas, size);
    match size {
        OutputSize::Canvas => format!("Canvas size ({w}×{h})"),
        OutputSize::Preset(side) => {
            let name = match side {
                ShortSide::P1440 => "1440p (2K)".to_owned(),
                ShortSide::P2160 => "2160p (4K)".to_owned(),
                other => format!("{}p", other.px()),
            };
            format!("{name} — {w}×{h}")
        }
    }
}

/// A quality entry (EX-027): "Recommended — 10 Mbps".
pub fn quality_label(tier: QualityTier, kbps: u32) -> String {
    let name = match tier {
        QualityTier::Smaller => "Smaller file",
        QualityTier::Recommended => "Recommended",
        QualityTier::Higher => "Higher quality",
    };
    let mbps = format!("{:.1}", f64::from(kbps) / 1000.0);
    let mbps = mbps.trim_end_matches('0').trim_end_matches('.');
    format!("{name} — {mbps} Mbps")
}

/// The preflight footer line (EX-036): "Ready", "2 warnings · 1 problem".
pub fn preflight_line(issues: &[Issue], checking: bool) -> String {
    let count = |s: Severity| issues.iter().filter(|i| i.severity == s).count();
    let plural = |n: usize, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    let mut parts = Vec::new();
    let (warnings, problems, infos) = (
        count(Severity::Warning),
        count(Severity::Problem),
        count(Severity::Info),
    );
    if warnings > 0 {
        parts.push(plural(warnings, "warning"));
    }
    if problems > 0 {
        parts.push(plural(problems, "problem"));
    }
    if infos > 0 {
        parts.push(plural(infos, "note"));
    }
    match (checking, parts.is_empty()) {
        (true, _) => "Checking…".to_owned(),
        (false, true) => "Ready".to_owned(),
        (false, false) => parts.join(" · "),
    }
}

fn disabled_reason(issues: &[Issue]) -> String {
    issues
        .iter()
        .find(|i| i.severity == Severity::Problem)
        .map_or_else(|| "Checking…".to_owned(), |i| issue_text(&i.code))
}

/// FreeCut's sentence for a preflight finding (English; the ids are the
/// future Fluent message ids, README §5.13).
pub fn issue_text(code: &IssueCode) -> String {
    let clips = |n: usize| {
        if n == 1 {
            "1 clip".to_owned()
        } else {
            format!("{n} clips")
        }
    };
    match code {
        IssueCode::EmptyTimeline => "The timeline is empty. Add something to export.".to_owned(),
        IssueCode::EmptyFileName => "Enter a file name.".to_owned(),
        IssueCode::FileNameTooLong { chars, .. } => {
            format!("The file name is too long ({chars} characters; 200 at most).")
        }
        IssueCode::NoFolder => "Choose a folder to save to.".to_owned(),
        IssueCode::FolderMissing => {
            "The folder no longer exists. Choose another with Browse….".to_owned()
        }
        IssueCode::FolderNotWritable => {
            "FreeCut can't save in this folder. Choose another with Browse….".to_owned()
        }
        IssueCode::DriveDisconnected { drive } => format!("“{drive}” isn't connected."),
        IssueCode::TargetIsSource { .. } => {
            "This file is used in your project. Choose another name or folder.".to_owned()
        }
        IssueCode::OutputTooLarge { width, height, max } => {
            format!("{width}×{height} is too large to export (at most {max} pixels a side).")
        }
        IssueCode::NoEncoder => {
            "This FFmpeg build has no H.264 encoder (libx264), so FreeCut can't export.".to_owned()
        }
        IssueCode::IncompatibleFormat => "This codec can't be saved in this format.".to_owned(),
        IssueCode::UnsupportedFrameRate { rate } => {
            format!("{} fps isn't offered for export.", rate_label(*rate))
        }
        IssueCode::AudioBitrateUnsupported { kbps } => {
            format!("AAC at {kbps} kbps isn't offered.")
        }
        IssueCode::AudioSampleRateUnsupported { hz } => {
            format!("{} audio isn't offered.", hz_label(*hz))
        }
        IssueCode::LowDiskSpace {
            to_free, volume, ..
        } => match volume {
            Some(v) => format!(
                "Free at least {} on “{v}”, or choose another folder.",
                format_bytes(*to_free)
            ),
            None => format!(
                "Free at least {} on this drive, or choose another folder.",
                format_bytes(*to_free)
            ),
        },
        IssueCode::OfflineMedia { clips: n } => format!(
            "{} use missing media: the video shows an offline slate and the sound is silent.",
            clips(*n)
        ),
        IssueCode::FontsSubstituted { families } => format!(
            "Not installed, shown with Noto Sans: {}.",
            families.join(", ")
        ),
        IssueCode::AllVisualTracksHidden => {
            "Every video track is hidden: the export shows only the background.".to_owned()
        }
        IssueCode::SizeAboveFreeSpace {
            estimate,
            free_bytes,
        } => format!(
            "The file may need up to {}, but only {} is free.",
            format_bytes(*estimate),
            format_bytes(*free_bytes)
        ),
        IssueCode::FrameRateConversion { from, to } => format!(
            "Frames are converted from {} to {} fps.",
            rate_label(*from),
            rate_label(*to)
        ),
        IssueCode::Upscaling { output, source } => format!(
            "The output ({output}p) is larger than your largest source ({source}p); it may look soft."
        ),
        IssueCode::HiddenOrMutedTracks { tracks } => {
            if *tracks == 1 {
                "1 hidden or muted track isn't in the export.".to_owned()
            } else {
                format!("{tracks} hidden or muted tracks aren't in the export.")
            }
        }
        other => other.id().to_owned(),
    }
}

/// How many slated frames the result view names one by one.
const SLATED_SHOWN: usize = 5;

/// The result view's lines (EX-039): size, duration, picture, codecs, time
/// taken, and the frames drawn with the error slate after "Retry using
/// slates" (EX-022).
pub fn result_lines(result: &ExportResult) -> Vec<String> {
    let (w, h) = result.size;
    let mut lines = vec![
        format!(
            "{} · {}",
            format_bytes(result.bytes),
            format_clock(tick_duration(result.duration))
        ),
        format!("{w}×{h} · {} fps", rate_label(result.frame_rate)),
        if result.audio {
            "H.264 video, AAC audio, MP4".to_owned()
        } else {
            "H.264 video, MP4, no audio".to_owned()
        },
        format!("Exported in {}", format_clock(result.took)),
    ];
    if !result.slated.is_empty() {
        let n = result.slated.len();
        lines.push(if n == 1 {
            "1 frame shows the error slate: its picture couldn't be read".to_owned()
        } else {
            format!("{n} frames show the error slate: their pictures couldn't be read")
        });
        lines.extend(
            result
                .slated
                .iter()
                .take(SLATED_SHOWN)
                .map(|f| format!("• {f}")),
        );
        if n > SLATED_SHOWN {
            lines.push(format!("… and {} more", n - SLATED_SHOWN));
        }
    }
    lines
}

/// The names of the soloed tracks of `project`'s timeline (AU-012), in
/// timeline order; empty when nothing is soloed.
fn solo_names(project: &Project) -> Vec<String> {
    let solo = project.solo();
    if solo.is_empty() {
        return Vec::new();
    }
    let doc = project.doc();
    let seq = doc.root();
    crate::ui::mixer::strips(doc, seq, solo)
        .into_iter()
        .filter(|s| s.soloed)
        .map(|s| s.name)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_project::export::{Issue, IssueCode};

    use super::*;

    fn rate(num: u32, den: u32) -> FrameRate {
        FrameRate::new(num, den).unwrap()
    }

    #[test]
    fn rates_read_like_the_spec() {
        assert_eq!(rate_label(rate(30, 1)), "30");
        assert_eq!(rate_label(rate(30_000, 1001)), "29.97");
        assert_eq!(rate_label(rate(24_000, 1001)), "23.976");
        assert_eq!(rate_label(rate(60_000, 1001)), "59.94");
        assert_eq!(hz_label(48_000), "48 kHz");
        assert_eq!(hz_label(44_100), "44.1 kHz");
    }

    #[test]
    fn resolution_entries_show_the_real_size() {
        let portrait = (1080, 1920);
        assert_eq!(
            size_label(portrait, OutputSize::Canvas),
            "Canvas size (1080×1920)"
        );
        assert_eq!(
            size_label(portrait, OutputSize::Preset(ShortSide::P1080)),
            "1080p — 1080×1920"
        );
        assert_eq!(
            size_label((1920, 1080), OutputSize::Preset(ShortSide::P2160)),
            "2160p (4K) — 3840×2160"
        );
        assert_eq!(
            size_label((1920, 1080), OutputSize::Preset(ShortSide::P720)),
            "720p — 1280×720"
        );
    }

    #[test]
    fn quality_entries_show_their_bitrate() {
        assert_eq!(
            quality_label(QualityTier::Recommended, 10_000),
            "Recommended — 10 Mbps"
        );
        assert_eq!(
            quality_label(QualityTier::Smaller, 2_500),
            "Smaller file — 2.5 Mbps"
        );
        assert_eq!(
            quality_label(QualityTier::Higher, 20_000),
            "Higher quality — 20 Mbps"
        );
    }

    fn issue(code: IssueCode) -> Issue {
        Issue {
            severity: code.severity(),
            code,
            targets: Vec::new(),
        }
    }

    #[test]
    fn the_preflight_line_counts_by_severity() {
        assert_eq!(preflight_line(&[], false), "Ready");
        assert_eq!(preflight_line(&[], true), "Checking…");
        let issues = [
            issue(IssueCode::EmptyFileName),
            issue(IssueCode::AllVisualTracksHidden),
            issue(IssueCode::OfflineMedia { clips: 2 }),
        ];
        assert_eq!(preflight_line(&issues, false), "2 warnings · 1 problem");
        assert_eq!(
            disabled_reason(&issues),
            "Enter a file name.",
            "the button says why it is off"
        );
        assert_eq!(
            preflight_line(
                &[issue(IssueCode::HiddenOrMutedTracks { tracks: 1 })],
                false
            ),
            "1 note"
        );
    }

    #[test]
    fn every_issue_has_a_sentence() {
        let codes = [
            IssueCode::EmptyTimeline,
            IssueCode::EmptyFileName,
            IssueCode::FileNameTooLong {
                chars: 201,
                bytes: 205,
            },
            IssueCode::NoFolder,
            IssueCode::FolderMissing,
            IssueCode::FolderNotWritable,
            IssueCode::DriveDisconnected {
                drive: "Card A".into(),
            },
            IssueCode::OutputTooLarge {
                width: 9000,
                height: 5000,
                max: 8192,
            },
            IssueCode::NoEncoder,
            IssueCode::IncompatibleFormat,
            IssueCode::UnsupportedFrameRate { rate: rate(12, 1) },
            IssueCode::AudioBitrateUnsupported { kbps: 100 },
            IssueCode::AudioSampleRateUnsupported { hz: 22_050 },
            IssueCode::LowDiskSpace {
                free_bytes: 50_000_000,
                to_free: 1_200_000_000,
                volume: Some("Data".into()),
            },
            IssueCode::FontsSubstituted {
                families: vec!["Lobster".into()],
            },
            IssueCode::AllVisualTracksHidden,
            IssueCode::SizeAboveFreeSpace {
                estimate: 2_000_000_000,
                free_bytes: 1_000_000_000,
            },
            IssueCode::FrameRateConversion {
                from: rate(25, 1),
                to: rate(30, 1),
            },
            IssueCode::Upscaling {
                output: 2160,
                source: 1080,
            },
        ];
        for code in codes {
            let text = issue_text(&code);
            assert!(!text.starts_with("export-"), "{code:?} has no sentence");
        }
        assert_eq!(
            issue_text(&IssueCode::LowDiskSpace {
                free_bytes: 0,
                to_free: 1_200_000_000,
                volume: Some("Data".into()),
            }),
            "Free at least 1.2 GB on “Data”, or choose another folder."
        );
        assert_eq!(
            issue_text(&IssueCode::DriveDisconnected {
                drive: "Card A".into()
            }),
            "“Card A” isn't connected."
        );
    }

    #[test]
    fn the_result_view_lists_the_file() {
        let result = ExportResult {
            path: "/out/clip.mp4".into(),
            bytes: 12_400_000,
            duration: Tick::from_secs(65).unwrap(),
            size: (1920, 1080),
            frame_rate: rate(30_000, 1001),
            frames: 1948,
            audio: true,
            took: Duration::from_secs(21),
            slated: Vec::new(),
        };
        assert_eq!(
            result_lines(&result),
            [
                "12.4 MB · 1:05",
                "1920×1080 · 29.97 fps",
                "H.264 video, AAC audio, MP4",
                "Exported in 0:21",
            ]
        );

        // After "Retry using slates" the slated frames are listed.
        let slated = ExportResult {
            slated: (0..7)
                .map(|k| format!("“Interview” at frame {k}"))
                .collect(),
            ..result
        };
        let lines = result_lines(&slated);
        assert_eq!(
            lines[4],
            "7 frames show the error slate: their pictures couldn't be read"
        );
        assert_eq!(lines[5], "• “Interview” at frame 0");
        assert_eq!(lines.len(), 4 + 1 + SLATED_SHOWN + 1);
        assert_eq!(lines.last().unwrap(), "… and 2 more");
    }

    #[test]
    fn export_as_heard_is_off_each_time_the_dialog_opens() {
        use freecut_timeline::model::ProjectSettings;
        let project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let mut dialog = ExportDialog::new(&project, None);
        dialog.as_heard = true;
        dialog.visible = false;
        dialog.open(&project, None);
        assert!(!dialog.as_heard);
    }

    #[test]
    fn a_failed_export_of_a_closed_project_is_not_retried_on_the_open_one() {
        use freecut_timeline::model::ProjectSettings;

        use crate::jobs::export::FailureCode;
        let project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let failed = |code| {
            Stage::Failed(ExportFailure {
                code,
                detail: String::new(),
                subject: None,
            })
        };
        let mut dialog = ExportDialog::new(&project, None);
        dialog.settings.file.name = "Project A".into();

        // The project is still open: Retry starts again, and a decode
        // error offers the slate retry.
        dialog.stage = failed(FailureCode::EncoderFailed);
        assert_eq!(
            dialog.fix_chosen(FixAction::Retry),
            Some(ExportAction::Start)
        );
        assert!(matches!(dialog.stage, Stage::Settings));
        dialog.stage = failed(FailureCode::DecodeError);
        assert_eq!(
            dialog.fix_chosen(FailureCode::DecodeError.fixes()[0]),
            Some(ExportAction::StartWithSlates)
        );

        // Another project was opened while it ran: nothing is retried with
        // project A's settings, and only Copy details and Close are offered.
        dialog.project_closed();
        dialog.stage = failed(FailureCode::EncoderFailed);
        let Stage::Failed(failure) = &dialog.stage else {
            unreachable!()
        };
        assert!(dialog.offered_fixes(failure).is_empty());
        assert_eq!(dialog.fix_chosen(FixAction::Retry), None);
        assert!(!dialog.settings_current());

        // Opening Export again takes the open project's settings.
        dialog.open(&project, None);
        assert!(dialog.settings_current());
        assert_eq!(dialog.settings.file.name, "Untitled");
    }

    #[test]
    fn opening_reuses_the_projects_last_settings() {
        use freecut_timeline::model::ProjectSettings;
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let mut dialog = ExportDialog::new(&project, Some("/videos".into()));
        assert!(dialog.visible);
        assert_eq!(dialog.settings.file.name, "Untitled");
        assert_eq!(dialog.settings.file.folder, Some(PathBuf::from("/videos")));
        assert_eq!(dialog.settings.video.size, OutputSize::Canvas);
        assert_eq!(dialog.settings.video.quality, QualityTier::Recommended);
        assert!(
            !dialog.settings.audio.include,
            "an empty timeline is silent"
        );

        let mut last = dialog.settings.clone();
        last.file.name = "Holiday".into();
        last.video.quality = QualityTier::Higher;
        last.file.folder = None;
        project.export = Some(last);
        dialog.open(&project, Some("/videos".into()));
        assert_eq!(dialog.settings.file.name, "Holiday");
        assert_eq!(dialog.settings.video.quality, QualityTier::Higher);
        assert_eq!(
            dialog.settings.file.folder,
            Some(PathBuf::from("/videos")),
            "a folder that didn't survive the file is filled in"
        );
        assert_eq!(dialog.conflict, ConflictChoice::KeepBoth);
    }

    #[test]
    fn folder_labels_stay_short() {
        assert_eq!(
            folder_label(Path::new("/Users/dana/Movies/Exports")),
            "…/Movies/Exports"
        );
        assert_eq!(folder_label(Path::new("/Movies")), "/Movies");
    }

    #[test]
    fn stale_destination_checks_are_ignored() {
        let project =
            Project::new(freecut_timeline::model::ProjectSettings::new_default().unwrap()).unwrap();
        let mut dialog = ExportDialog::new(&project, None);
        dialog.dest_for = Some(("/a".into(), "x".into()));
        dialog.destination_checked(
            ("/b".into(), "x".into()),
            Destination {
                target_exists: true,
                ..Destination::default()
            },
        );
        assert_eq!(dialog.dest, None);
        dialog.destination_checked(("/a".into(), "x".into()), Destination::default());
        assert_eq!(dialog.dest, Some(Destination::default()));
    }

    /// §13.28: the sheet is `min(window − 48, 1160)` wide, in 3 / 2 / 1
    /// columns at 1100 / 760 pt.
    #[test]
    fn the_preview_gives_way_so_the_summary_fits() {
        // The Summary ends 40 pt below the room: the preview shrinks by 40.
        assert_eq!(next_preview_cap(260.0, 568.0, 528.0), 220.0);
        // Room to spare: it grows back, never past its tallest.
        assert_eq!(next_preview_cap(220.0, 400.0, 528.0), PREVIEW_MAX);
        // Never below its shortest.
        assert_eq!(next_preview_cap(260.0, 900.0, 300.0), PREVIEW_MIN);
    }

    #[test]
    fn the_sheet_widens_into_three_columns() {
        assert_eq!(export_sheet_width(1440.0), 1160.0);
        assert_eq!(export_sheet_width(1000.0), 952.0);
        assert_eq!(export_sheet_width(100.0), 100.0);
        assert_eq!(export_columns(1440.0), 3);
        assert_eq!(export_columns(1100.0), 3);
        assert_eq!(export_columns(1099.0), 2);
        assert_eq!(export_columns(760.0), 2);
        assert_eq!(export_columns(759.0), 1);
        assert_eq!(export_button_label(Some(120_000_000)), "Export · ≈ 120 MB");
        assert_eq!(export_button_label(Some(0)), "Export");
        assert_eq!(export_button_label(None), "Export");
    }

    /// EX-027 as cards: Higher, Recommended, Smaller, each with its
    /// bitrate and estimate; the chosen tier is current.
    #[test]
    fn quality_cards_list_the_tiers_with_estimates() {
        use freecut_timeline::edit::InsertTarget;
        use freecut_timeline::model::ProjectSettings;
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        crate::ui::text::insert_text(&mut project, InsertTarget::FirstFree, Tick::ZERO).unwrap();
        let doc = project.doc();
        let duration = doc.sequence(doc.root()).unwrap().duration();
        let mut settings = ExportSettings::defaults("x", None, doc);
        let cards = quality_cards(&settings, doc, duration);
        let tiers: Vec<_> = cards.iter().map(|c| c.tier).collect();
        assert_eq!(
            tiers,
            [
                QualityTier::Higher,
                QualityTier::Recommended,
                QualityTier::Smaller
            ]
        );
        assert!(cards[0].bytes > cards[1].bytes && cards[1].bytes > cards[2].bytes);
        assert!(cards[1].current && !cards[0].current && !cards[2].current);
        assert!(cards[1].note.ends_with(" Mbps"), "{}", cards[1].note);
        assert_eq!(
            cards[1].bytes,
            estimate_size(&settings, doc.settings(), duration).bytes
        );
        settings.video.quality = QualityTier::Smaller;
        let cards = quality_cards(&settings, doc, duration);
        assert!(cards[2].current);
    }

    /// The sheet draws headless in its settings stage and asks for the
    /// encoder and folder checks, as the dialog did.
    #[test]
    fn the_sheet_draws_and_asks_for_its_checks() {
        use freecut_timeline::model::ProjectSettings;
        let project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let mut dialog = ExportDialog::new(&project, Some("/videos".into()));
        let ctx = egui::Context::default();
        crate::theme::apply(
            &ctx,
            &crate::theme::TOKENS,
            crate::theme::ThemeKind::Dark,
            crate::theme::Density::Comfortable,
        );
        let fonts = FontRegistry::bundled();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            ..egui::RawInput::default()
        };
        let mut actions = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            actions = dialog.show(ui.ctx(), &project, &fonts, None);
        });
        out.textures_delta.clear();
        assert!(actions.contains(&ExportAction::ProbeEncoder), "{actions:?}");
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, ExportAction::Probe { .. })),
            "{actions:?}"
        );
        assert!(dialog.visible);
    }
}
